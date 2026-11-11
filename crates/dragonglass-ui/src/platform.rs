//! Platform conventions for shortcuts and menus.
//!
//! Default shortcuts use GPUI's `secondary` modifier: Command on macOS, Control on Windows
//! and Linux. Saved user bindings keep whatever the user recorded. Labels are written the
//! way the platform writes them (`⌘⇧G` on macOS, `Ctrl+Shift+G` elsewhere).

/// Whether this build targets macOS.
pub const IS_MAC: bool = cfg!(target_os = "macos");

/// Canonical form of a keystroke string: lowercase, `-` separated.
///
/// GPUI only splits on `-`, so the `ctrl+shift+g` spelling people copy from docs would
/// otherwise become one long key name that never matches.
pub fn normalize(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace('+', "-")
}

/// Human-readable label for a keystroke string: `⌘⇧G` on macOS, `Ctrl+Shift+G` elsewhere.
pub fn label(key: &str) -> String {
    let key = normalize(key);
    if key.is_empty() {
        return String::new();
    }

    let (mut control, mut alt, mut shift, mut platform) = (false, false, false, false);
    let mut name = None;
    for part in key.split('-') {
        match part {
            "ctrl" | "control" => control = true,
            "alt" | "option" => alt = true,
            "shift" => shift = true,
            "cmd" | "command" | "super" | "win" | "platform" => platform = true,
            "secondary" => {
                if IS_MAC {
                    platform = true;
                } else {
                    control = true;
                }
            }
            "" => {}
            other => name = Some(other),
        }
    }

    let name = name.map(display_key).unwrap_or_default();

    if IS_MAC {
        let mut label = String::new();
        if control {
            label.push('⌃');
        }
        if alt {
            label.push('⌥');
        }
        if shift {
            label.push('⇧');
        }
        if platform {
            label.push('⌘');
        }
        label.push_str(&name);
        label
    } else {
        let mut parts = Vec::new();
        if control {
            parts.push("Ctrl");
        }
        if alt {
            parts.push("Alt");
        }
        if shift {
            parts.push("Shift");
        }
        if platform {
            parts.push("Cmd");
        }
        parts.push(&name);
        parts.join("+")
    }
}

fn display_key(key: &str) -> String {
    let symbol = match key {
        "up" => "↑",
        "down" => "↓",
        "left" => "←",
        "right" => "→",
        "tab" => "⇥",
        "escape" => "⎋",
        "backspace" => "⌫",
        "delete" => "⌦",
        "enter" => "↩",
        "space" => "Space",
        "pagedown" => "PgDn",
        "pageup" => "PgUp",
        "home" => "Home",
        "end" => "End",
        _ => {
            if key.len() == 1 {
                return key.to_ascii_uppercase();
            }
            return capitalize(key);
        }
    };
    symbol.to_string()
}

fn capitalize(key: &str) -> String {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_separators_and_case() {
        assert_eq!(normalize(" Ctrl+Shift+F "), "ctrl-shift-f");
        assert_eq!(normalize("cmd-shift-g"), "cmd-shift-g");
    }

    #[test]
    fn shortcuts_follow_the_platform() {
        assert_eq!(normalize("secondary-shift-g"), "secondary-shift-g");
        if IS_MAC {
            assert_eq!(label("secondary-shift-g"), "⇧⌘G");
        } else {
            assert_eq!(label("secondary-shift-g"), "Ctrl+Shift+G");
        }
    }

    #[test]
    fn labels_are_readable() {
        if IS_MAC {
            assert_eq!(label("cmd-shift-g"), "⇧⌘G");
            assert_eq!(label("ctrl-o"), "⌃O");
            assert_eq!(label("cmd-shift-left"), "⇧⌘←");
        } else {
            assert_eq!(label("ctrl-shift-g"), "Ctrl+Shift+G");
            assert_eq!(label("ctrl-o"), "Ctrl+O");
        }
    }
}
