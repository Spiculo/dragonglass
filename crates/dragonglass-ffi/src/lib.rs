//! UniFFI bindings for the headless Dragon Glass engine.
//!
//! The desktop app talks to `dragonglass-core` directly. Mobile clients cannot, so this
//! crate exposes a small, stable, mobile-shaped API over the same engine. It is deliberately
//! synchronous: Kotlin and Swift callers run these methods off the UI thread.

uniffi::setup_scaffolding!("dragonglass");

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use dragonglass_core::assets::Attachments;
use dragonglass_core::fingerprint::{self, DocumentFingerprint, SaveReport};
use dragonglass_core::graph::{self, EdgeKind, Graph};
use dragonglass_core::relations::{self, NoteRelations};
use dragonglass_core::{
    bookmarks, config, drafts, find, frontmatter, html, markdown, query, search, stats, storage,
    templates, tree, vaultfs, NodeKind, Note, NoteRef, TreeNode, Vault,
};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MobileError {
    #[error("{reason}")]
    Failure { reason: String },
}

impl MobileError {
    fn new(message: impl Into<String>) -> Self {
        Self::Failure {
            reason: message.into(),
        }
    }
}

impl From<dragonglass_core::Error> for MobileError {
    fn from(error: dragonglass_core::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<std::io::Error> for MobileError {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}

/// A note as shown in a list. Paths are vault-relative and always use `/`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NoteSummary {
    pub relative: String,
    pub title: String,
}

/// A flattened sidebar row, carrying its depth instead of a recursive tree.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TreeRow {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub depth: i32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct SearchHit {
    pub relative: String,
    pub line: i32,
    pub text: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct HeadingInfo {
    pub level: i32,
    pub text: String,
    pub line: i32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PropertyInfo {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct StatsInfo {
    pub words: i32,
    pub characters: i32,
    pub sentences: i32,
    pub paragraphs: i32,
    pub reading_minutes: i32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct QueryRow {
    pub relative: String,
    pub title: String,
    pub values: Vec<PropertyInfo>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct GraphNode {
    pub id: i32,
    pub relative: String,
    pub links: i32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct GraphEdge {
    pub from: i32,
    pub to: i32,
    /// `link`, `embed` or `relation`.
    pub kind: String,
    /// The frontmatter key for relations, otherwise empty.
    pub label: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct GraphView {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct RelationInfo {
    pub kind: String,
    pub target: String,
    pub label: Option<String>,
    pub line: Option<i32>,
    pub incoming: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TagCount {
    pub tag: String,
    pub notes: i32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TemplateInfo {
    pub name: String,
    pub relative: String,
}

/// A deleted note or folder waiting in the vault trash.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TrashEntry {
    /// Vault-relative path of the trash entry, for `restore_trash_entry`.
    pub trash_relative: String,
    /// Where the entry was before it was deleted.
    pub original: String,
    /// Deletion time in milliseconds since the Unix epoch, when known.
    pub deleted_at: i64,
}

struct State {
    notes: Vec<NoteRef>,
    graph: Option<Graph>,
    relations: Option<Vec<NoteRelations>>,
}

/// A vault opened by the mobile app.
#[derive(uniffi::Object)]
pub struct MobileVault {
    root: PathBuf,
    state: Mutex<State>,
}

/// The identity of one version of a note on disk.
#[derive(Debug, Clone, Copy, uniffi::Record)]
pub struct FingerprintInfo {
    pub size: u64,
    /// Modification time in Unix milliseconds (0 when unknown).
    pub modified_ms: u64,
    pub content_hash: u64,
}

/// The result of a conflict-safe save.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SaveReportInfo {
    /// `saved`, `conflict` or `error`.
    pub status: String,
    pub fingerprint: Option<FingerprintInfo>,
    /// Content hash of the version on disk when the save conflicted.
    pub disk_hash: Option<u64>,
    pub message: Option<String>,
}

/// A recovery draft for one note.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DraftInfo {
    pub text: String,
    pub line: u32,
    pub col: u32,
    pub written_at_ms: u64,
    pub base_hash: Option<u64>,
}

/// Reading typography, shared with the desktop reader.
#[derive(Debug, Clone, Copy, uniffi::Record)]
pub struct ReaderSettingsInfo {
    pub body_size: u32,
    pub measure: u32,
}

fn fingerprint_info(info: DocumentFingerprint) -> FingerprintInfo {
    let modified_ms = info
        .modified
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    FingerprintInfo {
        size: info.size,
        modified_ms,
        content_hash: info.content_hash,
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// The shared sync state, so Android and iOS render one state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncStateInfo {
    Idle,
    Dirty,
    Syncing,
    Conflict,
    Error,
}

#[uniffi::export]
pub fn sync_state(
    changed: u32,
    untracked: u32,
    ahead: u32,
    behind: u32,
    conflicts: u32,
    busy: bool,
    failed: bool,
) -> SyncStateInfo {
    let snapshot = dragonglass_core::git::SyncSnapshot {
        changed: changed as usize,
        untracked: untracked as usize,
        ahead: ahead as usize,
        behind: behind as usize,
        conflicts: conflicts as usize,
    };
    match dragonglass_core::git::SyncState::of(snapshot, busy, failed) {
        dragonglass_core::git::SyncState::Idle => SyncStateInfo::Idle,
        dragonglass_core::git::SyncState::Dirty => SyncStateInfo::Dirty,
        dragonglass_core::git::SyncState::Syncing => SyncStateInfo::Syncing,
        dragonglass_core::git::SyncState::Conflict => SyncStateInfo::Conflict,
        dragonglass_core::git::SyncState::Error => SyncStateInfo::Error,
    }
}

/// The 1-based source line a `#Heading` or `#^block` fragment points at, if present.
#[uniffi::export]
pub fn resolve_link_fragment(source: String, fragment: String) -> Option<u32> {
    let fragment = fragment.trim().trim_start_matches('#').trim();
    if fragment.is_empty() {
        return None;
    }
    if let Some(id) = fragment.strip_prefix('^') {
        let marker = format!("^{id}");
        return source
            .lines()
            .position(|line| line.trim_end().ends_with(&marker))
            .map(|index| index as u32 + 1);
    }
    let wanted = fragment.to_lowercase();
    markdown::headings(&source)
        .into_iter()
        .find(|heading| heading.text.trim().to_lowercase() == wanted)
        .map(|heading| heading.line as u32)
}

#[uniffi::export]
impl MobileVault {
    /// Open an existing directory of markdown notes. Mobile apps normally pass their
    /// app-specific documents directory here.
    #[uniffi::constructor]
    pub fn open(root: String) -> Result<std::sync::Arc<Self>, MobileError> {
        let vault = Vault::open(&root)?;
        let root = vault.root().to_path_buf();
        let notes = vault.notes();
        Ok(std::sync::Arc::new(Self {
            root,
            state: Mutex::new(State {
                notes,
                graph: None,
                relations: None,
            }),
        }))
    }

    pub fn root(&self) -> String {
        self.root.to_string_lossy().to_string()
    }

    /// Re-scan the vault after files changed on disk.
    pub fn refresh(&self) -> Result<(), MobileError> {
        let notes = Vault::open(&self.root)?.notes();
        let mut state = self.lock()?;
        state.notes = notes;
        state.graph = None;
        state.relations = None;
        Ok(())
    }

    pub fn notes(&self) -> Vec<NoteSummary> {
        let Ok(state) = self.lock() else {
            return Vec::new();
        };
        state
            .notes
            .iter()
            .map(|note| NoteSummary {
                relative: note.relative.clone(),
                title: note.stem().to_string(),
            })
            .collect()
    }

    /// Sidebar rows, directories first, with each row's indentation depth.
    pub fn tree_rows(&self) -> Vec<TreeRow> {
        let Ok(state) = self.lock() else {
            return Vec::new();
        };
        let folders = Vault::open(&self.root)
            .map(|vault| vault.folders())
            .unwrap_or_default();
        let tree = tree::build_with_dirs(&state.notes, &folders);
        let mut rows = Vec::new();
        flatten_tree(&tree, 0, &mut rows);
        rows
    }

    /// Fingerprint a note on disk, for a later conflict-safe save.
    pub fn document_fingerprint(&self, relative: String) -> Result<FingerprintInfo, MobileError> {
        let path = self.readable_path(&relative)?;
        Ok(fingerprint_info(DocumentFingerprint::of_path(&path)?))
    }

    /// Save a note only if its content still hashes to `expected_hash`.
    ///
    /// `expected_hash = None` means "no baseline" and writes. A conflict leaves the disk
    /// version untouched and reports its hash so the app can offer Reload / Keep mine.
    pub fn save_document_with_fingerprint(
        &self,
        relative: String,
        text: String,
        expected_hash: Option<u64>,
    ) -> SaveReportInfo {
        let path = match self.writable_path(&relative) {
            Ok(path) => path,
            Err(error) => {
                return SaveReportInfo {
                    status: "error".to_string(),
                    fingerprint: None,
                    disk_hash: None,
                    message: Some(error.to_string()),
                };
            }
        };
        let expected = expected_hash.map(|content_hash| DocumentFingerprint {
            size: 0,
            modified: None,
            content_hash,
        });
        match fingerprint::save_if_unchanged(&path, &text, expected.as_ref()) {
            SaveReport::Saved { fingerprint } => {
                let _ = self.refresh();
                SaveReportInfo {
                    status: "saved".to_string(),
                    fingerprint: Some(fingerprint_info(fingerprint)),
                    disk_hash: None,
                    message: None,
                }
            }
            SaveReport::Conflict { disk } => SaveReportInfo {
                status: "conflict".to_string(),
                fingerprint: None,
                disk_hash: disk.map(|fingerprint| fingerprint.content_hash),
                message: None,
            },
            SaveReport::Error { message } => SaveReportInfo {
                status: "error".to_string(),
                fingerprint: None,
                disk_hash: None,
                message: Some(message),
            },
        }
    }

    /// Write a recovery draft for a note (call from an idle/lifecycle hook).
    pub fn save_draft(
        &self,
        relative: String,
        text: String,
        line: u32,
        col: u32,
    ) -> Result<(), MobileError> {
        let base = self
            .readable_path(&relative)
            .ok()
            .and_then(|path| DocumentFingerprint::of_path(&path).ok())
            .map(|fingerprint| fingerprint.content_hash);
        drafts::save(
            &self.root,
            &drafts::Draft {
                note: relative,
                line: line as usize,
                col: col as usize,
                at: now_millis(),
                base,
                text,
            },
        )?;
        Ok(())
    }

    /// The draft for a note, if one is newer than what the app last saw.
    pub fn load_draft(&self, relative: String) -> Option<DraftInfo> {
        drafts::load(&self.root, &relative).map(|draft| DraftInfo {
            text: draft.text,
            line: draft.line as u32,
            col: draft.col as u32,
            written_at_ms: draft.at,
            base_hash: draft.base,
        })
    }

    pub fn discard_draft(&self, relative: String) {
        drafts::discard(&self.root, &relative);
    }

    /// Reading typography from the vault config.
    pub fn reader_settings(&self) -> ReaderSettingsInfo {
        let config = config::load(&self.root);
        ReaderSettingsInfo {
            body_size: config.reader_size as u32,
            measure: config.reader_width as u32,
        }
    }

    pub fn set_reader_settings(&self, body_size: u32, measure: u32) -> Result<(), MobileError> {
        let mut config = config::load(&self.root);
        config.reader_size = body_size.clamp(11, 24) as u16;
        config.reader_width = measure.clamp(480, 1200) as u16;
        config::save(&self.root, &config)?;
        Ok(())
    }

    pub fn note_source(&self, relative: String) -> Result<String, MobileError> {
        let path = self.readable_path(&relative)?;
        Ok(fs::read_to_string(path)?)
    }

    /// Render a note as a standalone HTML document for a WebView.
    pub fn note_html(&self, relative: String) -> Result<String, MobileError> {
        let source = self.note_source(relative)?;
        Ok(render_html(source))
    }

    pub fn save_note(&self, relative: String, source: String) -> Result<(), MobileError> {
        let path = self.writable_path(&relative)?;
        storage::write_atomic(&path, &source)?;
        self.refresh()
    }

    /// Create a note and fail rather than overwrite an existing one.
    pub fn create_note(&self, relative: String, source: String) -> Result<(), MobileError> {
        let path = self.writable_path(&relative)?;
        if path.exists() {
            return Err(MobileError::new(format!("{relative} already exists")));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(source.as_bytes())?;
        file.sync_all()?;
        self.refresh()
    }

    /// Create a folder (and any missing parents).
    pub fn create_folder(&self, relative: String) -> Result<(), MobileError> {
        vaultfs::create_folder(&self.root, &relative)?;
        self.refresh()
    }

    /// Rename or move a note or folder. Fails rather than overwriting another entry.
    /// Returns the sanitized vault-relative destination.
    pub fn rename_entry(&self, from: String, to: String) -> Result<String, MobileError> {
        let renamed = vaultfs::rename(&self.root, &from, &to)?;
        self.refresh()?;
        Ok(renamed)
    }

    /// Move a note or folder to the vault trash. Returns its trash-relative path.
    pub fn trash_note(&self, relative: String) -> Result<String, MobileError> {
        let path = vaultfs::trash(&self.root, &relative)?;
        let trashed = path
            .strip_prefix(&self.root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        self.refresh()?;
        Ok(trashed)
    }

    /// Everything currently in the vault trash, newest first.
    pub fn trash_entries(&self) -> Vec<TrashEntry> {
        vaultfs::list_trash(&self.root)
            .unwrap_or_default()
            .into_iter()
            .map(|entry| TrashEntry {
                trash_relative: entry.trash_relative,
                original: entry.original,
                deleted_at: entry.deleted_at_ms as i64,
            })
            .collect()
    }

    /// Restore one trash entry to its original path. Returns that path.
    pub fn restore_trash_entry(&self, trash_relative: String) -> Result<String, MobileError> {
        let restored = vaultfs::restore(&self.root, &trash_relative)?;
        self.refresh()?;
        Ok(restored)
    }

    /// Permanently remove everything in the trash. Returns the entry count.
    pub fn empty_trash(&self) -> Result<i64, MobileError> {
        Ok(vaultfs::empty_trash(&self.root)? as i64)
    }

    /// Full-text search, capped at `limit` hits.
    pub fn search(&self, query: String, limit: i32) -> Vec<SearchHit> {
        let Ok(state) = self.lock() else {
            return Vec::new();
        };
        search::full_text(&state.notes, &query, limit.max(0) as usize)
            .into_iter()
            .map(|hit| SearchHit {
                relative: hit.relative,
                line: hit.line_number as i32,
                text: hit.line,
            })
            .collect()
    }

    /// Quick-switcher style fuzzy path matching.
    pub fn fuzzy(&self, query: String, limit: i32) -> Vec<NoteSummary> {
        let Ok(state) = self.lock() else {
            return Vec::new();
        };
        search::fuzzy_notes(&state.notes, &query, limit.max(0) as usize)
            .into_iter()
            .map(|note| NoteSummary {
                relative: note.relative.clone(),
                title: note.stem().to_string(),
            })
            .collect()
    }

    /// Run a Bases/DataView-style query such as `where tag project sort by title`.
    pub fn query(&self, expression: String) -> Vec<QueryRow> {
        let Ok(state) = self.lock() else {
            return Vec::new();
        };
        query::run(&state.notes, &expression)
            .into_iter()
            .map(|row| QueryRow {
                relative: row.relative,
                title: row.title,
                values: row
                    .values
                    .into_iter()
                    .map(|(key, value)| PropertyInfo { key, value })
                    .collect(),
            })
            .collect()
    }

    pub fn graph(&self) -> GraphView {
        let Ok(mut state) = self.lock() else {
            return GraphView {
                nodes: Vec::new(),
                edges: Vec::new(),
            };
        };
        if state.graph.is_none() {
            let graph = graph::build(&state.notes);
            state.graph = Some(graph);
        }
        let Some(graph) = state.graph.as_ref() else {
            return GraphView {
                nodes: Vec::new(),
                edges: Vec::new(),
            };
        };

        GraphView {
            nodes: graph
                .nodes
                .iter()
                .map(|node| GraphNode {
                    id: node.id as i32,
                    relative: node.relative.clone(),
                    links: node.links as i32,
                })
                .collect(),
            edges: graph
                .edges
                .iter()
                .map(|edge| GraphEdge {
                    from: edge.from as i32,
                    to: edge.to as i32,
                    kind: match edge.kind {
                        EdgeKind::Link => "link".to_string(),
                        EdgeKind::Embed => "embed".to_string(),
                        EdgeKind::Relation(_) => "relation".to_string(),
                    },
                    label: match edge.kind {
                        EdgeKind::Relation(label) => label.to_string(),
                        _ => String::new(),
                    },
                })
                .collect(),
        }
    }

    /// Outgoing and incoming relations for one note.
    pub fn relations(&self, relative: String) -> Vec<RelationInfo> {
        let Ok(mut state) = self.lock() else {
            return Vec::new();
        };
        if state.relations.is_none() {
            let built = relations::build(&state.notes);
            state.relations = Some(built);
        }
        let Some(entries) = state
            .relations
            .as_ref()
            .and_then(|all| relations::for_note(all, &relative))
        else {
            return Vec::new();
        };

        let mut out = Vec::new();
        for relation in &entries.outgoing {
            out.push(RelationInfo {
                kind: relation.kind.label().to_string(),
                target: relation.target.clone(),
                label: relation.label.clone(),
                line: relation.line.map(|line| line as i32),
                incoming: false,
            });
        }
        for relation in &entries.incoming {
            out.push(RelationInfo {
                kind: relation.kind.label().to_string(),
                target: relation.target.clone(),
                label: relation.label.clone(),
                line: relation.line.map(|line| line as i32),
                incoming: true,
            });
        }
        for (tag, notes) in &entries.shared_tags {
            out.push(RelationInfo {
                kind: "shared tag".to_string(),
                target: tag.clone(),
                label: Some(notes.join(", ")),
                line: None,
                incoming: false,
            });
        }
        out
    }

    /// All tags in the vault, with the number of notes that use each one.
    pub fn tags(&self) -> Vec<TagCount> {
        let Ok(state) = self.lock() else {
            return Vec::new();
        };
        let mut counts: std::collections::BTreeMap<String, i32> = std::collections::BTreeMap::new();
        for note in &state.notes {
            let Ok(loaded) = Note::load(&note.path) else {
                continue;
            };
            for tag in loaded.tags() {
                *counts.entry(tag).or_insert(0) += 1;
            }
        }
        counts
            .into_iter()
            .map(|(tag, notes)| TagCount { tag, notes })
            .collect()
    }

    /// Notes bookmarked in this vault.
    pub fn bookmarks(&self) -> Vec<String> {
        bookmarks::load(&self.root)
    }

    /// Toggle a bookmark. Returns true when the note is now bookmarked.
    pub fn toggle_bookmark(&self, relative: String) -> Result<bool, MobileError> {
        self.readable_path(&relative)?;
        let mut list = bookmarks::load(&self.root);
        let bookmarked = bookmarks::toggle(&mut list, &relative);
        bookmarks::save(&self.root, &list)?;
        Ok(bookmarked)
    }

    /// Markdown templates in the vault's `Templates` folder.
    pub fn templates(&self) -> Vec<TemplateInfo> {
        templates::list(&self.root, "Templates")
            .into_iter()
            .map(|template| TemplateInfo {
                name: template.name,
                relative: template.relative,
            })
            .collect()
    }

    /// Create a note from a template, substituting the standard variables.
    pub fn create_from_template(
        &self,
        template: String,
        title: String,
    ) -> Result<String, MobileError> {
        let source = self.note_source(template)?;
        let title = title.trim();
        if title.is_empty() {
            return Err(MobileError::new("the note title cannot be empty"));
        }
        let relative = format!("{}.md", sanitize_name(title));
        if self.writable_path(&relative)?.exists() {
            return Err(MobileError::new(format!("{relative} already exists")));
        }
        let text = self.substitute(&source, title);
        self.create_note(relative.clone(), text)?;
        Ok(relative)
    }

    /// Open or create today's daily note, using `Templates/Daily.md` when present.
    pub fn open_daily(&self) -> Result<String, MobileError> {
        self.open_periodic("daily", "%Y-%m-%d")
    }

    /// Open or create this week's note.
    pub fn open_weekly(&self) -> Result<String, MobileError> {
        self.open_periodic("weekly", "%G-W%V")
    }

    /// Open or create this month's note.
    pub fn open_monthly(&self) -> Result<String, MobileError> {
        self.open_periodic("monthly", "%Y-%m")
    }
}

impl MobileVault {
    fn lock(&self) -> Result<MutexGuard<'_, State>, MobileError> {
        self.state
            .lock()
            .map_err(|_| MobileError::new("vault state lock poisoned"))
    }

    /// Reject paths that could escape the vault before touching the filesystem.
    fn writable_path(&self, relative: &str) -> Result<PathBuf, MobileError> {
        let relative = relative.replace('\\', "/");
        if relative.is_empty()
            || relative
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(MobileError::new(format!("invalid note path: {relative}")));
        }
        Ok(self.root.join(relative))
    }

    fn readable_path(&self, relative: &str) -> Result<PathBuf, MobileError> {
        let path = self.writable_path(relative)?;
        let resolved = path
            .canonicalize()
            .map_err(|_| MobileError::new(format!("note not found: {relative}")))?;
        if !resolved.starts_with(&self.root) {
            return Err(MobileError::new("path outside vault"));
        }
        Ok(resolved)
    }

    fn vault_name(&self) -> String {
        self.root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string()
    }

    /// Substitute the template variables shared by daily notes and user templates.
    fn substitute(&self, template: &str, title: &str) -> String {
        let now = chrono::Local::now();
        template
            .replace("{{title}}", title)
            .replace("{{date}}", &now.format("%Y-%m-%d").to_string())
            .replace("{{time}}", &now.format("%H:%M").to_string())
            .replace("{{weekday}}", &now.format("%A").to_string())
            .replace("{{vault}}", &self.vault_name())
            .replace("{{cursor}}", "")
    }

    fn open_periodic(&self, folder: &str, format: &str) -> Result<String, MobileError> {
        let title = chrono::Local::now().format(format).to_string();
        let relative = format!("{folder}/{title}.md");
        let path = self.writable_path(&relative)?;
        if !path.exists() {
            let template = fs::read_to_string(self.root.join("Templates").join(match folder {
                "daily" => "Daily.md",
                "weekly" => "Weekly.md",
                _ => "Monthly.md",
            }))
            .unwrap_or_else(|_| format!("# {title}\n\n"));
            let source = self
                .substitute(&template, &title)
                .replace("{{folder}}", folder);
            self.create_note(relative.clone(), source)?;
        } else {
            self.refresh()?;
        }
        Ok(relative)
    }
}

/// Make a user-entered title safe as a single vault-relative file name.
fn sanitize_name(name: &str) -> String {
    let sanitized: String = name
        .trim()
        .chars()
        .map(|ch| match ch {
            ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\\' | '/' => '-',
            ch if ch.is_control() => '-',
            ch => ch,
        })
        .collect();
    let sanitized = sanitized.trim_matches(['.', ' ']).to_string();
    if sanitized.is_empty() {
        "Untitled".to_string()
    } else {
        sanitized
    }
}

fn flatten_tree(node: &TreeNode, depth: i32, out: &mut Vec<TreeRow>) {
    for child in &node.children {
        out.push(TreeRow {
            name: child.name.clone(),
            path: child.path.clone(),
            is_dir: child.kind == NodeKind::Dir,
            depth,
        });
        if child.kind == NodeKind::Dir {
            flatten_tree(child, depth + 1, out);
        }
    }
}

/// Parse headings for an outline panel.
#[uniffi::export]
pub fn headings(source: String) -> Vec<HeadingInfo> {
    markdown::headings(&source)
        .into_iter()
        .map(|heading| HeadingInfo {
            level: heading.level as i32,
            text: heading.text,
            line: heading.line as i32,
        })
        .collect()
}

/// Frontmatter properties, in display order.
#[uniffi::export]
pub fn properties(source: String) -> Vec<PropertyInfo> {
    frontmatter::display_properties(&source)
        .into_iter()
        .map(|property| PropertyInfo {
            key: property.key,
            value: property.value.display(),
        })
        .collect()
}

/// Render markdown to a complete HTML document, including the engine's export CSS.
#[uniffi::export]
pub fn render_html(source: String) -> String {
    wrap_html(html::render_body(&source))
}

/// Render markdown with the vault's attachment index, so `![[shot.png]]` resolves to a
/// subfolder path the WebView can load relative to the vault root.
#[uniffi::export]
pub fn render_html_from_vault(source: String, root: String) -> String {
    let index = Attachments::index(std::path::Path::new(&root));
    let body = html::render_body_with(&source, Some(&index), &mut Vec::new());
    wrap_html(body)
}

fn wrap_html(body: String) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <style>{}</style></head><body>{body}</body></html>",
        html::STYLE
    )
}

/// One in-note find match, in UTF-16 code units (what Android text fields index by).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FindRange {
    pub start: u32,
    pub end: u32,
}

/// Find matches in `source` with the engine's finder: literal or regex, case and whole-word
/// options. Offsets are UTF-16 code units so Kotlin can use them for a text selection.
#[uniffi::export]
pub fn find_in_text(
    source: String,
    pattern: String,
    regex: bool,
    case_sensitive: bool,
    whole_word: bool,
) -> Vec<FindRange> {
    let options = find::Options {
        regex,
        case_sensitive,
        whole_word,
    };
    let Ok(matches) = find::find_all(&source, &pattern, options) else {
        return Vec::new();
    };
    if matches.is_empty() {
        return Vec::new();
    }

    // Character index -> UTF-16 offset, once for the whole document.
    let mut offsets = Vec::with_capacity(source.chars().count() + 1);
    let mut total = 0u32;
    offsets.push(0);
    for character in source.chars() {
        total += character.len_utf16() as u32;
        offsets.push(total);
    }

    let last = offsets.len() - 1;
    matches
        .into_iter()
        .map(|found| FindRange {
            start: offsets[found.range.start.min(last)],
            end: offsets[found.range.end.min(last)],
        })
        .collect()
}

/// Word/character counts for the current document.
#[uniffi::export]
pub fn document_stats(source: String) -> StatsInfo {
    let stats = stats::compute(&source);
    StatsInfo {
        words: stats.words as i32,
        characters: stats.characters as i32,
        sentences: stats.sentences as i32,
        paragraphs: stats.paragraphs as i32,
        reading_minutes: stats.reading_minutes as i32,
    }
}

/// The engine version, shown in an About screen.
#[uniffi::export]
pub fn engine_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_offsets_are_utf16_units() {
        // The emoji is two UTF-16 units, so the second match starts at 5, not 4.
        let found = find_in_text("a😀b a".to_string(), "a".to_string(), false, true, false);
        assert_eq!(
            found,
            vec![
                FindRange { start: 0, end: 1 },
                FindRange { start: 5, end: 6 }
            ]
        );
    }

    #[test]
    fn find_honours_options_and_bad_patterns() {
        assert_eq!(
            find_in_text(
                "Beta beta".to_string(),
                "beta".to_string(),
                false,
                true,
                false
            )
            .len(),
            1
        );
        assert_eq!(
            find_in_text("a1 b2".to_string(), r"\d".to_string(), true, false, false).len(),
            2
        );
        assert!(find_in_text("a".to_string(), "(".to_string(), true, false, false).is_empty());
    }

    #[test]
    fn link_fragments_resolve_headings_and_blocks() {
        let source = "# One\ntext\n## Sub\nbody ^note-1\n";
        assert_eq!(
            resolve_link_fragment(source.to_string(), "One".to_string()),
            Some(1)
        );
        assert_eq!(
            resolve_link_fragment(source.to_string(), "#sub".to_string()),
            Some(3)
        );
        assert_eq!(
            resolve_link_fragment(source.to_string(), "^note-1".to_string()),
            Some(4)
        );
        assert_eq!(
            resolve_link_fragment(source.to_string(), "Nope".to_string()),
            None
        );
    }

    #[test]
    fn a_fingerprinted_save_detects_an_external_edit() {
        let root = temp_vault("fingerprint");
        fs::write(root.join("note.md"), "original").unwrap();
        let vault = MobileVault::open(root.to_string_lossy().to_string()).unwrap();
        let loaded = vault.document_fingerprint("note.md".to_string()).unwrap();

        let saved = vault.save_document_with_fingerprint(
            "note.md".to_string(),
            "mine".to_string(),
            Some(loaded.content_hash),
        );
        assert_eq!(saved.status, "saved");
        assert!(saved.fingerprint.is_some());

        let stale = loaded.content_hash;
        fs::write(root.join("note.md"), "someone else").unwrap();
        let conflicted = vault.save_document_with_fingerprint(
            "note.md".to_string(),
            "mine again".to_string(),
            Some(stale),
        );
        assert_eq!(conflicted.status, "conflict");
        assert!(conflicted.disk_hash.is_some());
        assert_eq!(
            fs::read_to_string(root.join("note.md")).unwrap(),
            "someone else"
        );
    }

    #[test]
    fn drafts_roundtrip_through_the_vault() {
        let root = temp_vault("drafts");
        fs::write(root.join("note.md"), "body").unwrap();
        let vault = MobileVault::open(root.to_string_lossy().to_string()).unwrap();
        vault
            .save_draft("note.md".to_string(), "typed text".to_string(), 3, 0)
            .unwrap();
        let draft = vault.load_draft("note.md".to_string()).expect("draft");
        assert_eq!(draft.text, "typed text");
        assert_eq!(draft.line, 3);
        assert!(draft.base_hash.is_some());
        assert!(vault.load_draft("other.md".to_string()).is_none());
        vault.discard_draft("note.md".to_string());
        assert!(vault.load_draft("note.md".to_string()).is_none());
    }

    #[test]
    fn reader_settings_roundtrip_and_clamp() {
        let root = temp_vault("reader");
        let vault = MobileVault::open(root.to_string_lossy().to_string()).unwrap();
        let default = vault.reader_settings();
        assert_eq!(default.body_size, 15);
        assert_eq!(default.measure, 780);
        vault.set_reader_settings(18, 900).unwrap();
        let loaded = vault.reader_settings();
        assert_eq!(loaded.body_size, 18);
        assert_eq!(loaded.measure, 900);
        vault.set_reader_settings(1, 9999).unwrap();
        let clamped = vault.reader_settings();
        assert_eq!(clamped.body_size, 11);
        assert_eq!(clamped.measure, 1200);
    }

    fn temp_vault(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "dragon-glass-ffi-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn opens_lists_and_reads_a_vault() {
        let root = temp_vault("open");
        fs::create_dir_all(root.join("notes")).unwrap();
        fs::write(root.join("notes/Hello.md"), "# Hello\n\nBody").unwrap();
        fs::write(root.join("Index.md"), "# Index").unwrap();

        let vault = MobileVault::open(root.to_string_lossy().to_string()).unwrap();
        let notes = vault.notes();
        assert_eq!(notes.len(), 2);
        assert_eq!(
            notes
                .iter()
                .map(|note| note.relative.as_str())
                .collect::<Vec<_>>(),
            vec!["Index.md", "notes/Hello.md"]
        );
        assert_eq!(
            vault.note_source("notes/Hello.md".to_string()).unwrap(),
            "# Hello\n\nBody"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn writes_and_searches_notes() {
        let root = temp_vault("write");
        let vault = MobileVault::open(root.to_string_lossy().to_string()).unwrap();
        vault
            .create_note("Journal.md".to_string(), "# Journal\n\nneedle".to_string())
            .unwrap();
        assert_eq!(vault.notes().len(), 1);

        let hits = vault.search("needle".to_string(), 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].relative, "Journal.md");

        vault
            .save_note("Journal.md".to_string(), "# Journal\n\nupdated".to_string())
            .unwrap();
        assert!(vault
            .note_source("Journal.md".to_string())
            .unwrap()
            .contains("updated"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_paths_outside_the_vault() {
        let root = temp_vault("escape");
        let vault = MobileVault::open(root.to_string_lossy().to_string()).unwrap();
        assert!(vault.note_source("../outside.md".to_string()).is_err());
        assert!(vault
            .save_note("../outside.md".to_string(), "no".to_string())
            .is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn titles_are_sanitized_for_file_names() {
        assert_eq!(sanitize_name("My: note?"), "My- note-");
        assert_eq!(sanitize_name("  ...  "), "Untitled");
        assert_eq!(sanitize_name("a/b"), "a-b");
    }

    #[test]
    fn moves_renames_and_restores_through_the_trash() {
        let root = temp_vault("trash");
        let vault = MobileVault::open(root.to_string_lossy().to_string()).unwrap();
        vault
            .create_note("notes/gone.md".to_string(), "keep me".to_string())
            .unwrap();

        vault
            .rename_entry("notes/gone.md".to_string(), "notes/kept.md".to_string())
            .unwrap();
        assert_eq!(vault.notes()[0].relative, "notes/kept.md");

        let trashed = vault.trash_note("notes/kept.md".to_string()).unwrap();
        assert!(trashed.starts_with(".dragon-glass/trash/"));
        assert!(vault.notes().is_empty());

        let entries = vault.trash_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].original, "notes/kept.md");

        let restored = vault
            .restore_trash_entry(entries[0].trash_relative.clone())
            .unwrap();
        assert_eq!(restored, "notes/kept.md");
        assert_eq!(
            vault.note_source("notes/kept.md".to_string()).unwrap(),
            "keep me"
        );
        assert!(vault.trash_entries().is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_headings_properties_and_html() {
        let source = "---\ntitle: Hi\n---\n# One\n\n## Two\n";
        let headings = headings(source.to_string());
        assert_eq!(headings.len(), 2);
        assert_eq!(headings[0].level, 1);
        assert_eq!(headings[1].text, "Two");

        let properties = properties(source.to_string());
        assert_eq!(properties[0].key, "title");
        assert_eq!(properties[0].value, "Hi");

        let html = render_html(source.to_string());
        assert!(html.contains("<h1>One</h1>"));
    }
}
