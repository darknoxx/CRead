# CLAUDE.md — CRead

CRead is a fullscreen writing terminal that looks like a 1987 DOS machine: one
Linux binary, an IBM VGA bitmap font, an emulated raster and a CRT pass.
Documents are plain Markdown in an XDG vault and sync over WebDAV.

This file is the onboarding map for Claude. **Read [`AGENTS.md`](AGENTS.md)
before changing architecture** — it is the binding list of invariants, written
for anyone (human or agent) changing the repository. [`README.md`](README.md)
is the user-facing overview, `crates/app/src/manual.md` is the in-app manual,
and [`CHANGELOG.md`](CHANGELOG.md) is the release history.

## Current state (check before trusting)

- Version: **0.33.0** (`Cargo.toml`, `ui::VERSION`).
- `main` is the only branch. The old `experimental/*` branches were fully
  merged into `main` and deleted on 2026-11-21.
- The latest release is `v0.33.0`; tags and release assets are on GitHub.
- Run `git status`, `git log -1` and `git describe --tags` before a task that
  depends on the current state.

## First 10 minutes

The shipped target is Linux. Rust stable, edition 2024, `rust-version = 1.85`.

Debian/Ubuntu build dependencies (the same list as CI):

```sh
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  build-essential pkg-config \
  libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev wayland-protocols \
  libasound2-dev libudev-dev libssl-dev shellcheck
```

Then:

```sh
cargo build
cargo test --workspace
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
shellcheck packaging/*.sh
```

`cargo run` needs the binary crate:

```sh
cargo run -p cread -- --demo -w
```

Do not launch the fullscreen app on the user's desktop unless asked. Use the
windowed demo or the screenshot path.

### Visual checks without taking over the screen

```sh
cargo build -p cread
target/debug/cread --demo -w --size 1280x800 --screenshot /tmp/cread.png
target/debug/cread --show welcome|action|search|vault|settings|help \
  --screenshot /tmp/cread.png
target/debug/cread --demo -w --size 1280x800 --show action \
  --command "sca" --screenshot /tmp/cread.png
```

`--command` prefills the prompt; it is the only way to screenshot a typed
command state. More dev views: `webdav`, `manual`, `notice`, `quit`,
`shutdown`, `reboot`, `backtolinux`, `software`, `pictures`, `riffplay`,
`riffinsert`, `pocketboi[:<rom>]`, `poker`, `pokerplay`, `pokerhand`,
`pokerrisk`, `pokerover`, `calendar`, `calendaredit`, `reminder`,
`picture[:<png>]`. See `cread --help` for the exact list.

## Architecture map

| Path | Owns | Rule |
| --- | --- | --- |
| `crates/core` | buffer, editor, cursor, wrap, undo, markdown spans, command parsing, themes, prefs, storage, calendar, music, picture, raster/screen, resolution, `Program` trait | **no GPU, no window, no network** — this is what keeps the hard parts unit-testable |
| `crates/render` | bitmap font, CPU compositor, wgpu pipeline, CRT shader | everything that touches the GPU |
| `crates/sync` | WebDAV client, conflict policy, background worker | blocking calls only, never the UI thread |
| `crates/sound` | which audio output device the machine opens | policy shared by the shell and programs |
| `crates/update` | GitHub release check, download, checksum + minisign, atomic replacement | never install unsigned or unverified bytes |
| `crates/app` | winit loop, input, mode machine, cell-grid composition, screens | glue; keep logic in core when it can live there |
| `crates/programs/*` | one crate per module (`softhand`, `pocketboi`, `poker`, `scripture`) | a program owns its surface and gestures, never the prompt or files |
| `packaging/` | kiosk unit, polkit rule, bootstrap, install and update scripts | the Pi recipe; nothing here is compiled into the binary |
| `docs/` | Raspberry Pi recipe | setup, not user documentation |
| `vendor/mizu-core` | patched Game Boy emulator | re-apply the patches on upgrade; see `vendor/README.md` |

Key files:

- `crates/app/src/app.rs` — `App`, `Mode`, `Home`, `PROGRAMS`, input, frame loop.
- `crates/app/src/app/compose.rs` — draws the cell grid and the raster layer.
- `crates/app/src/app/commands/*` — command implementations.
- `crates/app/src/ui.rs` — shared shell UI: top bar, prompts, scrollbar, dialogs.
- `crates/app/src/manual.md` — in-app manual, embedded with `include_str!`.
- `crates/app/src/welcome.md` — built-in welcome document.
- `crates/core/src/command.rs` — `Command`, `SPECS`, parser.
- `crates/core/src/storage.rs` — vault rooms and atomic file IO.
- `crates/core/src/prefs.rs` — persisted preferences.
- `crates/core/src/markdown.rs` — markup stripping for view mode.
- `crates/core/src/resolution.rs` — integer `Viewport`.
- `crates/core/src/program.rs` — `Program` trait.
- `crates/render/src/raster.rs` — `Framebuffer` and cell blitting.
- `crates/render/src/gpu.rs` — wgpu setup and adapter limits.
- `crates/render/src/crt.wgsl` — CRT shader and uniform layout.
- `AGENTS.md` — the binding invariants. Read it.

## Non-negotiables

The full list is in `AGENTS.md`; these are the ones most likely to be broken:

1. **Never scale by a fractional factor.** `Viewport` picks an integer `scale`,
   renders at `physical / scale`, and upscales by exactly that. No
   "fit to window" mode.
2. **Text is drawn at emulated resolution.** The compositor writes glyph bits
   into the framebuffer; the GPU only adds scanlines, bloom and a vignette.
3. **Nothing animates by accident.** There is no general frame clock. The shell
   clocks are scanline drift, the fake loading bar, the launch bar (`boot_at`)
   and the music clock; a program may ask for frames through
   `Program::wants_frames`. The fastest clock wins in `frame()`.
4. **`dirty = true` alone does not draw.** Input handlers must end in `poke()`
   (or rely on the choke point at the end of `App::on_key`).
5. **Bitmaps only.** Two faces (`Classic`, `Thin`); no anti-aliasing, no TTF,
   no sub-pixel positioning.
6. **Programs are compiled in.** A module is a crate in `crates/programs` plus
   an entry in `PROGRAMS` in `app.rs`. No runtime plugins, no plugin directory.
7. **One top bar, one clock.** `ui::draw_top_bar` owns row zero; `crate::clock`
   is the only place that formats date and time.
8. **The manual is embedded.** Edit `crates/app/src/manual.md`; it is not a
   vault document and never syncs.
9. **The vault has five rooms:** `Documents/`, `Pictures/`, `Roms/`,
   `Calendar/`, `Notes/`. `saveas` cannot leave its room. Loose documents from
   an older vault are moved into `Documents/`, never overwritten.
10. **Saving is the writer's decision.** Autosave is opt-in; do not add a
    convenience write path that fires on its own.
11. **Local files win sync.** A both-sides change parks the remote copy as
    `name.conflict-<unix>.<ext>`; calendar deletions are tombstones. Sync
    mirrors the vault; the hidden `.cread-sync` index stays local.
12. **An update is signed or it is nothing.** Never install unverified bytes.
    Keep `minisign.pub`, the keys in `crates/update` and the release workflow
    in sync.
13. **A kiosk update is a session restart.** `--kiosk` saves and exits; systemd
    restarts cage and CRead. Restart with the captured `self_exe`, never
    `current_exe()`.
14. **Sound and clipboard belong to the shell.** `audio.rs` and the
    `arboard::Clipboard` handle live in `App`; programs do not own them.
15. **No `unsafe`.** Keep it that way.
16. **No async in the UI thread, no web view.** Background work goes through
    worker threads and channels. CRead is one Linux binary, not a browser host.
17. **Device limits come from the adapter.** `gpu.rs` passes
    `adapter.limits()`; do not go back to `DeviceDescriptor::default()`.
18. **The CRT uniform layout must match WGSL byte for byte.** `CrtUniforms` is
    48 bytes and has a `const` assertion; a mismatch does not error, it renders
    wrongly.
19. **The logo blit happens after the cell grid.** Overlays hide the splash and
    clear the page behind them, or old art peeks through.
20. **Scrollable screens carry a scrollbar.** Use `ui::draw_scrollbar` with
    `first`/`visible`/`total`.
21. **Vendored dependencies are patched with a reason.** `vendor/README.md`
    lists every change; re-apply them on upgrade.

## How to add a command

Everything the user can do is a row in `cread_core::command::SPECS`:

1. Add a `Command` variant in `crates/core/src/command.rs`.
2. Add a `Spec` to `SPECS` with name, aliases, argument, summary and scopes.
3. Add the parse arm.
4. Add the `run_command` arm in `crates/app/src/app/commands/` (document,
   program, screen or system).
5. `help` and Tab completion read from `SPECS` automatically.
6. Update the command table in `README.md`, `crates/app/src/welcome.md` if it
   belongs to the first-run story, and `CHANGELOG.md`.
7. Add a parser test in `command.rs` and, if the behaviour is testable without
   a window, a core or app test.

## How to add a module

1. New crate under `crates/programs/<id>` with a `Cargo.toml` and `src/lib.rs`.
2. Add it to `members` in the workspace `Cargo.toml` and as a dependency in
   `crates/app/Cargo.toml`.
3. Implement `cread_core::Program` (`crates/core/src/program.rs`): `title`,
   `key`, `pointer`, `draw`, optional `command`, `status`, `palette`, `chrome`,
   `key_legend`, `wants_frames`.
4. Add a `ProgramInfo` to `PROGRAMS` in `crates/app/src/app.rs`, a
   `program_home` match, and the front-door/open path; add it to
   `software_rows`.
5. Programs draw on the classic 80x25 stage; the shell centres it. Use
   `Raster` (640x368, 16 fixed colours) for pictures and `Screen` for text.
6. Add a logo under `assets/` if it needs a splash; follow the existing
   `*-source.png` convention.
7. Add tests inside the crate; visual-check with `--show <id>` or a dev view.
8. Update `README.md`, `crates/app/src/manual.md`, `welcome.md` where relevant,
   and `CHANGELOG.md`.

`riff` is a special case: its playback lives in the shell
(`crates/app/src/audio.rs`) because it must outlive the screen. Do not move it
into a program.

## Testing

- Pure logic belongs in `core`, with a test: wrap, `find_all`, grapheme
  cursors, undo coalescing, command parsing, theme slots, XDG resolution,
  conflict matrix, percent-encoding.
- `render` tests cover font extraction, style synthesis, the compositor and the
  splash blitter.
- `app` tests exercise the cell grid: layout, clipping, palette box, selection
  and search highlighting. They do not need a window.
- Visual regressions: take a screenshot and look at it. Several real bugs were
  only caught that way.
- Before committing:
  `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --workspace`.
- CI also runs `shellcheck packaging/*.sh` and the tests natively on aarch64.

## Release

Only when the user explicitly asks. The flow:

1. Bump `version` in the workspace `Cargo.toml`.
2. Move the *Unreleased* CHANGELOG entries under `## X.Y.Z`.
3. `git tag -a vX.Y.Z -m "..."`; push branch and tag.
4. CI builds in `ubuntu:22.04` for x86_64 and natively on aarch64, packages,
   signs with the `MINISIGN_KEY` secret, and publishes the release with the
   matching CHANGELOG section as the body.
5. The updater trusts the current and spare minisign keys baked into
   `crates/update`; a release that is not signed is refused.

Do not tag, push, delete releases, change the license or rotate keys without
explicit instruction.

## Working agreement for Claude

- Read `AGENTS.md` before architectural changes; treat it as binding.
- Prefer the smallest change that fits the existing structure. Do not rewrite
  unrelated code or reformat the whole tree.
- Keep user-facing strings English; comments and commit messages English too.
  The only German in the repository is the user's own documents.
- Errors in the UI go through `set_status` (transient, 4 s), not stderr.
- Use `--vault /tmp/...` for tests; never touch the real vault or the user's
  documents.
- Do not run the fullscreen app on the user's desktop; use `-w`, `--demo` or
  `--screenshot`.
- Do not `cargo clean`, delete `target/`, or commit build artifacts,
  screenshots, vault data, cartridges, disc images or secrets.
- Do not add a dependency without checking its licence against
  `THIRD-PARTY.md`; a GPL dependency linked into the binary would make the MIT
  distribution impossible.
- Do not add `unsafe`, async in the UI thread, a web view, a runtime plugin
  loader, or a fractional scale mode.
- Work on `main` unless the user names a branch.
- Before committing: fmt, clippy, tests. Before a release: the full CI list.
- If a command or module changes, update the README table, the in-app manual,
  `welcome.md` where relevant, and `CHANGELOG.md`.
- Ask before: version bumps, tags, pushes, release deletion, license changes,
  destructive file operations, or changing a documented invariant.

## Where to read more

- [`AGENTS.md`](AGENTS.md) — invariants, architecture rules, release details.
- [`README.md`](README.md) — user-facing overview, install, flags, platform.
- [`crates/app/src/manual.md`](crates/app/src/manual.md) — the in-app manual.
- [`docs/raspberry-pi.md`](docs/raspberry-pi.md) — kiosk setup and recovery.
- [`CHANGELOG.md`](CHANGELOG.md) — what changed, release by release.
- [`THIRD-PARTY.md`](THIRD-PARTY.md) — fonts, Bible text, vendored code,
  dependency licences.
- [`vendor/README.md`](vendor/README.md) — every patch to `mizu-core`.
- [`packaging/`](packaging/) — install and update scripts; read before changing
  kiosk behaviour.
