# Protocol Sources

The native rewrite implements protocol behavior independently and does not link
OpenLogi crates. Wire behavior was cross-checked against:

- Logitech HID++ 2.0 feature specifications
- Linux `hid-logitech-hidpp` driver behavior
- Solaar protocol documentation
- the local OpenLogi baseline as a product-behavior reference

Relevant M2 feature IDs are Root `0x0000`, Unified Battery `0x1004`, legacy
Battery Status `0x1000`, Adjustable DPI `0x2201`, enhanced SmartShift `0x2111`,
and legacy SmartShift `0x2110`.

No OpenLogi brand assets are used. OpenLogi's own source and its forked HID++
library retain their upstream licenses in `/workspace/upstreams/OpenLogi` and
are not copied into this native workspace.
