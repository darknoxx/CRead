# Changelog

## 0.24.0

- **A top bar on the boot screen, with date and time**: `01.11.26 - 19:25` in
  the middle, in the same bar the editor already wore. The logo moves down a
  row so nothing sits under it.
- **The editor's bar has a clock too**, and Softhand wears the same bar:
  `bildname.png  modified  19:25  ~/Dokumente/CRead/Pictures`. It needed the
  program to know whether it has unsaved work: Softhand now marks itself
  `modified` while drawing and `saved` after the shell writes the file.
- **Softhand's canvas and palette strip move down by exactly one bar row**, so
  the canvas keeps its height; the pointer mapping subtracts the same offset,
  or a click would land 16 pixels off.
- **`SETTINGS` chooses the clock and date format**: `TIME` (24 hours or 12 with
  AM/PM) and `DATE` (`DD.MM.YY` or `MM/DD/YY`), remembered in the preferences.
- **`SETTINGS` is sorted by what belongs together**, with rules between sync,
  the look, the clock and the machine's own rows. Arrow keys skip the rules.

## 0.23.2

- **The INFO screen says what CReadOS is and who made it.** It now carries the
  mission line ("a minimal fake operating system, that helps you to stay focused
  on what really counts"), the stack (Rust, winit, wgpu), the font credit with
  its licence and link, and what Pocketboi runs on: mizu-core 1.3.0 (MIT), the
  Game Boy emulator in Rust, vendored with five small patches, with its link.
  Signed `- darknoxx 2026`.
- The about text is word-wrapped now, so a narrow grid gets more lines instead
  of sentences chopped at the edge.

## 0.23.1

- **The restart after an update actually runs the new version.** `install`
  renames the running binary to `cread.old` before putting the new one in its
  place; on Linux `/proc/self/exe` follows the *inode*, so `current_exe()` then
  names the old file. The restart used exactly that and started the previous
  version — the update only took effect after starting CRead from a fresh
  shell. The executable path is now captured at startup and the restart uses
  that; a test keeps it from drifting back to `current_exe()`.
- `--update-once` was never affected: it asks for the path before installing.

## 0.23.0

- **A screensaver: an analog clock on black.** After five minutes without a key
  or a mouse movement the screen goes dark and the clock takes over — hour,
  minute and one red second hand, large in the middle of the screen. Any key
  brings back exactly what was behind it; the mouse is ignored on purpose, so a
  nudge does not wake it. It is the shell resting, not a module: no `SOFTWARE`
  entry, no theme colours, and the CRT pass still runs over the picture.
- **`SETTINGS` grew two rows**: `SCREENSAVER` (on/off) and `IDLE` (1, 2, 5, 10,
  15, 30 or 60 minutes). The command exists too: `screensaver` starts it now,
  `screensaver off` / `on`, `screensaver 15` sets the time.
- The clock is drawn into a full-screen raster with a new
  `Framebuffer::draw_canvas`, so it owns every pixel without the cell grid and
  the second hand only redraws once a second.

## 0.22.0

- **CRead updates itself, from GitHub, with a signature check.** `SETTINGS` has
  an `UPDATE` row (and `update` is a command like everything else): it asks for
  the latest release behind a progress window, and when there is a newer one it
  offers `0.22.0 is available. Install and restart?`. Yes downloads the tarball,
  checks SHA-256 **and** a minisign signature, replaces the binary (keeping the
  old one as `cread.old`), saves the document and starts the new binary with the
  arguments the old one had.
- **It refuses rather than guesses**: unsigned releases, a checksum or
  signature that does not match, a missing `cread` in the archive, development
  builds and directories it cannot write to all stop the install.
- **Two public keys are embedded**, the current one and a spare. A binary only
  trusts the keys it was built with, so shipping the spare now means a key
  rotation later does not strand installations.
- Releases are signed in CI (`MINISIGN_KEY` secret) and publish
  `cread-linux-x86_64.tar.gz.minisig`; `minisign.pub` is in the repo so anyone
  can verify a download by hand.
- `cread --update-once` does the same without the UI, for scripts and for
  testing the pipeline.

## 0.21.2

- **The release now carries its paperwork.** The tarball held `cread`,
  `README.md` and `VERSION.txt` only; MIT asks for the licence text to travel
  with the binary, and the fonts want their attribution. `LICENSE`,
  `THIRD-PARTY.md` and `FONTS.md` are in the archive from this release on.

## 0.21.1

Nothing about the program changed; this is the repository getting ready to be
public.

- **MIT, on paper and in the tree.** The workspace declared `MIT OR
  Apache-2.0`, but no `LICENSE` file existed and none of the seven crates
  inherited the field, so the declaration did nothing. There is a `LICENSE`
  now, every manifest carries `license.workspace` and `repository.workspace`,
  and the README has a licence section.
- **`THIRD-PARTY.md`.** What is not CRead's own work and under which terms:
  `mizu-core` (MIT, vendored and patched), the bitmap fonts (CC BY-SA 4.0,
  VileR), the artwork (MIT, with the code), and the dependency list — including
  `symphonia` as the only MPL-2.0 component.
- **No personal traces.** A `/home/noxx` path in a test is neutral, and
  `.gitignore` no longer names a local download. The unused Gambatte checkout is
  gone; no GPL code was ever tracked.
- The release still builds the same binary; the tag exists so the first public
  release matches the tree.

## 0.21.0

- **`saveas` removes the untitled placeholder.** With autosave on, a new document
  lived as `untitled-N.md` and stayed there after it was named, so every draft
  left a second copy in the vault — and in WebDAV. Naming it now writes the
  chosen file and deletes the placeholder in the same step.
- **The server copy goes too.** Local deletion alone would not have helped: the
  next sync round would have pulled `untitled-N.md` straight back. The editor
  tells the sync engine which file is its unnamed draft (`set_draft`), the index
  remembers it, and when the draft is named the remote copy is deleted in the
  same round. Documents the writer made are still never deleted; a draft that
  survives a crash is promoted to an ordinary document at startup instead of
  being thrown away.
- `Report` gained a `deleted` count, so the status line says "1 draft removed"
  rather than hiding it.

## 0.20.2

- **The arrow keys work in Riff's insert list again.** When the vault and music
  browsers were merged, the up/down handler kept pointing at the shell's browser
  slot instead of the list on screen: the highlight moved an invisible browser
  and the visible one stood still. Both now step their own list, and a test
  keeps the two apart.

## 0.20.1

- **A window stands on its own page.** The vault browser, the music list's
  dialog, the WebDAV form and the quit question used to float over whatever was
  on screen, so boot rows or a line of the document peeked out around the
  frame. A dialog now clears the page behind it: nothing under it is drawn at
  all, and closing it brings the screen back.

## 0.20.0

- **Every program opens in its own room.** `open` in Inspiro used to land in
  the vault root (`CRead/`), with `Documents/`, `Pictures/` and `Roms/` as rows;
  it now starts in `Documents/`, where documents live. Softhand's picker starts
  in `Pictures/` likewise, and it is a real folder browser now (subfolders and
  `..` included) instead of one flat list. `THE VAULT` on the boot screen still
  shows the whole machine.
- **A room lists only what its program can use.** `Documents/` shows documents,
  `Pictures/` shows pictures; a stray `notes.md` in `Pictures/` no longer shows
  up as something to paint over.
- **`in_softhand` meant "any program".** It checked `program.is_some()`, so
  `save`, `open` and `new` took Softhand's path while Pocketboi was in front.
  The shell now remembers which module a program belongs to.

## 0.19.0

- **One browser, two roots.** The vault browser and Riff's music folder were two
  folder walkers with their own selection, scrolling and "one folder up". They
  are one `Browser` now: the kind decides which files count and what Enter does
  (open in a program, or load into the deck), the walking and filtering are
  shared. Riff's list gains the same `..` row and folder markers as the vault.
- **The music browser stays in the music folder.** `Backspace` at the root used
  to walk up into the home directory; now the root is the end of the road, just
  like the vault.

## 0.18.0

Internal shape, no new behaviour: this is the first half of the cleanup the
review asked for.

- **`compose` has layers instead of one block.** A frame is now a whole-frame
  screen (menu, loading bar, help), then the main screen (program, reader,
  front door or document), then the prompt text, then an overlay — each its own
  function in `crates/app/src/app/compose.rs`. The old 590-line function is
  gone; the document screen no longer has to be read past the Riff counter.
- **Commands are grouped by what they are about.** `run_command` is a
  switchboard; the work lives in `app/commands/{document,screen,system,program}.rs`.
  A command is now one function next to its siblings instead of an arm in a
  350-line match.
- `app.rs`: 4.879 → 4.017 lines, the rest in `app/compose.rs` (605) and
  `app/commands/*` (367).

## 0.17.0

Nothing behaves differently in this one; it is the cleanup the review asked for.

- `app.rs` came apart: the boot rows, the vault browser, the WebDAV form and the
  sync worker each live in their own module under `crates/app/src/app/`. The
  file still owns the mode machine and the commands, which is the part that
  genuinely is one thing.
- One box-drawing routine (`ui::frame`) replaces six hand-written copies; the
  screens cannot drift apart at the corners any more.
- The boot screen has one list, not two: `boot_rows` is built from `boot_items`,
  so a new row is one variant. The `broken` flag (a leftover from the diskette
  era, always false) is gone.
- The old names are gone with the concept: the file dialog is the vault browser
  (`Browser`, `Mode::Browser`) and the WebDAV screen is `Mode::Webdav`, not
  "settings" next to a SETTINGS menu.

## 0.16.0

- **`rename` cannot overwrite another document any more.** It used to call the
  filesystem and let it replace whatever had that name; now a taken name is
  refused, and a rename stays in the folder the document lives in — the same
  rule the browser's `Ctrl+R` uses.
- **A name that folds away is refused.** `saveas .` or `saveas ..` used to
  produce `Documents.md` *next to* the folder; pictures had the same hole. Both
  now say that a name is needed.
- **Conflict copies stay beside their file.** A dot in a folder name
  (`Documents/v1.2/notes`) made the old code split the whole path and invent a
  folder called `v1.conflict-<unix>.2`.
- **The exit question sees more.** `rename`, `delete` and a cartridge's battery
  save count as unsynced now; the battery file is written before the last round,
  not after it. `open` and `new` also show up in the boot screen's `help`.
- **WebDAV gives up.** The client had no timeout, so a stuck server could hold
  the worker's thread — and that thread is joined when CRead closes. Ten seconds
  to connect, thirty for a round.
- **Quiet sync rounds stay quiet.** Folders are only created when the server is
  missing them, instead of `MKCOL` for every path segment every round.
- The sound device policy moved into its own small crate (`crates/sound`): the
  music deck no longer borrows it from the Pocketboi program. Autosave errors
  now reach the status line (they used to be cleared in the same frame), the
  vault tidy-up reports everything in one message, and stale docs/pictures
  (`disk.png`) are gone.

## 0.15.0

- **The vault has folders, and `THE VAULT` is a browser.** The third row of the
  boot screen replaces DOCUMENTS: documents in `Documents/`, pictures in
  `Pictures/`, cartridges in `Roms/`, and every file opens with the program that
  owns it (`.md` Inspiro, `.png` Softhand, `.gb` Pocketboi). `Enter` walks into a
  folder, `Esc` and `←` go back up, typing filters the folder you are in.
- **`saveas` has one home.** Documents land in `Documents/`, pictures in
  `Pictures/`; subfolders are allowed, `..` and absolute paths are folded away.
  `open eins` still finds `Documents/kapitel/eins.md`.
- **Old vaults are tidied on first use.** Loose `.md`/`.txt` in the vault root
  move into `Documents/`, nothing is overwritten, and the status line reports
  what happened.
- **Quitting asks before unsynced work is left behind.** `exit` on the boot
  screen, `exit!` and the window close button show a small "Sync to server?"
  box when this session wrote something the server has not seen. `Yes` runs one
  last round; a failure keeps CRead open with the error instead of quitting.

## 0.14.1

- **WebDAV syncs the whole vault, not only the Markdown.** Pictures and the
  cartridges in `Roms` were left out, so the ROM folder arrived on the server
  empty. The tree is mirrored now — subfolders included, empty ones too, so
  files dropped in from another machine come back on the next round. Downloads
  are written byte for byte instead of as text.

## 0.14.0

- **SETTINGS is a row on the boot screen.** WebDAV syncs the whole vault —
  documents, pictures and ROMs — so it belongs to the machine, not to the
  editor. The new screen also holds the theme, the bitmap face, window or
  fullscreen, and the version. One list, one place, reachable with `settings`
  from any prompt; before, `settings` opened the WebDAV form directly.
- SOFTWARE and SETTINGS share one menu screen: a name, what it is, and a value
  on the right.

## 0.13.0

- **Pocketboi**, a Game Boy in the machine: cartridges live in `Roms` inside the
  vault, the screen is drawn into the raster with a bezel, and the controls are
  `Space`/`C`, `Enter`/right `Ctrl` and left `Ctrl`. `P` cycles the palette.
- The emulator runs on its own thread at 59.7275 Hz, so sound no longer depends
  on how fast the screen is drawn, and `mizu-core` is vendored with a list of
  patches in `vendor/README.md`.
- **Gamepads** work: `gilrs`, with east as B, south as A, and the left stick
  doubling the D-pad.
- **Sound follows the system's output device** (PipeWire, then PulseAudio, then
  `default`), never a hardware device.

## 0.12.1

- **Sound follows the system's output device.** Both the game and the music
  player opened the default ALSA device, which on some setups is the hardware
  one — a game could land on the speakers while the system was set to
  headphones, and only switching output back and forth in the desktop settings
  moved it. They now ask for the sound server's own PCM (`pipewire`, then
  `pulse`, then `default`) and never fall back to a hardware device, so the
  stream belongs to the server that owns the routing.

## 0.12.0

- **Pocketboi, second pass.** Four greys instead of the DMG green (`P` cycles
  green and amber), a bezel around the glass, and `exit` steps back to the shelf
  before it leaves the module.
- **The SOFTWARE cursor survives an install.** Installing inserts a row above
  the shelf, and the boot screen's cursor is an index — it landed on the rule and
  the highlight vanished until the next arrow key. It now follows the row it was
  on.
- **Pocketboi.** A Game Boy and Game Boy Color on the raster, running Mizu
  (`mizu-core`, MIT, vendored and patched — see `vendor/README.md`). Cartridges
  come from `<vault>/Roms`; the module lists folders and `.gb`/`.gbc` files, maps
  the 160x144 screen to four shades and doubles it, prints the key legend under
  the screen — on the page, not in a black box — and plays the APU on its own
  audio stream (at the 88.2 kHz Mizu actually emits, not the 44.1 kHz its
  source claims — that mistake played everything an octave low and a beat
  behind). The raster now starts as the theme's page colour and cells can
  carry ink without a background, so a program can sit on the page like the
  rest of the interface. Text painted with a cell background still uses the
  *theme's* palette, while the raster uses the fixed one, so a background cell
  showed as a slightly grey box on paper — Pocketboi now writes its labels as
  ink and lets the page show through. The pixel layer itself starts *clear*
  (`Raster::CLEAR`): a program paints only what it means to paint, and
  everything else is the shell's own page colour. The buttons are `Space`
  (A), `C` (B), `Enter`/right `Ctrl` (Start) and left `Ctrl` (Select) — chosen to
  land in the same place on QWERTY, QWERTZ and AZERTY. It is the first module that animates:
  the shell runs a frame clock while a cartridge is loaded, and a panic in the
  emulator costs the game, not the application.

## 0.11.0

- **SOFTWARE replaces the diskettes.** The boot screen lists the modules first,
  with drive letters handed out in the order they were installed; below a rule
  sit `SOFTWARE` and `DOCUMENTS`, left-aligned with the modules, because they are
  not drives. SOFTWARE is a shelf with one row per compiled module: Enter
  installs or removes it behind a short loading bar, the choice is remembered and
  the boot screen follows at once. Inspiro is the system and cannot be removed;
  Softhand and Riff ship switched off, so a fresh start is a system with a shelf.
- **The diskette machinery is gone.** No `Disks` folder, no `.DSK` files, no
  `run`, `dir` or `eject`, no read-only reader, no copy step in the release. The
  modules are compiled in and nothing is loaded at runtime.
- **Autosave is opt-in.** Nothing is written until you run `save`; `autosave on`
  restores the old behaviour, and quitting still saves a modified document.
- **Now playing everywhere.** While a track runs, the status line of the other
  screens shows `♪ title · mm:ss`; on a narrow bar the hint makes room for it.
- Two fixes: playing music no longer stops the scanline drift (the music clock
  was checked before the raster clock), and the prompt sigil is readable in the
  paper theme (the accent falls back to the bar's own foreground when the two do
  not contrast).

## 0.10.1

- **The sample diskettes are in the release tarball.** `disks/RIFF.DSK` and
  `disks/HELLO.DSK` ship next to the binary, so a fresh download can actually
  mount Riff instead of wondering why the row is missing.

## 0.10.0

- **Riff.** A music player with a transport, a playlist and a volume, playing the
  XDG music folder (MP3, FLAC, OGG, WAV). It ships on a diskette
  (`examples/disks/RIFF.DSK`), so it only appears on the boot screen while that
  disk is in. `riff play|stop|pause|next|prev|random|volume` work from any prompt,
  even before Riff was opened, and the music keeps playing while another program
  is in front. This is the first real audio dependency: `rodio`/`symphonia`, and
  `libasound2-dev` at build time. The player has the same action line as every
  other module, `i` inserts the highlighted folder without opening it first, and
  returning to Riff with a disc in goes straight to the player.
- **Now playing, everywhere.** While a track runs, the status line of the other
  screens shows `♪ title · mm:ss`; on a narrow bar the hint makes room for it.
- **Two shelves on the boot screen.** A second rule separates the built-in
  modules from the diskettes, so what is mounted is visible at a glance.
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
