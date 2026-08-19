# Design notes

Why catbus99 is shaped the way it is. Each of these was a real fork in the road.

## The governor is enforced by the compiler, not by convention

Every flash-endurance guarantee depends on all writes passing through one governor. That
has to be enforced, because "remember to call the governor" is exactly the kind of rule that
holds until someone adds a new client under time pressure — and here the new client might be
an autonomous agent in a retry loop.

Two designs were tried and **do not work in Rust**:

- **A capability token** — `upload(payload, permit: &WritePermit)`. The permit's constructor
  must be `pub` for the governor to mint one from another crate, which means anyone else can
  mint one too.
- **A Cargo feature** — gating the raw write behind `unchecked-writes`. Cargo features are
  *unified* across a build graph: the moment one crate enables it, every crate in the same
  build gets it.

Rust's privacy is per-crate, so the only way to enforce the invariant is to put the raw write
and the policy guarding it **in the same crate**. Hence `catbus99-device` contains both the
USB-HID transport and the governor, with `Device::upload_container` crate-private and
`Governor::upload_to_panel` as the only public door.

For belt and braces, the low-level `write_report` — used for config-channel commands like
setting the clock — refuses any report beginning `AA 50`, so a bulk image upload cannot be
hand-rolled out of individual report writes either.

There is no `--force` anywhere. A bound that can be routed around is not a bound. A refusal
explains itself and says when the write will be allowed.

## Precision is a cost

The flash budget is not a limit on how often catbus99 *runs*. Polling is free. It is a limit
on **how precisely it displays things**, because only a change in rendered pixels costs a
write.

This turns what looks like a formatting option into a safety control. `quantize_minutes`
defaults to 15 on time widgets not for taste but because a one-minute clock would exhaust the
panel in about 69 days. The same logic applies to every widget: a progress bar quantised to
5% steps costs a fraction of one tracking 0.1%, for a difference nobody can see at 160×96.

## A daemon owns the device

If the CLI, the scheduler and the MCP server each opened the device directly, each could
satisfy its own rate limit while together tripling the write rate, and each would keep its own
copy of the wear state so the odometer would undercount. Centralising also gives one place
that knows what is currently on screen, which is what makes change-skip work across clients.

The CLI refuses a direct write while a daemon is running, and a second daemon refuses to
steal a live socket, for the same reason.

## Duration is a rendering concern, not a protocol one

The container carries N frames but only **N−1 single-byte delays**, so no frame can be held
longer than 255 units. A cat that blinks every two seconds is not expressible as two frames
at any delay value.

catbus99 handles duration by picking a uniform tick and **duplicating** frames. The tick
starts at the GCD of the source durations — the largest tick that reproduces the timing
exactly, so as few frames as possible are duplicated — and coarsens only if the frame budget
demands it. When the budget is genuinely too small the animation is decimated evenly rather
than truncated, so the whole loop is still represented instead of just its beginning.

This has a second benefit. Because every delay byte then carries the same value, the
unresolved question of *which* frame an N−1 delay applies to stops mattering. The design
sidesteps an open protocol question instead of depending on an answer.

## A bitmap font, not a scaled outline font

An outline font was tried two ways and failed both, on the real panel:

- **Thresholded coverage** deleted glyph stems. Measured across 220 sizes, the face never
  rasterised better than 74% "crisp" (coverage near 0 or 1), and only at ~20px where the cap
  height is 13px — unusable on a 96px screen. At 8px nearly every pixel carries *partial*
  coverage, so a threshold removes the letterforms rather than sharpening them.
- **Alpha blending** was legible in a magnified PNG preview and grey mush on glass.

Neither was a tuning problem. An outline font makes a rasterisation decision that has no good
answer at this size; bitmap glyphs are defined *on* the pixel grid, so there is no decision to
get wrong. On a display this small, contrast beats letterform fidelity.

The same measurement produced a corollary: **dithering must be per-widget.** It helps
gradients and photographs and actively destroys flat colour art, adding high-frequency noise
where the source had clean regions. (catbus99 does not yet implement this properly — see the
README's status section.)

## Stale data must look stale

A reading past its TTL renders dimmed with `--` in place of text, and a missing binding
renders a placeholder rather than defaulting to zero.

On an interactive screen you can refresh and see. On a glanceable, non-interactive one, a
number that silently stopped updating is worse than no number — a zero looks exactly like a
real measurement of zero. This is the case most likely to regress invisibly, which is why the
visual regression harness has a scene dedicated to it.

## A preview can prove a layout wrong; it cannot prove one right

Twice during development a magnified PNG preview looked fine while the physical panel did
not. A 4× nearest-neighbour preview is *systematically* flattering: four times the physical
size, none of the panel's contrast behaviour.

The regression harness is therefore scoped honestly. It catches *changes* — a single stray
pixel — which is exactly what a regression test should do. It cannot tell you whether the
design is legible. Only the panel can.
