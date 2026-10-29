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
- `help` lists the rest, including the screen effects.

## Mostly invisible

Your documents are plain Markdown in `~/Documents/CRead`. Autosave
runs 1.5 seconds after you stop typing, and writes are atomic, so a
crash cannot leave half a page behind.

Headings are bright, `code` sits on an inverse cell, **strong** text
gets a double strike and *emphasis* is sheared a little to the right.
The markers stay visible, only dimmed, so the text never reflows
while you type.

---

Press `Esc`, type `help`, and press `Enter`.
