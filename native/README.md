# Logi Forge Native Agent M2

This workspace is the clean-room native device slice. It does not depend on
OpenLogi crates. Protocol behavior is implemented behind a transport trait and
tested against scripted HID++ reports before a real hidraw node is opened.

Implemented Linux scope:

- `/sys/class/hidraw` discovery filtered to Logitech vendor `046d`
- non-blocking `/dev/hidrawN` request/response transport
- HID++ 2.0 root ping and runtime feature resolution
- unified (`0x1004`) and legacy (`0x1000`) battery reads
- adjustable DPI (`0x2201`) read, supported-list expansion, write, and read-back verification
- enhanced (`0x2111`) and legacy (`0x2110`) SmartShift read/write
- direct devices (`ff`) and receiver slots (`1..6`)
- action parser and button-event router with tap/hold lifecycle semantics
- Linux evdev exclusive grab, passthrough mirror, and uinput injection
- browser/media actions plus arbitrary letter-key custom shortcuts
- application-aware profile selection by stable application ID and executable
- Linux X11 foreground watcher via `xdotool` with a graceful no-desktop fallback
- strict TOML-driven capture bindings and per-application profile overlays
- Linux F1-F12 evdev capture driven by the keyboard config section
- HID++ Fn-lock read/write with `0x40a3` to `0x40a2` fallback and read-back verification
- automatic RGB fallback across HID++ `0x8070`, `0x8081`, and `0x8080`
- HID++ `0x1b04` media-mode F-row diversion with verified arm/restore writes
- resident Unix-socket agent with inventory polling and guarded config reapply

Build and test:

```bash
cargo test --workspace
cargo run -p logi-forge-cli -- config validate ./examples/logi-forge.toml
cargo run -p logi-forge-cli -- config print ./examples/logi-forge.toml
cargo run -p logi-forge-cli -- list
cargo run -p logi-forge-cli -- probe /dev/hidraw3 --index ff
cargo run -p logi-forge-cli -- dpi get /dev/hidraw3
cargo run -p logi-forge-cli -- inject 'CustomShortcut: Ctrl+Shift+P'
cargo run -p logi-forge-cli -- capture /dev/input/event7 \
  --bind 275=BrowserBack \
  --bind 276='CustomShortcut: Ctrl+Shift+P'
cargo run -p logi-forge-cli -- capture /dev/input/event7 \
  --config ./examples/logi-forge.toml
cargo run -p logi-forge-cli -- foreground
cargo run -p logi-forge-cli -- keyboard capture /dev/input/event9 \
  --config ./examples/logi-forge.toml
cargo run -p logi-forge-cli -- keyboard divert /dev/hidraw5 \
  --config ./examples/logi-forge.toml --index ff
cargo run -p logi-forge-cli -- keyboard fn-lock get /dev/hidraw5 --index ff
cargo run -p logi-forge-cli -- keyboard fn-lock set /dev/hidraw5 on --index ff
cargo run -p logi-forge-cli -- keyboard lighting set /dev/hidraw5 18a06f --index ff
cargo run -p logi-forge-agent
cargo run -p logi-forge-cli -- capture /dev/input/event7 \
  --bind 275=BrowserBack \
  --profile 'editor:com.microsoft.VSCode=275:CustomShortcut: Ctrl+Shift+P'
```

Logitech Options+ and Logi Forge must not own the same receiver concurrently.
Install `packaging/99-logi-forge.rules` through a real package or copy it to
`/etc/udev/rules.d/` during local development, then reload udev rules. The
capture command grabs the source event device exclusively, forwards
unmapped mouse events through a virtual mirror, and replaces mapped buttons with
the configured action. Release the grab with Ctrl+C. The current sandbox has
no `/dev/input` or `/dev/uinput` devices, so physical smoke testing remains an
explicit acceptance item.

Profiles use `profile-id:app-id=button-code:action`. When at least one profile
is configured, capture polls the X11 foreground PID every 150 ms and swaps the
effective bindings atomically. The application ID reported by the Linux
adapter is the executable path, which is stable across process restarts. Run
`logi-forge foreground` to obtain the exact value for the current desktop. Wayland
desktop integrations are intentionally left behind a provider trait for the
next platform-specific adapter.

Keyboard capture handles the ordinary Linux F1-F12 evdev stream. The separate
`keyboard divert` command handles Signature/MX media-mode F4-F12 controls over
HID++ `0x1b04`, arms only controls the keyboard reports as divertable, and
restores firmware ownership when Enter stops the session. Automatic re-arm
after wireless sleep belongs to the upcoming resident agent lifecycle. RGB
selects `0x8070`, `0x8081`, or `0x8080` by runtime feature discovery and keeps
writes volatile to avoid EEPROM wear.

The resident agent reads `LOGI_FORGE_AGENT_SOCKET` and `LOGI_FORGE_CONFIG`.
Set `LOGI_FORGE_DEVICE_PATH=/dev/hidrawN` and optionally
`LOGI_FORGE_DEVICE_INDEX=ff|1..6` only after diagnostics identify the route the
agent should own. Without that explicit route it remains read-only and reports
`apply.status = disabled` through native IPC.

The hosted bridge forwards keyboard Fn-lock, color, and brightness writes to
the agent. Each write checks the native revision, updates the strict runtime
TOML through a temporary-file rename, and reapplies hardware before the hosted
Mock state commits.

TOML bindings accept portable mouse names (`Back`, `Forward`, `DpiToggle`,
`MiddleClick`, `LeftClick`, `RightClick`), Linux `BTN_*` names, or numeric
evdev codes. `None` captures and suppresses a known button; unknown
vendor-specific controls set to `None` are ignored by this adapter. Use
`logi-forge foreground` to capture the exact `app_id` for a TOML profile.
