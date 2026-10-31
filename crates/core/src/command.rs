//! The action-mode command line.
//!
//! Parsing lives here, away from the window, so the whole vocabulary is a table
//! that both the parser and the `help` screen read from.

/// A parsed command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Save,
    SaveAs(String),
    Open(Option<String>),
    New,
    Rename(String),
    Delete,
    Sync,
    Theme(Option<String>),
    /// `Some(true/false)` sets, `None` toggles.
    /// Bare `scanlines` switches them on; `-r` adds the drift.
    Scanlines {
        on: Option<bool>,
        roll: bool,
    },
    Bloom(Option<bool>),
    /// Let the scanline pattern drift downwards.
    Roll(Option<bool>),
    Typewriter(Option<bool>),
    /// `Some(name)` picks a face, `None` cycles to the next one.
    Font(Option<String>),
    /// `true` = go fullscreen.
    Fullscreen(bool),
    Undo,
    Redo,
    /// Open the search prompt, optionally with a query already typed.
    Find(Option<String>),
    /// Pick a tool inside a program (Softhand's pencil, fill, ...).
    Tool(Option<String>),
    /// Brush size inside a program.
    Size(Option<u32>),
    /// Palette index inside a program.
    Colour(Option<u8>),
    /// Toggle reader mode.
    View,
    /// Run a diskette by name; bare `run` runs what the boot screen has
    /// selected.
    Run(Option<String>),
    /// List the diskettes in the drive folder.
    Dir,
    /// Back to the boot screen: leave whatever is open, without quitting.
    Exit,
    /// `exit!` - save and quit, from anywhere.
    ExitForce,
    Settings,
    /// `Some("all")` shows the whole vocabulary instead of this screen's.
    Help(Option<String>),
    About,
}

/// Where a command makes sense. `help` only lists the ones that belong to the
/// screen you are standing on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The boot screen: drives, disks, settings.
    Boot,
    /// The editor (Inspiro): documents and the writing screen.
    Editor,
    /// A program that owns the screen (Softhand).
    Program,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandError {
    Empty,
    Unknown(String),
    MissingArgument {
        command: &'static str,
        argument: &'static str,
    },
    BadValue {
        value: String,
        expected: &'static str,
    },
}

impl CommandError {
    /// Short, user-facing text for the prompt line.
    pub fn message(&self) -> String {
        match self {
            CommandError::Empty => String::new(),
            CommandError::Unknown(word) => format!("unknown command: {word}"),
            CommandError::MissingArgument { command, argument } => {
                format!("{command} needs {argument}")
            }
            CommandError::BadValue { value, expected } => {
                format!("{value:?} is not {expected}")
            }
        }
    }
}

/// One row of the command vocabulary.
pub struct Spec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    /// `""`, `"<name>"` (required) or `"[name]"` (optional).
    pub argument: &'static str,
    pub summary: &'static str,
    /// Screens this command belongs to. `help` reads this.
    pub scopes: &'static [Scope],
}

pub const SPECS: &[Spec] = &[
    Spec {
        name: "save",
        aliases: &["w", "safe", "write"],
        argument: "",
        summary: "write it to disk",
        scopes: &[Scope::Editor, Scope::Program],
    },
    Spec {
        name: "saveas",
        aliases: &["sa"],
        argument: "<name>",
        summary: "save under a new name in the vault",
        scopes: &[Scope::Editor, Scope::Program],
    },
    Spec {
        name: "open",
        aliases: &["o", "e"],
        argument: "[name]",
        summary: "open a document or picture (bare: list)",
        scopes: &[Scope::Editor, Scope::Program],
    },
    Spec {
        name: "new",
        aliases: &["n"],
        argument: "",
        summary: "start a new document or picture",
        scopes: &[Scope::Editor, Scope::Program],
    },
    Spec {
        name: "rename",
        aliases: &["mv", "rn"],
        argument: "<name>",
        summary: "rename the current document",
        scopes: &[Scope::Editor],
    },
    Spec {
        name: "delete",
        aliases: &["rm", "del"],
        argument: "",
        summary: "delete the current document",
        scopes: &[Scope::Editor],
    },
    Spec {
        name: "sync",
        aliases: &[],
        argument: "",
        summary: "sync the vault with WebDAV now",
        scopes: &[Scope::Boot, Scope::Editor],
    },
    Spec {
        name: "theme",
        aliases: &[],
        argument: "[name]",
        summary: "switch theme (no argument: next)",
        scopes: &[Scope::Boot, Scope::Editor, Scope::Program],
    },
    Spec {
        name: "scanlines",
        aliases: &["scan", "sl"],
        argument: "[on|off] -r",
        summary: "toggle or set the scanlines",
        scopes: &[Scope::Editor],
    },
    Spec {
        name: "bloom",
        aliases: &[],
        argument: "[on|off]",
        summary: "toggle or set the phosphor glow",
        scopes: &[Scope::Editor],
    },
    Spec {
        name: "roll",
        aliases: &["drift"],
        argument: "[on|off]",
        summary: "let the scanline pattern drift downwards",
        scopes: &[Scope::Editor],
    },
    Spec {
        name: "typewriter",
        aliases: &["tw"],
        argument: "[on|off]",
        summary: "toggle or set typewriter scrolling",
        scopes: &[Scope::Editor],
    },
    Spec {
        name: "font",
        aliases: &["weight"],
        argument: "[classic|thin]",
        summary: "face: classic VGA or thin ISO (bare: toggle)",
        scopes: &[Scope::Boot, Scope::Editor, Scope::Program],
    },
    Spec {
        name: "fullscreen",
        aliases: &["fs"],
        argument: "",
        summary: "go fullscreen",
        scopes: &[Scope::Boot, Scope::Editor, Scope::Program],
    },
    Spec {
        name: "windowed",
        aliases: &["win"],
        argument: "",
        summary: "leave fullscreen",
        scopes: &[Scope::Boot, Scope::Editor, Scope::Program],
    },
    Spec {
        name: "undo",
        aliases: &["u"],
        argument: "",
        summary: "undo the last edit",
        scopes: &[Scope::Editor, Scope::Program],
    },
    Spec {
        name: "redo",
        aliases: &[],
        argument: "",
        summary: "redo",
        scopes: &[Scope::Editor, Scope::Program],
    },
    Spec {
        name: "view",
        aliases: &["read"],
        argument: "",
        summary: "reading mode: markup stripped, nothing to edit",
        scopes: &[Scope::Editor],
    },
    Spec {
        name: "find",
        aliases: &["search"],
        argument: "[text]",
        summary: "search the document (Ctrl+F, F3 for the next hit)",
        scopes: &[Scope::Editor],
    },
    Spec {
        name: "run",
        aliases: &["exec"],
        argument: "[name]",
        summary: "run a diskette (bare: the one selected on the boot screen)",
        scopes: &[Scope::Boot],
    },
    Spec {
        name: "dir",
        aliases: &["disks"],
        argument: "",
        summary: "list the diskettes in the drive folder",
        scopes: &[Scope::Boot],
    },
    Spec {
        name: "tool",
        aliases: &["tools"],
        argument: "[name]",
        summary: "pencil, eraser, line, rect, fill, pick",
        scopes: &[Scope::Program],
    },
    Spec {
        name: "size",
        aliases: &["brush", "width"],
        argument: "[n]",
        summary: "brush size, 1 to 16",
        scopes: &[Scope::Program],
    },
    Spec {
        name: "colour",
        aliases: &["color", "ink"],
        argument: "[n]",
        summary: "palette colour, 0 to 15",
        scopes: &[Scope::Program],
    },
    Spec {
        name: "settings",
        aliases: &["set", "cfg"],
        argument: "",
        summary: "WebDAV settings",
        scopes: &[Scope::Boot, Scope::Editor],
    },
    Spec {
        name: "help",
        aliases: &["?", "h"],
        argument: "[all]",
        summary: "list these commands",
        scopes: &[Scope::Boot, Scope::Editor, Scope::Program],
    },
    Spec {
        name: "about",
        aliases: &["version"],
        argument: "",
        summary: "version and credits",
        scopes: &[Scope::Boot, Scope::Editor, Scope::Program],
    },
    Spec {
        name: "exit",
        aliases: &["welcome", "home"],
        argument: "",
        summary: "back to the boot screen (there: quit)",
        scopes: &[Scope::Boot, Scope::Editor, Scope::Program],
    },
    Spec {
        name: "exit!",
        aliases: &["quit", "q", "x"],
        argument: "",
        summary: "save and quit from anywhere",
        scopes: &[Scope::Boot, Scope::Editor, Scope::Program],
    },
];

/// The commands that belong to one screen, in table order.
pub fn specs_for(scope: Scope) -> Vec<&'static Spec> {
    SPECS
        .iter()
        .filter(|spec| spec.scopes.contains(&scope))
        .collect()
}

/// Parse a command line. Arguments are everything after the first whitespace, so
/// file names may contain spaces.
pub fn parse(input: &str) -> Result<Command, CommandError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(CommandError::Empty);
    }

    let (word, rest) = match trimmed.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (trimmed, ""),
    };

    // A trailing bang makes a command unmistakable: `exit` steps back to the
    // boot screen, `exit!` quits. Accept `exit !` with a space too.
    let (word, bang) = match word.strip_suffix('!') {
        Some(stem) => (stem, true),
        None => (word, rest == "!"),
    };

    // Command names are case insensitive: this is a writing tool, not a shell.
    let needle = word.to_lowercase();
    let spec = SPECS
        .iter()
        .find(|s| s.name == needle || s.aliases.contains(&needle.as_str()))
        .ok_or_else(|| CommandError::Unknown(word.to_string()))?;

    let required = |rest: &str| -> Result<String, CommandError> {
        if rest.is_empty() {
            Err(CommandError::MissingArgument {
                command: spec.name,
                argument: spec.argument,
            })
        } else {
            Ok(rest.to_string())
        }
    };
    let optional = |rest: &str| -> Option<String> { (!rest.is_empty()).then(|| rest.to_string()) };
    let switch = |rest: &str| -> Result<Option<bool>, CommandError> {
        parse_switch(rest).map_err(|value| CommandError::BadValue {
            value,
            expected: "on or off",
        })
    };

    Ok(match spec.name {
        "save" => Command::Save,
        "saveas" => Command::SaveAs(required(rest)?),
        "open" => Command::Open(optional(rest)),
        "new" => Command::New,
        "rename" => Command::Rename(required(rest)?),
        "delete" => Command::Delete,
        "sync" => Command::Sync,
        "theme" => Command::Theme(optional(rest)),
        "scanlines" => {
            let roll = rest
                .split_whitespace()
                .any(|word| word == "-r" || word == "--roll");
            let plain = rest
                .split_whitespace()
                .filter(|word| !word.starts_with('-'))
                .collect::<Vec<_>>()
                .join(" ");
            Command::Scanlines {
                on: switch(&plain)?,
                roll,
            }
        }
        "bloom" => Command::Bloom(switch(rest)?),
        "roll" => Command::Roll(switch(rest)?),
        "typewriter" => Command::Typewriter(switch(rest)?),
        "font" => Command::Font(optional(rest)),
        "fullscreen" => Command::Fullscreen(true),
        "windowed" => Command::Fullscreen(false),
        "undo" => Command::Undo,
        "redo" => Command::Redo,
        "view" => Command::View,
        "find" => Command::Find(optional(rest)),
        "run" => Command::Run(optional(rest)),
        "dir" => Command::Dir,
        "exit" => {
            if bang {
                Command::ExitForce
            } else {
                Command::Exit
            }
        }
        "exit!" => Command::ExitForce,
        "settings" => Command::Settings,
        "help" => Command::Help(optional(rest)),
        "tool" => Command::Tool(optional(rest)),
        "size" => Command::Size(match optional(rest) {
            None => None,
            Some(text) => Some(text.parse().map_err(|_| CommandError::BadValue {
                value: text.clone(),
                expected: "a number from 1 to 16",
            })?),
        }),
        "colour" => Command::Colour(match optional(rest) {
            None => None,
            Some(text) => Some(text.parse().map_err(|_| CommandError::BadValue {
                value: text.clone(),
                expected: "a number from 0 to 15",
            })?),
        }),
        "about" => Command::About,
        other => return Err(CommandError::Unknown(other.to_string())),
    })
}

/// `Ok(None)` for an empty argument meaning "toggle".
fn parse_switch(rest: &str) -> Result<Option<bool>, String> {
    match rest.trim().to_ascii_lowercase().as_str() {
        "" => Ok(None),
        "on" | "yes" | "true" | "1" => Ok(Some(true)),
        "off" | "no" | "false" | "0" => Ok(Some(false)),
        other => Err(other.to_string()),
    }
}

/// What the prompt line should suggest after the current input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// The input rewritten with the completion applied.
    pub completed: String,
    /// Dimmed hint shown to the right.
    pub hint: String,
}

/// Complete the command word, and always report the matching summary as a hint.
pub fn complete(input: &str) -> Option<Completion> {
    let text = input.trim_start();
    if text.contains(char::is_whitespace) {
        // Past the command word we only keep the hint for the exact command.
        let word = text.split_whitespace().next().unwrap_or("").to_lowercase();
        let spec = SPECS
            .iter()
            .find(|s| s.name == word || s.aliases.contains(&word.as_str()))?;
        return Some(Completion {
            completed: input.to_string(),
            hint: format!(
                "{}{}  {}",
                spec.name,
                if spec.argument.is_empty() {
                    String::new()
                } else {
                    format!(" {}", spec.argument)
                },
                spec.summary
            ),
        });
    }

    let needle = text.to_lowercase();
    let mut matches: Vec<&Spec> = SPECS
        .iter()
        .filter(|s| s.name.starts_with(&needle))
        .collect();
    matches.sort_by_key(|s| s.name);
    let first = *matches.first()?;

    let hint = format!(
        "{}{}  {}",
        first.name,
        if first.argument.is_empty() {
            String::new()
        } else {
            format!(" {}", first.argument)
        },
        first.summary
    );

    let completed = if matches.len() == 1 {
        format!("{} ", first.name)
    } else {
        let names: Vec<&str> = matches.iter().map(|s| s.name).collect();
        longest_common_prefix(&names).to_string()
    };

    Some(Completion { completed, hint })
}

fn longest_common_prefix<'a>(items: &[&'a str]) -> &'a str {
    let first = match items.first() {
        Some(f) => *f,
        None => return "",
    };
    let mut end = first.len();
    for item in &items[1..] {
        let mut i = 0;
        for (a, b) in first.bytes().zip(item.bytes()) {
            if a != b || i >= end {
                break;
            }
            i += 1;
        }
        end = i;
    }
    &first[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spec_parses_with_its_required_argument() {
        for spec in SPECS {
            let line = if spec.argument.starts_with('<') {
                format!("{} thing", spec.name)
            } else {
                spec.name.to_string()
            };
            assert!(parse(&line).is_ok(), "{} does not parse", spec.name);
        }
    }

    #[test]
    fn aliases_resolve_to_the_same_command() {
        assert_eq!(parse("w"), parse("save"));
        assert_eq!(parse("safe"), parse("save"));
        assert_eq!(parse("q"), Ok(Command::ExitForce));
        assert_eq!(parse("quit"), Ok(Command::ExitForce));
        assert_eq!(parse("welcome"), Ok(Command::Exit));
        assert_eq!(parse("?"), parse("help"));
        assert_eq!(parse("o"), parse("open"));
    }

    #[test]
    fn help_is_scoped_to_the_screen_you_are_on() {
        let boot = specs_for(Scope::Boot);
        assert!(boot.iter().any(|s| s.name == "run"));
        assert!(boot.iter().any(|s| s.name == "dir"));
        assert!(
            !boot.iter().any(|s| s.name == "save"),
            "nothing to save yet"
        );
        assert!(boot.iter().any(|s| s.name == "exit"));

        let editor = specs_for(Scope::Editor);
        assert!(editor.iter().any(|s| s.name == "save"));
        assert!(editor.iter().any(|s| s.name == "find"));
        assert!(!editor.iter().any(|s| s.name == "tool"));

        let program = specs_for(Scope::Program);
        assert!(program.iter().any(|s| s.name == "tool"));
        assert!(program.iter().any(|s| s.name == "size"));
        assert!(program.iter().any(|s| s.name == "colour"));
        assert!(!program.iter().any(|s| s.name == "rename"));
        assert!(!program.iter().any(|s| s.name == "sync"));
    }

    #[test]
    fn paint_settings_parse() {
        assert_eq!(parse("size 4"), Ok(Command::Size(Some(4))));
        assert_eq!(parse("size"), Ok(Command::Size(None)));
        assert!(matches!(
            parse("size x"),
            Err(CommandError::BadValue { .. })
        ));
        assert_eq!(parse("colour 12"), Ok(Command::Colour(Some(12))));
        assert_eq!(parse("color 12"), Ok(Command::Colour(Some(12))));
        assert_eq!(parse("tool fill"), Ok(Command::Tool(Some("fill".into()))));
        assert_eq!(parse("brush"), Ok(Command::Size(None)));
        assert_eq!(parse("help all"), Ok(Command::Help(Some("all".into()))));
    }

    #[test]
    fn exit_steps_back_and_exit_bang_quits() {
        assert_eq!(parse("exit"), Ok(Command::Exit));
        assert_eq!(parse("exit!"), Ok(Command::ExitForce));
        assert_eq!(parse("exit !"), Ok(Command::ExitForce));
        assert_eq!(parse("EXIT!"), Ok(Command::ExitForce));
        assert_eq!(parse("home"), Ok(Command::Exit));
        assert_eq!(
            parse("exi"),
            Err(CommandError::Unknown("exi".into())),
            "no abbreviations"
        );
    }

    #[test]
    fn arguments_keep_their_spaces() {
        assert_eq!(
            parse("open meine notizen.md"),
            Ok(Command::Open(Some("meine notizen.md".into())))
        );
        assert_eq!(
            parse("saveas  über   alles.md "),
            Ok(Command::SaveAs("über   alles.md".into()))
        );
    }

    #[test]
    fn theme_accepts_plus_and_minus() {
        assert_eq!(parse("theme +"), Ok(Command::Theme(Some("+".into()))));
        assert_eq!(parse("theme -"), Ok(Command::Theme(Some("-".into()))));
    }

    #[test]
    fn optional_arguments_may_be_missing() {
        assert_eq!(parse("open"), Ok(Command::Open(None)));
        assert_eq!(parse("theme"), Ok(Command::Theme(None)));
        assert_eq!(
            parse("theme amber"),
            Ok(Command::Theme(Some("amber".into())))
        );
    }

    #[test]
    fn switches_accept_the_obvious_spellings() {
        assert_eq!(
            parse("scanlines"),
            Ok(Command::Scanlines {
                on: None,
                roll: false
            })
        );
        assert_eq!(
            parse("scanlines on"),
            Ok(Command::Scanlines {
                on: Some(true),
                roll: false
            })
        );
        assert_eq!(
            parse("scanlines off"),
            Ok(Command::Scanlines {
                on: Some(false),
                roll: false
            })
        );
        assert_eq!(
            parse("scanlines -r"),
            Ok(Command::Scanlines {
                on: None,
                roll: true
            })
        );
        assert_eq!(
            parse("scanlines off -r"),
            Ok(Command::Scanlines {
                on: Some(false),
                roll: true
            })
        );
        assert_eq!(parse("bloom 0"), Ok(Command::Bloom(Some(false))));
        assert_eq!(parse("tw yes"), Ok(Command::Typewriter(Some(true))));
    }

    #[test]
    fn required_arguments_are_reported_as_such() {
        let err = parse("saveas").unwrap_err();
        assert_eq!(
            err,
            CommandError::MissingArgument {
                command: "saveas",
                argument: "<name>"
            }
        );
        assert!(err.message().contains("needs <name>"));
    }

    #[test]
    fn nonsense_is_rejected_with_the_offending_word() {
        assert_eq!(
            parse("frobnicate"),
            Err(CommandError::Unknown("frobnicate".into()))
        );
        assert!(parse("").is_err());
        assert!(matches!(
            parse("bloom maybe"),
            Err(CommandError::BadValue { .. })
        ));
    }

    #[test]
    fn cases_are_forgiving_where_it_matters() {
        assert_eq!(
            parse("SCANLINES ON"),
            Ok(Command::Scanlines {
                on: Some(true),
                roll: false
            })
        );
        assert_eq!(parse("Save"), Ok(Command::Save));
        assert_eq!(parse("  SyNc  "), Ok(Command::Sync));
    }

    #[test]
    fn completion_finds_the_only_match() {
        let c = complete("savea").unwrap();
        assert_eq!(c.completed, "saveas ");
        assert!(c.hint.starts_with("saveas "));

        let c = complete("hel").unwrap();
        assert_eq!(c.completed, "help ");

        // `exit` and `exit!` share a prefix and complete only as far as that.
        let c = complete("exi").unwrap();
        assert_eq!(c.completed, "exit");
    }

    #[test]
    fn completion_reports_a_common_prefix_when_ambiguous() {
        // "s" matches save, saveas, scanlines, sync, settings.
        let c = complete("s").unwrap();
        assert_eq!(c.completed, "s");
        assert!(c.hint.contains("save"));
    }

    #[test]
    fn completion_keeps_the_hint_after_the_command_word() {
        let c = complete("theme ").unwrap();
        assert_eq!(c.completed, "theme ");
        assert!(c.hint.contains("switch theme"));
    }

    #[test]
    fn completion_of_nothing_is_none() {
        assert!(complete("zzz").is_none());
    }

    #[test]
    fn help_has_a_row_for_every_command() {
        assert!(SPECS.len() >= 15);
        assert!(SPECS.iter().all(|s| !s.summary.is_empty()));
    }
}
