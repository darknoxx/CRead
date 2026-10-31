# Vendored: mizu-core 1.3.0

Pocketboi runs on [Mizu](https://github.com/Amjad50/mizu), a Game Boy and Game
Boy Color emulator written in Rust (MIT). The crate is vendored here instead of
pulled from crates.io because it needed five changes before it could live inside
a desktop application. Each one is small; all of them are in the vendored copy,
so an upgrade means re-applying them.

Why vendoring at all: `[patch.crates-io]` in the workspace `Cargo.toml` points
`mizu-core` here. Cargo will not build it from the registry, and a yanked or
deleted release cannot break the build.

## The changes

**`src/cpu.rs` — `LD B,B` is an instruction, not a breakpoint.**
Upstream treated opcode `0x40` as a debugger breakpoint that printed a line on
every execution and set a `Breakpoint` state. It is a normal one-byte no-op that
real games execute; it now returns `Ok(0)` like any other no-op.

**`src/lib.rs` — an illegal opcode stops the frame, not the process.**
`clock_for_frame` unwrapped `Cpu::next_instruction`, which returns
`Err(IllegalInstruction)` for unknown opcodes: a malformed cartridge could panic
the host. It now returns `Result<(), RunError>`, and Pocketboi turns the error
into a stopped game with a message.

**`src/lib.rs` — no `assert!` while loading a save state.**
A backwards stream position (`overflowing_sub` followed by `assert!`) is now a
saturating subtraction: a corrupt save file should not panic.

**`src/ppu.rs`, `src/cartridge.rs` — stdout stays quiet.**
Several `println!` calls fired during normal play (turning the display off
outside VBLANK happens in real games, plus "LOG:", "Loading SRAM", "target
gameboy"). They are gone; the two genuine warnings now go to stderr.

## License

Mizu is MIT; `LICENSE` is copied verbatim from the crate. The patches above are
ours, under the same MIT terms as the rest of CRead.
