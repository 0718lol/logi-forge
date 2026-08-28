use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use evdev::{AttributeSet, Device, EventType, InputEvent, KeyCode};
use logi_forge_core::{
    Action, ActionRouter, AppIdentity, AppProfile, ButtonEvent, Config, Dispatch, ForgeError,
    HidBus, HidNode, InputCommand, InputKey, Result,
};
use logi_forge_hidpp::{HidTransport, LONG_REPORT_ID, LONG_REPORT_LENGTH};

const O_NONBLOCK: i32 = 0x800;

/// Resolves a portable mouse button label or numeric evdev code.
///
/// # Errors
///
/// Returns a configuration error for unknown names and codes outside `u16`.
pub fn resolve_evdev_button_codes(label: &str) -> Result<Vec<u16>> {
    let label = label.trim();
    if let Ok(code) = label.parse::<u16>() {
        return Ok(vec![code]);
    }
    match label.to_ascii_lowercase().as_str() {
        "left" | "leftclick" | "btn_left" => Ok(vec![KeyCode::BTN_LEFT.code()]),
        "right" | "rightclick" | "btn_right" => Ok(vec![KeyCode::BTN_RIGHT.code()]),
        "middle" | "middleclick" | "btn_middle" => Ok(vec![KeyCode::BTN_MIDDLE.code()]),
        "back" => Ok(vec![KeyCode::BTN_SIDE.code(), KeyCode::BTN_BACK.code()]),
        "side" | "btn_side" => Ok(vec![KeyCode::BTN_SIDE.code()]),
        "btn_back" => Ok(vec![KeyCode::BTN_BACK.code()]),
        "forward" => Ok(vec![KeyCode::BTN_EXTRA.code(), KeyCode::BTN_FORWARD.code()]),
        "extra" | "btn_extra" => Ok(vec![KeyCode::BTN_EXTRA.code()]),
        "btn_forward" => Ok(vec![KeyCode::BTN_FORWARD.code()]),
        "dpitoggle" | "task" | "btn_task" => Ok(vec![KeyCode::BTN_TASK.code()]),
        _ => Err(ForgeError::ConfigError(format!(
            "unknown Linux button {label}; use Back, Forward, DpiToggle, MiddleClick, LeftClick, RightClick, BTN_* or a numeric evdev code"
        ))),
    }
}

/// Resolves an F-row key label for Linux keyboard capture.
///
/// # Errors
///
/// Returns a configuration error for labels outside F1-F12 and invalid numeric codes.
pub fn resolve_evdev_keyboard_codes(label: &str) -> Result<Vec<u16>> {
    let label = label.trim();
    if let Ok(code) = label.parse::<u16>() {
        return Ok(vec![code]);
    }
    let code = match label.to_ascii_uppercase().as_str() {
        "F1" => KeyCode::KEY_F1,
        "F2" => KeyCode::KEY_F2,
        "F3" => KeyCode::KEY_F3,
        "F4" => KeyCode::KEY_F4,
        "F5" => KeyCode::KEY_F5,
        "F6" => KeyCode::KEY_F6,
        "F7" => KeyCode::KEY_F7,
        "F8" => KeyCode::KEY_F8,
        "F9" => KeyCode::KEY_F9,
        "F10" => KeyCode::KEY_F10,
        "F11" => KeyCode::KEY_F11,
        "F12" => KeyCode::KEY_F12,
        _ => {
            return Err(ForgeError::ConfigError(format!(
                "unknown Linux keyboard key {label}; use F1..F12 or a numeric evdev code"
            )));
        }
    };
    Ok(vec![code.code()])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureRouting {
    pub bindings: Vec<(u16, Action)>,
    pub profiles: Vec<AppProfile>,
}

/// Builds the Linux action router inputs for the selected configured device.
///
/// Known bindings set to `None` remain captured and suppress their physical
/// input. Unknown vendor-specific controls set to `None` are ignored.
///
/// # Errors
///
/// Returns a configuration error when capture is disabled, the selected device
/// is disabled, or an active binding cannot be mapped to an evdev button code.
pub fn capture_routing_from_config(config: &Config) -> Result<CaptureRouting> {
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

    let bindings = resolve_config_bindings(&device.bindings)?;
    let profiles = device
        .profiles
        .iter()
        .map(|(id, profile)| {
            Ok(AppProfile {
                id: id.clone(),
                app_id: profile.app_id.clone(),
                executable: profile.executable.clone(),
                bindings: resolve_config_bindings(&profile.bindings)?
                    .into_iter()
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(CaptureRouting { bindings, profiles })
}

/// Builds F-row capture bindings from the selected device's keyboard section.
///
/// # Errors
///
/// Returns a configuration error when capture or the selected device is
/// disabled, no keyboard section exists, or a key label is unsupported.
pub fn keyboard_capture_routing_from_config(config: &Config) -> Result<CaptureRouting> {
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
        ForgeError::ConfigError(format!(
            "selected device {} has no keyboard configuration",
            config.selected_device
        ))
    })?;
    let bindings = resolve_bindings_with(&keyboard.bindings, resolve_evdev_keyboard_codes)?;
    Ok(CaptureRouting {
        bindings,
        profiles: Vec::new(),
    })
}

fn resolve_config_bindings(
    bindings: &std::collections::BTreeMap<String, Action>,
) -> Result<Vec<(u16, Action)>> {
    resolve_bindings_with(bindings, resolve_evdev_button_codes)
}

fn resolve_bindings_with(
    bindings: &std::collections::BTreeMap<String, Action>,
    key_resolver: fn(&str) -> Result<Vec<u16>>,
) -> Result<Vec<(u16, Action)>> {
    let mut routes = HashMap::new();
    for (label, action) in bindings {
        let codes = match key_resolver(label) {
            Ok(codes) => codes,
            Err(_) if *action == Action::None => continue,
            Err(error) => return Err(error),
        };
        for code in codes {
            if routes.insert(code, action.clone()).is_some() {
                return Err(ForgeError::ConfigError(format!(
                    "multiple bindings resolve to evdev button code {code}"
                )));
            }
        }
    }
    Ok(routes.into_iter().collect())
}

pub trait ForegroundAppProvider: Send {
    /// Returns the current foreground application, if the desktop exposes one.
    ///
    /// # Errors
    ///
    /// Returns a typed process or desktop integration error.
    fn current(&mut self) -> Result<Option<AppIdentity>>;
}

#[derive(Default)]
pub struct XdotoolForegroundProvider;

impl ForegroundAppProvider for XdotoolForegroundProvider {
    fn current(&mut self) -> Result<Option<AppIdentity>> {
        let output = match Command::new("xdotool")
            .args(["getactivewindow", "getwindowpid"])
            .output()
        {
            Ok(output) => output,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(ForgeError::Io {
                    operation: "query foreground X11 window",
                    detail: error.to_string(),
                });
            }
        };
        if !output.status.success() {
            return Ok(None);
        }
        let pid = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u32>()
            .map_err(|error| {
                ForgeError::InvalidResponse(format!("xdotool returned invalid PID: {error}"))
            })?;
        process_identity(pid).map(Some)
    }
}

/// Queries the desktop foreground application through the available Linux X11 adapter.
///
/// # Errors
///
/// Returns a process metadata error when the foreground PID cannot be inspected.
pub fn foreground_app() -> Result<Option<AppIdentity>> {
    let mut provider = XdotoolForegroundProvider;
    provider.current()
}

fn process_identity(pid: u32) -> Result<AppIdentity> {
    let exe_path =
        std::fs::read_link(format!("/proc/{pid}/exe")).map_err(|error| ForgeError::Io {
            operation: "read foreground executable",
            detail: error.to_string(),
        })?;
    let executable = exe_path.display().to_string();
    let name = std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .unwrap_or_else(|_| {
            exe_path.file_name().map_or_else(
                || "unknown".into(),
                |name| name.to_string_lossy().into_owned(),
            )
        })
        .trim()
        .to_owned();
    Ok(AppIdentity {
        id: executable.clone(),
        name,
        executable: Some(executable),
    })
}

/// Enumerates all Linux hidraw nodes using sysfs metadata.
///
/// # Errors
///
/// Returns an error when `/sys/class/hidraw` cannot be read.
pub fn discover() -> Result<Vec<HidNode>> {
    discover_under(Path::new("/sys/class/hidraw"), Path::new("/dev"))
}

/// Enumerates Linux hidraw nodes whose vendor ID is Logitech `046d`.
///
/// # Errors
///
/// Returns an error when sysfs enumeration fails.
pub fn discover_logitech() -> Result<Vec<HidNode>> {
    Ok(discover()?
        .into_iter()
        .filter(HidNode::is_logitech)
        .collect())
}

/// Enumerates hidraw metadata under injectable roots for host use and tests.
///
/// # Errors
///
/// Returns an error when the sysfs root or one of its directory entries cannot be read.
pub fn discover_under(sys_class: &Path, dev_root: &Path) -> Result<Vec<HidNode>> {
    let entries = match fs::read_dir(sys_class) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ForgeError::Io {
                operation: "enumerate hidraw sysfs",
                detail: error.to_string(),
            });
        }
    };
    let mut nodes = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| ForgeError::Io {
            operation: "read hidraw sysfs entry",
            detail: error.to_string(),
        })?;
        let name = entry.file_name();
        let uevent_path = entry.path().join("device/uevent");
        let Ok(contents) = fs::read_to_string(uevent_path) else {
            continue;
        };
        if let Some(node) = parse_uevent(&contents, dev_root.join(name)) {
            nodes.push(node);
        }
    }
    nodes.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(nodes)
}

fn parse_uevent(contents: &str, path: PathBuf) -> Option<HidNode> {
    let values: HashMap<&str, &str> = contents
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    let mut id = values.get("HID_ID")?.split(':');
    let bus = u16::from_str_radix(id.next()?, 16).ok()?;
    let vendor_id = u16::try_from(u32::from_str_radix(id.next()?, 16).ok()?).ok()?;
    let product_id = u16::try_from(u32::from_str_radix(id.next()?, 16).ok()?).ok()?;
    Some(HidNode {
        path,
        vendor_id,
        product_id,
        bus: HidBus::from(bus),
        name: values.get("HID_NAME").map(|value| (*value).to_owned()),
        serial: values
            .get("HID_UNIQ")
            .filter(|value| !value.is_empty())
            .map(|value| (*value).to_owned()),
    })
}

pub struct HidrawTransport {
    path: PathBuf,
    file: File,
    long_reports_only: bool,
}

pub struct LinuxInputInjector {
    device: evdev::uinput::VirtualDevice,
}

impl LinuxInputInjector {
    /// Creates a virtual keyboard/media device used for remapped controls.
    ///
    /// # Errors
    ///
    /// Returns a permission or kernel I/O error when `/dev/uinput` is unavailable.
    pub fn open() -> Result<Self> {
        let mut builder = evdev::uinput::VirtualDevice::builder()
            .map_err(|error| input_error("open uinput", &error))?
            .name("Logi Forge Virtual Controls");
        let mut keys = AttributeSet::<KeyCode>::new();
        for key in supported_virtual_keys() {
            keys.insert(key);
        }
        builder = builder
            .with_keys(&keys)
            .map_err(|error| input_error("configure uinput keys", &error))?;
        Self::build(builder)
    }

    fn build(builder: evdev::uinput::VirtualDeviceBuilder<'_>) -> Result<Self> {
        let device = builder
            .build()
            .map_err(|error| input_error("create uinput device", &error))?;
        Ok(Self { device })
    }

    fn from_evdev(source: &Device) -> Result<Self> {
        let mut builder = evdev::uinput::VirtualDevice::builder()
            .map_err(|error| input_error("open uinput", &error))?
            .name("Logi Forge Remapped Mouse");
        let mut keys = AttributeSet::<KeyCode>::new();
        if let Some(source_keys) = source.supported_keys() {
            for key in source_keys {
                keys.insert(key);
            }
        }
        for key in supported_virtual_keys() {
            keys.insert(key);
        }
        builder = builder
            .with_keys(&keys)
            .map_err(|error| input_error("configure passthrough keys", &error))?;
        if let Some(axes) = source.supported_relative_axes() {
            builder = builder
                .with_relative_axes(axes)
                .map_err(|error| input_error("configure passthrough axes", &error))?;
        }
        Self::build(builder)
    }

    /// Emits one routed action, including the release half of taps and hold chords.
    ///
    /// # Errors
    ///
    /// Returns a kernel I/O error when the virtual device cannot accept events.
    pub fn dispatch(&mut self, dispatch: &Dispatch) -> Result<()> {
        match dispatch {
            Dispatch::Consume => Ok(()),
            Dispatch::Tap(command) => self.tap(command),
            Dispatch::HoldStart(command) => self.chord(command, 1),
            Dispatch::HoldEnd(command) => self.chord(command, 0),
        }
    }

    fn tap(&mut self, command: &InputCommand) -> Result<()> {
        match command {
            InputCommand::Noop => Ok(()),
            InputCommand::Tap(key) => {
                self.emit_keys(&[*key], 1)?;
                self.emit_keys(&[*key], 0)
            }
            InputCommand::Chord(keys) | InputCommand::HoldChord(keys) => {
                self.emit_keys(keys, 1)?;
                let mut release = keys.clone();
                release.reverse();
                self.emit_keys(&release, 0)
            }
            InputCommand::Device(action) => Err(ForgeError::InvalidArgument(format!(
                "device action {action:?} must be dispatched by the resident agent"
            ))),
        }
    }

    fn chord(&mut self, command: &InputCommand, value: i32) -> Result<()> {
        match command {
            InputCommand::Noop => Ok(()),
            InputCommand::Tap(key) => self.emit_keys(&[*key], value),
            InputCommand::Chord(keys) | InputCommand::HoldChord(keys) => {
                self.emit_keys(keys, value)
            }
            InputCommand::Device(action) => Err(ForgeError::InvalidArgument(format!(
                "device action {action:?} must be dispatched by the resident agent"
            ))),
        }
    }

    fn emit_keys(&mut self, keys: &[InputKey], value: i32) -> Result<()> {
        let events: Vec<_> = keys
            .iter()
            .map(|key| InputEvent::new(EventType::KEY.0, key_code(*key).code(), value))
            .collect();
        self.device
            .emit(&events)
            .map_err(|error| input_error("emit virtual input", &error))
    }

    fn emit_raw(&mut self, events: &[InputEvent]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        self.device
            .emit(events)
            .map_err(|error| input_error("forward physical input", &error))
    }
}

fn supported_virtual_keys() -> Vec<KeyCode> {
    [
        InputKey::Back,
        InputKey::Forward,
        InputKey::Copy,
        InputKey::Paste,
        InputKey::Undo,
        InputKey::Redo,
        InputKey::PlayPause,
        InputKey::PrevTrack,
        InputKey::NextTrack,
        InputKey::VolumeUp,
        InputKey::VolumeDown,
        InputKey::Mute,
        InputKey::Ctrl,
        InputKey::Shift,
        InputKey::Alt,
        InputKey::Meta,
        InputKey::Space,
        InputKey::Up,
        InputKey::Down,
        InputKey::Key('a'),
        InputKey::Key('b'),
        InputKey::Key('c'),
        InputKey::Key('d'),
        InputKey::Key('e'),
        InputKey::Key('f'),
        InputKey::Key('g'),
        InputKey::Key('h'),
        InputKey::Key('i'),
        InputKey::Key('j'),
        InputKey::Key('k'),
        InputKey::Key('l'),
        InputKey::Key('m'),
        InputKey::Key('n'),
        InputKey::Key('o'),
        InputKey::Key('p'),
        InputKey::Key('q'),
        InputKey::Key('r'),
        InputKey::Key('s'),
        InputKey::Key('t'),
        InputKey::Key('w'),
        InputKey::Key('x'),
        InputKey::Key('u'),
        InputKey::Key('v'),
        InputKey::Key('y'),
        InputKey::Key('z'),
    ]
    .into_iter()
    .map(key_code)
    .collect()
}

fn key_code(key: InputKey) -> KeyCode {
    match key {
        InputKey::Back => KeyCode::KEY_BACK,
        InputKey::Forward => KeyCode::KEY_FORWARD,
        InputKey::Copy => KeyCode::KEY_COPY,
        InputKey::Paste => KeyCode::KEY_PASTE,
        InputKey::Undo => KeyCode::KEY_UNDO,
        InputKey::Redo => KeyCode::KEY_REDO,
        InputKey::PlayPause => KeyCode::KEY_PLAYPAUSE,
        InputKey::PrevTrack => KeyCode::KEY_PREVIOUSSONG,
        InputKey::NextTrack => KeyCode::KEY_NEXTSONG,
        InputKey::VolumeUp => KeyCode::KEY_VOLUMEUP,
        InputKey::VolumeDown => KeyCode::KEY_VOLUMEDOWN,
        InputKey::Mute => KeyCode::KEY_MUTE,
        InputKey::Ctrl => KeyCode::KEY_LEFTCTRL,
        InputKey::Shift => KeyCode::KEY_LEFTSHIFT,
        InputKey::Alt => KeyCode::KEY_LEFTALT,
        InputKey::Meta => KeyCode::KEY_LEFTMETA,
        InputKey::Space => KeyCode::KEY_SPACE,
        InputKey::Up => KeyCode::KEY_UP,
        InputKey::Down => KeyCode::KEY_DOWN,
        InputKey::Key('a') => KeyCode::KEY_A,
        InputKey::Key('b') => KeyCode::KEY_B,
        InputKey::Key('c') => KeyCode::KEY_C,
        InputKey::Key('d') => KeyCode::KEY_D,
        InputKey::Key('e') => KeyCode::KEY_E,
        InputKey::Key('f') => KeyCode::KEY_F,
        InputKey::Key('g') => KeyCode::KEY_G,
        InputKey::Key('h') => KeyCode::KEY_H,
        InputKey::Key('i') => KeyCode::KEY_I,
        InputKey::Key('j') => KeyCode::KEY_J,
        InputKey::Key('k') => KeyCode::KEY_K,
        InputKey::Key('l') => KeyCode::KEY_L,
        InputKey::Key('m') => KeyCode::KEY_M,
        InputKey::Key('n') => KeyCode::KEY_N,
        InputKey::Key('o') => KeyCode::KEY_O,
        InputKey::Key('p') => KeyCode::KEY_P,
        InputKey::Key('q') => KeyCode::KEY_Q,
        InputKey::Key('r') => KeyCode::KEY_R,
        InputKey::Key('s') => KeyCode::KEY_S,
        InputKey::Key('t') => KeyCode::KEY_T,
        InputKey::Key('u') => KeyCode::KEY_U,
        InputKey::Key('v') => KeyCode::KEY_V,
        InputKey::Key('w') => KeyCode::KEY_W,
        InputKey::Key('x') => KeyCode::KEY_X,
        InputKey::Key('y') => KeyCode::KEY_Y,
        InputKey::Key('z') => KeyCode::KEY_Z,
        InputKey::ShowDesktop | InputKey::MissionControl | InputKey::AppExpose => {
            KeyCode::KEY_RESERVED
        }
        InputKey::Key(_) => KeyCode::KEY_RESERVED,
    }
}

fn input_error(operation: &'static str, error: &std::io::Error) -> ForgeError {
    if error.kind() == ErrorKind::PermissionDenied {
        ForgeError::PermissionDenied(PathBuf::from("/dev/uinput or /dev/input"))
    } else {
        ForgeError::Io {
            operation,
            detail: error.to_string(),
        }
    }
}

/// Captures a grabbed evdev device and routes mapped key events to an injector.
///
/// # Errors
///
/// Returns a typed permission or I/O error when the event device cannot be opened, grabbed, or read.
pub fn capture_events(path: impl AsRef<Path>, router: &ActionRouter) -> Result<()> {
    capture_events_inner(path.as_ref(), router)
}

/// Captures events while polling the foreground application for profile changes.
///
/// # Errors
///
/// Returns a typed permission or I/O error when the input device, watcher, or virtual device fails.
pub fn capture_events_with_foreground(
    path: impl AsRef<Path>,
    router: &Arc<ActionRouter>,
) -> Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    let watcher_stop = Arc::clone(&stop);
    let watcher_router = Arc::clone(router);
    let watcher = thread::spawn(move || {
        let mut provider = XdotoolForegroundProvider;
        while !watcher_stop.load(Ordering::Relaxed) {
            if let Ok(app) = provider.current() {
                watcher_router.set_foreground_app(app);
            }
            thread::sleep(Duration::from_millis(150));
        }
    });
    let result = capture_events_inner(path.as_ref(), router);
    stop.store(true, Ordering::Relaxed);
    let _ = watcher.join();
    result
}

fn capture_events_inner(path: &Path, router: &ActionRouter) -> Result<()> {
    let path = path.to_path_buf();
    let mut device =
        Device::open(&path).map_err(|error| input_error("open evdev device", &error))?;
    device
        .grab()
        .map_err(|error| input_error("grab evdev device", &error))?;
    let mut injector = match LinuxInputInjector::from_evdev(&device) {
        Ok(injector) => injector,
        Err(error) => {
            let _ = device.ungrab();
            return Err(error);
        }
    };
    let result = capture_grabbed(&mut device, router, &mut injector);
    let _ = device.ungrab();
    result
}

fn capture_grabbed(
    device: &mut Device,
    router: &ActionRouter,
    injector: &mut LinuxInputInjector,
) -> Result<()> {
    loop {
        let events = device
            .fetch_events()
            .map_err(|error| input_error("read evdev events", &error))?;
        let mut passthrough = Vec::new();
        for event in events {
            let is_sync = event.event_type() == EventType::SYNCHRONIZATION && event.code() == 0;
            if event.event_type() == EventType::KEY {
                if let Some(dispatch) = router.route(ButtonEvent {
                    code: event.code(),
                    value: event.value(),
                })? {
                    injector.dispatch(&dispatch)?;
                } else {
                    passthrough.push(event);
                }
            } else if !is_sync {
                passthrough.push(event);
            }
            if is_sync {
                injector.emit_raw(&passthrough)?;
                passthrough.clear();
            }
        }
    }
}

impl HidrawTransport {
    /// Opens a hidraw node for non-blocking request/response transactions.
    ///
    /// # Errors
    ///
    /// Returns a typed permission or I/O error when the node cannot be opened.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_NONBLOCK)
            .open(&path)
            .map_err(|error| match error.kind() {
                ErrorKind::PermissionDenied => ForgeError::PermissionDenied(path.clone()),
                _ => ForgeError::Io {
                    operation: "open hidraw device",
                    detail: error.to_string(),
                },
            })?;
        Ok(Self {
            path,
            file,
            long_reports_only: false,
        })
    }

    fn write_report(&mut self, request: &[u8]) -> Result<()> {
        match self.file.write_all(request) {
            Ok(()) => Ok(()),
            Err(error)
                if error.kind() == ErrorKind::InvalidInput
                    && request.len() < LONG_REPORT_LENGTH =>
            {
                let mut widened = [0; LONG_REPORT_LENGTH];
                widened[0] = LONG_REPORT_ID;
                widened[1..request.len()].copy_from_slice(&request[1..]);
                self.file
                    .write_all(&widened)
                    .map_err(|retry| ForgeError::Io {
                        operation: "write widened HID++ report",
                        detail: retry.to_string(),
                    })?;
                self.long_reports_only = true;
                Ok(())
            }
            Err(error) => Err(ForgeError::Io {
                operation: "write HID++ report",
                detail: error.to_string(),
            }),
        }
    }
}

impl HidTransport for HidrawTransport {
    fn transact(&mut self, request: &[u8], timeout: Duration) -> Result<Vec<u8>> {
        let mut outgoing = request.to_vec();
        if self.long_reports_only && outgoing.len() < LONG_REPORT_LENGTH {
            outgoing.resize(LONG_REPORT_LENGTH, 0);
            outgoing[0] = LONG_REPORT_ID;
        }
        self.write_report(&outgoing)?;
        let deadline = Instant::now() + timeout;
        let mut buffer = [0; 64];
        loop {
            match self.file.read(&mut buffer) {
                Ok(0) => {}
                Ok(size) if response_matches(request, &buffer[..size]) => {
                    return Ok(buffer[..size].to_vec());
                }
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => {
                    return Err(ForgeError::Io {
                        operation: "read HID++ report",
                        detail: format!("{}: {error}", self.path.display()),
                    });
                }
            }
            if Instant::now() >= deadline {
                return Err(ForgeError::Timeout);
            }
            thread::sleep(Duration::from_millis(4));
        }
    }

    fn write_report(&mut self, report: &[u8]) -> Result<()> {
        HidrawTransport::write_report(self, report)
    }

    fn read_report(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let deadline = Instant::now() + timeout;
        let mut buffer = [0; 64];
        loop {
            match self.file.read(&mut buffer) {
                Ok(0) => {}
                Ok(size) => return Ok(buffer[..size].to_vec()),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => {
                    return Err(ForgeError::Io {
                        operation: "read unsolicited HID++ report",
                        detail: format!("{}: {error}", self.path.display()),
                    });
                }
            }
            if Instant::now() >= deadline {
                return Err(ForgeError::Timeout);
            }
            thread::sleep(Duration::from_millis(4));
        }
    }
}

fn response_matches(request: &[u8], response: &[u8]) -> bool {
    if request.len() < 4 || response.len() < 4 || response[1] != request[1] {
        return false;
    }
    let normal = response[2] == request[2] && response[3] == request[3];
    let error = response.len() >= 6
        && response[2] == 0xff
        && response[3] == request[2]
        && response[4] == request[3];
    normal || error
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs::{create_dir_all, remove_dir_all, write};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn parses_sysfs_inventory_and_filters_logitech() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("logi-forge-{suffix}"));
        let sys = root.join("sys");
        let dev = root.join("dev");
        create_dir_all(sys.join("hidraw7/device")).unwrap();
        create_dir_all(&dev).unwrap();
        write(
            sys.join("hidraw7/device/uevent"),
            "HID_ID=0003:0000046D:0000C548\nHID_NAME=Logitech USB Receiver\nHID_UNIQ=abc123\n",
        )
        .unwrap();

        let nodes = discover_under(&sys, &dev).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].path, dev.join("hidraw7"));
        assert_eq!(nodes[0].vendor_id, 0x046d);
        assert_eq!(nodes[0].product_id, 0xc548);
        assert_eq!(nodes[0].bus, HidBus::Usb);
        assert!(nodes[0].is_logitech());
        remove_dir_all(root).unwrap();
    }

    #[test]
    fn matches_normal_and_error_responses_but_ignores_events() {
        let request = [0x10, 0xff, 5, 0x21, 0, 0, 0];
        assert!(response_matches(&request, &[0x11, 0xff, 5, 0x21, 0, 0, 0]));
        assert!(response_matches(
            &request,
            &[0x11, 0xff, 0xff, 5, 0x21, 9, 0]
        ));
        assert!(!response_matches(&request, &[0x11, 0xff, 5, 0x20, 1, 2, 3]));
    }

    #[test]
    fn missing_hidraw_class_is_an_empty_inventory() {
        let missing = std::env::temp_dir().join("logi-forge-definitely-missing");
        assert!(
            discover_under(&missing, Path::new("/dev"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn resolves_portable_and_numeric_mouse_button_labels() {
        assert_eq!(resolve_evdev_button_codes("Back").unwrap(), vec![275, 278]);
        assert_eq!(resolve_evdev_button_codes("BTN_EXTRA").unwrap(), vec![276]);
        assert_eq!(resolve_evdev_button_codes("274").unwrap(), vec![274]);
        assert_eq!(resolve_evdev_button_codes("DpiToggle").unwrap(), vec![279]);
    }

    #[test]
    fn builds_default_and_profile_routes_from_config() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/logi-forge.toml");
        let config = Config::load_from_path(path).unwrap();
        let routing = capture_routing_from_config(&config).unwrap();
        let bindings = routing.bindings.into_iter().collect::<HashMap<_, _>>();
        assert_eq!(bindings.get(&275), Some(&Action::BrowserBack));
        assert_eq!(bindings.get(&278), Some(&Action::BrowserBack));
        assert_eq!(bindings.get(&276), Some(&Action::BrowserForward));
        assert_eq!(
            bindings.get(&274),
            Some(&Action::HoldShortcut("Ctrl+Space".into()))
        );
        assert_eq!(bindings.get(&279), Some(&Action::None));
        assert_eq!(routing.profiles.len(), 1);
        assert_eq!(routing.profiles[0].bindings.get(&275), Some(&Action::Copy));
    }

    #[test]
    fn rejects_multiple_names_for_the_same_active_button() {
        let bindings = BTreeMap::from([
            ("Back".into(), Action::Copy),
            ("BTN_SIDE".into(), Action::Paste),
        ]);
        assert!(resolve_config_bindings(&bindings).is_err());
    }

    #[test]
    fn builds_f_row_capture_routes_from_keyboard_config() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/logi-forge.toml");
        let config = Config::load_from_path(path).unwrap();
        let routing = keyboard_capture_routing_from_config(&config).unwrap();
        let bindings = routing.bindings.into_iter().collect::<HashMap<_, _>>();
        assert_eq!(
            bindings.get(&KeyCode::KEY_F1.code()),
            Some(&Action::MissionControl)
        );
        assert_eq!(
            bindings.get(&KeyCode::KEY_F4.code()),
            Some(&Action::ShowDesktop)
        );
        assert!(routing.profiles.is_empty());
        assert_eq!(resolve_evdev_keyboard_codes("F12").unwrap(), vec![88]);
        assert!(resolve_evdev_keyboard_codes("F13").is_err());
    }
}
