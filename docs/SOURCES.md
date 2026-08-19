# Data sources

A **source** is any executable that prints readings as JSON. catbus99 runs it on a cron
schedule and binds the results into your layout.

## The contract

Your program is invoked with no arguments. It must **exit 0** and print one JSON object on
stdout:

```json
{"datapoints": [
  {"key": "session", "value": 0.62, "unit": "ratio", "label": "SESSION"},
  {"key": "resets_at", "value": "2026-08-19T22:00:00Z"}
]}
```

| Field | Required | Meaning |
| --- | --- | --- |
| `key` | yes | Reading name, unique within the source |
| `value` | yes | Number, string, boolean, or an RFC 3339 timestamp |
| `unit` | no | Shown next to the value, e.g. `"%"` |
| `label` | no | Human-readable name |
| `ttl_secs` | no | Overrides the source's default freshness window |

catbus99 stamps the source id and observation time itself; you supply only the reading.

**Why a subprocess and not a plugin API:** a Rust plugin interface would have restricted
contributors to Rust. This way your existing shell, Python, Swift, or Node script works
untouched, and a source can be tested by running it in a terminal.

## Registering one

```toml
# ~/.config/catbus99/sources.toml
[[source]]
id         = "claude"
command    = ["~/bin/claude-usage.sh"]   # argv; a leading ~ is expanded
schedule   = "0 */5 * * * *"             # cron WITH seconds (6 fields)
timeout_ms = 10000                       # default
ttl_secs   = 900                         # default freshness window
```

The schedule is a six-field cron expression — the leading field is **seconds**, so
`0 */5 * * * *` means "every five minutes on the minute", not "every five seconds".

```sh
catbus99 sources              # list sources and their last run
catbus99 run-source claude    # run one now
```

## Failure is contained

A source that exits non-zero, hangs, or prints nonsense produces an error in the daemon log
and **leaves your previous readings in place** to age out on their own TTL. One broken
source must never take out the screen.

```console
$ catbus99 sources
  claude    0 */5 * * * *    FAILED: exit 3: the api key expired
```

stderr is captured (truncated) into that message, because it is what tells you *why*.

## Freshness

Every reading carries a TTL. Past it, widgets bound to it render **dimmed** with `--` in
place of text.

This matters more than it sounds. On a glanceable, non-interactive display, a number that
has silently stopped updating is worse than no number at all — nothing tells the reader it
went stale. Set `ttl_secs` to a little more than your schedule interval.

## Polling is free; rendering is not

The scheduler runs sources and re-renders, then the **governor** decides whether the result
is worth a flash write. Those are separate on purpose.

Poll as often as you find useful — it costs nothing. A one-minute poll against a
fifteen-minute write floor is not waste: it means the image that *does* get written reflects
data seconds old rather than fifteen minutes old.

What costs flash is the *rendered image changing*. Round your values coarsely and the
governor's change-skip rule does the rest. See [FLASH_BUDGET.md](FLASH_BUDGET.md).

## A worked example

`examples/demo-source/demo-source.sh` reports real load average as a 0..1 ratio and a CPU
percentage. It is deliberately written in shell to make the point that any language works.
