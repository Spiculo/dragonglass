//! YAML frontmatter detection, splitting, and a deliberately small property model.
//!
//! We do not parse full YAML. Instead we recognise the simple shapes Obsidian properties
//! use — scalars, inline lists and block lists — and refuse anything else rather than risk
//! rewriting a user's frontmatter incorrectly.

use crate::{Error, Result};

/// Result of splitting a document into its frontmatter and body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Split<'a> {
    /// Raw YAML between the `---` fences, if present.
    pub frontmatter: Option<&'a str>,
    /// Everything after the closing fence (or the whole document).
    pub body: &'a str,
}

/// Split a document at its leading `---` frontmatter block.
///
/// Only a fence on the very first line counts, matching Obsidian/YAML frontmatter.
/// BOM is tolerated.
pub fn split(src: &str) -> Split<'_> {
    let s = src.strip_prefix('\u{feff}').unwrap_or(src);

    let Some(after_open) = s
        .strip_prefix("---\r\n")
        .or_else(|| s.strip_prefix("---\n"))
    else {
        return Split {
            frontmatter: None,
            body: src,
        };
    };

    let open_len = s.len() - after_open.len();
    let mut cur = open_len;

    for line in after_open.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" || trimmed == "..." {
            return Split {
                frontmatter: Some(&s[open_len..cur]),
                body: &s[cur + line.len()..],
            };
        }
        cur += line.len();
    }

    // Unterminated fence: treat the whole document as body.
    Split {
        frontmatter: None,
        body: src,
    }
}

/// A single frontmatter property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub key: String,
    pub value: PropertyValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyValue {
    Scalar(String),
    List(Vec<String>),
}

impl PropertyValue {
    /// Human-readable one-line form, used by the UI.
    pub fn display(&self) -> String {
        match self {
            PropertyValue::Scalar(value) => value.clone(),
            PropertyValue::List(items) => items.join(", "),
        }
    }

    /// Parse a UI-entered value: `[a, b]` becomes a list, empty becomes an empty list,
    /// anything else a scalar.
    pub fn from_input(input: &str) -> Self {
        let input = input.trim();
        if input.starts_with('[') && input.ends_with(']') {
            let inner = &input[1..input.len() - 1];
            let items = inner
                .split(',')
                .map(|s| unquote(s.trim()))
                .filter(|s| !s.is_empty())
                .collect();
            PropertyValue::List(items)
        } else if input.is_empty() {
            PropertyValue::List(Vec::new())
        } else {
            PropertyValue::Scalar(unquote(input))
        }
    }
}

/// Parse the document's frontmatter into properties.
///
/// Errors on constructs we do not understand, so callers never silently drop YAML.
pub fn properties(src: &str) -> Result<Vec<Property>> {
    let Some(frontmatter) = split(src).frontmatter else {
        return Ok(Vec::new());
    };

    let mut properties: Vec<Property> = Vec::new();
    let mut open_list: Option<usize> = None;

    for (index, raw) in frontmatter.lines().enumerate() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if line.len() != trimmed.len() {
            // Indented: only a list item belonging to the preceding empty key is valid.
            if let Some(item) = trimmed.strip_prefix("- ") {
                if let Some(prop) = open_list.and_then(|i| properties.get_mut(i)) {
                    if let PropertyValue::List(items) = &mut prop.value {
                        items.push(unquote(item.trim()));
                        continue;
                    }
                }
            }
            return Err(Error::UnsupportedFrontmatter {
                line: index + 1,
                text: trimmed.to_string(),
            });
        }

        let Some((key, value)) = line.split_once(':') else {
            return Err(Error::UnsupportedFrontmatter {
                line: index + 1,
                text: line.to_string(),
            });
        };

        let key = key.trim();
        if key.is_empty() {
            return Err(Error::UnsupportedFrontmatter {
                line: index + 1,
                text: line.to_string(),
            });
        }

        let value = value.trim();
        open_list = None;

        if value.is_empty() {
            properties.push(Property {
                key: key.to_string(),
                value: PropertyValue::List(Vec::new()),
            });
            open_list = Some(properties.len() - 1);
        } else {
            properties.push(Property {
                key: key.to_string(),
                value: PropertyValue::from_input(value),
            });
        }
    }

    Ok(properties)
}

/// Rewrite the document with `properties` as its frontmatter, keeping the body.
pub fn with_properties(src: &str, properties: &[Property]) -> String {
    let body = split(src).body;

    if properties.is_empty() {
        return body.to_string();
    }

    let mut out = String::from("---\n");
    for property in properties {
        match &property.value {
            PropertyValue::Scalar(value) => {
                out.push_str(&property.key);
                out.push_str(": ");
                out.push_str(value);
                out.push('\n');
            }
            PropertyValue::List(items) => {
                if items.is_empty() {
                    out.push_str(&property.key);
                    out.push_str(": []\n");
                } else {
                    out.push_str(&property.key);
                    out.push_str(": [");
                    out.push_str(&items.join(", "));
                    out.push_str("]\n");
                }
            }
        }
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_frontmatter() {
        let s = "# Title\n\nbody\n";
        let r = split(s);
        assert_eq!(r.frontmatter, None);
        assert_eq!(r.body, s);
    }

    #[test]
    fn basic_frontmatter() {
        let s = "---\ntags: [a, b]\n---\n# Title\n";
        let r = split(s);
        assert_eq!(r.frontmatter, Some("tags: [a, b]\n"));
        assert_eq!(r.body, "# Title\n");
    }

    #[test]
    fn unterminated_fence_is_body() {
        let s = "---\ntags: a\n";
        let r = split(s);
        assert_eq!(r.frontmatter, None);
        assert_eq!(r.body, s);
    }

    #[test]
    fn tolerates_bom() {
        let s = "\u{feff}---\ntitle: x\n---\nbody";
        let r = split(s);
        assert_eq!(r.frontmatter, Some("title: x\n"));
        assert_eq!(r.body, "body");
    }

    #[test]
    fn parses_scalars_and_lists() {
        let src =
            "---\ntitle: Hello\ntags: [a, b]\naliases:\n  - one\n  - two\nempty: []\n---\nbody\n";
        let found = properties(src).unwrap();
        assert_eq!(found.len(), 4);
        assert_eq!(
            found[0],
            Property {
                key: "title".into(),
                value: PropertyValue::Scalar("Hello".into())
            }
        );
        assert_eq!(
            found[1].value,
            PropertyValue::List(vec!["a".into(), "b".into()])
        );
        assert_eq!(
            found[2].value,
            PropertyValue::List(vec!["one".into(), "two".into()])
        );
        assert_eq!(found[3].value, PropertyValue::List(Vec::new()));
    }

    #[test]
    fn no_frontmatter_has_no_properties() {
        assert!(properties("body only").unwrap().is_empty());
    }

    #[test]
    fn rejects_nested_mappings() {
        let src = "---\nnested:\n  child: x\n---\n";
        assert!(properties(src).is_err());
    }

    #[test]
    fn with_properties_replaces_frontmatter_and_keeps_body() {
        let src = "---\nold: value\n---\n# Body\n";
        let props = vec![
            Property {
                key: "title".into(),
                value: PropertyValue::Scalar("T".into()),
            },
            Property {
                key: "tags".into(),
                value: PropertyValue::List(vec!["a".into(), "b".into()]),
            },
        ];
        assert_eq!(
            with_properties(src, &props),
            "---\ntitle: T\ntags: [a, b]\n---\n# Body\n"
        );
    }

    #[test]
    fn removing_all_properties_keeps_body() {
        assert_eq!(with_properties("---\na: b\n---\nbody", &[]), "body");
    }
}
