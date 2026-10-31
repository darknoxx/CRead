# Changelog

## Unreleased

- **`eject`.** Takes a diskette out for the session: `eject D`, `eject hello`, or
  bare `eject` for the selected row. The drive disappears from the boot screen and
  from `dir` until CRead restarts; the file in `Disks` is not touched.
- **Saving is manual.** Autosave no longer runs on its own: nothing is written
  until you run `save`. `autosave on` restores the old behaviour, the choice is
  remembered, and quitting still saves a modified document.

## 0.9.0

- **The launch is a small boot.** A `Starting CRead OS...` bar runs for a second
  and a half, the speaker beeps, and only then is the boot screen drawn — nothing
  of it appears before the bar is full. The bar does not fill smoothly: it jumps
  and pauses like a drive seeking, with a blinking block beside the percentage.
  Any key skips the wait. The CRT flicker of 0.8.0 is gone; an empty screen that
  flickers gave the game away.

## 0.8.0

- **The screen warms up.** On launch the picture stutters three times over nine
  tenths of a second, like a CRT looking for a signal, then the PC speaker beeps
  once. Any key skips it; `beep off` turns the sound off and `--boot` replays the
  flicker for screenshots. The beep is a WAV handed to `paplay`, `pw-play`,
  `aplay` or `play` — no audio stack in the build, and silence if none is there.

## 0.7.0

- **Softhand.** The first program with pixels: a 16-colour paint program with
  pencil, eraser, line, rect, fill and pick, eight undo steps, and pictures saved
  as PNG in `<vault>/Pictures`. Its front door looks like Inspiro's. It is
  installed with CRead and needs no diskette.
- **A boot screen with a shelf.** The vault is `A: DOCUMENTS`, then a rule, then
  the modules: `B: INSPIRO` and `C: SOFTHAND`, with diskettes after them. The
  arrow keys step over the rule.
- **Program disks.** A `.dsk` whose header names a `program=` puts that program on
  the boot screen; remove the diskette and the row is gone. Softhand is installed
  and needs no diskette — the mechanism is there for modules that ship on one.
- **A pixel layer under the text grid.** Programs draw into a 640x368
  sixteen-colour raster; cells can be transparent so menus sit on top of a
  picture, and everything still goes through one CRT pass.
- **`exit` and `exit!`.** `exit` (was `welcome`) returns to the boot screen from
  anywhere, and quits CRead when you are already there; `exit!` (was `quit`)
  saves and quits from anywhere. `q` still quits, now spelled `exit!`.
- **`help` is scoped.** It lists the commands of the screen you are on — the
  boot screen, the editor, or the running program — in two columns with the
  highlighted row explained in the status line. `help all` still shows
  everything.
- **Paint settings as commands.** `tool <name>`, `size <n>` and `colour <n>` in
  Softhand's prompt; bare, they report the current setting.
- The boot screen shrinks the logo by a whole step when the drive list is long,
  instead of dropping rows off the bottom.
- The help screen lists the commands in two columns (the vocabulary outgrew one
  page) and shows the highlighted command's description in the status line.

## 0.6.1

- **Inspiro is in main.** The editor has a name, a front door and a loading bar:
  `A: INSPIRO` on the boot screen, then *where your ideas have room to grow*,
  then `new`/`open`.
- The Inspiro logo no longer floats over the help screen, the palette, or the
  boot screen you return to with `welcome`: only the screen that composed it
  blits it.
- The loading bar runs a little longer and is full before the wait ends, so it
  no longer stops short of the right edge.
- `open` on Inspiro's front door shows the document list again; the front door
  used to finish composing before the palette was drawn.
- An empty `new` document no longer leaves its file behind. `new` still reserves
  `untitled.md` straight away, but leaving the document blank (another `new`,
  `open`, `welcome`, a disk, or quitting) deletes it again, so the vault does not
  silt up. A document with a single character, in the buffer or on disk, stays.

## 0.6.0

- **CRead OS.** The start screen is a boot screen now: a drive list with
  `A: INSPIRO`, `B: DOCUMENTS` and every diskette in `<vault>/Disks`. Arrows
  select, Enter runs, and the old greeting is still there.
- **Diskettes** are single plain-text files: a `key=value` header, then files
  introduced by `---FILE <name>`. A data disk opens in a file browser and its
  documents read in the viewer, read-only. A disk that will not parse still
  appears, in red, with the reason.
- `run [name]` and `dir` for the command line.
- **Inspiro.** The editor has a name and its own front door: `A: INSPIRO` on the
  boot screen runs a short fake loading bar, then shows the Inspiro logo,
  *where your ideas have room to grow* and the usual `new`/`open` prompt. Opening
  a document from `B: DOCUMENTS` goes through the same loader, and any key skips
  it. `welcome` still returns to the boot screen.

## 0.5.0

- **Scanlines and their drift are on by default.** The moving raster is what
  stops the pattern reading as a printed overlay.
- **No more bold or italic.** A 1-bit face at 8x16 cannot fake weight or slant
  without losing strokes, so headings, strong and emphasis are distinguished by
  colour and the dimmed markers instead. The synthesis is gone from the font.
- The boot logo no longer bleeds through the help screen.
- The start screen points at `help` for the rest of the commands.

## 0.4.0

The CRT pass, rebuilt.

- **Paper is the default theme.** Dark ink on warm paper. The effects adapt:
  glow is damped to a hint of ink bleed instead of a haze, and the boot logo
  inverts to dark ink.
- **New boot logo**, drawn from its own alpha and luminance so the white
  counters show the background through. It is blitted at native size in emulated
  pixels, shrinking only by a whole divisor when the raster is too small —
  fractional scaling would destroy pixel art.
- **Real glow.** A separable gaussian at half resolution replaces the eight
  neighbour taps whose radius was about the size of a glyph, which is why it
  read as a blur around every letter. The blur is 2.2x wider horizontally than
  vertically, because a CRT beam sweeps sideways.
- **Scanlines are opt-in** (`scanlines`). Where the raster affords three device
  pixels per line the beam gets a cosine profile; at two it degenerates, so it
  stays a discrete row. `scanlines -r` adds a slow downward drift, which stops
  the pattern reading as a printed overlay. The drift brings back a 60 fps frame
  clock while it runs.
- `welcome` returns to the start screen, saving first.
- `font` toggles between the two 8x16 faces: classic VGA and the thinner ISO.

## 0.3.0

Writing, not fighting the editor.

- **Selection and clipboard.** Shift plus any movement key marks text, `Ctrl+A`
  selects everything, `Ctrl+C`/`Ctrl+X`/`Ctrl+V` talk to the system clipboard,
  and typing replaces the selection.
- **Mouse.** Click places the cursor, dragging selects, the wheel scrolls.
- **Incremental search.** `Ctrl+F` or the `find` command opens a `/` prompt.
  Hits are marked as you type; `F3` and `Shift+F3` walk them.
- **Preferences.** Theme, typewriter scrolling, the CRT switches and the
  typeface survive a restart (`~/.config/cread/prefs.json`).
- **Palette.** `Ctrl+R` renames and `Ctrl+D` deletes the highlighted file, plus
  a `+ sync now` entry.
- **Boot logo** on the start screen, tinted to match the theme.
- **Second typeface.** `font thin` switches to the ISO 9241 face from the same
  bitmap pack: identical grid, about 31% less ink.
- **View mode.** `view` renders the document without markup: headings bold, code
  on an inverse field, links reduced to their text, rules drawn across the
  column. Read-only, scrolls like a reader, `Esc` returns to the text.
- Italic slants by two pixels instead of three, which was cutting visible gaps
  into 8px glyphs.
- **Flicker removed.** It annoyed more than it charmed. With nothing left to
  animate the frame clock is gone too, so frames are drawn on change and when
  the cursor blinks; idle CPU on a software rasteriser drops from ~12% to ~3%.
- Fixed: holding Up/Down moved the cursor in jumps, because those handlers never
  scheduled a frame. Fixed: the logo was painted over the file palette.

## 0.2.0

The release that makes it usable as a daily writer.

- **Start screen.** Launching shows a centred greeting with a prompt: `new` for
  an empty document, `open` for the vault. The app no longer silently reopens
  the last file.
- **Black theme by default.** Black background, white text, white block cursor.
  `theme +` and `theme -` cycle; names still work.
- **Two modes.** Write mode for the text, action mode (`Esc`) for a shell-like
  command line with `save`, `saveas`, `open`, `new`, `rename`, `delete`, `sync`,
  `settings`, `help`, `about`, `quit` and the effect switches. Tab completion,
  command history, full-screen help.
- **Title bar** shows `name  saved`/`modified` on the left and the document
  folder on the right; status messages are transient and no longer evict the
  mode hint.
- **60 fps** on real GPUs (30 on software rasterisers, `--fps N` overrides).
  Frame pacing now schedules from the frame start, and the cursor blink is timed
  independently of the frame rate.
- **Vault follows XDG user dirs**, so a German desktop gets `~/Dokumente/CRead`.
- `new` creates `untitled.md`, `untitled-2.md`, … on disk immediately.
- `--version` reports the build.

## 0.1.0

First commit that does something useful.

- IBM VGA 8×16 bitmap face, integer-scaled raster, CRT pass with scanlines,
  phosphor bleed and flicker.
- Markdown editor with soft wrap, undo, autosave and atomic writes.
- WebDAV sync with ETag tracking and conflict copies.
- CI: format, clippy and tests on every push; release builds for Linux.
