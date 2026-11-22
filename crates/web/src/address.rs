//! What the writer typed after `web`: an address or words to search for.
//!
//! Only `http` and `https` are ever navigated to from the prompt. Anything that
//! looks like another scheme (`file:`, `javascript:`, `chrome:`) is searched
//! for instead, so a typo cannot open the machine's own files in a page.

/// Where searches go. DuckDuckGo needs no account and no cookie banner.
pub const SEARCH: &str = "https://duckduckgo.com/?q=";

/// The page a fresh browser opens on.
pub const HOME: &str = "https://duckduckgo.com/";

/// Turn the prompt's text into a URL.
pub fn resolve(input: &str) -> String {
    let text = input.trim();
    if text.is_empty() {
        return HOME.to_string();
    }
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return text.to_string();
    }
    if !text.contains(char::is_whitespace) && looks_like_host(host(text)) {
        let scheme = if is_local(host(text)) {
            "http"
        } else {
            "https"
        };
        return format!("{scheme}://{text}");
    }
    format!("{SEARCH}{}", form_encode(text))
}

/// The host part of a URL or of a bare `host/path`, without port or user.
pub fn host(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    if authority.starts_with('[') {
        // IPv6 literal: keep the brackets, drop the port.
        return authority
            .split_once(']')
            .map(|(h, _)| &authority[..h.len() + 1])
            .unwrap_or(authority);
    }
    authority.split(':').next().unwrap_or("")
}

/// Is this page encrypted on the way here?
pub fn is_secure(url: &str) -> bool {
    url.to_ascii_lowercase().starts_with("https://")
}

fn looks_like_host(host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    if is_local(host) {
        return true;
    }
    // `de.wikipedia.org`, `192.168.1.10`: a dot with something on both sides,
    // and nothing that cannot be in a host name.
    let valid = host
        .chars()
        .all(|c| c.is_alphanumeric() || c == '.' || c == '-');
    let parts: Vec<&str> = host.split('.').collect();
    valid && parts.len() >= 2 && parts.iter().all(|p| !p.is_empty())
}

fn is_local(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host.starts_with("127.") || host == "[::1]"
}

/// `application/x-www-form-urlencoded`, which is what a search box sends.
fn form_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_get_https() {
        assert_eq!(resolve("de.wikipedia.org"), "https://de.wikipedia.org");
        assert_eq!(
            resolve("de.wikipedia.org/wiki/IBM_PS/2"),
            "https://de.wikipedia.org/wiki/IBM_PS/2"
        );
        assert_eq!(resolve("  example.com  "), "https://example.com");
    }

    #[test]
    fn typed_schemes_are_kept() {
        assert_eq!(resolve("http://example.com"), "http://example.com");
        assert_eq!(resolve("HTTPS://Example.com/A"), "HTTPS://Example.com/A");
    }

    #[test]
    fn the_local_machine_is_plain_http() {
        assert_eq!(resolve("localhost:8080"), "http://localhost:8080");
        assert_eq!(resolve("127.0.0.1/x"), "http://127.0.0.1/x");
    }

    #[test]
    fn words_are_searched() {
        assert_eq!(
            resolve("ibm ps/2 model 30"),
            "https://duckduckgo.com/?q=ibm+ps%2F2+model+30"
        );
        assert_eq!(resolve("rust"), "https://duckduckgo.com/?q=rust");
        assert_eq!(resolve("ä"), "https://duckduckgo.com/?q=%C3%A4");
    }

    #[test]
    fn other_schemes_are_never_opened() {
        for typed in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "chrome://settings",
            "data:text/html,hi",
        ] {
            assert!(resolve(typed).starts_with(SEARCH), "{typed}");
        }
    }

    #[test]
    fn empty_goes_home() {
        assert_eq!(resolve(""), HOME);
    }

    #[test]
    fn the_host_is_what_the_bar_shows() {
        assert_eq!(host("https://de.wikipedia.org/wiki/X"), "de.wikipedia.org");
        assert_eq!(host("https://user:pw@evil.example:8443/?q"), "evil.example");
        assert_eq!(host("http://[::1]:8080/"), "[::1]");
        assert_eq!(host("about:blank"), "about");
        assert!(is_secure("https://a.b"));
        assert!(!is_secure("http://a.b"));
    }
}
