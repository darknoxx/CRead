# AGENTS.md

Notes for anyone (human or agent) changing this repository.

## What this is

CRead is a fullscreen writing terminal that looks like a 1987 DOS machine: one
binary, a black screen, an IBM VGA bitmap font and a CRT pass. Documents are
plain Markdown in an XDG folder and sync over WebDAV.

Rust, edition 2024, no async in the UI thread, no web view.

## Commands

```bash
cargo build                     # debug build, optimised deps (see Cargo.toml)
cargo test                      # 160 tests, under a second
cargo fmt --all                 # CI enforces --check
cargo clippy --all-targets      # CI enforces -D warnings
cargo run -p cread -- --demo -w # run without taking over the screen
cargo install --path crates/app --force   # refresh ~/.cargo/bin/cread
```

`cargo run` needs `-p cread`: this is a virtual workspace.

Visual checks without a display, and the way every effect change is verified:

```bash
cread --demo -w --size 1280x800 --screenshot /tmp/shot.png
cread --show welcome|action|search|palette|settings|help --screenshot /tmp/shot.png
cread --demo --size 1280x800 --show action --command "sca" --screenshot /tmp/shot.png
```

The last one prefills the prompt, which is the only way to screenshot a typed
command state.

## Layout

| Crate | Owns | Rule |
| --- | --- | --- |
| `crates/core` | buffer, cursor, wrap, undo, markdown spans, command parsing, themes, prefs, file IO | **no GPU, no window, no network** — this is what makes the tricky parts unit-testable |
| `crates/render` | bitmap font, CPU compositor, wgpu pipeline, CRT shader | owns everything that touches the GPU |
| `crates/sync` | WebDAV client, conflict policy, background worker | blocking calls only, never on the UI thread |
| `crates/app` | winit loop, input, mode machine, cell-grid composition | glue; keep logic out of it when it can live in core |
| `crates/programs/*` | one crate per module (Inspiro's editor lives in `app`, Softhand here) | a program owns its surface and gestures, never the prompt or the files |

## Invariants

**Never scale the emulated raster by a fractional factor.** `cread_core::Viewport`
picks an integer `scale`, renders at `physical / scale` and upscales by exactly
that. That is the whole reason scanlines stay crisp on any display; a "fit to
window" mode would break it.

**Text is drawn at emulated resolution.** The CPU compositor writes glyph bits
into a 640x400-ish framebuffer; the GPU only adds scanlines, phosphor bleed and
a vignette. Do not render UI text at native resolution "for sharpness" — it
turns the look into a modern UI with a CRT filter.

**Nothing animates.** There is no frame clock. A frame is drawn when something
is `dirty` and `next_frame` is reached; the only scheduled redraw is the cursor
blink. Two consequences:

- Setting `self.dirty = true` alone does **not** draw anything. Input handlers
  must end up calling `poke()` (or rely on the choke point at the end of
  `App::on_key`). Forgetting this makes that movement lag behind the key repeat
  by up to 620 ms — the exact bug that made Up/Down feel unresponsive.
- Adding an effect that changes every frame (flicker, drift, animation) means
  bringing a frame clock back deliberately, not by accident. There are exactly
  three: scanline drift, the fake loading bar, and the CRT warm-up on launch
  (`BOOT_STEPS`, 0.9 s, skipped by any key and by every screenshot unless
  `--boot`).

**The CRT uniform layout must match the WGSL struct byte for byte.** `CrtUniforms`
is 32 bytes and has a `const` assertion; the shader pads to the same size. A
mismatch does not error — the wrong field is read and the screen can go black.

**The logo blit happens after the cell grid.** `Framebuffer::draw_splash` runs
after `draw_screen`, so anything drawn into the grid (palette, settings form)
would be painted over. Overlays therefore hide the splash.

**Bitmaps only.** The font is 1-bit; styles are synthesised (bold = right shift
or, italic = row shear). No anti-aliasing, no sub-pixel positioning, no TTF
rasterising. Two faces ship: `Face::Classic` (VGA) and `Face::Thin` (ISO 9241).

**Programs are compiled in; the diskette is the mount.** A module is a crate in
`crates/programs` plus a row in the registry in `app.rs`. A `.dsk` whose header
says `program=<id>` puts that row on the boot screen under its own letter and
nothing more: there is no runtime, no loading of code, and `Disks` is a mount
point, not a plugin directory.

**Two surfaces, one raster.** A program draws pixels into `Raster` (16 fixed
colours, 640x368) and text into the `Screen` on top; cells can be transparent so
menus sit over a picture. The framebuffer only ever sees one composited image,
so the CRT pass stays the same for text and pictures.

**The clipboard handle must outlive the copy.** On Wayland the data is served by
the owning process; `arboard::Clipboard` is kept in `App` for exactly that
reason. Creating one per operation silently loses the text.

**Local files win.** Sync never overwrites a both-sides change: the remote copy
is parked as `name.conflict-<unix>.md` and the local text stays. Nothing is ever
deleted on the server.

## Conventions

- **Commands, not hotkeys.** Everything the user can do is a row in
  `cread_core::command::SPECS`. Adding a command means: `Command` variant, a
  `Spec`, a parse arm, and a `run_command` arm. The help screen and Tab
  completion read from `SPECS`, so they stay in sync for free.
- Keys stay for *editing gestures*: movement, selection, undo, clipboard, search
  navigation. Those are not commands.
- **User-facing strings are English**; comments and commit messages are English
  too. The one place German appears is the user's own documents.
- Errors go to the status line via `set_status` (transient, 4 s) — not to stderr
  in the UI, except for startup failures.
- Stripping markup for view mode lives in `markdown::render`, not in the UI.
  The UI only knows how to paint a `Rendered` row.
- `unsafe` is not used anywhere. Keep it that way.

## Tests

- Pure logic goes in `core` and gets a test. `wrap`, `find_all`, cursors on
  grapheme clusters, undo coalescing, command parsing, theme slots, XDG
  resolution, conflict matrix, percent-encoding.
- `render` tests cover font extraction, style synthesis, the compositor and the
  splash blitter.
- `app` tests exercise the cell grid: layout, clipping, palette box, selection
  and search highlighting. They do not need a window.
- Visual regressions: take a screenshot and look at it. Several real bugs
  (uniform layout, invisible hint, logo over the palette) were only caught that
  way.

## Documentation to keep in step

| Change | Also update |
| --- | --- |
| any command | nothing for `help` (generated), but `README.md`'s table and `crates/app/src/welcome.md` |
| behaviour worth mentioning | `CHANGELOG.md` under *Unreleased* |
| new asset | `assets/fonts/README.md` (provenance and licence) and the layout block in `README.md` |

## Release

1. Bump `version` in the workspace `Cargo.toml`.
2. Move the *Unreleased* CHANGELOG entries under `## X.Y.Z`.
3. `git tag -a vX.Y.Z -m "..."` and push both branch and tag.

The `release` workflow builds inside `ubuntu:22.04` (so the binary needs no
glibc newer than 2.34), packages `cread-linux-x86_64.tar.gz` with a `VERSION.txt`
and a checksum, uploads it as an artifact, and attaches it to a GitHub Release
whose body is the matching CHANGELOG section. `ci` runs fmt, clippy and tests on
every push and pull request.

Pushing is done over HTTPS with the `gh` credential helper; the SSH key here is
passphrase-protected and there is usually no agent running.
