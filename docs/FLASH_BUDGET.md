# Flash budget and display resolution

The display's flash is rated at 100,000 program/erase cycles, and we conservatively count
one upload as one cycle. That budget is not really a limit on *how often catbus99 runs* —
it is a limit on **how precisely it displays things**.

## Why precision is a cost

The write governor never uploads a container byte-identical to the last one. So the cost
of a widget is not how often it is polled, but **how often its rendered pixels change** —
which is set by the resolution it displays at.

A clock makes this vivid:

| Displayed resolution | Image changes/day | Projected panel life |
| --- | --- | --- |
| `20:17` — 1 minute | 1,440 | **~69 days** |
| `20:15` — 5 minutes | 288 | ~1 year |
| `20:15` — 15 minutes | 96 | ~2.9 years |
| `~8pm` — 30 minutes | 48 | ~5.7 years |

A minute-resolution clock would destroy the display in about two months. The same
arithmetic applies to every widget: a progress bar quantised to 5% steps costs a fraction
of one tracking 0.1%, for a difference no one can see at 160x96.

## The rule

**Round every displayed value to the coarsest resolution that is still useful, and the
change-skip rule enforces the budget for free.**

Poll as often as you like — polling is cheap and keeps the data fresh for the moment a
write does happen. It is the *rendering* that must be coarse.

## Why we cannot delegate the clock to the keyboard

The keyboard has a native clock screen that updates itself in firmware at zero cost. It
cannot be used while catbus99 owns the screen: the complete captured command set of the
official driver (`AA 10/12/18/34/50`, see `PROTOCOL.md`) contains **no display-mode
command**. Uploading an image takes the screen until the keyboard is power-cycled, and
nothing switches it back.

`AA 34` sets the keyboard's RTC, but that only affects the native screen we are no longer
showing. It remains worth calling — the RTC drifts ~10s/day — so the native clock is
correct whenever the user does power-cycle back to it.
