# The MCP server

`catbus99 mcp` exposes the keyboard screen to agents over stdio.

```sh
catbus99 daemon &                 # must be running: it owns the device
claude mcp add catbus99 -- catbus99 mcp
```

`catbus99 mcp --install` prints the exact command and a config snippet.

## Why every tool is a thin client

The MCP server holds no device handle and contains no write logic. Each tool sends one
request over the daemon's Unix socket — the same socket the CLI uses. The daemon owns the
device, and the write governor lives *inside* the write path, so an agent inherits the
flash-endurance limits automatically. There is no path from a tool call to the panel that
skips the governor, and no override parameter anywhere in the surface.

## The tools

| Tool | Writes to flash? |
| --- | --- |
| `preview_screen` — render to PNG | no |
| `get_status`, `get_layout`, `get_data_points`, `get_wear_budget`, `list_sources` | no |
| `push_data_point` — publish a reading | no |
| `run_source` — run a data source now | no |
| `sync_clock` — set the keyboard RTC (config channel) | no |
| `set_widget`, `set_layout` — with `render: false` | no |
| `set_widget`, `set_layout` — with `render: true` | governed |
| `render_screen` — with `execute: true` | governed |
| `show_image` — display a PNG/JPEG/GIF | governed |
| `clear_screen` — blank the panel | governed |

## Design notes

**`preview_screen` is free and advertised first.** An agent can iterate on a layout
indefinitely against a PNG without touching flash, and every write-capable tool
description points back at it.

**Writes report the governor's verdict, not just success.** A refusal returns
`uploaded: false` with a `reason` and `retry_after_secs`, so a model can learn to batch
changes rather than retry blindly:

```json
{ "ok": true, "uploaded": false, "reason": "rate_limited",
  "detail": "rate limited: next scheduled write allowed in 7m 12s",
  "retry_after_secs": 432, "uploads_used": 21, "uploads_remaining": 99979 }
```

**`uploaded` always means bytes reached the panel.** A dry run reports
`reason: "would_upload"` with `uploaded: false`. Reporting `true` for work that did not
happen would be mildly confusing to a person and actively misleading to an agent, which
would carry on as though the screen had changed.

**`clear_screen` is honest about its cost.** The keyboard has no hardware clear; the tool
uploads a blank image and spends a write like any other change. Saying so in the
description stops an agent treating it as free cleanup.

**The server instructions carry the hardware's one irreversible property.** They are read
once, before any tool is chosen, so they state the finite write budget, the
preview-then-commit workflow, and the advice to round displayed values coarsely — the
guidance that most changes how an agent uses the screen.
