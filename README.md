# Logi Forge

Logi Forge is a clean-room rewrite workspace for an OpenLogi-inspired device
manager. The first artifact is a hosted frontend connected to an independent
mock agent over versioned HTTP/SSE transport. It does not reuse OpenLogi
proprietary brand assets or claim wire compatibility with OpenLogi.

## Upstream Snapshot

- Repository: `https://github.com/AprilNEA/OpenLogi`
- Source location: `/workspace/upstreams/OpenLogi`
- Baseline commit: `b32ae0872252a20353a2b8b51f1ef74b065c78f6`
- Commit date: `2026-08-27T16:46:49Z`
- Upstream version in workspace: `0.8.1`
- Config schema observed: `6`
- IPC protocol observed: `29`

## Rewrite Target

Long-term parity target:

- Native background agent owns device I/O, input capture, and input injection.
- Frontend is an IPC client and never writes hardware directly.
- TOML config remains strict, local-first, and migration-aware.
- Mock agent stays contract-compatible for hardware-free demos and tests.
- Hardware support covers HID++ receivers, Bluetooth/direct devices, UVC
  cameras, standalone lights, mouse/keyboard profiles, and pairing.

Implemented first vertical slice:

- Mock inventory for mouse, keyboard, camera, and light.
- Device gallery and detail panels.
- Button remapping, Actions Ring, DPI, SmartShift, lighting, camera controls,
  keyboard F-row, and config preview.
- Versioned protocol snapshot and typed command errors.
- Atomic local mock-state persistence across service restarts.
- Capability, online-state, value-range, and revision validation.
- Server-generated TOML preview and live SSE inventory updates.
- Clear boundary for replacing mock writes with a native agent.
- Structured diagnostics for native status, device nodes, and host permissions.

Native M2 implementation:

- Independent Rust workspace under `native/` with core, HID++, Linux, and CLI crates.
- Real Linux sysfs/hidraw discovery and request transport.
- HID++ battery, adjustable DPI, and SmartShift read/write operations.
- DPI writes validate the device's supported list and verify with read-back.
- Scripted protocol tests run without physical hardware.
- A udev rule is provided for active-session access to Logitech hidraw nodes.

M4 profile implementation:

- Application-aware profile resolver with deterministic specificity rules.
- Linux X11 foreground watcher and CLI `--profile` bindings.
- Atomic effective binding swaps while the evdev capture loop is running.

Native agent runtime:

- Resident Rust agent with real hidraw inventory polling and strict config loading.
- Unix-socket JSON IPC bridged into the existing HTTP/SSE protocol-v1 snapshot.
- Mock inventory remains available for hardware-free demos; native status,
  inventory, config health, and apply results are shown separately.
- Optional guarded hardware apply through `LOGI_FORGE_DEVICE_PATH` and
  `LOGI_FORGE_DEVICE_INDEX`, including Fn-lock verification and RGB backend selection.
- Keyboard Fn-lock, color, and brightness PATCH requests persist through the
  native agent with independent revision checks and atomic TOML replacement.

The current sandbox exposes no `/dev/hidraw*` nodes. Native code is compiled,
linted, protocol-tested, and connected to the hosted frontend, but physical
MX Master/MX Keys acceptance remains pending.

## Run and verify

The ASteam daemon starts the product from `app.toml`. For local verification:

```bash
PORT=3000 npm start
npm test
npm run lint:native
```

The hosted build exposes `GET /api/v1/diagnostics` for hardware-free checks of
the Native Agent, Linux `hidraw`/`input`/`uinput` nodes, and configuration apply
status. The UI exposes the same data in the Runtime inspector.

Set `LOGI_FORGE_API_TOKEN` to require a Bearer token for all mutating API
requests. The browser UI receives a same-origin, `HttpOnly` session cookie;
external CLI clients should send `Authorization: Bearer <token>`.

## Implementation Direction

Use a split architecture:

- `agent`: Rust, from-scratch device/session model, platform adapters behind
  traits.
- `frontend`: TypeScript/Tauri or React UI, using the prototype as the first
  screen model.
- `protocol`: versioned JSON or MessagePack IPC DTOs; append-only after v1.
- `tests`: mock transport and scripted inventory before real hardware tests.
