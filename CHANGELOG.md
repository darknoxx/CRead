# Changelog

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
