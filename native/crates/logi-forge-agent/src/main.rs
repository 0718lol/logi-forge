use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use logi_forge_core::{Config, ForgeError, HidNode, KeyboardLightingConfig, Result, RgbColor};
use logi_forge_hidpp::{DIRECT_DEVICE_INDEX, HidppClient};
use logi_forge_linux::{HidrawTransport, discover_logitech};
use serde_json::{Value, json};

const PROTOCOL_VERSION: u8 = 1;
const POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug)]
struct AgentState {
    revision: u64,
    fingerprint: String,
    devices: Vec<Value>,
    config: Value,
    apply: Value,
    updated_at_ms: u128,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            revision: 0,
            fingerprint: String::new(),
            devices: Vec::new(),
            config: json!({ "status": "loading" }),
            apply: json!({ "status": "disabled", "reason": "LOGI_FORGE_DEVICE_PATH is unset" }),
            updated_at_ms: now_ms(),
        }
    }
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("logi-forge-agent: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let socket_path = env::var_os("LOGI_FORGE_AGENT_SOCKET").map_or_else(
        || PathBuf::from("/tmp/logi-forge-agent.sock"),
        PathBuf::from,
    );
    let config_path = env::var_os("LOGI_FORGE_CONFIG")
        .map_or_else(|| PathBuf::from("examples/logi-forge.toml"), PathBuf::from);
    if socket_path.exists() {
        fs::remove_file(&socket_path).map_err(|error| ForgeError::Io {
            operation: "remove stale agent socket",
            detail: format!("{}: {error}", socket_path.display()),
        })?;
    }
    let listener = UnixListener::bind(&socket_path).map_err(|error| ForgeError::Io {
        operation: "bind agent socket",
        detail: format!("{}: {error}", socket_path.display()),
    })?;
    let state = Arc::new(Mutex::new(AgentState::default()));
    let operations = Arc::new(Mutex::new(()));
    let poll_state = Arc::clone(&state);
    let poll_config_path = config_path.clone();
    thread::spawn(move || poll_runtime(&poll_state, &poll_config_path));
    eprintln!("logi-forge-agent socket={}", socket_path.display());

    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                let connection_state = Arc::clone(&state);
                let connection_operations = Arc::clone(&operations);
                let connection_config_path = config_path.clone();
                thread::spawn(move || {
                    if let Err(error) = handle_connection(
                        stream,
                        &connection_state,
                        &connection_operations,
                        &connection_config_path,
                    ) {
                        eprintln!("logi-forge-agent connection: {error}");
                    }
                });
            }
            Err(error) => {
                return Err(ForgeError::Io {
                    operation: "accept agent connection",
                    detail: error.to_string(),
                });
            }
        }
    }
    Ok(())
}

fn poll_runtime(state: &Arc<Mutex<AgentState>>, config_path: &Path) {
    let mut applied_fingerprint = String::new();
    loop {
        let inventory = discover_logitech();
        let config = Config::load_from_path(config_path);
        let fingerprint = format!("{inventory:?}|{config:?}");
        let apply = if fingerprint == applied_fingerprint {
            None
        } else {
            applied_fingerprint.clone_from(&fingerprint);
            Some(apply_config_if_routed(config.as_ref().ok()))
        };
        let devices = inventory
            .as_ref()
            .map(|nodes| nodes.iter().map(device_json).collect())
            .unwrap_or_default();
        let config_json = match &config {
            Ok(value) => json!({
                "status": "ready",
                "path": config_path,
                "schemaVersion": value.schema_version,
                "selectedDevice": value.selected_device,
            }),
            Err(error) => json!({
                "status": "error",
                "path": config_path,
                "error": error.to_string(),
            }),
        };
        let inventory_error = inventory.err().map(|error| error.to_string());
        let mut current = lock_state(state);
        if current.fingerprint != fingerprint {
            current.revision += 1;
            current.fingerprint = fingerprint;
        }
        current.devices = devices;
        current.config = config_json;
        if let Some(apply) = apply {
            current.apply = apply;
        }
        current.updated_at_ms = now_ms();
        if let Some(error) = inventory_error {
            current.devices.clear();
            current.apply = json!({ "status": "inventory-error", "error": error });
        }
        drop(current);
        thread::sleep(POLL_INTERVAL);
    }
}

fn apply_config_if_routed(config: Option<&Config>) -> Value {
    let Some(config) = config else {
        return json!({ "status": "skipped", "reason": "configuration is invalid" });
    };
    let Some(path) = env::var_os("LOGI_FORGE_DEVICE_PATH").map(PathBuf::from) else {
        return json!({ "status": "disabled", "reason": "LOGI_FORGE_DEVICE_PATH is unset" });
    };
    let index = match parse_device_index(env::var("LOGI_FORGE_DEVICE_INDEX").as_deref().ok()) {
        Ok(index) => index,
        Err(error) => return json!({ "status": "error", "error": error.to_string() }),
    };
    match apply_keyboard_config(config, &path, index) {
        Ok(operations) => json!({
            "status": "applied",
            "path": path,
            "deviceIndex": format!("{index:02x}"),
            "operations": operations,
        }),
        Err(error) => json!({
            "status": "error",
            "path": path,
            "error": error.to_string(),
        }),
    }
}

fn apply_keyboard_config(config: &Config, path: &Path, index: u8) -> Result<Vec<Value>> {
    let device = config.devices.get(&config.selected_device).ok_or_else(|| {
        ForgeError::ConfigError(format!(
            "selected_device {} is missing from devices",
            config.selected_device
        ))
    })?;
    let Some(keyboard) = &device.keyboard else {
        return Ok(Vec::new());
    };
    let transport = HidrawTransport::open(path)?;
    let mut client = HidppClient::new(transport, index);
    let mut operations = Vec::new();
    if let Some(enabled) = keyboard.fn_lock {
        let info = client.set_fn_lock(enabled)?;
        operations.push(json!({
            "operation": "fn-lock",
            "enabled": info.enabled,
            "feature": format!("{:?}", info.feature),
        }));
    }
    if let Some(lighting) = &keyboard.lighting {
        let color = scale_color(RgbColor::parse(&lighting.color)?, lighting.brightness);
        let info = client.set_keyboard_color(color)?;
        operations.push(json!({
            "operation": "lighting",
            "backend": format!("{:?}", info.backend),
            "zonesWritten": info.zones_written,
            "brightness": lighting.brightness,
        }));
    }
    Ok(operations)
}

fn scale_color(color: RgbColor, brightness: u8) -> RgbColor {
    let scale =
        |channel| u8::try_from(u16::from(channel) * u16::from(brightness) / 100).unwrap_or(0);
    RgbColor {
        red: scale(color.red),
        green: scale(color.green),
        blue: scale(color.blue),
    }
}

fn parse_device_index(value: Option<&str>) -> Result<u8> {
    let Some(value) = value else {
        return Ok(DIRECT_DEVICE_INDEX);
    };
    if value.eq_ignore_ascii_case("ff") {
        return Ok(DIRECT_DEVICE_INDEX);
    }
    let parsed = value
        .parse::<u8>()
        .map_err(|_| ForgeError::InvalidArgument(format!("invalid device index {value}")))?;
    if !(1..=6).contains(&parsed) {
        return Err(ForgeError::InvalidArgument(
            "receiver index must be 1..6 or ff".into(),
        ));
    }
    Ok(parsed)
}

fn device_json(node: &HidNode) -> Value {
    let key = node.serial.as_ref().map_or_else(
        || {
            format!(
                "hidraw:{:04x}:{:04x}:{}",
                node.vendor_id,
                node.product_id,
                node.path.display()
            )
        },
        |serial| format!("serial:{serial}"),
    );
    json!({
        "key": key,
        "route": node.path,
        "name": node.name.as_deref().unwrap_or("Logitech HID++ device"),
        "vendorId": format!("{:04x}", node.vendor_id),
        "productId": format!("{:04x}", node.product_id),
        "serial": node.serial,
        "bus": format!("{:?}", node.bus),
        "online": true,
    })
}

fn handle_connection(
    mut stream: UnixStream,
    state: &Arc<Mutex<AgentState>>,
    operations: &Arc<Mutex<()>>,
    config_path: &Path,
) -> Result<()> {
    let mut request = String::new();
    BufReader::new(stream.try_clone().map_err(io_error("clone agent stream"))?)
        .read_line(&mut request)
        .map_err(|error| ForgeError::Io {
            operation: "read agent request",
            detail: error.to_string(),
        })?;
    let request = serde_json::from_str::<Value>(&request)
        .map_err(|error| ForgeError::InvalidArgument(format!("invalid agent request: {error}")))?;
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let response = match method {
        "health" => json!({ "status": "ok", "protocolVersion": PROTOCOL_VERSION }),
        "snapshot" => snapshot_json(&lock_state(state)),
        "write" => write_setting(&request, state, operations, config_path),
        _ => json!({
            "error": { "code": "UNKNOWN_METHOD", "message": format!("Unknown method: {method}") }
        }),
    };
    let mut body = serde_json::to_vec(&response)
        .map_err(|error| ForgeError::InvalidResponse(error.to_string()))?;
    body.push(b'\n');
    stream.write_all(&body).map_err(|error| ForgeError::Io {
        operation: "write agent response",
        detail: error.to_string(),
    })
}

fn write_setting(
    request: &Value,
    state: &Arc<Mutex<AgentState>>,
    operations: &Arc<Mutex<()>>,
    config_path: &Path,
) -> Value {
    let _operation = operations
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let expected_revision = request.get("revision").and_then(Value::as_u64);
    let actual_revision = lock_state(state).revision;
    if expected_revision.is_some_and(|expected| expected != actual_revision) {
        return json!({
            "error": {
                "code": "REVISION_CONFLICT",
                "message": "Native agent state changed; refresh and retry",
                "details": { "expectedRevision": expected_revision, "actualRevision": actual_revision },
            }
        });
    }
    let Some(path) = request.get("path").and_then(Value::as_str) else {
        return request_error("INVALID_VALUE", "write requires path");
    };
    let Some(value) = request.get("value") else {
        return request_error("INVALID_VALUE", "write requires value");
    };
    let mut config = match Config::load_from_path(config_path) {
        Ok(config) => config,
        Err(error) => return request_error("CONFIG_LOAD_FAILED", &error.to_string()),
    };
    if let Err(error) = update_keyboard_setting(&mut config, path, value) {
        return request_error("INVALID_VALUE", &error.to_string());
    }
    if let Err(error) = persist_config(config_path, &config) {
        return request_error("CONFIG_WRITE_FAILED", &error.to_string());
    }
    let apply = apply_config_if_routed(Some(&config));
    let mut current = lock_state(state);
    current.revision += 1;
    current.config = json!({
        "status": "ready",
        "path": config_path,
        "schemaVersion": config.schema_version,
        "selectedDevice": config.selected_device,
    });
    current.apply = apply.clone();
    current.updated_at_ms = now_ms();
    let snapshot = snapshot_json(&current);
    json!({ "status": "ok", "apply": apply, "snapshot": snapshot })
}

fn update_keyboard_setting(config: &mut Config, path: &str, value: &Value) -> Result<()> {
    let selected = config
        .devices
        .get_mut(&config.selected_device)
        .ok_or_else(|| {
            ForgeError::ConfigError(format!(
                "selected_device {} is missing from devices",
                config.selected_device
            ))
        })?;
    let keyboard = selected.keyboard.as_mut().ok_or_else(|| {
        ForgeError::ConfigError("selected device has no keyboard configuration".into())
    })?;
    match path {
        "fnLock" => {
            keyboard.fn_lock = Some(
                value
                    .as_bool()
                    .ok_or_else(|| ForgeError::InvalidArgument("fnLock must be boolean".into()))?,
            );
        }
        "lighting" => {
            let color = value.as_str().ok_or_else(|| {
                ForgeError::InvalidArgument("lighting must be an RGB color".into())
            })?;
            RgbColor::parse(color)?;
            keyboard
                .lighting
                .get_or_insert(KeyboardLightingConfig {
                    color: String::new(),
                    brightness: 100,
                })
                .color = color.trim_start_matches('#').to_ascii_lowercase();
        }
        "brightness" => {
            let brightness = value
                .as_u64()
                .and_then(|number| u8::try_from(number).ok())
                .ok_or_else(|| ForgeError::InvalidArgument("brightness must be 0..=100".into()))?;
            if brightness > 100 {
                return Err(ForgeError::InvalidArgument(
                    "brightness must be 0..=100".into(),
                ));
            }
            keyboard
                .lighting
                .get_or_insert(KeyboardLightingConfig {
                    color: "ffffff".into(),
                    brightness,
                })
                .brightness = brightness;
        }
        _ => {
            return Err(ForgeError::InvalidArgument(format!(
                "native keyboard field {path} is not writable"
            )));
        }
    }
    config.validate()
}

fn persist_config(path: &Path, config: &Config) -> Result<()> {
    let body = config.to_toml_string()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| ForgeError::Io {
            operation: "create config directory",
            detail: error.to_string(),
        })?;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, body).map_err(|error| ForgeError::Io {
        operation: "write temporary config",
        detail: error.to_string(),
    })?;
    fs::rename(&temporary, path).map_err(|error| ForgeError::Io {
        operation: "replace config",
        detail: error.to_string(),
    })
}

fn request_error(code: &str, message: &str) -> Value {
    json!({ "error": { "code": code, "message": message, "details": {} } })
}

fn snapshot_json(state: &AgentState) -> Value {
    json!({
        "status": "online",
        "protocolVersion": PROTOCOL_VERSION,
        "revision": state.revision,
        "inventoryStatus": "ready",
        "transport": "native-unix",
        "updatedAtMs": state.updated_at_ms.to_string(),
        "devices": state.devices,
        "config": state.config,
        "apply": state.apply,
    })
}

fn lock_state(state: &Arc<Mutex<AgentState>>) -> MutexGuard<'_, AgentState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn io_error(operation: &'static str) -> impl FnOnce(std::io::Error) -> ForgeError {
    move |error| ForgeError::Io {
        operation,
        detail: error.to_string(),
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_rgb_without_overflow() {
        assert_eq!(
            scale_color(
                RgbColor {
                    red: 255,
                    green: 128,
                    blue: 1,
                },
                50,
            ),
            RgbColor {
                red: 127,
                green: 64,
                blue: 0,
            }
        );
    }

    #[test]
    fn validates_receiver_indices() {
        assert_eq!(parse_device_index(None).unwrap(), 0xff);
        assert_eq!(parse_device_index(Some("ff")).unwrap(), 0xff);
        assert_eq!(parse_device_index(Some("2")).unwrap(), 2);
        assert!(parse_device_index(Some("7")).is_err());
    }

    #[test]
    fn updates_keyboard_settings_with_strict_types() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/logi-forge.toml");
        let mut config = Config::load_from_path(path).unwrap();
        update_keyboard_setting(&mut config, "fnLock", &json!(true)).unwrap();
        update_keyboard_setting(&mut config, "lighting", &json!("#AABBCC")).unwrap();
        update_keyboard_setting(&mut config, "brightness", &json!(35)).unwrap();
        let keyboard = config.devices[&config.selected_device]
            .keyboard
            .as_ref()
            .unwrap();
        assert_eq!(keyboard.fn_lock, Some(true));
        assert_eq!(keyboard.lighting.as_ref().unwrap().color, "aabbcc");
        assert_eq!(keyboard.lighting.as_ref().unwrap().brightness, 35);
        assert!(update_keyboard_setting(&mut config, "brightness", &json!(101)).is_err());
        assert!(update_keyboard_setting(&mut config, "lighting", &json!("invalid")).is_err());
    }

    #[test]
    fn persists_config_with_an_atomic_replace() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/logi-forge.toml");
        let config = Config::load_from_path(source).unwrap();
        let target = env::temp_dir().join(format!("logi-forge-agent-{}.toml", now_ms()));
        persist_config(&target, &config).unwrap();
        assert_eq!(Config::load_from_path(&target).unwrap(), config);
        fs::remove_file(target).unwrap();
    }
}
