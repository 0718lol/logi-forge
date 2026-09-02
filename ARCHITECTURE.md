# Rewrite Architecture

## Boundary

This is a clean-room style rewrite inspired by OpenLogi's product behavior. It
does not copy OpenLogi's proprietary brand assets. The upstream source snapshot
is kept under `/workspace/upstreams/OpenLogi` as a behavior reference.

## Processes

| Process | Responsibility |
| --- | --- |
| `logi-forge-agent` | Own hardware I/O, input capture, injection, foreground app tracking, pairing |
| `logi-forge-ui` | Render state, edit config, call agent commands |
| `logi-forge-overlay` | Render cursor-centered Actions Ring and return selected slot |
| `logi-forge-cli` | List devices, validate config, run diagnostics |

The UI and overlay never open HID devices directly. All hardware writes go
through the agent so exclusive ownership, retries, and permission failures have
one source of truth.

The hosted bridge uses the Native Agent inventory as the authoritative device
list whenever at least one native device is detected. When no native device is
available, it keeps the Mock inventory as an explicit Demo fallback so the UI
remains usable without hardware. Native devices that do not yet expose a write
adapter are marked `hardwareWritable = false`, so the UI does not present their
mock controls as real hardware operations.

## Packages

| Package | Suggested stack | Notes |
| --- | --- | --- |
| `core` | Rust | Config, device model, action catalog, no I/O |
| `protocol` | Rust + generated TypeScript | Versioned IPC DTOs |
| `agent` | Rust + Tokio | Runtime orchestration and platform adapters |
| `device` | Rust | HID++ sessions, UVC, raw HID driver traits |
| `hook` | Rust | macOS CGEventTap, Linux evdev/uinput, Windows WH_MOUSE_LL |
| `inject` | Rust | CGEvent/uinput/SendInput/MPRIS |
| `desktop` | Tauri + TypeScript | Frontend; can reuse current prototype layout |
| `cli` | Rust clap | Diagnostics and config validation |

## Contracts

- Config is TOML, local-first, strict, and versioned.
- IPC is append-only after v1; breaking changes bump `PROTOCOL_VERSION`.
- Hardware capabilities gate UI panels; device kind is only a fallback.
- Every hardware write returns a typed error: unsupported feature, offline,
  permission denied, receiver busy, or transport failure.
- Mock agent implements the same protocol as the real agent.

## Current Transport

The hosted interview build uses protocol v1 over JSON HTTP commands plus SSE
snapshots. Every mutation carries the last observed revision, so concurrent
writes fail explicitly instead of silently overwriting state. The mock agent
persists state atomically under `.runtime/`; the future Rust agent can replace
the transport without changing the device DTOs or frontend capability gates.

## Native M2 Workspace

| Crate | Boundary |
| --- | --- |
| `logi-forge-core` | Host-independent device DTOs and typed errors |
| `logi-forge-hidpp` | HID++ framing, correlation, feature resolution, battery/DPI/SmartShift |
| `logi-forge-linux` | `/sys/class/hidraw` inventory and non-blocking `/dev/hidraw` transport |
| `logi-forge-cli` | Human-operated diagnostics and verified hardware writes |

The protocol crate accepts a `HidTransport`, so scripted reports and real Linux
hidraw use the same code path. No OpenLogi crate is linked into this workspace.

Mutating HTTP commands can be protected with `LOGI_FORGE_API_TOKEN`. The
browser receives a same-origin `HttpOnly` session cookie, while external
clients use `Authorization: Bearer <token>`.

## Native Agent Runtime

`logi-forge-agent` is a resident Rust process reached over a newline-delimited
JSON Unix socket. It polls Linux hidraw inventory and strict TOML configuration,
increments its revision only when either changes, and reports typed config/apply
status to the hosted Node transport. The Node layer keeps Mock devices available
for hardware-free interviews while exposing native status and real inventory in
the same protocol-v1 snapshot.

Hardware configuration is applied only when `LOGI_FORGE_DEVICE_PATH` identifies
the owned hidraw route. `LOGI_FORGE_DEVICE_INDEX` selects `ff` or receiver slot
`1..6`. This explicit route guard prevents the agent from guessing which device
behind a shared receiver should receive Fn-lock or lighting writes. Inventory
changes cause the saved configuration to be reapplied, covering disconnect and
reconnect without writing continuously while the device is stable.

Keyboard `fnLock`, `lighting`, and `brightness` PATCH requests now cross the
native boundary before the Mock presentation state commits. The Node mutation
queue performs a side-effect-free Mock validation first, then the native agent
checks its own revision, atomically replaces the runtime TOML, and applies the
setting when a hardware route is configured. A native hardware error aborts the
UI commit, so the visible state cannot claim a write the device rejected.

## M3 Input Pipeline

```text
physical evdev -> exclusive grab -> ActionRouter
                         |              |
                         |              +-> mapped button -> action -> uinput
                         +-> unmapped event -> virtual mouse passthrough -> uinput
```

The router consumes mapped button press/release events, ignores repeats, and
leaves all other device events intact. A tap emits a complete press/release
sequence; a hold action emits the press on `value=1` and release on
`value=0`. The Linux virtual device declares both source capabilities and all
target keys needed by custom shortcuts.

## M4 Profile Resolution

Profiles are owned by the Core router. A profile can match an application ID,
an executable path, or both. Matching both has the highest specificity, then
application ID, then executable path. Profile bindings overlay defaults, and
`ActionRouter::set_foreground_app` replaces the complete effective map under a
mutex so the capture thread never observes a partially updated profile.

## M3 Input Pipeline

```text
physical evdev -> exclusive grab -> ActionRouter
                         |              |
                         |              +-> mapped button -> action -> uinput
                         +-> unmapped event -> virtual mouse passthrough -> uinput
```

The router consumes mapped button press/release events, ignores repeats, and
leaves all other device events intact. A tap emits a complete press/release
sequence; a hold action emits the press on `value=1` and release on
`value=0`. The Linux virtual device declares both source capabilities and all
target keys needed by custom shortcuts.

## First Real Slice

1. Define `DeviceInventory`, `Capabilities`, `Action`, `Binding`, `Config`.
2. Implement mock agent with observable snapshot and command handlers.
3. Move the frontend prototype to Tauri and replace in-memory state with IPC.
4. Add Linux HID++ discovery for one Logitech mouse.
5. Add DPI read/write and SmartShift read/write with smoke diagnostics.
6. Add focused tests for config parsing, action serialization, and mock writes.
