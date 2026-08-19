#!/usr/bin/env bash
# A catbus99 data source.
#
# The contract: exit 0 and print one JSON object on stdout. Any executable in any
# language qualifies -- this one is deliberately shell to make that point.
set -euo pipefail

if [[ "${1:-}" == "--describe" ]]; then
  cat <<'JSON'
{"datapoints": [
  {"key": "session", "value": 0, "label": "Session usage 0..1"},
  {"key": "weekly",  "value": 0, "label": "Weekly usage 0..1"},
  {"key": "cpu",     "value": 0, "label": "CPU load percent"},
  {"key": "resets_at", "value": "", "label": "Next reset (RFC3339)"}
]}
JSON
  exit 0
fi

# Real load average, normalised to a 0..1 bar against core count.
cores=$(sysctl -n hw.ncpu 2>/dev/null || echo 8)
load=$(uptime | sed 's/.*load averages*: //' | awk '{print $1}' | tr -d ,)
cpu=$(awk -v l="$load" -v c="$cores" 'BEGIN { p = (l / c) * 100; if (p > 100) p = 100; printf "%.1f", p }')
session=$(awk -v l="$load" -v c="$cores" 'BEGIN { printf "%.3f", (l / c) }')
resets=$(date -u -v+2H '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || date -u '+%Y-%m-%dT%H:%M:%SZ')

cat <<JSON
{"datapoints": [
  {"key": "session",   "value": $session, "unit": "ratio", "label": "SESSION"},
  {"key": "weekly",    "value": 0.35,     "unit": "ratio", "label": "WEEKLY"},
  {"key": "cpu",       "value": $cpu,     "unit": "%",     "label": "CPU"},
  {"key": "resets_at", "value": "$resets"}
]}
JSON
