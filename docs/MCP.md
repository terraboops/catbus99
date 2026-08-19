# The MCP server

`catbus99 mcp` exposes the keyboard screen to agents over stdio.

```sh
catbus99 daemon &                 # must be running: it owns the device
claude mcp add catbus99 -- catbus99 mcp
```

`catbus99 mcp --install` prints the exact command and a config snippet.

## Why every tool is a thin client

The MCP server holds no device handle and contains no write logic. Each tool sends one request over the
daemon's Unix socket, the same socket the CLI uses. The daemon owns the device and the write
governor lives inside the write path, so an agent inherits the limits whether it wants them
or not. No tool call reaches the panel without passing the governor, and nothing in this
surface overrides it.

## The tools

| Tool | Writes to flash? |
| --- | --- |
| `preview_screen` (render to PNG) | no |
| `get_status`, `get_layout`, `get_data_points`, `get_wear_budget`, `list_sources` | no |
| `push_data_point` (publish a reading) | no |
| `run_source` (run a data source now) | no |
| `sync_clock` (sets the keyboard RTC, config channel) | no |
| `set_widget`, `set_layout` with `render: false` | no |
| `set_widget`, `set_layout` with `render: true` | governed |
| `render_screen` with `execute: true` | governed |
| `show_image` (PNG, JPEG or GIF) | governed |
| `clear_screen` (blanks the panel) | governed |

## Design notes

`preview_screen` is free and named first in every write tool's description. An agent can
iterate on a layout as long as it likes against a PNG without touching flash.

Writes report the governor's verdict rather than plain success or failure. A refusal comes
back as `uploaded: false` with a reason and a number of seconds, which is enough for a model
to batch its changes instead of retrying:

```json
{ "ok": true, "uploaded": false, "reason": "rate_limited",
  "detail": "rate limited: next scheduled write allowed in 7m 12s",
  "retry_after_secs": 432, "uploads_used": 21, "uploads_remaining": 99979 }
```

`uploaded` always means bytes reached the panel. A dry run reports `would_upload` with
`uploaded: false`. Saying `true` for work that never happened is mildly confusing to a person
and actively misleading to an agent, which will carry on as though the screen changed.

`clear_screen` is honest about its cost. The keyboard has no hardware clear, so the tool
uploads a blank image and spends a write like anything else. Saying that in the description
is what stops an agent treating it as free cleanup.

The server instructions get read once, before any tool is chosen, which makes them the right
place for the finite write budget, the preview-then-commit workflow, and the advice to round
values coarsely. Tool descriptions shape individual calls; instructions shape strategy.
