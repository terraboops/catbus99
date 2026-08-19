# catbus99

[![CI](https://github.com/terraboops/catbus99/actions/workflows/ci.yml/badge.svg)](https://github.com/terraboops/catbus99/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

The Epomaker TH99 Pro has a little 160×96 screen above the numpad. Out of the box it shows
a clock. The only supported way to change that is a Windows driver.

catbus99 drives it from macOS. Images, GIFs, progress bars, gauges, countdowns, fed by shell
scripts on a schedule or pushed by an AI agent. It talks to the keyboard exactly the way the
official driver does, over USB-HID. No firmware changes, no QMK, no reflashing.

![The keyboard screen showing usage bars, a gauge, a countdown and a sparkline](assets/screenshots/screen-4x.png)

*Shown at 4×. The green label came from an agent over MCP; everything else is a shell script
reporting load average every minute.*

```sh
catbus99 daemon &                        # owns the display
catbus99 set demo cpu 73 --render        # push a number, draw it
catbus99 image cat.gif --execute         # or just show a cat
claude mcp add catbus99 -- catbus99 mcp  # or hand it to an agent
```

## First, the catch

The screen's storage is a Puya PY25Q128HA, rated for about 100,000 erase cycles. You cannot
replace it. Every time the picture changes, you spend one.

That number sounds enormous until you do the division. A clock showing minutes updates 1,440
times a day, which works out to roughly **69 days** before you're out of budget. Two months
and your keyboard has a dead screen, and it died displaying a clock the keyboard could
already draw for free.

So catbus99 is built around not writing. The daemon never uploads a picture identical to the
one already on the panel, which means the cost isn't how often you poll, it's how precisely
you display. Round the clock to fifteen minutes and the pixels only change 96 times a day no
matter how often the data updates underneath.

| Displayed as | Changes/day | Screen lasts |
| --- | --- | --- |
| `20:17` | 1,440 | 69 days |
| `20:15` | 96 | 2.9 years |
| `~8pm` | 48 | 5.7 years |

Time widgets quantise to 15 minutes by default. Poll every second if you want, it's free.

```console
$ catbus99 wear
  uploads so far:   22
  budget used:      0.0220% of 100000 rated cycles
  uploads left:     99978
```

Everything that writes goes through one governor: change-skip, a rate limit with a hard
5-minute floor, a separate small allowance for when you're iterating by hand, and a counter
that persists across runs. Failed uploads count too, because a transfer that dies at report
9 of 16 still wrote nine reports worth of flash.

You cannot route around it. The raw upload function is private to its crate, so
`Governor::upload_to_panel` isn't the polite way to write to the screen, it's the only way
the compiler permits. There's no `--force` flag, on purpose. I tried two nicer-looking
designs first and both are broken in Rust: a capability token needs a public constructor, so
anyone can mint one, and a Cargo feature gets unified across the build graph the moment any
crate enables it. Privacy is per-crate, so the write and the policy have to live together.
[docs/DESIGN.md](docs/DESIGN.md) has the details.

## Install

You need a TH99 Pro on a **USB cable** (the screen isn't reachable over 2.4 GHz or
Bluetooth), Rust 1.82+, and macOS or Linux.

```sh
git clone https://github.com/terraboops/catbus99
cd catbus99
cargo build --release
./target/release/catbus99 probe
```

`probe` opens nothing and writes nothing. You want to see both interfaces:

```console
$ catbus99 probe
  iface  2  usage_page 0xff68  usage 0x0061  Epomaker TH99PRO
  iface  3  usage_page 0xff67  usage 0x0061  Epomaker TH99PRO
  OK: both interfaces identified unambiguously.
```

If it finds nothing, check the cable and close Epomaker's driver, which holds the interface
exclusively.

Then put something on the screen:

```sh
catbus99 selftest --execute --pattern
```

You should get four colour bands (red, green, blue, white top to bottom), a black square in
the **top-left**, and a one-pixel white border. That pattern is deliberately lopsided. Colour
bands catch a wrong row stride, the off-centre square catches a flip or rotation, and the
border catches an off-by-one. My first test pattern was solid black and solid white frames,
which look identical no matter how badly you've mangled the geometry. Lesson learned.

## Using it

### Pictures

```sh
catbus99 image photo.png --preview out.png   # render to a file, no flash write
catbus99 image cat.gif --execute
catbus99 image cat.gif --hold-first 2000 --execute
```

`--hold-first` is there because of a quirk. The container holds N frames but only N−1
one-byte delays, so nothing can stay on screen longer than 255 units. A cat that blinks every
two seconds simply isn't expressible as two frames. catbus99 gets long holds by repeating
frames, picking the largest tick that still reproduces your timing exactly so it repeats as
few as it can.

Without the flag, that blinking cat GIF plays at its source timing of 100ms per frame, which
is a cat having a seizure five times a second. Ask me how I know.

### Screens made of widgets

![Widget gallery](assets/screenshots/widgets-3x.png)

Layouts are TOML or JSON. Named slots on the 160×96 grid, one widget each:

```toml
id = "usage"
background = "#0a0c10"

[[slots]]
id = "session"
rect = { x = 2, y = 12, w = 156, h = 17 }

[slots.widget]
type = "progress_bar"
show_value = true
color = "#4ac8ff"
value = { kind = "data_point", source = "claude", key = "session_pct" }
label = { kind = "literal", value = "SESSION" }
```

There's a complete one in [`examples/layouts/usage.toml`](examples/layouts/usage.toml).

```sh
catbus99 layout examples/layouts/usage.toml --preview out.png
catbus99 layout examples/layouts/usage.toml --execute
catbus99 layout --schema > layout.schema.json
```

You get `label`, `progress_bar` (solid or segmented), `gauge`, `reset_timer`, `clock`,
`image`, `sparkline`, `fill` and `blank`. Widgets point at data rather than containing it, so
one layout renders to the panel, to a PNG, or to anything else, with values resolved when you
draw.

### Old data looks old

![Fresh versus stale rendering](assets/screenshots/stale-3x.png)

Every reading carries a TTL. Past it, the widget dims and the text becomes `--`.

This matters more on a keyboard than it would in a browser tab. You can't refresh a keyboard.
If a source dies at 2pm and the bar just sits there, you'll trust a number that stopped being
true hours ago, and nothing on the screen will tell you. Fresh and stale have to look
different at a glance.

### Feeding it your own data

Any program, any language. Exit 0, print JSON:

```sh
#!/usr/bin/env bash
echo '{"datapoints":[{"key":"session","value":0.62,"unit":"ratio","label":"SESSION"}]}'
```

```toml
# ~/.config/catbus99/sources.toml
[[source]]
id       = "claude"
command  = ["~/bin/claude-usage.sh"]
schedule = "0 */5 * * * *"   # cron, with a seconds field on the front
ttl_secs = 900
```

A subprocess instead of a plugin API, because a Rust plugin API would have meant everyone
writes Rust. Your existing zsh script works as-is, and you can debug it by running it. If a
source hangs, crashes, or prints garbage, it gets logged and your last-known values stay put
until their TTL runs out. One broken script shouldn't take the screen down.
[docs/SOURCES.md](docs/SOURCES.md).

### Letting an agent drive

```sh
claude mcp add catbus99 -- catbus99 mcp
```

Fourteen tools. The MCP server holds no device handle and contains no write logic, it just
talks to the daemon socket like everything else does, so an agent inherits the write limits
whether it wants to or not.

`preview_screen` renders a PNG and costs nothing, so a model can iterate as much as it likes.
Writes come back with the governor's actual verdict:

```json
{ "uploaded": false, "reason": "rate_limited",
  "detail": "next scheduled write allowed in 7m 12s",
  "retry_after_secs": 432, "uploads_remaining": 99978 }
```

A model that gets told "432 seconds" learns to batch. A model that gets told "error" retries
in a loop. `uploaded` always means bytes actually reached the panel, and a dry run reports
`would_upload` rather than lying about it. [docs/MCP.md](docs/MCP.md).

### Keymap backup

```sh
catbus99 keymap --out base-layer.json
catbus99 keymap --fn-layer
```

Read-only. The table turns out to be the firmware's 16×7 key matrix rather than the visual
layout, so the numpad is threaded through the middle of each row. Entries we can't identify
are kept byte-for-byte instead of guessed at, which means a backup restores exactly what it
read. Restore isn't implemented.

## Shape of the thing

```
   MCP client        CLI          cron schedule
        │             │                │
        └─────────────┴────────────────┘
                      │  unix socket, newline JSON
              ┌───────▼────────┐
              │   catbus99d    │
              │  data points   │
              │  layout        │
              │  renderer      │
              │  WRITE GOVERNOR│
              └───────┬────────┘
                      │  USB-HID
              MI_02 config · MI_03 TFT
```

One process owns the device and everyone else is a client. If the CLI, the scheduler and the
MCP server each opened it directly, all three could stay inside their own rate limit while
collectively tripling the write rate, and each would keep its own copy of the wear counter.

Six crates: `catbus99-proto` is the wire format and does no I/O at all, so the protocol is
fully testable without a keyboard. `catbus99-device` holds the USB transport and the governor
together, for the privacy reason above. Then `-model` (typed layouts), `-render` (compositor,
RGB565, bitmap text), `-daemon` (socket, scheduler) and `-mcp` (tools).

## Things we learned the hard way

The protocol notes live in [docs/PROTOCOL.md](docs/PROTOCOL.md), captured off the wire and
checked against the actual panel. A few highlights.

**Clearing the screen isn't a command.** It's a single-frame black image, and it costs a
write like anything else. If a tool tells you clearing is free, it's wrong.

**A still image only needs one frame.** The official driver sends every still as two
identical frames, 16 reports, 65,536 bytes. One frame displays exactly the same and costs
half that.

**Nothing gives the screen back.** There's no command anywhere in the driver's vocabulary
that returns the panel to its native clock. Once you upload an image you own the screen until
the keyboard is unplugged. Which is precisely why quantising matters: the keyboard could draw
a clock for free, and the moment you use catbus99 it can't.

**Outline fonts don't work at this size.** I measured 220 different sizes of a pixel font
through a rasteriser and it never came out better than 74% crisp, and only at 20px, on a
96-pixel-tall screen. Thresholding ate the stems. Blending looked great in a magnified preview
and like grey soup on the actual panel. catbus99 ships a 5×8 bitmap font instead: glyphs
defined on the pixel grid, integer scaling only, no anti-aliasing anywhere. At this size
contrast beats letterforms.

**macOS hidapi will abort your process** if the thread that called `hid_init()` exits while
you're still using the handle. Not an error, a SIGTRAP. Worth knowing if you write async Rust,
because tokio retires blocking-pool threads after ten idle seconds. catbus99 initialises on a
dedicated thread that parks forever and never dies.

## State of things

Working and verified on hardware: transport on macOS, images and GIFs, the widget compositor,
the governor and odometer, the source scheduler, the MCP server, clock sync, keymap backup.

Not built yet: keymap restore, launchd install, Homebrew and crates.io packaging, per-widget
dithering, and moving uploads off the async runtime (a long animation currently blocks the
daemon while it transfers).

Still unknown: what unit the frame delay byte is in. Every capture carried `0x19` while the
driver's own UI said 50ms, so animation timing is approximate. Repeating frames sidesteps the
question, since with a uniform tick it stops mattering which frame a delay belongs to.

Tested on exactly one keyboard: a TH99 Pro on firmware V1.17, macOS 15, Apple Silicon. CI runs
the whole suite on Linux and macOS, but no Linux box has driven a real panel yet, and the
tests are hardware-free by design so a green build proves the code works, not the screen. If
you have a different unit or firmware, `catbus99 probe --json` in an issue would be genuinely
useful.

## Hacking on it

```sh
cargo test                                    # 205 tests, no keyboard needed
cargo clippy --all-targets -- -D warnings
```

There's a visual regression harness. Scenes render at a fixed timestamp with fixed data and
get compared byte-for-byte against golden PNGs, and on failure it writes out the actual image
plus a diff with changed pixels in magenta.

```sh
CATBUS99_BLESS=1 cargo test -p catbus99-render --test regression
```

I checked that it can actually fail, which felt worth doing: nudging the stale-dimming factor
by 1%, invisible in a thumbnail, trips it with 740 changed pixels.

One warning if you send a patch that changes rendering. A preview can prove a layout is
broken but it can't prove one is good. Twice during this project a magnified PNG looked
completely fine and the real panel was unreadable. Look at the screen.

More in [docs/TESTING.md](docs/TESTING.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

## Careful

This writes to flash that wears out and can't be replaced, using undocumented firmware
commands, on a keyboard nobody officially supports doing this to. It never sends firmware,
reset, macro or lighting commands, and the only writes it makes are images, the clock, and
keymaps if you go implement restore. It ships with conservative limits and counts everything.
Still: your keyboard, your risk.

Unofficial. Not affiliated with Epomaker, who make a nice keyboard and did not ask for any of
this.

MIT, see [LICENSE](LICENSE). Third-party bits in [CREDITS.md](CREDITS.md).
