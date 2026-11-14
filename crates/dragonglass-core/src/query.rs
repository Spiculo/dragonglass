//! A small query language over notes, for Bases/DataView-style tables.
//!
//! Grammar (clauses in any order):
//!
//! ```text
//! where <key> <value>     filter by frontmatter property (or `where tag <tag>`)
//! sort by <key> [desc]    order by a property
//! limit <n>               cap the number of rows
//! ```

use crate::{frontmatter, Note, NoteRef};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// `(key, value)` equality filters on frontmatter properties.
    pub filters: Vec<(String, String)>,
    pub tag: Option<String>,
    /// `(key, descending)`.
    pub sort: Option<(String, bool)>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub relative: String,
    pub title: String,
    pub values: Vec<(String, String)>,
}

pub fn parse(input: &str) -> Query {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let mut query = Query::default();
    let mut i = 0;

    while i < tokens.len() {
        match tokens[i] {
            "where" if i + 2 < tokens.len() => {
                let key = tokens[i + 1];
                let value = tokens[i + 2];
                if key.eq_ignore_ascii_case("tag") {
                    query.tag = Some(value.to_string());
                } else {
                    query.filters.push((key.to_string(), value.to_string()));
                }
                i += 3;
            }
            "sort" if i + 2 < tokens.len() && tokens[i + 1].eq_ignore_ascii_case("by") => {
                let key = tokens[i + 2].to_string();
                let desc = tokens
                    .get(i + 3)
                    .map(|t| t.eq_ignore_ascii_case("desc"))
                    .unwrap_or(false);
                query.sort = Some((key, desc));
                i += if desc { 4 } else { 3 };
            }
            "limit" if i + 1 < tokens.len() => {
                query.limit = tokens[i + 1].parse().ok();
                i += 2;
            }
            _ => i += 1,
        }
    }

    query
}

pub fn run(notes: &[NoteRef], input: &str) -> Vec<Row> {
    run_query(notes, &parse(input))
}

/// Run an already-parsed query. Lets clients filter by a property value that contains
/// spaces, which the string grammar cannot express.
pub fn run_query(notes: &[NoteRef], query: &Query) -> Vec<Row> {
    let mut rows: Vec<(Row, String)> = Vec::new();

    for note_ref in notes {
        let Ok(note) = Note::load(&note_ref.path) else {
            continue;
        };
        let properties = frontmatter::properties(&note.source).unwrap_or_default();

        if let Some(tag) = &query.tag {
            if !note
                .tags()
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(tag))
            {
                continue;
            }
        }

        let matches = query.filters.iter().all(|(key, value)| {
            properties
                .iter()
                .find(|p| p.key.eq_ignore_ascii_case(key))
                .map(|p| p.value.display().eq_ignore_ascii_case(value))
                .unwrap_or(false)
        });
        if !matches {
            continue;
        }

        let values: Vec<(String, String)> = properties
            .iter()
            .map(|p| (p.key.clone(), p.value.display()))
            .collect();

        let title = properties
            .iter()
            .find(|p| p.key.eq_ignore_ascii_case("title"))
            .map(|p| p.value.display())
            .unwrap_or_else(|| note_ref.stem().to_string());

        let sort_value = query
            .sort
            .as_ref()
            .and_then(|(key, _)| {
                properties
                    .iter()
                    .find(|p| p.key.eq_ignore_ascii_case(key))
                    .map(|p| p.value.display())
            })
            .unwrap_or_default()
            .to_lowercase();

        rows.push((
            Row {
                relative: note_ref.relative.clone(),
                title,
                values,
            },
            sort_value,
        ));
    }

    if let Some((_, descending)) = &query.sort {
        rows.sort_by(|a, b| {
            let ordering = a.1.cmp(&b.1);
            if *descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }

    let mut rows: Vec<Row> = rows.into_iter().map(|(row, _)| row).collect();
    if let Some(limit) = query.limit {
        rows.truncate(limit);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-query-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn note(dir: &std::path::Path, name: &str, body: &str) -> NoteRef {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        NoteRef {
            path,
            relative: name.to_string(),
        }
    }

    #[test]
    fn parses_clauses() {
        let query = parse("where status done where tag project sort by title desc limit 5");
        assert_eq!(query.filters, vec![("status".into(), "done".into())]);
        assert_eq!(query.tag.as_deref(), Some("project"));
        assert_eq!(query.sort, Some(("title".into(), true)));
        assert_eq!(query.limit, Some(5));
    }

    #[test]
    fn filters_sorts_and_limits() {
        let dir = temp_vault("run");
        let b = note(&dir, "b.md", "---\nstatus: done\ntitle: Beta\n---\nbody\n");
        let a = note(&dir, "a.md", "---\nstatus: done\ntitle: Alpha\n---\nbody\n");
        let c = note(&dir, "c.md", "---\nstatus: open\ntitle: Gamma\n---\nbody\n");
        let notes = vec![b, a, c];

        let rows = run(&notes, "where status done sort by title");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title, "Alpha");
        assert_eq!(rows[1].title, "Beta");

        let rows = run(&notes, "where status done sort by title desc limit 1");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Beta");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn filters_by_tag() {
        let dir = temp_vault("tag");
        let a = note(&dir, "a.md", "---\ntags: [project]\n---\nbody\n");
        let b = note(&dir, "b.md", "# Other\nbody #misc\n");
        let notes = vec![a, b];

        let rows = run(&notes, "where tag project");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "a");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
