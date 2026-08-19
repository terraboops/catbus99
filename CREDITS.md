# Credits and third-party material

## Hardware specifications

The display's storage is identified as a Puya PY25Q128HA (16 MB SPI flash, 100,000
program/erase cycles) in Gough Lui's [teardown of the TH99 Pro][teardown], which is also the
source for the RTC part and its measured drift. We have not opened the keyboard ourselves,
so those figures come from that write-up rather than from our own measurement.

Everything else in [docs/PROTOCOL.md](docs/PROTOCOL.md) was captured off the wire from
Epomaker's own driver and verified against the panel.

[teardown]: https://goughlui.com/2026/05/05/review-teardown-epomaker-th99-pro-usb-2-4g-bt5-0-hot-swap-96-keyboard-w-lcd-knob-rgb-leds/

## Bundled font

`assets/fonts/` is Spleen 5x8 by Frederic Cambus, BSD-2-Clause. See
[`assets/fonts/LICENSE.spleen`](assets/fonts/LICENSE.spleen). The glyph table in
`crates/catbus99-render/src/font5x8.rs` is generated from it by `tools/bdf2rs.py`, which is
kept in the repo so the table can be regenerated or swapped rather than sitting there as an
opaque blob.

<https://github.com/fcambus/spleen>

## Demo image

`assets/cat-blinking.gif` is ["Cat blinking"][catgif] from Wikimedia Commons, CC BY-SA 4.0.
It is a demo asset only and keeps its own licence rather than this project's MIT.

[catgif]: https://commons.wikimedia.org/wiki/File:Cat_blinking.gif

## Other Epomaker screens

If you have an RT100 rather than a TH99 Pro, [strodgers/epomaker-controller][rt100] (MIT)
drives that one on Linux. It speaks a completely different protocol (64-byte reports,
`0xA5`/`0x25`/`0xAC` commands, per-report checksums) and does not support the TH99 Pro.

[rt100]: https://github.com/strodgers/epomaker-controller

## Trademark

Epomaker and TH99 are trademarks of their respective owners. This project is unofficial and
is not affiliated with, endorsed by, or supported by Epomaker.
