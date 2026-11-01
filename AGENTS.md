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
cargo test                      # 206 tests, under a second
cargo fmt --all                 # CI enforces --check
cargo clippy --all-targets      # CI enforces -D warnings
cargo run -p cread -- --demo -w # run without taking over the screen
cargo install --path crates/app --force   # refresh ~/.cargo/bin/cread
```

`cargo run` needs `-p cread`: this is a virtual workspace.

Visual checks without a display, and the way every effect change is verified:

```bash
cread --demo -w --size 1280x800 --screenshot /tmp/shot.png
cread --show welcome|action|search|vault|settings|help --screenshot /tmp/shot.png
cread --demo --size 1280x800 --show action --command "sca" --screenshot /tmp/shot.png
```

The last one prefills the prompt, which is the only way to screenshot a typed
command state. Dev views beyond the everyday ones: `webdav` (the sync form),
`quit` (the exit question), `software`, `pictures`, `riffplay`, `riffinsert`,
`pocketboi[:<rom>]`, `picture[:<png>]`.

## Layout

| Crate | Owns | Rule |
| --- | --- | --- |
| `crates/core` | buffer, cursor, wrap, undo, markdown spans, command parsing, themes, prefs, file IO | **no GPU, no window, no network** — this is what makes the tricky parts unit-testable |
| `crates/render` | bitmap font, CPU compositor, wgpu pipeline, CRT shader | owns everything that touches the GPU |
| `crates/sync` | WebDAV client, conflict policy, background worker | blocking calls only, never on the UI thread |
| `crates/sound` | which output device the machine opens, for game and music alike | a policy both the shell and a program need, so it belongs to neither |
| `crates/update` | GitHub release check, download, checksum + minisign, atomic replacement | unsigned or unverified bytes are never installed |
| `crates/app` | winit loop, input, mode machine, cell-grid composition; the screens in `app/{boot,browser,compose,sync,webdav}.rs`, commands in `app/commands/*` | glue; keep logic out of it when it can live in core |
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
  four: scanline drift, the fake loading bar, the launch bar (`boot_at`), and
  the music clock (`riff_tick`: twice a second while a track plays, which is what
  starts the next track). They are ordered in `frame()` and the **fastest one
  wins**: the roll is checked before the music clock, or playing a track would
  drop the frame rate to 2 fps and freeze the raster.

**The CRT uniform layout must match the WGSL struct byte for byte.** `CrtUniforms`
is 32 bytes and has a `const` assertion; the shader pads to the same size. A
mismatch does not error — the wrong field is read and the screen can go black.

**The logo blit happens after the cell grid.** `Framebuffer::draw_splash` runs
after `draw_screen`, so anything drawn into the grid (browser, settings form)
would be painted over. Overlays therefore hide the splash. They also clear the
page behind them (`App::dialog_open`): a window stands alone, nothing from the
screen underneath may peek out around its frame.

**Bitmaps only.** The font is 1-bit; styles are synthesised (bold = right shift
or, italic = row shear). No anti-aliasing, no sub-pixel positioning, no TTF
rasterising. Two faces ship: `Face::Classic` (VGA) and `Face::Thin` (ISO 9241).

**Programs are compiled in; SOFTWARE is the shelf.** A module is a crate in
`crates/programs` plus an entry in `PROGRAMS` in `app.rs`. Nothing is loaded at
runtime: the `SOFTWARE` screen only toggles which ones the boot screen shows, and
the choice lives in the preferences. Installing shows the loading bar on purpose
— the moment is part of the fiction — and Inspiro is the system, never removable.
There is no plugin directory and no file to copy.

**An update is signed or it is nothing.** The updater trusts two minisign keys
baked into `crates/update`: the current one and a spare for rotation (a binary
only trusts the keys it was built with). It refuses a release without a
signature, a checksum or signature that does not match, a tarball without a
`cread`, and a development build. The old binary is kept as `cread.old`; the
replaced file is a sibling temp file renamed over it, never a copy across
filesystems. `--update-once` is the non-interactive path for scripts and tests.

**Sound belongs to the shell.** The music deck (`crates/app/src/audio.rs`) lives
in `App`, not in the Riff program, because playback has to outlive the screen and
because `riff ...` must work from any prompt. Riff is a disk program
(`installed: false`) whose UI is composed by the shell like the editor's. A
program that needs to keep running while another one is in front belongs here,
not in `crates/programs`.

**Vendored dependencies are patched with a reason.** `vendor/` holds a copy of a
crate we build against, wired in with `[patch.crates-io]`, plus a README listing
every change and why. Today that is `mizu-core` (Pocketboi's emulator), patched
to stop printing to stdout, to survive an illegal opcode, and to keep a corrupt
save state from panicking. Do not add a vendor copy without that README, and
re-apply the patches instead of dropping them on upgrade.

**Two surfaces, one raster.** A program draws pixels into `Raster` (16 fixed
colours, 640x368) and text into the `Screen` on top; cells can be transparent so
menus sit over a picture. The framebuffer only ever sees one composited image,
so the CRT pass stays the same for text and pictures.

**The clipboard handle must outlive the copy.** On Wayland the data is served by
the owning process; `arboard::Clipboard` is kept in `App` for exactly that
reason. Creating one per operation silently loses the text.

**The vault has three rooms.** `Documents/` for text, `Pictures/` for PNGs,
`Roms/` for cartridges; the vault root is only the shelf, and `saveas` cannot
leave its room (`..` and absolute names are folded away). `THE VAULT` on the
boot screen is a folder browser that opens each file with the program that owns
it; each program's own `open` starts in its room (`Documents/` for Inspiro,
`Pictures/` for Softhand, the music folder for Riff) and only lists what that
program can use. Loose documents from an older vault are moved into
`Documents/` on first use — never overwritten, a name clash stays put and is
reported.

**Saving is the writer's decision.** Autosave is opt-in (`autosave on`, stored in
the preferences); by default a document only reaches disk through `save`,
`saveas`, quitting or closing the window. Do not add a "convenience" write path
that fires on its own.

**Local files win.** Sync never overwrites a both-sides change: the remote copy
is parked next to the local file as `name.conflict-<unix>.<ext>` and the local
version stays. Documents are never deleted on the server. The one exception is
the editor's own unnamed draft (`untitled-N.md`): it is marked as a draft in the
sync index (`set_draft`), and when `saveas` names it, its remote copy is deleted
with the local placeholder. A draft that outlives the session is promoted back
to an ordinary document at startup, so nothing the writer typed is thrown away.

**Sync mirrors the vault, not the documents.** Everything under the vault root
goes up — `Roms`, `Pictures`, folders you make yourself — with the tree
recreated on the server; only hidden entries (the sync index lives in
`.cread-sync`) stay local. Downloads are written byte for byte
(`storage::save_bytes_atomic`), because a cartridge is not UTF-8.

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
| new dependency or vendored crate | `THIRD-PARTY.md` — MIT is the project licence, and a GPL dependency would change that |

## Release

1. Bump `version` in the workspace `Cargo.toml`.
2. Move the *Unreleased* CHANGELOG entries under `## X.Y.Z`.
3. `git tag -a vX.Y.Z -m "..."` and push both branch and tag.

The release job signs `${DIST}.tar.gz` with the key in the `MINISIGN_KEY`
secret (`rsign sign`, minisign format) and publishes the `.minisig` next to the
archive; the job fails if the secret is missing, because the updater refuses
unsigned releases. The secret key lives outside the repository; the public half
is `minisign.pub`, and the spare's public key is in `crates/update/src/lib.rs`.
Rotating keys: ship the next public key in a release *before* signing with it.

The `release` workflow builds inside `ubuntu:22.04` (so the binary needs no
glibc newer than 2.34), packages `cread-linux-x86_64.tar.gz` with a `VERSION.txt`
and a checksum, uploads it as an artifact, and attaches it to a GitHub Release
whose body is the matching CHANGELOG section. `ci` runs fmt, clippy and tests on
every push and pull request.

Pushing is done over HTTPS with the `gh` credential helper; the SSH key here is
passphrase-protected and there is usually no agent running.
