# TH99 Pro protocol

Protocol facts for the Epomaker TH99 Pro (`0C45:800A`, wired), captured from live WebHID
traffic of Epomaker's own driver on 2026-08-18 and confirmed by driving the display from
catbus99 on macOS.

Everything below was observed directly. Where something is inferred rather than seen, it
says so.

The one figure not from our own observation is the flash chip and its endurance rating,
which comes from a third-party teardown; see [CREDITS.md](../CREDITS.md).

## Interfaces

Identified on macOS by hidapi's `interface_number`, which **is** populated (no fallback
needed). The official driver opens exactly the same two, by usage page:

| Purpose | iface | usage_page | usage | Report size |
| --- | --- | --- | --- | --- |
| Config  | 2 | `0xff68` | `0x0061` | 64 bytes |
| TFT     | 3 | `0xff67` | `0x0061` | 4104 bytes |

Both are vendor-defined usage pages, so macOS grants access without an Input Monitoring
prompt. The keyboard's HID collections live on ifaces 0/1 and are untouched.

## Config channel (iface 2)

64-byte reports. Request `0xAA`, acknowledgement is the same bytes with `0xAA` → `0x55`.

```
byte 0    : 0xAA request / 0x55 reply
byte 1    : command
byte 2    : declared length
bytes 3-5 : 24-bit LE offset
byte 6    : final-packet flag
byte 7    : reserved
bytes 8.. : payload
```

Observed live during device bind and screen operations:

| Command | Meaning | Declared length |
| --- | --- | --- |
| `0x10` | read device info (reply embeds `0c 45` VID and `0a 80` PID LE) | `0x38` |
| `0x12` | read keymap table, paged by the 24-bit offset in bytes 3-5 (steps of `0x38`) | `0x38` |
| `0x34` | **set clock** | **`0x18`** |

**Worth pinning:** the live driver sends declared length **`0x18` (24)** here, not the
`0x38` used by the keymap reads. The keyboard echoes whatever it is sent, so the
acknowledgement does not validate this field, so a wrong value passes unnoticed.

`AA 34` payload at bytes 8..18: marker `5A 01 5A`, then `yy mm dd HH MM SS weekday`, all
**plain binary** (not BCD), weekday ISO Mon=1..Sun=7, year is `year - 2000`.

Live example: `aa 34 18 00 00 00 01 00 | 5a 01 5a 1a 08 12 13 1a 0a 02`
→ 2026-08-18 19:26:10, Tuesday.

## TFT channel (iface 3)

4104-byte output reports:

```
AA 50 | seq:u16le | count:u16le | 0x0650:u16le | <4096-byte block>
```

Every report is acknowledged with 64 bytes: `55 41 00 01` followed by 60 zeros.

**There is no init report, no footer, and no commit command.** A screen update is *only*
the `AA 50` report stream. A `Write Image` capture contains exactly 16 sends, all `AA 50`,
and nothing else on either interface.

### Container

Concatenated report bodies:

```
0..256    metadata
  [0]       N = frame count
  [1..N]    N-1 per-frame delay bytes
  [N]       0x00 terminator
  [N+1..]   0xFF fill
256..     N frames, each 160x96 RGB565 little-endian (30,720 bytes)
..        zero padding to a 4096-byte boundary
```

Verified live. `Clear Screen` sends `01 00 ff ff …` (one frame, 8 reports);
`Write Image` sends `02 19 00 ff ff …` (two frames, 16 reports).

- **Frame geometry is 160x96**, proven from the capture: the driver duplicates a still
  image into two identical frames, so the repeat period of the payload *is* the frame
  size, and it tests exactly 30,720 bytes.
- **No rotation or flip.** Pixels are row-major from the top-left, little-endian RGB565.
  (Contrast the Epomaker RT100, whose community driver must flip and rotate.)
- **Delay byte units are still UNCONFIRMED.** The driver's UI field is labelled "Frame
  Delay (ms)" (default 50), but every capture we took carried `0x19` = 25 on the wire,
  `Write Image` and both `Write Animation` runs alike. Scripting the UI field to 200 did
  not change the wire value, though that edit may simply not have reached the app's
  internal state. Upstream separately observed `0x32` and `0x0F`, so the field is
  certainly not a constant. Plausible readings are 1ms units (UI value halved for some
  reason) or 2ms units. **Calibrate empirically in Phase 2**: upload known delay values
  and time the screen. Until then, treat animation timing as approximate.
- **Maximum 250 frames** (the driver's UI states "1 / 250 Frames"), not the 254 the
  metadata layout would allow.

### Keymap table (`AA 12` base layer, `AA 16` Fn layer)

Read in 56-byte pages, the 24-bit LE offset in header bytes 3-5 stepping by `0x38`. Eight
pages cover the whole table; further pages read as zero.

The table is the firmware's key matrix, 16 columns by 7 rows, 112 entries. It is not the
visual layout. Each row of 16 holds a run of the main block followed by that row's numpad
keys, which is unambiguous once a full stock keymap is decoded: row 1 ends
`NumLock KP/ KP*`, row 2 ends `KP7 KP8 KP9`, row 3 ends `KP4 KP5 KP6`.

Each entry is four bytes:

| Entry | Meaning |
| --- | --- |
| `02 00 <usage> 00` | a standard HID Keyboard/Keypad usage (`0x29` Esc, `0x04` A, `0xE1` LeftShift) |
| `03 <id> 00 00` | a firmware function; ids `0xE9` and `0xEA` appear in the stock base layer, meanings unknown |
| `0d 00 00 <n>` | seen only on the **Fn layer** (Esc and Q/W/E/R, `n` = 1..5); meaning unknown |
| `00 00 00 00` | unassigned |

Verified by decoding a complete stock base layer: all 112 entries resolve, and the table
round-trips byte-for-byte. Entries whose meaning is not established are deliberately kept
verbatim rather than guessed at, so a backup restores exactly what was read.

`catbus99 keymap [--fn-layer] [--out file.json]` reads and decodes this. It is read-only;
restoring a keymap is a separate destructive operation and is not implemented.

### Complete observed command set

Captured from Epomaker's own driver across device bind, clock sync, clear, image write and
animation write. This is the **entire** vocabulary it uses:

| Command | Meaning |
| --- | --- |
| `AA 10` | read device info |
| `AA 12` | read keymap table (paged) |
| `AA 18` | read config table -- **all 19 pages read back as zero** on a stock unit (probably the macro table, empty) |
| `AA 34` | set RTC |
| `AA 50` | TFT image upload |

**There is no display-mode command.** Nothing switches the panel back to its native
clock/status screen. The driver's UI offers only Version and Factory Reset besides the
screen editor. Once an image is uploaded the host owns the screen until a power cycle
(which restores the native screen) or a factory reset.

This has a hard consequence: **the keyboard cannot be asked to render the time itself
while catbus99 is using the screen.** Any clock we show is an image we wrote.

### Clearing the screen

**There is a clear-screen operation.** It is the driver's `Clear Screen` is simply a **single-frame all-black image upload**
(`01 00 ff…`, 8 reports). It is an ordinary upload and costs a flash write like any other.

### Still images: one frame or two?

The official driver writes stills as two identical frames (16 reports, 65,536 bytes). A
single-frame container is also valid and displays fine, which `Clear Screen` proves, and it
costs half the bytes (8 reports, 32,768 bytes). catbus99 should prefer one frame for
stills.

## Practical note: designing a test pattern

Do not test with two *differing* frames. Two frames alternating at 25-50ms is a 20Hz
strobe, not a still, and it is easy to misread as "the screen is broken" or "nothing
happened". Use identical frames plus an asymmetric, multi-colour pattern: colour bands
reveal stride and channel-order faults, an off-centre marker reveals flips and rotations,
and a one-pixel border reveals off-by-one errors. `catbus99 selftest --pattern` does this.
