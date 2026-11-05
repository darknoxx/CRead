# CRead

A writing terminal that pretends it is 1987.

Fullscreen, 80x25, DOS blue, a block cursor and nothing else. Text is drawn into
an emulated raster and blown up by an integer factor, so the scanlines are
always exactly one emulated pixel tall — on a 1080p laptop, a 4K panel or an
ultrawide.

```
┌──────────────────────────────────────────────────────────────┐
│ welcome.md                                saved · sync idle  │
├──────────────────────────────────────────────────────────────┤
│ # Welcome to CRead                                           │
│                                                              │
│ A writing terminal that pretends it is 1987.                 │
│                                                              │
│ ## Getting started                                           │
│                                                              │
│ - `Esc`, then `save`. Everything in ~/Documents/CRead is     │
│   Markdown.                                                  │
└──────────────────────────────────────────────────────────────┘
```

## Build and run

Requires Rust (2024 edition) and, on Linux, the usual Wayland/X11 development
headers, ALSA for the music player (`libasound2-dev`) and udev for game pads
(`libudev-dev`), all on Debian/Ubuntu.

```bash
cargo build --release
./target/release/cread              # fullscreen, opens the last document
./target/release/cread notes.md     # or a specific file
```

## Download a build

Every push to `main` builds ready-to-run Linux binaries in CI, for x86_64 and
for the Raspberry Pi's aarch64:

1. Open the **Actions** tab and pick the newest *Release build* run.
2. Download the `cread-linux-x86_64` or `cread-linux-aarch64` artifact.
3. `tar xzf cread-linux-x86_64.tar.gz && ./cread` (use the `aarch64` name on a Pi)

Modules are compiled in; `software` in the boot screen's prompt switches them on.

Tagged releases also publish both tarballs on the
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

                       A: INSPIRO     where your ideas have room to grow.
                       B: SOFTHAND    16 colors, endless possibilities.
                       C: RIFF        a Soundtrack for your life.
                       D: POCKETBOI   get gaming, my friend!
                       E: US POKER    ten dollars and a dream.
                       F: SCRIPTURE   a quiet place for the Word.
                       ─────────────────────────────────────────
                       SOFTWARE       install and remove modules
                       THE VAULT      documents, pictures, cartridges, calendar
                       CALENDAR       appointments and reminders
                       SETTINGS       webdav, theme, font, screen
```

The letters are the modules, handed out in the order you installed them: install
Riff before Softhand and Riff becomes `B`. Below the rule sit the shelf, the
vault and the settings, which are not drives and so have none. A fresh install has only **Inspiro**, the writing terminal;
**Softhand** (paint), **Riff** (music), **Pocketboi** (games), **US Poker**
and **SCRIPTURE** (the English Bible) sit in the shelf until you switch them on
there. Every module
opens with a fake loading bar before its own front door appears.

```
                                  [logo]

                     where your ideas have room to grow.

                          type new or open to start
>▌
```

There `new` opens an empty document and `open` shows `Documents/` — the
editor's own room, not the whole vault. The boot screen's `THE VAULT` row is
there when you want to see everything. Opening a file from either goes through
the same loader, and any key skips it. Pass a file name on the command line and
it opens straight away, bar and all.

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
are on — drives and the shelf on the boot screen, documents and search in the
editor, tools and colours in Softhand. `help all` shows the whole vocabulary, and on the
help screen the arrow keys walk it while the status line explains each row.

| Command | Aliases | What it does |
| --- | --- | --- |
| `save` | `w`, `safe` | write the document to disk |
| `autosave [on\|off]` | `aw` | save while you type; off by default |
| `saveas <name>` | `sa` | save under a new name in `Documents/` |
| `open [name]` | `o`, `e` | this program's folder, or open a name directly |
| `new` | `n` | start an empty document |
| `rename <name>` | `mv`, `rn` | rename the current document |
| `delete` | `rm`, `del` | delete it, after a `y/n` |
| `find [text]` | `search` | search the document (`Ctrl+F`) |
| `note` | `memo` | open a quick note; `Esc` saves it in `Notes/` |
| `view` | `read` | reading mode: markup stripped, nothing to edit |
| `exit` | `welcome`, `home` | back to the boot screen |
| `software` | `install`, `modules` | install or remove modules |
| `riff <command>` | `music` | the music player: play, stop, next, volume, … |
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
| `manual` | `handbook` | the user manual, read-only |
| `calendar` | `cal` | appointments, reminders and the month |
| `help` | `?`, `h` | the list above, on screen |
| `about` | | version and credits |
| `exit!` | `quit`, `q`, `x` | save and exit from anywhere |
| `shutdown` | `poweroff` | save and power off the machine |
| `reboot` | `restart` | save and restart the machine |
| `backtolinux` | `linux` | leave the kiosk for a Linux console |

Saving is yours to do: nothing is written until you run `save`. If you would
rather not think about it, `autosave on` writes 1.5 seconds after you stop
typing, and the setting is remembered. While the document is clean, the bar
says `autosave on` instead of `saved`. Quitting (`exit!`) and closing the window
still save a modified document, so you cannot lose the last paragraph that way.
`new` and `open` refuse to replace a modified buffer; save it first.

A new document starts as `untitled.md` (then `untitled-2.md`, …) so autosave has
somewhere to write. That file is a draft: naming it with `saveas` writes the
text under the name you chose and removes the placeholder — locally and, on the
next round, on the server too. A draft that survives a crash becomes an ordinary
document instead, so text is never thrown away.

## Selection

`Shift` plus any movement key marks text, `Ctrl+A` takes everything, and the
usual `Ctrl+C` / `Ctrl+X` / `Ctrl+V` work with the system clipboard. Marked text
is drawn inverted, and typing or `Backspace` replaces it. The mouse does the
obvious thing: click places the cursor, dragging marks, the wheel scrolls.

## SETTINGS

The last row of the boot screen is the machine's own control panel: the things
that belong to the device and not to a document.

```
SETTINGS
  ┌────────────────────────────────────────────────────────────┐
  │ ► WEBDAV       sync the whole vault                    off │
  │ ────────────────────────────────────────────────────────── │
  │   THEME        colour scheme                         paper │
  │   FONT         bitmap face                         classic │
  │   SCREEN       window or whole screen               window │
  │   SCREENSAVER  clock when idle                          on │
  │   IDLE         until it starts                       5 min │
  │ ────────────────────────────────────────────────────────── │
  │   TIME         clock format                       24 hours │
  │   DATE         date format                        DD.MM.YY │
  │ ────────────────────────────────────────────────────────── │
  │   INFO         CRead OS and device info             0.33.0 │
  │   UPDATE       check github for a new version       0.33.0 │
  │   SOFTWARE     install and remove modules                  │
  │   STORAGE      12.4/29.7 GB                 ██████░░░░ 62% │
  │   NETWORK      local address                  192.168.1.42 │
  │   USER MANUAL  read the manual                             │
  └────────────────────────────────────────────────────────────┘
```

`USER MANUAL` opens the manual that ships inside the binary: the same text
as `crates/app/src/manual.md`, read-only, with a scrollbar on the right. Its
Contents and appendices are a menu: Up/Down selects a point, Enter jumps to it,
and Page Up/Down scrolls. It is not an Inspiro document; `Esc` takes you back
to SETTINGS.

WebDAV syncs the *vault* — documents, pictures and ROMs — so it is set up here,
next to the other switches that belong to the machine. `Enter` acts on the
selected row: WebDAV opens the form, the rest cycle or open in place. `settings`
reachable from any prompt opens the same screen.

## SOFTWARE

CRead OS is the start screen: the logo, a greeting, the drives and a prompt. The
modules carry the letters (`A:` is Inspiro, the rest follow in install order),
the rule separates them from the three rows that are not drives, and
`SOFTWARE` opens the shelf:

```
SOFTWARE                                  install and remove modules
  ┌──────────────────────────────────────────────────────────┐
  │ ► INSPIRO   where your ideas have room to grow.     system│
  │   SOFTHAND  16 colors, endless possibilities.     install │
  │   RIFF      a Soundtrack for your life.           install │
  │   POCKETBOI get gaming, my friend!                install │
  │   US POKER   ten dollars and a dream.             install │
  │   SCRIPTURE  a quiet place for the Word.          install │
  └──────────────────────────────────────────────────────────┘
```

`Enter` installs or removes, with a short bar in between — the moment is part of
the show. The choice is remembered, and the boot screen follows immediately.
Inspiro is the system and cannot be removed; it is greyed out and stays. Nothing
is downloaded: every module is compiled into the binary, and the shelf only
decides what is visible.

`software` opens the same screen from any prompt, so you can install Riff while
writing and find its row on the boot screen afterwards. Removing a module leaves
your documents and pictures where they are; removing Riff stops the music.

## The vault

Everything you make lives in one folder — `~/Dokumente/CRead` on a German
desktop, `~/Documents/CRead` in English — and that folder is the machine's whole
disk. `THE VAULT`, a row of the boot screen, opens it as a browser:

```
THE VAULT                                documents, pictures, cartridges, calendar
  ┌──────────────────────────────────────────────────────────┐
  │ ► + new document                                          │
  │   + sync now                                              │
  │   Documents/                                              │
  │   Pictures/                                               │
  │   Roms/                                                   │
  │   Calendar/                                               │
  │   Notes/                                                  │
  └──────────────────────────────────────────────────────────┘
```

`Enter` walks into a folder, `Esc` and `←` go back up, typing filters the
current folder. Every file opens with the program that owns it: `.md`, `.txt`
and `.markdown` in Inspiro, `.png` in Softhand, `.gb`/`.gbc` in Pocketboi.
Folders first, then files; a file nothing here can open is simply not listed,
because this is a shelf and not a file manager.

Five folders are the shelf:

| Folder | Holds | Written by |
| --- | --- | --- |
| `Documents/` | Markdown and text | Inspiro, `new`, `saveas` |
| `Pictures/` | 16-colour PNGs | Softhand |
| `Roms/` | `.gb` and `.gbc` cartridges | you, or a sync from another machine |
| `Calendar/` | one Markdown file per appointment | the calendar, or an editor |
| `Notes/` | quick notes as Markdown | the `note` action |

Every program opens in its own room: `open` in Inspiro lists `Documents/`,
`open` in Softhand lists `Pictures/`, and `THE VAULT` on the boot screen is the
whole shelf. A room only lists what its program can do something with — a text
file in `Pictures/` is not a painting and is not shown.

### Quick notes

`note` (or `memo`) at any prompt opens a small floating sheet over the current
screen. The screen behind it stays visible, so a phone number can be caught
without leaving what you were doing. Type immediately; `Esc` writes the text to
`Notes/` and closes the sheet. An empty sheet leaves no file. Notes are ordinary
Markdown, so `THE VAULT` and Inspiro can open them later.

`saveas` has a fixed home: documents always land in `Documents/`, pictures
always in `Pictures/`. It refuses a name that already exists; an existing
document or picture is never silently replaced. Subfolders are fine
(`saveas kapitel/eins` writes
`Documents/kapitel/eins.md`); `..` and absolute paths are not, they are folded
away. `open` still finds a document anywhere in the vault, and a bare name is
enough: `open eins` finds `Documents/kapitel/eins.md`.

A vault from before the layout change still opens: on the first use, loose
documents sitting in the vault root move into `Documents/`. Nothing is
overwritten — a name that is already taken stays where it is and the status
line says so.

## Calendar

`CALENDAR` is a system screen, not a module: open it from the boot screen or
type `calendar` at any prompt. The month grid starts on Monday; arrows move by
day or week, `PgUp`/`PgDn` by month, `Home` returns to today. `Tab` switches
between the grid and the selected day's list; the day heading shows `[GRID]` or
`[LIST]`, and the active panel is outlined with a thin frame. In the list the
up/down arrows select an appointment. `Enter`
always makes a new appointment; in list focus `E` edits the selected one and
`D` twice deletes it. `Esc` goes back to where you came from.

Every appointment is one Markdown file in `Calendar/`:

```markdown
---
title: Doctor
date: 2026-11-05
time: 10:30
remind: 15
repeat: none
deleted: false
---
Bring the card.
```

The form writes it; Inspiro can open it too. `time` empty means an all-day
entry (no alarm); `remind` is minutes before the time. `repeat: weekly` repeats
the appointment on the same weekday from `date` onwards; the reminder fires
each week. The list shows `*` on days with appointments. In a day's list,
all-day appointments always come first, followed by timed ones in clock order.

The event form opens with the first field selected. `Enter` selects it and
shows a cursor; `Enter` again confirms and moves to the next field. `Tab` still
moves between fields, the last row is `SAVE`, and `Enter` on it writes the file.
Space toggles `Repeat` between `none` and `weekly`; in every other field it is an
ordinary space. Leave `Remind` empty for no alarm (an all-day event has none
either). Titles and notes may contain spaces. Only `SAVE` writes the file; while
there are unsaved changes the top bar says `UNSAVED`, and `Esc` asks before
discarding them.

A due reminder double-beeps and puts a small window in front: `Enter` opens the
calendar on that appointment, `Esc` closes it. If the screensaver was running,
the reminder wakes it and `Esc` returns to the clock. Reminders only fire while
CRead is running; a reminder missed while it was closed is dropped silently.
`beep off` leaves the window without the sound.

Deleting writes a visible tombstone (`deleted: true`) instead of removing the
file. That is what keeps the local-first WebDAV sync from bringing the
appointment back on the next round; the calendar hides tombstones. Conflict
copies (`*.conflict-…md`) are ignored and remain in the vault for manual
resolution. A sync that downloads calendar files rescans the folder and
reschedules the next reminder.

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
layer. It is installed through `software` like Riff. Its front door looks like
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

## Riff

Riff is the music player. It is installed from the `SOFTWARE` shelf and appears
on the boot screen under its own letter; remove it there and the row is gone
again, the music stops, and nothing else changes.

It plays the XDG music folder (`~/Musik` or `~/Music`), not a folder of its own:
`i` opens the browser, Enter walks into a folder, Enter on a track plays it, and
`i` inside a folder inserts the whole album. `Esc` and `←` go back up — the
browser never leaves the music folder, so a stray Backspace cannot land you in
your home directory. Nothing is loaded automatically, so the list is what you
chose. MP3, FLAC, OGG and WAV.

```
R I F F                                    a Soundtrack for your life.
 ┌──────────────────────────────────────────────────────────────┐
 │ ♫ 01 Aufwachen                                               │
 │   Morgenlicht  ·  1 of 3                                     │
 │   00:42 ████████████████░░░░░░░░░░░░░░░░░░░░       03:51     │
 └──────────────────────────────────────────────────────────────┘
        ◄◄      ►      ■      ►►            vol ████████░░
```

`Space` pauses, the left and right arrows change track, up/down is the volume, `i`
inserts (the highlighted folder, or the folder being browsed), `r` toggles
shuffle, `Esc` opens the command line in the same system line every module has.
Coming back to Riff with a disc still in lands on the player, not the front door. Everything there also
works as a command, from **any** prompt, even before Riff was opened once:

| Command | What it does |
| --- | --- |
| `riff play [track]` | resume, or start the list (optionally a named track) |
| `riff stop` | silence it |
| `riff pause` | play/pause |
| `riff next` / `riff prev` | walk the list |
| `riff random [on\|off]` | shuffle |
| `riff volume [0-100]` | loudness, remembered |
| `riff insert` | the music browser (inside Riff) |
| `riff status` | what is playing, in the status line |

The sound lives in the shell, not in the program: that is why it keeps playing
while you are writing in Inspiro or painting in Softhand — and why their status
line carries `♪ 01 Aufwachen · 00:42`, so you always know what is running. When
the bar is too narrow for both, the hint gives way to the track.

## Pocketboi

Pocketboi is a Game Boy on the same raster as everything else. It runs
[Mizu](https://github.com/Amjad50/mizu), a GB/GBC emulator in Rust, vendored
under `vendor/mizu-core` with a handful of patches (see `vendor/README.md`): the
upstream code printed to stdout during normal play and panicked on an illegal
opcode.

Cartridges live in `<vault>/Roms` — drop `.gb` and `.gbc` files there and the
module lists them, folders included. `Enter` plays, `Backspace` goes up a folder.

The machine's 160x144 screen is reduced to four shades and blown up by two, so it
sits centred in the 640x384 raster, with the key legend underneath. The buttons
sit where they sit on every keyboard layout — `Space` is A, `C` is B, `Enter` or
the right `Ctrl` is Start, the left `Ctrl` is Select — so a German or French
keyboard does not shuffle them. The picture is drawn in four greys by default;
`P` cycles to the classic green and to amber. The first `exit` puts the cartridge
back on the shelf, the second leaves the module. A game pad works too — any
controller the system knows about (a SNES-style one maps A to east, B to south,
Start and Select to their names, and the left stick doubles the D-pad); when one
is connected, the legend says so. The keyboard keeps working either way. The cartridge runs on a thread of its own, at the machine's 59.7275 Hz, so its
sound does not depend on how fast the screen is drawn. It comes from Mizu's APU
on the shell's own audio device — the sound server's PCM, so it follows whichever
output the desktop is set to. The cartridge saves its battery RAM next to the ROM
when you leave.

Nothing is shipped with it: the ROMs are yours to bring.

## US Poker

US Poker is a five-card-draw machine, installed from the `SOFTWARE` shelf
like the other modules. Its front door is the supplied logo, the bankroll left
from the last visit and the highscore; `Enter` sits down.

The bankroll starts at `$10.00`. `B` steps the bet through `$0.10`, `$0.20`,
`$0.50`, `$1`, `$2`, `$5`, `$10` and `$20`, never above what is left. `1`–`5`
hold cards, `Enter` deals and draws, `T` takes a win, `R` opens the risk ladder,
`C` cashes out and leaves; on the lose screen `Y` starts again with `$10.00`
and `N` cashes out the empty wallet. The paytable is 9/6 Jacks or Better, paid in dollars
against the bet.

The risk ladder is the German kind: after a win its two adjacent fields blink,
and `Enter` stops the light. The timing is the show; the RNG decides. Success
moves one step up, failure one step down, `T` takes what is left, and `$0.00`
loses the round. The steps are `$0.15`, `$0.30`, `$0.40`, `$0.60`, `$0.80`,
`$1.20`, `$1.60`, `$2.40`, `$3`, `$4`, `$6`, `$8`, `$12`, `$16`, `$24`, `$32`,
`$48`, `$84` and `$140.00`. The top cannot be risked further.

The wallet, highscore and bankruptcy count live in
`~/.config/cread/poker.save`, not in the vault and not in WebDAV. `C`, `exit`
or leaving the module saves; the next visit continues from that amount. The
`BEST` line and the front door show the highest bankroll ever reached; the
front door also shows `BANKRUPTS`, which starts at 0 and rises by one every
time the bankroll falls below the minimum, forever.

When the bankroll cannot pay `$0.10`, the lose screen offers **RETRY? Y / N**.
`Y` starts a fresh game with `$10.00` and keeps the bankruptcy count; `N` cashes
out the empty wallet and returns to the front door. The short effects — the
three-note win, the loss buzzer, the card and coin clicks — are generated in
the program and go through `cread-sound`, like Pocketboi's speaker. `M` or
`mute` silences them.

## SCRIPTURE

SCRIPTURE is the English Bible reader, installed from the `SOFTWARE` shelf like
the other modules. Its cover uses the supplied SCRIPTURE logo; `Enter` opens the
reader. The left column lists all 66 books in their Old and New Testament
groups, the right side shows the chapter as flowing text with verse numbers in
the margin, and the right-hand scrollbar is the same one Inspiro uses.

`Tab` switches between the book list and the text, `Up`/`Down` chooses a book
or scrolls, `Left`/`Right` changes chapter, `Enter` opens the selected book and
`C` opens a chapter grid for long books such as Psalms. `Esc` opens the command
line; `Tab` moves focus between the book list and the text, and the active
panel gets a thin frame. `exit` returns to the CRead welcome screen, like the
other modules. The reader keeps its place in
`~/.config/cread/scripture.state`.

There is no separate light/dark or font-size switch: SCRIPTURE follows the
theme and face selected in SETTINGS. Its text is the public-domain Webster's
World Bible (1913); the German edition is not included. No Python or Qt code
from EZScripture is used — only the English text.

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
bleed and a gentle vignette. Nothing in the shell animates: the picture is static until
the text or the cursor changes. A program may ask for frames of its own —
Pocketboi while a cartridge runs, US Poker only while the risk ladder is
live.

On a 1280x800 panel that gives 640x400 at 2x — the real VGA text mode. On a
2560x1080 ultrawide it is 1280x540. Automatic scaling targets a comfortable line
height and follows the monitor's scale factor, so 4K at 200% gets 4x rather than
2x. Override it with `--scale N`.

Programs keep the classic 640x400 stage (80x25 cells, the game and its legend,
the canvas and its palette). On a bigger emulated screen the shell centres that
stage instead of leaving it in the corner, and the pointer mapping follows it.

There is no frame clock: a frame is drawn when the text changes and otherwise
only when the cursor is due to blink, so the app sits at a fraction of a percent
of a core while you read. Typing redraws immediately.

## Documents and sync

Documents are plain Markdown files in `Documents/` inside your XDG document
folder — `~/Dokumente/CRead` on a German desktop, `~/Documents/CRead` in English.
Override the vault with `--vault DIR` or `CREAD_VAULT`. Writes are atomic: temp
file, fsync, rename. A crash leaves either the old file or the new one, never
half a page.

WebDAV sync mirrors that folder — the whole vault, not just the notes in it:
documents, the `Pictures` folder, the cartridges in `Roms`, and any folder you
make yourself. The tree is recreated on the server, empty folders included, so a
ROM you drop into the server from your desktop shows up in Pocketboi on the next
round. Hidden entries (and the sync index) stay local. Run `settings` in action
mode and fill in:

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
| changed | changed | **keep both** — local file stays, the remote version is saved as `name.conflict-<unix>.md` |

Documents are never deleted on the server. The one exception is the editor's own
unnamed draft: `untitled-N.md` is marked in the sync index when the editor makes
it, and when you name it with `saveas` its server copy goes with the local
placeholder instead of coming back on the next round. `cread --sync-once` runs a
single round and prints the result, which is handy for cron or for debugging.

## Updates

`UPDATE` is the last switch in `SETTINGS`, and `update` works from any prompt.
It asks GitHub for the newest release and shows a progress window while it
waits; three things can come back:

- **you are current** — the status line says `0.22.0 is the newest`, nothing
  else happens;
- **a newer release exists** — `0.22.0 is available. Install and restart?`
  with Yes and No; `Esc`/No keeps it for later and the settings row shows
  `0.22.0 available`;
- **something went wrong** — the reason in the status line, and nothing on disk
  is touched.

`Esc` while it is checking cancels; an install is not cancellable, because the
binary may already be half replaced.

Yes downloads the tarball, checks it against the release's SHA-256 and its
**minisign signature**, and only then replaces the running binary. The previous
one is kept as `cread.old` next to it, so a bad release is one `mv` away from
being undone. The document is saved first, and the new binary is started with
the same arguments (`--vault`, `--file`, everything), which is the "restart" in
the question.

A few things it refuses on purpose: a release without a signature, a checksum
or signature that does not match, a tarball without a `cread` inside, a
development build (`target/debug/cread`), and a directory it cannot write to.
Only stable releases are considered — no pre-releases — and only the asset for
this architecture.

For scripts, `cread --update-once` does the same without the UI: it prints
`0.22.0 is the newest`, or installs and prints where the old binary went.

Releases are signed with a minisign key; [`minisign.pub`](minisign.pub) is the
public half, and you can check a download yourself:

```bash
minisign -V -p minisign.pub -m cread-linux-x86_64.tar.gz
# on a Pi the asset is cread-linux-aarch64.tar.gz
# or: rsign verify -p minisign.pub cread-linux-x86_64.tar.gz
```

## The bars

Screens with work to show wear the same bar in row zero: a name on the left, a
clock in the middle, a place on the right. The editor shows the file, its
`modified`/`saved` state (`autosave on` while autosave keeps it clean), the time
and the folder; Softhand does the same with
the picture name, its state and `~/Dokumente/CRead/Pictures`; the boot screen
only has the date and the time:

```
01.11.26 - 19:25                 (boot screen)
welcome.md  modified   19:25   ~/Dokumente/CRead      (editor)
bildname.png  modified  19:25  ~/Dokumente/CRead/Pictures   (Softhand)
```

Softhand's canvas and palette strip sit one bar row further down, so the canvas
keeps its full height and the clicks still land where the crosshair is.

`SETTINGS` chooses the clock (`TIME`: 24 or 12 hours, with AM/PM) and the date
(`DATE`: `DD.MM.YY` or `MM/DD/YY`). Both are remembered. The clock redraws with
the cursor blink, so it never costs an extra frame clock.

## Screensaver

After five minutes without a key or a mouse movement the screen goes black and
an analog clock appears in the middle — hour hand, minute hand and one red
second hand. Any key brings back whatever was behind it; the mouse does
nothing, so a nudged desk does not wake the machine.

It is the machine resting, not a program: it ignores the theme (black page,
fixed VGA colours), it is not on the `SOFTWARE` shelf, and the CRT pass runs
over it like over any other picture. The second hand steps once a second.

`SETTINGS` has the switch and the idle time (1, 2, 5, 10, 15, 30 or 60
minutes). From any prompt, `screensaver` starts it now, `screensaver off` and
`screensaver on` change the switch, and `screensaver 15` sets the minutes.

## Leaving with unsynced work

Quitting is the one moment where a background sync cannot help, so CRead asks
instead of guessing. If anything was written to disk since the last successful
round — a save, a new document, a picture, a delete or a rename — and the
server has not seen it yet, both `exit` on the boot screen and `exit!` show:

```
  ╔═ SYNC TO SERVER? ═════════════════════════════════════════╗
  ║                                                           ║
  ║  Changes in this session have not been sent yet.          ║
  ║                                                           ║
  ║                      [ Yes ]  [ No ]                      ║
  ╚═══════════════ Enter confirm   Esc no ═══════════════════╝
```

`Yes` runs one last round before the window closes; if the server cannot be
reached, CRead stays open with the error in the status line and nothing is lost.
`No`, `Esc` and closing the window while the box is up all leave at once. The
sidebar close button goes through the same question, after saving the open
document. With sync off there is nothing to ask, so the box never appears.

## Flags worth knowing

```
--scale N         force the integer upscale factor
--theme NAME      black | dos-blue | amber | green | paper
--no-crt          disable all effects
--backend NAME    vulkan | gl
--size WxH        window size (also drives --screenshot)
--vault DIR       use another folder as the vault
--kiosk           hand an update's restart to systemd (kiosk installs)
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

On a Raspberry Pi 4 or 5 the same binary runs as a kiosk under cage. On a
fresh Pi OS Lite the quickest path is the signed installer:

```sh
curl -fLO https://github.com/darknoxx/CRead/releases/latest/download/install.sh
less install.sh
sudo bash install.sh
```

The full recipe, including WebDAV, updates and SD-card notes, is in
[docs/raspberry-pi.md](docs/raspberry-pi.md).

## Layout

```
crates/core      buffer, cursor, soft wrap, undo, markdown spans, file IO
crates/render    emulated framebuffer, bitmap font, wgpu CRT pass
crates/sync      WebDAV client, conflict policy, background worker
crates/sound     which output device the machine opens
crates/app       window, input, frame loop, UI composition
crates/programs/ modules: one crate per program, compiled in
assets/          bitmap faces, boot logo
assets/splash…   pixel art, blitted into the emulated framebuffer
assets/inspiro…  the same, for Inspiro's front door
assets/softhand… and for Softhand's
assets/riff…     and for Riff's
assets/american… and for US Poker's front door
assets/scripture… and for SCRIPTURE's cover
LICENSE          MIT
THIRD-PARTY.md   what is not ours, and its terms
```

`crates/core` has no GPU, window or network dependency, so the tricky parts are
plain unit tests. `cargo test` runs all of them.

## Credits

Two bitmap faces ship with it: the classic IBM VGA 8x16 and the thinner ISO 9241
variant from the same pack. `font thin` swaps them at runtime and the choice is
remembered.

The boot logo (`assets/splash.png`), the Inspiro logo (`assets/inspiro.png`), the
Softhand logo (`assets/softhand.png`), the US Poker logo
(`assets/american_logo.png`) and the SCRIPTURE logo (`assets/scripture.png`) are
blitted at their native size in emulated pixels, so they keep hard edges
through the CRT upscale. Themes tint them with the text colour: ink on paper,
light on the dark schemes.

The character set is the IBM VGA 8x16 ROM font from VileR's *Ultimate Oldschool
PC Font Pack* (CC BY-SA 4.0, <https://int10h.org/oldschool-pc-fonts/>), reduced to
the raw 256 x 16 byte bitmap. The handful of glyphs CP437 never had (`€`, `„“”`,
`–—`, `…`) are hand-drawn in `crates/render/src/font.rs`.

## Licence

CRead is MIT — see [`LICENSE`](LICENSE). The pixel art in `assets/` is under the
same terms.

Not CRead's own work: the bitmap fonts are CC BY-SA 4.0 (VileR's *Ultimate
Oldschool PC Font Pack*), the vendored emulator `vendor/mizu-core` is MIT with
CRead's patches on top, and SCRIPTURE's English text is the public-domain
Webster's World Bible (1913). [`THIRD-PARTY.md`](THIRD-PARTY.md) has the details
and the full dependency list.
