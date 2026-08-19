# Design notes

Why catbus99 is shaped the way it is. Each of these was a fork in the road where the obvious
option turned out to be wrong.

## The governor is enforced by the compiler

Every flash-endurance guarantee depends on all writes going through one governor. "Remember
to call the governor" is exactly the kind of rule that holds right up until someone adds a
new client in a hurry, and in this project the new client might be an autonomous agent
retrying in a loop.

Two designs looked better and don't work in Rust.

A **capability token**, where `upload()` demands a `&WritePermit`, fails because the permit's
constructor has to be public for the governor to mint one from another crate. If the governor
can make one, so can anybody.

A **Cargo feature** gating the raw write fails because features are unified across a build
graph. The moment one crate turns on `unchecked-writes`, every crate in that build gets it.

Rust privacy is per-crate, which leaves exactly one option: put the raw write and the policy
that guards it in the same crate. That's why `catbus99-device` contains both the USB-HID
transport and the governor, with `Device::upload_container` crate-private and
`Governor::upload_to_panel` as the only public door.

There's a second door I had to close. The low-level `write_report`, which exists for config
commands like setting the clock, will refuse any report starting with `AA 50`. Without that
you could hand-roll an image upload out of sixteen individual report writes and nothing would
count them.

No `--force` flag anywhere. A limit you can step around isn't a limit. When the governor
says no it tells you why and when to come back.

## Precision costs money

The flash budget isn't a limit on how often catbus99 runs. Polling is free. It's a limit on
how precisely you display things, because only a change in the rendered image costs a write.

That turns what looks like a formatting option into a safety control. `quantize_minutes`
defaults to 15 on time widgets, not for taste, but because a clock showing minutes would
exhaust the panel in about 69 days. The same maths applies everywhere: a progress bar in 5%
steps costs a twentieth of one tracking 0.1%, and at 160×96 nobody can see the difference.

## One process owns the device

If the CLI, the scheduler and the MCP server each opened the device, all three could sit
comfortably inside their own rate limit while together tripling the write rate. Each would
also keep its own copy of the wear counter, so none of them would be right.

Centralising also gives one place that knows what's currently on the panel, which is what
makes change-skip work across clients rather than per-process. The CLI refuses a direct write
while the daemon is up, and a second daemon refuses to take over a live socket, for the same
reason.

## Duration belongs to the renderer, not the protocol

The container carries N frames and only N−1 one-byte delays. Nothing can stay on screen for
longer than 255 units, so a cat that blinks every two seconds cannot be expressed as two
frames at any delay value.

catbus99 handles duration by choosing one uniform tick and repeating frames. The tick starts
at the GCD of the source durations, the largest value that reproduces your timing exactly, so
as few frames get duplicated as possible. It only coarsens if the frame budget forces it, and
when the budget is genuinely too small it thins the animation evenly instead of cutting it
off, so you still see the whole loop.

There's a bonus. Because every delay byte then holds the same value, it stops mattering which
frame a delay belongs to, which happens to be a question about this protocol we never
answered. The design sidesteps it rather than depending on a guess.

## A bitmap font, not a scaled one

I tried an outline font two ways and both failed on the actual panel.

Thresholding the coverage ate the stems. I measured 220 sizes and the face never rasterised
better than 74% crisp, and only at around 20px, where the cap height is 13 pixels on a
96-pixel screen. At 8px almost every pixel lands on partial coverage, so a threshold deletes
letterforms instead of sharpening them.

Alpha blending kept the stems and looked genuinely fine in a magnified preview. On glass it
was grey soup.

Neither was a tuning problem. An outline font forces a rasterisation decision that has no
good answer at this size. Bitmap glyphs are defined on the pixel grid, so there's no decision
to get wrong. Contrast beats letterform fidelity here, and it isn't close.

The same measurement produced a corollary about dithering: it has to be per-widget. It helps
photographs and gradients and it wrecks flat colour, scattering noise across regions that
were clean in the source. catbus99 doesn't implement that properly yet.

## Old data has to look old

A reading past its TTL renders dimmed with `--` instead of text, and a missing binding draws
a placeholder rather than falling back to zero.

On a web page you refresh and find out. You can't refresh a keyboard. If a source dies and
the widget keeps showing its last number, that number looks exactly like a live one, and a
zero looks exactly like a real measurement of zero. This is also the case most likely to break
without anyone noticing, since the fresh and stale paths differ by a colour multiply and a
string, which is why the regression harness has a scene dedicated to it.

## A preview can prove a layout wrong, never right

Twice during this project a magnified PNG looked completely fine while the physical panel was
unreadable. A 4× nearest-neighbour preview is flattering by construction: four times the size,
none of the panel's contrast behaviour.

So the regression harness is scoped honestly. It catches changes, down to a single pixel,
which is what a regression test is for. It cannot tell you whether a design is legible. Only
the screen can do that.
