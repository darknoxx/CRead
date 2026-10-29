//! Command-line surface. Kept dependency-free on purpose: the whole app is one
//! binary and a handful of flags.

use std::path::PathBuf;

use cread_core::Theme;
use cread_render::CrtSettings;

pub const USAGE: &str = "\
cread — retro CRT writing terminal

USAGE:
    cread [OPTIONS] [FILE]

OPTIONS:
    -w, --windowed          Start in a window instead of fullscreen
        --scale N           Force an integer upscale factor (default: auto)
        --theme NAME        black | dos-blue | amber | green | paper
        --size WxH          Window size, e.g. 1920x1080 (handy for testing)
        --backend NAME      vulkan | gl
        --no-crt            Disable scanlines, bloom and flicker
        --no-scanlines      Disable scanlines only
        --no-bloom          Disable bloom only
        --vault DIR         Document folder (default: ~/Documents/CRead)
        --demo              Open the built-in welcome document
        --sync-once         Run one WebDAV sync round, print the result, exit
        --show VIEW         Start in a view: editor | welcome | action | palette |
                            settings | help
        --command TEXT      Prefill the action prompt (handy for demos)
        --screenshot-delay MS  Wait before capturing (lets background work finish)
        --screenshot PATH   Render one frame to PATH (PNG) and exit
    -V, --version           Show the version
    -h, --help              Show this help
";

#[derive(Clone, Debug)]
pub struct Options {
    pub fullscreen: bool,
    pub scale: Option<u32>,
    pub theme: Theme,
    pub crt: CrtSettings,
    pub size: Option<(u32, u32)>,
    pub backend: Option<String>,
    pub screenshot: Option<PathBuf>,
    pub file: Option<PathBuf>,
    pub vault: Option<PathBuf>,
    pub demo: bool,
    pub sync_once: bool,
    pub show: String,
    pub screenshot_delay: u64,
    pub command: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            fullscreen: true,
            scale: None,
            theme: Theme::BLACK,
            crt: CrtSettings::default(),
            size: None,
            backend: None,
            screenshot: None,
            file: None,
            vault: None,
            demo: false,
            sync_once: false,
            show: String::from("editor"),
            screenshot_delay: 0,
            command: String::new(),
        }
    }
}

#[allow(clippy::large_enum_variant)]
pub enum ParseOutcome {
    Run(Options),
    Help,
    Version,
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<ParseOutcome, String> {
    let mut opts = Options::default();
    let mut args = args.into_iter().peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(ParseOutcome::Help),
            "-V" | "--version" => return Ok(ParseOutcome::Version),
            "-w" | "--windowed" => opts.fullscreen = false,
            "--no-crt" => {
                opts.crt.scanline = 0.0;
                opts.crt.bloom = 0.0;
            }
            "--no-scanlines" => opts.crt.scanline = 0.0,
            "--no-bloom" => opts.crt.bloom = 0.0,
            "--scale" => {
                let v = args.next().ok_or("--scale needs a value")?;
                let n: u32 = v.parse().map_err(|_| format!("bad scale: {v}"))?;
                if !(1..=16).contains(&n) {
                    return Err(format!("scale must be 1..16, got {n}"));
                }
                opts.scale = Some(n);
            }
            "--theme" => {
                let v = args.next().ok_or("--theme needs a value")?;
                opts.theme = Theme::by_name(&v).ok_or_else(|| format!("unknown theme: {v}"))?;
            }
            "--size" => {
                let v = args.next().ok_or("--size needs a value")?;
                opts.size = Some(parse_size(&v)?);
            }
            "--backend" => {
                let v = args.next().ok_or("--backend needs a value")?;
                if v != "vulkan" && v != "gl" {
                    return Err(format!("unknown backend: {v}"));
                }
                opts.backend = Some(v);
            }
            "--vault" => {
                let v = args.next().ok_or("--vault needs a path")?;
                opts.vault = Some(PathBuf::from(v));
            }
            "--demo" => opts.demo = true,
            "--sync-once" => opts.sync_once = true,
            "--screenshot-delay" => {
                let v = args.next().ok_or("--screenshot-delay needs a value")?;
                opts.screenshot_delay = v.parse().map_err(|_| format!("bad delay: {v}"))?;
            }
            "--command" => {
                let v = args.next().ok_or("--command needs a value")?;
                opts.command = v;
            }
            "--show" => {
                let v = args.next().ok_or("--show needs a value")?;
                if !["editor", "welcome", "action", "palette", "settings", "help"]
                    .contains(&v.as_str())
                {
                    return Err(format!("unknown view: {v}"));
                }
                opts.show = v;
            }
            "--screenshot" => {
                let v = args.next().ok_or("--screenshot needs a path")?;
                opts.screenshot = Some(PathBuf::from(v));
            }
            other if other.starts_with('-') => return Err(format!("unknown flag: {other}")),
            other => {
                if opts.file.is_some() {
                    return Err("only one file can be opened at a time".into());
                }
                opts.file = Some(PathBuf::from(other));
            }
        }
    }

    Ok(ParseOutcome::Run(opts))
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("bad size: {s}"))?;
    let w: u32 = w.parse().map_err(|_| format!("bad width: {w}"))?;
    let h: u32 = h.parse().map_err(|_| format!("bad height: {h}"))?;
    if w < 320 || h < 200 {
        return Err(format!("size too small: {s}"));
    }
    Ok((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Options {
        match parse(args.iter().map(|s| s.to_string())).unwrap() {
            ParseOutcome::Run(o) => o,
            ParseOutcome::Help | ParseOutcome::Version => panic!("expected options"),
        }
    }

    #[test]
    fn defaults_are_fullscreen_dos_blue() {
        let o = run(&[]);
        assert!(o.fullscreen);
        assert_eq!(o.theme, Theme::BLACK);
        assert!(o.crt.scanline > 0.0);
        assert!(o.file.is_none());
    }

    #[test]
    fn parses_theme_scale_and_file() {
        let o = run(&["--windowed", "--scale", "3", "--theme", "amber", "notes.md"]);
        assert!(!o.fullscreen);
        assert_eq!(o.scale, Some(3));
        assert_eq!(o.theme, Theme::AMBER);
        assert_eq!(o.file.unwrap().to_str(), Some("notes.md"));
    }

    #[test]
    fn no_crt_zeroes_every_effect() {
        let o = run(&["--no-crt"]);
        assert_eq!(o.crt.scanline, 0.0);
        assert_eq!(o.crt.bloom, 0.0);
    }

    fn err(args: &[&str]) -> String {
        match parse(args.iter().map(|s| s.to_string())) {
            Err(e) => e,
            Ok(_) => panic!("expected an error for {args:?}"),
        }
    }

    #[test]
    fn version_and_help_are_their_own_outcomes() {
        assert!(matches!(
            parse(["--version".to_string()]),
            Ok(ParseOutcome::Version)
        ));
        assert!(matches!(
            parse(["-V".to_string()]),
            Ok(ParseOutcome::Version)
        ));
        assert!(matches!(
            parse(["--help".to_string()]),
            Ok(ParseOutcome::Help)
        ));
    }

    #[test]
    fn rejects_nonsense() {
        assert!(err(&["--scale", "0"]).contains("scale"));
        assert!(err(&["--theme", "neon"]).contains("theme"));
        assert!(err(&["--backend", "dx12"]).contains("backend"));
        assert!(err(&["--wat"]).contains("unknown flag"));
        assert!(err(&["--size", "12x99"]).contains("too small"));
        assert!(err(&["--scale"]).contains("needs a value"));
        assert!(err(&["a.md", "b.md"]).contains("one file"));
    }

    #[test]
    fn parses_screenshot_size() {
        let o = run(&["--size", "1920x1080", "--screenshot", "out.png"]);
        assert_eq!(o.size, Some((1920, 1080)));
        assert_eq!(o.screenshot.unwrap().to_str(), Some("out.png"));
    }
}
