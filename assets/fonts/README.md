# Bitmap fonts

## `ibm_vga_8x16.bin`

The IBM VGA 8x16 text-mode character set, as raw glyph data: 256 glyphs in CP437
order, 16 bytes each, one byte per scanline, MSB leftmost.

It was extracted from the original ROM dump distributed with VileR's
*The Ultimate Oldschool PC Font Pack v2.2*, specifically
`Bm437_IBM_VGA_8x16.FON` (`MZ`/`NE` container → `RT_FONT` resource → FNT header →
bitmap block). The 4 KB asset here is that bitmap block verbatim.

- Source: <https://int10h.org/oldschool-pc-fonts/>
- Licence: CC BY-SA 4.0 (© VileR)

The pack is only needed to regenerate the asset. Nothing at build or run time
depends on it.

## `ibm_dos_iso8.bin`

The ISO 9241 variant from the same pack (`Bm437_IBM_DOS_ISO8.FON`), extracted the
same way. Same 8x16 cell, same CP437 layout, but drawn with thinner strokes —
22.7 inked pixels per glyph against 33 for the VGA face. It is what `font thin`
switches to.

## Extended glyphs

CP437 has no `€`, no typographic quotes and no en/em dashes, which normal prose
needs. Those nine glyphs are hand-drawn directly in
`crates/render/src/font.rs` as ASCII art, on the same 8x16 grid.
