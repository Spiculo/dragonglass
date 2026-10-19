//! Decide what to do when a note's file changes on disk while it is open.

/// The action to take for an external change to the open note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// The file now matches the buffer; nothing to do.
    Ignore,
    /// The buffer has no unsaved edits; adopt the file.
    Reload,
    /// The buffer has unsaved edits that would be lost; warn the user.
    Conflict,
}

/// Classify an external change given the in-memory buffer and its dirty state.
pub fn classify(buffer: &str, dirty: bool, disk: &str) -> Resolution {
    if buffer == disk {
        Resolution::Ignore
    } else if dirty {
        Resolution::Conflict
    } else {
        Resolution::Reload
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_content_is_ignored() {
        assert_eq!(classify("same", true, "same"), Resolution::Ignore);
    }

    #[test]
    fn clean_buffer_is_reloaded() {
        assert_eq!(classify("old", false, "new"), Resolution::Reload);
    }

    #[test]
    fn dirty_buffer_conflicts() {
        assert_eq!(classify("mine", true, "theirs"), Resolution::Conflict);
    }
}
