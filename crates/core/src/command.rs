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
    Scanlines(Option<bool>),
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
    /// Toggle reader mode.
    View,
    Settings,
    Help,
    About,
    Quit,
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
}

pub const SPECS: &[Spec] = &[
    Spec {
        name: "save",
        aliases: &["w", "safe", "write"],
        argument: "",
        summary: "write the document to disk",
    },
    Spec {
        name: "saveas",
        aliases: &["sa"],
        argument: "<name>",
        summary: "save under a new name in the vault",
    },
    Spec {
        name: "open",
        aliases: &["o", "e"],
        argument: "[name]",
        summary: "open a document (bare: file list)",
    },
    Spec {
        name: "new",
        aliases: &["n"],
        argument: "",
        summary: "start an empty document",
    },
    Spec {
        name: "rename",
        aliases: &["mv", "rn"],
        argument: "<name>",
        summary: "rename the current document",
    },
    Spec {
        name: "delete",
        aliases: &["rm", "del"],
        argument: "",
        summary: "delete the current document",
    },
    Spec {
        name: "sync",
        aliases: &[],
        argument: "",
        summary: "sync the vault with WebDAV now",
    },
    Spec {
        name: "theme",
        aliases: &[],
        argument: "[name]",
        summary: "switch theme (no argument: next)",
    },
    Spec {
        name: "scanlines",
        aliases: &["scan", "sl"],
        argument: "[on|off]",
        summary: "toggle or set the scanlines",
    },
    Spec {
        name: "bloom",
        aliases: &[],
        argument: "[on|off]",
        summary: "toggle or set the phosphor glow",
    },
    Spec {
        name: "roll",
        aliases: &["drift"],
        argument: "[on|off]",
        summary: "let the scanline pattern drift downwards",
    },
    Spec {
        name: "typewriter",
        aliases: &["tw"],
        argument: "[on|off]",
        summary: "toggle or set typewriter scrolling",
    },
    Spec {
        name: "font",
        aliases: &["weight"],
        argument: "[classic|thin]",
        summary: "face: classic VGA or thin ISO (bare: toggle)",
    },
    Spec {
        name: "fullscreen",
        aliases: &["fs"],
        argument: "",
        summary: "go fullscreen",
    },
    Spec {
        name: "windowed",
        aliases: &["win"],
        argument: "",
        summary: "leave fullscreen",
    },
    Spec {
        name: "undo",
        aliases: &["u"],
        argument: "",
        summary: "undo the last edit",
    },
    Spec {
        name: "redo",
        aliases: &[],
        argument: "",
        summary: "redo",
    },
    Spec {
        name: "view",
        aliases: &["read"],
        argument: "",
        summary: "reading mode: markup stripped, nothing to edit",
    },
    Spec {
        name: "find",
        aliases: &["search"],
        argument: "[text]",
        summary: "search the document (Ctrl+F, F3 for the next hit)",
    },
    Spec {
        name: "settings",
        aliases: &["set", "cfg"],
        argument: "",
        summary: "WebDAV settings",
    },
    Spec {
        name: "help",
        aliases: &["?", "h"],
        argument: "",
        summary: "list these commands",
    },
    Spec {
        name: "about",
        aliases: &["version"],
        argument: "",
        summary: "version and credits",
    },
    Spec {
        name: "quit",
        aliases: &["q", "x", "exit"],
        argument: "",
        summary: "save and quit",
    },
];

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
        "scanlines" => Command::Scanlines(switch(rest)?),
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
        "settings" => Command::Settings,
        "help" => Command::Help,
        "about" => Command::About,
        "quit" => Command::Quit,
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
        assert_eq!(parse("q"), parse("quit"));
        assert_eq!(parse("?"), parse("help"));
        assert_eq!(parse("o"), parse("open"));
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
        assert_eq!(parse("scanlines"), Ok(Command::Scanlines(None)));
        assert_eq!(parse("scanlines on"), Ok(Command::Scanlines(Some(true))));
        assert_eq!(parse("scanlines off"), Ok(Command::Scanlines(Some(false))));
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
        assert_eq!(parse("SCANLINES ON"), Ok(Command::Scanlines(Some(true))));
        assert_eq!(parse("Save"), Ok(Command::Save));
        assert_eq!(parse("  SyNc  "), Ok(Command::Sync));
    }

    #[test]
    fn completion_finds_the_only_match() {
        let c = complete("savea").unwrap();
        assert_eq!(c.completed, "saveas ");
        assert!(c.hint.starts_with("saveas "));

        let c = complete("qui").unwrap();
        assert_eq!(c.completed, "quit ");
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
