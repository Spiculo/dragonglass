//! `dragonglass://` URIs, used for OS integration and automation.
//!
//! ```text
//! dragonglass://open?vault=/path/to/vault&note=notes/a.md
//! dragonglass://search?vault=/path/to/vault&query=widgets
//! ```

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uri {
    /// `open`, `new`, `search`, …
    pub action: String,
    pub vault: Option<String>,
    pub note: Option<String>,
    pub query: Option<String>,
}

pub fn parse(input: &str) -> Option<Uri> {
    let rest = input.trim().strip_prefix("dragonglass://")?;

    let (action, parameters) = match rest.split_once('?') {
        Some((action, parameters)) => (action, Some(parameters)),
        None => (rest, None),
    };

    let action = action.trim_end_matches('/').trim().to_string();
    if action.is_empty() {
        return None;
    }

    let mut uri = Uri {
        action,
        vault: None,
        note: None,
        query: None,
    };

    for pair in parameters.unwrap_or("").split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let value = percent_decode(value);
        if value.is_empty() {
            continue;
        }

        match key.trim() {
            "vault" => uri.vault = Some(value),
            "note" | "path" => uri.note = Some(value),
            "query" | "q" => uri.query = Some(value),
            _ => {}
        }
    }

    Some(uri)
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_open_uri() {
        let uri = parse("dragonglass://open?vault=/vaults/notes&note=notes%2Fa.md").unwrap();
        assert_eq!(uri.action, "open");
        assert_eq!(uri.vault.as_deref(), Some("/vaults/notes"));
        assert_eq!(uri.note.as_deref(), Some("notes/a.md"));
    }

    #[test]
    fn parses_a_search_uri_with_plus_decoding() {
        let uri = parse("dragonglass://search?vault=v&query=hello+world").unwrap();
        assert_eq!(uri.action, "search");
        assert_eq!(uri.query.as_deref(), Some("hello world"));
    }

    #[test]
    fn tolerates_a_missing_query_string() {
        let uri = parse("dragonglass://open").unwrap();
        assert_eq!(uri.action, "open");
        assert!(uri.note.is_none());
    }

    #[test]
    fn rejects_other_schemes_and_empty_actions() {
        assert!(parse("https://example.com").is_none());
        assert!(parse("dragonglass://").is_none());
    }
}
