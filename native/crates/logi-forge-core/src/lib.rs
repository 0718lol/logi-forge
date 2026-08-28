mod config;

pub use config::*;

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    None,
    BrowserBack,
    BrowserForward,
    MissionControl,
    AppExpose,
    CaptureRegion,
    Copy,
    Paste,
    Undo,
    Redo,
    ShowDesktop,
    CycleDpiPresets,
    ToggleSmartShift,
    PrevTrack,
    PlayPause,
    NextTrack,
    VolumeUp,
    VolumeDown,
    MuteVolume,
    CustomShortcut(String),
    HoldShortcut(String),
}

impl Action {
    /// Parses the stable action labels used by the UI and TOML configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when a shortcut has no key sequence or an unknown action is supplied.
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        let action = match value {
            "None" => Self::None,
            "BrowserBack" => Self::BrowserBack,
            "BrowserForward" => Self::BrowserForward,
            "MissionControl" => Self::MissionControl,
            "AppExpose" => Self::AppExpose,
            "CaptureRegion" => Self::CaptureRegion,
            "Copy" => Self::Copy,
            "Paste" => Self::Paste,
            "Undo" => Self::Undo,
            "Redo" => Self::Redo,
            "ShowDesktop" => Self::ShowDesktop,
            "CycleDpiPresets" => Self::CycleDpiPresets,
            "ToggleSmartShift" => Self::ToggleSmartShift,
            "PrevTrack" => Self::PrevTrack,
            "PlayPause" => Self::PlayPause,
            "NextTrack" => Self::NextTrack,
            "VolumeUp" => Self::VolumeUp,
            "VolumeDown" => Self::VolumeDown,
            "MuteVolume" => Self::MuteVolume,
            custom if custom.strip_prefix("CustomShortcut:").is_some() => {
                Self::CustomShortcut(custom[15..].trim_start().to_owned())
            }
            hold if hold.strip_prefix("HoldShortcut:").is_some() => {
                Self::HoldShortcut(hold[13..].trim_start().to_owned())
            }
            other => {
                return Err(ForgeError::InvalidArgument(format!(
                    "unknown action {other}"
                )));
            }
        };
        if matches!(&action, Self::CustomShortcut(value) | Self::HoldShortcut(value) if value.trim().is_empty())
        {
            return Err(ForgeError::InvalidArgument(
                "shortcut cannot be empty".into(),
            ));
        }
        Ok(action)
    }

    #[must_use]
    pub fn is_hold(&self) -> bool {
        matches!(self, Self::HoldShortcut(_))
    }

    #[must_use]
    pub fn shortcut(&self) -> Option<&str> {
        match self {
            Self::CustomShortcut(value) | Self::HoldShortcut(value) => Some(value),
            _ => None,
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("None"),
            Self::BrowserBack => formatter.write_str("BrowserBack"),
            Self::BrowserForward => formatter.write_str("BrowserForward"),
            Self::MissionControl => formatter.write_str("MissionControl"),
            Self::AppExpose => formatter.write_str("AppExpose"),
            Self::CaptureRegion => formatter.write_str("CaptureRegion"),
            Self::Copy => formatter.write_str("Copy"),
            Self::Paste => formatter.write_str("Paste"),
            Self::Undo => formatter.write_str("Undo"),
            Self::Redo => formatter.write_str("Redo"),
            Self::ShowDesktop => formatter.write_str("ShowDesktop"),
            Self::CycleDpiPresets => formatter.write_str("CycleDpiPresets"),
            Self::ToggleSmartShift => formatter.write_str("ToggleSmartShift"),
            Self::PrevTrack => formatter.write_str("PrevTrack"),
            Self::PlayPause => formatter.write_str("PlayPause"),
            Self::NextTrack => formatter.write_str("NextTrack"),
            Self::VolumeUp => formatter.write_str("VolumeUp"),
            Self::VolumeDown => formatter.write_str("VolumeDown"),
            Self::MuteVolume => formatter.write_str("MuteVolume"),
            Self::CustomShortcut(value) => write!(formatter, "CustomShortcut: {value}"),
            Self::HoldShortcut(value) => write!(formatter, "HoldShortcut: {value}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKey {
    Back,
    Forward,
    Copy,
    Paste,
    Undo,
    Redo,
    ShowDesktop,
    MissionControl,
    AppExpose,
    PlayPause,
    PrevTrack,
    NextTrack,
    VolumeUp,
    VolumeDown,
    Mute,
    Ctrl,
    Shift,
    Alt,
    Meta,
    Space,
    Up,
    Down,
    Key(char),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputCommand {
    Noop,
    Tap(InputKey),
    Chord(Vec<InputKey>),
    HoldChord(Vec<InputKey>),
    Device(DeviceAction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceAction {
    CycleDpiPresets,
    ToggleSmartShift,
}

impl Action {
    /// Resolves an action into a platform-neutral input command.
    ///
    /// # Errors
    ///
    /// Returns an error when a custom shortcut contains an unsupported token.
    pub fn input_command(&self) -> Result<InputCommand> {
        let command = match self {
            Self::None => InputCommand::Noop,
            Self::BrowserBack => InputCommand::Tap(InputKey::Back),
            Self::BrowserForward => InputCommand::Tap(InputKey::Forward),
            Self::MissionControl => InputCommand::Chord(vec![InputKey::Meta, InputKey::Up]),
            Self::AppExpose => InputCommand::Chord(vec![InputKey::Meta, InputKey::Down]),
            Self::CaptureRegion => {
                InputCommand::Chord(vec![InputKey::Meta, InputKey::Shift, InputKey::Key('s')])
            }
            Self::Copy => InputCommand::Chord(vec![InputKey::Ctrl, InputKey::Key('c')]),
            Self::Paste => InputCommand::Chord(vec![InputKey::Ctrl, InputKey::Key('v')]),
            Self::Undo => InputCommand::Chord(vec![InputKey::Ctrl, InputKey::Key('z')]),
            Self::Redo => InputCommand::Chord(vec![InputKey::Ctrl, InputKey::Key('y')]),
            Self::ShowDesktop => InputCommand::Chord(vec![InputKey::Meta, InputKey::Key('d')]),
            Self::CycleDpiPresets => InputCommand::Device(DeviceAction::CycleDpiPresets),
            Self::ToggleSmartShift => InputCommand::Device(DeviceAction::ToggleSmartShift),
            Self::PrevTrack => InputCommand::Tap(InputKey::PrevTrack),
            Self::PlayPause => InputCommand::Tap(InputKey::PlayPause),
            Self::NextTrack => InputCommand::Tap(InputKey::NextTrack),
            Self::VolumeUp => InputCommand::Tap(InputKey::VolumeUp),
            Self::VolumeDown => InputCommand::Tap(InputKey::VolumeDown),
            Self::MuteVolume => InputCommand::Tap(InputKey::Mute),
            Self::CustomShortcut(value) => InputCommand::Chord(parse_shortcut(value)?),
            Self::HoldShortcut(value) => InputCommand::HoldChord(parse_shortcut(value)?),
        };
        Ok(command)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonEvent {
    pub code: u16,
    pub value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dispatch {
    Consume,
    Tap(InputCommand),
    HoldStart(InputCommand),
    HoldEnd(InputCommand),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppIdentity {
    pub id: String,
    pub name: String,
    pub executable: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppProfile {
    pub id: String,
    pub app_id: Option<String>,
    pub executable: Option<String>,
    pub bindings: HashMap<u16, Action>,
}

impl AppProfile {
    /// Creates a profile that applies to one stable application identifier.
    #[must_use]
    pub fn for_app(
        id: impl Into<String>,
        app_id: impl Into<String>,
        bindings: impl IntoIterator<Item = (u16, Action)>,
    ) -> Self {
        Self {
            id: id.into(),
            app_id: Some(app_id.into()),
            executable: None,
            bindings: bindings.into_iter().collect(),
        }
    }

    fn match_score(&self, app: &AppIdentity) -> Option<u8> {
        if self.app_id.is_none() && self.executable.is_none() {
            return None;
        }
        if self.app_id.as_ref().is_some_and(|id| id != &app.id)
            || self
                .executable
                .as_ref()
                .is_some_and(|executable| app.executable.as_ref() != Some(executable))
        {
            return None;
        }
        Some(
            (if self.app_id.is_some() { 4 } else { 0 })
                + if self.executable.is_some() { 2 } else { 0 },
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RouterState {
    foreground: Option<AppIdentity>,
    active_profile: Option<String>,
    bindings: HashMap<u16, Action>,
}

pub struct ActionRouter {
    default_bindings: HashMap<u16, Action>,
    profiles: Vec<AppProfile>,
    state: Mutex<RouterState>,
}

impl ActionRouter {
    #[must_use]
    pub fn new(bindings: impl IntoIterator<Item = (u16, Action)>) -> Self {
        let default_bindings = bindings.into_iter().collect::<HashMap<_, _>>();
        Self {
            state: Mutex::new(RouterState {
                bindings: default_bindings.clone(),
                foreground: None,
                active_profile: None,
            }),
            default_bindings,
            profiles: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_profiles(
        bindings: impl IntoIterator<Item = (u16, Action)>,
        profiles: impl IntoIterator<Item = AppProfile>,
    ) -> Self {
        let mut router = Self::new(bindings);
        router.profiles.extend(profiles);
        router
    }

    fn state(&self) -> MutexGuard<'_, RouterState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Updates the foreground application and atomically swaps effective bindings.
    ///
    /// The highest-specificity matching profile wins: application ID beats
    /// executable name, and a profile matching both beats either-only profiles.
    pub fn set_foreground_app(&self, foreground: Option<AppIdentity>) -> Option<String> {
        let (active_profile, bindings) =
            resolve_bindings(&self.default_bindings, &self.profiles, foreground.as_ref());
        let mut state = self.state();
        state.foreground = foreground;
        state.active_profile.clone_from(&active_profile);
        state.bindings = bindings;
        active_profile
    }

    #[must_use]
    pub fn active_profile_id(&self) -> Option<String> {
        self.state().active_profile.clone()
    }

    #[must_use]
    pub fn foreground_app(&self) -> Option<AppIdentity> {
        self.state().foreground.clone()
    }

    /// Routes one evdev key event and consumes only controls explicitly mapped by the user.
    ///
    /// # Errors
    ///
    /// Returns an error when the mapped action contains an invalid shortcut token.
    pub fn route(&self, event: ButtonEvent) -> Result<Option<Dispatch>> {
        let state = self.state();
        let Some(action) = state.bindings.get(&event.code) else {
            return Ok(None);
        };
        let command = action.input_command()?;
        let dispatch = match event.value {
            1 if action.is_hold() => Dispatch::HoldStart(command),
            1 => Dispatch::Tap(command),
            0 if action.is_hold() => Dispatch::HoldEnd(command),
            _ => Dispatch::Consume,
        };
        Ok(Some(dispatch))
    }
}

fn resolve_bindings(
    defaults: &HashMap<u16, Action>,
    profiles: &[AppProfile],
    foreground: Option<&AppIdentity>,
) -> (Option<String>, HashMap<u16, Action>) {
    let Some(foreground) = foreground else {
        return (None, defaults.clone());
    };
    let selected = profiles
        .iter()
        .enumerate()
        .filter_map(|(index, profile)| {
            profile
                .match_score(foreground)
                .map(|score| (score, index, profile))
        })
        .max_by_key(|(score, index, _)| (*score, std::cmp::Reverse(*index)));
    let Some((_, _, profile)) = selected else {
        return (None, defaults.clone());
    };
    let mut bindings = defaults.clone();
    bindings.extend(profile.bindings.clone());
    (Some(profile.id.clone()), bindings)
}

fn parse_shortcut(value: &str) -> Result<Vec<InputKey>> {
    let mut keys = Vec::new();
    for token in value
        .split('+')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let key = match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => InputKey::Ctrl,
            "shift" => InputKey::Shift,
            "alt" | "option" => InputKey::Alt,
            "cmd" | "command" | "meta" | "super" => InputKey::Meta,
            "space" => InputKey::Space,
            value
                if value.len() == 1
                    && value
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphanumeric()) =>
            {
                InputKey::Key(value.chars().next().unwrap_or_default())
            }
            other => {
                return Err(ForgeError::InvalidArgument(format!(
                    "unsupported shortcut token {other}"
                )));
            }
        };
        keys.push(key);
    }
    if keys.is_empty() {
        return Err(ForgeError::InvalidArgument(
            "shortcut cannot be empty".into(),
        ));
    }
    Ok(keys)
}

pub const LOGITECH_VENDOR_ID: u16 = 0x046d;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HidNode {
    pub path: PathBuf,
    pub vendor_id: u16,
    pub product_id: u16,
    pub bus: HidBus,
    pub name: Option<String>,
    pub serial: Option<String>,
}

impl HidNode {
    #[must_use]
    pub const fn is_logitech(&self) -> bool {
        self.vendor_id == LOGITECH_VENDOR_ID
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HidBus {
    Usb,
    Bluetooth,
    Other(u16),
}

impl From<u16> for HidBus {
    fn from(value: u16) -> Self {
        match value {
            0x0003 => Self::Usb,
            0x0005 => Self::Bluetooth,
            other => Self::Other(other),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryStatus {
    Discharging,
    Charging,
    AlmostFull,
    Full,
    SlowCharging,
    Error(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryInfo {
    pub percentage: u8,
    pub status: BatteryStatus,
    pub source_feature: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DpiInfo {
    pub current: u16,
    pub supported: Vec<u16>,
    pub source_feature: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelMode {
    FreeSpin,
    Ratchet,
}

impl WheelMode {
    #[must_use]
    pub const fn wire_value(self) -> u8 {
        match self {
            Self::FreeSpin => 1,
            Self::Ratchet => 2,
        }
    }
}

impl TryFrom<u8> for WheelMode {
    type Error = ForgeError;

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::FreeSpin),
            2 => Ok(Self::Ratchet),
            _ => Err(ForgeError::InvalidResponse(format!(
                "unknown SmartShift wheel mode {value}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartShiftFeature {
    Legacy,
    Enhanced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmartShiftInfo {
    pub mode: WheelMode,
    pub auto_disengage: u8,
    pub torque: Option<u8>,
    pub feature: SmartShiftFeature,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FnLockFeature {
    MultiHost,
    SingleHost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FnLockInfo {
    pub enabled: bool,
    pub default_enabled: bool,
    pub supports_manual_toggle: bool,
    pub feature: FnLockFeature,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RgbColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardLightingBackend {
    Effects,
    PerKeyV2,
    PerKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyboardLightingInfo {
    pub backend: KeyboardLightingBackend,
    pub zones_written: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReprogControlInfo {
    pub cid: u16,
    pub task_id: u16,
    pub flags: u16,
}

impl ReprogControlInfo {
    #[must_use]
    pub const fn is_divertable(self) -> bool {
        self.flags & (1 << 5) != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReprogControlsInfo {
    pub feature_index: u8,
    pub controls: Vec<ReprogControlInfo>,
}

impl RgbColor {
    /// Parses an RGB color in `RRGGBB` or `#RRGGBB` form.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument error for any other shape or non-hex digit.
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.strip_prefix('#').unwrap_or(value);
        if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ForgeError::InvalidArgument(format!(
                "invalid RGB color {value}; expected RRGGBB"
            )));
        }
        Ok(Self {
            red: u8::from_str_radix(&value[0..2], 16).unwrap_or_default(),
            green: u8::from_str_radix(&value[2..4], 16).unwrap_or_default(),
            blue: u8::from_str_radix(&value[4..6], 16).unwrap_or_default(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureMap {
    pub unified_battery: Option<u8>,
    pub legacy_battery: Option<u8>,
    pub adjustable_dpi: Option<u8>,
    pub smartshift_enhanced: Option<u8>,
    pub smartshift_legacy: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForgeError {
    Io {
        operation: &'static str,
        detail: String,
    },
    PermissionDenied(PathBuf),
    Timeout,
    DeviceNotFound(String),
    FeatureUnsupported(u16),
    InvalidArgument(String),
    ConfigError(String),
    InvalidResponse(String),
    Hidpp {
        code: u8,
        feature_index: u8,
        function: u8,
    },
    VerificationFailed {
        requested: u16,
        actual: u16,
    },
}

impl fmt::Display for ForgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { operation, detail } => write!(formatter, "{operation}: {detail}"),
            Self::PermissionDenied(path) => {
                write!(formatter, "permission denied: {}", path.display())
            }
            Self::Timeout => formatter.write_str("HID++ request timed out"),
            Self::DeviceNotFound(detail) => write!(formatter, "device not found: {detail}"),
            Self::FeatureUnsupported(id) => write!(formatter, "feature 0x{id:04x} is unsupported"),
            Self::InvalidArgument(detail) => write!(formatter, "invalid argument: {detail}"),
            Self::ConfigError(detail) => write!(formatter, "config error: {detail}"),
            Self::InvalidResponse(detail) => write!(formatter, "invalid HID++ response: {detail}"),
            Self::Hidpp {
                code,
                feature_index,
                function,
            } => write!(
                formatter,
                "HID++ error 0x{code:02x} at feature index 0x{feature_index:02x}, function {function}"
            ),
            Self::VerificationFailed { requested, actual } => {
                write!(
                    formatter,
                    "write verification failed: requested {requested}, device reports {actual}"
                )
            }
        }
    }
}

impl std::error::Error for ForgeError {}

pub type Result<T> = std::result::Result<T, ForgeError>;

#[cfg(test)]
mod action_tests {
    use super::{
        Action, ActionRouter, AppIdentity, AppProfile, ButtonEvent, Dispatch, InputCommand,
        InputKey, RgbColor,
    };

    #[test]
    fn parses_and_round_trips_shortcuts() {
        let action = Action::parse("CustomShortcut: Ctrl+Shift+P").unwrap();
        assert_eq!(action.to_string(), "CustomShortcut: Ctrl+Shift+P");
        assert_eq!(
            action.input_command().unwrap(),
            InputCommand::Chord(vec![InputKey::Ctrl, InputKey::Shift, InputKey::Key('p')])
        );
        assert_eq!(
            Action::parse("CustomShortcut:Ctrl+Shift+P").unwrap(),
            action
        );
    }

    #[test]
    fn maps_builtin_actions() {
        assert_eq!(
            Action::Copy.input_command().unwrap(),
            InputCommand::Chord(vec![InputKey::Ctrl, InputKey::Key('c')])
        );
        assert_eq!(
            Action::BrowserBack.input_command().unwrap(),
            InputCommand::Tap(InputKey::Back)
        );
        assert!(Action::parse("Unknown").is_err());
        assert!(Action::parse("CustomShortcut: ").is_err());
        assert_eq!(
            Action::ShowDesktop.input_command().unwrap(),
            InputCommand::Chord(vec![InputKey::Meta, InputKey::Key('d')])
        );
    }

    #[test]
    fn parses_rgb_colors_strictly() {
        assert_eq!(
            RgbColor::parse("#18A06f").unwrap(),
            RgbColor {
                red: 0x18,
                green: 0xa0,
                blue: 0x6f,
            }
        );
        assert!(RgbColor::parse("18a06").is_err());
        assert!(RgbColor::parse("18x06f").is_err());
    }

    #[test]
    fn routes_taps_and_hold_lifecycle_without_touching_unmapped_buttons() {
        let router = ActionRouter::new([
            (275, Action::BrowserBack),
            (276, Action::parse("HoldShortcut: Ctrl+Space").unwrap()),
        ]);
        assert_eq!(
            router
                .route(ButtonEvent {
                    code: 275,
                    value: 1
                })
                .unwrap(),
            Some(Dispatch::Tap(InputCommand::Tap(InputKey::Back)))
        );
        assert_eq!(
            router
                .route(ButtonEvent {
                    code: 276,
                    value: 1
                })
                .unwrap(),
            Some(Dispatch::HoldStart(InputCommand::HoldChord(vec![
                InputKey::Ctrl,
                InputKey::Space,
            ])))
        );
        assert_eq!(
            router
                .route(ButtonEvent {
                    code: 276,
                    value: 0
                })
                .unwrap(),
            Some(Dispatch::HoldEnd(InputCommand::HoldChord(vec![
                InputKey::Ctrl,
                InputKey::Space,
            ])))
        );
        assert_eq!(
            router
                .route(ButtonEvent {
                    code: 274,
                    value: 1
                })
                .unwrap(),
            None
        );
    }

    #[test]
    fn selects_the_most_specific_foreground_profile_and_reverts_to_defaults() {
        let router = ActionRouter::with_profiles(
            [(275, Action::BrowserBack), (276, Action::BrowserForward)],
            [
                AppProfile::for_app("editor", "com.example.Editor", [(275, Action::Copy)]),
                AppProfile {
                    id: "editor-exact".into(),
                    app_id: Some("com.example.Editor".into()),
                    executable: Some("editor".into()),
                    bindings: [(275, Action::Paste)].into_iter().collect(),
                },
            ],
        );
        let app = AppIdentity {
            id: "com.example.Editor".into(),
            name: "Editor".into(),
            executable: Some("editor".into()),
        };
        assert_eq!(
            router.set_foreground_app(Some(app)),
            Some("editor-exact".into())
        );
        assert_eq!(router.active_profile_id(), Some("editor-exact".into()));
        assert_eq!(
            router
                .route(ButtonEvent {
                    code: 275,
                    value: 1
                })
                .unwrap(),
            Some(Dispatch::Tap(InputCommand::Chord(vec![
                InputKey::Ctrl,
                InputKey::Key('v'),
            ])))
        );
        router.set_foreground_app(None);
        assert_eq!(router.active_profile_id(), None);
        assert_eq!(router.foreground_app(), None);
        assert_eq!(
            router
                .route(ButtonEvent {
                    code: 275,
                    value: 1
                })
                .unwrap(),
            Some(Dispatch::Tap(InputCommand::Tap(InputKey::Back)))
        );
    }
}
