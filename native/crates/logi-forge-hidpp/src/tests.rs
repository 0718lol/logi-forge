use std::collections::VecDeque;
use std::time::Duration;

use logi_forge_core::{
    BatteryStatus, FnLockFeature, ForgeError, KeyboardLightingBackend, RgbColor, SmartShiftFeature,
    WheelMode,
};

use super::{DIRECT_DEVICE_INDEX, HidTransport, HidppClient, LONG_REPORT_ID, Result};

struct Step {
    expected: Vec<u8>,
    response: Vec<u8>,
}

#[derive(Default)]
struct ScriptedTransport {
    steps: VecDeque<Step>,
    writes: Vec<Vec<u8>>,
    reads: VecDeque<Vec<u8>>,
}

impl ScriptedTransport {
    fn push(&mut self, expected: &[u8], response: Vec<u8>) {
        self.steps.push_back(Step {
            expected: expected.to_vec(),
            response,
        });
    }

    fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

impl HidTransport for ScriptedTransport {
    fn transact(&mut self, request: &[u8], _timeout: Duration) -> Result<Vec<u8>> {
        let step = self.steps.pop_front().ok_or_else(|| {
            ForgeError::InvalidResponse(format!("unexpected request {request:02x?}"))
        })?;
        assert_eq!(request, step.expected, "wire request mismatch");
        Ok(step.response)
    }

    fn write_report(&mut self, report: &[u8]) -> Result<()> {
        self.writes.push(report.to_vec());
        Ok(())
    }

    fn read_report(&mut self, _timeout: Duration) -> Result<Vec<u8>> {
        self.reads.pop_front().ok_or(ForgeError::Timeout)
    }
}

fn short_response(request: &[u8], payload: [u8; 3]) -> Vec<u8> {
    vec![
        request[0], request[1], request[2], request[3], payload[0], payload[1], payload[2],
    ]
}

fn long_response(request: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut response = vec![0; 20];
    response[0] = LONG_REPORT_ID;
    response[1..4].copy_from_slice(&request[1..4]);
    response[4..4 + payload.len()].copy_from_slice(payload);
    response
}

fn resolve_request(software_id: u8, feature: u16) -> [u8; 7] {
    let [high, low] = feature.to_be_bytes();
    [0x10, DIRECT_DEVICE_INDEX, 0, software_id, high, low, 0]
}

#[test]
fn ping_and_feature_resolution_use_rotating_software_ids() {
    let mut transport = ScriptedTransport::default();
    let ping = [0x10, 0xff, 0, 0x11, 0, 0, 0xa5];
    transport.push(&ping, short_response(&ping, [2, 1, 0xa5]));
    let feature = resolve_request(2, 0x2201);
    transport.push(&feature, short_response(&feature, [5, 0, 2]));

    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    assert_eq!(client.ping().unwrap(), (2, 1));
    assert_eq!(client.resolve_feature(0x2201).unwrap(), Some(5));
    assert!(client.into_transport().is_empty());
}

#[test]
fn reads_unified_battery() {
    let mut transport = ScriptedTransport::default();
    let resolve = resolve_request(1, 0x1004);
    transport.push(&resolve, short_response(&resolve, [7, 0, 0]));
    let read = [0x10, 0xff, 7, 0x12, 0, 0, 0];
    transport.push(&read, long_response(&read, &[82, 4, 1, 0]));

    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    let battery = client.battery().unwrap();
    assert_eq!(battery.percentage, 82);
    assert_eq!(battery.status, BatteryStatus::Charging);
    assert_eq!(battery.source_feature, 0x1004);
}

#[test]
fn preserves_unified_battery_charging_states() {
    assert_eq!(super::unified_battery_status(2), BatteryStatus::AlmostFull);
    assert_eq!(
        super::unified_battery_status(4),
        BatteryStatus::SlowCharging
    );
    assert_eq!(super::unified_battery_status(7), BatteryStatus::Error(7));
}

#[test]
fn reads_and_expands_adjustable_dpi() {
    let mut transport = ScriptedTransport::default();
    let resolve = resolve_request(1, 0x2201);
    transport.push(&resolve, short_response(&resolve, [5, 0, 0]));
    let count = [0x10, 0xff, 5, 0x02, 0, 0, 0];
    transport.push(&count, short_response(&count, [1, 0, 0]));
    let list = [0x10, 0xff, 5, 0x13, 0, 0, 0];
    transport.push(
        &list,
        long_response(&list, &[0, 0x01, 0x90, 0xe1, 0x90, 0x06, 0x40, 0, 0]),
    );
    let current = [0x10, 0xff, 5, 0x24, 0, 0, 0];
    transport.push(&current, short_response(&current, [0, 0x03, 0x20]));

    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    let dpi = client.dpi().unwrap();
    assert_eq!(dpi.current, 800);
    assert_eq!(dpi.supported, [400, 800, 1200, 1600]);
}

fn push_dpi_read(transport: &mut ScriptedTransport, first_software_id: u8, current_dpi: u16) {
    let resolve = resolve_request(first_software_id, 0x2201);
    transport.push(&resolve, short_response(&resolve, [5, 0, 0]));
    let count = [0x10, 0xff, 5, first_software_id + 1, 0, 0, 0];
    transport.push(&count, short_response(&count, [1, 0, 0]));
    let list = [0x10, 0xff, 5, 0x10 | (first_software_id + 2), 0, 0, 0];
    transport.push(
        &list,
        long_response(&list, &[0, 0x03, 0x20, 0x06, 0x40, 0, 0]),
    );
    let current = [0x10, 0xff, 5, 0x20 | (first_software_id + 3), 0, 0, 0];
    let [high, low] = current_dpi.to_be_bytes();
    transport.push(&current, short_response(&current, [0, high, low]));
}

#[test]
fn writes_supported_dpi_and_verifies_read_back() {
    let mut transport = ScriptedTransport::default();
    push_dpi_read(&mut transport, 1, 800);
    let resolve = resolve_request(5, 0x2201);
    transport.push(&resolve, short_response(&resolve, [5, 0, 0]));
    let write = [0x10, 0xff, 5, 0x36, 0, 0x06, 0x40];
    transport.push(&write, short_response(&write, [0, 0x06, 0x40]));
    push_dpi_read(&mut transport, 7, 1600);

    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    let result = client.set_dpi(1600).unwrap();
    assert_eq!(result.current, 1600);
    assert!(client.into_transport().is_empty());
}

#[test]
fn reads_enhanced_smartshift() {
    let mut transport = ScriptedTransport::default();
    let resolve = resolve_request(1, 0x2111);
    transport.push(&resolve, short_response(&resolve, [9, 0, 0]));
    let read = [0x10, 0xff, 9, 0x12, 0, 0, 0];
    transport.push(&read, long_response(&read, &[2, 12, 45]));

    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    let smartshift = client.smartshift().unwrap();
    assert_eq!(smartshift.mode, WheelMode::Ratchet);
    assert_eq!(smartshift.auto_disengage, 12);
    assert_eq!(smartshift.torque, Some(45));
    assert_eq!(smartshift.feature, SmartShiftFeature::Enhanced);
}

#[test]
fn writes_enhanced_smartshift_and_reads_result() {
    let mut transport = ScriptedTransport::default();
    let resolve = resolve_request(1, 0x2111);
    transport.push(&resolve, short_response(&resolve, [9, 0, 0]));
    let write = [0x10, 0xff, 9, 0x22, 1, 0, 55];
    transport.push(&write, long_response(&write, &[1, 12, 55]));
    let resolve_again = resolve_request(3, 0x2111);
    transport.push(&resolve_again, short_response(&resolve_again, [9, 0, 0]));
    let read = [0x10, 0xff, 9, 0x14, 0, 0, 0];
    transport.push(&read, long_response(&read, &[1, 12, 55]));

    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    let result = client
        .set_smartshift(WheelMode::FreeSpin, Some(55))
        .unwrap();
    assert_eq!(result.mode, WheelMode::FreeSpin);
    assert_eq!(result.torque, Some(55));
    assert!(client.into_transport().is_empty());
}

#[test]
fn surfaces_hidpp_error_frames() {
    let mut transport = ScriptedTransport::default();
    let request = resolve_request(1, 0x2201);
    let response = vec![
        0x11, 0xff, 0xff, 0, 1, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    transport.push(&request, response);

    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    assert!(matches!(
        client.resolve_feature(0x2201),
        Err(ForgeError::Hidpp { code: 9, .. })
    ));
}

#[test]
fn reads_multi_host_fn_lock_for_current_host() {
    let mut transport = ScriptedTransport::default();
    let resolve = resolve_request(1, 0x40a3);
    transport.push(&resolve, short_response(&resolve, [6, 0, 0]));
    let read = [0x10, 0xff, 6, 0x02, 0xff, 0, 0];
    transport.push(&read, long_response(&read, &[2, 1, 0, 1]));

    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    let info = client.fn_lock().unwrap();
    assert!(info.enabled);
    assert!(!info.default_enabled);
    assert!(info.supports_manual_toggle);
    assert_eq!(info.feature, FnLockFeature::MultiHost);
    assert!(client.into_transport().is_empty());
}

#[test]
fn falls_back_to_single_host_fn_lock_and_verifies_write() {
    let mut transport = ScriptedTransport::default();
    let multi = resolve_request(1, 0x40a3);
    transport.push(&multi, short_response(&multi, [0, 0, 0]));
    let single = resolve_request(2, 0x40a2);
    transport.push(&single, short_response(&single, [8, 0, 0]));
    let write = [0x10, 0xff, 8, 0x13, 1, 0, 0];
    transport.push(&write, long_response(&write, &[1, 0]));
    let read = [0x10, 0xff, 8, 0x04, 0, 0, 0];
    transport.push(&read, long_response(&read, &[1, 0]));

    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    let info = client.set_fn_lock(true).unwrap();
    assert!(info.enabled);
    assert_eq!(info.feature, FnLockFeature::SingleHost);
    assert!(client.into_transport().is_empty());
}

#[test]
fn rejects_fn_lock_write_when_read_back_disagrees() {
    let mut transport = ScriptedTransport::default();
    let multi = resolve_request(1, 0x40a3);
    transport.push(&multi, short_response(&multi, [5, 0, 0]));
    let write = [0x10, 0xff, 5, 0x12, 0xff, 1, 0];
    transport.push(&write, long_response(&write, &[0xff, 1, 0, 1]));
    let read = [0x10, 0xff, 5, 0x03, 0xff, 0, 0];
    transport.push(&read, long_response(&read, &[0xff, 0, 0, 1]));

    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    assert!(matches!(
        client.set_fn_lock(true),
        Err(ForgeError::InvalidResponse(message)) if message.contains("verification failed")
    ));
}

#[test]
fn writes_volatile_fixed_color_to_reported_effect_zones() {
    let mut transport = ScriptedTransport::default();
    let resolve = resolve_request(1, 0x8070);
    transport.push(&resolve, short_response(&resolve, [12, 0, 0]));
    let info = [0x10, 0xff, 12, 0x02, 0, 0, 0];
    transport.push(&info, long_response(&info, &[2, 0, 0, 0, 0]));

    for (zone, software_id) in [(0, 3), (1, 4)] {
        let mut write = vec![0; 20];
        write[0] = 0x11;
        write[1] = 0xff;
        write[2] = 12;
        write[3] = 0x30 | software_id;
        write[4..9].copy_from_slice(&[zone, 1, 0x18, 0xa0, 0x6f]);
        transport.push(&write, long_response(&write, &[]));
    }

    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    let info = client
        .set_keyboard_color(RgbColor {
            red: 0x18,
            green: 0xa0,
            blue: 0x6f,
        })
        .unwrap();
    assert_eq!(info.zones_written, 2);
    assert_eq!(info.backend, KeyboardLightingBackend::Effects);
    assert!(client.into_transport().is_empty());
}

#[test]
fn reports_effect_lighting_as_unsupported_when_feature_is_absent() {
    let mut transport = ScriptedTransport::default();
    let resolve = resolve_request(1, 0x8070);
    transport.push(&resolve, short_response(&resolve, [0, 0, 0]));
    let per_key_v2 = resolve_request(2, 0x8081);
    transport.push(&per_key_v2, short_response(&per_key_v2, [0, 0, 0]));
    let per_key = resolve_request(3, 0x8080);
    transport.push(&per_key, short_response(&per_key, [0, 0, 0]));
    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    assert!(matches!(
        client.set_keyboard_color(RgbColor {
            red: 1,
            green: 2,
            blue: 3,
        }),
        Err(ForgeError::FeatureUnsupported(0x8080))
    ));
}

#[test]
fn falls_back_to_per_key_v2_and_commits_present_zones() {
    let mut transport = ScriptedTransport::default();
    let effects = resolve_request(1, 0x8070);
    transport.push(&effects, short_response(&effects, [0, 0, 0]));
    let resolve = resolve_request(2, 0x8081);
    transport.push(&resolve, short_response(&resolve, [13, 0, 0]));

    for (page, software_id, bits) in [(0, 3, 0x22), (1, 4, 0), (2, 5, 0)] {
        let read = [0x10, 0xff, 13, software_id, 0, page, 0];
        let mut payload = [0; 16];
        payload[2] = bits;
        transport.push(&read, long_response(&read, &payload));
    }
    let mut set = vec![0; 20];
    set[..4].copy_from_slice(&[0x11, 0xff, 13, 0x66]);
    set[4..12].copy_from_slice(&[1, 2, 3, 1, 5, 0, 0, 0]);
    transport.push(&set, long_response(&set, &[]));
    let mut commit = vec![0; 20];
    commit[..4].copy_from_slice(&[0x11, 0xff, 13, 0x77]);
    transport.push(&commit, long_response(&commit, &[]));

    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    let info = client
        .set_keyboard_color(RgbColor {
            red: 1,
            green: 2,
            blue: 3,
        })
        .unwrap();
    assert_eq!(info.backend, KeyboardLightingBackend::PerKeyV2);
    assert_eq!(info.zones_written, 2);
    assert!(client.into_transport().is_empty());
}

#[test]
fn falls_back_to_unacknowledged_per_key_frames() {
    let mut transport = ScriptedTransport::default();
    for (software_id, feature) in [(1, 0x8070), (2, 0x8081)] {
        let resolve = resolve_request(software_id, feature);
        transport.push(&resolve, short_response(&resolve, [0, 0, 0]));
    }
    let resolve = resolve_request(3, 0x8080);
    transport.push(&resolve, short_response(&resolve, [14, 0, 0]));

    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    let info = client
        .set_keyboard_color(RgbColor {
            red: 0x11,
            green: 0x22,
            blue: 0x33,
        })
        .unwrap();
    assert_eq!(info.backend, KeyboardLightingBackend::PerKey);
    assert_eq!(info.zones_written, 0xe9);
    let transport = client.into_transport();
    assert!(transport.steps.is_empty());
    assert_eq!(transport.writes.len(), 18);
    assert_eq!(
        transport.writes[0][..12],
        [0x12, 0xff, 14, 0x3a, 0, 1, 0, 14, 0, 0x11, 0x22, 0x33]
    );
    assert_eq!(
        transport.writes.last().unwrap()[..4],
        [0x11, 0xff, 14, 0x5a]
    );
}

#[test]
fn enumerates_and_verifies_reprogrammable_control_diversion() {
    let mut transport = ScriptedTransport::default();
    let resolve = resolve_request(1, 0x1b04);
    transport.push(&resolve, short_response(&resolve, [15, 0, 0]));
    let count = [0x10, 0xff, 15, 0x02, 0, 0, 0];
    transport.push(&count, short_response(&count, [2, 0, 0]));
    for (row, software_id, cid, flags) in [(0, 3, 0x00d4u16, 0x22), (1, 4, 0x0103, 0x02)] {
        let mut request = vec![0; 20];
        request[..4].copy_from_slice(&[0x11, 0xff, 15, 0x10 | software_id]);
        request[4] = row;
        let [high, low] = cid.to_be_bytes();
        transport.push(
            &request,
            long_response(&request, &[high, low, 0, row + 1, flags, row, 1, 1, 0]),
        );
    }
    let mut divert = vec![0; 20];
    divert[..7].copy_from_slice(&[0x11, 0xff, 15, 0x35, 0, 0xd4, 0x23]);
    transport.push(&divert, long_response(&divert, &[0, 0xd4, 0x03]));
    let read = [0x10, 0xff, 15, 0x26, 0, 0xd4, 0];
    transport.push(&read, long_response(&read, &[0, 0xd4, 1]));

    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    let info = client.reprog_controls().unwrap();
    assert_eq!(info.feature_index, 15);
    assert_eq!(info.controls.len(), 2);
    assert!(info.controls[0].is_divertable());
    assert!(!info.controls[1].is_divertable());
    client
        .set_control_diverted(info.feature_index, 0x00d4, true)
        .unwrap();
    assert!(client.into_transport().is_empty());
}

#[test]
fn decodes_and_filters_diverted_control_snapshots() {
    let event = [
        0x11, 0xff, 15, 0, 0, 0xd4, 1, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    assert_eq!(
        super::decode_diverted_controls(&event, 0xff, 15),
        Some([0x00d4, 0x0103, 0, 0])
    );
    assert_eq!(super::decode_diverted_controls(&event, 1, 15), None);
    assert_eq!(super::keyboard_control_cid("f12"), Some(0x00e9));
    assert_eq!(super::keyboard_control_cid("F3"), None);

    let mut transport = ScriptedTransport::default();
    let mut unrelated = event;
    unrelated[2] = 9;
    transport.reads.push_back(unrelated.to_vec());
    transport.reads.push_back(event.to_vec());
    let mut client = HidppClient::new(transport, DIRECT_DEVICE_INDEX);
    assert_eq!(
        client
            .next_diverted_controls(15, Duration::from_millis(20))
            .unwrap(),
        [0x00d4, 0x0103, 0, 0]
    );
}
