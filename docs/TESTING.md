# Testing

```sh
cargo test                      # everything
cargo clippy --all-targets -- -D warnings
```

No test requires a keyboard. Hardware-touching code is exercised for *shape* — that a
probe returns, that an open either succeeds or fails cleanly — never for a successful
upload, so the suite is the same with the keyboard unplugged.

## The visual regression harness

`crates/catbus99-render/tests/regression.rs` renders a set of scenes at a **fixed
timestamp** with **fixed data** and compares each byte-for-byte against a committed golden
PNG in `tests/fixtures/`.

Rendering is deterministic, so exact comparison is correct and strictly stronger than a
perceptual threshold: it catches a single stray pixel, which at 160x96 is a real defect.

On failure the harness writes `<scene>.actual.png` and `<scene>.diff.png` beside the
golden, with changed pixels in magenta, so a regression is *visible* rather than merely
reported.

### Regenerating goldens

```sh
CATBUS99_BLESS=1 cargo test -p catbus99-render --test regression
```

Blessing rewrites every golden, so **review the image diff before committing**. In the
fixture alone a blessed regression looks identical to a blessed fix; the harness earns its
keep by making the change show up in review.

### The scenes

| Scene | Guards against |
| --- | --- |
| `widget_gallery` | every widget rendering together |
| `degraded_data` | stale and missing data staying visibly distinct from fresh |
| `text_sizes` | glyph rendering, alignment, and shrink-to-fit |
| `bar_extremes` | off-by-one at 0%, 1%, 50%, 99%, 100% |
| `degenerate` | zero-span gauge, flat sparkline, elapsed timer, missing image, out-of-bounds slot |
| `colour_quantisation` | the RGB565 round trip, including near-black and near-white |

`degraded_data` is the one most likely to regress silently: the fresh and stale paths
differ only by a colour multiply and a placeholder string, so a mistake there produces a
screen that looks fine and quietly lies about how current its numbers are.

**The harness is self-tested.** Changing the stale-dim factor from 0.35 to 0.36 — a 1%
shift, invisible in a thumbnail — fails `degraded_data` with 740 changed pixels.

## What each suite covers

| Crate | Focus |
| --- | --- |
| `catbus99-proto` | container/report wire format, clock packets, keymap decoding, property tests |
| `catbus99-device` | governor rules, odometer persistence, **hidapi threading**, path resolution |
| `catbus99-model` | serialisation, binding resolution, staleness, layout linting |
| `catbus99-render` | colour, fitting, animation timing, compositing, text, visual regression |
| `catbus99-daemon` | control protocol wire format, source subprocess contract, cron scheduling, socket integration |
| `catbus99-mcp` | the JSON an agent sees |

## Two suites worth understanding

**`probe_isolation.rs`** exists because of a crash, not a hypothesis. On macOS libhidapi
ties its IOKit state to the thread that calls `hid_init()`; if that thread exits, later use
aborts the process with SIGTRAP. These tests reproduce it directly and would fail — by
aborting — if the dedicated init thread were removed.

**`sources.rs`** treats a data source as hostile: non-zero exit, unparseable output, a
hang, a missing executable, and 100 KB of stderr. A source is arbitrary user code, and one
misbehaving must never take out the screen.
