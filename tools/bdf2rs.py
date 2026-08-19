#!/usr/bin/env python3
"""Convert a 5x8 BDF bitmap font into the Rust table in catbus99-render.

Usage: python3 tools/bdf2rs.py spleen-5x8.bdf > crates/catbus99-render/src/font5x8.rs

Kept in the repo so the embedded glyph table can be regenerated or replaced rather
than being an opaque blob.
"""
import re, sys, pathlib

src = pathlib.Path(sys.argv[1]).read_text(errors="replace")
glyphs = {}
for block in src.split("STARTCHAR")[1:]:
    m = re.search(r"^ENCODING (-?\d+)", block, re.M)
    bm = re.search(r"^BITMAP\s*$(.*?)^ENDCHAR", block, re.M | re.S)
    if m and bm:
        glyphs[int(m.group(1))] = [r.strip() for r in bm.group(1).strip().splitlines() if r.strip()]

lo, hi = 32, 126
rows_out = []
for cp in range(lo, hi + 1):
    vals = [int(r, 16) & 0xFF for r in glyphs.get(cp, [])[:8]]
    vals += [0] * (8 - len(vals))
    rows_out.append("    [%s], // %s" % (", ".join("0x%02X" % v for v in vals), chr(cp)))
print("\n".join(rows_out))
