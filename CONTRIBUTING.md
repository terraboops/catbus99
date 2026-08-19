# Contributing

Bug reports, protocol findings, and hardware reports from other units are all welcome.

## Before you start

```sh
cargo test                                  # 205 tests, no keyboard required
cargo clippy --all-targets -- -D warnings
cargo fmt
```

No test requires hardware. If a change you make needs a keyboard to verify, say so in the PR
and describe what you saw on the panel — see below.

## The one rule that matters

**All writes to the display go through `Governor::upload_to_panel`.** The raw upload is
crate-private specifically so this is enforced by the compiler rather than by review. If you
find yourself needing to widen that visibility, please open an issue first — it is almost
certainly the wrong fix, and `docs/DESIGN.md` explains the two approaches that were already
tried and rejected.

The display's flash is rated for ~100,000 writes and cannot be replaced. Any feature that
increases how often the rendered image *changes* is spending a finite, shared resource.

## Verifying a visual change

A PNG preview can prove a layout is wrong; it cannot prove one is right. This bit us twice:
a magnified preview looked fine while the physical panel was illegible.

So for anything that alters rendering:

1. Run the regression harness — `cargo test -p catbus99-render --test regression`
2. If the change is intended, regenerate and **review the image diff before committing**:
   ```sh
   CATBUS99_BLESS=1 cargo test -p catbus99-render --test regression
   ```
   In the fixture alone a blessed regression looks identical to a blessed fix.
3. Look at it on a real panel if you have one, and say what you saw in the PR.

## Protocol findings

If you capture something new, please include the raw bytes and how you captured them.
`docs/PROTOCOL.md` distinguishes what is **verified** from what is **assumed**, and that
distinction is worth preserving — several upstream claims turned out to be wrong, and several
of ours will too.

Entries whose meaning is not established are preserved verbatim rather than guessed at. A
backup that silently mistranslates a key is worse than one that admits it does not know.

## Other hardware

catbus99 is tested on a single TH99 Pro (firmware V1.17) on macOS 15, Apple Silicon. If you
have a different unit, firmware, or OS, `catbus99 probe --json` output in an issue is genuinely
useful — especially if interface discovery fails.

## Style

Match the surrounding code. Comments explain *why*, not *what*; the codebase leans on this
heavily for the parts where the obvious approach is wrong.
