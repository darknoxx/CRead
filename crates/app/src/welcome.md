# Welcome to CRead

A writing terminal that pretends it is 1987.

There is no sidebar, no ribbon and no cloud icon. There is a black
screen, a block cursor, and your text.

## Two modes

You start in *write* mode. Type. That is the whole job.

Press `Esc` and the bottom line becomes a command prompt, like a DOS
shell. Type a command, press `Enter`, and you are back in the text.
`Esc` on an empty prompt leaves it again.

> The prompt is not a menu. It takes names: `open`, `save`, `sync`.
> `Tab` completes, the arrow keys walk the history.

## The commands you will actually use

- `save` writes the document (aliases `w`, `safe`).
- `saveas <name>` puts it in the vault under a new name.
- `open` shows the file list, `open <name>` opens it directly.
- `rename <name>` and `delete` tidy up the vault.
- `sync` pushes everything to WebDAV.
- `exit` goes back to the boot screen, `exit!` quits from anywhere.
- `help` lists the rest, including the screen effects.

## Marking text

- `Shift` and the arrows mark text, `Ctrl+A` marks everything.
- `Ctrl+C`, `Ctrl+X`, `Ctrl+V` copy, cut and paste.
- Typing or `Backspace` replaces whatever is marked.
- The mouse works too: click to place the cursor, drag to mark, wheel to
  scroll.

## Finding things

`Ctrl+F` opens a search prompt. Type, and every hit is marked while the
cursor jumps to the first one; `F3` walks forward, `Shift+F3` back.
`Enter` keeps the cursor where it landed, `Esc` puts it back.

## Reading

`view` strips the markup and shows the document as a reader sees it. It opens
where your cursor was; `Esc` takes you back to the text.

## Files

`open` lists the vault. Inside the list, `Ctrl+R` renames the highlighted
file and `Ctrl+D` deletes it.

## Mostly invisible

Your documents are plain Markdown in `~/Documents/CRead`. Autosave
runs 1.5 seconds after you stop typing, and writes are atomic, so a
crash cannot leave half a page behind.

Headings and **strong** and *emphasis* are simply coloured — no bold, no
slant. At eight pixels wide, synthesised weight and shear only ever made the
glyphs harder to read. The markers stay visible, only dimmed, so the text
never reflows while you type.

---

Press `Esc`, type `help`, and press `Enter`.
