use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::path::Path;
use std::process::ExitCode;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use logi_forge_core::{
    Action, ActionRouter, AppProfile, Config, ForgeError, HidNode, Result, RgbColor, WheelMode,
};
use logi_forge_hidpp::{DIRECT_DEVICE_INDEX, HidppClient, keyboard_control_cid};
use logi_forge_linux::{
    HidrawTransport, LinuxInputInjector, capture_events, capture_events_with_foreground,
    capture_routing_from_config, discover_logitech, foreground_app,
    keyboard_capture_routing_from_config, resolve_evdev_button_codes,
};

const HELP: &str = r"Logi Forge native diagnostics

Usage:
  logi-forge list [--json]
  logi-forge probe <hidraw-path> [--index <ff|1..6>]
  logi-forge battery <hidraw-path> [--index <ff|1..6>]
  logi-forge dpi get <hidraw-path> [--index <ff|1..6>]
  logi-forge dpi set <hidraw-path> <value> [--index <ff|1..6>]
  logi-forge smartshift get <hidraw-path> [--index <ff|1..6>]
  logi-forge smartshift set <hidraw-path> <ratchet|free> [--torque <1..100>] [--index <ff|1..6>]
  logi-forge inject <action label>
  logi-forge capture <event-path> --config <config-path> [--bind <button>=<action label>]
  logi-forge capture <event-path> --bind <button>=<action label> [--bind ...]
      [--profile <profile-id>:<app-id>=<button-code>:<action label>]
  logi-forge keyboard capture <event-path> --config <config-path>
  logi-forge keyboard divert <hidraw-path> --config <config-path> [--index <ff|1..6>]
  logi-forge keyboard fn-lock get <hidraw-path> [--index <ff|1..6>]
  logi-forge keyboard fn-lock set <hidraw-path> <on|off> [--index <ff|1..6>]
  logi-forge keyboard lighting set <hidraw-path> <RRGGBB> [--index <ff|1..6>]
  logi-forge foreground
  logi-forge config validate <config-path>
  logi-forge config print <config-path>

Direct USB/Bluetooth devices normally use index ff. Receiver slots use 1..6.
Stop Logitech Options+ before opening a receiver; both agents require exclusive access.
";

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(ForgeError::InvalidArgument(message)) => {
            eprintln!("error: {message}\n\n{HELP}");
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("error: {error}");
            if matches!(error, ForgeError::PermissionDenied(_)) {
                eprintln!(
                    "hint: install packaging/99-logi-forge.rules or run with suitable hidraw permissions"
                );
            }
            ExitCode::from(1)
        }
    }
}

fn run(mut args: Vec<String>) -> Result<()> {
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        println!("{HELP}");
        return Ok(());
    }
    let command = args.remove(0);
    match command.as_str() {
        "list" => list_nodes(&args),
        "probe" => {
            let index = take_index(&mut args)?;
            let path = one_path(&args)?;
            with_client(&path, index, |client| {
                let (major, minor) = client.ping()?;
                let features = client.feature_map()?;
                println!("path: {}", path.display());
                println!("device_index: 0x{index:02x}");
                println!("hidpp: {major}.{minor}");
                println!("features: {features:?}");
                Ok(())
            })
        }
        "battery" => {
            let index = take_index(&mut args)?;
            let path = one_path(&args)?;
            with_client(&path, index, |client| {
                let info = client.battery()?;
                println!(
                    "battery: {}% status={:?} feature=0x{:04x}",
                    info.percentage, info.status, info.source_feature
                );
                Ok(())
            })
        }
        "dpi" => dpi_command(args),
        "smartshift" => smartshift_command(args),
        "inject" => inject_command(&args),
        "capture" => capture_command(args),
        "keyboard" => keyboard_command(args),
        "foreground" if args.is_empty() => foreground_command(),
        "config" => config_command(args),
        _ => Err(ForgeError::InvalidArgument(format!(
            "unknown command {command}"
        ))),
    }
}

fn keyboard_command(mut args: Vec<String>) -> Result<()> {
    let operation = take_first(&mut args, "keyboard requires capture or fn-lock")?;
    match operation.as_str() {
        "capture" => keyboard_capture_command(args),
        "divert" => keyboard_divert_command(args),
        "fn-lock" => fn_lock_command(args),
        "lighting" => keyboard_lighting_command(args),
        _ => Err(ForgeError::InvalidArgument(format!(
            "unknown keyboard command {operation}"
        ))),
    }
}

fn keyboard_divert_command(mut args: Vec<String>) -> Result<()> {
    let index = take_index(&mut args)?;
    let config_path = take_optional_value(&mut args, "--config")?.ok_or_else(|| {
        ForgeError::InvalidArgument("keyboard divert requires --config <config-path>".into())
    })?;
    let path = one_path(&args)?;
    let config = Config::load_from_path(config_path)?;
    let requested = media_bindings_from_config(&config)?;
    if requested.is_empty() {
        return Err(ForgeError::ConfigError(
            "selected keyboard has no media-mode F4-F12 bindings".into(),
        ));
    }

    let transport = HidrawTransport::open(&path)?;
    let mut client = HidppClient::new(transport, index);
    let controls = client.reprog_controls()?;
    let available = controls
        .controls
        .iter()
        .filter(|control| control.is_divertable())
        .map(|control| control.cid)
        .collect::<BTreeSet<_>>();
    let armed = requested
        .into_iter()
        .filter(|(cid, _)| available.contains(cid))
        .collect::<BTreeMap<_, _>>();
    if armed.is_empty() {
        return Err(ForgeError::FeatureUnsupported(0x1b04));
    }

    let mut injector = LinuxInputInjector::open()?;
    let stop = Arc::new(AtomicBool::new(false));
    let handler_stop = Arc::clone(&stop);
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        handler_stop.store(true, Ordering::Relaxed);
    });

    let mut diverted = Vec::new();
    for &cid in armed.keys() {
        if let Err(error) = client.set_control_diverted(controls.feature_index, cid, true) {
            let _ = restore_diverted_controls(&mut client, controls.feature_index, &diverted);
            return Err(error);
        }
        diverted.push(cid);
    }
    let router = ActionRouter::new(armed);
    let mut held = BTreeSet::new();
    println!(
        "diverting {} media-mode controls from {}; press Enter to stop",
        diverted.len(),
        path.display()
    );
    let session_result = loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        match client.next_diverted_controls(controls.feature_index, Duration::from_millis(250)) {
            Ok(snapshot) => {
                let current = snapshot
                    .into_iter()
                    .filter(|cid| *cid != 0 && diverted.contains(cid))
                    .collect::<BTreeSet<_>>();
                if let Err(error) = dispatch_control_edges(&router, &mut injector, &held, &current)
                {
                    break Err(error);
                }
                held = current;
            }
            Err(ForgeError::Timeout) => {}
            Err(error) => break Err(error),
        }
    };
    let empty = BTreeSet::new();
    let release_result = dispatch_control_edges(&router, &mut injector, &held, &empty);
    let restore_result = restore_diverted_controls(&mut client, controls.feature_index, &diverted);
    session_result.and(release_result).and(restore_result)
}

fn media_bindings_from_config(config: &Config) -> Result<Vec<(u16, Action)>> {
    if !config.app_settings.capture_mouse_events {
        return Err(ForgeError::ConfigError(
            "app_settings.capture_mouse_events is disabled".into(),
        ));
    }
    let device = config.devices.get(&config.selected_device).ok_or_else(|| {
        ForgeError::ConfigError(format!(
            "selected_device {} is missing from devices",
            config.selected_device
        ))
    })?;
    if !device.enabled {
        return Err(ForgeError::ConfigError(format!(
            "selected device {} is disabled",
            config.selected_device
        )));
    }
    let keyboard = device.keyboard.as_ref().ok_or_else(|| {
        ForgeError::ConfigError("selected device has no keyboard configuration".into())
    })?;
    Ok(keyboard
        .bindings
        .iter()
        .filter_map(|(label, action)| keyboard_control_cid(label).map(|cid| (cid, action.clone())))
        .collect())
}

fn dispatch_control_edges(
    router: &ActionRouter,
    injector: &mut LinuxInputInjector,
    before: &BTreeSet<u16>,
    after: &BTreeSet<u16>,
) -> Result<()> {
    for (&cid, value) in before
        .difference(after)
        .map(|cid| (cid, 0))
        .chain(after.difference(before).map(|cid| (cid, 1)))
    {
        if let Some(dispatch) = router.route(logi_forge_core::ButtonEvent { code: cid, value })? {
            injector.dispatch(&dispatch)?;
        }
    }
    Ok(())
}

fn restore_diverted_controls(
    client: &mut HidppClient<HidrawTransport>,
    feature_index: u8,
    diverted: &[u16],
) -> Result<()> {
    let mut first_error = None;
    for &cid in diverted {
        if let Err(error) = client.set_control_diverted(feature_index, cid, false)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    if let Some(error) = first_error {
        Err(error)
    } else {
        Ok(())
    }
}

fn keyboard_lighting_command(mut args: Vec<String>) -> Result<()> {
    let index = take_index(&mut args)?;
    let operation = take_first(&mut args, "keyboard lighting requires set")?;
    let path = take_first(&mut args, "keyboard lighting requires a hidraw path")?;
    if operation != "set" || args.len() != 1 {
        return Err(ForgeError::InvalidArgument(
            "keyboard lighting requires set <hidraw-path> <RRGGBB>".into(),
        ));
    }
    let color = RgbColor::parse(&args[0])?;
    with_client(Path::new(&path), index, |client| {
        let info = client.set_keyboard_color(color)?;
        println!(
            "keyboard-lighting: #{:02x}{:02x}{:02x} zones={} backend={:?} volatile",
            color.red, color.green, color.blue, info.zones_written, info.backend
        );
        Ok(())
    })
}

fn keyboard_capture_command(mut args: Vec<String>) -> Result<()> {
    let config_path = take_optional_value(&mut args, "--config")?.ok_or_else(|| {
        ForgeError::InvalidArgument("keyboard capture requires --config <config-path>".into())
    })?;
    if args.len() != 1 {
        return Err(ForgeError::InvalidArgument(
            "keyboard capture requires one event path".into(),
        ));
    }
    let routing = keyboard_capture_routing_from_config(&Config::load_from_path(config_path)?)?;
    if routing.bindings.is_empty() {
        return Err(ForgeError::ConfigError(
            "selected keyboard has no F-row bindings".into(),
        ));
    }
    let router = ActionRouter::new(routing.bindings);
    println!(
        "capturing keyboard {} exclusively; press Ctrl+C to stop",
        args[0]
    );
    capture_events(&args[0], &router)
}

fn fn_lock_command(mut args: Vec<String>) -> Result<()> {
    let index = take_index(&mut args)?;
    let operation = take_first(&mut args, "keyboard fn-lock requires get or set")?;
    let path = take_first(&mut args, "keyboard fn-lock requires a hidraw path")?;
    match operation.as_str() {
        "get" if args.is_empty() => with_client(Path::new(&path), index, |client| {
            let info = client.fn_lock()?;
            print_fn_lock(info);
            Ok(())
        }),
        "set" if args.len() == 1 => {
            let enabled = match args[0].as_str() {
                "on" => true,
                "off" => false,
                value => {
                    return Err(ForgeError::InvalidArgument(format!(
                        "invalid Fn-lock state {value}; expected on or off"
                    )));
                }
            };
            with_client(Path::new(&path), index, |client| {
                let info = client.set_fn_lock(enabled)?;
                print_fn_lock(info);
                Ok(())
            })
        }
        _ => Err(ForgeError::InvalidArgument(
            "invalid keyboard fn-lock command".into(),
        )),
    }
}

fn print_fn_lock(info: logi_forge_core::FnLockInfo) {
    println!(
        "fn-lock: {} default={} manual={} feature={:?}",
        if info.enabled { "on" } else { "off" },
        if info.default_enabled { "on" } else { "off" },
        info.supports_manual_toggle,
        info.feature
    );
}

fn foreground_command() -> Result<()> {
    match foreground_app()? {
        Some(app) => println!(
            "foreground: id={} name={} executable={}",
            app.id,
            app.name,
            app.executable.as_deref().unwrap_or("unknown")
        ),
        None => println!("foreground: unavailable"),
    }
    Ok(())
}

fn inject_command(args: &[String]) -> Result<()> {
    if args.is_empty() {
        return Err(ForgeError::InvalidArgument(
            "inject requires an action label".into(),
        ));
    }
    let action = Action::parse(&args.join(" "))?;
    let dispatch = action.input_command().map(|command| {
        if action.is_hold() {
            logi_forge_core::Dispatch::HoldStart(command)
        } else {
            logi_forge_core::Dispatch::Tap(command)
        }
    })?;
    let mut injector = LinuxInputInjector::open()?;
    injector.dispatch(&dispatch)?;
    if action.is_hold() {
        std::thread::sleep(std::time::Duration::from_millis(120));
        let command = action.input_command()?;
        injector.dispatch(&logi_forge_core::Dispatch::HoldEnd(command))?;
    }
    println!("injected: {action}");
    Ok(())
}

fn capture_command(mut args: Vec<String>) -> Result<()> {
    let options = parse_capture_options(&mut args)?;
    let router = Arc::new(ActionRouter::with_profiles(
        options.bindings,
        options.profiles.clone(),
    ));
    println!(
        "capturing {} exclusively; press Ctrl+C to stop",
        options.event_path
    );
    if options.profiles.is_empty() {
        capture_events(&options.event_path, &router)
    } else {
        capture_events_with_foreground(&options.event_path, &router)
    }
}

#[derive(Debug)]
struct CaptureOptions {
    event_path: String,
    bindings: Vec<(u16, Action)>,
    profiles: Vec<AppProfile>,
}

fn parse_capture_options(args: &mut Vec<String>) -> Result<CaptureOptions> {
    let config_path = take_optional_value(args, "--config")?;
    let (configured_bindings, mut profiles) = if let Some(path) = config_path {
        let routing = capture_routing_from_config(&Config::load_from_path(path)?)?;
        (routing.bindings, routing.profiles)
    } else {
        (Vec::new(), Vec::new())
    };
    let mut bindings = configured_bindings.into_iter().collect::<HashMap<_, _>>();
    while let Some(position) = args.iter().position(|arg| arg == "--bind") {
        if position + 1 >= args.len() {
            return Err(ForgeError::InvalidArgument(
                "--bind requires code=action".into(),
            ));
        }
        let binding = args.remove(position + 1);
        args.remove(position);
        let (code, action) = binding.split_once('=').ok_or_else(|| {
            ForgeError::InvalidArgument("binding must look like 275=BrowserBack".into())
        })?;
        let action = Action::parse(action)?;
        for code in resolve_evdev_button_codes(code)? {
            bindings.insert(code, action.clone());
        }
    }
    while let Some(position) = args.iter().position(|arg| arg == "--profile") {
        if position + 1 >= args.len() {
            return Err(ForgeError::InvalidArgument(
                "--profile requires profile-id:app-id=code:action".into(),
            ));
        }
        let specification = args.remove(position + 1);
        args.remove(position);
        let (target, action_label) = specification.split_once('=').ok_or_else(|| {
            ForgeError::InvalidArgument(
                "profile must look like vscode:com.example.Editor=275:Copy".into(),
            )
        })?;
        let (profile_id, app_id) = target.split_once(':').ok_or_else(|| {
            ForgeError::InvalidArgument("profile target must look like profile-id:app-id".into())
        })?;
        let (code, action_label) = action_label.split_once(':').ok_or_else(|| {
            ForgeError::InvalidArgument("profile action must look like code:action".into())
        })?;
        let action = Action::parse(action_label)?;
        let profile_bindings = resolve_evdev_button_codes(code)?
            .into_iter()
            .map(|code| (code, action.clone()));
        profiles.push(AppProfile::for_app(profile_id, app_id, profile_bindings));
    }
    if args.len() != 1 || (bindings.is_empty() && profiles.is_empty()) {
        return Err(ForgeError::InvalidArgument(
            "capture requires one event path and bindings from --config, --bind, or --profile"
                .into(),
        ));
    }
    Ok(CaptureOptions {
        event_path: args.remove(0),
        bindings: bindings.into_iter().collect(),
        profiles,
    })
}

fn config_command(mut args: Vec<String>) -> Result<()> {
    let operation = take_first(&mut args, "config requires validate or print")?;
    let path = one_path(&args)?;
    let config = Config::load_from_path(&path)?;
    match operation.as_str() {
        "validate" => {
            println!(
                "config: ok schema={} selected_device={} devices={}",
                config.schema_version,
                config.selected_device,
                config.devices.len()
            );
            Ok(())
        }
        "print" => {
            print!("{}", config.to_toml_string()?);
            Ok(())
        }
        _ => Err(ForgeError::InvalidArgument(format!(
            "unknown config command {operation}"
        ))),
    }
}

fn list_nodes(args: &[String]) -> Result<()> {
    if args.iter().any(|arg| arg != "--json") {
        return Err(ForgeError::InvalidArgument(
            "list accepts only --json".into(),
        ));
    }
    let nodes = discover_logitech()?;
    if args.iter().any(|arg| arg == "--json") {
        print_nodes_json(&nodes);
    } else if nodes.is_empty() {
        println!("No Logitech hidraw nodes found.");
    } else {
        for node in nodes {
            println!(
                "{} {:04x}:{:04x} {:?} {}{}",
                node.path.display(),
                node.vendor_id,
                node.product_id,
                node.bus,
                node.name.as_deref().unwrap_or("unknown"),
                node.serial
                    .as_ref()
                    .map_or(String::new(), |serial| format!(" serial={serial}"))
            );
        }
    }
    Ok(())
}

fn dpi_command(mut args: Vec<String>) -> Result<()> {
    let index = take_index(&mut args)?;
    let operation = take_first(&mut args, "dpi requires get or set")?;
    let path = take_first(&mut args, "dpi requires a hidraw path")?;
    match operation.as_str() {
        "get" if args.is_empty() => with_client(Path::new(&path), index, |client| {
            let info = client.dpi()?;
            println!(
                "dpi: {} supported={:?} feature=0x{:04x}",
                info.current, info.supported, info.source_feature
            );
            Ok(())
        }),
        "set" if args.len() == 1 => {
            let value = args[0].parse::<u16>().map_err(|_| {
                ForgeError::InvalidArgument(format!("invalid DPI value {}", args[0]))
            })?;
            with_client(Path::new(&path), index, |client| {
                let info = client.set_dpi(value)?;
                println!(
                    "dpi: {} verified supported={:?}",
                    info.current, info.supported
                );
                Ok(())
            })
        }
        _ => Err(ForgeError::InvalidArgument("invalid dpi command".into())),
    }
}

fn smartshift_command(mut args: Vec<String>) -> Result<()> {
    let index = take_index(&mut args)?;
    let torque = take_torque(&mut args)?;
    let operation = take_first(&mut args, "smartshift requires get or set")?;
    let path = take_first(&mut args, "smartshift requires a hidraw path")?;
    match operation.as_str() {
        "get" if args.is_empty() && torque.is_none() => {
            with_client(Path::new(&path), index, |client| {
                let info = client.smartshift()?;
                println!(
                    "smartshift: mode={:?} threshold={} torque={:?} feature={:?}",
                    info.mode, info.auto_disengage, info.torque, info.feature
                );
                Ok(())
            })
        }
        "set" if args.len() == 1 => {
            let mode = match args[0].as_str() {
                "ratchet" => WheelMode::Ratchet,
                "free" | "free-spin" => WheelMode::FreeSpin,
                other => {
                    return Err(ForgeError::InvalidArgument(format!(
                        "unknown wheel mode {other}"
                    )));
                }
            };
            with_client(Path::new(&path), index, |client| {
                let info = client.set_smartshift(mode, torque)?;
                println!(
                    "smartshift: mode={:?} threshold={} torque={:?} verified",
                    info.mode, info.auto_disengage, info.torque
                );
                Ok(())
            })
        }
        _ => Err(ForgeError::InvalidArgument(
            "invalid smartshift command".into(),
        )),
    }
}

fn with_client(
    path: &Path,
    index: u8,
    operation: impl FnOnce(&mut HidppClient<HidrawTransport>) -> Result<()>,
) -> Result<()> {
    let transport = HidrawTransport::open(path)?;
    let mut client = HidppClient::new(transport, index);
    operation(&mut client)
}

fn one_path(args: &[String]) -> Result<std::path::PathBuf> {
    if args.len() != 1 {
        return Err(ForgeError::InvalidArgument(
            "expected one hidraw path".into(),
        ));
    }
    Ok(args[0].clone().into())
}

fn take_first(args: &mut Vec<String>, message: &str) -> Result<String> {
    if args.is_empty() {
        return Err(ForgeError::InvalidArgument(message.into()));
    }
    Ok(args.remove(0))
}

fn take_optional_value(args: &mut Vec<String>, option: &str) -> Result<Option<String>> {
    let positions = args
        .iter()
        .enumerate()
        .filter_map(|(index, arg)| (arg == option).then_some(index))
        .collect::<Vec<_>>();
    if positions.len() > 1 {
        return Err(ForgeError::InvalidArgument(format!(
            "{option} may only be specified once"
        )));
    }
    let Some(position) = positions.first().copied() else {
        return Ok(None);
    };
    if position + 1 >= args.len() {
        return Err(ForgeError::InvalidArgument(format!(
            "{option} requires a value"
        )));
    }
    let value = args.remove(position + 1);
    args.remove(position);
    Ok(Some(value))
}

fn take_index(args: &mut Vec<String>) -> Result<u8> {
    let Some(position) = args.iter().position(|arg| arg == "--index") else {
        return Ok(DIRECT_DEVICE_INDEX);
    };
    if position + 1 >= args.len() {
        return Err(ForgeError::InvalidArgument(
            "--index requires a value".into(),
        ));
    }
    let value = args.remove(position + 1);
    args.remove(position);
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

fn take_torque(args: &mut Vec<String>) -> Result<Option<u8>> {
    let Some(position) = args.iter().position(|arg| arg == "--torque") else {
        return Ok(None);
    };
    if position + 1 >= args.len() {
        return Err(ForgeError::InvalidArgument(
            "--torque requires a value".into(),
        ));
    }
    let value = args.remove(position + 1);
    args.remove(position);
    let parsed = value
        .parse::<u8>()
        .map_err(|_| ForgeError::InvalidArgument(format!("invalid torque {value}")))?;
    Ok(Some(parsed))
}

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn print_nodes_json(nodes: &[HidNode]) {
    println!("[");
    for (index, node) in nodes.iter().enumerate() {
        let comma = if index + 1 == nodes.len() { "" } else { "," };
        let name = node
            .name
            .as_deref()
            .map_or("null".into(), |value| format!("\"{}\"", json_escape(value)));
        let serial = node
            .serial
            .as_deref()
            .map_or("null".into(), |value| format!("\"{}\"", json_escape(value)));
        println!(
            "  {{\"path\":\"{}\",\"vendor_id\":{},\"product_id\":{},\"bus\":\"{:?}\",\"name\":{},\"serial\":{}}}{}",
            json_escape(&node.path.display().to_string()),
            node.vendor_id,
            node.product_id,
            node.bus,
            name,
            serial,
            comma
        );
    }
    println!("]");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_config_path() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/logi-forge.toml")
            .display()
            .to_string()
    }

    #[test]
    fn capture_options_load_config_and_allow_named_override() {
        let mut args = vec![
            "/dev/input/event7".into(),
            "--config".into(),
            example_config_path(),
            "--bind".into(),
            "Back=Paste".into(),
        ];
        let options = parse_capture_options(&mut args).unwrap();
        let bindings = options.bindings.into_iter().collect::<HashMap<_, _>>();
        assert_eq!(options.event_path, "/dev/input/event7");
        assert_eq!(bindings.get(&KeyCodeForTest::BACK), Some(&Action::Paste));
        assert_eq!(
            bindings.get(&KeyCodeForTest::FORWARD),
            Some(&Action::BrowserForward)
        );
        assert_eq!(options.profiles.len(), 1);
    }

    #[test]
    fn capture_options_reject_unknown_active_button_and_duplicate_config() {
        let mut args = vec![
            "/dev/input/event7".into(),
            "--bind".into(),
            "GestureButton=Copy".into(),
        ];
        assert!(parse_capture_options(&mut args).is_err());

        let path = example_config_path();
        let mut args = vec![
            "/dev/input/event7".into(),
            "--config".into(),
            path.clone(),
            "--config".into(),
            path,
        ];
        assert!(parse_capture_options(&mut args).is_err());
    }

    #[test]
    fn media_bindings_select_only_hidpp_capable_f_row_positions() {
        let config = Config::load_from_path(example_config_path()).unwrap();
        let bindings = media_bindings_from_config(&config)
            .unwrap()
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings.get(&0x00d4), Some(&Action::ShowDesktop));
    }

    struct KeyCodeForTest;

    impl KeyCodeForTest {
        const BACK: u16 = 275;
        const FORWARD: u16 = 276;
    }
}
