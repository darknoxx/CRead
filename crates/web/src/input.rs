//! Keys and pointer events, translated into what `Input.dispatch*` expects.

use cread_core::KeyPress;
use serde_json::{Value, json};

/// CDP's modifier bit for Control.
const CTRL: u8 = 2;

/// The `Input.dispatchKeyEvent` parameters for one key: down, then up.
///
/// Characters carry `text`, so a field types them and a page's own shortcuts
/// (`k` on a video, the space bar) see a real keydown. `None` for keys a page
/// never gets: the bare Control keys are the shell's.
pub fn key_events(key: KeyPress) -> Option<Vec<Value>> {
    let (key_name, code, vk, text, modifiers, command): (
        String,
        String,
        u32,
        Option<String>,
        u8,
        Option<&str>,
    ) = match key {
        KeyPress::Char(' ') => (" ".into(), "Space".into(), 32, Some(" ".into()), 0, None),
        KeyPress::Char(c) => {
            let (code, vk) = char_code(c);
            (c.to_string(), code, vk, Some(c.to_string()), 0, None)
        }
        KeyPress::Ctrl(c) => {
            let (code, vk) = char_code(c);
            // Editing commands have to be named, or the page sees the keys and
            // nothing happens in a text field.
            let command = match c {
                'a' => Some("selectAll"),
                'z' => Some("undo"),
                'y' => Some("redo"),
                _ => None,
            };
            (c.to_string(), code, vk, None, CTRL, command)
        }
        KeyPress::Enter => named("Enter", 13, Some("\r")),
        KeyPress::Backspace => named("Backspace", 8, None),
        KeyPress::Delete => named("Delete", 46, None),
        KeyPress::Tab => named("Tab", 9, None),
        KeyPress::Escape => named("Escape", 27, None),
        KeyPress::Left => named("ArrowLeft", 37, None),
        KeyPress::Right => named("ArrowRight", 39, None),
        KeyPress::Up => named("ArrowUp", 38, None),
        KeyPress::Down => named("ArrowDown", 40, None),
        KeyPress::Home => named("Home", 36, None),
        KeyPress::End => named("End", 35, None),
        KeyPress::PageUp => named("PageUp", 33, None),
        KeyPress::PageDown => named("PageDown", 34, None),
        KeyPress::Function(n) if (1..=12).contains(&n) => {
            let name = format!("F{n}");
            (name.clone(), name, 111 + u32::from(n), None, 0, None)
        }
        KeyPress::Function(_) | KeyPress::ControlLeft | KeyPress::ControlRight => return None,
    };

    let mut down = json!({
        "type": if text.is_some() { "keyDown" } else { "rawKeyDown" },
        "key": key_name,
        "code": code,
        "windowsVirtualKeyCode": vk,
        "nativeVirtualKeyCode": vk,
        "modifiers": modifiers,
    });
    if let Some(text) = &text {
        down["text"] = Value::from(text.as_str());
        down["unmodifiedText"] = Value::from(text.as_str());
    }
    if let Some(command) = command {
        down["commands"] = json!([command]);
    }
    let up = json!({
        "type": "keyUp",
        "key": key_name,
        "code": code,
        "windowsVirtualKeyCode": vk,
        "nativeVirtualKeyCode": vk,
        "modifiers": modifiers,
    });
    Some(vec![down, up])
}

fn named(
    name: &str,
    vk: u32,
    text: Option<&str>,
) -> (
    String,
    String,
    u32,
    Option<String>,
    u8,
    Option<&'static str>,
) {
    (
        name.into(),
        name.into(),
        vk,
        text.map(str::to_string),
        0,
        None,
    )
}

/// The physical key and Windows key code for a character, where one exists.
/// Characters a US keyboard has no key for (`ä`, `€`) still type through `text`.
fn char_code(c: char) -> (String, u32) {
    match c {
        'a'..='z' => (format!("Key{}", c.to_ascii_uppercase()), c as u32 - 32),
        'A'..='Z' => (format!("Key{c}"), c as u32),
        '0'..='9' => (format!("Digit{c}"), c as u32),
        _ => (String::new(), 0),
    }
}

/// What a pointer did, in page pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mouse {
    Move {
        x: i32,
        y: i32,
        buttons: u8,
    },
    Press {
        x: i32,
        y: i32,
        button: u8,
        clicks: u32,
        buttons: u8,
    },
    Release {
        x: i32,
        y: i32,
        button: u8,
        clicks: u32,
        buttons: u8,
    },
    /// Positive scrolls down, in pixels.
    Wheel {
        x: i32,
        y: i32,
        delta: i32,
    },
}

/// CDP's name for a winit button index (left, right, middle).
pub fn button_name(button: u8) -> &'static str {
    match button {
        0 => "left",
        1 => "right",
        2 => "middle",
        _ => "none",
    }
}

/// CDP's bit for a button in the `buttons` mask.
pub fn button_bit(button: u8) -> u8 {
    match button {
        0 => 1,
        1 => 2,
        2 => 4,
        _ => 0,
    }
}

/// The `Input.dispatchMouseEvent` parameters.
pub fn mouse_event(mouse: Mouse) -> Value {
    match mouse {
        Mouse::Move { x, y, buttons } => json!({
            "type": "mouseMoved", "x": x, "y": y, "buttons": buttons,
            "button": if buttons & 1 != 0 { "left" } else { "none" },
        }),
        Mouse::Press {
            x,
            y,
            button,
            clicks,
            buttons,
        } => json!({
            "type": "mousePressed", "x": x, "y": y, "button": button_name(button),
            "buttons": buttons, "clickCount": clicks,
        }),
        Mouse::Release {
            x,
            y,
            button,
            clicks,
            buttons,
        } => json!({
            "type": "mouseReleased", "x": x, "y": y, "button": button_name(button),
            "buttons": buttons, "clickCount": clicks,
        }),
        Mouse::Wheel { x, y, delta } => json!({
            "type": "mouseWheel", "x": x, "y": y, "deltaX": 0, "deltaY": delta,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_letter_types_its_text() {
        let events = key_events(KeyPress::Char('k')).unwrap();
        assert_eq!(events[0]["type"], "keyDown");
        assert_eq!(events[0]["text"], "k");
        assert_eq!(events[0]["code"], "KeyK");
        assert_eq!(events[0]["windowsVirtualKeyCode"], 75);
        assert_eq!(events[1]["type"], "keyUp");
    }

    #[test]
    fn umlauts_type_without_a_key_code() {
        let events = key_events(KeyPress::Char('ä')).unwrap();
        assert_eq!(events[0]["text"], "ä");
        assert_eq!(events[0]["windowsVirtualKeyCode"], 0);
    }

    #[test]
    fn enter_submits_a_form() {
        let events = key_events(KeyPress::Enter).unwrap();
        assert_eq!(events[0]["key"], "Enter");
        assert_eq!(events[0]["text"], "\r");
    }

    #[test]
    fn arrows_are_raw_keys() {
        let events = key_events(KeyPress::Down).unwrap();
        assert_eq!(events[0]["type"], "rawKeyDown");
        assert_eq!(events[0]["windowsVirtualKeyCode"], 40);
        assert!(events[0].get("text").is_none());
    }

    #[test]
    fn ctrl_a_selects_everything() {
        let events = key_events(KeyPress::Ctrl('a')).unwrap();
        assert_eq!(events[0]["modifiers"], 2);
        assert_eq!(events[0]["commands"][0], "selectAll");
    }

    #[test]
    fn the_bare_control_keys_stay_home() {
        assert!(key_events(KeyPress::ControlLeft).is_none());
    }

    #[test]
    fn a_click_names_its_button() {
        let press = mouse_event(Mouse::Press {
            x: 10,
            y: 20,
            button: 0,
            clicks: 2,
            buttons: 1,
        });
        assert_eq!(press["type"], "mousePressed");
        assert_eq!(press["button"], "left");
        assert_eq!(press["clickCount"], 2);
        let wheel = mouse_event(Mouse::Wheel {
            x: 0,
            y: 0,
            delta: 120,
        });
        assert_eq!(wheel["deltaY"], 120);
    }
}
