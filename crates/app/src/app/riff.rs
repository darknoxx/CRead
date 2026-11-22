//! The Riff music player's keys, the deck and the file explorer.
//!
//! Playback itself is the shell's (`audio.rs`): it has to outlive the screen.

use super::*;

impl App {
    /// Riff's own keys: space, arrows, and a few letters.
    pub(super) fn on_key_riff(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        if self.program_prompt {
            if matches!(&event.logical_key, Key::Named(NamedKey::Escape)) && self.command.is_empty()
            {
                self.program_prompt = false;
                self.poke();
                return;
            }
            self.on_key_action(el, event);
            return;
        }
        let outcome = match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.program_prompt = true;
                self.command.clear();
                self.command_error.clear();
                self.poke();
                return;
            }
            Key::Named(NamedKey::Space) => Some(self.run_riff(RiffAction::Pause)),
            Key::Named(NamedKey::ArrowLeft) => Some(self.run_riff(RiffAction::Prev)),
            Key::Named(NamedKey::ArrowRight) => Some(self.run_riff(RiffAction::Next)),
            Key::Named(NamedKey::ArrowUp) => {
                self.volume_step(5);
                None
            }
            Key::Named(NamedKey::ArrowDown) => {
                self.volume_step(-5);
                None
            }
            Key::Character(text) if text.eq_ignore_ascii_case("i") => {
                Some(self.run_riff(RiffAction::Insert))
            }
            Key::Character(text) if text.eq_ignore_ascii_case("s") => {
                Some(self.run_riff(RiffAction::Stop))
            }
            Key::Character(text) if text.eq_ignore_ascii_case("r") => {
                Some(self.run_riff(RiffAction::Random(None)))
            }
            _ => None,
        };
        match outcome {
            Some(Ok(message)) if !message.is_empty() => self.set_status(message),
            Some(Err(e)) => self.set_status(e),
            _ => {}
        }
        self.poke();
    }

    /// The insert browser: arrows, enter, backspace, i.
    pub(super) fn on_key_riff_insert(&mut self, event: KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.riff.explorer = None;
                self.close_explorer_into_player();
            }
            Key::Named(NamedKey::Enter) => self.explorer_enter(),
            Key::Named(NamedKey::ArrowUp) => self.explorer_move(-1),
            Key::Named(NamedKey::ArrowDown) => self.explorer_move(1),
            Key::Named(NamedKey::Backspace) | Key::Named(NamedKey::ArrowLeft) => {
                self.explorer_up();
            }
            Key::Character(text) if text.eq_ignore_ascii_case("i") => {
                if let Err(e) = self.explorer_insert_folder() {
                    self.set_status(e);
                }
            }
            _ => {}
        }
        self.poke();
    }

    /// The output device, opened the first time music is asked for.
    pub(super) fn audio(&mut self) -> Result<&mut crate::audio::Audio, String> {
        if self.audio.is_none() {
            self.audio = Some(crate::audio::Audio::open(self.volume)?);
        }
        self.audio
            .as_mut()
            .ok_or_else(|| "audio output is not available".to_string())
    }

    /// Start whatever the playlist points at.
    pub(super) fn riff_start_current(&mut self) -> Result<String, String> {
        let Some(track) = self.riff.playlist.current().cloned() else {
            return Err("no music inserted".into());
        };
        let volume = self.volume;
        {
            let audio = self.audio()?;
            audio.set_volume(volume);
            audio.play(&track.path)?;
        }
        self.riff.playing = true;
        Ok(format!("\u{266b} {}", track.title))
    }

    /// Fill an empty playlist from the folder or file inserted last time.
    pub(super) fn riff_ensure_loaded(&mut self) -> Result<(), String> {
        if !self.riff.playlist.is_empty() {
            return Ok(());
        }
        let last = self.prefs.riff_last.clone();
        if last.is_empty() {
            return Err("no music - open Riff and insert a folder".into());
        }
        let path = PathBuf::from(&last);
        if !path.exists() {
            return Err(format!("the inserted music is gone: {last}"));
        }
        let tracks = if path.is_dir() {
            music::tracks_in(&path)
        } else {
            vec![music::Track::from_path(path.clone())]
        };
        if tracks.is_empty() {
            return Err(format!("no music in {last}"));
        }
        self.riff.playlist = music::Playlist::load(tracks, 0);
        self.riff.playlist.set_shuffle(self.prefs.riff_random);
        self.riff.source = last;
        Ok(())
    }

    /// Put a folder or a single file into the player and start it at `index`.
    pub(super) fn riff_load(&mut self, source: &Path, index: usize) -> Result<String, String> {
        let tracks = if source.is_dir() {
            music::tracks_in(source)
        } else {
            vec![music::Track::from_path(source.to_path_buf())]
        };
        if tracks.is_empty() {
            return Err(format!("no music in {}", source.display()));
        }
        self.riff.playlist = music::Playlist::load(tracks, index);
        self.riff.playlist.set_shuffle(self.prefs.riff_random);
        self.riff.source = source.display().to_string();
        self.prefs.riff_last = self.riff.source.clone();
        self.save_prefs();
        self.riff_start_current()
    }

    pub(super) fn run_riff(&mut self, action: RiffAction) -> Result<String, String> {
        if !self.is_installed("riff") {
            return Err("Riff is not installed - open SOFTWARE".into());
        }
        match action {
            RiffAction::Insert => {
                if !self.riff_is_front() {
                    return Err("open Riff to insert a disc".into());
                }
                self.open_explorer();
                Ok(String::new())
            }
            RiffAction::Status => Ok(self.riff_status()),
            RiffAction::Volume(want) => {
                let next = want.unwrap_or(self.volume);
                self.set_volume(next);
                Ok(format!("volume {}", self.volume))
            }
            RiffAction::Random(want) => {
                let on = want.unwrap_or(!self.riff.playlist.shuffle());
                self.riff.playlist.set_shuffle(on);
                self.prefs.riff_random = on;
                self.save_prefs();
                Ok(if on {
                    "random on".into()
                } else {
                    "random off".to_string()
                })
            }
            RiffAction::Pause => {
                let Some(audio) = self.audio.as_mut() else {
                    return Err("nothing playing".into());
                };
                if audio.finished() {
                    return Err("nothing playing".into());
                }
                if audio.is_paused() {
                    audio.resume();
                    self.riff.playing = true;
                    Ok("playing".into())
                } else {
                    audio.pause();
                    self.riff.playing = false;
                    Ok("paused".into())
                }
            }
            RiffAction::Stop => {
                if let Some(audio) = self.audio.as_mut() {
                    audio.stop();
                }
                self.riff.playing = false;
                Ok("stopped".into())
            }
            RiffAction::Play(name) => {
                self.riff_ensure_loaded()?;
                if let Some(name) = name {
                    let needle = name.to_lowercase();
                    let found = self
                        .riff
                        .playlist
                        .tracks()
                        .iter()
                        .position(|track| track.title.to_lowercase().contains(&needle))
                        .ok_or_else(|| format!("no track called {name}"))?;
                    self.riff.playlist.select(found);
                    return self.riff_start_current();
                }
                let idle = self
                    .audio
                    .as_ref()
                    .map(|audio| audio.finished() || audio.is_paused())
                    .unwrap_or(true);
                if self.riff.playing && !idle {
                    return Ok("already playing".into());
                }
                self.riff_start_current()
            }
            step @ (RiffAction::Next | RiffAction::Prev) => {
                self.riff_ensure_loaded()?;
                let forward = matches!(step, RiffAction::Next);
                let moved = if forward {
                    self.riff.playlist.next()
                } else {
                    self.riff.playlist.prev()
                };
                if !moved {
                    return Err(if forward {
                        "end of the disc".into()
                    } else {
                        "start of the disc".into()
                    });
                }
                self.riff_start_current()
            }
        }
    }

    /// `♪ Morgenlicht · 00:42` while a track is loaded, for the status bars of
    /// the other screens. Riff's player shows this itself, so it is left out
    /// there.
    pub(super) fn now_playing(&self) -> Option<String> {
        if self.mode == Mode::Riff {
            return None;
        }
        let track = self.riff.playlist.current()?;
        let audio = self.audio.as_ref()?;
        if audio.finished() {
            return None;
        }
        Some(format!(
            "\u{266a} {} \u{00b7} {}",
            track.title,
            crate::audio::clock(audio.position())
        ))
    }

    pub(super) fn riff_status(&self) -> String {
        match self.riff.playlist.current() {
            Some(track) => {
                let (at, of) = self.riff.playlist.position();
                let state = if self.riff.playing {
                    "playing"
                } else {
                    "paused"
                };
                format!("\u{266b} {} ({at} of {of}) - {state}", track.title)
            }
            None => "riff: no disc inserted".into(),
        }
    }

    pub(super) fn set_volume(&mut self, volume: u8) {
        self.volume = volume.min(100);
        if let Some(audio) = self.audio.as_mut() {
            audio.set_volume(self.volume);
        }
        self.prefs.volume = self.volume;
        self.save_prefs();
        self.dirty = true;
    }

    pub(super) fn volume_step(&mut self, by: i32) {
        let next = (self.volume as i32 + by).clamp(0, 100) as u8;
        self.set_volume(next);
    }

    /// True while Riff is the thing in front.
    pub(super) fn riff_is_front(&self) -> bool {
        matches!(self.mode, Mode::Riff | Mode::RiffInsert)
            || (!self.has_document && self.home == Home::Riff && self.mode == Mode::Action)
    }

    pub(super) fn open_explorer(&mut self) {
        let mut browser = Browser::music(music::music_dir());
        if let Err(e) = browser.refresh() {
            self.set_status(e);
        }
        self.riff.explorer = Some(browser);
        self.mode = Mode::RiffInsert;
        self.program_prompt = false;
        self.dirty = true;
    }

    pub(super) fn explorer_move(&mut self, delta: isize) {
        let Some(browser) = self.riff.explorer.as_mut() else {
            return;
        };
        browser.step(delta);
        self.poke();
    }

    /// Enter in the music browser: the same accept the vault uses, on the
    /// browser the explorer owns. It swaps into the shell's slot so there is
    /// one implementation, then puts it back if the screen is still the list.
    pub(super) fn explorer_enter(&mut self) {
        let Some(browser) = self.riff.explorer.take() else {
            return;
        };
        let vault_root = self.vault();
        self.browser = browser;
        self.browser_accept();
        if self.mode == Mode::RiffInsert {
            let browser = std::mem::replace(&mut self.browser, Browser::vault(vault_root));
            self.riff.explorer = Some(browser);
        } else {
            // The track took over; do not leave the music shelf in the slot the
            // vault browser expects.
            self.browser = Browser::vault(vault_root);
        }
    }

    pub(super) fn explorer_up(&mut self) {
        if let Some(browser) = self.riff.explorer.as_mut()
            && browser.path.as_os_str().is_empty()
        {
            // At the music root: nowhere to go, and never into the home folder.
            return;
        }
        if let Some(browser) = self.riff.explorer.as_mut() {
            browser.path = browser
                .path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            browser.selected = 0;
            if let Err(e) = browser.refresh() {
                self.set_status(e);
            }
        }
        self.poke();
    }

    /// What `i` would insert: the highlighted folder if the cursor is on one,
    /// otherwise the folder being browsed.
    pub(super) fn explorer_insert_target(&self) -> Option<PathBuf> {
        let browser = self.riff.explorer.as_ref()?;
        Some(match browser.current()? {
            Action::EnterDir(relative) => browser.root.join(relative),
            _ => browser.dir(),
        })
    }

    pub(super) fn explorer_insert_folder(&mut self) -> Result<(), String> {
        let Some(dir) = self.explorer_insert_target() else {
            return Ok(());
        };
        let message = self.riff_load(&dir, 0)?;
        self.close_explorer_into_player();
        self.set_status(message);
        Ok(())
    }

    pub(super) fn close_explorer_into_player(&mut self) {
        self.riff.explorer = None;
        self.mode = Mode::Riff;
        self.program_prompt = false;
        self.dirty = true;
    }

    /// One second of music time: advance at the end of a track, redraw the
    /// counter while playing.
    pub(super) fn riff_tick(&mut self) -> bool {
        if !self.riff.playing {
            return false;
        }
        if self.audio.as_ref().is_none_or(|audio| !audio.finished()) {
            let second = self
                .audio
                .as_ref()
                .map(|audio| audio.position().as_secs())
                .unwrap_or(0);
            if second != self.riff.second {
                self.riff.second = second;
                self.dirty = true;
            }
            return true;
        }
        if self.riff.playlist.next() {
            let _ = self.riff_start_current();
        } else {
            self.riff.playing = false;
            if let Some(audio) = self.audio.as_mut() {
                audio.stop();
            }
            self.set_status("end of the disc");
        }
        true
    }
}
