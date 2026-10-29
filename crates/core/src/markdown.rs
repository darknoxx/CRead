//! Markdown *inline* highlighting for a monospace grid.
//!
//! Markers are dimmed rather than hidden. Hiding them would reflow the text and
//! make the cursor jump while typing, which is exactly what you don't want in a
//! writing app.

/// Visual role of a run of characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Normal,
    /// Syntax punctuation: `#`, `**`, backticks, the URL of a link.
    Marker,
    Strong,
    Emphasis,
    StrongEmphasis,
    Code,
    Link,
    LinkUrl,
    /// Text of a heading (the `#`s are markers).
    Heading,
    Quote,
    Bullet,
    /// A horizontal rule line.
    Rule,
}

/// A contiguous run of characters with one role, in char indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

/// Parse one line. `in_fence` is the fenced-code state coming in; the boolean
/// returned is the state for the next line.
pub fn parse_line(line: &str, in_fence: bool) -> (Vec<Span>, bool) {
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut spans = Vec::new();

    let fence = is_fence_line(line);
    if in_fence || fence {
        spans.push(Span {
            start: 0,
            end: len,
            kind: if fence { Kind::Marker } else { Kind::Code },
        });
        return (spans, fence != in_fence);
    }

    if len == 0 {
        return (spans, false);
    }

    if is_rule(line) {
        spans.push(Span {
            start: 0,
            end: len,
            kind: Kind::Rule,
        });
        return (spans, false);
    }

    // Block prefixes.
    let mut body_start = 0;
    let mut base = Kind::Normal;

    let hashes = chars.iter().take_while(|c| **c == '#').count();
    if (1..=6).contains(&hashes) && (hashes == len || chars[hashes] == ' ') {
        let prefix_end = (hashes + 1).min(len);
        spans.push(Span {
            start: 0,
            end: prefix_end,
            kind: Kind::Marker,
        });
        body_start = prefix_end;
        base = Kind::Heading;
    } else if chars[0] == '>' {
        let prefix_end = if len > 1 && chars[1] == ' ' { 2 } else { 1 };
        spans.push(Span {
            start: 0,
            end: prefix_end,
            kind: Kind::Marker,
        });
        body_start = prefix_end;
        base = Kind::Quote;
    } else if let Some(prefix_end) = bullet_len(&chars) {
        spans.push(Span {
            start: 0,
            end: prefix_end,
            kind: Kind::Bullet,
        });
        body_start = prefix_end;
    }

    inline(&chars, body_start, len, base, &mut spans, 0);
    spills_to_end(&mut spans, len);
    (spans, false)
}

/// True when the line opens or closes a fenced code block.
pub fn is_fence_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

fn is_rule(line: &str) -> bool {
    let t = line.trim();
    if t.len() < 3 {
        return false;
    }
    let first = t.chars().next().unwrap_or(' ');
    matches!(first, '-' | '*' | '=' | '_') && t.chars().all(|c| c == first || c == ' ')
}

fn bullet_len(chars: &[char]) -> Option<usize> {
    if chars.len() >= 2 && matches!(chars[0], '-' | '*' | '+') && chars[1] == ' ' {
        return Some(2);
    }
    // "12. " style ordered list.
    let digits = chars.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits + 1 < chars.len() && chars[digits] == '.' && chars[digits + 1] == ' ' {
        return Some(digits + 2);
    }
    None
}

fn inline(chars: &[char], from: usize, to: usize, base: Kind, out: &mut Vec<Span>, depth: usize) {
    let mut i = from;
    let mut text_start = from;

    while i < to {
        let c = chars[i];

        if c == '`'
            && let Some(close) = find(chars, i + 1, to, '`') {
                push(out, text_start, i, base);
                push(out, i, i + 1, Kind::Marker);
                push(out, i + 1, close, Kind::Code);
                push(out, close, close + 1, Kind::Marker);
                i = close + 1;
                text_start = i;
                continue;
            }

        if (c == '*' || c == '_') && depth < 4 {
            let doubled = i + 1 < to && chars[i + 1] == c;
            let marker = if doubled { 2 } else { 1 };
            let kind = match (doubled, base) {
                (true, _) => Kind::Strong,
                (false, Kind::Strong | Kind::StrongEmphasis) => Kind::StrongEmphasis,
                (false, _) => Kind::Emphasis,
            };
            if let Some(close) = find_run(chars, i + marker, to, c, marker)
                && close > i + marker {
                    push(out, text_start, i, base);
                    push(out, i, i + marker, Kind::Marker);
                    inline(chars, i + marker, close, kind, out, depth + 1);
                    push(out, close, close + marker, Kind::Marker);
                    i = close + marker;
                    text_start = i;
                    continue;
                }
        }

        if c == '[' && depth < 4
            && let Some(close) = find(chars, i + 1, to, ']')
                && close + 1 < to && chars[close + 1] == '('
                    && let Some(paren) = find(chars, close + 2, to, ')') {
                        push(out, text_start, i, base);
                        push(out, i, i + 1, Kind::Marker);
                        inline(chars, i + 1, close, Kind::Link, out, depth + 1);
                        push(out, close, close + 1, Kind::Marker);
                        push(out, close + 1, paren + 1, Kind::LinkUrl);
                        i = paren + 1;
                        text_start = i;
                        continue;
                    }

        if c == '\\' && i + 1 < to {
            i += 2;
            continue;
        }

        i += 1;
    }

    push(out, text_start, to, base);
}

fn push(out: &mut Vec<Span>, start: usize, end: usize, kind: Kind) {
    if end > start {
        out.push(Span { start, end, kind });
    }
}

/// Guarantee full coverage so the renderer can walk spans without gaps.
fn spills_to_end(spans: &mut Vec<Span>, len: usize) {
    let covered = spans.last().map(|s| s.end).unwrap_or(0);
    if covered < len {
        spans.push(Span {
            start: covered,
            end: len,
            kind: Kind::Normal,
        });
    }
}

fn find(chars: &[char], from: usize, to: usize, target: char) -> Option<usize> {
    (from..to).find(|&i| chars[i] == target)
}

fn find_run(chars: &[char], from: usize, to: usize, target: char, run: usize) -> Option<usize> {
    let mut i = from;
    while i + run <= to {
        if (0..run).all(|k| chars[i + k] == target) {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Vec<Span> {
        parse_line(line, false).0
    }

    fn kinds(line: &str) -> Vec<Kind> {
        parse(line).into_iter().map(|s| s.kind).collect()
    }

    fn text_of(line: &str, spans: &[Span]) -> Vec<String> {
        let chars: Vec<char> = line.chars().collect();
        spans
            .iter()
            .map(|s| chars[s.start..s.end].iter().collect())
            .collect()
    }

    #[test]
    fn plain_text_is_one_span_and_covers_everything() {
        let spans = parse("just prose");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, Kind::Normal);
        assert_eq!(spans[0].end, 10);
    }

    #[test]
    fn spans_are_contiguous() {
        for line in [
            "# Heading with **bold**",
            "- item with `code` and [link](https://x.y)",
            "> quote *emph*",
            "---",
            "",
            "```` fence",
        ] {
            let spans = parse(line);
            let mut at = 0;
            for s in &spans {
                assert_eq!(s.start, at, "{line:?} has a gap");
                at = s.end;
            }
            assert_eq!(at, line.chars().count(), "{line:?} not fully covered");
        }
    }

    #[test]
    fn headings_mark_the_hashes() {
        let line = "## Title";
        let spans = parse(line);
        assert_eq!(spans[0].kind, Kind::Marker);
        assert_eq!(text_of(line, &spans), vec!["## ", "Title"]);
        assert_eq!(spans[1].kind, Kind::Heading);
    }

    #[test]
    fn strong_and_emphasis() {
        assert_eq!(kinds("a **b** c"), vec![Kind::Normal, Kind::Marker, Kind::Strong, Kind::Marker, Kind::Normal]);
        assert_eq!(kinds("a *b* c"), vec![Kind::Normal, Kind::Marker, Kind::Emphasis, Kind::Marker, Kind::Normal]);
    }

    #[test]
    fn nested_emphasis_inside_strong() {
        let ks = kinds("**bold *ital* end**");
        assert!(ks.contains(&Kind::Strong));
        assert!(ks.contains(&Kind::StrongEmphasis));
    }

    #[test]
    fn inline_code_wins_over_emphasis() {
        let line = "use `a * b` here";
        let ks = kinds(line);
        assert!(ks.contains(&Kind::Code));
        assert!(!ks.contains(&Kind::Emphasis), "{ks:?}");
    }

    #[test]
    fn links_split_text_and_url() {
        let line = "see [docs](https://example.com) now";
        let spans = parse(line);
        let parts = text_of(line, &spans);
        assert!(parts.contains(&"docs".to_string()));
        assert!(parts.contains(&"(https://example.com)".to_string()));
        assert!(spans.iter().any(|s| s.kind == Kind::Link));
        assert!(spans.iter().any(|s| s.kind == Kind::LinkUrl));
    }

    #[test]
    fn rules_and_bullets() {
        assert_eq!(kinds("---"), vec![Kind::Rule]);
        assert_eq!(kinds("- item")[0], Kind::Bullet);
        assert_eq!(kinds("12. item")[0], Kind::Bullet);
    }

    #[test]
    fn fences_toggle_state() {
        let (spans, next) = parse_line("```rust", false);
        assert_eq!(spans[0].kind, Kind::Marker);
        assert!(next);

        let (spans, next) = parse_line("let x = 1; // **not bold**", true);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, Kind::Code);
        assert!(next);

        let (spans, next) = parse_line("```", true);
        assert_eq!(spans[0].kind, Kind::Marker);
        assert!(!next);
    }

    #[test]
    fn escaped_markers_are_literal() {
        let ks = kinds(r"a \*not emphasis\* b");
        assert!(!ks.contains(&Kind::Emphasis), "{ks:?}");
    }

    #[test]
    fn unterminated_markers_stay_normal() {
        assert_eq!(kinds("**oops"), vec![Kind::Normal]);
        assert_eq!(kinds("[label]"), vec![Kind::Normal]);
    }

    #[test]
    fn empty_input_is_safe() {
        assert!(parse("").is_empty());
    }
}
