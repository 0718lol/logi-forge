use std::time::Duration;

use logi_forge_core::{
    BatteryInfo, BatteryStatus, DpiInfo, FeatureMap, FnLockFeature, FnLockInfo, ForgeError,
    KeyboardLightingBackend, KeyboardLightingInfo, ReprogControlInfo, ReprogControlsInfo, Result,
    RgbColor, SmartShiftFeature, SmartShiftInfo, WheelMode,
};

pub const SHORT_REPORT_ID: u8 = 0x10;
pub const LONG_REPORT_ID: u8 = 0x11;
pub const SHORT_REPORT_LENGTH: usize = 7;
pub const LONG_REPORT_LENGTH: usize = 20;
pub const DIRECT_DEVICE_INDEX: u8 = 0xff;

const ROOT_FEATURE_INDEX: u8 = 0x00;
const UNIFIED_BATTERY: u16 = 0x1004;
const LEGACY_BATTERY: u16 = 0x1000;
const ADJUSTABLE_DPI: u16 = 0x2201;
const SMARTSHIFT_ENHANCED: u16 = 0x2111;
const SMARTSHIFT_LEGACY: u16 = 0x2110;
const FN_INVERSION_MULTI_HOST: u16 = 0x40a3;
const FN_INVERSION_SINGLE_HOST: u16 = 0x40a2;
const CURRENT_HOST: u8 = 0xff;
const COLOR_LED_EFFECTS: u16 = 0x8070;
const FIXED_COLOR_EFFECT: u8 = 1;
const MAX_EFFECT_ZONES: u8 = 4;
const PER_KEY_LIGHTING_V2: u16 = 0x8081;
const PER_KEY_LIGHTING: u16 = 0x8080;
const ZONE_PRESENCE_BYTES: usize = 14;
const MAX_SINGLE_COLOR_ZONES: usize = 13;
const VERY_LONG_REPORT_ID: u8 = 0x12;
const REPROG_CONTROLS: u16 = 0x1b04;

pub trait HidTransport {
    /// Sends one HID++ request and waits for its correlated response.
    ///
    /// # Errors
    ///
    /// Returns a transport, permission, disconnection, or timeout error.
    fn transact(&mut self, request: &[u8], timeout: Duration) -> Result<Vec<u8>>;

    /// Writes a report that intentionally has no correlated response.
    ///
    /// # Errors
    ///
    /// Returns a transport error when the report cannot be written.
    fn write_report(&mut self, _report: &[u8]) -> Result<()> {
        Err(ForgeError::InvalidArgument(
            "transport does not support unacknowledged reports".into(),
        ))
    }

    /// Reads one unsolicited report, waiting up to `timeout`.
    ///
    /// # Errors
    ///
    /// Returns a timeout or transport error when no report can be read.
    fn read_report(&mut self, _timeout: Duration) -> Result<Vec<u8>> {
        Err(ForgeError::InvalidArgument(
            "transport does not support unsolicited reports".into(),
        ))
    }
}

pub struct HidppClient<T> {
    transport: T,
    device_index: u8,
    next_software_id: u8,
    timeout: Duration,
}

impl<T: HidTransport> HidppClient<T> {
    #[must_use]
    pub fn new(transport: T, device_index: u8) -> Self {
        Self {
            transport,
            device_index,
            next_software_id: 1,
            timeout: Duration::from_millis(900),
        }
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    fn software_id(&mut self) -> u8 {
        let current = self.next_software_id;
        self.next_software_id = if current == 0x0f { 1 } else { current + 1 };
        current
    }

    fn call(&mut self, feature_index: u8, function: u8, args: [u8; 3]) -> Result<[u8; 16]> {
        if function > 0x0f {
            return Err(ForgeError::InvalidArgument(format!(
                "function id {function} exceeds four bits"
            )));
        }
        let software_id = self.software_id();
        let function_and_software = (function << 4) | software_id;
        let request = [
            SHORT_REPORT_ID,
            self.device_index,
            feature_index,
            function_and_software,
            args[0],
            args[1],
            args[2],
        ];
        let response = self.transport.transact(&request, self.timeout)?;
        decode_response(
            &response,
            self.device_index,
            feature_index,
            function_and_software,
        )
    }

    fn call_long(&mut self, feature_index: u8, function: u8, args: [u8; 16]) -> Result<[u8; 16]> {
        if function > 0x0f {
            return Err(ForgeError::InvalidArgument(format!(
                "function id {function} exceeds four bits"
            )));
        }
        let software_id = self.software_id();
        let function_and_software = (function << 4) | software_id;
        let mut request = [0; LONG_REPORT_LENGTH];
        request[0] = LONG_REPORT_ID;
        request[1] = self.device_index;
        request[2] = feature_index;
        request[3] = function_and_software;
        request[4..].copy_from_slice(&args);
        let response = self.transport.transact(&request, self.timeout)?;
        decode_response(
            &response,
            self.device_index,
            feature_index,
            function_and_software,
        )
    }

    /// Reads the device's HID++ protocol major and minor version.
    ///
    /// # Errors
    ///
    /// Returns an error when the device cannot be reached or echoes invalid data.
    pub fn ping(&mut self) -> Result<(u8, u8)> {
        let nonce = 0xa5;
        let payload = self.call(ROOT_FEATURE_INDEX, 1, [0, 0, nonce])?;
        if payload[2] != nonce {
            return Err(ForgeError::InvalidResponse("ping nonce mismatch".into()));
        }
        Ok((payload[0], payload[1]))
    }

    /// Resolves a stable feature ID to the device's runtime feature index.
    ///
    /// # Errors
    ///
    /// Returns an error when the root-feature transaction fails.
    pub fn resolve_feature(&mut self, feature_id: u16) -> Result<Option<u8>> {
        let [high, low] = feature_id.to_be_bytes();
        let payload = self.call(ROOT_FEATURE_INDEX, 0, [high, low, 0])?;
        Ok((payload[0] != 0).then_some(payload[0]))
    }

    /// Resolves every feature currently used by the M2 agent.
    ///
    /// # Errors
    ///
    /// Returns an error when any root-feature lookup fails.
    pub fn feature_map(&mut self) -> Result<FeatureMap> {
        Ok(FeatureMap {
            unified_battery: self.resolve_feature(UNIFIED_BATTERY)?,
            legacy_battery: self.resolve_feature(LEGACY_BATTERY)?,
            adjustable_dpi: self.resolve_feature(ADJUSTABLE_DPI)?,
            smartshift_enhanced: self.resolve_feature(SMARTSHIFT_ENHANCED)?,
            smartshift_legacy: self.resolve_feature(SMARTSHIFT_LEGACY)?,
        })
    }

    /// Reads unified or legacy battery state, preferring unified battery data.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, malformed responses, or unsupported devices.
    pub fn battery(&mut self) -> Result<BatteryInfo> {
        if let Some(index) = self.resolve_feature(UNIFIED_BATTERY)? {
            let payload = self.call(index, 1, [0; 3])?;
            return Ok(BatteryInfo {
                percentage: payload[0],
                status: unified_battery_status(payload[2]),
                source_feature: UNIFIED_BATTERY,
            });
        }
        if let Some(index) = self.resolve_feature(LEGACY_BATTERY)? {
            let payload = self.call(index, 0, [0; 3])?;
            return Ok(BatteryInfo {
                percentage: payload[0],
                status: legacy_battery_status(payload[2]),
                source_feature: LEGACY_BATTERY,
            });
        }
        Err(ForgeError::FeatureUnsupported(UNIFIED_BATTERY))
    }

    /// Reads current and supported DPI values for sensor zero.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, malformed ranges, or unsupported devices.
    pub fn dpi(&mut self) -> Result<DpiInfo> {
        let index = self
            .resolve_feature(ADJUSTABLE_DPI)?
            .ok_or(ForgeError::FeatureUnsupported(ADJUSTABLE_DPI))?;
        let sensor_count = self.call(index, 0, [0; 3])?[0];
        if sensor_count == 0 {
            return Err(ForgeError::InvalidResponse(
                "DPI feature reports no sensors".into(),
            ));
        }
        let list_payload = self.call(index, 1, [0, 0, 0])?;
        let supported = parse_dpi_list(&list_payload[1..])?;
        let current_payload = self.call(index, 2, [0, 0, 0])?;
        let current = u16::from_be_bytes([current_payload[1], current_payload[2]]);
        Ok(DpiInfo {
            current,
            supported,
            source_feature: ADJUSTABLE_DPI,
        })
    }

    /// Writes a supported DPI value and verifies it by reading the sensor back.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is unsupported, I/O fails, or read-back differs.
    pub fn set_dpi(&mut self, dpi: u16) -> Result<DpiInfo> {
        let before = self.dpi()?;
        if !before.supported.contains(&dpi) {
            return Err(ForgeError::InvalidArgument(format!(
                "DPI {dpi} is not in the device-supported list"
            )));
        }
        let index = self
            .resolve_feature(ADJUSTABLE_DPI)?
            .ok_or(ForgeError::FeatureUnsupported(ADJUSTABLE_DPI))?;
        let [high, low] = dpi.to_be_bytes();
        self.call(index, 3, [0, high, low])?;
        let after = self.dpi()?;
        if after.current != dpi {
            return Err(ForgeError::VerificationFailed {
                requested: dpi,
                actual: after.current,
            });
        }
        Ok(after)
    }

    /// Reads enhanced or legacy `SmartShift` state.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, unknown wire values, or unsupported devices.
    pub fn smartshift(&mut self) -> Result<SmartShiftInfo> {
        if let Some(index) = self.resolve_feature(SMARTSHIFT_ENHANCED)? {
            let payload = self.call(index, 1, [0; 3])?;
            return Ok(SmartShiftInfo {
                mode: WheelMode::try_from(payload[0])?,
                auto_disengage: payload[1],
                torque: Some(payload[2]),
                feature: SmartShiftFeature::Enhanced,
            });
        }
        if let Some(index) = self.resolve_feature(SMARTSHIFT_LEGACY)? {
            let payload = self.call(index, 0, [0; 3])?;
            return Ok(SmartShiftInfo {
                mode: WheelMode::try_from(payload[0])?,
                auto_disengage: payload[1],
                torque: None,
                feature: SmartShiftFeature::Legacy,
            });
        }
        Err(ForgeError::FeatureUnsupported(SMARTSHIFT_ENHANCED))
    }

    /// Writes `SmartShift` mode and optional enhanced torque, then reads the state back.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid torque, unsupported enhanced fields, or I/O failures.
    pub fn set_smartshift(
        &mut self,
        mode: WheelMode,
        torque: Option<u8>,
    ) -> Result<SmartShiftInfo> {
        if torque.is_some_and(|value| value == 0 || value > 100) {
            return Err(ForgeError::InvalidArgument(
                "SmartShift torque must be between 1 and 100".into(),
            ));
        }
        if let Some(index) = self.resolve_feature(SMARTSHIFT_ENHANCED)? {
            self.call(index, 2, [mode.wire_value(), 0, torque.unwrap_or(0)])?;
            return self.smartshift();
        }
        if torque.is_some() {
            return Err(ForgeError::FeatureUnsupported(SMARTSHIFT_ENHANCED));
        }
        if let Some(index) = self.resolve_feature(SMARTSHIFT_LEGACY)? {
            self.call(index, 1, [mode.wire_value(), 0, 0])?;
            return self.smartshift();
        }
        Err(ForgeError::FeatureUnsupported(SMARTSHIFT_ENHANCED))
    }

    /// Reads Fn-lock state, preferring the per-host feature used by Easy-Switch keyboards.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed state values, transport failures, or unsupported devices.
    pub fn fn_lock(&mut self) -> Result<FnLockInfo> {
        if let Some(index) = self.resolve_feature(FN_INVERSION_MULTI_HOST)? {
            let payload = self.call(index, 0, [CURRENT_HOST, 0, 0])?;
            return parse_fn_lock(payload, FnLockFeature::MultiHost);
        }
        if let Some(index) = self.resolve_feature(FN_INVERSION_SINGLE_HOST)? {
            let payload = self.call(index, 0, [0; 3])?;
            return parse_fn_lock(payload, FnLockFeature::SingleHost);
        }
        Err(ForgeError::FeatureUnsupported(FN_INVERSION_MULTI_HOST))
    }

    /// Writes Fn-lock and verifies the resulting state with a read-back.
    ///
    /// # Errors
    ///
    /// Returns an error when neither feature is present, the write fails, or
    /// the keyboard reports a different state after the write.
    pub fn set_fn_lock(&mut self, enabled: bool) -> Result<FnLockInfo> {
        let requested = u8::from(enabled);
        let info = if let Some(index) = self.resolve_feature(FN_INVERSION_MULTI_HOST)? {
            self.call(index, 1, [CURRENT_HOST, requested, 0])?;
            parse_fn_lock(
                self.call(index, 0, [CURRENT_HOST, 0, 0])?,
                FnLockFeature::MultiHost,
            )?
        } else if let Some(index) = self.resolve_feature(FN_INVERSION_SINGLE_HOST)? {
            self.call(index, 1, [requested, 0, 0])?;
            parse_fn_lock(self.call(index, 0, [0; 3])?, FnLockFeature::SingleHost)?
        } else {
            return Err(ForgeError::FeatureUnsupported(FN_INVERSION_MULTI_HOST));
        };
        if info.enabled != enabled {
            return Err(ForgeError::InvalidResponse(format!(
                "Fn-lock write verification failed: requested {enabled}, device reports {}",
                info.enabled
            )));
        }
        Ok(info)
    }

    /// Applies a volatile fixed RGB color through HID++ `0x8070`.
    ///
    /// The write is intentionally RAM-only to avoid EEPROM wear. Devices that
    /// expose only per-key lighting return `FeatureUnsupported` until the
    /// `0x8081/0x8080` fallback backend is enabled.
    ///
    /// # Errors
    ///
    /// Returns an error when the feature is unavailable or a zone write fails.
    pub fn set_keyboard_color(&mut self, color: RgbColor) -> Result<KeyboardLightingInfo> {
        if let Some(index) = self.resolve_feature(COLOR_LED_EFFECTS)? {
            return self.set_color_effects(index, color);
        }
        if let Some(index) = self.resolve_feature(PER_KEY_LIGHTING_V2)? {
            let info = self.set_color_per_key_v2(index, color)?;
            if info.zones_written != 0 {
                return Ok(info);
            }
        }
        if let Some(index) = self.resolve_feature(PER_KEY_LIGHTING)? {
            return self.set_color_per_key(index, color);
        }
        Err(ForgeError::FeatureUnsupported(PER_KEY_LIGHTING))
    }

    fn set_color_effects(&mut self, index: u8, color: RgbColor) -> Result<KeyboardLightingInfo> {
        let reported_zones = self.call(index, 0, [0; 3])?[0];
        let zones_to_write = if reported_zones == 0 {
            MAX_EFFECT_ZONES
        } else {
            reported_zones.min(MAX_EFFECT_ZONES)
        };
        for zone in 0..zones_to_write {
            let args = fixed_color_args(zone, color);
            self.call_long(index, 3, args)?;
            if zone + 1 < zones_to_write {
                std::thread::sleep(Duration::from_millis(8));
            }
        }
        Ok(KeyboardLightingInfo {
            backend: KeyboardLightingBackend::Effects,
            zones_written: u16::from(zones_to_write),
        })
    }

    fn set_color_per_key_v2(&mut self, index: u8, color: RgbColor) -> Result<KeyboardLightingInfo> {
        let mut zones = Vec::new();
        for (page, base) in [(0, 0u16), (1, 112), (2, 224)] {
            let payload = self.call(index, 0, [0, page, 0])?;
            collect_present_zones(base, &payload[2..], &mut zones);
        }
        for chunk in zones.chunks(MAX_SINGLE_COLOR_ZONES) {
            let mut args = [0; 16];
            args[..3].copy_from_slice(&[color.red, color.green, color.blue]);
            args[3..3 + chunk.len()].copy_from_slice(chunk);
            self.call_long(index, 6, args)?;
        }
        if !zones.is_empty() {
            self.call_long(index, 7, [0; 16])?;
        }
        Ok(KeyboardLightingInfo {
            backend: KeyboardLightingBackend::PerKeyV2,
            zones_written: u16::try_from(zones.len()).unwrap_or(u16::MAX),
        })
    }

    fn set_color_per_key(&mut self, index: u8, color: RgbColor) -> Result<KeyboardLightingInfo> {
        for report in per_key_reports(self.device_index, index, color) {
            self.transport.write_report(&report)?;
        }
        Ok(KeyboardLightingInfo {
            backend: KeyboardLightingBackend::PerKey,
            zones_written: 0xe9,
        })
    }

    /// Enumerates HID++ `0x1b04` controls and their diversion capabilities.
    ///
    /// # Errors
    ///
    /// Returns an error when the feature is absent or a control row is malformed.
    pub fn reprog_controls(&mut self) -> Result<ReprogControlsInfo> {
        let feature_index = self
            .resolve_feature(REPROG_CONTROLS)?
            .ok_or(ForgeError::FeatureUnsupported(REPROG_CONTROLS))?;
        let count = self.call(feature_index, 0, [0; 3])?[0];
        let mut controls = Vec::with_capacity(usize::from(count));
        for index in 0..count {
            let mut args = [0; 16];
            args[0] = index;
            let payload = self.call_long(feature_index, 1, args)?;
            controls.push(ReprogControlInfo {
                cid: u16::from_be_bytes([payload[0], payload[1]]),
                task_id: u16::from_be_bytes([payload[2], payload[3]]),
                flags: u16::from(payload[4]) | (u16::from(payload[8]) << 8),
            });
        }
        Ok(ReprogControlsInfo {
            feature_index,
            controls,
        })
    }

    /// Enables or disables temporary diversion for one control and verifies it.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails or read-back disagrees.
    pub fn set_control_diverted(
        &mut self,
        feature_index: u8,
        cid: u16,
        diverted: bool,
    ) -> Result<()> {
        let [high, low] = cid.to_be_bytes();
        let mut args = [0; 16];
        args[..3].copy_from_slice(&[high, low, 0x22 | u8::from(diverted)]);
        self.call_long(feature_index, 3, args)?;
        let payload = self.call(feature_index, 2, [high, low, 0])?;
        let reported_cid = u16::from_be_bytes([payload[0], payload[1]]);
        let reported = payload[2] & 1 != 0;
        if reported_cid != cid || reported != diverted {
            return Err(ForgeError::InvalidResponse(format!(
                "control diversion verification failed for 0x{cid:04x}: device reports cid=0x{reported_cid:04x} diverted={reported}"
            )));
        }
        Ok(())
    }

    /// Waits for the next diverted-button snapshot from `0x1b04`.
    ///
    /// # Errors
    ///
    /// Returns a timeout or transport error while waiting for an event.
    pub fn next_diverted_controls(
        &mut self,
        feature_index: u8,
        timeout: Duration,
    ) -> Result<[u16; 4]> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(ForgeError::Timeout);
            }
            let report = self.transport.read_report(remaining)?;
            if let Some(cids) = decode_diverted_controls(&report, self.device_index, feature_index)
            {
                return Ok(cids);
            }
        }
    }
}

/// Maps Signature/MX media-mode F-row positions to `0x1b04` control IDs.
#[must_use]
pub fn keyboard_control_cid(label: &str) -> Option<u16> {
    match label.trim().to_ascii_uppercase().as_str() {
        "F4" => Some(0x00d4),
        "F5" => Some(0x0103),
        "F6" => Some(0x0108),
        "F7" => Some(0x010a),
        "F8" => Some(0x011c),
        "F9" => Some(0x00e5),
        "F10" => Some(0x00e7),
        "F11" => Some(0x00e8),
        "F12" => Some(0x00e9),
        _ => None,
    }
}

#[must_use]
pub fn decode_diverted_controls(
    report: &[u8],
    device_index: u8,
    feature_index: u8,
) -> Option<[u16; 4]> {
    if !matches!(report.first(), Some(&SHORT_REPORT_ID | &LONG_REPORT_ID))
        || report.len() < 12
        || report[1] != device_index
        || report[2] != feature_index
        || report[3] != 0
    {
        return None;
    }
    Some([
        u16::from_be_bytes([report[4], report[5]]),
        u16::from_be_bytes([report[6], report[7]]),
        u16::from_be_bytes([report[8], report[9]]),
        u16::from_be_bytes([report[10], report[11]]),
    ])
}

fn collect_present_zones(base: u16, bitfield: &[u8], zones: &mut Vec<u8>) {
    for (byte_index, byte) in bitfield.iter().take(ZONE_PRESENCE_BYTES).enumerate() {
        for bit in 0..8u16 {
            if byte & (1 << bit) == 0 {
                continue;
            }
            let Ok(offset) = u16::try_from(byte_index * 8) else {
                continue;
            };
            let Ok(zone) = u8::try_from(base + offset + bit) else {
                continue;
            };
            if !matches!(zone, 0 | 0xff) {
                zones.push(zone);
            }
        }
    }
}

fn per_key_reports(device_index: u8, feature_index: u8, color: RgbColor) -> Vec<Vec<u8>> {
    let mut reports = Vec::new();
    let key_ids = (0u8..=0xe8).collect::<Vec<_>>();
    for chunk in key_ids.chunks(14) {
        let mut report = vec![0; 64];
        report[0] = VERY_LONG_REPORT_ID;
        report[1] = device_index;
        report[2] = feature_index;
        report[3] = 0x3a;
        report[5] = 1;
        report[7] = 14;
        for (index, key) in chunk.iter().enumerate() {
            let offset = 8 + index * 4;
            report[offset..offset + 4].copy_from_slice(&[*key, color.red, color.green, color.blue]);
        }
        reports.push(report);
    }
    let mut commit = vec![0; LONG_REPORT_LENGTH];
    commit[0] = LONG_REPORT_ID;
    commit[1] = device_index;
    commit[2] = feature_index;
    commit[3] = 0x5a;
    reports.push(commit);
    reports
}

fn fixed_color_args(zone: u8, color: RgbColor) -> [u8; 16] {
    let mut args = [0; 16];
    args[0] = zone;
    args[1] = FIXED_COLOR_EFFECT;
    args[2] = color.red;
    args[3] = color.green;
    args[4] = color.blue;
    args[12] = 0;
    args
}

fn parse_fn_lock(payload: [u8; 16], feature: FnLockFeature) -> Result<FnLockInfo> {
    let offset = usize::from(feature == FnLockFeature::MultiHost);
    let enabled = parse_bool_state(payload[offset], "Fn-lock state")?;
    let default_enabled = parse_bool_state(payload[offset + 1], "Fn-lock default state")?;
    let supports_manual_toggle = match feature {
        FnLockFeature::MultiHost => payload[3] & 1 != 0,
        FnLockFeature::SingleHost => true,
    };
    Ok(FnLockInfo {
        enabled,
        default_enabled,
        supports_manual_toggle,
        feature,
    })
}

fn parse_bool_state(value: u8, field: &str) -> Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ForgeError::InvalidResponse(format!(
            "{field} has unknown value {value}"
        ))),
    }
}

fn decode_response(
    response: &[u8],
    device_index: u8,
    feature_index: u8,
    function_and_software: u8,
) -> Result<[u8; 16]> {
    let expected_length = match response.first() {
        Some(&SHORT_REPORT_ID) => SHORT_REPORT_LENGTH,
        Some(&LONG_REPORT_ID) => LONG_REPORT_LENGTH,
        Some(report) => {
            return Err(ForgeError::InvalidResponse(format!(
                "unexpected report id 0x{report:02x}"
            )));
        }
        None => return Err(ForgeError::InvalidResponse("empty report".into())),
    };
    if response.len() != expected_length {
        return Err(ForgeError::InvalidResponse(format!(
            "report 0x{:02x} has length {}, expected {expected_length}",
            response[0],
            response.len()
        )));
    }
    if response[1] != device_index {
        return Err(ForgeError::InvalidResponse("device index mismatch".into()));
    }
    if response[2] == 0xff && response.len() >= 7 {
        if response[3] == feature_index && response[4] == function_and_software {
            return Err(ForgeError::Hidpp {
                code: response[5],
                feature_index,
                function: function_and_software >> 4,
            });
        }
        return Err(ForgeError::InvalidResponse(
            "unmatched HID++ error frame".into(),
        ));
    }
    if response[2] != feature_index || response[3] != function_and_software {
        return Err(ForgeError::InvalidResponse(
            "response header mismatch".into(),
        ));
    }
    let mut payload = [0; 16];
    let available = response.len() - 4;
    payload[..available].copy_from_slice(&response[4..]);
    Ok(payload)
}

fn parse_dpi_list(bytes: &[u8]) -> Result<Vec<u16>> {
    let mut values = Vec::new();
    let mut offset = 0;
    while offset + 1 < bytes.len() {
        let value = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
        if value == 0 {
            break;
        }
        if value >> 13 == 0b111 {
            let step = value & 0x1fff;
            let Some(start) = values.last().copied() else {
                return Err(ForgeError::InvalidResponse("DPI range has no start".into()));
            };
            if step == 0 || offset + 3 >= bytes.len() {
                return Err(ForgeError::InvalidResponse("malformed DPI range".into()));
            }
            let end = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]);
            if end < start {
                return Err(ForgeError::InvalidResponse("descending DPI range".into()));
            }
            let mut next = u32::from(start) + u32::from(step);
            while next < u32::from(end) {
                values.push(
                    u16::try_from(next)
                        .map_err(|_| ForgeError::InvalidResponse("DPI value overflow".into()))?,
                );
                next += u32::from(step);
            }
            values.push(end);
            offset += 4;
        } else {
            values.push(value);
            offset += 2;
        }
    }
    if values.is_empty() {
        return Err(ForgeError::InvalidResponse("empty DPI list".into()));
    }
    values.sort_unstable();
    values.dedup();
    Ok(values)
}

fn unified_battery_status(value: u8) -> BatteryStatus {
    match value {
        0 => BatteryStatus::Discharging,
        1 => BatteryStatus::Charging,
        2 => BatteryStatus::AlmostFull,
        3 => BatteryStatus::Full,
        4 => BatteryStatus::SlowCharging,
        other => BatteryStatus::Error(other),
    }
}

fn legacy_battery_status(value: u8) -> BatteryStatus {
    match value {
        0 => BatteryStatus::Discharging,
        1 => BatteryStatus::Charging,
        2 => BatteryStatus::AlmostFull,
        3 => BatteryStatus::Full,
        4 => BatteryStatus::SlowCharging,
        other => BatteryStatus::Error(other),
    }
}

#[cfg(test)]
mod tests;
