//! The calendar screen, its event form and the reminder service.
//!
//! Events are one Markdown file each under `<vault>/Calendar`; the WebDAV
//! worker already mirrors that folder. A deleted event becomes a visible
//! tombstone so the local-first sync cannot bring it back from the server.

use super::*;
use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, NaiveDateTime, NaiveTime};
use cread_core::Cell;
use cread_core::calendar as cal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CalendarFocus {
    Grid,
    List,
}

pub(super) struct CalendarState {
    pub(super) month: cal::Month,
    pub(super) selected: NaiveDate,
    pub(super) events: Vec<cal::Entry>,
    pub(super) focus: CalendarFocus,
    pub(super) selected_event: usize,
}

impl CalendarState {
    pub(super) fn today() -> Self {
        let today = Local::now().date_naive();
        Self {
            month: month_for(today),
            selected: today,
            events: Vec::new(),
            focus: CalendarFocus::Grid,
            selected_event: 0,
        }
    }

    fn selected_events(&self) -> Vec<&cal::Entry> {
        let mut events: Vec<&cal::Entry> = self
            .events
            .iter()
            .filter(|entry| entry.event.occurs_on(self.selected))
            .collect();
        events.sort_by_key(|entry| {
            (
                entry.event.time.is_some(),
                entry.event.time,
                entry.event.title.clone(),
            )
        });
        events
    }

    fn clamp(&mut self) {
        let count = self.selected_events().len();
        self.selected_event = if count == 0 {
            0
        } else {
            self.selected_event.min(count - 1)
        };
    }
}

#[derive(Clone)]
pub(super) struct EventForm {
    pub(super) path: Option<PathBuf>,
    pub(super) title: String,
    pub(super) date: String,
    pub(super) time: String,
    pub(super) remind: String,
    pub(super) notes: String,
    pub(super) repeat: String,
    pub(super) field: usize,
    pub(super) editing: bool,
    pub(super) dirty: bool,
    pub(super) confirm_discard: bool,
    pub(super) error: String,
}

impl EventForm {
    fn new(date: NaiveDate) -> Self {
        Self {
            path: None,
            title: String::new(),
            date: date.format("%Y-%m-%d").to_string(),
            time: String::new(),
            remind: "10".into(),
            notes: String::new(),
            repeat: "none".into(),
            field: 0,
            editing: false,
            dirty: false,
            confirm_discard: false,
            error: String::new(),
        }
    }

    fn edit(entry: &cal::Entry) -> Self {
        Self {
            path: Some(entry.path.clone()),
            title: entry.event.title.clone(),
            date: entry.event.date.format("%Y-%m-%d").to_string(),
            time: entry
                .event
                .time
                .map(|time| time.format("%H:%M").to_string())
                .unwrap_or_default(),
            remind: entry
                .event
                .remind
                .map(|minutes| minutes.to_string())
                .unwrap_or_default(),
            notes: entry.event.notes.trim_end().to_string(),
            repeat: entry.event.repeat.as_str().into(),
            field: 0,
            editing: false,
            dirty: false,
            confirm_discard: false,
            error: String::new(),
        }
    }

    fn values(&self) -> [(&'static str, &str); 6] {
        [
            ("Title", &self.title),
            ("Date", &self.date),
            ("Time", &self.time),
            ("Repeat", &self.repeat),
            ("Remind", &self.remind),
            ("Notes", &self.notes),
        ]
    }

    fn active_value_mut(&mut self) -> &mut String {
        match self.field {
            0 => &mut self.title,
            1 => &mut self.date,
            2 => &mut self.time,
            3 => &mut self.repeat,
            4 => &mut self.remind,
            _ => &mut self.notes,
        }
    }
}

pub(super) struct ReminderDue {
    pub(super) at: NaiveDateTime,
    pub(super) key: String,
    pub(super) title: String,
    pub(super) when: String,
    pub(super) date: NaiveDate,
    pub(super) path: PathBuf,
}

fn month_for(date: NaiveDate) -> cal::Month {
    cal::month(date.year(), date.month()).unwrap_or_else(|| cal::month(1970, 1).unwrap())
}

fn days_in_month(date: NaiveDate) -> u32 {
    let first = NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap_or(date);
    let next = if first.month() == 12 {
        NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
    };
    next.map(|next| (next - first).num_days() as u32)
        .unwrap_or(28)
}

fn shift_month(date: NaiveDate, delta: i32) -> NaiveDate {
    let month_index = date.year() * 12 + date.month0() as i32 + delta;
    let year = month_index.div_euclid(12);
    let month = month_index.rem_euclid(12) as u32 + 1;
    let day = date.day().min(
        NaiveDate::from_ymd_opt(year, month, 1)
            .map(days_in_month)
            .unwrap_or(28),
    );
    NaiveDate::from_ymd_opt(year, month, day).unwrap_or(date)
}

fn reminder_key(entry: &cal::Entry, at: NaiveDateTime) -> String {
    format!("{}|{}", entry.path.display(), at.format("%Y-%m-%d %H:%M"))
}

impl App {
    pub(super) fn open_calendar(&mut self) {
        let back = self.mode;
        self.prepare_vault();
        self.reload_calendar();
        let today = Local::now().date_naive();
        self.calendar.month = month_for(today);
        self.calendar.selected = today;
        self.calendar.focus = CalendarFocus::Grid;
        self.calendar.selected_event = 0;
        self.calendar_delete_armed = None;
        self.calendar_back = back;
        self.mode = Mode::Calendar;
        self.program_prompt = false;
        self.command.clear();
        self.command_error.clear();
        self.dirty = true;
    }

    pub(super) fn open_calendar_at(&mut self, date: NaiveDate, path: PathBuf) {
        self.prepare_vault();
        self.reload_calendar();
        self.calendar.month = month_for(date);
        self.calendar.selected = date;
        self.calendar.focus = CalendarFocus::List;
        self.calendar_delete_armed = None;
        self.calendar.selected_event = self
            .calendar
            .selected_events()
            .iter()
            .position(|entry| entry.path == path)
            .unwrap_or(0);
        // A reminder that fires while the calendar is already in front must not
        // make the calendar its own exit target.
        if !matches!(self.reminder_back, Mode::Calendar | Mode::CalendarEdit) {
            self.calendar_back = self.reminder_back;
        }
        self.mode = Mode::Calendar;
        self.program_prompt = false;
        self.dirty = true;
    }

    pub(super) fn leave_calendar(&mut self) {
        self.calendar_form = None;
        self.calendar_delete_armed = None;
        self.mode = if matches!(self.calendar_back, Mode::Calendar | Mode::CalendarEdit) {
            Mode::Action
        } else {
            self.calendar_back
        };
        self.program_prompt = false;
        self.command.clear();
        self.command_error.clear();
        self.dirty = true;
    }

    pub(super) fn reload_calendar(&mut self) {
        self.prepare_vault();
        let dir = storage::calendar_in(&self.vault());
        self.calendar.events = cal::load_events(&dir);
        self.calendar.clamp();
        self.schedule_reminder();
        self.dirty = true;
    }

    fn move_calendar(&mut self, days: i64) {
        let date = self
            .calendar
            .selected
            .checked_add_signed(ChronoDuration::days(days))
            .unwrap_or(self.calendar.selected);
        self.calendar.selected = date;
        self.calendar.month = month_for(date);
        self.calendar.selected_event = 0;
        self.dirty = true;
    }

    fn change_month(&mut self, delta: i32) {
        let date = shift_month(self.calendar.selected, delta);
        self.calendar.selected = date;
        self.calendar.month = month_for(date);
        self.calendar.selected_event = 0;
        self.dirty = true;
    }

    fn focus_today(&mut self) {
        let today = Local::now().date_naive();
        self.calendar.selected = today;
        self.calendar.month = month_for(today);
        self.calendar.selected_event = 0;
        self.dirty = true;
    }

    pub(super) fn new_event_form(&mut self) {
        let date = self.calendar.selected;
        self.calendar_form = Some(EventForm::new(date));
        self.mode = Mode::CalendarEdit;
        self.dirty = true;
    }

    pub(super) fn edit_selected_event(&mut self) {
        let Some(entry) = self
            .calendar
            .selected_events()
            .get(self.calendar.selected_event)
            .cloned()
        else {
            self.set_status("no appointment to edit");
            return;
        };
        let entry = (*entry).clone();
        self.calendar_form = Some(EventForm::edit(&entry));
        self.mode = Mode::CalendarEdit;
        self.dirty = true;
    }

    /// `D` twice: the first press only names what the second would tombstone.
    fn arm_or_delete_selected_event(&mut self) {
        let Some(entry) = self
            .calendar
            .selected_events()
            .get(self.calendar.selected_event)
            .cloned()
        else {
            self.calendar_delete_armed = None;
            self.set_status("no appointment to delete");
            return;
        };
        if self.calendar_delete_armed.as_deref() == Some(entry.path.as_path()) {
            self.calendar_delete_armed = None;
            self.delete_selected_event();
        } else {
            self.calendar_delete_armed = Some(entry.path.clone());
            self.set_status(format!("press D again to delete {}", entry.event.title));
        }
    }

    pub(super) fn delete_selected_event(&mut self) {
        let Some(entry) = self
            .calendar
            .selected_events()
            .get(self.calendar.selected_event)
            .cloned()
        else {
            self.set_status("no appointment to delete");
            return;
        };
        let mut event = entry.event.clone();
        event.deleted = true;
        match storage::save_atomic(&entry.path, &event.to_markdown()) {
            Ok(()) => {
                self.unsynced = true;
                let title = event.title.clone();
                self.reload_calendar();
                self.set_status(format!("deleted {title}"));
            }
            Err(e) => self.set_status(format!("delete failed: {e}")),
        }
        self.dirty = true;
    }

    pub(super) fn save_event_form(&mut self) {
        let Some(form) = self.calendar_form.clone() else {
            return;
        };
        let title = form.title.trim().to_string();
        if title.is_empty() {
            if let Some(form) = self.calendar_form.as_mut() {
                form.error = "a title is required".into();
            }
            return;
        }
        let date = match NaiveDate::parse_from_str(form.date.trim(), "%Y-%m-%d") {
            Ok(date) => date,
            Err(_) => {
                if let Some(form) = self.calendar_form.as_mut() {
                    form.error = "date must be YYYY-MM-DD".into();
                }
                return;
            }
        };
        let time = if form.time.trim().is_empty() {
            None
        } else {
            match NaiveTime::parse_from_str(form.time.trim(), "%H:%M") {
                Ok(time) => Some(time),
                Err(_) => {
                    if let Some(form) = self.calendar_form.as_mut() {
                        form.error = "time must be HH:MM or empty".into();
                    }
                    return;
                }
            }
        };
        let remind = if time.is_none() || form.remind.trim().is_empty() {
            None
        } else {
            match form.remind.trim().parse::<u32>() {
                Ok(minutes) => Some(minutes),
                Err(_) => {
                    if let Some(form) = self.calendar_form.as_mut() {
                        form.error = "reminder must be a number of minutes".into();
                    }
                    return;
                }
            }
        };
        let repeat = match cal::Repeat::parse(&form.repeat) {
            Some(repeat) => repeat,
            None => {
                if let Some(form) = self.calendar_form.as_mut() {
                    form.error = "repeat must be none or weekly".into();
                }
                return;
            }
        };
        let event = cal::Event {
            title,
            date,
            time,
            remind,
            repeat,
            deleted: false,
            notes: form.notes.trim_end().to_string(),
        };
        self.prepare_vault();
        let path = form.path.clone().unwrap_or_else(|| {
            cal::free_event_path(
                &storage::calendar_in(&self.vault()),
                date,
                time,
                &event.title,
            )
        });
        match storage::save_atomic(&path, &event.to_markdown()) {
            Ok(()) => {
                self.unsynced = true;
                self.calendar_form = None;
                self.calendar.selected = date;
                self.calendar.month = month_for(date);
                self.calendar.focus = CalendarFocus::List;
                self.calendar.selected_event = 0;
                self.mode = Mode::Calendar;
                self.reload_calendar();
                self.set_status(format!("saved {}", event.title));
            }
            Err(e) => {
                if let Some(form) = self.calendar_form.as_mut() {
                    form.error = format!("save failed: {e}");
                }
            }
        }
        self.dirty = true;
    }

    pub(super) fn on_key_calendar(&mut self, el: &ActiveEventLoop, event: KeyEvent) {
        let _ = el;
        // Arming a delete only survives until the next keypress.
        if !matches!(&event.logical_key, Key::Character(text) if text.eq_ignore_ascii_case("d")) {
            self.calendar_delete_armed = None;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.leave_calendar(),
            Key::Named(NamedKey::Enter) => self.new_event_form(),
            Key::Named(NamedKey::Tab) => {
                self.calendar.focus = if self.calendar.focus == CalendarFocus::Grid {
                    CalendarFocus::List
                } else {
                    CalendarFocus::Grid
                };
                self.calendar.selected_event = 0;
                self.poke();
            }
            Key::Named(NamedKey::ArrowLeft) => {
                if self.calendar.focus == CalendarFocus::Grid {
                    self.move_calendar(-1);
                }
                self.poke();
            }
            Key::Named(NamedKey::ArrowRight) => {
                if self.calendar.focus == CalendarFocus::Grid {
                    self.move_calendar(1);
                }
                self.poke();
            }
            Key::Named(NamedKey::ArrowUp) => {
                if self.calendar.focus == CalendarFocus::List {
                    let count = self.calendar.selected_events().len();
                    if count > 0 {
                        self.calendar.selected_event =
                            self.calendar.selected_event.saturating_sub(1);
                    }
                } else {
                    self.move_calendar(-7);
                }
                self.poke();
            }
            Key::Named(NamedKey::ArrowDown) => {
                if self.calendar.focus == CalendarFocus::List {
                    let count = self.calendar.selected_events().len();
                    if count > 0 {
                        self.calendar.selected_event =
                            (self.calendar.selected_event + 1).min(count - 1);
                    }
                } else {
                    self.move_calendar(7);
                }
                self.poke();
            }
            Key::Named(NamedKey::PageUp) => {
                self.change_month(-1);
                self.poke();
            }
            Key::Named(NamedKey::PageDown) => {
                self.change_month(1);
                self.poke();
            }
            Key::Named(NamedKey::Home) => {
                self.focus_today();
                self.poke();
            }
            Key::Character(text) => {
                if self.mods.control_key() || self.mods.alt_key() {
                    return;
                }
                for ch in text.chars() {
                    match ch.to_ascii_lowercase() {
                        'n' => self.new_event_form(),
                        'e' | 'd' if self.calendar.focus != CalendarFocus::List => {
                            self.set_status("select an appointment in the list first");
                        }
                        'e' => self.edit_selected_event(),
                        'd' => self.arm_or_delete_selected_event(),
                        't' => self.focus_today(),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn on_key_calendar_edit(&mut self, event: KeyEvent) {
        let shift = self.mods.shift_key();
        let Some(form) = self.calendar_form.as_mut() else {
            return;
        };
        if form.confirm_discard {
            match &event.logical_key {
                Key::Character(text) if text.eq_ignore_ascii_case("y") => {
                    self.calendar_form = None;
                    self.mode = Mode::Calendar;
                    self.dirty = true;
                }
                Key::Character(text) if text.eq_ignore_ascii_case("n") => {
                    form.confirm_discard = false;
                    self.poke();
                }
                Key::Named(NamedKey::Escape) => {
                    form.confirm_discard = false;
                    self.poke();
                }
                _ => {}
            }
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                if form.dirty {
                    form.confirm_discard = true;
                } else {
                    self.calendar_form = None;
                    self.mode = Mode::Calendar;
                }
                self.dirty = true;
            }
            Key::Named(NamedKey::Enter) => {
                if form.field == 6 {
                    self.save_event_form();
                    return;
                }
                if form.editing {
                    form.field += 1;
                    form.editing = form.field < 6;
                } else {
                    form.editing = true;
                }
                self.poke();
            }
            Key::Named(NamedKey::Tab) => {
                if shift {
                    form.field = form.field.saturating_sub(1);
                } else {
                    form.field = (form.field + 1) % 7;
                }
                form.editing = form.field < 6;
                self.poke();
            }
            Key::Named(NamedKey::Backspace) => {
                if form.editing {
                    form.active_value_mut().pop();
                    form.dirty = true;
                }
                self.poke();
            }
            Key::Named(NamedKey::Space) => {
                if form.field == 3 {
                    form.repeat = if form.repeat == "weekly" {
                        "none".into()
                    } else {
                        "weekly".into()
                    };
                    form.editing = true;
                    form.dirty = true;
                } else if form.editing {
                    form.active_value_mut().push(' ');
                    form.dirty = true;
                }
                self.poke();
            }
            Key::Character(text) => {
                if !form.editing || self.mods.alt_key() {
                    return;
                }
                for ch in text.chars().filter(|ch| !ch.is_control()) {
                    form.active_value_mut().push(ch);
                }
                form.dirty = true;
                self.poke();
            }
            _ => {}
        }
    }

    pub(super) fn on_key_reminder(&mut self, event: KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => {
                let date = self.reminder_date;
                let path = self.reminder_path.clone();
                self.open_calendar_at(date, path);
            }
            _ => {
                self.mode = self.reminder_back;
                self.dirty = true;
            }
        }
    }

    pub(super) fn schedule_reminder(&mut self) {
        // Look one minute back so a second appointment due at the same moment
        // is still seen after the first one fired. Skip over occurrences that
        // already fired; for a weekly event that advances to next week rather
        // than getting stuck on today.
        let base = Local::now().naive_local() - ChronoDuration::minutes(1);
        let mut next: Option<ReminderDue> = None;
        for entry in &self.calendar.events {
            if entry.event.deleted {
                continue;
            }
            let mut search_from = base;
            while let Some((occurrence, at)) = entry.event.next_reminder_after(search_from) {
                let key = reminder_key(entry, at);
                if self.reminder_fired.contains(&key) {
                    search_from = at + ChronoDuration::seconds(1);
                    continue;
                }
                let due = ReminderDue {
                    at,
                    key,
                    title: entry.event.title.clone(),
                    when: format!(
                        "{} · {}",
                        occurrence.format("%A %d %B %Y"),
                        entry.event.time_label(self.clock_24h)
                    ),
                    date: occurrence,
                    path: entry.path.clone(),
                };
                if next.as_ref().is_none_or(|old| due.at < old.at) {
                    next = Some(due);
                }
                break;
            }
        }
        self.reminder = next;
    }

    pub(super) fn pump_reminders(&mut self) {
        if matches!(
            self.mode,
            Mode::Reminder
                | Mode::CalendarEdit
                | Mode::Quit
                | Mode::Power
                | Mode::UpdatePrompt
                | Mode::Loading
                | Mode::Checking
        ) {
            return;
        }
        let now = Local::now().naive_local();
        let Some(due) = &self.reminder else {
            return;
        };
        if due.at > now {
            return;
        }
        let due = self.reminder.take().unwrap();
        self.reminder_fired.insert(due.key.clone());
        self.reminder_back = self.mode;
        self.reminder_title = due.title.clone();
        self.reminder_when = due.when.clone();
        self.reminder_date = due.date;
        self.reminder_path = due.path.clone();
        self.mode = Mode::Reminder;
        self.beep_twice();
        self.dirty = true;
        self.schedule_reminder();
    }

    pub(super) fn next_reminder_instant(&self) -> Option<Instant> {
        let due = self.reminder.as_ref()?;
        let now = Local::now().naive_local();
        let seconds = due.at.signed_duration_since(now).num_seconds();
        if seconds <= 0 {
            return None;
        }
        Some(Instant::now() + Duration::from_secs(seconds as u64))
    }

    pub(super) fn draw_reminder(&mut self, screen: &mut Screen) {
        let width = screen.cols().saturating_sub(12).clamp(30, 58);
        let height = 9u32.min(screen.rows());
        let x0 = screen.cols().saturating_sub(width) / 2;
        let y0 = screen.rows().saturating_sub(height) / 2;
        for y in y0..y0 + height {
            for x in x0..x0 + width {
                screen.put(x, y, Cell::blank(self.theme.fg, self.theme.bg));
            }
        }
        screen.put_str(x0, y0, "\u{2554}", self.theme.fg, self.theme.bg);
        screen.put_str(x0 + width - 1, y0, "\u{2557}", self.theme.fg, self.theme.bg);
        screen.put_str(
            x0,
            y0 + height - 1,
            "\u{255a}",
            self.theme.fg,
            self.theme.bg,
        );
        screen.put_str(
            x0 + width - 1,
            y0 + height - 1,
            "\u{255d}",
            self.theme.fg,
            self.theme.bg,
        );
        screen.hline(
            x0 + 1,
            y0,
            width - 2,
            '\u{2550}',
            self.theme.fg,
            self.theme.bg,
        );
        screen.hline(
            x0 + 1,
            y0 + height - 1,
            width - 2,
            '\u{2550}',
            self.theme.fg,
            self.theme.bg,
        );
        for y in y0 + 1..y0 + height - 1 {
            screen.put(x0, y, Cell::new('\u{2551}', self.theme.fg, self.theme.bg));
            screen.put(
                x0 + width - 1,
                y,
                Cell::new('\u{2551}', self.theme.fg, self.theme.bg),
            );
        }
        let title = " REMINDER ";
        screen.put_str(
            x0 + width.saturating_sub(title.chars().count() as u32) / 2,
            y0,
            title,
            self.theme.accent,
            self.theme.bg,
        );
        screen.put_str_ellipsis(
            x0 + 2,
            y0 + 2,
            &self.reminder_title,
            width - 4,
            self.theme.fg,
            self.theme.bg,
        );
        screen.put_str_ellipsis(
            x0 + 2,
            y0 + 3,
            &self.reminder_when,
            width - 4,
            self.theme.dim,
            self.theme.bg,
        );
        let hint = "Enter: open calendar   Esc: close";
        screen.put_str_ellipsis(
            x0 + 2,
            y0 + height - 2,
            hint,
            width - 4,
            self.theme.dim,
            self.theme.bg,
        );
    }

    fn draw_calendar_panel(
        &self,
        screen: &mut Screen,
        x0: u32,
        x1: u32,
        y0: u32,
        y1: u32,
        active: bool,
    ) {
        let colour = if active {
            self.theme.accent
        } else {
            self.theme.dim
        };
        screen.put(x0, y0, Cell::new('\u{250c}', colour, self.theme.bg));
        screen.put(x1, y0, Cell::new('\u{2510}', colour, self.theme.bg));
        screen.put(x0, y1, Cell::new('\u{2514}', colour, self.theme.bg));
        screen.put(x1, y1, Cell::new('\u{2518}', colour, self.theme.bg));
        screen.hline(
            x0 + 1,
            y0,
            x1.saturating_sub(x0 + 1),
            '\u{2500}',
            colour,
            self.theme.bg,
        );
        screen.hline(
            x0 + 1,
            y1,
            x1.saturating_sub(x0 + 1),
            '\u{2500}',
            colour,
            self.theme.bg,
        );
        for y in y0 + 1..y1 {
            screen.put(x0, y, Cell::new('\u{2502}', colour, self.theme.bg));
            screen.put(x1, y, Cell::new('\u{2502}', colour, self.theme.bg));
        }
    }

    pub(super) fn draw_calendar(&mut self, screen: &mut Screen) {
        let now = Local::now();
        let clock = crate::clock::stamp(&now, self.clock_24h, self.date_day_first, false);
        let month = cal::month_label(self.calendar.month.year, self.calendar.month.month);
        ui::draw_top_bar(
            screen,
            &self.theme,
            &ui::TopBar {
                title: "CALENDAR",
                state: &month,
                clock: &clock,
                location: "Calendar",
            },
        );

        screen.fill(Cell::blank(self.theme.fg, self.theme.bg));
        // Put the top bar back after the fill.
        ui::draw_top_bar(
            screen,
            &self.theme,
            &ui::TopBar {
                title: "CALENDAR",
                state: &month,
                clock: &clock,
                location: "Calendar",
            },
        );

        self.draw_calendar_panel(
            screen,
            0,
            screen.cols().saturating_sub(1),
            1,
            9,
            self.calendar.focus == CalendarFocus::Grid,
        );
        self.draw_calendar_panel(
            screen,
            0,
            screen.cols().saturating_sub(1),
            11,
            21,
            self.calendar.focus == CalendarFocus::List,
        );

        let x0 = 4u32;
        for (col, label) in cal::weekday_labels().iter().enumerate() {
            screen.put_str(
                x0 + col as u32 * 10 + 3,
                2,
                label,
                self.theme.dim,
                self.theme.bg,
            );
        }
        let today = now.date_naive();
        for (row, week) in self.calendar.month.weeks.iter().enumerate() {
            for (col, day) in week.iter().enumerate() {
                let Some(date) = day else {
                    continue;
                };
                let x = x0 + col as u32 * 10;
                let y = 3 + row as u32;
                let selected = *date == self.calendar.selected;
                let is_today = *date == today;
                let count = self
                    .calendar
                    .events
                    .iter()
                    .filter(|entry| entry.event.occurs_on(*date))
                    .count();
                let (fg, bg) = if selected {
                    (self.theme.bg, self.theme.fg)
                } else if is_today {
                    (self.theme.accent, self.theme.bg)
                } else {
                    (self.theme.fg, self.theme.bg)
                };
                if selected {
                    screen.hline(x, y, 9, ' ', fg, bg);
                }
                screen.put_str(x + 3, y, &date.day().to_string(), fg, bg);
                if count > 0 {
                    screen.put_str(
                        x + 7,
                        y,
                        "*",
                        if selected { bg } else { self.theme.accent },
                        bg,
                    );
                }
            }
        }

        let heading = format!(
            "{} {}   [{}]",
            self.calendar
                .selected
                .format("%A")
                .to_string()
                .to_uppercase(),
            self.calendar.selected.format("%d %B %Y"),
            if self.calendar.focus == CalendarFocus::Grid {
                "GRID"
            } else {
                "LIST"
            }
        );
        screen.put_str(2, 10, &heading, self.theme.accent, self.theme.bg);
        let events = self.calendar.selected_events();
        if events.is_empty() {
            screen.put_str(2, 12, "no appointments", self.theme.dim, self.theme.bg);
        } else {
            for (index, entry) in events.iter().take(9).enumerate() {
                let y = 12 + index as u32;
                let selected = self.calendar.focus == CalendarFocus::List
                    && index == self.calendar.selected_event;
                let (fg, bg) = if selected {
                    (self.theme.bg, self.theme.fg)
                } else {
                    (self.theme.fg, self.theme.bg)
                };
                let line = format!(
                    "{}  {}",
                    entry.event.time_label(self.clock_24h),
                    entry.event.title
                );
                if selected {
                    screen.hline(1, y, screen.cols().saturating_sub(2), ' ', fg, bg);
                }
                screen.put_str_ellipsis(2, y, &line, screen.cols().saturating_sub(4), fg, bg);
            }
        }

        screen.put_str_ellipsis(
            1,
            22,
            "Tab focus   Enter new   E edit   D delete x2   PgUp/PgDn month   Esc back",
            screen.cols().saturating_sub(2),
            self.theme.dim,
            self.theme.bg,
        );
        let status = if self.status.is_empty() {
            "Calendar"
        } else {
            self.status.as_str()
        };
        screen.put_str_ellipsis(
            1,
            23,
            status,
            screen.cols().saturating_sub(2),
            self.theme.fg,
            self.theme.bg,
        );
    }

    pub(super) fn draw_calendar_form(&mut self, screen: &mut Screen) {
        let now = Local::now();
        let clock = crate::clock::stamp(&now, self.clock_24h, self.date_day_first, false);
        let Some(form) = self.calendar_form.as_ref() else {
            return;
        };
        let title = if form.path.is_some() {
            "EDIT EVENT"
        } else {
            "NEW EVENT"
        };
        let title = if form.dirty {
            format!("{title} · UNSAVED")
        } else {
            title.to_string()
        };
        screen.fill(Cell::blank(self.theme.fg, self.theme.bg));
        ui::draw_top_bar(
            screen,
            &self.theme,
            &ui::TopBar {
                title: "CALENDAR",
                state: &title,
                clock: &clock,
                location: "Calendar",
            },
        );
        let left = 7u32;
        let right = 72u32;
        let top = 2u32;
        let bottom = 20u32;
        screen.put_str(left, top, "\u{2554}", self.theme.fg, self.theme.bg);
        screen.put_str(right, top, "\u{2557}", self.theme.fg, self.theme.bg);
        screen.put_str(left, bottom, "\u{255a}", self.theme.fg, self.theme.bg);
        screen.put_str(right, bottom, "\u{255d}", self.theme.fg, self.theme.bg);
        screen.hline(
            left + 1,
            top,
            right - left - 1,
            '\u{2550}',
            self.theme.fg,
            self.theme.bg,
        );
        screen.hline(
            left + 1,
            bottom,
            right - left - 1,
            '\u{2550}',
            self.theme.fg,
            self.theme.bg,
        );
        for y in top + 1..bottom {
            screen.put(left, y, Cell::new('\u{2551}', self.theme.fg, self.theme.bg));
            screen.put(
                right,
                y,
                Cell::new('\u{2551}', self.theme.fg, self.theme.bg),
            );
        }
        screen.put_str(left + 3, top + 1, &title, self.theme.accent, self.theme.bg);
        let values = form.values();
        for (index, (label, value)) in values.iter().enumerate() {
            let y = top + 3 + index as u32 * 2;
            let selected = form.field == index;
            let (fg, bg) = if selected {
                (self.theme.bg, self.theme.fg)
            } else {
                (self.theme.fg, self.theme.bg)
            };
            if selected {
                screen.hline(left + 2, y, right - left - 3, ' ', fg, bg);
            }
            screen.put_str(
                left + 2,
                y,
                label,
                if selected { fg } else { self.theme.dim },
                bg,
            );
            let value_x = left + 22;
            screen.put_str_ellipsis(value_x, y, value, right.saturating_sub(value_x + 1), fg, bg);
            if selected && form.editing {
                let cursor = value_x + value.chars().count() as u32;
                if cursor < right {
                    screen.put(
                        cursor,
                        y,
                        Cell::new('\u{2588}', self.theme.bg, self.theme.fg),
                    );
                }
            }
        }
        let save_selected = form.field == 6;
        if save_selected {
            screen.hline(
                left + 2,
                top + 15,
                right - left - 3,
                ' ',
                self.theme.bg,
                self.theme.fg,
            );
        }
        screen.put_str(
            left + 2,
            top + 15,
            if save_selected { "> SAVE" } else { "  SAVE" },
            if save_selected {
                self.theme.bg
            } else {
                self.theme.accent
            },
            if save_selected {
                self.theme.fg
            } else {
                self.theme.bg
            },
        );
        if !form.error.is_empty() {
            screen.put_str_ellipsis(
                left + 2,
                bottom.saturating_sub(1),
                &form.error,
                right - left - 3,
                self.theme.error,
                self.theme.bg,
            );
        }
        screen.put_str(
            left + 2,
            bottom + 1,
            "Enter edit/confirm   Tab next   Esc cancel",
            self.theme.dim,
            self.theme.bg,
        );
        screen.put_str(
            left + 2,
            bottom + 2,
            "Date YYYY-MM-DD · Time HH:MM (blank = all day)",
            self.theme.dim,
            self.theme.bg,
        );
        screen.put_str(
            left + 2,
            bottom + 3,
            "Repeat none/weekly (Space toggles)",
            self.theme.dim,
            self.theme.bg,
        );
        screen.put_str(
            left + 2,
            bottom + 4,
            "Remind minutes before (blank = none)",
            self.theme.dim,
            self.theme.bg,
        );
        if form.confirm_discard {
            ui::draw_confirm(
                screen,
                &self.theme,
                "DISCARD EVENT?",
                "Changes have not been saved.",
                false,
            );
        }
    }
}
