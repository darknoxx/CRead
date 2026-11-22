//! Printing through CUPS: the printers it knows, the default, and a job.
//!
//! CRead does not set printers up. CUPS lists network printers that print
//! without a driver (IPP Everywhere, AirPrint) by itself, and `lp -d NAME`
//! prints on one even before a queue exists. The writer's choice is kept in
//! CRead's preferences and named on every job: CUPS does not store a default
//! for a printer it has only discovered.

use std::path::Path;

use crate::run;

/// The printers CUPS knows and which one is the default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Printers {
    pub names: Vec<String>,
    pub default: Option<String>,
}

/// Characters per line on the page: 12 per inch across A4 with 2 cm margins.
pub const PAGE_COLUMNS: usize = 80;

/// Ask CUPS. An empty list is not an error: there may just be no printer yet.
pub fn list() -> Result<Printers, String> {
    let names = parse_names(&run("lpstat", &["-e"], None)?);
    let default = run("lpstat", &["-d"], None)
        .ok()
        .and_then(|out| parse_default(&out));
    Ok(Printers { names, default })
}

/// Print a document's text: one typewriter face, wide margins.
pub fn print_text(printer: Option<&str>, title: &str, text: &str) -> Result<String, String> {
    let mut args = destination(printer);
    args.extend([
        "-t".to_string(),
        title.to_string(),
        "-o".into(),
        "cpi=12".into(),
        "-o".into(),
        "lpi=6".into(),
        "-o".into(),
        "page-left=57".into(),
        "-o".into(),
        "page-right=57".into(),
        "-o".into(),
        "page-top=57".into(),
        "-o".into(),
        "page-bottom=57".into(),
    ]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run("lp", &args, Some(text))?;
    Ok(job_id(&out))
}

/// Print a picture, scaled to the page.
pub fn print_file(printer: Option<&str>, title: &str, path: &Path) -> Result<String, String> {
    let mut args = destination(printer);
    let path = path.display().to_string();
    args.extend([
        "-t".into(),
        title.to_string(),
        "-o".into(),
        "fit-to-page".into(),
        path,
    ]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run("lp", &args, None)?;
    Ok(job_id(&out))
}

/// Which printer a job goes to: the chosen one, else the only one there is,
/// else CUPS's own default (`None`).
pub fn resolve(chosen: Option<&str>, known: &[String]) -> Option<String> {
    if let Some(name) = chosen {
        return Some(name.to_string());
    }
    match known {
        [only] => Some(only.clone()),
        _ => None,
    }
}

fn destination(printer: Option<&str>) -> Vec<String> {
    match printer {
        Some(name) => vec!["-d".into(), name.into()],
        None => Vec::new(),
    }
}

/// `lpstat -e`: one destination per line.
pub fn parse_names(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// `lpstat -d`: `system default destination: NAME`, or a line saying there
/// is none.
pub fn parse_default(out: &str) -> Option<String> {
    out.lines()
        .find_map(|l| l.split_once("destination:"))
        .map(|(_, name)| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

/// `lp`: `request id is Samsung-12 (1 file(s))`.
pub fn job_id(out: &str) -> String {
    out.split_whitespace()
        .skip_while(|w| *w != "is")
        .nth(1)
        .unwrap_or("")
        .to_string()
}

/// The document as it should look on paper: markup gone, wrapped to the page.
pub fn page_text(lines: &[String]) -> String {
    let rows = cread_core::markdown::render(lines, PAGE_COLUMNS);
    let mut text = String::new();
    for row in rows {
        text.push_str(row.text.trim_end());
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printers_and_the_default_are_read() {
        let names = parse_names("Samsung_M2070\nPDF\n\n");
        assert_eq!(names, ["Samsung_M2070", "PDF"]);
        assert_eq!(
            parse_default("system default destination: Samsung_M2070\n"),
            Some("Samsung_M2070".into())
        );
        assert_eq!(parse_default("no system default destination\n"), None);
    }

    #[test]
    fn the_job_id_is_picked_out() {
        assert_eq!(
            job_id("request id is Samsung_M2070-12 (1 file(s))\n"),
            "Samsung_M2070-12"
        );
        assert_eq!(job_id(""), "");
    }

    #[test]
    fn a_page_has_no_markup_and_fits_the_paper() {
        let long = "word ".repeat(40);
        let lines = vec![
            "# Title".to_string(),
            String::new(),
            long,
            "**bold**".into(),
        ];
        let text = page_text(&lines);
        assert!(text.starts_with("Title\n"), "{text}");
        assert!(text.lines().all(|l| l.chars().count() <= PAGE_COLUMNS));
        assert!(text.contains("bold") && !text.contains("**"));
    }

    #[test]
    fn the_choice_wins_then_the_only_printer() {
        let two = ["A".to_string(), "B".to_string()];
        assert_eq!(resolve(Some("B"), &two), Some("B".into()));
        assert_eq!(resolve(None, &two[..1]), Some("A".into()));
        assert_eq!(resolve(None, &two), None);
        assert_eq!(resolve(None, &[]), None);
    }
}
