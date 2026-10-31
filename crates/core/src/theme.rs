//! Colour palettes. Every theme is a 16-entry VGA-style table, so a cell only
//! stores a palette index — exactly like a real text-mode attribute byte.

/// An RGB triple.
pub type Rgb = [u8; 3];

/// A 16-colour palette plus the semantic slots the editor uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    /// The 16 VGA palette entries.
    pub palette: [Rgb; 16],
    /// Default text / background.
    pub fg: u8,
    pub bg: u8,
    /// Dimmed markup markers (`**`, `#`, `[](...)`).
    pub dim: u8,
    /// Headings, links.
    pub accent: u8,
    /// Error text in the command prompt.
    pub error: u8,
    /// Inline code background.
    pub code_bg: u8,
    /// Status line, and the dimmed text on it.
    pub status_fg: u8,
    pub status_bg: u8,
    pub status_dim: u8,
    /// Single-line top bar (used for notices / sync state).
    pub notice_fg: u8,
    pub notice_bg: u8,
}

impl Theme {
    pub fn color(&self, index: u8) -> Rgb {
        self.palette[(index & 0x0f) as usize]
    }

    pub fn background(&self) -> Rgb {
        self.color(self.bg)
    }

    /// True for schemes meant to be read in daylight (dark ink on light paper).
    ///
    /// Glow effects are emissive and look wrong on those, so the renderer dials
    /// them back instead of pretending the panel is a CRT.
    pub fn is_light(&self) -> bool {
        let [r, g, b] = self.background();
        (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) > 140.0
    }

    /// The accent, unless it would vanish on the status bar.
    ///
    /// The paper scheme puts dark ink on a pale page, but its status bar is
    /// black — a dark blue prompt sigil on it is nearly invisible, so the bar's
    /// own foreground is used instead. Every other scheme already contrasts.
    pub fn accent_on_status(&self) -> u8 {
        let luma =
            |rgb: Rgb| 0.2126 * rgb[0] as f32 + 0.7152 * rgb[1] as f32 + 0.0722 * rgb[2] as f32;
        let accent = luma(self.color(self.accent));
        let bar = luma(self.color(self.status_bg));
        if accent - bar < 60.0 {
            self.status_fg
        } else {
            self.accent
        }
    }

    /// The classic 16-colour CGA/VGA table.
    pub const CGA: [Rgb; 16] = [
        [0x00, 0x00, 0x00],
        [0x00, 0x00, 0xaa],
        [0x00, 0xaa, 0x00],
        [0x00, 0xaa, 0xaa],
        [0xaa, 0x00, 0x00],
        [0xaa, 0x00, 0xaa],
        [0xaa, 0x55, 0x00],
        [0xaa, 0xaa, 0xaa],
        [0x55, 0x55, 0x55],
        [0x55, 0x55, 0xff],
        [0x55, 0xff, 0x55],
        [0x55, 0xff, 0xff],
        [0xff, 0x55, 0x55],
        [0xff, 0x55, 0xff],
        [0xff, 0xff, 0x55],
        [0xff, 0xff, 0xff],
    ];

    /// Black background, white text: the look of a plain text terminal.
    /// This is the default.
    pub const BLACK: Theme = Theme {
        name: "black",
        palette: Theme::CGA,
        fg: 15,
        bg: 0,
        dim: 8,
        accent: 15,
        error: 12,
        code_bg: 8,
        status_fg: 15,
        status_bg: 8,
        status_dim: 7,
        notice_fg: 15,
        notice_bg: 8,
    };

    pub const DOS_BLUE: Theme = Theme {
        name: "dos-blue",
        palette: Theme::CGA,
        fg: 7,
        bg: 1,
        dim: 8,
        accent: 15,
        error: 12,
        code_bg: 0,
        status_fg: 1,
        status_bg: 7,
        status_dim: 8,
        notice_fg: 15,
        notice_bg: 1,
    };

    pub const AMBER: Theme = Theme {
        name: "amber",
        palette: monochrome([0xff, 0xb0, 0x00]),
        fg: 13,
        bg: 0,
        dim: 7,
        accent: 15,
        error: 15,
        code_bg: 1,
        status_fg: 0,
        status_bg: 13,
        status_dim: 8,
        notice_fg: 15,
        notice_bg: 1,
    };

    pub const GREEN: Theme = Theme {
        name: "green",
        palette: monochrome([0x33, 0xff, 0x66]),
        fg: 13,
        bg: 0,
        dim: 7,
        accent: 15,
        error: 15,
        code_bg: 1,
        status_fg: 0,
        status_bg: 13,
        status_dim: 8,
        notice_fg: 15,
        notice_bg: 1,
    };

    /// Daylight mode: dark text on off-white paper.
    pub const PAPER: Theme = Theme {
        name: "paper",
        palette: paper_palette(),
        fg: 0,
        bg: 15,
        dim: 7,
        accent: 1,
        error: 4,
        code_bg: 8,
        status_fg: 15,
        status_bg: 0,
        status_dim: 7,
        notice_fg: 15,
        notice_bg: 1,
    };

    /// The scheme the app starts with.
    pub const DEFAULT: Theme = Theme::PAPER;

    pub const ALL: [Theme; 5] = [
        Theme::BLACK,
        Theme::DOS_BLUE,
        Theme::AMBER,
        Theme::GREEN,
        Theme::PAPER,
    ];

    pub fn by_name(name: &str) -> Option<Theme> {
        Theme::ALL
            .into_iter()
            .find(|t| t.name == name || (name == "dos" && t.name == "dos-blue"))
    }
}

/// Build 16 brightness steps of a single phosphor colour.
const fn monochrome(tint: Rgb) -> [Rgb; 16] {
    const LEVELS: [u8; 16] = [
        0, 24, 40, 56, 72, 88, 104, 120, 136, 152, 168, 184, 200, 216, 232, 255,
    ];
    let mut out = [[0u8; 3]; 16];
    let mut i = 0;
    while i < 16 {
        let l = LEVELS[i] as u32;
        out[i] = [
            ((tint[0] as u32 * l) / 255) as u8,
            ((tint[1] as u32 * l) / 255) as u8,
            ((tint[2] as u32 * l) / 255) as u8,
        ];
        i += 1;
    }
    out
}

const fn paper_palette() -> [Rgb; 16] {
    let mut out = Theme::CGA;
    out[0] = [0x1a, 0x1a, 0x1a];
    out[1] = [0x00, 0x00, 0x99];
    out[2] = [0x00, 0x77, 0x00];
    out[3] = [0x00, 0x77, 0x77];
    out[4] = [0xaa, 0x00, 0x00];
    out[5] = [0x99, 0x00, 0x99];
    out[6] = [0xaa, 0x55, 0x00];
    out[7] = [0x77, 0x77, 0x77];
    out[8] = [0xaa, 0xaa, 0xaa];
    out[9] = [0x33, 0x33, 0xcc];
    out[10] = [0x22, 0x99, 0x22];
    out[11] = [0x22, 0x99, 0x99];
    out[12] = [0xcc, 0x22, 0x22];
    out[13] = [0xbb, 0x22, 0xbb];
    out[14] = [0x99, 0x77, 0x00];
    out[15] = [0xf4, 0xf1, 0xe8];
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_resolves_its_semantic_slots() {
        for theme in Theme::ALL {
            for slot in [
                theme.fg,
                theme.bg,
                theme.dim,
                theme.accent,
                theme.code_bg,
                theme.status_fg,
                theme.status_bg,
                theme.status_dim,
                theme.error,
            ] {
                assert!(slot < 16, "{} has out-of-range slot {slot}", theme.name);
            }
            assert_ne!(theme.fg, theme.bg, "{}", theme.name);
        }
    }

    #[test]
    fn mono_themes_are_ordered_by_brightness() {
        let lum = |c: Rgb| c[0] as u32 + c[1] as u32 + c[2] as u32;
        for theme in [Theme::AMBER, Theme::GREEN] {
            for i in 1..16 {
                assert!(lum(theme.palette[i]) >= lum(theme.palette[i - 1]));
            }
        }
    }

    #[test]
    fn lookup_by_name() {
        assert_eq!(Theme::by_name("dos"), Some(Theme::DOS_BLUE));
        assert_eq!(Theme::by_name("black"), Some(Theme::BLACK));
        assert_eq!(Theme::BLACK.fg, 15, "body text is pure white");
        assert_eq!(Theme::ALL[0], Theme::BLACK, "black is the default");
        assert_eq!(Theme::by_name("amber"), Some(Theme::AMBER));
        assert!(Theme::by_name("nope").is_none());
    }
}
