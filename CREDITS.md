# Credits and third-party material

## Protocol discovery

The TH99 Pro's USB-HID protocol was first reverse-engineered by **[regenbildr]** in
[epomaker-th99-pro-AI-usage][up], from USB captures of Epomaker's official Windows driver.
That work established the `AA 50` TFT channel, the container layout, and the flash-endurance
concern that shapes this project's design.

catbus99 is an **independent implementation** written from protocol facts — byte layouts,
opcodes, and constants, which are not copyrightable. No upstream source code was copied.
Where our own captures disagree with the upstream notes, the differences are documented in
[docs/PROTOCOL.md](docs/PROTOCOL.md).

[regenbildr]: https://github.com/regenbildr
[up]: https://github.com/regenbildr/epomaker-th99-pro-AI-usage

## Prior art

[strodgers/epomaker-controller][rt100] (MIT) drives the Epomaker **RT100**'s screen on
Linux. It does not support the TH99 Pro — the RT100 speaks a different protocol family
(64-byte reports, `0xA5`/`0x25`/`0xAC` commands, checksums) — but reading it clarified how
Epomaker structures screen uploads in general.

[rt100]: https://github.com/strodgers/epomaker-controller

## Bundled font

`assets/fonts/` — **Spleen 5x8** by Frederic Cambus, BSD-2-Clause. See
[`assets/fonts/LICENSE.spleen`](assets/fonts/LICENSE.spleen). The glyph table in
`crates/catbus99-render/src/font5x8.rs` is generated from it by `tools/bdf2rs.py`.

<https://github.com/fcambus/spleen>

## Demo image

`assets/cat-blinking.gif` — ["Cat blinking"][catgif] from Wikimedia Commons,
**CC BY-SA 4.0**. Used only as a demo asset; it is not covered by this project's MIT
licence and retains its own.

[catgif]: https://commons.wikimedia.org/wiki/File:Cat_blinking.gif

## Trademark

Epomaker and TH99 are trademarks of their respective owners. This project is **unofficial**
and is not affiliated with, endorsed by, or supported by Epomaker.
