//! Drawing one frame.
//!
//! Four layers, in order: a whole-frame screen that owns the grid (a menu, the
//! loading bar, help), the main screen under it, the prompt line that belongs to
//! the document, and an overlay on top. Separate functions, so a screen can be
//! read without scrolling past the other eleven.

use super::*;

impl App {
    /// Compose the next frame into `self.screen`.
    pub(super) fn compose(&mut self) {
        let mut screen = std::mem::replace(&mut self.screen, Screen::new(1, 1));
        // Only the screen composed below may blit a logo. Clearing both here is
        // what keeps the Inspiro artwork from floating over the help, the
        // browser or the boot screen.
        self.splash_at = None;
        self.inspiro_at = None;
        self.softhand_at = None;
        self.riff_at = None;

        if !self.standalone_screen(&mut screen) {
            if self.dialog_open() {
                // A dialog stands on its own page: whatever was behind it is
                // not drawn at all, so no boot row or line of text peeks out
                // around the frame.
                screen.fill(cread_core::Cell::blank(self.theme.fg, self.theme.bg));
            } else {
                let (hint, error) = self.prompt_hint();
                self.main_screen(&mut screen, &hint, &error);
            }
            self.overlay(&mut screen);
        }
        self.screen = screen;
    }

    /// Windows that replace the screen instead of floating over it.
    fn dialog_open(&self) -> bool {
        matches!(
            self.mode,
            Mode::Browser | Mode::Webdav | Mode::Quit | Mode::UpdatePrompt | Mode::Notice
        )
    }

    /// Screens that own the whole grid and return before the rest is drawn.
    /// True means "this frame is done".
    fn standalone_screen(&mut self, screen: &mut Screen) -> bool {
        if self.mode == Mode::Screensaver {
            // Drawn into a full-screen raster, not the cell grid: an analog
            // clock needs pixels. `frame` blits it with a black page.
            let (width, height) = (self.fb.width(), self.fb.height());
            if self.screensaver_raster.width() != width
                || self.screensaver_raster.height() != height
            {
                self.screensaver_raster = Raster::new(width, height);
            }
            crate::screensaver::draw_now(&mut self.screensaver_raster);
            return true;
        }
        if self.mode == Mode::Manual {
            let (width, _) = ui::layout(screen.cols());
            if self.manual_width != width {
                let lines: Vec<String> = MANUAL.lines().map(str::to_string).collect();
                self.manual_rows = cread_core::markdown::render(&lines, width);
                self.manual_width = width;
            }
            let now_local = chrono::Local::now();
            let clock = crate::clock::stamp(&now_local, self.clock_24h, self.date_day_first, false);
            let view = ui::ManualView {
                clock: &clock,
                rows: &self.manual_rows,
                top: self.manual_top,
            };
            ui::draw_manual(screen, &self.theme, &view);
            return true;
        }
        if self.mode == Mode::Software {
            let rows = self.software_rows();
            ui::draw_menu(
                screen,
                &self.theme,
                "SOFTWARE",
                "install and remove modules",
                &rows,
                self.software_selected,
            );
            return true;
        }
        if self.mode == Mode::Setup {
            let rows = self.setup_rows();
            ui::draw_menu(
                screen,
                &self.theme,
                "SETTINGS",
                "",
                &rows,
                self.setup_selected,
            );
            return true;
        }
        if self.mode == Mode::RiffInsert {
            if let Some(browser) = self.riff.explorer.as_ref() {
                let entries: Vec<(String, bool)> = browser
                    .entries
                    .iter()
                    .map(|(name, action)| (name.clone(), browser::is_folder(action)))
                    .collect();
                let dir = browser.dir().display().to_string();
                let view = ui::ExplorerView {
                    dir: &dir,
                    entries: &entries,
                    selected: browser.selected,
                };
                ui::draw_explorer(screen, &self.theme, &view);
            }
            return true;
        }
        if self.mode == Mode::Riff {
            let (title, album) = match self.riff.playlist.current() {
                Some(track) => (track.title.clone(), track.album.clone()),
                None => (String::new(), String::new()),
            };
            let (position, length) = self
                .audio
                .as_ref()
                .map(|audio| (audio.position(), audio.length()))
                .unwrap_or_default();
            let paused = self.audio.as_ref().is_some_and(|audio| audio.is_paused());
            let (at, of) = self.riff.playlist.position();
            let counter = if of == 0 {
                String::new()
            } else {
                format!("{at} of {of}")
            };
            let current = self.riff.playlist.current_index();
            let tracks = self.riff.playlist.tracks();
            let start = current.unwrap_or(0).saturating_sub(3);
            let lines: Vec<(String, bool)> = tracks
                .iter()
                .enumerate()
                .skip(start)
                .take(8)
                .map(|(index, track)| (track.title.clone(), Some(index) == current))
                .collect();
            let note = if title.is_empty() {
                "no disc inserted - press i to insert music"
            } else {
                ""
            };
            let elapsed = crate::audio::clock(position);
            let total = crate::audio::clock(length);
            let view = ui::RiffView {
                title: &title,
                album: &album,
                progress: if length.is_zero() {
                    0.0
                } else {
                    position.as_secs_f32() / length.as_secs_f32()
                },
                elapsed: &elapsed,
                total: &total,
                counter: &counter,
                playing: self.riff.playing,
                paused,
                volume: self.volume,
                lines: &lines,
                note,
            };
            ui::draw_riff(screen, &self.theme, &view);

            // Same action line as every other module: the title on the left,
            // the hints on the right, or the prompt while it is open.
            let keys = if self.riff.playlist.shuffle() {
                "spc pause   \u{2190}\u{2192} track   i insert   r random on   Esc commands"
            } else {
                "spc pause   \u{2190}\u{2192} track   i insert   r random   Esc commands"
            };
            let command_hint = if self.command.is_empty() {
                "riff play   riff stop   riff next   riff volume".to_string()
            } else {
                command::complete(&self.command)
                    .map(|c| c.hint)
                    .unwrap_or_default()
            };
            let prompt = self.program_prompt.then_some((
                self.command.as_str(),
                command_hint.as_str(),
                self.command_error.as_str(),
            ));
            ui::draw_program_line(screen, &self.theme, "RIFF", keys, None, prompt);
            return true;
        }
        if let Some(started) = self.boot_at {
            let progress = boot_progress(started.elapsed().as_secs_f32());
            let blink = ((started.elapsed().as_secs_f32() * 2.5) as u32).is_multiple_of(2);
            ui::draw_loading(
                screen,
                &self.theme,
                BOOT_LABEL,
                progress,
                blink,
                "any key to skip",
            );
            return true;
        }
        if self.mode == Mode::Loading {
            let elapsed = self.loading_started.elapsed().as_secs_f32();
            // The meter is full before the wait ends, so the last thing you see
            // is a complete bar rather than a stalled one.
            let seconds = match &self.pending {
                Some(Pending::Software { .. }) => SOFTWARE_SECONDS,
                _ => LOADING_SECONDS,
            };
            let progress = (elapsed / (seconds * 0.78)).powf(0.8).clamp(0.0, 1.0);
            let label = match &self.pending {
                Some(Pending::Inspiro) => "Loading INSPIRO...".to_string(),
                Some(Pending::Program(index)) => format!(
                    "Loading {}...",
                    PROGRAMS.get(*index).map(|p| p.entry).unwrap_or("program")
                ),
                Some(Pending::NewPicture) => "New picture...".to_string(),
                Some(Pending::Picture(_)) => "Loading picture...".to_string(),
                Some(Pending::Rom(_)) => "Loading cartridge...".to_string(),
                Some(Pending::Document(_)) => "Loading document...".to_string(),
                Some(Pending::NewDocument) => "New document...".to_string(),
                Some(Pending::Software { id, install }) => {
                    let name = PROGRAMS
                        .iter()
                        .find(|program| program.id == *id)
                        .map(|program| program.entry)
                        .unwrap_or("module");
                    format!(
                        "{} {name}...",
                        if *install { "Installing" } else { "Removing" }
                    )
                }
                None => "Loading...".to_string(),
            };
            let blink = ((elapsed * 2.5) as u32).is_multiple_of(2);
            let hint = if self.update_installing {
                "installing, please wait"
            } else {
                "Esc cancel"
            };
            ui::draw_loading(screen, &self.theme, &label, progress, blink, hint);
            return true;
        }
        if self.mode == Mode::Checking {
            let elapsed = self.update_started.elapsed().as_secs_f32();
            let label = if self.update_installing {
                let version = self
                    .update_offer
                    .as_ref()
                    .map(|offer| offer.version.clone())
                    .unwrap_or_default();
                format!("Installing {version}...")
            } else {
                "Checking github...".to_string()
            };
            // Ease towards the end and wait there: only the worker knows when
            // the round is done, and a bar that keeps creeping would lie.
            let progress = (elapsed / 6.0).powf(0.8).clamp(0.0, 1.0) * 0.9;
            let blink = ((elapsed * 2.5) as u32).is_multiple_of(2);
            let hint = if self.update_installing {
                "installing, please wait"
            } else {
                "Esc cancel"
            };
            ui::draw_loading(screen, &self.theme, &label, progress, blink, hint);
            return true;
        }
        if self.mode == Mode::Help {
            self.splash_at = None;
            let specs: Vec<&command::Spec> = if self.help_about {
                Vec::new()
            } else if self.help_all {
                command::SPECS.iter().collect()
            } else {
                command::specs_for(self.help_scope())
            };
            ui::draw_help(
                screen,
                &self.theme,
                self.help_about,
                &specs,
                self.help_selected,
            );
            return true;
        }

        false
    }

    /// What the prompt line says: the hint while a command is being typed, or
    /// the pending yes/no question, plus the error under it.
    fn prompt_hint(&self) -> (String, String) {
        // A pending yes/no question takes over the prompt line.
        let question = self.confirm.clone().unwrap_or_default();
        let hint = if !question.is_empty() {
            String::new()
        } else if self.command.is_empty() {
            if self.program.is_some() {
                // The prompt is the shell's, but the vocabulary belongs to the
                // module: a game has no actions to save.
                if self.program_id == Some("pocketboi") {
                    "mute   exit".to_string()
                } else {
                    "save   open   undo   exit".to_string()
                }
            } else if self.mode == Mode::Riff {
                "riff play   riff stop   riff next   riff volume".to_string()
            } else if self.has_document {
                // Left empty on purpose: the prompt then shows how to leave.
                String::new()
            } else if self.home == Home::Riff {
                "riff insert   riff play   help for commands".to_string()
            } else if matches!(self.home, Home::SoftHand | Home::Inspiro) {
                "new   open   help for commands".to_string()
            } else {
                "↑↓ select   Enter run   help for commands".to_string()
            }
        } else {
            command::complete(&self.command)
                .map(|c| c.hint)
                .unwrap_or_default()
        };

        let error = if question.is_empty() {
            self.command_error.clone()
        } else {
            question
        };
        (hint, error)
    }

    /// The screen under everything else: a program, the reader, a front door or
    /// the document.
    fn main_screen(&mut self, screen: &mut Screen, hint: &str, error: &str) {
        let cols = screen.cols();
        let rows = screen.rows();
        let now_local = chrono::Local::now();
        let clock = crate::clock::stamp(&now_local, self.clock_24h, self.date_day_first, false);
        let stamp = crate::clock::stamp(&now_local, self.clock_24h, self.date_day_first, true);

        if self.program.is_some() {
            // The program owns the grid: cells it does not paint must show the
            // pixels underneath, not whatever was on the screen before.
            screen.fill(cread_core::Cell::TRANSPARENT);
            let mut raster = std::mem::replace(&mut self.raster, Raster::new(1, 1));
            // Clear means "the page shows through": a program paints only what
            // it means to paint, and the rest is the shell's background.
            raster.fill_clear();
            let ink = Raster::nearest(self.theme.color(self.theme.fg));
            // Programs are written for the classic 80x25 screen; a bigger grid
            // gets that whole stage centred instead of pinned to the corner.
            let mut stage = Screen::new(80, 25);
            stage.fill(cread_core::Cell::TRANSPARENT);
            if let Some(program) = self.program.as_mut() {
                program.palette(Raster::nearest(self.theme.background()), ink);
                program.draw(&mut stage, &mut raster);
            }
            self.raster = raster;

            let (stage_x, stage_y) = self.program_stage(cols, rows);
            for y in 0..stage.rows() {
                for x in 0..stage.cols() {
                    if let Some(cell) = stage.get(x, y)
                        && !cell.is_transparent()
                    {
                        screen.put(stage_x + x, stage_y + y, *cell);
                    }
                }
            }

            if self.program_id == Some("softhand") {
                // The same bar as the editor: picture name and state on the
                // left, the clock in the middle, the folder on the right. The
                // palette strip has moved down with the canvas, so the bottom
                // row is free for the prompt only.
                let name = self
                    .picture_path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "untitled.png".to_string());
                let state = if self
                    .program
                    .as_ref()
                    .is_some_and(|program| program.modified())
                {
                    "modified"
                } else {
                    "saved"
                };
                let location = abbreviate_home(&picture::dir(&self.vault()));
                ui::draw_top_bar(
                    screen,
                    &self.theme,
                    &ui::TopBar {
                        title: &name,
                        state,
                        clock: &clock,
                        location: &location,
                    },
                );
                if self.program_prompt {
                    let prompt = Some((self.command.as_str(), hint, error));
                    ui::draw_program_line(screen, &self.theme, "", "", None, prompt);
                }
                // The tool keys sit right under the colour strip. The smallest
                // grid has no row left for them, so they are skipped there.
                let legend = self
                    .program
                    .as_ref()
                    .map(|program| program.key_legend())
                    .unwrap_or("");
                if !legend.is_empty() {
                    let (stage_x, stage_y) = self.program_stage(cols, rows);
                    let y = stage_y + 24;
                    let width = legend.chars().count() as u32;
                    let x = stage_x + 80u32.saturating_sub(width) / 2;
                    if y < rows && x < cols {
                        screen.put_str(x, y, legend, self.theme.dim, self.theme.bg);
                    }
                }
            } else {
                let title = self
                    .program
                    .as_ref()
                    .map(|p| p.title().to_string())
                    .unwrap_or_default();
                let state = self
                    .program
                    .as_ref()
                    .map(|p| p.status())
                    .unwrap_or_default();
                let status = if state.is_empty() {
                    "Esc commands".to_string()
                } else {
                    format!("{state}    Esc commands")
                };
                let prompt = self
                    .program_prompt
                    .then_some((self.command.as_str(), hint, error));
                let now = self.now_playing();
                ui::draw_program_line(screen, &self.theme, &title, &status, now.as_deref(), prompt);
            }
        } else if self.mode == Mode::View {
            let (width, _) = ui::layout(cols);
            if width != self.view_width {
                self.view_rows = cread_core::markdown::render(self.doc.editor.lines(), width);
                self.view_width = width;
            }
            let stats = self.doc.editor.stats();
            let state = if self.doc.editor.modified() {
                "modified"
            } else if self.doc.path.is_some() {
                "saved"
            } else {
                ""
            };
            let title = self.doc.title();
            let location = self.location();
            let view = ui::ReaderView {
                clock: &clock,
                title: &title,
                state,
                location: &location,
                rows: &self.view_rows,
                top: self.view_top,
                words: stats.words,
                hint: "Esc back to write mode",
            };
            ui::draw_reader(screen, &self.theme, &view);
        } else if !self.has_document {
            let show_prompt = matches!(self.mode, Mode::Action | Mode::Search);

            // Pixel art shrinks by a whole factor at most, and only when the
            // raster genuinely cannot hold it.
            let max_w = cols * cread_core::FONT_W;
            let max_h = rows.saturating_sub(8) * cread_core::FONT_H;
            // An overlay (browser, settings) covers the front door's logo the
            // same way it covers the boot screen's.
            let overlay_open = matches!(
                self.mode,
                Mode::Browser
                    | Mode::Webdav
                    | Mode::Quit
                    | Mode::Software
                    | Mode::Setup
                    | Mode::Help
            );
            if self.home == Home::Inspiro {
                let title_row =
                    match front_door_layout(&self.inspiro_logo, max_w, max_h, cols, rows) {
                        Some((x, y, divisor, img_rows)) => {
                            self.inspiro_at = (!overlay_open).then_some((x, y, divisor));
                            rows.saturating_sub(img_rows + 1 + 3) / 2 + img_rows + 1
                        }
                        None => {
                            self.inspiro_at = None;
                            rows.saturating_sub(3) / 2
                        }
                    };
                ui::draw_inspiro(screen, &self.theme, title_row, &self.command, hint, error);
            } else if self.home == Home::Riff {
                let title_row = match front_door_layout(&self.riff_logo, max_w, max_h, cols, rows) {
                    Some((x, y, divisor, img_rows)) => {
                        self.riff_at = (!overlay_open).then_some((x, y, divisor));
                        rows.saturating_sub(img_rows + 1 + 3) / 2 + img_rows + 1
                    }
                    None => {
                        self.riff_at = None;
                        rows.saturating_sub(3) / 2
                    }
                };
                ui::draw_front_door(
                    screen,
                    &self.theme,
                    title_row,
                    "a Soundtrack for your life.",
                    "type riff insert to load music",
                    &self.command,
                    hint,
                    error,
                );
            } else if self.home == Home::SoftHand {
                let title_row =
                    match front_door_layout(&self.softhand_logo, max_w, max_h, cols, rows) {
                        Some((x, y, divisor, img_rows)) => {
                            self.softhand_at = (!overlay_open).then_some((x, y, divisor));
                            rows.saturating_sub(img_rows + 1 + 3) / 2 + img_rows + 1
                        }
                        None => {
                            self.softhand_at = None;
                            rows.saturating_sub(3) / 2
                        }
                    };
                ui::draw_front_door(
                    screen,
                    &self.theme,
                    title_row,
                    "16 colors, endless possibilities.",
                    "type new or open to start",
                    &self.command,
                    hint,
                    error,
                );
            } else {
                let items = self.boot_rows();
                let note = String::new();

                // The whole group - logo, greeting, drives - is centred vertically,
                // like the old welcome screen was; pinning the logo to the top made
                // it look like a different screen. A long drive list shrinks the
                // logo by a whole step rather than losing rows off the bottom.
                let (divisor, img_rows) = boot_logo(
                    &self.splash,
                    max_w,
                    max_h,
                    rows,
                    items.len() as u32,
                    !note.is_empty(),
                );
                let gap = u32::from(img_rows > 0);
                let block =
                    img_rows + gap + 1 + 1 + items.len() as u32 + u32::from(!note.is_empty());
                let top = rows.saturating_sub(block) / 2;
                let title_row = top + img_rows + gap;

                self.splash_at = (img_rows > 0 && !overlay_open).then(|| {
                    let width = self
                        .splash
                        .as_ref()
                        .map(|s| (s.width() / divisor) as i32)
                        .unwrap_or(0);
                    let x = (cols * cread_core::FONT_W) as i32 / 2 - width / 2;
                    (x, (top * cread_core::FONT_H) as i32, divisor)
                });

                let view = ui::WelcomeView {
                    clock: &stamp,
                    show_prompt,
                    title_row,
                    items: &items,
                    selected: self.boot_selected,
                    note: &note,
                    input: &self.command,
                    hint,
                    error,
                };
                ui::draw_welcome(screen, &self.theme, &view);
            }
        } else {
            // A document is open: the logo belongs to the start screen only.
            self.splash_at = None;
            let (text_width, margin) = ui::layout(cols);
            let height = ui::text_height(rows);

            if self.follow_cursor {
                let anchor = self.typewriter.then_some(height / 2);
                self.doc.editor.scroll_to_cursor(text_width, height, anchor);
            }

            let visible = self.doc.editor.visible_rows(text_width, height);
            let cursor = self.doc.editor.cursor();
            let cursor_cell = visible
                .iter()
                .position(|r| r.contains(cursor))
                .map(|i| (i, cursor.col - visible[i].start));

            let in_fence = self
                .doc
                .editor
                .lines()
                .iter()
                .take(self.doc.editor.top().line)
                .filter(|l| cread_core::markdown::is_fence_line(l))
                .count()
                % 2
                == 1;

            let stats: Stats = self.doc.editor.stats();
            let state = if self.doc.editor.modified() {
                "modified"
            } else if self.doc.path.is_some() {
                if self.autosave {
                    "autosave on"
                } else {
                    "saved"
                }
            } else {
                ""
            };
            let location = self.location();
            let selection = self.doc.editor.selection();
            let (matches, current_match) = match &self.search {
                Some(search) => (
                    search.matches.as_slice(),
                    (!search.matches.is_empty()).then_some(search.current),
                ),
                None => (&[][..], None),
            };
            let message = if self.status_at.elapsed() < STATUS_TTL {
                self.status.as_str()
            } else {
                ""
            };

            let search_hint = self.search.as_ref().map(|s| {
                if s.matches.is_empty() {
                    "no matches   Esc cancel".to_string()
                } else {
                    format!(
                        "{}/{}   Enter keep   Esc cancel",
                        s.current + 1,
                        s.matches.len()
                    )
                }
            });
            let bottom = if self.mode == Mode::Search {
                ui::Bottom::Prompt {
                    sigil: "/",
                    input: self.search.as_ref().map(|s| s.query.as_str()).unwrap_or(""),
                    hint: search_hint.as_deref().unwrap_or(""),
                    error: "",
                }
            } else if self.mode == Mode::Action {
                ui::Bottom::Prompt {
                    sigil: ">",
                    input: &self.command,
                    hint,
                    error,
                }
            } else {
                ui::Bottom::Status { message }
            };
            // The document cursor belongs to write mode; the prompt has its own.
            let cursor_cell = if self.mode == Mode::Write {
                cursor_cell
            } else {
                None
            };

            let title = self.doc.title();
            let now = self.now_playing();
            let scroll = self.doc.editor.scroll_metrics(text_width);
            let view = ui::DocumentView {
                clock: &clock,
                title: &title,
                state,
                location: &location,
                lines: self.doc.editor.lines(),
                rows: &visible,
                selection,
                matches,
                current_match,
                cursor: cursor_cell,
                margin,
                stats,
                cursor_pos: (cursor.line, cursor.col),
                bottom,
                in_fence,
                cursor_on: self.blink,
                now: now.as_deref(),
                scroll,
            };
            ui::draw_document(screen, &self.theme, &view);
        }
    }

    /// Dialogs that sit on top of whatever the main screen drew.
    fn overlay(&mut self, screen: &mut Screen) {
        match self.mode {
            Mode::Browser => {
                let items = self.browser.items();
                let question;
                let (prompt, input, hint) = match &self.browser.mode {
                    BrowserMode::Browse if self.browser.kind == BrowserKind::Pictures => {
                        (">", self.browser.input.as_str(), "Enter open   Esc back")
                    }
                    BrowserMode::Browse => (
                        ">",
                        self.browser.input.as_str(),
                        "Enter open   Ctrl+R rename   Ctrl+D delete",
                    ),
                    BrowserMode::ConfirmDelete(path) => {
                        question = format!("delete {}? y/n", self.doc_title_of(path));
                        ("?", "", question.as_str())
                    }
                    BrowserMode::Rename { new, .. } => {
                        ("name>", new.as_str(), "Enter rename   Esc cancel")
                    }
                };
                let view = ui::BrowserView {
                    title: &self.browser.title,
                    prompt,
                    input,
                    items: &items,
                    selected: self.browser.selected,
                    hint,
                };
                ui::draw_browser(screen, &self.theme, &view);
            }
            Mode::Webdav => {
                let fields = self.form.fields();
                let view = ui::FormView {
                    title: "WEBDAV",
                    fields: &fields,
                    focus: self.form.focus,
                    note: &self.form.note,
                    hint: "Tab next   Enter toggle/save   Esc cancel",
                };
                ui::draw_form(screen, &self.theme, &view);
            }
            Mode::Quit => {
                let yes = self.quit.map(|prompt| prompt.yes).unwrap_or(true);
                ui::draw_confirm(
                    screen,
                    &self.theme,
                    "SYNC TO SERVER?",
                    "Changes in this session have not been sent yet.",
                    yes,
                );
            }
            Mode::Notice => {
                ui::draw_notice(screen, &self.theme, &self.notice_text);
            }
            Mode::UpdatePrompt => {
                let version = self
                    .update_offer
                    .as_ref()
                    .map(|offer| offer.version.clone())
                    .unwrap_or_default();
                ui::draw_confirm(
                    screen,
                    &self.theme,
                    "UPDATE AVAILABLE",
                    &format!("{version} is available. Install and restart?"),
                    self.update_yes,
                );
            }
            _ => {}
        }
    }
}
