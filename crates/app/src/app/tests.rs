use super::boot::BootItem;
use super::manual::{manual_heading, manual_jumps};
use super::*;

fn app() -> App {
    let mut app = App::new(Options::default());
    // The developer's own preferences live on this machine; a test wants a
    // fresh shelf and must not write over them.
    app.prefs.installed.clear();
    app.persist = false;
    app
}

fn rows_as_text(screen: &Screen) -> String {
    let mut out = String::new();
    for y in 0..screen.rows() {
        for x in 0..screen.cols() {
            out.push(screen.get(x, y).unwrap().ch);
        }
        out.push('\n');
    }
    out
}

fn vault(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cread-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn app_in(dir: &Path) -> App {
    let mut app = App::new(Options {
        vault: Some(dir.to_path_buf()),
        ..Options::default()
    });
    app.prefs.installed.clear();
    app.persist = false;
    app
}

#[test]
fn autosave_is_off_until_it_is_asked_for() {
    let dir = vault("autosave");
    let mut app = app_in(&dir);
    app.new_document().unwrap();
    let path = app.doc.path.clone().unwrap();
    app.mode = Mode::Write;
    app.doc.editor.insert_text("typed but not saved");
    // Pretend the last keystroke was long ago.
    app.doc.last_edit = Instant::now() - AUTOSAVE_DELAY * 2;

    app.autosave_if_due();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "", "still empty");

    app.autosave = true;
    app.autosave_if_due();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "typed but not saved"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn opening_another_document_never_discards_unsaved_text() {
    let dir = vault("open-unsaved");
    let mut app = app_in(&dir);
    app.new_document().unwrap();
    app.doc.editor.insert_text("not saved yet");
    std::fs::create_dir_all(dir.join("Documents")).unwrap();
    std::fs::write(dir.join("Documents/other.md"), "other").unwrap();

    let error = app
        .open_document(dir.join("Documents/other.md"))
        .unwrap_err();
    assert!(error.contains("unsaved"), "{error}");
    assert_eq!(app.doc.editor.text(), "not saved yet");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saveas_refuses_to_overwrite_an_existing_document() {
    let dir = vault("saveas-existing");
    let mut app = app_in(&dir);
    app.new_document().unwrap();
    app.doc.editor.insert_text("mine");
    std::fs::create_dir_all(dir.join("Documents")).unwrap();
    std::fs::write(dir.join("Documents/other.md"), "theirs").unwrap();

    let error = app.save_as("other").unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(
        std::fs::read_to_string(dir.join("Documents/other.md")).unwrap(),
        "theirs"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_without_a_document_does_not_create_untitled() {
    let dir = vault("save-no-doc");
    let mut app = app_in(&dir);
    let error = app.save().unwrap_err();
    assert!(error.contains("no document"), "{error}");
    assert!(!dir.join("Documents/untitled.md").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_scratch_document_is_removed_when_it_is_left() {
    let dir = vault("scratch-empty");
    let mut app = app_in(&dir);
    app.new_document().unwrap();
    let path = app.doc.path.clone().unwrap();
    assert!(path.is_file(), "new reserves a name straight away");

    app.discard_scratch();
    assert!(!path.exists(), "nothing was written, so nothing stays");

    // The name is free again, so the vault does not fill with untitled-n.
    app.new_document().unwrap();
    assert_eq!(app.doc.path.as_deref(), Some(path.as_path()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_scratch_document_that_got_text_stays() {
    let dir = vault("scratch-used");
    let mut app = app_in(&dir);
    app.new_document().unwrap();
    let path = app.doc.path.clone().unwrap();

    // Text only in the buffer: still the user's, keep the reservation.
    app.doc.editor.insert_text("hello");
    app.discard_scratch();
    assert!(path.is_file());

    // Text on disk, empty buffer: autosave wrote something earlier.
    app.doc.editor = Editor::new();
    std::fs::write(&path, "hello\n").unwrap();
    app.discard_scratch();
    assert!(path.is_file());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reopening_riff_with_a_disc_goes_straight_to_the_player() {
    let dir = vault("riff-reopen");
    let mut app = app_in(&dir);
    let index = PROGRAMS.iter().position(|p| p.id == "riff").expect("known");

    app.pending = Some(Pending::Program(index));
    app.finish_loading();
    assert_eq!(app.home, Home::Riff);
    assert_eq!(app.mode, Mode::Action, "an empty tray shows the front door");

    app.riff.playlist = music::Playlist::load(vec![music::Track::from_path(dir.join("01.wav"))], 0);
    app.pending = Some(Pending::Program(index));
    app.finish_loading();
    assert_eq!(app.mode, Mode::Riff, "the disc is still in: the player");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn riff_says_what_it_is_missing_and_insert_only_opens_inside_riff() {
    let dir = vault("riff-empty");
    let mut app = app_in(&dir);
    app.prefs.riff_last.clear();
    // Not installed yet: the deck refuses to even talk.
    assert!(app.run_riff(RiffAction::Status).is_err());
    app.prefs.installed.push("riff".into());
    assert!(app.riff_status().contains("no disc"));
    let error = app.run_riff(RiffAction::Play(None)).unwrap_err();
    assert!(error.contains("no music"), "{error}");
    let error = app.run_riff(RiffAction::Insert).unwrap_err();
    assert!(error.contains("open Riff"), "{error}");

    app.home = Home::Riff;
    app.mode = Mode::Action;
    assert!(app.run_riff(RiffAction::Insert).is_ok());
    assert_eq!(app.mode, Mode::RiffInsert);
    assert!(app.riff.explorer.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn inserting_takes_the_highlighted_folder_not_just_the_open_one() {
    let dir = vault("riff-target");
    let music = dir.join("music");
    std::fs::create_dir_all(music.join("Album")).unwrap();
    std::fs::write(music.join("loose.wav"), b"x").unwrap();

    let mut app = app_in(&dir);
    let mut browser = Browser::music(music.clone());
    browser.refresh().unwrap();
    app.riff.explorer = Some(browser);
    assert_eq!(
        app.explorer_insert_target(),
        Some(music.join("Album")),
        "the cursor is on a folder"
    );

    // On a track row, the folder itself is inserted.
    app.riff.explorer.as_mut().unwrap().selected = 1;
    assert_eq!(app.explorer_insert_target(), Some(music));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_explorer_walks_folders_without_leaving_music() {
    let dir = vault("riff-explorer");
    let music = dir.join("music");
    std::fs::create_dir_all(music.join("Album")).unwrap();
    std::fs::write(music.join("Album/01.wav"), b"x").unwrap();
    std::fs::write(music.join("loose.wav"), b"x").unwrap();
    std::fs::write(music.join("cover.png"), b"x").unwrap();

    let mut app = app_in(&dir);
    app.home = Home::Riff;
    app.mode = Mode::Action;
    let mut browser = Browser::music(music.clone());
    browser.refresh().unwrap();
    app.riff.explorer = Some(browser);
    app.mode = Mode::RiffInsert;
    assert_eq!(
        app.riff.explorer.as_ref().unwrap().entries.len(),
        2,
        "the album and one track, no '..' at the music root"
    );
    assert!(browser::is_folder(
        &app.riff.explorer.as_ref().unwrap().entries[0].1
    ));
    assert!(
        matches!(
            app.riff.explorer.as_ref().unwrap().entries[1].1,
            Action::PlayTrack(_)
        ),
        "a playable file becomes a track, not an open action"
    );

    app.explorer_enter();
    assert!(app.riff.explorer.as_ref().unwrap().dir().ends_with("Album"));
    assert_eq!(
        app.riff.explorer.as_ref().unwrap().entries.len(),
        2,
        "'..' and the track inside"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_arrow_keys_move_the_music_list_not_the_vault() {
    let dir = vault("riff-move");
    let music = dir.join("music");
    std::fs::create_dir_all(music.join("Album")).unwrap();
    std::fs::write(music.join("loose.wav"), b"x").unwrap();
    std::fs::write(music.join("zeta.wav"), b"x").unwrap();

    let mut app = app_in(&dir);
    app.mode = Mode::RiffInsert;
    let mut browser = Browser::music(music.clone());
    browser.refresh().unwrap();
    app.riff.explorer = Some(browser);

    // The vault's slot is a different browser and must stay out of it.
    app.browser.selected = 7;
    app.explorer_move(1);
    assert_eq!(app.riff.explorer.as_ref().unwrap().selected, 1);
    assert_eq!(
        app.browser.selected, 7,
        "the music list has its own highlight"
    );

    app.explorer_move(1);
    assert_eq!(app.riff.explorer.as_ref().unwrap().selected, 2);
    app.explorer_move(1);
    assert_eq!(
        app.riff.explorer.as_ref().unwrap().selected,
        2,
        "clamped at the end"
    );
    app.explorer_move(-5);
    assert_eq!(app.riff.explorer.as_ref().unwrap().selected, 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_launch_bar_is_lumpy_but_never_goes_back() {
    assert_eq!(boot_progress(0.0), 0.0);
    assert_eq!(
        boot_progress(BOOT_SECONDS * 0.92),
        1.0,
        "full before the end"
    );
    assert_eq!(boot_progress(BOOT_SECONDS * 1.5), 1.0, "and stays full");

    // It holds still at times: that pause is the whole point.
    assert_eq!(
        boot_progress(BOOT_SECONDS * 0.18),
        boot_progress(BOOT_SECONDS * 0.28),
        "a seek pause"
    );
    // Then jumps to the next step in one frame.
    assert!(
        boot_progress(BOOT_SECONDS * 0.34) > boot_progress(BOOT_SECONDS * 0.32),
        "the bar moves in jumps"
    );

    // Monotonic, and never past 1.
    let mut last = 0.0;
    for step in 0..=400 {
        let value = boot_progress(step as f32 * BOOT_SECONDS / 400.0);
        assert!(value >= last, "went backwards at {step}");
        assert!((0.0..=1.0).contains(&value));
        last = value;
    }
}

#[test]
fn leaving_software_leaves_the_row_highlighted() {
    let dir = vault("software-cursor");
    let mut app = app_in(&dir);
    // Nothing installed: Inspiro, the rule, SOFTWARE, DOCUMENTS.
    app.boot_selected = 2;
    assert_eq!(app.boot_items()[2], BootItem::Software);

    app.open_software();
    assert_eq!(app.mode, Mode::Software);
    app.to_write();
    assert_eq!(app.mode, Mode::Action, "back on the boot screen");
    assert_eq!(app.boot_selected, 2, "the cursor stays on SOFTWARE");

    app.compose();
    let text: Vec<String> = (0..app.screen.rows())
        .map(|y| {
            (0..app.screen.cols())
                .map(|x| app.screen.get(x, y).unwrap().ch)
                .collect()
        })
        .collect();
    let row = text
        .iter()
        .position(|line| line.contains("SOFTWARE"))
        .expect("SOFTWARE row");
    let highlighted = (0..app.screen.cols()).any(|x| {
        app.screen
            .get(x, row as u32)
            .is_some_and(|cell| cell.bg == app.theme.fg)
    });
    assert!(highlighted, "row {row} lost its cursor: {:?}", text[row]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn installing_keeps_the_cursor_on_the_row_it_was_on() {
    let dir = vault("software-cursor-moved");
    let mut app = app_in(&dir);
    // The boot screen with the cursor on SOFTWARE.
    app.boot_selected = 2;
    assert_eq!(app.boot_items()[2], BootItem::Software);

    // Install Softhand from the shelf: a row is inserted above SOFTWARE.
    app.open_software();
    app.software_selected = 1;
    app.software_activate();
    app.finish_loading();
    // Still on the shelf, and the cursor there is unaffected.
    assert_eq!(app.mode, Mode::Software);

    app.to_write();
    assert_eq!(
        app.boot_items().get(app.boot_selected),
        Some(&BootItem::Software),
        "the cursor followed the row, not the index"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn software_installs_and_removes_modules() {
    let dir = vault("software");
    let mut app = app_in(&dir);
    // Fresh: only the system and the shelf.
    let labels =
        |app: &App| -> Vec<String> { app.boot_rows().into_iter().map(|row| row.label).collect() };
    assert_eq!(
        labels(&app),
        vec![
            "A: INSPIRO",
            "",
            "SOFTWARE",
            "THE VAULT",
            "CALENDAR",
            "SETTINGS"
        ]
    );

    // Row 0 is Inspiro, the system: it refuses to go.
    app.software_selected = 0;
    app.software_activate();
    assert_eq!(app.status, "Inspiro is the system - it stays");
    assert_eq!(app.mode, Mode::Action, "no bar for a refusal");

    // Enter on Softhand starts the bar; the bar is what finishes the job.
    app.software_selected = 1;
    app.software_activate();
    assert_eq!(app.mode, Mode::Loading);
    assert!(!app.is_installed("softhand"), "not until the bar is done");
    app.finish_loading();
    assert_eq!(app.mode, Mode::Software, "back to the shelf");
    assert!(app.is_installed("softhand"));
    assert_eq!(app.prefs.installed, vec!["softhand"]);
    assert_eq!(
        labels(&app),
        vec![
            "A: INSPIRO",
            "B: SOFTHAND",
            "",
            "SOFTWARE",
            "THE VAULT",
            "CALENDAR",
            "SETTINGS"
        ]
    );

    // And removing it takes the row away again.
    app.software_selected = 1;
    app.software_activate();
    app.finish_loading();
    assert!(!app.is_installed("softhand"));
    assert_eq!(app.prefs.installed.len(), 0);
    assert_eq!(
        labels(&app),
        vec![
            "A: INSPIRO",
            "",
            "SOFTWARE",
            "THE VAULT",
            "CALENDAR",
            "SETTINGS"
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn drive_letters_follow_the_order_modules_were_installed() {
    let dir = vault("letters");
    let mut app = app_in(&dir);
    // Riff first, then Softhand: Riff is B, Softhand is C.
    app.prefs.installed = vec!["riff".into(), "softhand".into()];
    let labels: Vec<String> = app.boot_rows().into_iter().map(|row| row.label).collect();
    assert_eq!(
        labels,
        vec![
            "A: INSPIRO",
            "B: RIFF",
            "C: SOFTHAND",
            "",
            "SOFTWARE",
            "THE VAULT",
            "CALENDAR",
            "SETTINGS"
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_calendar_is_centred_on_a_large_grid() {
    let dir = vault("calendar-centre");
    let mut app = app_in(&dir);
    app.open_calendar();
    app.screen = Screen::new(160, 50);
    app.compose();
    assert_eq!(app.screen.get(0, 0).unwrap().ch, ' ');
    assert_eq!(
        app.screen.get(41, 12).unwrap().ch,
        'C',
        "the stage starts at (40, 12)"
    );

    app.new_event_form();
    app.screen = Screen::new(160, 50);
    app.compose();
    assert_eq!(
        app.screen.get(41, 12).unwrap().ch,
        'C',
        "the form uses the same centred stage"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_calendar_outlines_the_focused_panel() {
    let dir = vault("calendar-focus-frame");
    let mut app = app_in(&dir);
    app.open_calendar();
    app.screen = Screen::new(80, 25);
    app.compose();
    assert_eq!(app.screen.get(0, 1).unwrap().fg, app.theme.accent);
    assert_eq!(app.screen.get(0, 11).unwrap().fg, app.theme.dim);

    app.calendar.focus = super::calendar::CalendarFocus::List;
    app.compose();
    assert_eq!(app.screen.get(0, 1).unwrap().fg, app.theme.dim);
    assert_eq!(app.screen.get(0, 11).unwrap().fg, app.theme.accent);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn calendar_loads_the_month_and_the_day_list() {
    let dir = vault("calendar");
    let cal_dir = storage::calendar_in(&dir);
    std::fs::create_dir_all(&cal_dir).unwrap();
    let today = chrono::Local::now().date_naive();
    let event = cread_core::calendar::Event {
        title: "Doctor".into(),
        date: today,
        time: Some(chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap()),
        remind: Some(15),
        repeat: cread_core::calendar::Repeat::Never,
        deleted: false,
        notes: String::new(),
    };
    std::fs::write(cal_dir.join("doctor.md"), event.to_markdown()).unwrap();

    let mut app = app_in(&dir);
    app.open_calendar();
    assert_eq!(app.mode, Mode::Calendar);
    assert!(
        app.calendar
            .events
            .iter()
            .any(|entry| entry.event.title == "Doctor")
    );
    app.compose();
    let text = rows_as_text(&app.screen);
    assert!(text.contains("CALENDAR"));
    assert!(text.contains("Doctor"));
    assert!(text.contains("10:00"));
    assert!(text.contains("[GRID]"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn all_day_appointments_sort_before_timed_ones() {
    let dir = vault("calendar-all-day-first");
    let mut app = app_in(&dir);
    app.open_calendar();
    let today = chrono::Local::now().date_naive();
    let last_week = today - chrono::Duration::days(7);
    let timed = cread_core::calendar::Event {
        title: "Timed".into(),
        date: last_week,
        time: chrono::NaiveTime::from_hms_opt(9, 0, 0),
        remind: None,
        repeat: cread_core::calendar::Repeat::Weekly,
        deleted: false,
        notes: String::new(),
    };
    let all_day = cread_core::calendar::Event {
        title: "All day".into(),
        date: today,
        time: None,
        remind: None,
        repeat: cread_core::calendar::Repeat::Never,
        deleted: false,
        notes: String::new(),
    };
    app.calendar.events = vec![
        cread_core::calendar::Entry {
            path: dir.join("timed.md"),
            event: timed,
        },
        cread_core::calendar::Entry {
            path: dir.join("all-day.md"),
            event: all_day,
        },
    ];
    app.compose();
    let text = rows_as_text(&app.screen);
    let all_day = text.find("All day").expect("all-day title");
    let timed = text.find("09:00").expect("timed title");
    assert!(all_day < timed, "all-day row must come first:\n{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unsaved_calendar_form_is_marked_and_warns_before_discarding() {
    let dir = vault("calendar-unsaved");
    let mut app = app_in(&dir);
    app.open_calendar();
    app.new_event_form();
    {
        let form = app.calendar_form.as_mut().unwrap();
        form.title = "Draft".into();
        form.dirty = true;
    }
    app.compose();
    assert!(rows_as_text(&app.screen).contains("UNSAVED"));
    app.calendar_form.as_mut().unwrap().confirm_discard = true;
    app.compose();
    assert!(rows_as_text(&app.screen).contains("DISCARD EVENT?"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn calendar_edit_shows_a_visible_cursor() {
    let dir = vault("calendar-cursor");
    let mut app = app_in(&dir);
    app.open_calendar();
    app.new_event_form();
    {
        let form = app.calendar_form.as_mut().unwrap();
        form.title = "Abc".into();
        form.editing = true;
    }
    app.compose();
    let text = rows_as_text(&app.screen);
    assert!(text.contains('\u{2588}'), "the block cursor: {text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn calendar_form_writes_a_markdown_event() {
    let dir = vault("calendar-form");
    let mut app = app_in(&dir);
    app.open_calendar();
    app.new_event_form();
    let today = chrono::Local::now().date_naive();
    {
        let form = app.calendar_form.as_mut().unwrap();
        form.title = "Checkup".into();
        form.date = today.format("%Y-%m-%d").to_string();
        form.time = "10:30".into();
        form.repeat = "weekly".into();
        form.remind = "10".into();
    }
    app.save_event_form();
    assert_eq!(app.mode, Mode::Calendar);
    assert!(app.calendar_form.is_none());
    let entries = cread_core::calendar::load_events(&storage::calendar_in(&dir));
    let saved = entries
        .iter()
        .find(|entry| entry.event.title == "Checkup")
        .expect("the event file");
    assert_eq!(saved.event.time, chrono::NaiveTime::from_hms_opt(10, 30, 0));
    assert_eq!(saved.event.remind, Some(10));
    assert_eq!(saved.event.repeat, cread_core::calendar::Repeat::Weekly);
    assert!(app.unsynced);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleting_a_calendar_event_leaves_a_tombstone() {
    let dir = vault("calendar-delete");
    let cal_dir = storage::calendar_in(&dir);
    std::fs::create_dir_all(&cal_dir).unwrap();
    let today = chrono::Local::now().date_naive();
    let event = cread_core::calendar::Event {
        title: "Old".into(),
        date: today,
        time: None,
        remind: None,
        repeat: cread_core::calendar::Repeat::Never,
        deleted: false,
        notes: String::new(),
    };
    let path = cal_dir.join("old.md");
    std::fs::write(&path, event.to_markdown()).unwrap();

    let mut app = app_in(&dir);
    app.open_calendar();
    app.calendar.selected = today;
    app.calendar.selected_event = 0;
    app.delete_selected_event();
    let entries = cread_core::calendar::load_events(&cal_dir);
    assert!(entries.iter().any(|entry| entry.event.deleted));
    assert!(
        !entries
            .iter()
            .any(|entry| entry.event.title == "Old" && !entry.event.deleted)
    );
    assert!(app.unsynced);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_due_calendar_reminder_opens_in_front() {
    let dir = vault("calendar-reminder");
    let mut app = app_in(&dir);
    app.open_calendar();
    app.beep_on = false;
    let now = chrono::Local::now();
    app.reminder = Some(ReminderDue {
        at: now.naive_local() - chrono::Duration::minutes(1),
        key: "test".into(),
        title: "Doctor".into(),
        when: "today".into(),
        date: now.date_naive(),
        path: std::path::PathBuf::new(),
    });
    app.pump_reminders();
    assert_eq!(app.mode, Mode::Reminder);
    assert_eq!(app.reminder_title, "Doctor");
    assert!(app.reminder_fired.contains("test"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_fired_weekly_reminder_advances_to_next_week() {
    let dir = vault("calendar-weekly-reminder");
    let mut app = app_in(&dir);
    app.open_calendar();
    let at = chrono::Local::now().naive_local() + chrono::Duration::hours(2);
    let path = dir.join("Calendar/weekly.md");
    let event = cread_core::calendar::Event {
        title: "Weekly".into(),
        date: at.date(),
        time: Some(at.time()),
        remind: Some(0),
        repeat: cread_core::calendar::Repeat::Weekly,
        deleted: false,
        notes: String::new(),
    };
    let entry = cread_core::calendar::Entry {
        path: path.clone(),
        event,
    };
    let key = format!("{}|{}", path.display(), at.format("%Y-%m-%d %H:%M"));
    app.calendar.events = vec![entry];
    app.reminder_fired.insert(key);
    app.schedule_reminder();
    assert_eq!(
        app.reminder.as_ref().map(|due| due.at),
        Some(at + chrono::Duration::weeks(1))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_reminder_opened_over_the_calendar_can_leave_the_calendar() {
    let dir = vault("calendar-reminder-back");
    let mut app = app_in(&dir);
    app.open_calendar();
    let back = app.calendar_back;
    app.reminder_back = Mode::Calendar;
    app.open_calendar_at(chrono::Local::now().date_naive(), dir.join("event.md"));
    app.leave_calendar();
    assert_ne!(app.mode, Mode::Calendar);
    assert_ne!(app.mode, Mode::CalendarEdit);
    assert_eq!(app.mode, back);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn american_poker_opens_on_its_own_front_door_and_returns_there() {
    let dir = vault("poker-front");
    let mut app = app_in(&dir);
    let index = PROGRAMS
        .iter()
        .position(|program| program.id == "poker")
        .expect("the poker module");

    app.pending = Some(Pending::Program(index));
    app.finish_loading();
    assert_eq!(app.home, Home::Poker);
    assert_eq!(app.mode, Mode::Action, "the logo is the front door");

    app.start_poker();
    assert_eq!(app.program_id, Some("poker"));
    assert_eq!(app.mode, Mode::Program);

    app.leave_poker();
    assert!(app.program.is_none());
    assert_eq!(app.home, Home::Poker);
    assert_eq!(app.mode, Mode::Action);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scripture_is_installable_and_opens_from_its_cover() {
    let dir = vault("scripture-front");
    let mut app = app_in(&dir);
    let index = PROGRAMS
        .iter()
        .position(|program| program.id == "scripture")
        .expect("the scripture module");
    let row = app
        .software_rows()
        .into_iter()
        .find(|row| row.name == "SCRIPTURE")
        .expect("SCRIPTURE on the shelf");
    assert_eq!(row.value, "install");
    app.prefs.installed.push("scripture".into());
    assert!(
        app.boot_items()
            .iter()
            .any(|item| matches!(item, BootItem::Program(i) if *i == index)),
        "the installed module appears on the boot screen"
    );

    app.pending = Some(Pending::Program(index));
    app.finish_loading();
    assert_eq!(app.home, Home::Scripture);
    assert_eq!(app.mode, Mode::Action, "the logo is the front door");

    app.compose();
    assert!(app.scripture_at.is_some(), "the cover logo is placed");
    assert!(rows_as_text(&app.screen).contains("PRESS ENTER TO READ"));

    app.start_scripture();
    assert_eq!(app.program_id, Some("scripture"));
    assert_eq!(app.mode, Mode::Program);
    app.compose();
    let text = rows_as_text(&app.screen);
    assert!(text.contains("SCRIPTURE"));
    assert!(text.contains("GENESIS 1/50"));

    app.leave_scripture();
    assert!(app.program.is_none());
    assert_eq!(app.home, Home::Boot, "exit goes to the welcome screen");
    assert_eq!(app.mode, Mode::Action);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_quick_note_is_saved_in_the_notes_room() {
    let dir = vault("note-save");
    let mut app = app_in(&dir);
    app.open_note().unwrap();
    app.note
        .as_mut()
        .unwrap()
        .editor
        .insert_text("Call 555-0123");
    app.save_note().unwrap();
    assert!(app.note.is_none());
    assert!(app.unsynced);
    let notes = storage::notes_in(&dir);
    let files: Vec<_> = std::fs::read_dir(&notes)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(std::fs::read_to_string(&files[0]).unwrap(), "Call 555-0123");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_selection_in_the_quick_note_is_drawn_inverted() {
    let dir = vault("note-selection");
    let mut app = app_in(&dir);
    app.open_note().unwrap();
    let note = app.note.as_mut().unwrap();
    note.editor.insert_text("ab");
    note.editor.move_left(true);

    let mut screen = Screen::new(80, 25);
    app.draw_note(&mut screen);
    // The 42x12 sheet is centred: text starts one cell inside its frame.
    let (x, y) = ((80 - 42) / 2 + 1, (25 - 12) / 2 + 1);
    let (paper, ink) = (app.theme.bg, app.theme.fg);
    let a = screen.get(x, y).unwrap();
    assert_eq!((a.ch, a.fg, a.bg), ('a', ink, paper), "unselected");
    let b = screen.get(x + 1, y).unwrap();
    assert_eq!((b.ch, b.fg, b.bg), ('b', paper, ink), "selected");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_note_is_discarded_without_a_file() {
    let dir = vault("note-empty");
    let mut app = app_in(&dir);
    app.open_note().unwrap();
    app.save_note().unwrap();
    assert!(app.note.is_none());
    assert!(!storage::notes_in(&dir).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_software_screen_lists_the_system_and_the_modules() {
    let dir = vault("software-rows");
    let mut app = app_in(&dir);
    let rows = app.software_rows();
    assert_eq!(rows[0].name, "INSPIRO");
    assert_eq!(rows[0].value, "system");
    let softhand = rows.iter().find(|row| row.name == "SOFTHAND").unwrap();
    assert_eq!(softhand.value, "install");
    app.prefs.installed.push("softhand".into());
    let rows = app.software_rows();
    let softhand = rows.iter().find(|row| row.name == "SOFTHAND").unwrap();
    assert_eq!(softhand.value, "installed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_arrow_keys_skip_the_rule() {
    let dir = vault("boot-nav");
    let mut app = app_in(&dir);
    assert_eq!(app.boot_selected, 0, "starts on INSPIRO");
    app.boot_step(1);
    assert_eq!(app.boot_selected, 2, "over the rule, onto SOFTWARE");
    app.boot_step(1);
    assert_eq!(app.boot_selected, 3, "onto THE VAULT");
    app.boot_step(1);
    assert_eq!(app.boot_selected, 4, "onto CALENDAR");
    app.boot_step(1);
    assert_eq!(app.boot_selected, 5, "onto SETTINGS");
    app.boot_step(1);
    assert_eq!(app.boot_selected, 5, "the end is the end");
    app.boot_step(-1);
    assert_eq!(app.boot_selected, 4);
    app.boot_step(-1);
    assert_eq!(app.boot_selected, 3);
    app.boot_step(-1);
    assert_eq!(app.boot_selected, 2);
    app.boot_step(-1);
    assert_eq!(app.boot_selected, 0, "back over the rule");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn help_shows_the_commands_of_the_screen_you_are_on() {
    let dir = vault("help-scope");
    let mut app = app_in(&dir);
    assert_eq!(app.help_scope(), command::Scope::Boot);

    app.home = Home::Inspiro;
    assert_eq!(app.help_scope(), command::Scope::Editor);

    app.home = Home::SoftHand;
    assert_eq!(app.help_scope(), command::Scope::Program);

    app.new_picture();
    assert_eq!(app.help_scope(), command::Scope::Program);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exit_leaves_a_program_and_the_boot_screen_is_next() {
    let dir = vault("leave-program");
    let mut app = app_in(&dir);
    app.home = Home::SoftHand;
    app.new_picture();
    assert_eq!(app.mode, Mode::Program);
    app.leave_to_boot();
    assert!(app.program.is_none());
    assert_eq!(app.mode, Mode::Action);
    assert_eq!(app.home, Home::Boot);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_click_on_the_canvas_reaches_the_program() {
    let dir = vault("pointer");
    let mut app = app_in(&dir);
    app.new_picture();
    let vp = Viewport::compute(1280, 800, ScaleMode::DEFAULT).unwrap();
    let scale = vp.scale as f64;
    app.vp = Some(vp);
    // A pointer position in real pixels lands on the raster pixel under it,
    // wherever the stage sits: the window origin is the stage origin.
    let (ox, oy) = app.program_origin();
    assert_eq!(ox, 0, "a 640-wide stage needs no horizontal offset");
    app.pointer = ((320.0 + ox as f64) * scale, (200.0 + oy as f64) * scale);
    assert_eq!(app.raster_point(), (320, 200));
    assert!(app.program_pointer(PointerKind::Down(cread_core::program::BUTTON_LEFT)));
    let canvas = app.program.as_ref().unwrap().picture().unwrap();
    assert_eq!(canvas.get(320, 200), Some(0), "pencil writes the colour");
    assert_eq!(canvas.get(10, 10), Some(cread_core::PAPER));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_pointer_is_ignored_while_the_prompt_is_open() {
    let dir = vault("pointer-prompt");
    let mut app = app_in(&dir);
    app.new_picture();
    app.program_prompt = true;
    app.vp = Viewport::compute(1280, 800, ScaleMode::DEFAULT);
    app.pointer = (200.0, 200.0);
    assert!(!app.program_pointer(PointerKind::Down(cread_core::program::BUTTON_LEFT)));
    let canvas = app.program.as_ref().unwrap().picture().unwrap();
    assert_eq!(canvas.get(100, 100), Some(cread_core::PAPER));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_a_picture_writes_a_png_next_to_the_documents() {
    let dir = vault("save-picture");
    let mut app = app_in(&dir);
    app.new_picture();
    let message = app.save_picture(None).unwrap();
    assert!(message.starts_with("saved picture"), "{message}");
    let files = picture::list(&dir).unwrap();
    assert_eq!(files.len(), 1);
    let back = picture::load(&files[0]).unwrap();
    assert_eq!(back.width(), CANVAS_W);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_picture_picker_lists_saved_pictures() {
    let dir = vault("pictures");
    let mut app = app_in(&dir);
    app.home = Home::SoftHand;
    app.open_pictures();
    assert_eq!(app.mode, Mode::Browser);
    assert!(app.browser.entries.is_empty(), "nothing saved yet");

    app.new_picture();
    app.save_picture(None).unwrap();
    app.leave_to_boot();
    app.home = Home::SoftHand;
    app.open_pictures();
    assert_eq!(app.browser.entries.len(), 1);
    assert!(matches!(app.browser.entries[0].1, Action::OpenPicture(_)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_offers_an_update_check() {
    let dir = vault("update-row");
    let app = app_in(&dir);
    let rows = app.setup_rows();
    let row = |name: &str| {
        rows.iter()
            .position(|row| row.name == name)
            .unwrap_or_else(|| panic!("no {name} row"))
    };
    assert!(rows.iter().any(|row| row.rule), "groups are separated");
    assert_eq!(rows[row("UPDATE")].value, ui::VERSION);
    assert!(row("INFO") < row("UPDATE"));
    assert!(row("UPDATE") < row("SOFTWARE"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_has_the_screensaver_switches() {
    let dir = vault("screensaver-rows");
    let mut app = app_in(&dir);
    let index = |app: &App, name: &str| {
        app.setup_rows()
            .iter()
            .position(|row| row.name == name)
            .unwrap_or_else(|| panic!("no {name} row"))
    };
    assert_eq!(app.setup_rows()[index(&app, "SCREENSAVER")].value, "on");
    assert_eq!(app.setup_rows()[index(&app, "IDLE")].value, "5 min");

    app.setup_selected = index(&app, "SCREENSAVER");
    app.setup_activate();
    assert!(!app.screensaver_on);
    assert_eq!(app.setup_rows()[index(&app, "SCREENSAVER")].value, "off");

    app.setup_selected = index(&app, "IDLE");
    app.setup_activate();
    assert_eq!(app.screensaver_minutes, 10);
    assert_eq!(app.setup_rows()[index(&app, "IDLE")].value, "10 min");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn programs_are_centred_on_a_larger_framebuffer() {
    let dir = vault("stage");
    let mut app = app_in(&dir);

    // A 160x50 grid is what a 100%-scaled 3072x1920 panel ends up with.
    assert_eq!(app.program_stage(160, 50), (40, 12), "a game is centred");
    app.new_picture();
    assert_eq!(
        app.program_stage(160, 50),
        (40, 13),
        "Softhand sits below its bar"
    );
    assert_eq!(app.program_stage(80, 25), (0, 1), "the classic grid");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_pointer_follows_the_centred_stage() {
    let dir = vault("stage-pointer");
    let mut app = app_in(&dir);
    app.new_picture();
    let vp = Viewport::compute(3072, 1920, ScaleMode::DEFAULT).unwrap();
    app.vp = Some(vp);
    let (ox, oy) = app.program_origin();
    assert!(ox > 0, "the stage is not in the corner: {ox}");
    assert!(oy > 0);

    let scale = vp.scale as f64;
    app.pointer = (
        vp.offset_x as f64 + (ox as f64 + 10.0) * scale,
        vp.offset_y as f64 + (oy as f64 + 10.0) * scale,
    );
    assert_eq!(app.raster_point(), (10, 10));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_settings_bar_has_no_subtitle() {
    let dir = vault("settings-bar");
    let mut app = app_in(&dir);
    app.open_setup();
    app.compose();
    let bar = rows_as_text(&app.screen)
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert!(bar.contains("SETTINGS"), "{bar:?}");
    assert!(!bar.contains("machine"), "no subtitle: {bar:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_game_prompt_offers_only_what_a_game_can_do() {
    let dir = vault("boi-prompt");
    let mut app = app_in(&dir);
    app.program = Some(Box::new(cread_pocketboi::Pocketboi::new(&dir)));
    app.program_id = Some("pocketboi");
    app.mode = Mode::Program;
    app.program_prompt = true;
    app.command.clear();
    app.compose();

    let text = rows_as_text(&app.screen);
    assert!(text.contains("mute") && text.contains("exit"), "{text}");
    assert!(
        !text.contains("save   open"),
        "a game has nothing to save: {text}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn opening_a_document_from_a_program_hands_the_screen_to_the_editor() {
    let dir = vault("open-from-program");
    std::fs::create_dir_all(dir.join("Documents")).unwrap();
    std::fs::write(dir.join("Documents/notiz.md"), "hallo").unwrap();

    let mut app = app_in(&dir);
    app.program = Some(Box::new(cread_pocketboi::Pocketboi::new(&dir)));
    app.program_id = Some("pocketboi");
    app.mode = Mode::Program;

    app.start_loading(Pending::Document(dir.join("Documents/notiz.md")));
    app.finish_loading();

    assert!(app.program.is_none(), "the game is not in front any more");
    assert!(app.program_id.is_none());
    assert_eq!(app.mode, Mode::Write);
    assert!(app.has_document);
    assert_eq!(app.doc.editor.text(), "hallo");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn info_returns_to_settings() {
    let dir = vault("info-back");
    let mut app = app_in(&dir);
    app.open_setup();
    app.setup_selected = app
        .setup_entries()
        .iter()
        .position(|(row, _)| row.name == "INFO")
        .expect("the info row");
    app.setup_activate();
    assert_eq!(app.mode, Mode::Help);
    assert!(app.help_about);

    app.leave_help();
    assert_eq!(app.mode, Mode::Setup, "back where INFO was opened");

    // `about` from the prompt still returns to the work it covered.
    app.help_from_setup = false;
    app.mode = Mode::Write;
    app.has_document = true;
    app.leave_help();
    assert_eq!(app.mode, Mode::Write);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_settings_switch_the_clock_and_date_formats() {
    let dir = vault("format-rows");
    let mut app = app_in(&dir);
    let index = |app: &App, name: &str| {
        app.setup_rows()
            .iter()
            .position(|row| row.name == name)
            .unwrap_or_else(|| panic!("no {name} row"))
    };

    assert_eq!(app.setup_rows()[index(&app, "TIME")].value, "24 hours");
    assert_eq!(app.setup_rows()[index(&app, "DATE")].value, "DD.MM.YY");

    app.setup_selected = index(&app, "TIME");
    app.setup_activate();
    assert!(!app.clock_24h);
    assert_eq!(app.setup_rows()[index(&app, "TIME")].value, "12 hours");

    app.setup_selected = index(&app, "DATE");
    app.setup_activate();
    assert!(!app.date_day_first);
    assert_eq!(app.setup_rows()[index(&app, "DATE")].value, "MM/DD/YY");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_settings_arrows_skip_the_rules() {
    let dir = vault("settings-rules");
    let mut app = app_in(&dir);
    app.open_setup();
    assert_eq!(app.setup_rows()[app.setup_selected].name, "WEBDAV");

    app.setup_step(1);
    assert_eq!(app.setup_rows()[app.setup_selected].name, "THEME");
    app.setup_step(-1);
    assert_eq!(app.setup_rows()[app.setup_selected].name, "WEBDAV");

    for _ in 0..40 {
        app.setup_step(1);
    }
    assert_eq!(app.setup_rows()[app.setup_selected].name, "USER MANUAL");
    assert!(
        !app.setup_rows()[app.setup_selected].rule,
        "never lands on a rule"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn autosave_says_so_while_the_document_is_clean() {
    let dir = vault("autosave-bar");
    let mut app = app_in(&dir);
    app.new_document().unwrap();
    let bar = |app: &mut App| {
        app.compose();
        rows_as_text(&app.screen)
            .lines()
            .next()
            .unwrap()
            .to_string()
    };

    let line = bar(&mut app);
    assert!(line.contains(" saved"), "a fresh file is saved: {line}");
    assert!(
        !line.contains("autosave on"),
        "autosave is off, not just missing from the state: {line}"
    );

    app.autosave = true;
    let line = bar(&mut app);
    assert!(line.contains("autosave on"), "the mode is visible: {line}");
    assert!(!line.contains("saved"), "{line}");

    app.doc.editor.insert_text("x");
    let line = bar(&mut app);
    assert!(
        line.contains(" modified"),
        "unwritten text still warns, even with autosave on: {line}"
    );
    assert!(!line.contains("autosave on"), "{line}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_settings_show_the_disk_and_keep_the_manual_last() {
    const GIB: u64 = 1024 * 1024 * 1024;
    let dir = vault("storage-row");
    let mut app = app_in(&dir);
    app.open_setup();

    let rows = app.setup_rows();
    let storage = rows
        .iter()
        .position(|row| row.name == "STORAGE")
        .expect("the storage row");
    let network = rows
        .iter()
        .position(|row| row.name == "NETWORK")
        .expect("the network row");
    assert_eq!(storage + 1, network, "network follows storage");
    assert_eq!(rows[network + 1].name, "WLAN", "wlan follows network");
    assert_eq!(network + 2, rows.len() - 1, "just before the manual");
    assert_eq!(rows.last().unwrap().name, "USER MANUAL");
    assert!(app.space.is_some(), "the vault's filesystem is readable");

    app.space = Some(space::Space {
        total: 100 * GIB,
        used: 50 * GIB,
        free: 50 * GIB,
    });
    app.compose();
    let text = rows_as_text(&app.screen);
    assert!(text.contains("STORAGE"), "{text}");
    assert!(text.contains("50%"), "{text}");
    assert!(text.contains('\u{2588}'), "the bar has filled cells");
    assert!(text.contains('\u{2591}'), "and empty cells");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_about_screen_shows_the_machine() {
    let dir = vault("about-machine");
    let mut app = app_in(&dir);
    app.refresh_machine();
    app.help_about = true;
    app.mode = Mode::Help;

    app.compose();
    let text = rows_as_text(&app.screen);
    assert!(text.contains("CRead 0."), "the version title: {text}");
    assert!(text.contains("Network"), "the local address line: {text}");
    assert!(text.contains("Kernel") || text.contains("System"), "{text}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_settings_open_the_manual_and_esc_brings_them_back() {
    let dir = vault("manual");
    let mut app = app_in(&dir);
    app.open_setup();
    let index = app
        .setup_rows()
        .iter()
        .position(|row| row.name == "USER MANUAL")
        .expect("the manual row");
    assert_eq!(
        index,
        app.setup_rows().len() - 1,
        "the manual is the last point"
    );

    app.setup_selected = index;
    app.setup_activate();
    assert_eq!(app.mode, Mode::Manual);

    app.compose();
    let text = rows_as_text(&app.screen);
    assert!(text.contains("Contents"), "the contents are on screen");
    assert!(text.contains("Starting"), "the first point is visible");
    assert!(text.contains("USER MANUAL"), "the bar says where we are");
    let column: String = (2..app.screen.rows() - 2)
        .map(|y| app.screen.get(app.screen.cols() - 1, y).unwrap().ch)
        .collect();
    assert!(
        column.contains('\u{2588}'),
        "the manual scrolls: {column:?}"
    );

    app.scroll_manual(40);
    assert!(app.manual_top > 0, "it scrolls");

    app.leave_manual();
    assert_eq!(app.mode, Mode::Setup, "back to the settings");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn switching_sync_says_so_and_returns_to_the_form() {
    let dir = vault("sync-notice");
    let mut app = app_in(&dir);
    app.sync_cfg = SyncConfig {
        url: "https://example.com/dav".into(),
        enabled: false,
        ..SyncConfig::default()
    };
    app.open_webdav();

    app.toggle_sync();
    assert_eq!(app.mode, Mode::Notice);
    assert_eq!(app.notice_text, "SYNC ON");
    assert!(app.sync_cfg.enabled, "the switch is real");
    app.compose();
    assert!(rows_as_text(&app.screen).contains("SYNC ON"));
    app.on_key_notice();
    assert_eq!(app.mode, Mode::Webdav, "back to the form");

    app.toggle_sync();
    assert_eq!(app.notice_text, "SYNC OFF");
    assert!(!app.sync_cfg.enabled);

    // A switch without a server stays off and says why.
    app.sync_cfg.url.clear();
    app.sync_cfg.enabled = false;
    app.open_webdav();
    app.toggle_sync();
    assert_eq!(app.mode, Mode::Webdav, "the form stays open");
    assert!(!app.sync_cfg.enabled);
    assert!(app.form.note.contains("URL"), "{}", app.form.note);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_narrowest_grid_still_draws_the_screens() {
    let dir = vault("narrow-grid");
    let mut app = app_in(&dir);
    app.screen = Screen::new(40, 12);

    app.mode = Mode::Write;
    app.compose();

    app.open_manual(Mode::Write);
    app.compose();
    assert!(!app.manual_rows.is_empty());

    app.open_notice("SYNC ON", Mode::Write);
    app.compose();

    app.quit = Some(QuitPrompt {
        yes: true,
        from: Mode::Write,
    });
    app.mode = Mode::Quit;
    app.compose();

    app.open_setup();
    app.compose();
    app.help_all = true;
    app.mode = Mode::Help;
    app.compose();
    app.open_software();
    app.compose();
    app.open_webdav();
    app.compose();
    app.open_browser();
    app.compose();
    app.mode = Mode::View;
    app.compose();
    app.mode = Mode::Search;
    app.compose();
    app.mode = Mode::Loading;
    app.compose();
    app.mode = Mode::Checking;
    app.compose();
    app.start_screensaver();
    app.compose();

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shutdown_asks_first_and_can_be_cancelled() {
    let dir = vault("shutdown");
    let mut app = app_in(&dir);
    app.mode = Mode::Action;

    app.cmd_shutdown().unwrap();
    assert_eq!(app.mode, Mode::Power);
    assert_eq!(app.power.map(|prompt| prompt.yes), Some(false));

    app.compose();
    assert!(rows_as_text(&app.screen).contains("POWER OFF"));

    app.cancel_power();
    assert_eq!(app.mode, Mode::Action);
    assert!(app.power.is_none());

    app.cmd_reboot().unwrap();
    app.compose();
    assert!(rows_as_text(&app.screen).contains("REBOOT"));
    app.cancel_power();

    app.cmd_backtolinux().unwrap();
    app.compose();
    assert!(rows_as_text(&app.screen).contains("LEAVE CREAD"));
    app.cancel_power();

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn manual_contents_entries_become_jump_targets() {
    let jumps = manual_jumps();
    assert_eq!(
        jumps.len(),
        28,
        "8 quick-start steps, 15 chapters, 5 appendices"
    );
    assert_eq!(jumps[0].label, "1. Starting");
    assert!(jumps.iter().all(|jump| !jump.label.starts_with("Part")));
    assert!(jumps.iter().all(|jump| {
        MANUAL
            .lines()
            .nth(jump.target)
            .is_some_and(|line| manual_heading(line).is_some())
    }));
    let first = MANUAL.lines().nth(jumps[0].target).unwrap();
    assert!(first.contains("Step 1: Starting"), "{first}");

    let last = jumps.last().unwrap();
    assert!(last.label.contains("About this program"));
    let appendix = MANUAL.lines().nth(last.target).unwrap();
    assert!(appendix.contains("Appendix E"), "{appendix}");
}

#[test]
fn enter_jumps_the_manual_to_the_selected_contents_entry() {
    let dir = vault("manual-jump");
    let mut app = app_in(&dir);
    app.open_manual(Mode::Write);
    assert!(app.manual_nav);
    assert_eq!(app.manual_choice, 0);
    assert_eq!(
        app.manual_selection_row()
            .map(|row| app.manual_rows[row].source_line),
        Some(app.manual_jumps[0].line)
    );

    app.select_manual(1);
    assert_eq!(app.manual_choice, 1);
    app.enter_manual_selection();
    assert!(!app.manual_nav);
    assert_eq!(
        app.manual_rows[app.manual_top].source_line,
        app.manual_jumps[1].target
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_manual_returns_to_where_it_was_opened() {
    let dir = vault("manual-command");
    let mut app = app_in(&dir);
    app.mode = Mode::Write;
    app.has_document = true;

    app.open_manual(Mode::Write);
    assert_eq!(app.mode, Mode::Manual);
    app.leave_manual();
    assert_eq!(app.mode, Mode::Write);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn softhand_shows_its_tool_keys_under_the_colours() {
    let dir = vault("softhand-legend");
    let mut app = app_in(&dir);
    app.new_picture();
    app.screen = Screen::new(80, 32);
    app.compose();
    let text = rows_as_text(&app.screen);
    assert!(text.contains("B pencil"), "the legend is there: {text}");
    assert!(text.contains("E eraser"));
    assert!(text.contains("I pick"));

    app.screen = Screen::new(80, 25);
    app.compose();
    assert!(
        !rows_as_text(&app.screen).contains("B pencil"),
        "25 rows leave no space, and the palette is not covered"
    );

    app.screen = Screen::new(80, 26);
    app.compose();
    assert!(
        rows_as_text(&app.screen).contains("B pencil"),
        "one free row is enough for the legend"
    );

    app.program_prompt = true;
    app.compose();
    assert!(
        !rows_as_text(&app.screen).contains("B pencil"),
        "the prompt takes the last row when it needs it"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_long_document_has_a_scrollbar_on_the_right() {
    let dir = vault("scrollbar");
    let mut app = app_in(&dir);
    app.new_document().unwrap();
    app.doc.editor = Editor::from_text(&"line\n".repeat(60));
    app.compose();
    let column: String = (2..app.screen.rows() - 2)
        .map(|y| app.screen.get(app.screen.cols() - 1, y).unwrap().ch)
        .collect();
    assert!(column.contains('\u{2588}'), "there is a thumb: {column:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn softhand_wears_the_same_bar_as_the_editor() {
    let dir = vault("softhand-bar");
    let mut app = app_in(&dir);
    app.new_picture();
    app.vp = Some(Viewport::compute(1280, 800, ScaleMode::DEFAULT).unwrap());
    app.compose();
    let bar = rows_as_text(&app.screen)
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert!(bar.contains("untitled.png"), "{bar}");
    assert!(bar.contains("modified"), "{bar}");

    app.save_picture(None).unwrap();
    app.compose();
    let bar = rows_as_text(&app.screen)
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert!(bar.contains("saved"), "the state follows the save: {bar}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_screensaver_rests_after_the_configured_idle_time() {
    let dir = vault("screensaver-idle");
    let mut app = app_in(&dir);
    assert!(app.screensaver_on, "on out of the box");
    assert_eq!(app.screensaver_minutes, 5);
    assert!(!app.screensaver_due(), "fresh input");

    app.last_input = Instant::now() - Duration::from_secs(5 * 60 + 1);
    assert!(app.screensaver_due());
    app.start_screensaver();
    assert_eq!(app.mode, Mode::Screensaver);

    assert!(app.wake_screensaver(), "a key wakes it");
    assert_eq!(app.mode, Mode::Action);
    assert!(!app.screensaver_due(), "and the clock waits again");

    app.screensaver_on = false;
    app.last_input = Instant::now() - Duration::from_secs(60 * 60);
    assert!(!app.screensaver_due(), "switched off in settings");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_screensaver_returns_to_the_screen_it_covered() {
    let dir = vault("screensaver-back");
    let mut app = app_in(&dir);
    app.new_document().unwrap();
    app.mode = Mode::Write;

    app.start_screensaver();
    assert_eq!(app.screensaver_back, Mode::Write);
    assert!(app.wake_screensaver());
    assert_eq!(app.mode, Mode::Write, "the document, not the boot screen");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_screensaver_draws_the_clock_and_no_page() {
    let dir = vault("screensaver-draw");
    let mut app = app_in(&dir);
    app.start_screensaver();
    app.compose();

    assert!(
        app.screensaver_raster
            .pixels()
            .iter()
            .any(|index| *index != 0),
        "the clock was drawn"
    );
    assert_eq!(
        app.screensaver_raster.get(0, 0),
        Some(0),
        "and the page is black"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_restart_uses_the_path_captured_before_the_update() {
    let dir = vault("restart-path");
    let mut app = app_in(&dir);
    // `install` renames the running binary to `cread.old`; from then on
    // `/proc/self/exe` follows the inode and `current_exe()` names the old
    // file. The restart must use what was captured at startup.
    let captured = PathBuf::from("/tmp/cread-captured");
    app.self_exe = Some(captured.clone());
    assert_eq!(app.restart_target(), Some(captured));
    assert_ne!(
        app.restart_target(),
        std::env::current_exe().ok(),
        "not wherever the test binary happens to be"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn asking_for_an_update_shows_the_progress_window() {
    let dir = vault("update-window");
    let mut app = app_in(&dir);
    // Nothing listens there: the worker fails at once, no network needed.
    app.update_api = "http://127.0.0.1:1/releases".into();
    app.open_setup();
    app.setup_selected = app
        .setup_rows()
        .iter()
        .position(|row| row.name == "UPDATE")
        .expect("the update row");
    app.setup_activate();

    assert_eq!(app.mode, Mode::Checking);
    app.compose();
    let text = rows_as_text(&app.screen);
    assert!(text.contains("Checking github"), "{text}");

    // Joins the worker before the test binary goes away.
    app.updater = None;
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_update_question_names_the_version() {
    let dir = vault("update-prompt");
    let mut app = app_in(&dir);
    app.update_offer = Some(cread_update::Offer {
        version: "0.99.0".into(),
        tarball: "https://example.org/a.tar.gz".into(),
        checksum: "https://example.org/a.sha256".into(),
        signature: "https://example.org/a.minisig".into(),
    });
    app.update_status = UpdateStatus::Available("0.99.0".into());
    app.update_yes = true;
    app.mode = Mode::UpdatePrompt;

    app.compose();
    let text = rows_as_text(&app.screen);
    assert!(text.contains("UPDATE AVAILABLE"), "{text}");
    assert!(text.contains("0.99.0 is available"), "{text}");
    assert!(text.contains("[ Yes ]"), "{text}");

    // The settings row remembers what the check found.
    let rows = app.setup_rows();
    let update = rows
        .iter()
        .position(|row| row.name == "UPDATE")
        .expect("the update row");
    assert_eq!(rows[update].value, "0.99.0 available");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_starts_in_the_room_the_program_lives_in() {
    let dir = vault("open-rooms");
    std::fs::create_dir_all(dir.join("Documents/kapitel")).unwrap();
    std::fs::write(dir.join("Documents/notiz.md"), "x").unwrap();
    std::fs::create_dir_all(dir.join("Pictures")).unwrap();
    std::fs::write(dir.join("Pictures/a.png"), "x").unwrap();
    std::fs::write(dir.join("Pictures/notes.md"), "x").unwrap();

    let mut app = app_in(&dir);
    app.home = Home::Inspiro;
    app.new_document().unwrap();
    app.cmd_open(None).unwrap();

    assert_eq!(app.mode, Mode::Browser);
    assert_eq!(app.browser.root, dir.join("Documents"));
    assert_eq!(app.browser.title, "DOCUMENTS");
    let names = app.browser.items();
    assert!(names.contains(&"notiz.md".to_string()), "{names:?}");
    assert!(names.contains(&"kapitel/".to_string()), "{names:?}");
    assert!(
        !names.contains(&"+ sync now".to_string()),
        "the whole-vault row is not the editor's"
    );

    // Softhand starts in Pictures, and a text file is not a painting.
    app.close_program();
    app.has_document = false;
    app.home = Home::SoftHand;
    app.cmd_open(None).unwrap();
    assert_eq!(app.browser.root, dir.join("Pictures"));
    assert_eq!(app.browser.title, "PICTURES");
    let names = app.browser.items();
    assert!(names.contains(&"a.png".to_string()), "{names:?}");
    assert!(!names.contains(&"notes.md".to_string()), "{names:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_vault_row_opens_every_room() {
    let dir = vault("open-vault");
    std::fs::create_dir_all(dir.join("Documents")).unwrap();
    std::fs::create_dir_all(dir.join("Roms")).unwrap();

    let mut app = app_in(&dir);
    app.has_document = false;
    app.home = Home::Boot;
    app.cmd_open(None).unwrap();

    assert_eq!(app.browser.root, dir, "the whole machine");
    assert_eq!(app.browser.title, "THE VAULT");
    let names = app.browser.items();
    assert!(names.contains(&"Documents/".to_string()), "{names:?}");
    assert!(names.contains(&"Roms/".to_string()), "{names:?}");
    assert!(names.contains(&"+ sync now".to_string()), "{names:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_softhand_is_softhand() {
    let dir = vault("not-softhand");
    let mut app = app_in(&dir);
    app.home = Home::SoftHand;
    app.new_picture();
    assert!(app.in_softhand(), "the paint canvas");

    // A game opened from the vault: the front door is long gone.
    app.home = Home::Boot;
    app.program_id = Some("pocketboi");
    assert!(
        !app.in_softhand(),
        "a game must not answer to Softhand's save/open/new"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_dialog_stands_on_its_own_page() {
    let dir = vault("dialog-page");
    let mut app = app_in(&dir);

    // Over the boot screen: no drive row may peek out around the frame.
    app.home = Home::Boot;
    app.has_document = false;
    app.open_browser();
    app.compose();
    let text = rows_as_text(&app.screen);
    assert!(text.contains("THE VAULT"), "the window is there");
    assert!(!text.contains("A: INSPIRO"), "no boot row behind it");
    assert!(!text.contains("SOFTWARE"), "no shelf row behind it");

    // Over a document: the text stays out of sight as well.
    app.new_document().unwrap();
    app.doc
        .editor
        .insert_text("dieser satz darf nicht durchscheinen");
    app.open_browser();
    app.compose();
    let text = rows_as_text(&app.screen);
    assert!(text.contains("THE VAULT"));
    assert!(
        !text.contains("durchscheinen"),
        "the document is not drawn under a dialog"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_vault_browser_walks_folders_and_opens_the_right_program() {
    let dir = vault("vault-browser");
    let mut app = app_in(&dir);
    std::fs::create_dir_all(dir.join("Documents/kapitel")).unwrap();
    std::fs::write(dir.join("Documents/kapitel/eins.md"), "hi").unwrap();
    std::fs::create_dir_all(dir.join("Roms")).unwrap();
    std::fs::write(dir.join("Roms/game.gb"), b"rom").unwrap();
    std::fs::create_dir_all(dir.join("Pictures")).unwrap();
    std::fs::write(dir.join("Pictures/a.png"), b"png").unwrap();

    app.open_browser();
    let names = app.browser.items();
    assert!(names.contains(&"Documents/".to_string()), "{names:?}");
    assert!(names.contains(&"Pictures/".to_string()), "{names:?}");
    assert!(names.contains(&"Roms/".to_string()), "{names:?}");
    assert_eq!(app.browser.title, "THE VAULT");

    app.browser_enter(PathBuf::from("Roms"));
    assert_eq!(app.browser.items(), vec!["..", "game.gb"]);
    assert_eq!(app.browser.title, "THE VAULT /Roms");
    app.browser.selected = 1;
    match app.browser.current() {
        Some(Action::OpenRom(path)) => assert!(path.ends_with("Roms/game.gb")),
        _ => panic!("a cartridge should open Pocketboi"),
    }

    app.browser_enter(PathBuf::from("Documents"));
    assert_eq!(app.browser.items(), vec!["..", "kapitel/"]);
    app.browser_enter(PathBuf::from("Documents/kapitel"));
    assert_eq!(app.browser.items(), vec!["..", "eins.md"]);

    assert!(app.browser_up(), "up from a folder");
    assert_eq!(app.browser.path, PathBuf::from("Documents"));
    assert!(app.browser_up());
    assert_eq!(app.browser.path, PathBuf::new());
    assert!(!app.browser_up(), "the root is the top");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saveas_only_knows_documents() {
    let dir = vault("saveas");
    let mut app = app_in(&dir);
    app.new_document().unwrap();
    assert!(
        app.doc
            .path
            .as_ref()
            .unwrap()
            .starts_with(dir.join("Documents")),
        "a new document is born in Documents/"
    );

    app.doc.editor.insert_text("kapitel eins");
    app.save_as("kapitel/eins").unwrap();
    assert!(dir.join("Documents/kapitel/eins.md").is_file());
    assert!(!dir.join("kapitel/eins.md").exists(), "not in the root");

    // Dots and absolute paths do not escape the folder.
    app.save_as("../../ausbruch").unwrap();
    assert!(dir.join("Documents/ausbruch.md").is_file());

    // A bare name still finds a nested document.
    let found = app.find_document("eins").unwrap();
    assert_eq!(found, dir.join("Documents/kapitel/eins.md"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn loose_documents_move_in_when_the_vault_is_first_used() {
    let dir = vault("migrate-app");
    std::fs::write(dir.join("alt.md"), "von früher").unwrap();
    let mut app = app_in(&dir);
    app.prepare_vault();
    assert!(dir.join("Documents/alt.md").is_file());
    assert!(!dir.join("alt.md").exists());
    assert!(app.unsynced, "a moved file has not reached the server");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_new_document_is_announced_as_a_draft_to_sync() {
    let dir = vault("draft-marker");
    let mut app = app_in(&dir);
    app.new_document().unwrap();

    let draft = cread_sync::draft(&dir).expect("the app said which file is the draft");
    assert!(draft.starts_with("Documents/untitled"), "{draft}");
    assert!(dir.join(&draft).is_file());

    app.save_as("kapitel/eins").unwrap();
    assert!(
        cread_sync::draft(&dir).is_none(),
        "naming the document ends the draft"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_explicit_save_keeps_the_placeholder_as_a_document() {
    let dir = vault("draft-save");
    let mut app = app_in(&dir);
    app.new_document().unwrap();
    let path = app.doc.path.clone().unwrap();
    app.doc.editor.insert_text("behalten");

    app.save().unwrap();
    assert!(path.is_file(), "the file stays");
    assert!(app.scratch.is_none(), "but it is not a draft any more");
    assert!(cread_sync::draft(&dir).is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saveas_removes_the_untitled_placeholder() {
    let dir = vault("saveas-scratch");
    let mut app = app_in(&dir);
    app.autosave = true;
    app.mode = Mode::Write;
    app.new_document().unwrap();
    let scratch = app.doc.path.clone().unwrap();
    assert!(scratch.is_file(), "the placeholder exists from the start");

    // Autosave fills the placeholder, as it would while typing.
    app.doc.editor.insert_text("kapitel eins");
    app.doc.last_edit = Instant::now() - AUTOSAVE_DELAY * 2;
    app.autosave_if_due();
    assert_eq!(std::fs::read_to_string(&scratch).unwrap(), "kapitel eins");

    let message = app.save_as("kapitel/eins").unwrap();
    assert!(dir.join("Documents/kapitel/eins.md").is_file());
    assert!(!scratch.exists(), "the placeholder is gone: {message}");
    assert!(app.scratch.is_none(), "and not remembered for later");
    assert!(
        message.contains("removed"),
        "the status line says what happened: {message}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saveas_keeps_the_placeholder_when_it_cannot_write() {
    let dir = vault("saveas-fails");
    let mut app = app_in(&dir);
    app.new_document().unwrap();
    let scratch = app.doc.path.clone().unwrap();

    assert!(app.save_as(".").is_err());
    assert!(scratch.is_file(), "nothing was written, nothing removed");
    assert!(app.scratch.is_some(), "still a placeholder");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_name_that_folds_away_is_refused_not_redirected() {
    let dir = vault("fold-name");
    let mut app = app_in(&dir);
    app.new_document().unwrap();
    app.doc.editor.insert_text("x");
    for bad in [".", "..", "../.."] {
        assert!(app.save_as(bad).is_err(), "{bad:?} must not name a file");
    }
    assert!(
        !dir.join("Documents.md").exists(),
        "no document next to the folder"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rename_refuses_to_overwrite_and_stays_in_its_folder() {
    let dir = vault("rename-guard");
    let mut app = app_in(&dir);
    std::fs::create_dir_all(dir.join("Documents/kapitel")).unwrap();
    std::fs::write(dir.join("Documents/kapitel/eins.md"), "eins").unwrap();
    std::fs::write(dir.join("Documents/kapitel/belegt.md"), "belegt").unwrap();
    app.open_document(dir.join("Documents/kapitel/eins.md"))
        .unwrap();

    let err = app.rename_current("belegt").unwrap_err();
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.join("Documents/kapitel/belegt.md")).unwrap(),
        "belegt",
        "the other document is untouched"
    );
    assert!(dir.join("Documents/kapitel/eins.md").is_file());

    app.rename_current("zwei").unwrap();
    assert!(
        dir.join("Documents/kapitel/zwei.md").is_file(),
        "a rename stays in its folder"
    );
    assert!(!dir.join("Documents/zwei.md").exists());
    assert!(app.unsynced, "a rename is a local change");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleting_a_document_counts_as_a_change() {
    let dir = vault("delete-counts");
    let mut app = app_in(&dir);
    app.new_document().unwrap();
    app.doc.editor.insert_text("weg damit");
    app.save().unwrap();
    app.unsynced = false;
    app.delete_current().unwrap();
    assert!(app.unsynced, "the server still has the file");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bare_name_never_escapes_the_vault_root() {
    let dir = vault("find-escape");
    let app = app_in(&dir);
    let outside = dir
        .parent()
        .unwrap()
        .join(format!("cread-aussen-{}.md", std::process::id()));
    std::fs::write(&outside, "geheim").unwrap();

    for name in ["../cread-aussen", "../../cread-aussen"] {
        assert!(
            app.find_document(name).is_none(),
            "{name:?} must not reach outside the vault"
        );
    }
    let _ = std::fs::remove_file(&outside);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quitting_asks_before_leaving_unsynced_work() {
    let dir = vault("quit-prompt");
    let mut app = app_in(&dir);
    app.sync_cfg = SyncConfig {
        url: "https://example.org/dav".into(),
        enabled: true,
        ..SyncConfig::default()
    };
    app.sync = Some(Worker::spawn(
        dir.clone(),
        app.sync_cfg.clone(),
        Duration::from_secs(3600),
    ));
    assert!(!app.wants_sync_before_quitting(), "nothing written yet");

    app.unsynced = true;
    assert!(app.wants_sync_before_quitting());
    assert!(app.ask_quit(Mode::Action));
    assert_eq!(app.mode, Mode::Quit);
    assert_eq!(
        app.quit,
        Some(QuitPrompt {
            yes: true,
            from: Mode::Action
        })
    );

    app.compose();
    let text = rows_as_text(&app.screen);
    assert!(text.contains("SYNC TO SERVER?"), "{text}");
    assert!(text.contains("Yes") && text.contains("No"), "{text}");

    // Sync off means no question, ever.
    app.sync_cfg.enabled = false;
    app.mode = Mode::Action;
    app.quit = None;
    assert!(!app.ask_quit(Mode::Action));

    // A failed last round keeps the app open and the data unsynced.
    app.sync_cfg.url = "http://127.0.0.1:1".into();
    app.sync_cfg.enabled = true;
    assert!(app.sync_before_quit().is_err());
    assert!(app.unsynced, "nothing reached the server");
    assert!(app.sync.is_some(), "the worker is running again");

    app.sync = None;
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_on_the_front_door_shows_the_browser() {
    let dir = vault("front-door-open");
    let mut app = app_in(&dir);
    app.home = Home::Inspiro;
    app.mode = Mode::Action;
    app.compose();
    assert!(!rows_as_text(&app.screen).contains("+ new document"));

    // 'open' is a command line away, and the front door composed its own
    // screen; it used to return from compose before the browser was drawn.
    app.command = "open".into();
    app.open_browser();
    app.compose();
    let text = rows_as_text(&app.screen);
    assert!(text.contains("+ new document"), "browser is on screen");
    assert!(text.contains("Enter open"), "with its hint");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_inspiro_logo_belongs_to_its_own_screen_only() {
    let mut app = app();
    app.home = Home::Inspiro;
    app.mode = Mode::Action;
    app.compose();
    assert!(app.inspiro_at.is_some(), "front door shows its logo");
    assert!(app.splash_at.is_none(), "and not the boot logo");

    // Help, the browser and the boot screen all draw over the same grid, so
    // a stale blit position floats on top of them.
    app.mode = Mode::Help;
    app.compose();
    assert!(app.inspiro_at.is_none());
    assert!(app.splash_at.is_none());

    app.home = Home::Boot;
    app.mode = Mode::Action;
    app.compose();
    assert!(app.inspiro_at.is_none());
    assert!(
        app.splash_at.is_some(),
        "the boot screen shows the boot logo"
    );

    // An overlay hides the boot logo while it is open.
    app.mode = Mode::Browser;
    app.compose();
    assert!(app.splash_at.is_none());
}

#[test]
fn wanderlust_is_on_the_shelf_and_web_needs_it_installed() {
    let dir = vault("wanderlust-shelf");
    let mut app = app_in(&dir);
    let row = app
        .software_rows()
        .into_iter()
        .find(|row| row.name == "WANDERLUST")
        .expect("WANDERLUST on the shelf");
    assert_eq!(row.value, "install");
    let error = app.cmd_web(Some("example.com".into())).unwrap_err();
    assert!(error.contains("not installed"), "{error}");
    assert!(app.program.is_none());
    let error = app.cmd_browse(&Command::Back).unwrap_err();
    assert!(error.contains("web"), "{error}");
}

#[test]
fn wanderlust_sits_under_the_top_bar_and_names_its_trouble() {
    let dir = vault("wanderlust-open");
    let mut app = app_in(&dir);
    app.prefs.installed.push("wanderlust".into());
    let index = PROGRAMS
        .iter()
        .position(|program| program.id == "wanderlust")
        .expect("the wanderlust module");
    app.pending = Some(Pending::Program(index));
    app.finish_loading();
    assert_eq!(app.mode, Mode::Program);
    assert_eq!(app.program_id, Some("wanderlust"));
    assert_eq!(
        app.program_stage(80, 25),
        (0, 1),
        "the page starts under the bar"
    );
    assert_eq!(app.program_stage_height(), 23);

    app.screen = Screen::new(80, 25);
    app.compose();
    let text = rows_as_text(&app.screen);
    let rows: Vec<&str> = text.lines().collect();
    assert!(rows[0].contains("WANDERLUST"), "top bar: {}", rows[0]);
    assert!(
        rows[24].contains("WANDERLUST"),
        "program line: {}",
        rows[24]
    );
    // No browser in a test: the door says how to get one.
    assert!(text.contains("apt install chromium"), "{text}");

    // `web` while the browser is in front keeps the same program.
    app.cmd_web(Some("example.com".into())).unwrap();
    assert_eq!(app.program_id, Some("wanderlust"));
    assert!(app.cmd_browse(&Command::Reload).is_ok());
}

#[test]
fn the_wifi_screen_lists_networks_and_asks_for_a_password() {
    let dir = vault("wifi-screen");
    let mut app = app_in(&dir);
    // No worker in a test: the events are handed in directly.
    app.open_wifi();
    assert_eq!(app.mode, Mode::Wifi);
    let net = |ssid: &str, signal, security: &str, in_use, saved| cread_wifi::Network {
        ssid: ssid.into(),
        signal,
        security: security.into(),
        in_use,
        saved,
    };
    app.wifi_event(cread_wifi::Event::Networks(vec![
        net("Home", 80, "WPA2", true, true),
        net("Cafe", 40, "", false, false),
    ]));
    app.wifi_event(cread_wifi::Event::Link(cread_wifi::Link::Wifi {
        ssid: "Home".into(),
        signal: 80,
    }));
    assert_eq!(app.wifi_label(), "Home");
    assert_eq!(app.net_mark(), Some(cread_render::font::NET_WIFI[2]));

    app.screen = Screen::new(80, 25);
    app.compose();
    let text = rows_as_text(&app.screen);
    assert!(
        text.contains("Home") && text.contains("connected"),
        "{text}"
    );
    assert!(text.contains("Cafe") && text.contains("open"), "{text}");

    app.wifi_event(cread_wifi::Event::Link(cread_wifi::Link::Offline));
    assert_eq!(app.net_mark(), None, "offline: no mark");
    assert_eq!(app.wifi_label(), "not connected");
    app.wifi_event(cread_wifi::Event::Link(cread_wifi::Link::Wired));
    assert_eq!(app.net_mark(), Some(cread_render::font::NET_WIRED));
}

#[test]
fn the_top_bar_carries_the_network_mark() {
    let mut screen = Screen::new(80, 25);
    ui::draw_top_bar(
        &mut screen,
        &Theme::BLACK,
        &ui::TopBar {
            title: "doc.md",
            state: "saved",
            clock: "12:00",
            location: "~/Documents",
            net: Some(cread_render::font::NET_WIFI[1]),
        },
    );
    let bar = rows_as_text(&screen).lines().next().unwrap().to_string();
    let chars: Vec<char> = bar.chars().collect();
    assert_eq!(chars[78], cread_render::font::NET_WIFI[1]);
    assert!(bar.contains("~/Documents \u{e002}"), "{bar:?}");
}
