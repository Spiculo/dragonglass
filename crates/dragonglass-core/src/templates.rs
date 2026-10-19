//! Minimal template substitution for newly created notes.
//!
//! Obsidian's templating is much richer; this covers the core variables needed for daily
//! notes and note scaffolding.

/// Replace `{{key}}` placeholders with the provided values.
pub fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in vars {
        out = out.replace(&format!("{{{{{}}}}}", key), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_placeholders() {
        let out = render(
            "# {{title}}\n\n{{date}} at {{time}}\n",
            &[
                ("title", "Today"),
                ("date", "2026-10-19"),
                ("time", "09:30"),
            ],
        );
        assert_eq!(out, "# Today\n\n2026-10-19 at 09:30\n");
    }

    #[test]
    fn leaves_unknown_placeholders() {
        assert_eq!(render("{{nope}}", &[]), "{{nope}}");
    }
}
