//! The WebDAV form: the one screen that only exists to fill in a server.
//!
//! It is a form, not a menu: the rows are fields, `Tab` walks them, `Enter`
//! saves. Reached from SETTINGS or straight from `webdav` / `settings`.

use super::*;

/// Editable copy of the WebDAV settings, shown as a form.
pub(super) struct FormState {
    url: String,
    username: String,
    password: String,
    remote_dir: String,
    enabled: bool,
    pub(super) focus: usize,
    pub(super) note: String,
}

impl FormState {
    pub(super) fn from_config(cfg: &SyncConfig) -> Self {
        Self {
            url: cfg.url.clone(),
            username: cfg.username.clone(),
            password: cfg.password.clone(),
            remote_dir: cfg.remote_dir.clone(),
            enabled: cfg.enabled,
            focus: 0,
            note: String::new(),
        }
    }

    fn to_config(&self) -> SyncConfig {
        SyncConfig {
            url: self.url.trim().to_string(),
            username: self.username.trim().to_string(),
            password: self.password.clone(),
            remote_dir: self.remote_dir.trim().trim_matches('/').to_string(),
            enabled: self.enabled,
        }
    }

    pub(super) fn field_count() -> usize {
        5
    }

    pub(super) fn fields(&self) -> Vec<ui::FormField> {
        vec![
            ui::FormField {
                label: "server".into(),
                value: self.url.clone(),
                secret: false,
            },
            ui::FormField {
                label: "user".into(),
                value: self.username.clone(),
                secret: false,
            },
            ui::FormField {
                label: "password".into(),
                value: self.password.clone(),
                secret: true,
            },
            ui::FormField {
                label: "folder".into(),
                value: self.remote_dir.clone(),
                secret: false,
            },
            ui::FormField {
                label: "sync".into(),
                value: if self.enabled {
                    "on".into()
                } else {
                    "off".into()
                },
                secret: false,
            },
        ]
    }

    fn text_field_mut(&mut self) -> Option<&mut String> {
        match self.focus {
            0 => Some(&mut self.url),
            1 => Some(&mut self.username),
            2 => Some(&mut self.password),
            3 => Some(&mut self.remote_dir),
            _ => None,
        }
    }
}

impl App {
    pub(super) fn open_webdav(&mut self) {
        self.form = FormState::from_config(&self.sync_cfg);
        self.form.note = format!("config: {}", SyncConfig::path().display());
        self.mode = Mode::Webdav;
        self.dirty = true;
    }

    /// The switch applies at once and says so; a form that only pretends to
    /// be on would be a lie.
    pub(super) fn toggle_sync(&mut self) {
        let wanted = !self.form.enabled;
        self.form.enabled = wanted;
        match self.persist_webdav() {
            Ok(()) => self.open_notice(if wanted { "SYNC ON" } else { "SYNC OFF" }, Mode::Webdav),
            Err(e) => {
                self.form.enabled = !wanted;
                self.form.note = e;
                self.dirty = true;
            }
        }
    }

    /// Write the form to disk and hand it to the worker, without leaving.
    fn persist_webdav(&mut self) -> Result<(), String> {
        let cfg = self.form.to_config();
        if cfg.enabled && !cfg.is_configured() {
            return Err("a server URL is required".into());
        }
        if self.persist {
            cfg.save()
                .map_err(|e| format!("cannot write config: {e}"))?;
        }
        self.sync_cfg = cfg.clone();
        if cfg.enabled {
            self.start_worker();
        }
        if let Some(worker) = self.sync.as_ref() {
            worker.configure(cfg);
        }
        Ok(())
    }

    pub(super) fn save_webdav(&mut self) {
        match self.persist_webdav() {
            Ok(()) => {
                self.to_write();
                self.set_status(if self.sync_cfg.enabled {
                    "sync on"
                } else {
                    "sync off"
                });
            }
            Err(e) => self.form.note = e,
        }
        self.dirty = true;
    }

    pub(super) fn on_key_webdav(&mut self, event: KeyEvent) {
        let ctrl = self.mods.control_key() || self.mods.super_key();
        match &event.logical_key {
            _ if ctrl
                && matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("s")) =>
            {
                self.save_webdav()
            }
            Key::Named(NamedKey::Escape) => self.to_write(),
            Key::Named(NamedKey::Enter) => {
                if self.form.focus == 4 {
                    self.toggle_sync();
                } else {
                    self.save_webdav();
                }
            }
            Key::Named(NamedKey::Tab) | Key::Named(NamedKey::ArrowDown) => {
                self.form.focus = (self.form.focus + 1) % FormState::field_count();
                self.poke();
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.form.focus =
                    (self.form.focus + FormState::field_count() - 1) % FormState::field_count();
                self.poke();
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some(field) = self.form.text_field_mut() {
                    field.pop();
                }
                self.poke();
            }
            Key::Named(NamedKey::Space) => {
                if let Some(field) = self.form.text_field_mut() {
                    field.push(' ');
                }
                self.poke();
            }
            Key::Character(text) => {
                if let Some(field) = self.form.text_field_mut() {
                    for ch in text.chars().filter(|c| !c.is_control()) {
                        field.push(ch);
                    }
                }
                self.poke();
            }
            _ => {}
        }
    }
}
