# Third-party notices

CRead itself is MIT; see [`LICENSE`](LICENSE). The pixel art in `assets/`
(splash, Inspiro, Softhand and Riff logos) ships under the same terms.

Three things in this repository are not CRead's own work.

## mizu-core 1.3.0 — MIT, vendored and patched

Pocketboi runs [Mizu](https://github.com/Amjad50/mizu), a Game Boy and Game Boy
Color emulator in Rust. The crate is vendored under `vendor/mizu-core` so the
build does not depend on a registry release; `vendor/mizu-core/LICENSE` is the
upstream MIT text, kept verbatim.

The five patches applied on top are CRead's and are under the same MIT terms;
[`vendor/README.md`](vendor/README.md) lists each one and why it exists.

## Bitmap fonts — CC BY-SA 4.0

`assets/fonts/ibm_vga_8x16.bin` and `assets/fonts/ibm_dos_iso8.bin` are glyph
data extracted from VileR's *The Ultimate Oldschool PC Font Pack v2.2*
(<https://int10h.org/oldschool-pc-fonts/>), licensed CC BY-SA 4.0.

The share-alike applies to those font files, not to the program that renders
them. [`assets/fonts/README.md`](assets/fonts/README.md) records the source and
the extraction step.

## crates.io dependencies

Everything else comes from crates.io, unmodified. `Cargo.lock` and each crate's
`Cargo.toml` are the authoritative list of versions and licences.

The direct dependencies are permissively licensed:

| Crate | Licence |
| --- | --- |
| `arboard` | MIT OR Apache-2.0 |
| `bytemuck` | Zlib OR Apache-2.0 OR MIT |
| `gilrs` | Apache-2.0 OR MIT |
| `png` | MIT OR Apache-2.0 |
| `pollster` | Apache-2.0 OR MIT |
| `reqwest_dav` | MIT OR Apache-2.0 |
| `rodio` | MIT OR Apache-2.0 |
| `serde`, `serde_json` | MIT OR Apache-2.0 |
| `sha2` | MIT OR Apache-2.0 |
| `tokio` | MIT |
| `unicode-segmentation` | MIT OR Apache-2.0 |
| `wgpu` | MIT OR Apache-2.0 |
| `winit` | Apache-2.0 |

One transitive dependency is under the Mozilla Public License 2.0:
**symphonia** (the audio decoders behind `rodio`). MPL-2.0 is file-level
copyleft: it asks that modifications to symphonia's own files stay under MPL
and that its source be available. CRead uses it unmodified; the source is at
<https://crates.io/crates/symphonia/0.5.5>.

TLS comes from `rustls`/`aws-lc-rs` (ISC, Apache-2.0, MIT, BSD-3-Clause), which
ship their own notices in their crates.
