# OpenLogi Parity Matrix

Baseline: AprilNEA/OpenLogi `b32ae0872252a20353a2b8b51f1ef74b065c78f6`.

## First Prototype

| Area | Upstream behavior | Prototype status |
| --- | --- | --- |
| Device inventory | Receiver, direct, raw HID, camera inventories | Mocked mouse, keyboard, camera, light |
| Agent status | Protocol version, inventory health, permissions | Live HTTP/SSE status, revision, config reload |
| Device detail tabs | Capability-gated tabs | Implemented in frontend state |
| Mouse bindings | Button remap, gesture, long press, hold shortcut | UI state and TOML preview |
| Actions Ring | Eight-slot overlay, per-app layouts | Eight-slot editor mock |
| Pointer | DPI presets, SmartShift, scroll inversion/resolution | Interactive mock writes |
| Keyboard | F-row remap, Fn lock, RGB lighting | Interactive mock writes |
| Camera | UVC preview, controls, profiles | Preview visual and control state |
| Light | Litra power, brightness, temperature | Interactive mock writes |
| Config | Strict TOML schema, schema migrations, backups | Agent-generated TOML and atomic mock-state persistence |

## Native Rewrite Milestones

| Milestone | Scope | Definition of done |
| --- | --- | --- |
| M1 Contract | DTOs, config schema, mock agent, frontend IPC | Config model, validation, HTTP/SSE, Unix native IPC, frontend bridge, native snapshots, and keyboard native writes complete; mouse/camera/light mutations still transition from Mock |
| M2 Linux mouse | HID++ discovery, battery, DPI, SmartShift | Code and scripted tests complete; physical mouse smoke pending |
| M3 Capture/inject | Mouse button capture and action injection | Core routing, evdev grab, passthrough mirror, uinput actions, and scripted tests complete; physical smoke pending |
| M4 Profiles | Foreground app watcher and per-app overlays | Core resolver, TOML profiles, Linux X11 watcher, CLI profile bindings, and scripted tests complete; Wayland/overlay pending |
| M5 Keyboard | F-key capture, Fn lock, RGB | F1-F12 evdev capture, 0x1b04 media-key diversion, dual-feature Fn lock, and 0x8070/0x8081/0x8080 RGB complete with scripted tests; resident wake re-arm and physical smoke pending |
| M6 Camera/light | UVC controls and Litra controls | Camera profile and light write apply to hardware |
| M7 Cross-platform | macOS and Windows adapters | Platform-specific gates documented and tested |
| M8 Packaging | Installers, permissions, autostart | User install path works without dev commands |

## Stretch Goals Beyond OpenLogi

- Rule engine for context-aware automation.
- Dry-run mode that explains which config/profile/action will win.
- Capability recorder that generates shareable hardware reports.
- Profile import/export with schema validation.
- Safer conflict diagnostics for Logi Options+ receiver ownership.
