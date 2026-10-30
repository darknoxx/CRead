# Changelog

## Unreleased

- **Boot logo** on the start screen, tinted to match the theme.
- **Second typeface.** `font thin` switches to the ISO 9241 face from the same
  bitmap pack: identical grid, ~31% less ink. `font classic` switches back, and
  the choice is remembered.

- **Preferences.** Theme, typewriter scrolling and the CRT switches survive a
  restart (`~/.config/cread/prefs.json`); command-line flags still win.
- **Incremental search.** `Ctrl+F` or the `find` command, hits highlighted as
  you type, `F3`/`Shift+F3` to walk them.
- **Mouse.** Click places the cursor, dragging selects, the wheel scrolls.
- **Palette.** `Ctrl+R` renames and `Ctrl+D` deletes the highlighted file, and
  there is a `+ sync now` entry.

- **Selection and clipboard.** Shift plus any movement key marks text, `Ctrl+A`
  selects everything, `Ctrl+C`/`Ctrl+X`/`Ctrl+V` talk to the system clipboard,
  and typing replaces the selection.
- **Flicker removed.** It annoyed more than it charmed. With nothing left to
  animate, the frame clock is gone too: frames are drawn on change and when the
  cursor blinks, which drops idle CPU from ~12% to ~3% on a software rasteriser.

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
