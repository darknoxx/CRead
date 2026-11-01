//! Commands about the screen: themes, the CRT, the face, the window.

use super::super::*;

impl App {
    pub(super) fn cmd_theme(&mut self, argument: Option<String>) -> Result<String, String> {
        match argument.as_deref() {
            None | Some("+") => self.cycle_theme(1),
            Some("-") => self.cycle_theme(-1),
            Some(name) => {
                let theme = Theme::by_name(name).ok_or_else(|| {
                    let names: Vec<&str> = Theme::ALL.iter().map(|t| t.name).collect();
                    format!("unknown theme: {name} ({})", names.join(", "))
                })?;
                self.theme = theme;
                self.save_prefs();
                self.dirty = true;
                Ok(format!("theme: {}", theme.name))
            }
        }
    }

    pub(super) fn cmd_scanlines(&mut self, on: Option<bool>, roll: bool) -> Result<String, String> {
        // Bare `scanlines` switches them on rather than toggling: it is the
        // everyday "give me the CRT" gesture.
        let enable = on.unwrap_or(true) || roll;
        self.crt.scanline = if enable {
            CrtSettings::default().scanline
        } else {
            0.0
        };
        if roll {
            self.crt.roll = ROLL_SPEED;
        }
        self.save_prefs();
        self.dirty = true;
        Ok(format!(
            "scanlines: {}{}",
            on_off(enable),
            if roll { ", roll on" } else { "" }
        ))
    }

    pub(super) fn cmd_bloom(&mut self, want: Option<bool>) -> Result<String, String> {
        let on = apply_switch(&mut self.crt.bloom, CrtSettings::default().bloom, want);
        self.save_prefs();
        self.dirty = true;
        Ok(format!("bloom: {}", on_off(on)))
    }

    pub(super) fn cmd_roll(&mut self, want: Option<bool>) -> Result<String, String> {
        let on = want.unwrap_or(self.crt.roll <= 0.0);
        self.crt.roll = if on { ROLL_SPEED } else { 0.0 };
        if on && self.crt.scanline <= 0.0 {
            // Drifting with no lines to drift is a no-op; turn them on.
            self.crt.scanline = CrtSettings::default().scanline;
        }
        self.save_prefs();
        self.dirty = true;
        Ok(format!("scanline roll: {}", on_off(on)))
    }

    pub(super) fn cmd_autosave(&mut self, want: Option<bool>) -> Result<String, String> {
        self.autosave = want.unwrap_or(!self.autosave);
        self.prefs.autosave = self.autosave;
        self.save_prefs();
        Ok(if self.autosave {
            "autosave on - the document is written as you type".into()
        } else {
            "autosave off - save is yours to run".to_string()
        })
    }

    pub(super) fn cmd_beep(&mut self, want: Option<bool>) -> Result<String, String> {
        self.beep_on = want.unwrap_or(!self.beep_on);
        self.prefs.beep = self.beep_on;
        self.save_prefs();
        self.beep();
        Ok(format!("beep: {}", on_off(self.beep_on)))
    }

    pub(super) fn cmd_typewriter(&mut self, want: Option<bool>) -> Result<String, String> {
        self.typewriter = want.unwrap_or(!self.typewriter);
        self.save_prefs();
        self.dirty = true;
        Ok(format!("typewriter: {}", on_off(self.typewriter)))
    }

    pub(super) fn cmd_font(&mut self, want: Option<String>) -> Result<String, String> {
        let next = match want.as_deref() {
            Some(name) => Face::by_name(name)
                .ok_or_else(|| format!("unknown font: {name} (classic, thin)"))?,
            None => match self.font.face() {
                Face::Classic => Face::Thin,
                Face::Thin => Face::Classic,
            },
        };
        self.font = Font::with_face(next);
        self.prefs.font = next.name().to_string();
        self.save_prefs();
        // The cell height feeds the scale heuristic and the grid size.
        self.recompute_viewport();
        self.dirty = true;
        Ok(format!("font: {}", next.name()))
    }

    pub(super) fn cmd_fullscreen(&mut self, on: bool) -> Result<String, String> {
        self.set_fullscreen(on);
        Ok(if on { "fullscreen" } else { "windowed" }.to_string())
    }
}
