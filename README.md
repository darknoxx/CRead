# CRead

A writing terminal that pretends it is 1987.

Fullscreen, 80x25, DOS blue, a block cursor and nothing else. Text is drawn into
an emulated raster and blown up by an integer factor, so the scanlines are
always exactly one emulated pixel tall — on a 1080p laptop, a 4K panel or an
ultrawide.

```
┌──────────────────────────────────────────────────────────────┐
│ welcome.md                                saved · sync idle   │
├──────────────────────────────────────────────────────────────┤
│ # Welcome to CRead                                            │
│                                                               │
│ A writing terminal that pretends it is 1987.                  │
│                                                               │
│ ## Getting started                                            │
│                                                               │
│ - `Ctrl+S` saves. Everything in ~/Documents/CRead is plain     │
│   Markdown.                                                   │
└──────────────────────────────────────────────────────────────┘
```

## Build and run

Requires Rust (2024 edition) and, on Linux, the usual Wayland/X11 development
headers.

```bash
cargo build --release
./target/release/cread              # fullscreen, opens the last document
./target/release/cread notes.md     # or a specific file
```

To have it on `$PATH` (this is a workspace, so point at the binary crate):

```bash
cargo install --path crates/app
cread
```

## Keys

| Key | Action |
| --- | --- |
| `Ctrl+S` | Save (autosaves 1.5 s after you stop typing) |
| `Ctrl+P` | Open the file palette / start a new document |
| `Ctrl+Z`, `Ctrl+Shift+Z` | Undo, redo |
| `Ctrl+Q`, `Esc` | Quit |
| `F1` | Cycle theme: DOS blue, amber, green, paper |
| `F2` `F3` `F4` | Toggle scanlines, bloom, flicker |
| `F8` | Typewriter scrolling (the current line stays put) |
| `F9` | WebDAV settings |
| `F11` | Toggle fullscreen |

## How the picture is built

The emulated resolution is never scaled fractionally. For a physical surface of
`W x H` and an integer factor `n`:

```
virtW = W / n            (integer division)
virtH = H / n
```

The frame is rendered at `virtW x virtH`, upscaled by exactly `n` and centred;
the leftover border (always fewer than `n` pixels) stays black. Text is drawn
into that small raster with the original IBM VGA 8x16 bitmap face, then the GPU
adds the CRT layer: scanlines keyed off the *emulated* row, a local phosphor
bleed, a slow luminance drift and a gentle vignette.

On a 1280x800 panel that gives 640x400 at 2x — the real VGA text mode. On a
2560x1080 ultrawide it is 1280x540. Automatic scaling targets a comfortable line
height and follows the monitor's scale factor, so 4K at 200% gets 4x rather than
2x. Override it with `--scale N`.

The app renders at 30 fps while you type and drops to 2 fps when idle, so it is
nearly free to leave open.

## Documents and sync

Documents are plain Markdown files in `~/Documents/CRead` (override with
`--vault DIR` or `CREAD_VAULT`). Writes are atomic: temp file, fsync, rename. A
crash leaves either the old file or the new one, never half a page.

WebDAV sync mirrors that folder. Press `F9` and fill in:

| Field | Example |
| --- | --- |
| server | `https://cloud.example.org/remote.php/dav/files/me` |
| user | your account name |
| password | an app password |
| folder | `cread` |

Settings land in `~/.config/cread/sync.json` with `0600`. If you would rather not
have the password on disk, leave it empty and export
`CREAD_WEBDAV_PASSWORD` instead.

Sync runs at startup, after every save (debounced), and every 60 seconds. The
conflict rule is the whole point:

| local | remote | result |
| --- | --- | --- |
| unchanged | unchanged | nothing |
| changed | unchanged | upload |
| unchanged | changed | download |
| changed | changed | **keep both** — local text stays, the remote version is saved as `name.conflict-<unix>.md` |

Nothing is ever deleted on the server. `cread --sync-once` runs a single round
and prints the result, which is handy for cron or for debugging.

## Flags worth knowing

```
--scale N         force the integer upscale factor
--theme NAME      dos-blue | amber | green | paper
--no-crt          disable all effects
--backend NAME    vulkan | gl
--size WxH        window size (also drives --screenshot)
--screenshot PATH render one frame to PNG and exit
```

## Platform notes

Linux is the target (Wayland and X11 both work). macOS should build with
`cargo install --path crates/app` and can be started from a shell alias; no app
bundle needed. GPU-wise, anything Vulkan, Metal or OpenGL works — the app also runs on
a software rasteriser (llvmpipe) without complaint, which is how it is developed.

## Layout

```
crates/core      buffer, cursor, soft wrap, undo, markdown spans, file IO
crates/render    emulated framebuffer, bitmap font, wgpu CRT pass
crates/sync      WebDAV client, conflict policy, background worker
crates/app       window, input, frame loop, UI composition
assets/fonts     IBM VGA 8x16 glyph data
```

`crates/core` has no GPU, window or network dependency, so the tricky parts are
plain unit tests. `cargo test` runs all of them.

## Credits

The character set is the IBM VGA 8x16 ROM font from VileR's *Ultimate Oldschool
PC Font Pack* (CC BY-SA 4.0, <https://int10h.org/oldschool-pc-fonts/>), reduced to
the raw 256 x 16 byte bitmap. The handful of glyphs CP437 never had (`€`, `„“”`,
`–—`, `…`) are hand-drawn in `crates/render/src/font.rs`.
