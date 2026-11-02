# CRead OS

## User Manual

**2026 Edition**

For beginners. No prior knowledge needed.

---

> **Before you begin**
>
> CRead OS is a program that reminds us of a nostalgic retro
> computer. The screen has room only for what is necessary.
>
> You need: a Linux computer and a keyboard.
> A mouse is optional.
>
> This manual has two parts. Part I gets you to your first document in ten
> minutes. Part II explains everything at a calmer pace.

## Contents

**Part I — Quick Start**

1. Starting
2. Opening the editor
3. Making a new sheet
4. Writing
5. Saving
6. Finding the document again
7. Quitting
8. The three ground rules

**Part II — The Manual**

1. The start screen
2. Two modes
3. Documents and the vault
4. Inspiro — the editor
5. Softhand — painting pictures
6. Riff — listening to music
7. Pocketboi — games
8. American Poker — the machine
9. The settings
10. The screensaver
11. Backing up with WebDAV
12. Updates

**Appendices**

- A — Commands from A to Z
- B — Keys in one place
- C — When something goes wrong
- D — Where are my files?
- E — About this program

---

# Part I — Quick Start

## First document in ten minutes

### Step 1: Starting

Open a terminal and type:

```
cread
```

Press Enter.

The screen turns black. Then the start screen appears.
Date and time are centred at the top. Below them you see a list.

### Step 2: Opening the editor

At the top of the list is:

```
A: INSPIRO
```

Press **Enter**. A short loading bar appears.
After that you see Inspiro's front door with the instruction:
`type new or open to start`.

### Step 3: Making a new sheet

Type:

```
new
```

and press Enter.

A small block is blinking now. That is the **cursor**. It shows where the
next letter will appear. You are in the editor.

### Step 4: Writing

Just start typing. For example:

```
Dear Mrs Meier,

I am writing this letter on a computer that seems to be from another time.
It has no windows. That is on purpose.
```

The text appears on the screen as you type.

### Step 5: Saving

Press **Esc**. The bottom line becomes the prompt. Type:

```
saveas letter
```

and press Enter.

Your text is now the file `letter.md` in the `Documents` folder.
The program tells you so in the bottom line.

> **Tip:** In the text, Esc opens the prompt. A second Esc takes you back
> to the text.

### Step 6: Finding the document again

Press **Esc**. Type:

```
open
```

and press Enter.

A window opens with the `Documents` folder. Your letter is in the list.
Choose it with the arrow keys. Press Enter. The letter is back.

### Step 7: Quitting

Press **Esc**. Type:

```
exit
```

and press Enter.

You are back at the CRead OS welcome screen.
If you want to leave CRead OS completely, type once more from the welcome
screen:

```
exit
```

If you have changes that are not on the server yet, CRead asks first.
Answer with **N** or press **Esc** if you do not want to sync.

That is it. You can write.

## The three ground rules

| Rule | Meaning |
| --- | --- |
| **Esc** | Opens the prompt. Pressed again: back to the text. |
| **Enter** | Runs what is in the prompt. |
| **help** | Shows the commands for the screen you are on. |

> **Note:** CRead has no menus to click. It has commands to type. It feels
> unusual, but it is surprisingly fast.

---

# Part II — The Manual

## Chapter 1: The start screen

After starting you see the start screen. It is the control centre of the
whole system. From here you reach everything you need:

```
A: INSPIRO       where your ideas have room to grow.
B: SOFTHAND      16 colors, endless possibilities.
C: RIFF          a Soundtrack for your life.
D: POCKETBOI     get gaming, my friend!
E: AMERICAN POKER ten dollars and a dream.
─────────────────────────────────────────────────
SOFTWARE         install and remove modules
THE VAULT        documents, pictures, cartridges
SETTINGS         webdav, theme, font, screen
```

### The drive letters

The rows with `A:`, `B:`, `C:` are the **modules**. A module is a program.
Inspiro is the only system program and always stays installed. You add the
others yourself. That keeps your welcome screen tidy.

The letters follow the order in which you installed the modules. If you
install Riff first, you get `B: RIFF`.

### The three rows below the rule

| Row | What it does |
| --- | --- |
| **SOFTWARE** | Shows the software shelf. That is where you install modules. |
| **THE VAULT** | Shows all the folders with your files. |
| **SETTINGS** | Shows the settings of CRead OS. |

### Using it

- The **arrow keys** choose a row.
- **Enter** opens it.
- At the bottom you can also type a command directly, for example `new`.

---

## Chapter 2: Two modes

CRead OS has two states. This is the most important idea in the whole
program.

### Write mode

You type. The text appears. The cursor blinks. That is all.

### Command mode

You press **Esc**. The bottom line becomes the prompt. It looks like
this:

```
> save▌
```

Type a command. Press **Enter**. The command runs.

Press **Esc** on an empty prompt to go back.

### The prompt helps

- **Tab** completes the command. `sav` becomes `save`.
- The **up and down arrow keys** show earlier commands.
- On the right there is a short hint about what the command does.
- A wrong command is reported as an error. Nothing bad happens.

> **Note:** The prompt is not a menu. It takes commands: `open`, `save`,
> `sync`.

---

## Chapter 3: Documents and the vault

### Where are my files?

All your files live in a single folder. It is called the **vault**.
By default that is:

```
~/Dokumente/CRead
```

On an English system the folder is `~/Documents/CRead`.

The vault has three main folders:

| Folder | Contents |
| --- | --- |
| `Documents` | Your texts. Every file ends in `.md`. |
| `Pictures` | Your pictures. Every file ends in `.png`. |
| `Roms` | Your Game Boy games. Every file ends in `.gb` or `.gbc`. |

### The vault browser

The `open` command opens a window. In the editor it shows the `Documents`
folder first. On the start screen it shows the whole vault.

- The **arrow keys** choose a row.
- **Enter** opens a folder or a file.
- **Esc** or the **left arrow** goes one level up.
- **Typing** filters the list. `not` shows only files with "not" in the name.
- **Backspace** deletes one letter of the filter.

Folders end with a slash, for example `chapter/`.
A file opens with the program that owns it:

| File | Program |
| --- | --- |
| `.md`, `.txt` | Inspiro (editor) |
| `.png` | Softhand (pictures) |
| `.gb`, `.gbc` | Pocketboi (games) |

### Saving

CRead does **not** save by itself. You decide when to save. That is on
purpose.

| Command | Effect |
| --- | --- |
| `save` | Saves under the current name. |
| `saveas name` | Saves under a new name in `Documents`. |
| `saveas chapter/one` | Creates a subfolder and saves there. |

A new document starts as `untitled.md`, then `untitled-2.md` and so on.
With `saveas letter` or `saveas invoice` you give it a real name. The
placeholder file is removed so no pointless duplicates are left behind.

> **Tip:** You can switch on automatic saving. Type `autosave on`. CRead
> then writes 1.5 seconds after the last keystroke. While the document is
> clean, the top bar says `autosave on` instead of `saved`. By default
> autosave is off, though.

### Renaming and deleting

| Command | Effect |
| --- | --- |
| `rename newname` | Renames the current document. |
| `delete` | Deletes it after asking once. |

In the vault browser you can do this without typing:

- **Ctrl+R** renames the chosen file.
- **Ctrl+D** deletes the chosen file.

### Opening a document directly

If you know the name, it is faster:

```
open letter
open chapter/one
```

CRead searches the whole `Documents` folder.

---

## Chapter 4: Inspiro — the editor

Inspiro is the standard writing program in CRead OS. It is always
installed.

### The screen

A narrow bar runs along the top:

```
letter.md  modified        14:32        ~/Dokumente/CRead
```

| Part | Meaning |
| --- | --- |
| Left | The file name and its state (`modified`, `saved`, or `autosave on`). |
| Middle | The time. |
| Right | The folder on your Linux distribution where the file lives. |

When autosave is on, a clean document says `autosave on` instead of `saved`.
`modified` still appears while text is waiting to be written.

Below it is the body of the text. Under that is a line with the word count
and the cursor position. On the right edge a thin bar shows how far you are
in the text. It moves as you scroll.

### Typing and moving

| Key | Effect |
| --- | --- |
| Letters | Write |
| Arrow keys | Move the cursor |
| Home / End | Start or end of the line |
| Page Up / Page Down | One page further |
| Backspace | Delete one character |
| Delete | Delete one character forwards |

### Marking and copying text

| Key | Effect |
| --- | --- |
| Shift + arrow keys | Mark text |
| Ctrl+A | Mark everything |
| Ctrl+C | Copy |
| Ctrl+X | Cut |
| Ctrl+V | Paste |
| Ctrl+Z | Undo |
| Ctrl+Shift+Z | Redo |

The mouse works too: clicking sets the cursor. Dragging marks text.
The wheel scrolls.

### Searching

Press **Ctrl+F**. A search line appears at the bottom. Type a word.
Every hit is marked.

| Key | Effect |
| --- | --- |
| Enter | The cursor stays at the hit |
| F3 | Next hit |
| Shift+F3 | Previous hit |
| Esc | Cancel the search |

### Reading mode

The `view` command shows the text without markup. Headings get a colour,
asterisks disappear. It is for reading. You cannot edit there.

Press **Esc** to come back.

### The typewriter

The `typewriter` command keeps the typing line in the middle of the
screen. The text rolls as you type. A matter of taste. It is off by
default.

---

## Chapter 5: Softhand — painting pictures

Softhand is the paint program. Install it first through **SOFTWARE**.

### The screen

The same bar runs along the top as in the editor:

```
picture.png  saved        14:35        ~/Dokumente/CRead/Pictures
```

Below it is the white surface. At the very bottom is the colour palette
with sixteen colours.

### The tools

Softhand is a mouse program. The most important tools have keys:

| Key | Tool |
| --- | --- |
| B | Pencil |
| E | Eraser |
| L | Line |
| R | Rectangle |
| F | Fill |
| I | Pick a colour |

| Key | Effect |
| --- | --- |
| Comma / full stop | Previous or next colour |
| Minus / Plus | Smaller or larger brush |
| U or Ctrl+Z | Undo |
| Y or Ctrl+Y | Redo |
| Esc | Prompt |

You paint directly with the mouse. The palette at the bottom picks a
colour. Dragging paints a stroke.

### Saving

| Command | Effect |
| --- | --- |
| `save` | Saves as `picture.png`, `picture-2.png` and so on. |
| `saveas name` | Saves under that name in `Pictures`. |

Pictures always live in `Pictures`. Subfolders are allowed:
`saveas sketch/blue`.

### More commands

| Command | Effect |
| --- | --- |
| `open` | Shows all pictures in `Pictures`. |
| `tool fill` | Chooses the fill tool. |
| `size 6` | Sets the brush size. |
| `colour 12` | Chooses colour number 12. |

---

## Chapter 6: Riff — listening to music

Riff is the music player. Install it through **SOFTWARE**.

Riff plays from your XDG music folder. That is the folder the system
knows: `~/Musik` or `~/Music`. CRead OS does not care which language your
Linux distribution runs in.

### Loading music

Type `riff insert`. A window with your folders opens.

| Key | Effect |
| --- | --- |
| Arrow keys | Move the selection |
| Enter | Open a folder or play a track |
| I | Load the whole folder |
| Backspace or left arrow | One level up |
| Esc | Back to the player |

The music plays. You can change the screen. The music keeps playing.

### The player

| Key | Effect |
| --- | --- |
| Space | Pause or continue |
| Left / right arrow | Previous or next track |
| Up / down arrow | Volume |
| R | Random order |
| Esc | Prompt |

### Commands

| Command | Effect |
| --- | --- |
| `riff play` | Play |
| `riff stop` | Stop |
| `riff next` | Next track |
| `riff prev` | Previous track |
| `riff volume 40` | Set the volume |
| `riff random` | Random order |
| `riff status` | What is playing? |

The `riff` command works from anywhere and inside every other module.
From a game, from the editor, from the start screen.

---

## Chapter 7: PocketBoi — games

Pocketboi is a GB emulator. It plays games that sit as files in
`Roms`. Install PocketBoi through **SOFTWARE**.

### The cartridge shelf

Choose the PocketBoi row on the start screen. You see a shelf with your
games.

| Key | Effect |
| --- | --- |
| Arrow keys | Move the selection |
| Enter | Start a game |
| Backspace | Leave the folder |
| Esc | Leave the module |

### The keys in a game

| Key | Game Boy |
| --- | --- |
| Space | A |
| C | B |
| Enter or right Ctrl | Start |
| Left Ctrl | Select |
| Arrow keys | D-pad |
| P | Switch the palette (grey, green, amber) |
| Esc | Prompt |

A game pad works too. A is on the right, B at the bottom, Start and
Select are named as on the pad. The left stick works like the D-pad.

### Leaving

The first **Esc** puts the game back on the shelf. The second Esc leaves
the module.

The battery save is written next to the game file when you leave. That
happens automatically. `game.gb` becomes `game.gb.sav`, for example.

### Sound

In a game, press Esc and type `mute` to silence Pocketboi.
`mute` again: sound back on.

---

## Chapter 8: American Poker — the machine

American Poker is a five-card-draw machine. Install it through
**SOFTWARE**, like the other modules. Choose its drive on the start screen:
the machine opens on its logo with the bankroll from the last visit and the
highscore. Press **Enter** to sit down.

You start with **$10.00**. The smallest bet is **$0.10**; `B` steps the bet
through `$0.10`, `$0.20`, `$0.50`, `$1`, `$2`, `$5`, `$10` and `$20`, but never
above the bankroll. The paytable is 9/6 Jacks or Better, paid against the bet.

| Key | Effect |
| --- | --- |
| 1–5 | Hold or release that card. |
| Enter | Deal, draw, take a win, or continue after a loss. |
| B | Raise the bet to the next affordable step. |
| R | Risk the current win on the ladder. |
| T | Take the current win. |
| C | Cash out and leave; the bankroll is saved. |
| N | New game with $10.00 after the bankroll is gone. |
| M | Mute or unmute the machine. |
| Esc | The command line (`mute`, `exit`). |

### The risk ladder

After a win, `R` opens the ladder. The two steps next to the current win blink
alternately: the higher one is the possible win, the lower one the fallback.
`Enter` stops the light. The timing is theatre; the machine's RNG decides.
Success moves one step up, failure one step down. `T` takes what is left; at
`$0.00` the win for that round is gone. The ladder ends at `$140.00`; a larger
main-game win is taken directly.

The steps are `$0.15`, `$0.30`, `$0.40`, `$0.60`, `$0.80`, `$1.20`, `$1.60`,
`$2.40`, `$3`, `$4`, `$6`, `$8`, `$12`, `$16`, `$24`, `$32`, `$48`, `$84` and
`$140.00`, with `$0.00` as the loss.

### The bankroll

`C` (or `exit`) takes a held win, saves the bankroll and returns to the front
door. The next visit starts with exactly that amount and the saved highscore.
The file is `~/.config/cread/poker.save`; it is not in the vault and not synced.
The front door and the `BEST` line show the highest bankroll ever reached.

If the bankroll cannot pay the minimum, the machine shows **GAME OVER**; `N`
starts a fresh game with `$10.00`. The highscore is kept.

The short sounds are generated: the win makes three rising clings, a loss
buzzes, and the deal, hold and cash-out click. `M` or `mute` silences them.

---

## Chapter 9: The settings

You reach **SETTINGS** from the start screen. The rows are sorted into
groups.

### Sync

| Row | Effect |
| --- | --- |
| WEBDAV | Opens the form for backing up over the network. |

### Look

| Row | Effect |
| --- | --- |
| THEME | The screen's style. Enter: next style. |
| FONT | The typeface. `classic` is the wide VGA face. `thin` is narrower. |
| SCREEN | Window or full screen. |
| SCREENSAVER | Screensaver on or off. |
| IDLE | Minutes until the screensaver: 1, 2, 5, 10, 15, 30 or 60. |

The styles are: `paper` (paper, the default), `black` (black),
`dos-blue` (DOS blue), `amber` (amber), `green` (green).

You can also change the colours with the `theme` command. For example:
`theme amber`.

### Clock and date

| Row | Effect |
| --- | --- |
| TIME | 24 hours or 12 hours with AM/PM. |
| DATE | `DD.MM.YY` or `MM/DD/YY`. |

You see both in the top bar. The start screen shows the date and the
time. The other screens show only the time.

### CRead OS

| Row | Effect |
| --- | --- |
| INFO | Version, credits and device info. |
| UPDATE | Looks for a new version. |
| SOFTWARE | The software shelf. |
| STORAGE | How full the disk is. Enter refreshes it. |
| NETWORK | Your local IP address. Enter refreshes it. |
| USER MANUAL | Opens this manual. Nothing in it can be changed. |

### Using it

- The **arrow keys** choose.
- **Enter** changes the setting.
- **Esc** goes back.

---

## Chapter 10: The screensaver

If you do nothing for a while, the screen turns black. A large clock with
hour, minute and second hands appears in the middle.

By default this happens after **five minutes**.

| What | Effect |
| --- | --- |
| Any key | Wakes the screen. The key is not used for anything else. |
| Moving the mouse | Does **not** wake it. |
| Clicking the mouse | Is ignored. |

You can start the screensaver right away, too:

```
screensaver
```

In the settings you switch it off or change the time. The command
`screensaver off` switches it off as well. `screensaver 15` sets fifteen
minutes.

---

## Chapter 11: Backing up with WebDAV

CRead can back up your files to a server automatically. That is
voluntary. Without settings, nothing happens.

### Setting it up

1. Open **SETTINGS**.
2. Choose **WEBDAV**.
3. Fill in the fields:

| Field | Example |
| --- | --- |
| server | `https://cloud.example.org/remote.php/dav/files/me` |
| user | Your user name |
| password | Your password |
| folder | `cread` |
| sync | `on` |

**Tab** moves to the next field. **Ctrl+S** saves the form. **Enter** in
the `sync` field switches the backup on or off; a small box confirms
`SYNC ON` or `SYNC OFF`. **Esc** leaves the form.

### What is backed up

The whole vault. That means texts, pictures and games. The folders too.

### When it is backed up

- When CRead starts.
- After saving.
- Every 60 seconds.
- When you type `sync`.

### When both sides have changed

CRead never overwrites anything. Your local file stays. The server's
version is saved next to it as a copy:

```
letter.conflict-1699999999.md
```

You can look at that file and delete it when you are done.

### Nothing is deleted

CRead does not delete documents on the server. There is one exception:
the placeholder file `untitled-N.md` also disappears from the server when
you give the document a real name.

### When quitting

If something changed in this session and has not been backed up yet,
CRead asks:

```
SYNC TO SERVER?
[ Yes ]  [ No ]
```

- **Yes**: CRead backs up once more and then quits.
- **No** or **Esc**: CRead quits right away.

---

## Chapter 12: Updates

CRead can update itself. That is voluntary and only happens when you
start it.

1. Open **SETTINGS**.
2. Choose **UPDATE**.
3. A window with a loading bar appears.
4. If everything is current, the bottom says: `<your version> is the newest`.
   For example: `0.24.2 is the newest`.
5. If there is a new version, this appears:

```
UPDATE AVAILABLE
0.25.0 is available. Install and restart?
[ Yes ]  [ No ]
```

With **Yes**, CRead downloads the new version. It checks the file. Then it
replaces itself and restarts. Your work is saved first.

The old version stays next to the new one as `cread.old`. If something is
wrong, you can switch back.

> **Safety:** CRead only installs a version with a valid signature. If
> the signature is missing, nothing is installed. An error message
> appears in the bottom line.

For scripts there is this command:

```
cread --update-once
```

It checks, installs and quits. Without a restart.

---

# Appendix A — Commands from A to Z

The most important commands. Upper and lower case do not matter.

| Command | Alias | What it does |
| --- | --- | --- |
| `about` | `version` | Shows the version and acknowledgements. |
| `autosave [on\|off]` | `aw` | Saves while you type. |
| `beep [on\|off]` | `bell` | The beep at startup. |
| `bloom [on\|off]` | | The glow around the letters. |
| `colour [0-15]` | `color`, `ink` | Colour in Softhand. |
| `delete` | `rm`, `del` | Deletes the document. |
| `exit` | `welcome`, `home` | Back to the start screen. |
| `exit!` | `quit`, `q`, `x` | Save and quit. |
| `find [text]` | `search` | Searches the document. |
| `font [classic\|thin]` | `weight` | Switches the typeface. |
| `fullscreen` | `fs` | The whole screen. |
| `help [all]` | `?`, `h` | Shows the commands. |
| `manual` | `handbook` | Shows this manual. |
| `mute` | `silence` | Silences the running program. |
| `new` | `n` | New document. |
| `open [name]` | `o`, `e` | Shows the vault or opens a name. |
| `redo` | | Redo. |
| `rename name` | `mv`, `rn` | Renames the document. |
| `riff [command]` | `music` | The music player. |
| `roll [on\|off]` | `drift` | The scanlines drift. |
| `save` | `w`, `safe`, `write` | Saves. |
| `shutdown` | `poweroff` | Saves and powers the machine off. |
| `reboot` | `restart` | Saves and restarts the machine. |
| `backtolinux` | `linux` | Leaves the kiosk and opens a Linux login. |
| `saveas name` | `sa` | Saves under a new name. |
| `scanlines [on\|off]` | `scan`, `sl` | The fine lines. |
| `screensaver [on\|off\|minutes]` | `saver` | The screensaver. |
| `settings` | `set`, `cfg` | The settings. |
| `size [1-16]` | `brush`, `width` | Brush size in Softhand. |
| `software` | `install`, `modules` | The module shelf. |
| `sync` | | Back up now. |
| `theme [name]` | | Change the colours. |
| `tool [name]` | `tools` | Tool in Softhand. |
| `typewriter [on\|off]` | `tw` | Typing line in the middle. |
| `undo` | `u` | Undo. |
| `update` | `up` | Look for a new version. |
| `view` | `read` | Reading mode. |
| `windowed` | `win` | Back to a window. |

---

# Appendix B — Keys in one place

## Everywhere

| Key | Effect |
| --- | --- |
| Esc | Open the prompt or go back. |
| Enter | Run it. |
| Tab | Complete the command. |
| Up / down arrow | Earlier commands or the selection. |

## In the editor

| Key | Effect |
| --- | --- |
| Ctrl+A | Mark everything. |
| Ctrl+C / X / V | Copy, cut, paste. |
| Ctrl+Z / Ctrl+Shift+Z | Undo / redo. |
| Ctrl+F | Search. |
| F3 / Shift+F3 | Next / previous hit. |

## In the vault browser

| Key | Effect |
| --- | --- |
| Enter | Open. |
| Esc or left arrow | One level up. |
| Typing | Filter the list. |
| Ctrl+R | Rename. |
| Ctrl+D | Delete. |

## In Softhand

| Key | Tool |
| --- | --- |
| B / E / L / R / F / I | Pencil, eraser, line, rectangle, fill, pick. |
| Comma / full stop | Colour. |
| Minus / Plus | Brush size. |
| U / Y | Undo / redo. |

## In Pocketboi

| Key | Effect |
| --- | --- |
| Space / C | A / B |
| Enter / right Ctrl | Start |
| Left Ctrl | Select |
| Arrow keys | D-pad |
| P | Switch colours |

## In Riff

| Key | Effect |
| --- | --- |
| Space | Pause. |
| Left / right arrow | Track. |
| Up / down arrow | Volume. |
| I | Load music. |
| R | Random. |

## In American Poker

| Key | Effect |
| --- | --- |
| 1–5 | Hold a card. |
| Enter | Deal, draw, take, or risk. |
| B | Raise the bet. |
| R | Risk the win. |
| T | Take the win. |
| C | Cash out and save. |
| N | New game after game over. |
| M | Mute. |

---

# Appendix C — When something goes wrong

### My text is gone

CRead does not save by itself. Check:

1. Did you type `save` or `saveas`?
2. Does the top bar say `saved`, `autosave on`, or `modified`?
3. Type `open` and look.

If the file is still open: press Esc and save at once.

### The screen is black with a clock

That is the screensaver. Press any key.

### No sound in Riff, Pocketboi or American Poker

CRead uses the system's output device. Check the system settings to see
whether speakers or headphones are selected. Then start CRead again.

### A game does not start

Check:

1. Is the file in `Roms`?
2. Does it end in `.gb` or `.gbc`?
3. Is Pocketboi installed? Look in **SOFTWARE**.

### Backing up does not work

Check the fields in **SETTINGS → WEBDAV**. Watch the exact address of the
server. The bottom line shows a message. It starts with `sync failed:`.

### Updating the Raspberry Pi OS

If CRead runs as a kiosk on a Raspberry Pi, `backtolinux` saves and opens a
normal Linux login on the screen. Log in there, then update the system:

```
sudo apt update
sudo apt full-upgrade -y
sudo reboot
```

The kiosk starts again after the reboot. `reboot` restarts the machine from
CRead, `shutdown` powers it off, and `backtolinux` only leaves CRead for the
Linux console. Do not use an overlay or read-only root: the updates and the
in-app updater need to write to the card.

### The update is not installed

The bottom line says why. The cause can be an unsigned release, which
CRead OS refuses for your protection, or a read-only folder. In that case
download the new version from the project page and replace the file by
hand. *https://github.com/darknoxx/CRead*

### The typeface is too small or too large

Press `fullscreen`. If that does not help, start CRead with a fixed size:

```
cread --scale 2
```

### My files are not where I expect them

Choose `THE VAULT` on the start screen. There you see all the folders.
If you use a different vault, start like this:

```
cread --vault /path/to/folder
```

---

# Appendix D — Where are my files?

| What | Where |
| --- | --- |
| The vault | `~/Dokumente/CRead` (or `~/Documents/CRead`) |
| Texts | `Documents/` in the vault |
| Pictures | `Pictures/` in the vault |
| Games | `Roms/` in the vault |
| Music | `~/Musik` or `~/Music` |
| Settings | `~/.config/cread/` |
| WebDAV backup | `~/.config/cread/sync.json` |

> **Note:** The files are ordinary files. You can open them with another
> program too. Texts are Markdown. Pictures are PNG.

---

# Appendix E — About this program

CRead OS is a small, fictional operating system. It is a writing program
in the clothes of a retro computer. There are no windows, no icons and no
notifications. Only you and your ideas.

**Version:** shown in **SETTINGS → INFO**.

**Licence:** MIT.

**Typeface:** IBM VGA 8x16 from VileR's *Ultimate Oldschool PC Font
Pack*, CC BY-SA 4.0, <https://int10h.org/oldschool-pc-fonts/>.

**Emulator:** Pocketboi uses mizu-core 1.3.0 (MIT), a Game Boy emulator
in Rust, with small changes: <https://github.com/Amjad50/mizu>.

**Acknowledgements:** to everyone who misses the calm of older computers.

---
