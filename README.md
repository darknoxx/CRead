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

On launch a bar runs for two seconds (`Starting CRead OS...`), in the uneven
jumps of a drive seeking rather than a smooth fill, the PC speaker beeps once,
and only then does the boot screen appear — nothing of it is drawn before the bar
is full. Any key skips the wait. `beep off` silences the beep;
`--boot` replays the bar (handy with `--screenshot`).

Then you get the boot screen: the logo, a greeting and a short drive list.

```
                             Welcome to CRead

                       A: DOCUMENTS   open a file from the vault
                       ─────────────────────────────────────────
                       B: INSPIRO     where your ideas have room to grow
                       C: SOFTHAND    16 colors, endless possibilities.
                       D: HELLO       a sample diskette
```

`A:` is the vault itself. Below the rule are the modules: `B:` is **Inspiro**,
the writing terminal, `C:` is **Softhand**, the paint program. Both are installed
with CRead, and both open with a fake loading bar before their own front door
appears.

```
                                  [logo]

                     where your ideas have room to grow

                          type new or open to start
>▌
```

There `new` opens an empty document and `open` shows the vault, exactly as
before. Opening a file from `A: DOCUMENTS` goes through the same loader, and any
key skips it. Pass a file name on the command line and it opens straight away,
bar and all.

Then you are in **write mode**: the document is live, the block cursor sits in
the text, and nothing else competes for your attention. `Esc` switches to **action
mode**, where the bottom line turns into a command prompt, like a DOS shell.
`Enter` runs the command, `Esc` on an empty prompt goes back to writing.

```
> save▌                                          save, saveas, sync, open …
```

`Tab` completes the command, the arrow keys walk history, and a dimmed hint on
the right tells you what the command does before you commit to it.

`help` is not one big list: it shows the commands that belong to the screen you
are on — drives and disks on the boot screen, documents and search in the editor,
tools and colours in Softhand. `help all` shows the whole vocabulary, and on the
help screen the arrow keys walk it while the status line explains each row.

| Command | Aliases | What it does |
| --- | --- | --- |
| `save` | `w`, `safe` | write the document to disk |
| `saveas <name>` | `sa` | save under a new name in the vault |
| `open [name]` | `o`, `e` | file list, or open a name directly |
| `new` | `n` | start an empty document |
| `rename <name>` | `mv`, `rn` | rename the current document |
| `delete` | `rm`, `del` | delete it, after a `y/n` |
| `find [text]` | `search` | search the document (`Ctrl+F`) |
| `view` | `read` | reading mode: markup stripped, nothing to edit |
| `exit` | `welcome`, `home` | back to the boot screen |
| `run [name]` | `exec` | run a diskette (bare: whatever the boot screen has selected) |
| `dir` | `disks` | list the diskettes in the drive folder |
| `beep [on\|off]` | `bell` | the startup beep; bare `beep` toggles |
| `font [classic\|thin]` | `weight` | typeface; bare `font` cycles |
| `sync` | | push the vault to WebDAV now |
| `theme [+\|-\|name]` | | next/previous, or black, dos-blue, amber, green, paper |
| `scanlines [on\|off] [-r]` | `scan`, `sl` | switch scanlines on; `-r` adds a slow downward drift |
| `bloom [on\|off]` | | phosphor glow |
| `typewriter [on\|off]` | `tw` | keep the current line in place |
| `fullscreen` / `windowed` | `fs` | |
| `undo` / `redo` | `u` | also on `Ctrl+Z` / `Ctrl+Shift+Z` |
| `settings` | `set`, `cfg` | WebDAV setup |
| `help` | `?`, `h` | the list above, on screen |
| `about` | | version and credits |
| `exit!` | `quit`, `q`, `x` | save and exit from anywhere |

Autosave runs 1.5 seconds after you stop typing, so a forgotten `save` costs at
most a sentence.

## Selection

`Shift` plus any movement key marks text, `Ctrl+A` takes everything, and the
usual `Ctrl+C` / `Ctrl+X` / `Ctrl+V` work with the system clipboard. Marked text
is drawn inverted, and typing or `Backspace` replaces it. The mouse does the
obvious thing: click places the cursor, dragging marks, the wheel scrolls.

## Diskettes

CRead OS is the start screen: the logo, a greeting, the drives and a prompt. The
vault is `A:`, the modules `B: INSPIRO` and `C: SOFTHAND` follow, and everything
after that comes from the `Disks` folder inside your vault:

```
~/Dokumente/CRead/Disks/MAIL.DSK
```

A diskette is a single plain-text file, so it can be read with `type`, mailed
around, or kept in git. The format:

```
CRead Disk 1
name=MAIL
api=1
about=Postfach und Nachrichten
author=you

---FILE README.TXT
Willkommen in der Post.
---FILE INBOX/1.TXT
Hallo.
```

Line one is the magic. Then `key=value` lines (`name` is required, `api` is the
container version), then files introduced by `---FILE <name>`. A diskette that
will not parse still shows up on the boot screen, in red, with the reason — you
can see that something is wrong without digging.

Data diskettes are read-only: opening a file shows it in the reader, and `Esc`
takes you back to the disk. Code diskettes (a payload that actually runs) are
the next step; the container is designed to carry them.

`examples/disks/HELLO.DSK` is a working sample.

### Program disks

A diskette can name a program in its header instead of carrying documents:

```
CRead Disk 1
name=MAILBOX
api=1
program=mailbox
```

The code is always in the binary; the diskette is what puts the drive on the
shelf. A program that ships as a removable module has no boot row until its
diskette is in `Disks` — take it out and the row is gone again. A diskette that
names a program this build does not have shows up in red: *needs program
mailbox, which is not installed*.

Softhand is installed, so it never needs a diskette; a `program=softhand` disk
would just be a data disk. `exit` returns to the boot screen (and quits CRead
when you are already there), `exit!` quits from anywhere.

## View mode

`view` renders the document the way a reader sees it: `#`, `**` and backticks
gone, headings coloured, code on an inverse field, links reduced to their text, and
`---` as a rule across the column. It is read-only — arrows, `PageUp`/`PageDown`
and the wheel scroll, `Esc` goes back to the text — and it opens at the line you
were last editing.

## Search

`Ctrl+F` (or `find`) opens a `/` prompt. Matches are marked as you type and the
cursor jumps to the first hit at or after where you were. `F3` and `Shift+F3`
walk the hits, also after the prompt is closed; `Enter` keeps the cursor where
it landed, `Esc` puts it back.

## Softhand

Softhand is the paint program, and the first module with pixels under the text
layer. It is installed with CRead — no diskette needed. Its front door looks like
Inspiro's (logo, tagline, `new`/`open`), and the
canvas is the full 640x368 raster with a sixteen-colour VGA palette along the
bottom. Pictures are PNGs in `<vault>/Pictures`; `save` picks `picture.png`,
`picture-2.png` and so on, `saveas` names one.

Drawing is a gesture, so it has keys: `b` pencil, `e` eraser, `l` line, `r` rect,
`f` fill, `i` pick colour, `,`/`.` previous/next colour, `-`/`+` brush size, `u`
and `Ctrl+Z` undo, `y` and `Ctrl+Y` redo. Click the palette strip to pick a
colour, drag to draw, wheel for the brush.

`Esc` turns the system line into a prompt; `Esc` again goes back to the canvas.
There: `save`, `saveas`, `open`, `undo`, `exit`, and the three that are easier to
type than to remember — `tool fill`, `size 6`, `colour 12`. Bare `tool`, `size`
and `colour` report what is set, and `help` lists exactly this vocabulary.

Undo keeps the last eight full-canvas snapshots — a paint program without undo
is a trap, and 240 KB per step is nothing.

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
bundle needed. GPU-wise, anything Vulkan, Metal or OpenGL works — wgpu picks the
adapter and prefers the integrated GPU on hybrid laptops to save battery. The
startup line tells you what you got:

```
cread: vault /home/you/Dokumente/CRead
cread: AMD Radeon 780M (Vulkan, IntegratedGpu) · surface Bgra8Unorm
```

`llvmpipe (Vulkan, Cpu)` instead means there is no GPU driver and Mesa is
rasterising on the CPU: it still works, it is just how the app is developed.

## Layout

```
crates/core      buffer, cursor, soft wrap, undo, markdown spans, file IO
crates/render    emulated framebuffer, bitmap font, wgpu CRT pass
crates/sync      WebDAV client, conflict policy, background worker
crates/app       window, input, frame loop, UI composition
crates/programs/ modules: one crate per program, compiled in
assets/          bitmap faces, boot logo
assets/splash…   pixel art, blitted into the emulated framebuffer
assets/inspiro…  the same, for Inspiro's front door
assets/softhand… and for Softhand's
```

`crates/core` has no GPU, window or network dependency, so the tricky parts are
plain unit tests. `cargo test` runs all of them.

## Credits

Two bitmap faces ship with it: the classic IBM VGA 8x16 and the thinner ISO 9241
variant from the same pack. `font thin` swaps them at runtime and the choice is
remembered.

The boot logo (`assets/splash.png`), the Inspiro logo (`assets/inspiro.png`) and
the Softhand logo (`assets/softhand.png`) are blitted at their native size in
emulated pixels, so they keep hard edges
through the CRT upscale. Themes tint them with the text colour: ink on paper,
light on the dark schemes.

The character set is the IBM VGA 8x16 ROM font from VileR's *Ultimate Oldschool
PC Font Pack* (CC BY-SA 4.0, <https://int10h.org/oldschool-pc-fonts/>), reduced to
the raw 256 x 16 byte bitmap. The handful of glyphs CP437 never had (`€`, `„“”`,
`–—`, `…`) are hand-drawn in `crates/render/src/font.rs`.
