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

## Download a build

Every push to `main` builds a ready-to-run Linux binary in CI:

1. Open the **Actions** tab and pick the newest *Release build* run.
2. Download the `cread-linux-x86_64` artifact.
3. `tar xzf cread-linux-x86_64.tar.gz && ./cread`

Tagged releases also publish the tarball on the
[Releases](https://github.com/darknoxx/CRead/releases) page, which is the
easiest way to get a build without a Rust toolchain. The Linux binary is made
against glibc 2.35 and only needs symbols up to `GLIBC_2.34`, so it runs on
Ubuntu 22.04 and newer.

To have it on `$PATH` (this is a workspace, so point at the binary crate):

```bash
cargo install --path crates/app
cread
```

## Two modes

On launch you get a greeting and an empty prompt:

```
                             Welcome to CRead

                        type new or open to start
```

`new` opens an empty document, `open` shows the vault. Pass a file name on the
command line and it opens straight away.

Then you are in **write mode**: the document is live, the block cursor sits in
the text, and nothing else competes for your attention. `Esc` switches to **action
mode**, where the bottom line turns into a command prompt, like a DOS shell.
`Enter` runs the command, `Esc` on an empty prompt goes back to writing.

```
> save▌                                          save, saveas, sync, open …
```

`Tab` completes the command, the arrow keys walk history, and a dimmed hint on
the right tells you what the command does before you commit to it.

| Command | Aliases | What it does |
| --- | --- | --- |
| `save` | `w`, `safe` | write the document to disk |
| `saveas <name>` | `sa` | save under a new name in the vault |
| `open [name]` | `o`, `e` | file list, or open a name directly |
| `new` | `n` | start an empty document |
| `rename <name>` | `mv`, `rn` | rename the current document |
| `delete` | `rm`, `del` | delete it, after a `y/n` |
| `find [text]` | `search` | search the document (`Ctrl+F`) |
| `sync` | | push the vault to WebDAV now |
| `theme [+\|-\|name]` | | next/previous, or black, dos-blue, amber, green, paper |
| `scanlines [on\|off]` | `scan`, `sl` | scanline strength |
| `bloom [on\|off]` | | phosphor glow |
| `typewriter [on\|off]` | `tw` | keep the current line in place |
| `fullscreen` / `windowed` | `fs` | |
| `undo` / `redo` | `u` | also on `Ctrl+Z` / `Ctrl+Shift+Z` |
| `settings` | `set`, `cfg` | WebDAV setup |
| `help` | `?`, `h` | the list above, on screen |
| `about` | | version and credits |
| `quit` | `q`, `x` | save and exit |

Autosave runs 1.5 seconds after you stop typing, so a forgotten `save` costs at
most a sentence.

## Selection

`Shift` plus any movement key marks text, `Ctrl+A` takes everything, and the
usual `Ctrl+C` / `Ctrl+X` / `Ctrl+V` work with the system clipboard. Marked text
is drawn inverted, and typing or `Backspace` replaces it. The mouse does the
obvious thing: click places the cursor, dragging marks, the wheel scrolls.

## Search

`Ctrl+F` (or `find`) opens a `/` prompt. Matches are marked as you type and the
cursor jumps to the first hit at or after where you were. `F3` and `Shift+F3`
walk the hits, also after the prompt is closed; `Enter` keeps the cursor where
it landed, `Esc` puts it back.

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
bleed and a gentle vignette. Nothing animates: the picture is static until the
text or the cursor changes.

On a 1280x800 panel that gives 640x400 at 2x — the real VGA text mode. On a
2560x1080 ultrawide it is 1280x540. Automatic scaling targets a comfortable line
height and follows the monitor's scale factor, so 4K at 200% gets 4x rather than
2x. Override it with `--scale N`.

There is no frame clock: a frame is drawn when the text changes and otherwise
only when the cursor is due to blink, so the app sits at a fraction of a percent
of a core while you read. Typing redraws immediately.

## Documents and sync

Documents are plain Markdown files in your XDG document folder — `~/Dokumente/CRead`
on a German desktop, `~/Documents/CRead` in English. Override it with `--vault DIR`
or `CREAD_VAULT`. Writes are atomic: temp file, fsync, rename. A
crash leaves either the old file or the new one, never half a page.

WebDAV sync mirrors that folder. Run `settings` in action mode and fill in:

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
--theme NAME      black | dos-blue | amber | green | paper
--no-crt          disable all effects
--backend NAME    vulkan | gl
--size WxH        window size (also drives --screenshot)
--screenshot PATH render one frame to PNG and exit
```

## Platform notes

Linux is the target (Wayland and X11 both work). macOS should build with
`cargo install --path crates/app` and can be started from a shell alias; no app
bundle needed. GPU-wise, anything Vulkan, Metal or OpenGL works — wgpu picks the adapter and
prefers the integrated GPU on hybrid laptops to save battery. The startup line
tells you what you got:

```
cread: AMD Radeon 780M (Vulkan, IntegratedGpu) · surface Bgra8Unorm · 60 fps
cread: llvmpipe (LLVM 21.1.8, 256 bits) (Vulkan, Cpu) · surface Bgra8Unorm · 30 fps
```

The second one is a software rasteriser: no GPU involved, which is how the app
is developed and why that build asks for fewer frames.

## Layout

```
crates/core      buffer, cursor, soft wrap, undo, markdown spans, file IO
crates/render    emulated framebuffer, bitmap font, wgpu CRT pass
crates/sync      WebDAV client, conflict policy, background worker
crates/app       window, input, frame loop, UI composition
assets/          IBM VGA 8x16 glyph data, the boot logo
assets/splash…   pixel art, blitted into the emulated framebuffer
```

`crates/core` has no GPU, window or network dependency, so the tricky parts are
plain unit tests. `cargo test` runs all of them.

## Credits

The boot logo (`assets/splash.png`) is blitted at its native size in emulated
pixels, so it keeps hard edges through the CRT upscale. Monochrome themes tint
it; the black theme keeps its own grey and teal.

The character set is the IBM VGA 8x16 ROM font from VileR's *Ultimate Oldschool
PC Font Pack* (CC BY-SA 4.0, <https://int10h.org/oldschool-pc-fonts/>), reduced to
the raw 256 x 16 byte bitmap. The handful of glyphs CP437 never had (`€`, `„“”`,
`–—`, `…`) are hand-drawn in `crates/render/src/font.rs`.
