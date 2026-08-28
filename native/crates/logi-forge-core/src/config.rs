use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{Action, ForgeError, Result};

pub const CURRENT_CONFIG_SCHEMA_VERSION: u32 = 1;

const DEVICE_NAME_LIMIT: usize = 80;
const ACTION_RING_SLOTS: [&str; 8] = [
    "Top",
    "TopRight",
    "Right",
    "BottomRight",
    "Bottom",
    "BottomLeft",
    "Left",
    "TopLeft",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub selected_device: String,
    #[serde(default)]
    pub app_settings: AppSettings,
    #[serde(default)]
    pub devices: BTreeMap<String, DeviceConfig>,
}

impl Config {
    /// Loads a strict TOML config from disk.
    ///
    /// # Errors
    ///
    /// Returns a configuration error when the file cannot be read, parsed, or
    /// validated against the current schema.
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path).map_err(|error| ForgeError::Io {
            operation: "read config",
            detail: format!("{}: {error}", path.display()),
        })?;
        Self::from_toml_str(&contents)
    }

    /// Parses and validates a strict TOML config document.
    ///
    /// # Errors
    ///
    /// Returns a configuration error when the document is malformed, contains
    /// unknown fields, or violates schema validation rules.
    pub fn from_toml_str(contents: &str) -> Result<Self> {
        let config = toml::from_str::<Self>(contents)
            .map_err(|error| ForgeError::ConfigError(error.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Serializes the config back to canonical TOML.
    ///
    /// # Errors
    ///
    /// Returns a configuration error if validation fails or TOML serialization
    /// fails unexpectedly.
    pub fn to_toml_string(&self) -> Result<String> {
        self.validate()?;
        toml::to_string_pretty(self).map_err(|error| ForgeError::ConfigError(error.to_string()))
    }

    /// Validates schema version, selected device ownership, and device fields.
    ///
    /// # Errors
    ///
    /// Returns a configuration error when any field violates the strict schema.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CURRENT_CONFIG_SCHEMA_VERSION {
            return Err(ForgeError::ConfigError(format!(
                "unsupported schema version {}",
                self.schema_version
            )));
        }
        if self.selected_device.trim().is_empty() {
            return Err(ForgeError::ConfigError(
                "selected_device cannot be empty".into(),
            ));
        }
        if self.devices.is_empty() {
            return Err(ForgeError::ConfigError(
                "configuration must define at least one device".into(),
            ));
        }
        let Some(selected) = self.devices.get(&self.selected_device) else {
            return Err(ForgeError::ConfigError(format!(
                "selected_device {} is missing from devices",
                self.selected_device
            )));
        };
        validate_device(&self.selected_device, selected)?;
        for (device_key, device) in &self.devices {
            validate_device(device_key, device)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSettings {
    #[serde(default = "default_true")]
    pub show_in_menu_bar: bool,
    #[serde(default = "default_true")]
    pub capture_mouse_events: bool,
    #[serde(default)]
    pub device_view_mode: DeviceViewMode,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            show_in_menu_bar: true,
            capture_mouse_events: true,
            device_view_mode: DeviceViewMode::Grid,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceViewMode {
    #[default]
    Grid,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScrollResolution {
    High,
    Low,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CameraProfile {
    Default,
    Streaming,
    #[serde(rename = "Video call")]
    VideoCall,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceConfig {
    pub custom_name: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dpi: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dpi_presets: Option<Vec<u16>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invert_scroll: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_resolution: Option<ScrollResolution>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bindings: BTreeMap<String, Action>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, ProfileConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_ring: Option<ActionRingConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard: Option<KeyboardConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<CameraConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<LightConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bindings: BTreeMap<String, Action>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyboardConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fn_lock: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lighting: Option<KeyboardLightingConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bindings: BTreeMap<String, Action>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyboardLightingConfig {
    pub color: String,
    pub brightness: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionRingConfig {
    pub default: ActionRingLayout,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionRingLayout {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub slots: BTreeMap<String, ActionRingSlot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionRingSlot {
    pub action: Action,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraConfig {
    pub profile: CameraProfile,
    pub zoom: u16,
    pub focus_auto: bool,
    pub exposure_auto: bool,
    pub brightness: u8,
    pub contrast: u8,
    pub saturation: u8,
    pub white_balance: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightConfig {
    pub power: bool,
    pub brightness: u8,
    pub temperature: u16,
    pub auto_power: bool,
}

fn validate_device(device_key: &str, device: &DeviceConfig) -> Result<()> {
    if device_key.trim().is_empty() {
        return Err(ForgeError::ConfigError("device key cannot be empty".into()));
    }
    if device.custom_name.trim().is_empty() || device.custom_name.len() > DEVICE_NAME_LIMIT {
        return Err(ForgeError::ConfigError(format!(
            "device {device_key} custom_name must be 1..={DEVICE_NAME_LIMIT} characters"
        )));
    }
    if let Some(dpi) = device.dpi
        && !(200..=8000).contains(&dpi)
    {
        return Err(ForgeError::ConfigError(format!(
            "device {device_key} dpi {dpi} is out of range"
        )));
    }
    if let Some(presets) = &device.dpi_presets {
        if presets.is_empty() {
            return Err(ForgeError::ConfigError(format!(
                "device {device_key} dpi_presets cannot be empty"
            )));
        }
        if presets.iter().any(|value| !(200..=8000).contains(value)) {
            return Err(ForgeError::ConfigError(format!(
                "device {device_key} dpi_presets contain out-of-range values"
            )));
        }
    }
    for binding_name in device.bindings.keys() {
        if binding_name.trim().is_empty() {
            return Err(ForgeError::ConfigError(format!(
                "device {device_key} contains an empty binding name"
            )));
        }
    }
    validate_profiles(device_key, &device.profiles)?;
    if let Some(action_ring) = &device.action_ring {
        validate_action_ring(device_key, action_ring)?;
    }
    if let Some(keyboard) = &device.keyboard {
        for binding_name in keyboard.bindings.keys() {
            if binding_name.trim().is_empty() {
                return Err(ForgeError::ConfigError(format!(
                    "device {device_key} contains an empty keyboard binding name"
                )));
            }
        }
        if let Some(lighting) = &keyboard.lighting {
            if lighting.brightness > 100 {
                return Err(ForgeError::ConfigError(format!(
                    "device {device_key} keyboard lighting brightness must be 0..=100"
                )));
            }
            super::RgbColor::parse(&lighting.color).map_err(|_| {
                ForgeError::ConfigError(format!(
                    "device {device_key} keyboard lighting color must be RRGGBB"
                ))
            })?;
        }
    }
    if let Some(camera) = &device.camera {
        if !(100..=500).contains(&camera.zoom) {
            return Err(ForgeError::ConfigError(format!(
                "device {device_key} camera zoom is out of range"
            )));
        }
        if camera.brightness > 100 || camera.contrast > 100 || camera.saturation > 100 {
            return Err(ForgeError::ConfigError(format!(
                "device {device_key} camera values must be 0..=100"
            )));
        }
        if !(2800..=6500).contains(&camera.white_balance) {
            return Err(ForgeError::ConfigError(format!(
                "device {device_key} camera white_balance is out of range"
            )));
        }
    }
    if let Some(light) = &device.light
        && (light.brightness > 100 || !(2700..=6500).contains(&light.temperature))
    {
        return Err(ForgeError::ConfigError(format!(
            "device {device_key} light values are out of range"
        )));
    }
    Ok(())
}

fn validate_profiles(device_key: &str, profiles: &BTreeMap<String, ProfileConfig>) -> Result<()> {
    for (profile_id, profile) in profiles {
        if profile_id.trim().is_empty() {
            return Err(ForgeError::ConfigError(format!(
                "device {device_key} contains an empty profile id"
            )));
        }
        let app_id = profile
            .app_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let executable = profile
            .executable
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        if app_id.is_none() && executable.is_none() {
            return Err(ForgeError::ConfigError(format!(
                "device {device_key} profile {profile_id} requires app_id or executable"
            )));
        }
        if profile.bindings.is_empty() {
            return Err(ForgeError::ConfigError(format!(
                "device {device_key} profile {profile_id} requires at least one binding"
            )));
        }
        if profile.bindings.keys().any(|name| name.trim().is_empty()) {
            return Err(ForgeError::ConfigError(format!(
                "device {device_key} profile {profile_id} contains an empty binding name"
            )));
        }
    }
    Ok(())
}

fn validate_action_ring(device_key: &str, action_ring: &ActionRingConfig) -> Result<()> {
    if action_ring.default.slots.is_empty() {
        return Err(ForgeError::ConfigError(format!(
            "device {device_key} action_ring must contain at least one slot"
        )));
    }
    for slot_name in action_ring.default.slots.keys() {
        if !ACTION_RING_SLOTS.contains(&slot_name.as_str()) {
            return Err(ForgeError::ConfigError(format!(
                "device {device_key} action_ring slot {slot_name} is unknown"
            )));
        }
    }
    for slot in action_ring.default.slots.values() {
        if slot.label.trim().is_empty() {
            return Err(ForgeError::ConfigError(format!(
                "device {device_key} action_ring slot labels cannot be empty"
            )));
        }
    }
    Ok(())
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{
        Action, ActionRingConfig, ActionRingLayout, ActionRingSlot, AppSettings,
        CURRENT_CONFIG_SCHEMA_VERSION, CameraConfig, CameraProfile, Config, DeviceConfig,
        DeviceViewMode, KeyboardConfig, KeyboardLightingConfig, LightConfig, ProfileConfig,
        ScrollResolution,
    };
    use std::collections::BTreeMap;

    fn sample_config() -> Config {
        let mut slot_map = BTreeMap::new();
        slot_map.insert(
            "Top".into(),
            ActionRingSlot {
                action: Action::Copy,
                label: "Top".into(),
            },
        );
        slot_map.insert(
            "TopRight".into(),
            ActionRingSlot {
                action: Action::Paste,
                label: "TopRight".into(),
            },
        );

        let mut bindings = BTreeMap::new();
        bindings.insert("Back".into(), Action::BrowserBack);
        bindings.insert("Forward".into(), Action::BrowserForward);
        bindings.insert(
            "MiddleClick".into(),
            Action::HoldShortcut("Ctrl+Space".into()),
        );

        let mut keyboard_bindings = BTreeMap::new();
        keyboard_bindings.insert("F1".into(), Action::MissionControl);

        let mut devices = BTreeMap::new();
        let mut profile_bindings = BTreeMap::new();
        profile_bindings.insert("Back".into(), Action::Copy);
        let mut profiles = BTreeMap::new();
        profiles.insert(
            "editor".into(),
            ProfileConfig {
                app_id: Some("/usr/bin/editor".into()),
                executable: None,
                bindings: profile_bindings,
            },
        );
        devices.insert(
            "unit:6be9d300".into(),
            DeviceConfig {
                custom_name: "MX Master 3S".into(),
                enabled: true,
                dpi: Some(1600),
                dpi_presets: Some(vec![800, 1600, 2400, 3200]),
                invert_scroll: Some(false),
                scroll_resolution: Some(ScrollResolution::High),
                bindings,
                profiles,
                action_ring: Some(ActionRingConfig {
                    default: ActionRingLayout { slots: slot_map },
                }),
                keyboard: Some(KeyboardConfig {
                    fn_lock: Some(false),
                    lighting: Some(KeyboardLightingConfig {
                        color: "18a06f".into(),
                        brightness: 72,
                    }),
                    bindings: keyboard_bindings,
                }),
                camera: Some(CameraConfig {
                    profile: CameraProfile::VideoCall,
                    zoom: 115,
                    focus_auto: true,
                    exposure_auto: true,
                    brightness: 55,
                    contrast: 48,
                    saturation: 52,
                    white_balance: 4600,
                }),
                light: Some(LightConfig {
                    power: true,
                    brightness: 64,
                    temperature: 4200,
                    auto_power: false,
                }),
            },
        );

        Config {
            schema_version: CURRENT_CONFIG_SCHEMA_VERSION,
            selected_device: "unit:6be9d300".into(),
            app_settings: AppSettings {
                show_in_menu_bar: true,
                capture_mouse_events: true,
                device_view_mode: DeviceViewMode::Grid,
            },
            devices,
        }
    }

    #[test]
    fn parses_and_round_trips_a_strict_config() {
        let config = sample_config();
        let rendered = config.to_toml_string().unwrap();
        assert!(rendered.contains("schema_version = 1"));
        assert!(rendered.contains("selected_device = \"unit:6be9d300\""));
        assert!(rendered.contains("HoldShortcut"));

        let parsed = Config::from_toml_str(&rendered).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn rejects_bad_schema_and_missing_device() {
        let mut config = sample_config();
        config.schema_version = 2;
        assert!(Config::validate(&config).is_err());

        let mut config = sample_config();
        config.selected_device = "missing".into();
        assert!(Config::validate(&config).is_err());
    }

    #[test]
    fn accepts_default_settings_and_valid_device_shapes() {
        let config = sample_config();
        assert!(config.validate().is_ok());
        assert!(config.app_settings.show_in_menu_bar);
        assert!(config.app_settings.capture_mouse_events);
    }

    #[test]
    fn loads_the_checked_in_example_config() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/logi-forge.toml");
        let config = Config::load_from_path(path).unwrap();
        assert_eq!(config.selected_device, "unit:6be9d300");
        assert_eq!(config.devices.len(), 1);
    }

    #[test]
    fn rejects_profiles_without_a_target_or_bindings() {
        let mut config = sample_config();
        let profile = config
            .devices
            .get_mut("unit:6be9d300")
            .unwrap()
            .profiles
            .get_mut("editor")
            .unwrap();
        profile.app_id = None;
        assert!(config.validate().is_err());

        let mut config = sample_config();
        config
            .devices
            .get_mut("unit:6be9d300")
            .unwrap()
            .profiles
            .get_mut("editor")
            .unwrap()
            .bindings
            .clear();
        assert!(config.validate().is_err());
    }
}
