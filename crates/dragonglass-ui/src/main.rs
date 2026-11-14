//! Dragon Glass UI (Phase 0/1).
//!
//! Opens a vault, shows its directory tree, loads notes into the in-house editor, and
//! provides a quick switcher (Ctrl+O / Cmd+O), full-text search (Ctrl+Shift+F /
//! Cmd+Shift+F) and a git menu (Ctrl+G / Cmd+G).

// Release builds are a normal GUI app on Windows: no console window behind the editor.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod editor;
mod flow;
mod graph_view;
mod math_element;
mod platform;
mod preview_window;
mod reading;
mod theme;
mod vault_picker;
mod vim;

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use dragonglass_core::assets::Attachments;
use dragonglass_core::fingerprint::SaveReport;
use dragonglass_core::frontmatter::{Property, PropertyValue};
use dragonglass_core::{
    bookmarks, config, conflict, diff, drafts, git, graph, history, keymap, lint, markdown, query,
    relations, resolve, search, session, shared, stats, storage, store, tags, templates, tree,
    update, uri, vaultfs, vaults, versions, workspace, NodeKind, Note, NoteRef, TreeNode, Vault,
};

/// Actions the hotkey editor can rebind: `(action, default key, label)`.
const HOTKEYS: &[(&str, &str, &str)] = &[
    ("OpenCommands", "secondary+p", "Command palette"),
    ("OpenSettings", "secondary-,", "Settings"),
    ("OpenSwitcher", "secondary+o", "Quick switcher"),
    ("OpenSearch", "secondary+shift+f", "Search"),
    ("OpenGraph", "secondary+shift+g", "Graph view"),
    ("OpenRelations", "secondary+shift+r", "Document relations"),
    ("OpenDaily", "secondary+shift+d", "Daily note"),
    ("OpenWeekly", "secondary+shift+w", "Weekly note"),
    ("OpenMonthly", "secondary+shift+m", "Monthly note"),
    ("OpenProperties", "secondary+shift+p", "Properties"),
    ("ToggleBookmark", "secondary+shift+b", "Toggle bookmark"),
    ("OpenGit", "secondary+g", "Git"),
    ("ToggleTheme", "secondary+shift+l", "Toggle theme"),
    ("NewWindow", "secondary+n", "New window"),
    (
        "ToggleSideBySide",
        "secondary-alt+1",
        "Read/write side by side",
    ),
    ("DetachTab", "secondary-alt+n", "Move tab to new window"),
    (
        "ToggleSplitEditor",
        "secondary-alt+2",
        "Two editors side by side",
    ),
    ("CloseTab", "secondary+w", "Close tab"),
    ("ReopenClosedTab", "secondary+shift+t", "Reopen closed tab"),
    ("SaveAs", "secondary+shift+s", "Save note as"),
    ("ToggleZen", "secondary+alt+z", "Zen mode"),
    ("NavBack", "secondary+[", "Back"),
    ("NavForward", "secondary+]", "Forward"),
    ("NextTab", "ctrl-tab", "Next tab"),
    ("PrevTab", "ctrl-shift-tab", "Previous tab"),
];

/// Resolve `./` and `../` in an explicit relative link against the folder of the note that
/// contains it. Other targets are returned unchanged and keep the global resolver rules.
fn relative_link_target(target: &str, base_dir: Option<&str>) -> String {
    let target = target.replace('\\', "/");
    if !(target.starts_with("./") || target.starts_with("../")) {
        return target;
    }
    let mut parts: Vec<&str> = base_dir
        .unwrap_or("")
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// Bind a saved override by action name.
fn bind_named(cx: &mut App, key: &str, action: &str) {
    // Accept both `ctrl+shift+g` and `ctrl-shift-g`; GPUI only understands `-`.
    let key = platform::normalize(key);
    let key = key.as_str();
    match action {
        "OpenCommands" => cx.bind_keys([KeyBinding::new(key, OpenCommands, None)]),
        "OpenSettings" => cx.bind_keys([KeyBinding::new(key, OpenSettings, None)]),
        "OpenSwitcher" => cx.bind_keys([KeyBinding::new(key, OpenSwitcher, None)]),
        "OpenSearch" => cx.bind_keys([KeyBinding::new(key, OpenSearch, None)]),
        "OpenGraph" => cx.bind_keys([KeyBinding::new(key, OpenGraph, None)]),
        "OpenRelations" => cx.bind_keys([KeyBinding::new(key, OpenRelations, None)]),
        "OpenDaily" => cx.bind_keys([KeyBinding::new(key, OpenDaily, None)]),
        "OpenWeekly" => cx.bind_keys([KeyBinding::new(key, OpenWeekly, None)]),
        "OpenMonthly" => cx.bind_keys([KeyBinding::new(key, OpenMonthly, None)]),
        "OpenProperties" => cx.bind_keys([KeyBinding::new(key, OpenProperties, None)]),
        "ToggleBookmark" => cx.bind_keys([KeyBinding::new(key, ToggleBookmark, None)]),
        "OpenGit" => cx.bind_keys([KeyBinding::new(key, OpenGit, None)]),
        "ToggleTheme" => cx.bind_keys([KeyBinding::new(key, ToggleTheme, None)]),
        "NewWindow" => cx.bind_keys([KeyBinding::new(key, NewWindow, None)]),
        "DetachTab" => cx.bind_keys([KeyBinding::new(key, DetachTab, None)]),
        "ToggleSideBySide" => cx.bind_keys([KeyBinding::new(key, ToggleSideBySide, None)]),
        "ToggleSplitEditor" => cx.bind_keys([KeyBinding::new(key, ToggleSplitEditor, None)]),
        "CloseTab" => cx.bind_keys([KeyBinding::new(key, CloseTab, None)]),
        "ReopenClosedTab" => cx.bind_keys([KeyBinding::new(key, ReopenClosedTab, None)]),
        "SaveAs" => cx.bind_keys([KeyBinding::new(key, SaveAs, None)]),
        "ToggleZen" => cx.bind_keys([KeyBinding::new(key, ToggleZen, None)]),
        "NavBack" => cx.bind_keys([KeyBinding::new(key, NavBack, None)]),
        "NavForward" => cx.bind_keys([KeyBinding::new(key, NavForward, None)]),
        "NextTab" => cx.bind_keys([KeyBinding::new(key, NextTab, None)]),
        "PrevTab" => cx.bind_keys([KeyBinding::new(key, PrevTab, None)]),
        _ => {}
    }
}
use dragonglass_plugins as plugins;
use editor::Editor;
use gpui::{
    actions, anchored, div, prelude::*, px, size, uniform_list, AnyElement, App, Application,
    Bounds, Context, Corner, CursorStyle, ElementId, Entity, FocusHandle, Focusable, KeyBinding,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, NavigationDirection,
    Pixels, Point, Render, ScrollStrategy, Timer, UniformListScrollHandle, WeakEntity, Window,
    WindowBounds, WindowOptions,
};
use graph_view::GraphView;
use reading::ReadingCache;
use reading::{FocusMode, ReaderFont, ReaderSettings};
use theme::Theme;

actions!(
    app_actions,
    [
        OpenSwitcher,
        OpenSearch,
        OpenGit,
        OpenDaily,
        OpenProperties,
        ToggleBookmark,
        OpenCommands,
        OpenSettings,
        ToggleTheme,
        ToggleVim,
        OpenGraph,
        OpenRelations,
        OpenWeekly,
        OpenMonthly,
        CloseQuery,
        CloseOverlay,
        RunQuery,
        NewWindow,
        ToggleSideBySide,
        DetachTab,
        ToggleSplitEditor,
        CloseTab,
        ReopenClosedTab,
        SaveAs,
        ToggleZen,
        ReloadConflicted,
        KeepMine,
        CompareConflicted,
        SaveConflictCopy,
        RecoverDraft,
        CompareDraft,
        DiscardDraft,
        NavBack,
        NavForward,
        NextTab,
        PrevTab,
        HideApp,
        HideOthers,
        ShowAll,
        Quit
    ]
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum SwitcherMode {
    QuickSwitch,
    Search,
    GitMenu,
    CommitInput,
    PropertiesMenu,
    PropertyInput,
    CommandsMenu,
    QueryInput,
    WorkspaceName,
    WorkspaceMenu,
    HotkeyMenu,
    HotkeyCapture,
    GraphFilterInput,
    VersionMenu,
    TemplateMenu,
    TemplateNewMenu,
    SharedMenu,
    SharedEditMenu,
    ReviewMenu,
    NewNoteName,
    SharedName,
    RelationInput,
    NewFolderName,
    RenameInput,
    DeleteConfirm,
    FileMenu,
    HistoryMenu,
    SettingInput,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CommandId {
    QuickSwitch,
    Search,
    Daily,
    Weekly,
    Monthly,
    Properties,
    Bookmark,
    Git,
    ToggleReading,
    Save,
    ToggleTheme,
    CheckUpdates,
    OpenGraph,
    RunQuery,
    SaveWorkspace,
    LoadWorkspace,
    UserTheme(usize),
    NewWindow,
    OpenVault,
    Hotkeys,
    GraphFilter,
    GraphDepth,
    ToggleSnippets,
    TogglePreview,
    DetachPreview,
    Plugin(usize),
    CloseTab,
    NextTab,
    PrevTab,
    Versions,
    SaveVersionA,
    SaveVersionB,
    InsertTemplate,
    NewFromTemplate,
    InsertShared,
    EditShared,
    ExtractShared,
    ReloadNote,
    ReviewDocument,
    Relations,
    RelationGraph,
    MoveBlockUp,
    MoveBlockDown,
    DuplicateBlock,
    DeleteBlock,
    AddRelation,
    ToggleSplitEditor,
    DetachTab,
    FindReplace,
    ToggleVim,
    ToggleFold,
    Settings,
    NewNote,
    NewFolder,
    RenameNote,
    DeleteNote,
    SaveAs,
    History,
    Zen,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GitAction {
    Commit,
    Pull,
    Push,
    Init,
    Refresh,
}

enum Item {
    Note {
        relative: String,
    },
    /// A full-text search hit: open the note at a 1-based line and select the match.
    NoteAt {
        relative: String,
        line: usize,
    },
    Git(GitAction),
    Property {
        key: Option<String>,
    },
    DeleteProperty {
        key: String,
    },
    Command(CommandId),
    Workspace(String),
    Hotkey(String),
    VersionSave(String),
    VersionCompare {
        left: Option<String>,
        right: String,
    },
    VersionPromote(String),
    VersionDelete(String),
    Template(String),
    TemplateNew(String),
    Shared(String),
    SharedEdit(String),
    Issue(usize),
    Tree(TreeAction),
    HistoryCompare(String),
    HistoryRestore(String),
}

/// Actions offered by the sidebar's context menu.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TreeAction {
    Open,
    NewNote,
    NewFolder,
    Rename,
    Delete,
}

/// The note or folder a file-menu action applies to.
#[derive(Clone, PartialEq, Eq)]
struct TreeTarget {
    path: String,
    is_dir: bool,
}

/// A numeric or text setting edited through the command panel.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingKey {
    FontSize,
    Accent,
    AttachmentsFolder,
    TemplatesFolder,
    DailyFolder,
    WeeklyFolder,
    MonthlyFolder,
}

impl SettingKey {
    fn label(self) -> &'static str {
        match self {
            SettingKey::FontSize => "Editor font size",
            SettingKey::Accent => "Accent colour",
            SettingKey::AttachmentsFolder => "Attachments folder",
            SettingKey::TemplatesFolder => "Templates folder",
            SettingKey::DailyFolder => "Daily notes folder",
            SettingKey::WeeklyFolder => "Weekly notes folder",
            SettingKey::MonthlyFolder => "Monthly notes folder",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Periodic {
    Daily,
    Weekly,
    Monthly,
}
struct SwitcherItem {
    label: String,
    detail: String,
    item: Item,
}

struct Switcher {
    mode: SwitcherMode,
    query: String,
    results: Vec<SwitcherItem>,
    selected: usize,
    /// When editing an existing property, its key; `None` when adding one.
    property_key: Option<String>,
    /// Action being rebound, while capturing a keystroke.
    capture_action: Option<String>,
}

/// Pick the tabs to reopen: an explicit URI note, then the saved session's notes that still
/// exist, then a fallback note. Untitled tabs are not restored.
fn restored_tabs(
    session: &session::Session,
    uri_note: Option<&str>,
    fallback: Option<&str>,
    exists: impl Fn(&str) -> bool,
) -> Vec<String> {
    if let Some(note) = uri_note.filter(|note| exists(note)) {
        return vec![note.to_string()];
    }

    let mut notes: Vec<String> = Vec::new();
    for tab in &session.tabs {
        if tab.note.is_empty() || !exists(&tab.note) {
            continue;
        }
        if !notes.contains(&tab.note) {
            notes.push(tab.note.clone());
        }
        if notes.len() >= 20 {
            break;
        }
    }
    if notes.is_empty() {
        if let Some(fallback) = fallback {
            notes.push(fallback.to_string());
        }
    }
    notes
}

/// One open note. Each tab owns its editor, so cursor, scroll and unsaved edits are
/// preserved when switching between notes.
struct Tab {
    relative: Option<String>,
    editor: Entity<Editor>,
    title: String,
    /// Whether the file changed on disk while this tab has unsaved edits.
    conflict: bool,
    /// Set when a restored tab could not be read, so the pane can offer Retry.
    load_error: Option<String>,
    /// A restored tab whose file has not been read yet. The active tab loads first.
    pending_load: bool,
}

/// How the status line is styled, and whether it expires.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Success,
    Warn,
    Error,
}

impl StatusKind {
    /// Errors stay until they are replaced or dismissed; everything else is transient.
    fn expires(self) -> bool {
        !matches!(self, StatusKind::Error)
    }

    fn lifetime(self) -> std::time::Duration {
        match self {
            StatusKind::Success => std::time::Duration::from_secs(3),
            StatusKind::Info => std::time::Duration::from_secs(6),
            StatusKind::Warn => std::time::Duration::from_secs(10),
            StatusKind::Error => std::time::Duration::from_secs(0),
        }
    }
}

/// One line of feedback, with the moment it was set so it can expire.
#[derive(Clone)]
struct Status {
    kind: StatusKind,
    text: String,
    at: std::time::Instant,
}

/// A tab the user closed, kept so Ctrl/Cmd+Shift+T can bring it back.
#[derive(Clone)]
struct ClosedTab {
    relative: Option<String>,
    title: String,
    reading: bool,
    line: usize,
}

/// One place the user visited, for back/forward navigation.
#[derive(Clone)]
struct NavEntry {
    relative: String,
    reading: bool,
    line: usize,
    reading_block: usize,
}

/// The value carried while dragging a tab between the tab bar, editor panes and windows.
#[derive(Clone)]
struct TabDrag {
    source: WeakEntity<DragonGlassApp>,
    index: usize,
    title: String,
}

/// A small floating tab rendered under the cursor during a drag.
struct DragGhost {
    title: String,
}

impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        div()
            .px_3()
            .py_1()
            .rounded_md()
            .bg(theme.header)
            .border_1()
            .border_color(theme.border)
            .text_sm()
            .text_color(theme.text)
            .child(self.title.clone())
    }
}

/// Where a dragged tab was released.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TabDrop {
    Merge,
    Split(SplitSide),
    Detach,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SplitSide {
    Left,
    Right,
    Above,
    Below,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SplitAxis {
    Horizontal,
    Vertical,
}

/// Which side panel a splitter drag is resizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizePanel {
    Sidebar,
    Outline,
    Relations,
}

/// An in-progress splitter drag.
#[derive(Clone, Copy)]
struct ResizeDrag {
    panel: ResizePanel,
    start_x: f32,
    start_width: f32,
    last_width: f32,
    last_tick: std::time::Instant,
}

/// A draggable splitter between a side panel and the editor.
fn resize_handle(panel: ResizePanel, theme: Theme, cx: &mut Context<DragonGlassApp>) -> AnyElement {
    div()
        .id(ElementId::Name(format!("resize-{panel:?}").into()))
        .w(px(5.0))
        .h_full()
        .flex_shrink_0()
        .cursor(CursorStyle::ResizeLeftRight)
        .hover(move |style| style.bg(theme.accent))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                let start_width = match panel {
                    ResizePanel::Sidebar => this.sidebar_width,
                    ResizePanel::Outline => this.outline_width,
                    ResizePanel::Relations => this.relations_width,
                };
                this.resize = Some(ResizeDrag {
                    panel,
                    start_x: f32::from(event.position.x),
                    start_width,
                    last_width: start_width,
                    last_tick: std::time::Instant::now(),
                });
                this.set_editors_resizing(true, cx);
                cx.notify();
            }),
        )
        .into_any_element()
}

/// Sanitize a vault-relative path for a newly created note.
fn sanitize_relative(path: &str) -> String {
    vaultfs::sanitize_relative(path)
}

fn tab_title(relative: &str) -> String {
    relative
        .rsplit('/')
        .next()
        .unwrap_or(relative)
        .trim_end_matches(".md")
        .trim_end_matches(".markdown")
        .to_string()
}

struct DragonGlassApp {
    editor: gpui::Entity<Editor>,
    /// All open notes; `editor` is always the active tab's editor.
    tabs: Vec<Tab>,
    /// Recently closed notes, newest last, so a close can be undone with the keyboard.
    closed_tabs: Vec<ClosedTab>,
    /// Hide the chrome (tab bar, sidebar, outline, relations) for distraction-free reading.
    zen: bool,
    /// Places visited in this window, and where the user currently sits in them.
    nav_history: Vec<NavEntry>,
    nav_index: usize,
    active_tab: usize,
    /// Additional tabs shown beside the active editor, in layout order.
    split_tabs: Vec<usize>,
    /// Whether the split panes are side-by-side or stacked.
    split_axis: SplitAxis,
    /// A detached window that shows only its editor, with none of the app chrome.
    minimal: bool,
    /// Set when a minimal window has given its last tab away; it closes on the next frame.
    close_requested: bool,
    /// The tab currently being dragged, if any.
    dragging: Option<TabDrag>,
    /// The platform window title last set, so it is only pushed when it changes.
    shown_title: String,
    /// Bumped when the active editor changes so document-derived caches invalidate even
    /// when two editors share the same buffer revision.
    cache_epoch: u64,
    vault_root: PathBuf,
    tree: TreeNode,
    /// Flattened tree rows for the sidebar. Rebuilt when the vault changes or a folder
    /// is collapsed, never in `render`, so scrolling does not walk the tree each frame.
    file_rows: Rc<Vec<Row>>,
    /// Vault-relative paths of folders whose contents are hidden in the sidebar.
    collapsed_dirs: HashSet<String>,
    /// Heading indices whose sections are folded in the outline.
    collapsed_outline: HashSet<usize>,
    note_refs: Vec<NoteRef>,
    /// Link resolution for the whole vault: paths, stems, frontmatter names and aliases.
    note_index: resolve::NoteIndex,
    /// `(insertion path, display name)` for wikilink autocomplete.
    note_suggestions: Rc<Vec<(String, String)>>,
    /// All tags in the vault, for `#tag` autocomplete.
    tag_suggestions: Rc<Vec<String>>,
    properties: Vec<Property>,
    /// Outline headings, cached per document version so scrolling never re-parses the note.
    headings: Rc<Vec<markdown::Heading>>,
    headings_version: (u64, u64),
    /// Virtualized split-preview content.
    preview_content: ReadingCache,
    bookmarks: Vec<String>,
    theme: Theme,
    conflict: bool,
    watcher: Option<notify::RecommendedWatcher>,
    update_source: Option<String>,
    update_available: Option<String>,
    full_graph: graph::Graph,
    graph_view: Option<gpui::Entity<GraphView>>,
    show_graph: bool,
    relations: Vec<relations::NoteRelations>,
    show_relations: bool,
    /// Panel widths in pixels, adjustable by dragging the splitters.
    sidebar_width: f32,
    outline_width: f32,
    relations_width: f32,
    resize: Option<ResizeDrag>,
    show_diff: bool,
    diff_title: String,
    diff_summary: String,
    diff_rows: Rc<Vec<diff::Line>>,
    review_issues: Vec<lint::Issue>,
    pending_shared: Option<String>,
    pending_template: Option<templates::TemplateFile>,
    stats: stats::Stats,
    stats_version: (u64, u64),
    query_results: Rc<Vec<query::Row>>,
    show_query: bool,
    /// Show the reading view beside the editor.
    split_preview: bool,
    /// Installed plugins, and a flattened list of `(plugin index, command id, title)`.
    plugins: Vec<plugins::PluginInfo>,
    plugin_commands: Vec<(usize, String, String)>,
    workspaces: Vec<workspace::Workspace>,
    user_themes: Vec<(String, Theme)>,
    bindings: Vec<keymap::Binding>,
    /// The selected theme before snippets are layered on.
    base_theme: Theme,
    snippet_overrides: Vec<(String, String)>,
    config: config::Config,
    active: Option<String>,
    vault_path: String,
    notes: usize,
    links: usize,
    git_status: Option<git::Status>,
    /// The one-line feedback in the header. Expires on its own unless it is an error.
    status: Option<Status>,
    /// A recovered draft waiting for the user's decision.
    pending_draft: Option<drafts::Draft>,
    /// When the throttled stats/outline pass last ran, and whether a timer is armed.
    derived_at: Option<std::time::Instant>,
    derived_armed: bool,
    /// When the split preview last re-parsed, and whether a timer is armed.
    preview_at: Option<std::time::Instant>,
    preview_armed: bool,
    /// Epoch (note) the preview was last built for.
    preview_epoch: u64,
    /// True while a one-shot task is waiting to expire the current status line.
    status_timer_armed: bool,
    switcher: Option<Switcher>,
    switcher_focus: FocusHandle,
    focus_handle: FocusHandle,
    /// Set while closing the window is waiting on the user's decision about unsaved notes.
    quit_prompt: bool,
    /// True when Cmd+Q started the quit, so resolving the prompt also quits the app.
    quit_after_prompt: bool,
    /// Set once the user has chosen to quit, so the close callback lets the window go.
    allow_close: bool,
    /// The settings page replaces the editor while it is open.
    show_settings: bool,
    /// Which setting the input panel is editing.
    pending_setting: Option<SettingKey>,
    /// Focus while the settings page is open, so typing cannot reach the hidden editor.
    settings_focus: FocusHandle,
    /// File or folder targeted by the sidebar menu.
    tree_menu: Option<TreeTarget>,
    /// Folder new notes and folders are created in.
    pending_dir: Option<String>,
    /// Path being renamed or deleted.
    pending_path: Option<String>,
    /// The name panel is choosing a file for the active untitled buffer.
    pending_save_as: bool,
    /// Window position of the open tab overflow menu, if any.
    tab_menu: Option<Point<Pixels>>,
    /// Generation of the running preview scroll-sync loop.
    scroll_sync_generation: u64,
    /// Scroll fractions seen on each side, so a sync only reacts to real movement.
    last_editor_progress: f32,
    last_preview_progress: f32,
    /// Scroll position of the outline panel, so the active heading stays visible.
    outline_scroll: UniformListScrollHandle,
    /// The heading the outline last scrolled to.
    outline_current: Option<String>,
}

impl DragonGlassApp {
    /// Build the callback reading views use to open clicked links.
    fn link_opener(&self, cx: &mut Context<Self>) -> reading::LinkOpener {
        let weak = cx.entity().downgrade();
        Rc::new(move |target: &str, window: &mut Window, cx: &mut App| {
            let _ = weak.update(cx, |app, cx| app.open_target(target, window, cx));
        })
    }

    /// Open a link target: a note (by path, stem, slug or alias) or an external URL.
    fn open_target(&mut self, target: &str, window: &mut Window, cx: &mut Context<Self>) {
        // A tag chip in the metadata header searches the vault for that tag.
        if let Some(tag) = target.strip_prefix("tag:") {
            let query = tag.trim().to_string();
            if !query.is_empty() {
                self.open_search_for(&query, window, cx);
                return;
            }
        }
        if target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with("mailto:")
            || target.starts_with("tel:")
            || target.starts_with("file://")
        {
            cx.open_url(target);
            return;
        }

        let (note, fragment) = match target.split_once('#') {
            Some((note, fragment)) => (note.trim(), Some(fragment.trim())),
            None => (target.trim(), None),
        };
        if note.is_empty() {
            // `[[#Local]]`: stay in this note and jump to the fragment.
            if let Some(fragment) = fragment {
                self.jump_to_fragment(fragment, window, cx);
            }
            return;
        }

        // `[[../Sibling]]` and `[[./Child]]` resolve from the current note's folder.
        let base = self
            .active
            .as_deref()
            .and_then(|active| active.rsplit_once('/').map(|(dir, _)| dir));
        let note = relative_link_target(note, base);
        let note = note.as_str();
        match self.note_index.resolve_relative(note) {
            Some(relative) => {
                let relative = relative.to_string();
                self.open_note(&relative, window, cx);
                if let Some(fragment) = fragment {
                    self.jump_to_fragment(fragment, window, cx);
                }
            }
            None => {
                // An unresolved wikilink creates the note, the way Obsidian does.
                let is_asset = Path::new(note)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| {
                        !ext.eq_ignore_ascii_case("md") && !ext.eq_ignore_ascii_case("markdown")
                    })
                    .unwrap_or(false);
                if is_asset {
                    // Attachments are not notes: hand them to the OS instead of failing.
                    let direct = self.vault_root.join(note);
                    let resolved = if direct.is_file() {
                        Some(direct)
                    } else {
                        Attachments::index(&self.vault_root)
                            .resolve(note)
                            .map(|relative| self.vault_root.join(relative))
                    };
                    if let Some(path) = resolved {
                        cx.open_url(&format!("file://{}", path.display()));
                        self.set_status(StatusKind::Info, format!("opened {note}"));
                    } else {
                        self.set_status(StatusKind::Error, format!("unresolved link: {note}"));
                    }
                    cx.notify();
                    return;
                }
                self.create_linked_note(note, window, cx);
            }
        }
    }

    /// Move to a `#heading` or `#^block` fragment in the active note.
    fn jump_to_fragment(&mut self, fragment: &str, window: &mut Window, cx: &mut Context<Self>) {
        let fragment = fragment.trim();
        if fragment.is_empty() {
            return;
        }
        let editor = self.editor.clone();
        let found = editor.update(cx, |editor, cx| {
            let found = match fragment.strip_prefix('^') {
                Some(id) => editor.jump_to_block_id(id),
                None => editor.jump_to_heading_text(fragment),
            };
            cx.notify();
            found
        });
        if found {
            window.focus(&self.editor.focus_handle(cx));
            self.set_status(StatusKind::Info, format!("jumped to {fragment}"));
        } else {
            self.set_status(
                StatusKind::Warn,
                format!("no heading or block named {fragment} in this note"),
            );
        }
        cx.notify();
    }

    /// Create and open a note for an unresolved wikilink.
    fn create_linked_note(&mut self, target: &str, window: &mut Window, cx: &mut Context<Self>) {
        let cleaned = target.trim().trim_matches('/');
        if cleaned.is_empty() {
            return;
        }

        let file = if cleaned.to_ascii_lowercase().ends_with(".md") {
            cleaned.to_string()
        } else {
            format!("{cleaned}.md")
        };

        // A bare name goes next to the current note; a path is vault-relative.
        let relative = if file.contains('/') {
            file
        } else {
            match self
                .active
                .as_deref()
                .and_then(|active| active.rsplit_once('/'))
            {
                Some((dir, _)) if !dir.is_empty() => format!("{dir}/{file}"),
                _ => file,
            }
        };
        let relative = sanitize_relative(&relative);
        if relative.is_empty() {
            return;
        }

        let path = self.vault_root.join(&relative);
        if path.exists() {
            // Indexed copies can lag behind the disk; never overwrite a real note.
            self.refresh_vault(cx);
            self.open_note(&relative, window, cx);
            return;
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let title = relative
            .rsplit('/')
            .next()
            .unwrap_or(&relative)
            .trim_end_matches(".md");
        let content = format!("# {title}\n\n");

        match storage::write_atomic(&path, &content) {
            Ok(()) => {
                self.refresh_vault(cx);
                self.set_status(StatusKind::Success, format!("created note: {relative}"));
                self.open_note(&relative, window, cx);
            }
            Err(error) => {
                self.set_status(StatusKind::Error, format!("could not create note: {error}"));
                cx.notify();
            }
        }
    }

    /// Create a blank note, optionally inside a folder. A name containing `/` is taken as
    /// a vault-relative path.
    fn create_note_in(
        &mut self,
        dir: Option<&str>,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = name.trim();
        if name.is_empty() {
            self.set_status(StatusKind::Error, "a note needs a name".to_string());
            cx.notify();
            return;
        }

        let mut relative = if name.contains('/') {
            name.to_string()
        } else {
            match dir {
                Some(dir) if !dir.is_empty() => format!("{dir}/{name}"),
                _ => name.to_string(),
            }
        };
        if !relative.to_ascii_lowercase().ends_with(".md") {
            relative.push_str(".md");
        }
        let relative = sanitize_relative(&relative);
        if relative.is_empty() {
            self.set_status(StatusKind::Error, "a note needs a name".to_string());
            cx.notify();
            return;
        }

        let title = relative
            .rsplit('/')
            .next()
            .unwrap_or(&relative)
            .trim_end_matches(".md");
        let content = format!("# {title}\n\n");

        match vaultfs::create_note(&self.vault_root, &relative, &content) {
            Ok(_) => {
                self.refresh_vault(cx);
                self.set_status(StatusKind::Success, format!("created note: {relative}"));
                self.open_note(&relative, window, cx);
            }
            Err(error) => {
                self.set_status(StatusKind::Error, format!("could not create note: {error}"));
                cx.notify();
            }
        }
    }

    /// Give the active untitled buffer a file name, writing its current contents there.
    fn save_active_as(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let name = name.trim();
        if name.is_empty() {
            self.set_status(StatusKind::Error, "a note needs a name".to_string());
            cx.notify();
            return;
        }
        let mut relative = name.to_string();
        if !relative.to_ascii_lowercase().ends_with(".md") {
            relative.push_str(".md");
        }
        let relative = sanitize_relative(&relative);
        if relative.is_empty() {
            self.set_status(StatusKind::Error, "a note needs a name".to_string());
            cx.notify();
            return;
        }

        // Saving under the same name is just a save; keep the tab and its history.
        let current = self
            .tabs
            .get(self.active_tab)
            .and_then(|tab| tab.relative.clone());
        if current.as_deref() == Some(relative.as_str()) {
            let result = self.editor.update(cx, |editor, _cx| editor.save());
            match result {
                Ok(()) => self.set_status(StatusKind::Success, format!("saved {relative}")),
                Err(error) => {
                    self.set_status(StatusKind::Error, format!("could not save note: {error}"))
                }
            }
            window.focus(&self.editor.focus_handle(cx));
            cx.notify();
            return;
        }

        if self.vault_root.join(&relative).exists() {
            self.set_status(StatusKind::Error, format!("{relative} already exists"));
            cx.notify();
            return;
        }

        let content = self.editor.read(cx).text();
        match vaultfs::create_note(&self.vault_root, &relative, &content) {
            Ok(path) => {
                let editor = self.editor.clone();
                editor.update(cx, |editor, _cx| {
                    editor.set_path(Some(path));
                    // The file already holds this content; this just marks the buffer clean.
                    let _ = editor.save();
                });
                let index = self.active_tab;
                if let Some(tab) = self.tabs.get_mut(index) {
                    tab.relative = Some(relative.clone());
                    tab.title = tab_title(&relative);
                }
                self.active = Some(relative.clone());
                self.show_settings = false;
                self.refresh_vault(cx);
                self.remember_session(cx);
                self.set_status(StatusKind::Success, format!("saved as {relative}"));
                window.focus(&self.editor.focus_handle(cx));
            }
            Err(error) => {
                self.set_status(StatusKind::Error, format!("could not save note: {error}"));
            }
        }
        cx.notify();
    }

    fn create_folder_in(&mut self, dir: Option<&str>, name: &str, cx: &mut Context<Self>) {
        let name = name.trim();
        if name.is_empty() {
            self.set_status(StatusKind::Error, "a folder needs a name".to_string());
            cx.notify();
            return;
        }
        let relative = match dir {
            Some(dir) if !dir.is_empty() => format!("{dir}/{name}"),
            _ => name.to_string(),
        };
        match vaultfs::create_folder(&self.vault_root, &relative) {
            Ok(_) => {
                self.refresh_vault(cx);
                self.set_status(
                    StatusKind::Success,
                    format!("created folder: {}", sanitize_relative(&relative)),
                );
            }
            Err(error) => {
                self.set_status(
                    StatusKind::Error,
                    format!("could not create folder: {error}"),
                );
            }
        }
        cx.notify();
    }

    /// Rename or move an entry, keeping open tabs pointed at the new path.
    fn rename_entry(&mut self, from: &str, to: &str, cx: &mut Context<Self>) {
        let mut to = sanitize_relative(to);
        if to.is_empty() {
            self.set_status(StatusKind::Error, "a name is required".to_string());
            cx.notify();
            return;
        }
        // Renaming a note by its display name should keep the extension.
        if from.to_ascii_lowercase().ends_with(".md") && !to.to_ascii_lowercase().ends_with(".md") {
            to.push_str(".md");
        }

        if let Err(error) = vaultfs::rename(&self.vault_root, from, &to) {
            self.set_status(StatusKind::Error, format!("could not rename: {error}"));
            cx.notify();
            return;
        }

        let root = self.vault_root.clone();
        let prefix = format!("{from}/");
        for tab in &mut self.tabs {
            let Some(relative) = tab.relative.clone() else {
                continue;
            };
            let updated = if relative == from {
                Some(to.clone())
            } else if relative.starts_with(&prefix) {
                Some(format!("{to}/{}", &relative[prefix.len()..]))
            } else {
                None
            };
            if let Some(updated) = updated {
                tab.title = tab_title(&updated);
                tab.relative = Some(updated.clone());
                let path = root.join(&updated);
                tab.editor
                    .update(cx, |editor, _cx| editor.set_path(Some(path)));
            }
        }
        self.active = self
            .tabs
            .get(self.active_tab)
            .and_then(|tab| tab.relative.clone());
        self.refresh_vault(cx);
        self.remember_session(cx);
        self.set_status(StatusKind::Success, format!("renamed {from} to {to}"));
        cx.notify();
    }

    /// Move an entry to `.dragon-glass/trash`, closing its tabs first so unsaved edits are
    /// written before the file leaves the vault.
    fn delete_entry(&mut self, relative: &str, window: &mut Window, cx: &mut Context<Self>) {
        let prefix = format!("{relative}/");
        let indices: Vec<usize> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| {
                tab.relative
                    .as_deref()
                    .is_some_and(|path| path == relative || path.starts_with(&prefix))
            })
            .map(|(index, _)| index)
            .collect();

        // Never trash a note whose unsaved buffer could not be written.
        for index in &indices {
            let editor = self.tabs[*index].editor.clone();
            if editor.read(cx).is_dirty() {
                if let Err(error) = editor.update(cx, |editor, _| editor.save()) {
                    self.set_status(
                        StatusKind::Error,
                        format!("could not save before delete: {error}"),
                    );
                    cx.notify();
                    return;
                }
            }
        }
        for index in indices.into_iter().rev() {
            self.close_tab(index, window, cx);
        }

        match vaultfs::trash(&self.vault_root, relative) {
            Ok(_) => {
                self.refresh_vault(cx);
                self.set_status(StatusKind::Success, format!("moved to trash: {relative}"));
            }
            Err(error) => {
                self.set_status(StatusKind::Error, format!("could not delete: {error}"));
            }
        }
        cx.notify();
    }

    fn open_tree_menu(
        &mut self,
        path: String,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tree_menu = Some(TreeTarget { path, is_dir });
        self.open_panel(SwitcherMode::FileMenu, window, cx);
    }

    /// Compare a recovery snapshot with the current buffer.
    fn compare_snapshot(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(relative) = self.active.clone() else {
            return;
        };
        let snapshot = match history::read(&self.vault_root, &relative, id) {
            Ok(text) => text,
            Err(error) => {
                self.set_status(
                    StatusKind::Error,
                    format!("could not read snapshot: {error}"),
                );
                cx.notify();
                return;
            }
        };
        let current = self.editor.read(cx).text();
        let diff = diff::lines(&snapshot, &current);
        self.diff_title = format!("current ↔ snapshot {id}");
        self.diff_summary = diff.summary();
        self.diff_rows = Rc::new(diff.lines);
        self.show_diff = true;
        self.show_graph = false;
        self.show_query = false;
        self.show_settings = false;
        self.set_status(
            StatusKind::Info,
            format!("snapshot diff: {}", self.diff_summary),
        );
        cx.notify();
    }

    /// Restore a snapshot over the note. The current buffer is snapshotted first, so the
    /// restore is reversible through the same history list.
    fn restore_snapshot(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(relative) = self.active.clone() else {
            return;
        };
        let snapshot = match history::read(&self.vault_root, &relative, id) {
            Ok(text) => text,
            Err(error) => {
                self.set_status(
                    StatusKind::Error,
                    format!("could not read snapshot: {error}"),
                );
                cx.notify();
                return;
            }
        };

        let current = self.editor.read(cx).text();
        let _ = history::record(&self.vault_root, &relative, &current);
        let path = self.vault_root.join(&relative);
        if let Err(error) = storage::write_atomic(&path, &snapshot) {
            self.set_status(
                StatusKind::Error,
                format!("could not restore snapshot: {error}"),
            );
            cx.notify();
            return;
        }
        self.editor.update(cx, |editor, cx| {
            let _ = editor.reload();
            cx.notify();
        });
        self.cache_epoch = self.cache_epoch.wrapping_add(1);
        self.refresh_properties(cx);
        self.refresh_git();
        self.set_status(StatusKind::Success, format!("restored snapshot {id}"));
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    /// Resize a side panel while its splitter is being dragged.
    ///
    /// A full app repaint is expensive, so movement is applied at most once per frame
    /// (16 ms) and only when the width actually changed by a visible amount.
    fn apply_resize_x(&mut self, x: f32, force: bool, cx: &mut Context<Self>) {
        let Some(drag) = self.resize else {
            return;
        };

        let dx = x - drag.start_x;
        let width = match drag.panel {
            ResizePanel::Sidebar => drag.start_width + dx,
            ResizePanel::Outline | ResizePanel::Relations => drag.start_width - dx,
        };
        let width = width.clamp(140.0, 560.0).round();

        let now = std::time::Instant::now();
        if !force {
            if (width - drag.last_width).abs() < 1.0 {
                return;
            }
            if now.duration_since(drag.last_tick) < std::time::Duration::from_millis(16)
                && (width - drag.last_width).abs() < 8.0
            {
                return;
            }
        }

        self.resize = Some(ResizeDrag {
            last_width: width,
            last_tick: now,
            ..drag
        });

        match drag.panel {
            ResizePanel::Sidebar => self.sidebar_width = width,
            ResizePanel::Outline => self.outline_width = width,
            ResizePanel::Relations => self.relations_width = width,
        }
        cx.notify();
    }

    fn apply_resize(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        self.apply_resize_x(f32::from(event.position.x), false, cx);
    }

    /// Freeze or resume editor wrapping for every open tab while a splitter moves.
    fn set_editors_resizing(&mut self, resizing: bool, cx: &mut Context<Self>) {
        let editors: Vec<Entity<Editor>> = self.tabs.iter().map(|tab| tab.editor.clone()).collect();
        for editor in editors {
            editor.update(cx, |editor, _| editor.set_resizing(resizing));
        }
    }

    /// End a splitter drag when the mouse button is released.
    fn finish_resize(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.resize.is_some() {
            self.apply_resize_x(f32::from(event.position.x), true, cx);
            self.resize = None;
            self.set_editors_resizing(false, cx);
            cx.notify();
        }
    }

    /// Create an editor pointed at this vault.
    fn new_editor(&self, cx: &mut Context<Self>) -> Entity<Editor> {
        let editor = cx.new(Editor::new);
        let root = self.vault_root.clone();
        let folder = self.config.attachments_folder.clone();
        let opener = self.link_opener(cx);
        let notes = (*self.note_suggestions).clone();
        let tags = (*self.tag_suggestions).clone();
        editor.update(cx, |editor, _cx| {
            editor.set_vault_root(&root);
            editor.set_attachments_folder(&folder);
            editor.set_link_opener(opener);
            editor.set_vim_enabled(self.config.vim_mode);
            editor.set_font_size(px(self.config.font_size as f32));
            editor.set_suggestions(notes, tags);
        });
        self.install_editor_callbacks(&editor, cx);
        editor
    }

    /// Wire an editor to this app: Save As, close, status and conflict reporting.
    fn install_editor_callbacks(&self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let save_as_weak = cx.entity().downgrade();
        let save_as: editor::SaveAsHandler = Rc::new(move |window, cx| {
            let _ = save_as_weak.update(cx, |app, cx| app.open_save_as(window, cx));
        });
        let close_weak = cx.entity().downgrade();
        let close_tab: editor::CloseTabHandler = Rc::new(move |window, cx| {
            let _ = close_weak.update(cx, |app, cx| {
                let index = app.active_tab;
                app.close_tab(index, window, cx);
            });
        });
        let status_weak = cx.entity().downgrade();
        let status: editor::StatusReporter = Rc::new(move |kind, message, cx| {
            let _ = status_weak.update(cx, |app, cx| {
                app.set_status(kind, message.to_string());
                cx.notify();
            });
        });
        let conflict_weak = cx.entity().downgrade();
        let conflict: editor::ConflictHandler = Rc::new(move |cx| {
            let _ = conflict_weak.update(cx, |app, cx| app.mark_conflict(cx));
        });
        editor.update(cx, |editor, _cx| {
            editor.set_save_as(save_as);
            editor.set_close_tab(close_tab);
            editor.set_status_reporter(status);
            editor.set_conflict_handler(conflict);
            editor.set_autosave(self.config.autosave);
        });
    }

    /// Wire every open tab, including the ones restored from the session file.
    fn configure_editors(&mut self, cx: &mut Context<Self>) {
        let editors: Vec<Entity<Editor>> = self.tabs.iter().map(|tab| tab.editor.clone()).collect();
        for editor in editors {
            self.install_editor_callbacks(&editor, cx);
        }
    }

    /// Open the name prompt that turns the active buffer into a file on disk.
    fn open_save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_pending_inputs();
        self.pending_save_as = true;
        self.switcher = Some(Switcher {
            mode: SwitcherMode::NewNoteName,
            query: self.active.clone().unwrap_or_default(),
            results: Vec::new(),
            selected: 0,
            property_key: None,
            capture_action: None,
        });
        self.show_settings = false;
        window.focus(&self.switcher_focus);
        self.set_status(StatusKind::Info, "name this note to save it");
        cx.notify();
    }

    fn save_as_action(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        if self.switcher.is_some() {
            return;
        }
        self.open_save_as(window, cx);
    }

    /// Load `relative` into an existing editor. Returns false if the file could not be read.
    fn load_into(&self, editor: &Entity<Editor>, relative: &str, cx: &mut Context<Self>) -> bool {
        let root = self.vault_root.clone();
        let folder = self.config.attachments_folder.clone();
        let path = root.join(relative);
        editor.update(cx, |editor, cx| {
            editor.set_vault_root(&root);
            editor.set_attachments_folder(&folder);
            match editor.load(&path) {
                Ok(()) => {
                    cx.notify();
                    true
                }
                Err(_) => false,
            }
        })
    }

    /// Save the active note and reading mode, so the next launch reopens them.
    fn remember_session(&self, cx: &Context<Self>) {
        let tabs: Vec<session::TabSession> = self
            .tabs
            .iter()
            .map(|tab| {
                let editor = tab.editor.read(cx);
                let (line, col) = editor.cursor_position();
                session::TabSession {
                    note: tab.relative.clone().unwrap_or_default(),
                    reading: editor.is_reading(),
                    line,
                    col,
                    scroll: editor.scroll_offset(),
                    reading_block: editor.reading_block(),
                    anchor: editor.selection_anchor(),
                    folded: editor.folded_headings(),
                }
            })
            .collect();
        let saved = session::Session {
            version: 2,
            active: self.active_tab.min(tabs.len().saturating_sub(1)),
            tabs,
        };
        let _ = session::save(&self.vault_root, &saved);
    }

    /// Show a one-line status message. Errors stay until replaced; the rest expire.
    fn set_status(&mut self, kind: StatusKind, text: impl Into<String>) {
        self.status = Some(Status {
            kind,
            text: text.into(),
            at: std::time::Instant::now(),
        });
    }

    /// Remove the status line (used when the action it described is cancelled).
    fn clear_status(&mut self) {
        self.status = None;
    }

    /// The title the OS should show: the active note, a dirty marker and the app name.
    fn desired_window_title(&self, cx: &Context<Self>) -> String {
        let Some(tab) = self.tabs.get(self.active_tab) else {
            return "Dragon Glass".to_string();
        };
        let dirty = tab.editor.read(cx).is_dirty();
        let marker = if dirty { " \u{25CF}" } else { "" };
        let name = tab.title.as_str();
        if self.minimal {
            format!("{name}{marker}")
        } else {
            format!("{name}{marker} — Dragon Glass")
        }
    }

    /// Drop a status line that has outlived its kind, and arm one shot to do it later.
    ///
    /// The timer is only created while a message is visible, so a quiet window stays idle.
    fn expire_status(&mut self, cx: &mut Context<Self>) {
        let Some(status) = &self.status else {
            return;
        };
        if !status.kind.expires() {
            return;
        }
        let lifetime = status.kind.lifetime();
        let elapsed = status.at.elapsed();
        if elapsed >= lifetime {
            self.status = None;
            return;
        }
        if self.status_timer_armed {
            return;
        }

        self.status_timer_armed = true;
        let wait = lifetime - elapsed;
        cx.spawn(async move |this, cx| {
            Timer::after(wait).await;
            let _ = this.update(cx, |app, cx| {
                app.status_timer_armed = false;
                app.expire_status(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Save every dirty tab, returning how many were written. All tabs are attempted even
    /// when one fails, so a quit never leaves later notes unwritten because of an earlier
    /// error.
    fn save_all(&mut self, cx: &mut Context<Self>) -> Result<usize, String> {
        let mut saved = 0;
        let mut first_error: Option<String> = None;

        for index in 0..self.tabs.len() {
            let editor = self.tabs[index].editor.clone();
            if !editor.read(cx).is_dirty() {
                continue;
            }
            if !editor.read(cx).has_path() {
                first_error
                    .get_or_insert_with(|| "an untitled note has no file name yet".to_string());
                continue;
            }
            match editor.update(cx, |editor, _cx| editor.save_checked()) {
                SaveReport::Saved { .. } => saved += 1,
                SaveReport::Conflict { .. } => {
                    if let Some(tab) = self.tabs.get_mut(index) {
                        tab.conflict = true;
                    }
                    first_error.get_or_insert_with(|| {
                        "a note changed on disk; resolve it before quitting".to_string()
                    });
                }
                SaveReport::Error { message } => {
                    first_error.get_or_insert(message);
                }
            }
        }

        let active_conflict = self
            .tabs
            .get(self.active_tab)
            .is_some_and(|tab| tab.conflict);
        self.conflict = active_conflict;

        match first_error {
            Some(error) => Err(error),
            None => Ok(saved),
        }
    }

    /// macOS quits through `applicationWillTerminate`, which does not call
    /// `windowShouldClose`. Cmd+Q is handled by `quit_action`, which runs the normal
    /// Save / Discard / Cancel contract; this observer is the last resort for a
    /// system-initiated termination (logout, shutdown), where there is no window to ask.
    fn install_quit_saver(&self, cx: &mut Context<Self>) {
        if cfg!(target_os = "macos") {
            cx.on_app_quit(|app, cx| {
                let _ = app.save_all(cx);
                async {}
            })
            .detach();
        }
    }

    /// Intercept the window close so unsaved notes are not silently dropped.
    fn request_close(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.remember_session(cx);

        if self.allow_close {
            return true;
        }

        let dirty = self
            .tabs
            .iter()
            .filter(|tab| tab.editor.read(cx).is_dirty())
            .count();
        if dirty == 0 {
            return true;
        }

        self.quit_prompt = true;
        self.set_status(
            StatusKind::Warn,
            format!(
                "{dirty} open {} unsaved changes",
                if dirty == 1 { "note has" } else { "notes have" }
            ),
        );
        cx.notify();
        false
    }

    fn save_and_quit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        match self.save_all(cx) {
            Ok(saved) => {
                self.allow_close = true;
                self.quit_prompt = false;
                self.close_requested = true;
                self.set_status(StatusKind::Success, format!("saved {saved} note(s)"));
                if self.quit_after_prompt {
                    self.quit_after_prompt = false;
                    cx.quit();
                }
                cx.notify();
            }
            Err(error) => {
                // Stay open so the user can resolve the conflict or the failure.
                self.set_status(StatusKind::Error, format!("could not save: {error}"));
                cx.notify();
            }
        }
    }

    fn discard_and_quit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.allow_close = true;
        self.quit_prompt = false;
        self.close_requested = true;
        if self.quit_after_prompt {
            self.quit_after_prompt = false;
            cx.quit();
        }
        cx.notify();
    }

    fn cancel_quit(&mut self, cx: &mut Context<Self>) {
        self.quit_prompt = false;
        self.quit_after_prompt = false;
        self.clear_status();
        cx.notify();
    }

    /// Switch the active tab, syncing the editor, outline and properties.
    fn activate_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }

        if self.tabs[index].pending_load {
            if let Some(relative) = self.tabs[index].relative.clone() {
                let editor = self.tabs[index].editor.clone();
                let loaded = self.load_into(&editor, &relative, cx);
                if let Some(tab) = self.tabs.get_mut(index) {
                    tab.pending_load = false;
                    tab.load_error = if loaded {
                        None
                    } else {
                        Some("the file could not be read".to_string())
                    };
                }
            } else {
                self.tabs[index].pending_load = false;
            }
        }

        self.active_tab = index;
        self.editor = self.tabs[index].editor.clone();
        self.active = self.tabs[index].relative.clone();
        self.conflict = self.tabs[index].conflict;
        self.cache_epoch = self.cache_epoch.wrapping_add(1);
        self.collapsed_outline.clear();
        self.show_settings = false;
        self.show_graph = false;
        self.show_query = false;
        self.show_diff = false;
        self.normalize_splits();
        self.refresh_properties(cx);
        if self.show_relations {
            self.relations = relations::build(&self.note_refs);
        }
        self.remember_session(cx);
        cx.notify();

        self.check_recovery(cx);
    }

    fn normalize_splits(&mut self) {
        let len = self.tabs.len();
        let active = self.active_tab;
        self.split_tabs
            .retain(|index| *index < len && *index != active);
        self.split_tabs.sort_unstable();
        self.split_tabs.dedup();
    }

    fn reindex_splits_after_remove(&mut self, removed: usize) {
        self.split_tabs = self
            .split_tabs
            .iter()
            .filter_map(|index| match index.cmp(&removed) {
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some(index - 1),
                std::cmp::Ordering::Less => Some(*index),
            })
            .collect();
    }

    fn open_note(&mut self, relative: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open_note_impl(relative, window, cx, true);
    }

    /// Open a note. `record` is false while back/forward replays history.
    fn open_note_impl(
        &mut self,
        relative: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        record: bool,
    ) {
        self.show_settings = false;
        // Already open: just focus it.
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.relative.as_deref() == Some(relative))
        {
            self.activate_tab(index, cx);
            window.focus(&self.editor.focus_handle(cx));
            return;
        }

        // Reuse a clean, empty tab when there is one.
        let empty = self.tabs.iter().enumerate().find_map(|(index, tab)| {
            (tab.relative.is_none() && !tab.editor.read(cx).is_dirty()).then_some(index)
        });

        // Following a link keeps the current mode: a reader stays a reader.
        let reading = self.editor.read(cx).is_reading();

        let title = tab_title(relative);
        if let Some(index) = empty {
            let editor = self.tabs[index].editor.clone();
            if !self.load_into(&editor, relative, cx) {
                self.set_status(StatusKind::Error, format!("could not open {relative}"));
                cx.notify();
                return;
            }
            editor.update(cx, |editor, cx| {
                editor.set_reading_mode(reading);
                cx.notify();
            });
            self.tabs[index].relative = Some(relative.to_string());
            self.tabs[index].title = title;
            self.tabs[index].conflict = false;
            self.activate_tab(index, cx);
        } else {
            let editor = self.new_editor(cx);
            if !self.load_into(&editor, relative, cx) {
                self.set_status(StatusKind::Error, format!("could not open {relative}"));
                cx.notify();
                return;
            }
            editor.update(cx, |editor, cx| {
                editor.set_reading_mode(reading);
                cx.notify();
            });
            self.tabs.push(Tab {
                relative: Some(relative.to_string()),
                editor: editor.clone(),
                title,
                conflict: false,
                load_error: None,
                pending_load: false,
            });
            self.active_tab = self.tabs.len() - 1;
            self.editor = editor;
            self.active = Some(relative.to_string());
            self.conflict = false;
            self.cache_epoch = self.cache_epoch.wrapping_add(1);
            self.collapsed_outline.clear();
            self.show_graph = false;
            self.show_query = false;
            self.show_diff = false;
            self.normalize_splits();
            self.refresh_properties(cx);
            if self.show_relations {
                self.relations = relations::build(&self.note_refs);
            }
            self.remember_session(cx);
            cx.notify();
        }

        window.focus(&self.editor.focus_handle(cx));
        self.check_recovery(cx);
        if record {
            self.remember_nav(cx);
        }
    }

    /// Remember where the user is now, and drop any forward history.
    fn remember_nav(&mut self, cx: &Context<Self>) {
        let Some(relative) = self.active.clone() else {
            return;
        };
        let (reading, line, reading_block) = {
            let editor = self.editor.read(cx);
            (
                editor.is_reading(),
                editor.cursor_position().0,
                editor.reading_block(),
            )
        };
        if let Some(current) = self.nav_history.get(self.nav_index) {
            if current.relative == relative && current.line == line && current.reading == reading {
                return;
            }
        }
        if !self.nav_history.is_empty() {
            self.nav_history.truncate(self.nav_index + 1);
        }
        self.nav_history.push(NavEntry {
            relative,
            reading,
            line,
            reading_block,
        });
        if self.nav_history.len() > 100 {
            self.nav_history.remove(0);
        }
        self.nav_index = self.nav_history.len().saturating_sub(1);
    }

    /// Reopen the place at `nav_index`, restoring its mode and position.
    fn navigate_to_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.nav_history.get(self.nav_index).cloned() else {
            return;
        };
        self.open_note_impl(&entry.relative, window, cx, false);
        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| {
            editor.set_reading_mode(entry.reading);
            editor.set_cursor_position(entry.line, 0);
            editor.scroll_reading_to(entry.reading_block);
            cx.notify();
        });
        self.set_status(
            StatusKind::Info,
            format!(
                "{} ({} of {})",
                entry.relative,
                self.nav_index + 1,
                self.nav_history.len()
            ),
        );
    }

    fn toggle_zen(&mut self, _: &ToggleZen, _window: &mut Window, cx: &mut Context<Self>) {
        self.zen = !self.zen;
        cx.set_global(FocusMode(self.zen));
        let editors: Vec<Entity<Editor>> = self.tabs.iter().map(|tab| tab.editor.clone()).collect();
        for editor in editors {
            editor.update(cx, |_, cx| cx.notify());
        }
        self.set_status(
            StatusKind::Info,
            if self.zen {
                "zen mode — Escape to leave"
            } else {
                "zen mode off"
            },
        );
        cx.notify();
    }

    /// Mouse back/forward buttons use the same history as the shortcuts.
    fn nav_button(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.button {
            MouseButton::Navigate(NavigationDirection::Back) => self.nav_back(&NavBack, window, cx),
            MouseButton::Navigate(NavigationDirection::Forward) => {
                self.nav_forward(&NavForward, window, cx)
            }
            _ => {}
        }
    }

    fn nav_back(&mut self, _: &NavBack, window: &mut Window, cx: &mut Context<Self>) {
        if self.nav_history.is_empty() || self.nav_index == 0 {
            self.set_status(StatusKind::Info, "no earlier note");
            cx.notify();
            return;
        }
        self.nav_index -= 1;
        self.navigate_to_current(window, cx);
        cx.notify();
    }

    fn nav_forward(&mut self, _: &NavForward, window: &mut Window, cx: &mut Context<Self>) {
        if self.nav_index + 1 >= self.nav_history.len() {
            self.set_status(StatusKind::Info, "no later note");
            cx.notify();
            return;
        }
        self.nav_index += 1;
        self.navigate_to_current(window, cx);
        cx.notify();
    }

    /// Close a tab. Dirty notes are saved first so closing never loses edits.
    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }

        // Ctrl+W on an untitled note asks for a name instead of refusing the close.
        let untitled_dirty = {
            let editor = self.tabs[index].editor.read(cx);
            !editor.has_path() && editor.is_dirty()
        };
        if untitled_dirty {
            self.activate_tab(index, cx);
            self.set_status(StatusKind::Info, "name this note, then close it again");
            self.open_save_as(window, cx);
            return;
        }

        let save_result = self.tabs[index].editor.update(cx, |editor, _cx| {
            if editor.is_dirty() {
                editor.save_checked()
            } else {
                SaveReport::Saved {
                    fingerprint: dragonglass_core::fingerprint::DocumentFingerprint::of_text(""),
                }
            }
        });
        match save_result {
            SaveReport::Saved { .. } => {}
            SaveReport::Conflict { .. } => {
                if let Some(tab) = self.tabs.get_mut(index) {
                    tab.conflict = true;
                }
                if index == self.active_tab {
                    self.conflict = true;
                }
                let title = self.tabs[index].title.clone();
                self.set_status(
                    StatusKind::Error,
                    format!("{title} changed on disk; reload or keep mine before closing"),
                );
                cx.notify();
                return;
            }
            SaveReport::Error { message } => {
                self.set_status(
                    StatusKind::Error,
                    format!("could not save before closing: {message}"),
                );
                cx.notify();
                return;
            }
        }

        let was_active = index == self.active_tab;
        let title = self.tabs[index].title.clone();
        let closed = {
            let tab = &self.tabs[index];
            ClosedTab {
                relative: tab.relative.clone(),
                title: tab.title.clone(),
                reading: tab.editor.read(cx).is_reading(),
                line: tab.editor.read(cx).cursor_line(),
            }
        };
        self.closed_tabs.push(closed);
        // A small stack, not a history: the last ten closes are undoable.
        if self.closed_tabs.len() > 10 {
            self.closed_tabs.remove(0);
        }
        self.tabs.remove(index);
        self.reindex_splits_after_remove(index);

        if self.tabs.is_empty() {
            let editor = self.new_editor(cx);
            self.tabs.push(Tab {
                relative: None,
                editor,
                title: "Untitled".to_string(),
                conflict: false,
                load_error: None,
                pending_load: false,
            });
            self.activate_tab(0, cx);
        } else if was_active {
            let next = index.min(self.tabs.len() - 1);
            self.activate_tab(next, cx);
        } else if index < self.active_tab {
            self.active_tab -= 1;
        }

        self.normalize_splits();
        self.set_status(
            StatusKind::Info,
            format!("closed {title} — Ctrl+Shift+T to reopen"),
        );
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    fn cycle_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() < 2 {
            return;
        }
        let count = self.tabs.len() as isize;
        let next = (self.active_tab as isize + delta).rem_euclid(count) as usize;
        self.activate_tab(next, cx);
        window.focus(&self.editor.focus_handle(cx));
    }

    /// React to a note changing on disk while it is open.
    fn on_external_change(&mut self, changed: &str, cx: &mut Context<Self>) {
        let changed_path = Path::new(changed);
        let mut matched = false;
        let mut active_matched = false;
        let mut reloaded = false;
        let mut conflicted = false;

        for index in 0..self.tabs.len() {
            let Some(relative) = self.tabs[index].relative.clone() else {
                continue;
            };
            let path = self.vault_root.join(&relative);
            let same_file = match (path.canonicalize(), changed_path.canonicalize()) {
                (Ok(a), Ok(b)) => a == b,
                _ => path.to_string_lossy() == changed,
            };
            if !same_file {
                continue;
            }

            matched = true;
            active_matched |= index == self.active_tab;
            eprintln!("[dragon-glass] external change: {changed}");

            let Ok(disk) = std::fs::read_to_string(&path) else {
                continue;
            };
            let editor = self.tabs[index].editor.clone();
            let (buffer, dirty) = {
                let editor = editor.read(cx);
                (editor.text(), editor.is_dirty())
            };

            match conflict::classify(&buffer, dirty, &disk) {
                conflict::Resolution::Ignore => {}
                conflict::Resolution::Reload => {
                    editor.update(cx, |editor, cx| {
                        let _ = editor.reload();
                        cx.notify();
                    });
                    self.tabs[index].conflict = false;
                    reloaded = true;
                }
                conflict::Resolution::Conflict => {
                    self.tabs[index].conflict = true;
                    conflicted = true;
                }
            }
        }

        if !matched {
            return;
        }

        if active_matched {
            self.conflict = self.tabs[self.active_tab].conflict;
            self.cache_epoch = self.cache_epoch.wrapping_add(1);
            self.refresh_properties(cx);
            if self.conflict {
                self.set_status(
                    StatusKind::Warn,
                    "changed on disk - saving will overwrite the file".to_string(),
                );
            } else if reloaded {
                self.set_status(
                    StatusKind::Warn,
                    "reloaded from disk (changed externally)".to_string(),
                );
            }
        } else if conflicted {
            self.set_status(
                StatusKind::Warn,
                "a note changed on disk in another tab".to_string(),
            );
        }

        cx.notify();
    }

    fn refresh_git(&mut self) {
        self.git_status = if git::is_repo(&self.vault_root) {
            git::status(&self.vault_root).ok()
        } else {
            None
        };
    }

    /// Re-scan the vault after files are created or removed.
    fn refresh_vault(&mut self, cx: &mut Context<Self>) {
        if let Ok(vault) = Vault::open(&self.vault_root) {
            self.note_refs = vault.notes();
            self.note_index = resolve::NoteIndex::new(&self.note_refs);
            self.tree = tree::build_with_dirs(&self.note_refs, &vault.folders());
            self.notes = self.note_refs.len();
            self.full_graph = graph::build(&self.note_refs);
            if self.show_relations {
                self.relations = relations::build(&self.note_refs);
            }
        }
        self.note_suggestions = Rc::new(
            self.note_refs
                .iter()
                .map(|note| (note.path_no_ext().to_string(), note.stem().to_string()))
                .collect(),
        );
        self.tag_suggestions = Rc::new(tags::vault_tags(&self.note_refs));
        let notes = (*self.note_suggestions).clone();
        let tag_list = (*self.tag_suggestions).clone();
        for tab in &self.tabs {
            tab.editor.update(cx, |editor, _| {
                editor.set_suggestions(notes.clone(), tag_list.clone())
            });
        }
        // A rescan is also when attachments may have appeared or disappeared.
        self.editor
            .update(cx, |editor, _| editor.invalidate_attachments());
        let mut rows = Vec::new();
        flatten(&self.tree, 0, &self.collapsed_dirs, &mut rows);
        self.file_rows = Rc::new(rows);
        self.refresh_git();
    }

    /// Show or hide a folder's children in the sidebar.
    fn toggle_dir(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.collapsed_dirs.remove(path) {
            self.collapsed_dirs.insert(path.to_string());
        }
        let mut rows = Vec::new();
        flatten(&self.tree, 0, &self.collapsed_dirs, &mut rows);
        self.file_rows = Rc::new(rows);
        cx.notify();
    }

    /// Fold or unfold a heading in the table of contents.
    fn toggle_outline(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.collapsed_outline.remove(&index) {
            self.collapsed_outline.insert(index);
        }
        cx.notify();
    }

    /// Unfold every ancestor of `index` so an outline jump lands on a visible heading.
    fn reveal_outline_heading(&mut self, index: usize) {
        let mut level = self
            .headings
            .get(index)
            .map(|heading| heading.level)
            .unwrap_or(1);
        for parent in (0..index).rev() {
            let Some(heading) = self.headings.get(parent) else {
                continue;
            };
            if heading.level < level {
                self.collapsed_outline.remove(&parent);
                level = heading.level;
                if level == 1 {
                    break;
                }
            }
        }
    }

    /// Open (or create) today's daily note, using `Templates/Daily.md` when present.
    fn open_daily(&mut self, _: &OpenDaily, window: &mut Window, cx: &mut Context<Self>) {
        self.create_periodic(Periodic::Daily, window, cx);
    }

    fn open_weekly(&mut self, _: &OpenWeekly, window: &mut Window, cx: &mut Context<Self>) {
        self.create_periodic(Periodic::Weekly, window, cx);
    }

    fn open_monthly(&mut self, _: &OpenMonthly, window: &mut Window, cx: &mut Context<Self>) {
        self.create_periodic(Periodic::Monthly, window, cx);
    }

    /// Open or create a daily/weekly/monthly note, using the matching template.
    fn create_periodic(&mut self, kind: Periodic, window: &mut Window, cx: &mut Context<Self>) {
        use chrono::Datelike as _;

        let now = chrono::Local::now();
        let date = now.format("%Y-%m-%d").to_string();
        let time = now.format("%H:%M").to_string();
        let iso = now.iso_week();

        let (folder, title, template_name) = match kind {
            Periodic::Daily => (
                self.config.daily_folder.clone(),
                date.clone(),
                "Daily.md".to_string(),
            ),
            Periodic::Weekly => (
                self.config.weekly_folder.clone(),
                format!("{}-W{:02}", iso.year(), iso.week()),
                "Weekly.md".to_string(),
            ),
            Periodic::Monthly => (
                self.config.monthly_folder.clone(),
                now.format("%Y-%m").to_string(),
                "Monthly.md".to_string(),
            ),
        };

        let relative = format!("{folder}/{title}.md");
        let path = self.vault_root.join(&relative);

        if !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let template_path = self
                .vault_root
                .join(&self.config.templates_folder)
                .join(&template_name);
            let content = match std::fs::read_to_string(&template_path) {
                Ok(template) => templates::render(
                    &template,
                    &[
                        ("date", date.as_str()),
                        ("time", time.as_str()),
                        ("title", title.as_str()),
                    ],
                ),
                Err(_) => format!("# {title}\n\n"),
            };

            if let Err(e) = storage::write_atomic(&path, &content) {
                self.set_status(StatusKind::Error, format!("could not create note: {e}"));
                cx.notify();
                return;
            }
            self.refresh_vault(cx);
        }

        self.open_note(&relative, window, cx);
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    fn compute_results(&self, mode: SwitcherMode, query: &str) -> Vec<SwitcherItem> {
        match mode {
            SwitcherMode::QuickSwitch => search::fuzzy_notes(&self.note_refs, query, 20)
                .into_iter()
                .map(|note| SwitcherItem {
                    label: note.stem().to_string(),
                    detail: note.relative.clone(),
                    item: Item::Note {
                        relative: note.relative,
                    },
                })
                .collect(),
            SwitcherMode::Search => search::full_text(&self.note_refs, query, 50)
                .into_iter()
                .map(|hit| SwitcherItem {
                    label: format!("{}:{}", hit.relative, hit.line_number),
                    detail: hit.line,
                    item: Item::NoteAt {
                        relative: hit.relative,
                        line: hit.line_number,
                    },
                })
                .collect(),
            SwitcherMode::GitMenu => {
                let mut items = Vec::new();
                if self.git_status.is_some() {
                    items.push(git_item(
                        "Commit",
                        "stage everything and commit",
                        GitAction::Commit,
                    ));
                    items.push(git_item(
                        "Pull",
                        "fast-forward from the remote",
                        GitAction::Pull,
                    ));
                    items.push(git_item("Push", "push to the remote", GitAction::Push));
                    items.push(git_item(
                        "Refresh",
                        "re-read git status",
                        GitAction::Refresh,
                    ));
                } else {
                    items.push(git_item(
                        "Initialize repository",
                        "git init -b main",
                        GitAction::Init,
                    ));
                }
                items
            }
            SwitcherMode::CommitInput => Vec::new(),
            SwitcherMode::PropertiesMenu => {
                let mut items = vec![SwitcherItem {
                    label: "Add property".to_string(),
                    detail: "key: value".to_string(),
                    item: Item::Property { key: None },
                }];
                for property in &self.properties {
                    items.push(SwitcherItem {
                        label: format!("Edit {}", property.key),
                        detail: property.value.display(),
                        item: Item::Property {
                            key: Some(property.key.clone()),
                        },
                    });
                }
                for property in &self.properties {
                    items.push(SwitcherItem {
                        label: format!("Delete {}", property.key),
                        detail: String::new(),
                        item: Item::DeleteProperty {
                            key: property.key.clone(),
                        },
                    });
                }
                items
            }
            SwitcherMode::PropertyInput => Vec::new(),
            SwitcherMode::QueryInput => Vec::new(),
            SwitcherMode::WorkspaceMenu => self
                .workspaces
                .iter()
                .map(|workspace| SwitcherItem {
                    label: workspace.name.clone(),
                    detail: describe_workspace(workspace),
                    item: Item::Workspace(workspace.name.clone()),
                })
                .collect(),
            SwitcherMode::WorkspaceName => Vec::new(),
            SwitcherMode::HotkeyMenu => HOTKEYS
                .iter()
                .map(|(action, default, label)| SwitcherItem {
                    label: (*label).to_string(),
                    detail: platform::label(
                        &keymap::key_for_action(&self.bindings, action)
                            .unwrap_or_else(|| (*default).to_string()),
                    ),
                    item: Item::Hotkey((*action).to_string()),
                })
                .collect(),
            SwitcherMode::HotkeyCapture => Vec::new(),
            SwitcherMode::GraphFilterInput => Vec::new(),
            SwitcherMode::NewNoteName => Vec::new(),
            SwitcherMode::SharedName => Vec::new(),
            SwitcherMode::RelationInput => Vec::new(),
            SwitcherMode::NewFolderName
            | SwitcherMode::RenameInput
            | SwitcherMode::DeleteConfirm
            | SwitcherMode::SettingInput => Vec::new(),
            SwitcherMode::FileMenu => {
                let Some(target) = &self.tree_menu else {
                    return Vec::new();
                };
                let menu_dir = if target.is_dir {
                    target.path.clone()
                } else {
                    target
                        .path
                        .rsplit_once('/')
                        .map(|(dir, _)| dir.to_string())
                        .unwrap_or_default()
                };

                let mut items = vec![
                    SwitcherItem {
                        label: if target.is_dir {
                            "Open folder".to_string()
                        } else {
                            "Open note".to_string()
                        },
                        detail: target.path.clone(),
                        item: Item::Tree(TreeAction::Open),
                    },
                    SwitcherItem {
                        label: "New note here".to_string(),
                        detail: if menu_dir.is_empty() {
                            "vault root".to_string()
                        } else {
                            menu_dir.clone()
                        },
                        item: Item::Tree(TreeAction::NewNote),
                    },
                    SwitcherItem {
                        label: "New folder here".to_string(),
                        detail: if menu_dir.is_empty() {
                            "vault root".to_string()
                        } else {
                            menu_dir.clone()
                        },
                        item: Item::Tree(TreeAction::NewFolder),
                    },
                ];
                if !target.path.is_empty() {
                    items.push(SwitcherItem {
                        label: "Rename / move".to_string(),
                        detail: target.path.clone(),
                        item: Item::Tree(TreeAction::Rename),
                    });
                    items.push(SwitcherItem {
                        label: "Delete (to trash)".to_string(),
                        detail: target.path.clone(),
                        item: Item::Tree(TreeAction::Delete),
                    });
                }

                let needle = query.to_lowercase();
                items.retain(|item| {
                    needle.is_empty() || item.label.to_lowercase().contains(&needle)
                });
                items
            }
            SwitcherMode::HistoryMenu => {
                let Some(relative) = self.active.clone() else {
                    return Vec::new();
                };
                let ids = history::list(&self.vault_root, &relative);
                let count = ids.len();
                let mut items = Vec::new();
                for (index, id) in ids.iter().rev().enumerate() {
                    let number = count - index;
                    let detail = if index == 0 {
                        format!("newest · {id}")
                    } else {
                        id.clone()
                    };
                    items.push(SwitcherItem {
                        label: format!("Compare with snapshot {number}"),
                        detail: detail.clone(),
                        item: Item::HistoryCompare(id.clone()),
                    });
                    items.push(SwitcherItem {
                        label: format!("Restore snapshot {number}"),
                        detail,
                        item: Item::HistoryRestore(id.clone()),
                    });
                }
                if items.is_empty() {
                    items.push(SwitcherItem {
                        label: "No snapshots yet".to_string(),
                        detail: "a snapshot is written whenever a note is overwritten".to_string(),
                        item: Item::Command(CommandId::History),
                    });
                }
                items
            }
            SwitcherMode::VersionMenu => {
                let Some(relative) = self.active.clone() else {
                    return Vec::new();
                };
                let mut items = vec![
                    SwitcherItem {
                        label: "Save current as version A".to_string(),
                        detail: "a/b versions".to_string(),
                        item: Item::VersionSave("A".to_string()),
                    },
                    SwitcherItem {
                        label: "Save current as version B".to_string(),
                        detail: "a/b versions".to_string(),
                        item: Item::VersionSave("B".to_string()),
                    },
                ];

                let versions = versions::list(&self.vault_root, &relative);
                for version in &versions {
                    let label = version.display_label();
                    items.push(SwitcherItem {
                        label: format!("Compare current with {label}"),
                        detail: version.id.clone(),
                        item: Item::VersionCompare {
                            left: None,
                            right: version.id.clone(),
                        },
                    });
                    items.push(SwitcherItem {
                        label: format!("Promote {label} to the note"),
                        detail: "snapshots the current note first".to_string(),
                        item: Item::VersionPromote(version.id.clone()),
                    });
                    items.push(SwitcherItem {
                        label: format!("Delete {label}"),
                        detail: String::new(),
                        item: Item::VersionDelete(version.id.clone()),
                    });
                }
                if versions.len() >= 2 {
                    items.push(SwitcherItem {
                        label: format!(
                            "Compare {} with {}",
                            versions[0].display_label(),
                            versions[1].display_label()
                        ),
                        detail: String::new(),
                        item: Item::VersionCompare {
                            left: Some(versions[0].id.clone()),
                            right: versions[1].id.clone(),
                        },
                    });
                }
                items
            }
            SwitcherMode::TemplateMenu | SwitcherMode::TemplateNewMenu => {
                let new_note = mode == SwitcherMode::TemplateNewMenu;
                templates::list(&self.vault_root, &self.config.templates_folder)
                    .into_iter()
                    .map(|template| SwitcherItem {
                        label: template.name.clone(),
                        detail: template.relative.clone(),
                        item: if new_note {
                            Item::TemplateNew(template.relative.clone())
                        } else {
                            Item::Template(template.relative.clone())
                        },
                    })
                    .collect()
            }
            SwitcherMode::SharedMenu | SwitcherMode::SharedEditMenu => {
                let edit = mode == SwitcherMode::SharedEditMenu;
                shared::list(&self.vault_root)
                    .into_iter()
                    .map(|block| SwitcherItem {
                        label: block.title.clone(),
                        detail: format!("{{{{shared:{}}}}}", block.slug),
                        item: if edit {
                            Item::SharedEdit(block.slug.clone())
                        } else {
                            Item::Shared(block.slug.clone())
                        },
                    })
                    .collect()
            }
            SwitcherMode::ReviewMenu => self
                .review_issues
                .iter()
                .map(|issue| SwitcherItem {
                    label: format!("{} · {}", issue.severity.label(), issue.rule),
                    detail: format!("line {}: {}", issue.line, issue.message),
                    item: Item::Issue(issue.line),
                })
                .collect(),
            SwitcherMode::CommandsMenu => vec![
                command_item("Quick switcher", "secondary+o", CommandId::QuickSwitch),
                command_item("Search", "secondary+shift+f", CommandId::Search),
                command_item("Daily note", "secondary+shift+d", CommandId::Daily),
                command_item("Weekly note", "secondary+shift+w", CommandId::Weekly),
                command_item("Monthly note", "secondary+shift+m", CommandId::Monthly),
                command_item("Properties", "secondary+shift+p", CommandId::Properties),
                command_item("Toggle bookmark", "secondary+shift+b", CommandId::Bookmark),
                command_item("Git", "secondary+g", CommandId::Git),
                command_item(
                    "Toggle reading mode",
                    "secondary+e",
                    CommandId::ToggleReading,
                ),
                command_item(
                    "Find and replace",
                    "secondary-alt+f",
                    CommandId::FindReplace,
                ),
                command_item("Toggle Vim mode", "secondary-alt+v", CommandId::ToggleVim),
                command_item(
                    "Fold/unfold heading",
                    "secondary-shift+[",
                    CommandId::ToggleFold,
                ),
                command_item("Save", "secondary+s", CommandId::Save),
                command_item("Toggle theme", "secondary+shift+l", CommandId::ToggleTheme),
                command_item("Settings", "secondary-,", CommandId::Settings),
                command_item("New note", "", CommandId::NewNote),
                command_item("New folder", "", CommandId::NewFolder),
                command_item("Rename current note", "", CommandId::RenameNote),
                command_item("Delete current note", "", CommandId::DeleteNote),
                command_item("Save note as", "", CommandId::SaveAs),
                command_item("Toggle zen mode", "secondary-alt+z", CommandId::Zen),
                command_item("File recovery", "", CommandId::History),
                command_item("Check for updates", "", CommandId::CheckUpdates),
                command_item("Graph view", "secondary+shift+g", CommandId::OpenGraph),
                command_item(
                    "Document relations",
                    "secondary+shift+r",
                    CommandId::Relations,
                ),
                command_item("Relation graph", "", CommandId::RelationGraph),
                command_item("A/B versions", "", CommandId::Versions),
                command_item("Save as version A", "", CommandId::SaveVersionA),
                command_item("Save as version B", "", CommandId::SaveVersionB),
                command_item("Insert template", "", CommandId::InsertTemplate),
                command_item("New note from template", "", CommandId::NewFromTemplate),
                command_item("Insert shared block", "", CommandId::InsertShared),
                command_item("Edit shared block", "", CommandId::EditShared),
                command_item(
                    "Extract selection as shared block",
                    "",
                    CommandId::ExtractShared,
                ),
                command_item("Reload note from disk", "", CommandId::ReloadNote),
                command_item("Review document", "", CommandId::ReviewDocument),
                command_item("Move block up", "ctrl-shift+up", CommandId::MoveBlockUp),
                command_item(
                    "Move block down",
                    "ctrl-shift+down",
                    CommandId::MoveBlockDown,
                ),
                command_item("Duplicate block", "", CommandId::DuplicateBlock),
                command_item("Delete block", "", CommandId::DeleteBlock),
                command_item("Add related note", "", CommandId::AddRelation),
                command_item("Run a query", "", CommandId::RunQuery),
                command_item("Save workspace", "", CommandId::SaveWorkspace),
                command_item("Keyboard shortcuts", "", CommandId::Hotkeys),
                command_item("Graph: filter", "", CommandId::GraphFilter),
                command_item("Graph: cycle depth", "", CommandId::GraphDepth),
                command_item("Toggle snippets", "", CommandId::ToggleSnippets),
                command_item(
                    "Read and write side by side",
                    "secondary-alt+1",
                    CommandId::TogglePreview,
                ),
                command_item("Pop preview out to a window", "", CommandId::DetachPreview),
                command_item("Load workspace", "", CommandId::LoadWorkspace),
                command_item("Open vault", "", CommandId::OpenVault),
                command_item("New window", "secondary+n", CommandId::NewWindow),
                command_item(
                    "Move tab to new window",
                    "secondary-alt+n",
                    CommandId::DetachTab,
                ),
                command_item(
                    "Two editors side by side",
                    "secondary-alt+2",
                    CommandId::ToggleSplitEditor,
                ),
                command_item("Close tab", "secondary+w", CommandId::CloseTab),
                command_item("Next tab", "ctrl-tab", CommandId::NextTab),
                command_item("Previous tab", "ctrl-shift-tab", CommandId::PrevTab),
            ]
            .into_iter()
            .chain(
                self.user_themes
                    .iter()
                    .enumerate()
                    .map(|(index, (name, _))| {
                        command_item(&format!("Theme: {name}"), "", CommandId::UserTheme(index))
                    }),
            )
            .chain(
                self.plugin_commands
                    .iter()
                    .enumerate()
                    .map(|(index, (_, _, title))| {
                        command_item(title, "plugin", CommandId::Plugin(index))
                    }),
            )
            .collect(),
        }
    }

    fn open_panel(&mut self, mode: SwitcherMode, window: &mut Window, cx: &mut Context<Self>) {
        self.switcher = Some(Switcher {
            mode,
            query: String::new(),
            results: self.compute_results(mode, ""),
            selected: 0,
            property_key: None,
            capture_action: None,
        });
        window.focus(&self.switcher_focus);
        cx.notify();
    }

    fn close_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switcher = None;
        // A cancelled dialog must not leave its target behind for the next one.
        self.pending_template = None;
        self.pending_dir = None;
        self.pending_path = None;
        self.pending_save_as = false;
        self.pending_setting = None;
        self.pending_shared = None;
        if self.show_settings {
            window.focus(&self.settings_focus);
        } else {
            window.focus(&self.editor.focus_handle(cx));
        }
        cx.notify();
    }

    /// Drop the target of any half-finished dialog before opening a general panel.
    fn clear_pending_inputs(&mut self) {
        self.pending_template = None;
        self.pending_dir = None;
        self.pending_path = None;
        self.pending_save_as = false;
        self.pending_setting = None;
        self.pending_shared = None;
    }

    fn open_switcher(&mut self, _: &OpenSwitcher, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_pending_inputs();
        self.open_panel(SwitcherMode::QuickSwitch, window, cx);
    }

    fn open_search(&mut self, _: &OpenSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_pending_inputs();
        self.open_panel(SwitcherMode::Search, window, cx);
    }

    /// Open search with a query already typed (used by tag chips).
    fn open_search_for(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_pending_inputs();
        self.show_settings = false;
        self.switcher = Some(Switcher {
            mode: SwitcherMode::Search,
            query: query.to_string(),
            results: self.compute_results(SwitcherMode::Search, query),
            selected: 0,
            property_key: None,
            capture_action: None,
        });
        window.focus(&self.switcher_focus);
        self.set_status(StatusKind::Info, format!("search: tag {query}"));
        cx.notify();
    }

    fn open_git(&mut self, _: &OpenGit, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_git();
        self.open_panel(SwitcherMode::GitMenu, window, cx);
    }

    fn open_commands(&mut self, _: &OpenCommands, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_pending_inputs();
        self.open_panel(SwitcherMode::CommandsMenu, window, cx);
    }

    fn open_settings_action(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_pending_inputs();
        self.switcher = None;
        self.show_settings = true;
        self.show_graph = false;
        self.show_query = false;
        self.show_diff = false;
        window.focus(&self.settings_focus);
        cx.notify();
    }

    fn save_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }

        let reading = self.editor.read(cx).is_reading();
        workspace::upsert(
            &mut self.workspaces,
            workspace::Workspace {
                name: name.to_string(),
                graph: self.show_graph,
                reading,
                note: self.active.clone(),
            },
        );

        self.set_status(
            StatusKind::Error,
            match workspace::save(&self.vault_root, &self.workspaces) {
                Ok(()) => format!("workspace saved: {name}"),
                Err(e) => format!("could not save workspace: {e}"),
            },
        );
        cx.notify();
    }

    fn apply_workspace(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self
            .workspaces
            .iter()
            .find(|workspace| workspace.name == name)
            .cloned()
        else {
            return;
        };

        self.show_query = false;
        if let Some(note) = &workspace.note {
            self.open_note(note, window, cx);
        }

        self.editor.update(cx, |editor, cx| {
            editor.set_reading_mode(workspace.reading);
            cx.notify();
        });

        if workspace.graph {
            self.open_graph(window, cx);
        } else {
            self.show_graph = false;
        }

        self.set_status(StatusKind::Info, format!("workspace: {name}"));
        cx.notify();
    }

    fn run_plugin_command(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some((plugin_index, command_id, title)) = self.plugin_commands.get(index).cloned()
        else {
            return;
        };
        let Some(plugin) = self.plugins.get(plugin_index) else {
            return;
        };

        let note = self.active.clone();
        let outcome = plugins::run_command(plugin, &command_id, &self.vault_root, note.as_deref());

        self.set_status(StatusKind::Error, {
            let mut applied: Vec<&'static str> = Vec::new();
            let message = match outcome {
                Ok(output) => {
                    let response = plugins::parse_response(&output);

                    if let Some(text) = response.insert {
                        self.editor.update(cx, |editor, cx| {
                            editor.insert_at_cursor(&text);
                            cx.notify();
                        });
                        applied.push("inserted");
                    }
                    if let Some(text) = response.append {
                        self.editor.update(cx, |editor, cx| {
                            editor.append_to_note(&text);
                            cx.notify();
                        });
                        applied.push("appended");
                    }

                    response.message.unwrap_or_else(|| {
                        let short: String = output.trim().chars().take(200).collect();
                        if short.is_empty() {
                            "(no output)".to_string()
                        } else {
                            short
                        }
                    })
                }
                Err(e) => format!("failed: {e}"),
            };

            let suffix = if applied.is_empty() {
                String::new()
            } else {
                format!(" [{}]", applied.join(", "))
            };
            format!("{title}: {message}{suffix}")
        });
        cx.notify();
    }

    fn open_graph(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.graph_view.clone() else {
            return;
        };

        let active = self.active.clone();
        let full = self.full_graph.clone();
        view.update(cx, |graph_view, cx| {
            match &active {
                Some(relative) => graph_view.show_local(&full, relative),
                None => graph_view.set_graph(full.clone()),
            }
            cx.notify();
        });

        self.show_graph = true;
        self.show_query = false;
        self.show_diff = false;
        window.focus(&view.focus_handle(cx));
        cx.notify();
    }

    /// Toggle the editorial relations panel, rebuilding it from disk each time it opens.
    fn open_relations(&mut self, _: &OpenRelations, _window: &mut Window, cx: &mut Context<Self>) {
        self.show_relations = !self.show_relations;
        if self.show_relations {
            self.relations = relations::build(&self.note_refs);
            self.set_status(StatusKind::Info, "document relations".to_string());
        }
        cx.notify();
    }

    /// A link graph that also includes frontmatter relationships.
    fn open_relation_graph(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.graph_view.clone() else {
            return;
        };
        let enriched = relations::graph(&self.note_refs);
        view.update(cx, |view, cx| {
            view.set_graph(enriched.clone());
            cx.notify();
        });
        self.show_graph = true;
        self.show_query = false;
        self.show_diff = false;
        let relations = enriched
            .edges
            .iter()
            .filter(|edge| edge.is_relation())
            .count();
        self.set_status(
            StatusKind::Info,
            format!(
                "relation graph: {} notes / {} relations",
                enriched.nodes.len(),
                relations
            ),
        );
        window.focus(&view.focus_handle(cx));
        cx.notify();
    }

    fn save_version(&mut self, label: &str, cx: &mut Context<Self>) {
        let Some(relative) = self.active.clone() else {
            self.set_status(
                StatusKind::Warn,
                "open a note before saving a version".to_string(),
            );
            cx.notify();
            return;
        };

        let content = self.editor.read(cx).text();
        self.set_status(
            StatusKind::Error,
            match versions::save(&self.vault_root, &relative, label, &content) {
                Ok(version) => format!(
                    "saved version {} ({} bytes)",
                    version.display_label(),
                    version.bytes
                ),
                Err(error) => format!("could not save version: {error}"),
            },
        );
        cx.notify();
    }

    fn compare_version(&mut self, left: Option<String>, right: String, cx: &mut Context<Self>) {
        let Some(relative) = self.active.clone() else {
            self.set_status(
                StatusKind::Warn,
                "open a note before comparing versions".to_string(),
            );
            cx.notify();
            return;
        };

        let compared = match &left {
            Some(left) => versions::compare(&self.vault_root, &relative, left, &right),
            None => versions::compare_with_note(&self.vault_root, &relative, &right),
        };

        let diff = match compared {
            Ok(diff) => diff,
            Err(error) => {
                self.set_status(
                    StatusKind::Error,
                    format!("could not compare versions: {error}"),
                );
                cx.notify();
                return;
            }
        };

        let label_of = |id: &str| {
            versions::list(&self.vault_root, &relative)
                .into_iter()
                .find(|version| version.id == id)
                .map(|version| version.display_label())
                .unwrap_or_else(|| id.to_string())
        };

        self.diff_title = match &left {
            Some(left) => format!("{} ↔ {}", label_of(left), label_of(&right)),
            None => format!("current ↔ {}", label_of(&right)),
        };
        self.diff_summary = diff.summary();
        self.diff_rows = Rc::new(diff.lines);
        self.show_diff = true;
        self.show_graph = false;
        self.show_query = false;
        self.set_status(
            StatusKind::Info,
            format!("version diff: {}", self.diff_summary),
        );
        cx.notify();
    }

    fn promote_version(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(relative) = self.active.clone() else {
            return;
        };

        // Promoting replaces the note on disk; never discard unsaved edits doing it.
        self.editor.update(cx, |editor, _cx| {
            if editor.is_dirty() {
                let _ = editor.save();
            }
        });

        let (kind, message) = match versions::promote(&self.vault_root, &relative, id) {
            Ok(_) => {
                self.editor.update(cx, |editor, cx| {
                    let _ = editor.reload();
                    cx.notify();
                });
                self.cache_epoch = self.cache_epoch.wrapping_add(1);
                self.refresh_properties(cx);
                if self.show_relations {
                    self.relations = relations::build(&self.note_refs);
                }
                (
                    StatusKind::Success,
                    format!("promoted version {id} to {relative}"),
                )
            }
            Err(error) => (
                StatusKind::Error,
                format!("could not promote version: {error}"),
            ),
        };
        self.set_status(kind, message);
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    fn delete_version(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(relative) = self.active.clone() else {
            return;
        };
        self.set_status(
            StatusKind::Error,
            match versions::remove(&self.vault_root, &relative, id) {
                Ok(()) => format!("deleted version {id}"),
                Err(error) => format!("could not delete version: {error}"),
            },
        );
        cx.notify();
    }

    fn insert_template(&mut self, relative: &str, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.vault_root.join(relative);
        let template = match std::fs::read_to_string(&path) {
            Ok(template) => template,
            Err(error) => {
                self.set_status(
                    StatusKind::Error,
                    format!("could not read template: {error}"),
                );
                cx.notify();
                return;
            }
        };

        let title = self
            .active
            .as_deref()
            .map(tab_title)
            .unwrap_or_else(|| "Untitled".to_string());
        let rendered = render_template(&template, &self.vault_root, self.active.as_deref(), &title);

        self.editor.update(cx, |editor, cx| {
            editor.insert_template(&rendered);
            cx.notify();
        });
        window.focus(&self.editor.focus_handle(cx));
        self.set_status(StatusKind::Info, format!("inserted template {relative}"));
        cx.notify();
    }

    fn create_from_template(
        &mut self,
        name: &str,
        template: &templates::TemplateFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = name.trim();
        if name.is_empty() {
            self.set_status(StatusKind::Error, "a note needs a name".to_string());
            cx.notify();
            return;
        }

        let folder = self
            .active
            .as_deref()
            .and_then(|active| active.rsplit_once('/'))
            .map(|(folder, _)| folder.to_string())
            .unwrap_or_default();
        let file_name = format!("{}.md", sanitize_filename(name));
        let relative = if folder.is_empty() {
            file_name
        } else {
            format!("{folder}/{file_name}")
        };
        let path = self.vault_root.join(&relative);
        if path.exists() {
            self.set_status(StatusKind::Error, format!("{relative} already exists"));
            cx.notify();
            return;
        }

        let template_text = match std::fs::read_to_string(&template.path) {
            Ok(text) => text,
            Err(error) => {
                self.set_status(
                    StatusKind::Error,
                    format!("could not read template: {error}"),
                );
                cx.notify();
                return;
            }
        };
        let rendered = render_template(&template_text, &self.vault_root, Some(&relative), name);

        if let Err(error) = storage::write_atomic(&path, &rendered) {
            self.set_status(StatusKind::Error, format!("could not create note: {error}"));
            cx.notify();
            return;
        }

        self.refresh_vault(cx);
        self.open_note(&relative, window, cx);
        self.set_status(
            StatusKind::Success,
            format!("created {relative} from {}", template.name),
        );
        cx.notify();
    }

    fn insert_shared(&mut self, slug: &str, window: &mut Window, cx: &mut Context<Self>) {
        let reference = format!("{{{{shared:{}}}}}", shared::sanitize(slug));
        self.editor.update(cx, |editor, cx| {
            editor.replace_selection_or_insert(&reference);
            cx.notify();
        });
        window.focus(&self.editor.focus_handle(cx));
        self.set_status(StatusKind::Info, format!("inserted shared block {slug}"));
        cx.notify();
    }

    fn extract_shared(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.clone();
        let text = editor
            .read(cx)
            .selection_text()
            .or_else(|| editor.read(cx).current_block_text());

        let Some(text) = text else {
            self.set_status(
                StatusKind::Warn,
                "select text or place the caret in a block first".to_string(),
            );
            cx.notify();
            return;
        };
        if text.trim().is_empty() {
            self.set_status(StatusKind::Warn, "nothing to extract".to_string());
            cx.notify();
            return;
        }

        self.pending_shared = Some(text);
        self.open_panel(SwitcherMode::SharedName, window, cx);
        self.set_status(StatusKind::Warn, "name the shared block".to_string());
    }

    fn review_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).text();
        self.review_issues = lint::review_with(&text, &self.note_index);
        if self.review_issues.is_empty() {
            self.set_status(StatusKind::Info, "review: no issues found".to_string());
            cx.notify();
            return;
        }

        self.set_status(
            StatusKind::Info,
            format!("review: {} issue(s)", self.review_issues.len()),
        );
        self.open_panel(SwitcherMode::ReviewMenu, window, cx);
    }

    fn add_relation(&mut self, target: &str, cx: &mut Context<Self>) {
        let target = target.trim();
        if target.is_empty() {
            return;
        }

        let mut properties = self.properties.clone();
        match properties
            .iter_mut()
            .find(|property| property.key == "related")
        {
            Some(property) => match &mut property.value {
                PropertyValue::List(items) => {
                    if !items.iter().any(|item| item == target) {
                        items.push(target.to_string());
                    }
                }
                PropertyValue::Scalar(value) => {
                    let existing = value.clone();
                    property.value = PropertyValue::List(vec![existing, target.to_string()]);
                }
                PropertyValue::Raw(_) => {}
            },
            None => properties.push(Property {
                key: "related".to_string(),
                value: PropertyValue::List(vec![target.to_string()]),
            }),
        }

        self.apply_properties(&properties, cx);
        self.relations = relations::build(&self.note_refs);
        self.set_status(StatusKind::Info, format!("related: {target}"));
    }

    /// The centre pane: the graph, the query table, a diff, or the editor (optionally with
    /// a preview pane beside it).
    fn center_view(
        &self,
        theme: Theme,
        preview: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.show_settings {
            return self.settings_view(theme, cx);
        }
        if self.show_diff {
            return diff_view(
                self.diff_title.clone(),
                self.diff_summary.clone(),
                self.diff_rows.clone(),
                theme,
            );
        }
        if self.show_query {
            return query_view(self.query_results.clone(), theme);
        }
        if self.show_graph {
            if let Some(view) = &self.graph_view {
                return view.clone().into_any_element();
            }
        }
        if let Some(error) = self
            .tabs
            .get(self.active_tab)
            .and_then(|tab| tab.load_error.clone())
        {
            return self.tab_error_view(error, theme, cx);
        }
        if !self.split_tabs.is_empty() {
            return match self.split_axis {
                SplitAxis::Horizontal => {
                    let mut row = div()
                        .flex()
                        .flex_row()
                        .size_full()
                        .child(div().flex_1().min_w(px(0.0)).child(self.editor.clone()));
                    for index in &self.split_tabs {
                        let index = *index;
                        if let Some(tab) = self.tabs.get(index) {
                            row = row
                                .child(div().w(px(1.0)).h_full().bg(theme.border).flex_shrink_0())
                                .child(
                                    div()
                                        .id(ElementId::Name(format!("split-pane-{index}").into()))
                                        .flex_1()
                                        .min_w(px(0.0))
                                        // Clicking a pane focuses it and makes it the
                                        // command target, so Ctrl+W and formatting apply
                                        // to the note the user is looking at.
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, _event, window, cx| {
                                                this.activate_tab(index, cx);
                                                let handle = this.editor.focus_handle(cx);
                                                window.focus(&handle);
                                            }),
                                        )
                                        .child(tab.editor.clone()),
                                );
                        }
                    }
                    row.into_any_element()
                }
                SplitAxis::Vertical => {
                    let mut column = div()
                        .flex()
                        .flex_col()
                        .size_full()
                        .child(div().flex_1().min_h(px(0.0)).child(self.editor.clone()));
                    for index in &self.split_tabs {
                        let index = *index;
                        if let Some(tab) = self.tabs.get(index) {
                            column = column
                                .child(div().h(px(1.0)).w_full().bg(theme.border).flex_shrink_0())
                                .child(
                                    div()
                                        .id(ElementId::Name(format!("split-pane-{index}").into()))
                                        .flex_1()
                                        .min_h(px(0.0))
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, _event, window, cx| {
                                                this.activate_tab(index, cx);
                                                let handle = this.editor.focus_handle(cx);
                                                window.focus(&handle);
                                            }),
                                        )
                                        .child(tab.editor.clone()),
                                );
                        }
                    }
                    column.into_any_element()
                }
            };
        }
        match preview {
            Some(preview) => div()
                .flex()
                .flex_row()
                .size_full()
                .child(div().flex_1().min_w(px(0.0)).child(self.editor.clone()))
                .child(preview)
                .into_any_element(),
            None => self.editor.clone().into_any_element(),
        }
    }

    /// A bar offered when the window is closed with unsaved notes.
    fn quit_prompt_bar(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let save = cx.entity();
        let discard = cx.entity();
        let cancel = cx.entity();

        let button = |label: &str,
                      color: gpui::Hsla,
                      id: &str,
                      entity: Entity<DragonGlassApp>,
                      action: fn(
            &mut DragonGlassApp,
            &mut Window,
            &mut Context<DragonGlassApp>,
        )| {
            div()
                .id(ElementId::Name(id.to_string().into()))
                .px_3()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .bg(color)
                .text_color(theme.accent_text)
                .hover(move |style| style.opacity(0.85))
                .child(label.to_string())
                .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                    entity.update(cx, |app, cx| action(app, window, cx));
                })
        };

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .bg(theme.panel_alt)
            .border_b_1()
            .border_color(theme.warn)
            .text_sm()
            .text_color(theme.text)
            .child(
                div()
                    .flex_1()
                    .text_color(theme.warn)
                    .child("This window has unsaved notes."),
            )
            .child(button(
                "Save and quit",
                theme.accent,
                "quit-save",
                save,
                Self::save_and_quit,
            ))
            .child(button(
                "Quit without saving",
                theme.chip,
                "quit-discard",
                discard,
                Self::discard_and_quit,
            ))
            .child(button(
                "Cancel",
                theme.chip,
                "quit-cancel",
                cancel,
                |app, _window, cx| app.cancel_quit(cx),
            ))
            .into_any_element()
    }

    /// Escape closes whatever is on top: the panel, then the query view, then the graph.
    fn close_overlay(&mut self, _: &CloseOverlay, window: &mut Window, cx: &mut Context<Self>) {
        if self.quit_prompt {
            self.cancel_quit(cx);
            return;
        }

        if self.zen {
            self.zen = false;
            cx.notify();
            return;
        }

        // Menus and popups close before the panels they belong to.
        if self.tab_menu.is_some() {
            self.tab_menu = None;
            cx.notify();
            return;
        }

        if self.tree_menu.is_some() {
            self.tree_menu = None;
            cx.notify();
            return;
        }

        if self.switcher.is_some() {
            self.close_panel(window, cx);
            return;
        }

        if self.show_settings {
            self.show_settings = false;
            window.focus(&self.editor.focus_handle(cx));
            cx.notify();
            return;
        }

        if self.show_diff {
            self.show_diff = false;
            window.focus(&self.editor.focus_handle(cx));
            cx.notify();
            return;
        }

        if self.show_query {
            self.show_query = false;
            window.focus(&self.editor.focus_handle(cx));
            cx.notify();
            return;
        }

        if self.show_graph {
            self.show_graph = false;
            window.focus(&self.editor.focus_handle(cx));
            cx.notify();
            return;
        }

        if self.show_relations {
            self.show_relations = false;
            window.focus(&self.editor.focus_handle(cx));
            cx.notify();
            return;
        }

        self.editor.update(cx, |editor, cx| {
            if editor.handle_escape(cx) {
                cx.notify();
            }
        });

        // Nothing else wanted the key: dismiss the stale status line, if there is one.
        if self.status.is_some() {
            self.clear_status();
            cx.notify();
        }
    }

    /// The pane shown instead of a blank editor when a restored tab could not be read.
    fn tab_error_view(&self, error: String, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let index = self.active_tab;
        let title = self
            .tabs
            .get(index)
            .map(|tab| tab.title.clone())
            .unwrap_or_default();
        let path = self
            .tabs
            .get(index)
            .and_then(|tab| tab.relative.clone())
            .unwrap_or_default();
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px_3()
                .py_1()
                .rounded_md()
                .bg(theme.chip)
                .text_sm()
                .text_color(theme.text)
                .cursor_pointer()
                .hover(move |style| style.bg(theme.hover))
                .child(label)
        };

        div()
            .id("tab-error")
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                div()
                    .text_color(theme.error)
                    .child(format!("{title} could not be opened")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(format!("{path} — {error}")),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(button("tab-retry", "Retry").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _event, window, cx| {
                            this.retry_tab(index, window, cx);
                        }),
                    ))
                    .child(button("tab-close", "Close tab").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _event, window, cx| {
                            this.close_tab(index, window, cx);
                        }),
                    )),
            )
            .into_any_element()
    }

    /// Try a failed tab again without closing and reopening it.
    fn retry_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(relative) = self.tabs.get(index).and_then(|tab| tab.relative.clone()) else {
            return;
        };
        let editor = self.tabs[index].editor.clone();
        if self.load_into(&editor, &relative, cx) {
            if let Some(tab) = self.tabs.get_mut(index) {
                tab.load_error = None;
            }
            if index == self.active_tab {
                self.refresh_properties(cx);
                self.cache_epoch = self.cache_epoch.wrapping_add(1);
            }
            self.set_status(StatusKind::Success, format!("opened {relative}"));
            window.focus(&self.editor.focus_handle(cx));
        } else {
            if let Some(tab) = self.tabs.get_mut(index) {
                tab.load_error = Some("still could not read the file".to_string());
            }
            self.set_status(StatusKind::Error, format!("could not open {relative}"));
        }
        cx.notify();
    }

    fn new_window(&mut self, _: &NewWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.open_new_window(window, cx);
    }

    /// Copy from the reading view even when focus sits on a panel (sidebar, outline, tabs).
    fn copy_selection(&mut self, _: &editor::Copy, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.editor.read(cx).is_reading() {
            return;
        }
        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| editor.copy_selection(cx));
    }

    /// Mark the active tab as changed on disk and show the resolution bar.
    fn mark_conflict(&mut self, cx: &mut Context<Self>) {
        let index = self.active_tab;
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.conflict = true;
        }
        self.conflict = true;
        cx.notify();
    }

    /// Resolve a conflict by dropping the buffer and taking the disk version.
    fn reload_conflicted(
        &mut self,
        _: &ReloadConflicted,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.editor.clone();
        let result = editor.update(cx, |editor, cx| {
            let result = editor.reload();
            cx.notify();
            result
        });
        match result {
            Ok(()) => {
                let index = self.active_tab;
                if let Some(tab) = self.tabs.get_mut(index) {
                    tab.conflict = false;
                }
                self.conflict = false;
                self.set_status(StatusKind::Info, "reloaded from disk");
                window.focus(&self.editor.focus_handle(cx));
            }
            Err(error) => {
                self.set_status(StatusKind::Error, format!("could not reload: {error}"));
            }
        }
        cx.notify();
    }

    /// Resolve a conflict by overwriting the disk version with the buffer.
    fn keep_mine(&mut self, _: &KeepMine, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.clone();
        let report = editor.update(cx, |editor, _cx| editor.save_forced());
        match report {
            SaveReport::Saved { .. } => {
                let index = self.active_tab;
                if let Some(tab) = self.tabs.get_mut(index) {
                    tab.conflict = false;
                }
                self.conflict = false;
                self.set_status(StatusKind::Success, "kept your version");
                window.focus(&self.editor.focus_handle(cx));
            }
            SaveReport::Conflict { .. } => {
                self.set_status(StatusKind::Error, "the note changed again while saving");
            }
            SaveReport::Error { message } => {
                self.set_status(StatusKind::Error, format!("could not save: {message}"));
            }
        }
        cx.notify();
    }

    /// Show the disk version next to the buffer, using the existing line diff.
    fn compare_conflicted(
        &mut self,
        _: &CompareConflicted,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(relative) = self.active.clone() else {
            return;
        };
        let disk = std::fs::read_to_string(self.vault_root.join(&relative)).unwrap_or_default();
        let mine = self.editor.read(cx).text();
        let diff = diff::lines(&disk, &mine);
        self.diff_title = format!("disk ↔ mine — {relative} (unsaved)");
        self.diff_summary = diff.summary();
        self.diff_rows = Rc::new(diff.lines);
        self.show_diff = true;
        self.set_status(StatusKind::Warn, format!("compare: {}", self.diff_summary));
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    /// Resolve a conflict by writing the buffer to a new file.
    fn save_conflict_copy(
        &mut self,
        _: &SaveConflictCopy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_save_as(window, cx);
    }

    /// Look for a draft that differs from the note on disk and offer to recover it.
    fn check_recovery(&mut self, cx: &mut Context<Self>) {
        self.pending_draft = None;
        let Some(relative) = self.active.clone() else {
            return;
        };
        let Some(draft) = drafts::load(&self.vault_root, &relative) else {
            return;
        };
        let disk = std::fs::read_to_string(self.vault_root.join(&relative)).unwrap_or_default();
        if draft.text == disk {
            // Nothing was lost: the note already holds the drafted text.
            drafts::discard(&self.vault_root, &relative);
            return;
        }
        self.pending_draft = Some(draft);
        cx.notify();
    }

    fn recover_draft(&mut self, _: &RecoverDraft, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.pending_draft.take() else {
            return;
        };
        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| {
            editor.apply_recovery(&draft.text, draft.line, draft.col);
            cx.notify();
        });
        drafts::discard(&self.vault_root, &draft.note);
        self.set_status(
            StatusKind::Info,
            "recovered the draft — save the note to keep it",
        );
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    fn compare_draft(&mut self, _: &CompareDraft, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.pending_draft.as_ref() else {
            return;
        };
        let Some(relative) = self.active.clone() else {
            return;
        };
        let disk = std::fs::read_to_string(self.vault_root.join(&relative)).unwrap_or_default();
        let diff = diff::lines(&disk, &draft.text);
        self.diff_title = format!("disk ↔ draft — {relative}");
        self.diff_summary = diff.summary();
        self.diff_rows = Rc::new(diff.lines);
        self.show_diff = true;
        self.set_status(
            StatusKind::Info,
            format!("draft diff: {}", self.diff_summary),
        );
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    fn discard_draft(&mut self, _: &DiscardDraft, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(draft) = self.pending_draft.take() {
            drafts::discard(&self.vault_root, &draft.note);
            self.set_status(StatusKind::Info, "draft discarded");
        }
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    /// Cmd+Q: run the same Save / Discard / Cancel gate as closing the window, then quit.
    fn quit_action(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        if self.request_close(window, cx) {
            cx.quit();
        } else {
            self.quit_after_prompt = true;
        }
    }

    fn close_tab_action(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.switcher.is_none() {
            self.close_tab(self.active_tab, window, cx);
        }
    }

    /// Bring back the most recently closed note, with its mode and cursor line.
    fn reopen_closed_tab_action(
        &mut self,
        _: &ReopenClosedTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.closed_tabs.pop() else {
            self.set_status(StatusKind::Info, "no recently closed note");
            cx.notify();
            return;
        };
        let Some(relative) = entry.relative.clone() else {
            self.set_status(
                StatusKind::Warn,
                "the closed tab was never saved, so there is no file to reopen",
            );
            cx.notify();
            return;
        };

        self.open_note(&relative, window, cx);
        let editor = self.editor.clone();
        editor.update(cx, |editor, cx| {
            editor.set_reading_mode(entry.reading);
            editor.set_cursor_line(entry.line);
            cx.notify();
        });
        self.set_status(StatusKind::Success, format!("reopened {}", entry.title));
        cx.notify();
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.switcher.is_none() {
            self.cycle_tab(1, window, cx);
        }
    }

    fn prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.switcher.is_none() {
            self.cycle_tab(-1, window, cx);
        }
    }

    /// Open another window on the same vault, with its own editor and panes.
    fn open_new_window(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let editor = cx.new(Editor::new);
        let notes = (*self.note_suggestions).clone();
        let tags = (*self.tag_suggestions).clone();
        editor.update(cx, |editor, _cx| {
            editor.set_vault_root(&self.vault_root);
            editor.set_attachments_folder(&self.config.attachments_folder);
            editor.set_vim_enabled(self.config.vim_mode);
            editor.set_font_size(px(self.config.font_size as f32));
            editor.set_suggestions(notes, tags);
            if let Some(active) = &self.active {
                let _ = editor.load(&self.vault_root.join(active));
            }
        });

        let sibling_tab = Tab {
            relative: self.active.clone(),
            editor,
            title: self
                .active
                .as_deref()
                .map(tab_title)
                .unwrap_or_else(|| "Untitled".to_string()),
            conflict: false,
            load_error: None,
            pending_load: false,
        };
        self.spawn_window_with_tabs(vec![sibling_tab], 0, false, cx);
    }

    /// Remove a tab from this window, keeping its editor alive, and repair the active and
    /// secondary indices. Used both by tear-out and by cross-window drag and drop.
    fn take_tab(&mut self, index: usize, cx: &mut Context<Self>) -> Option<Tab> {
        if index >= self.tabs.len() {
            return None;
        }

        self.dragging = None;
        let was_last = self.tabs.len() == 1;
        let old_active = self.active_tab;
        let tab = self.tabs.remove(index);
        self.reindex_splits_after_remove(index);
        if was_last && self.minimal {
            self.close_requested = true;
        }

        if self.tabs.is_empty() {
            if self.minimal {
                // A popped-out window giving away its last tab has nothing left to show;
                // it closes on its next frame instead of opening an empty document.
                self.active_tab = 0;
                self.split_tabs.clear();
                self.close_requested = true;
                return Some(tab);
            }
            let editor = self.new_editor(cx);
            self.tabs.push(Tab {
                relative: None,
                editor,
                title: "Untitled".to_string(),
                conflict: false,
                load_error: None,
                pending_load: false,
            });
            self.active_tab = 0;
            self.split_tabs.clear();
        } else {
            self.active_tab = if old_active == index {
                index.min(self.tabs.len() - 1)
            } else if old_active > index {
                old_active - 1
            } else {
                old_active
            };
        }

        self.activate_tab(self.active_tab, cx);
        Some(tab)
    }

    /// Move one tab into a real OS window, preserving its editor and unsaved state.
    fn detach_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.take_tab(index, cx) else {
            return;
        };
        let title = tab.title.clone();
        window.focus(&self.editor.focus_handle(cx));
        self.set_status(StatusKind::Info, format!("moved {title} to its own window"));
        cx.notify();
        self.spawn_window_with_tabs(vec![tab], 0, true, cx);
    }

    /// Remove every tab without creating a replacement, used when a popped-out window
    /// hands its contents back to its parent.
    fn take_all_tabs(&mut self) -> Vec<Tab> {
        self.dragging = None;
        self.split_tabs.clear();
        self.active_tab = 0;
        std::mem::take(&mut self.tabs)
    }

    /// Append tabs moved in from another window and focus the first of them.
    fn adopt_tabs(&mut self, tabs: Vec<Tab>, cx: &mut Context<Self>) {
        if tabs.is_empty() {
            return;
        }

        let first = self.tabs.len();
        self.tabs.extend(tabs);
        self.activate_tab(first, cx);
        self.set_status(
            StatusKind::Info,
            format!("returned {}", self.tabs[first].title),
        );
        cx.notify();
    }

    /// Move a dragged tab into this window and place it according to where it was dropped.
    fn accept_tab(
        &mut self,
        drag: &TabDrag,
        drop: TabDrop,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dragging = None;
        let same_window = drag.source.entity_id() == cx.entity().entity_id();
        let target = if same_window {
            if drag.index >= self.tabs.len() {
                return;
            }
            drag.index
        } else {
            let taken = drag
                .source
                .update(cx, |source, cx| source.take_tab(drag.index, cx));
            let Ok(Some(tab)) = taken else {
                return;
            };
            self.tabs.push(tab);
            self.tabs.len() - 1
        };

        match drop {
            TabDrop::Merge => {
                self.activate_tab(target, cx);
                self.set_status(StatusKind::Info, self.tabs[target].title.clone());
            }
            TabDrop::Split(side) => {
                let axis = match side {
                    SplitSide::Left | SplitSide::Right => SplitAxis::Horizontal,
                    SplitSide::Above | SplitSide::Below => SplitAxis::Vertical,
                };

                if target == self.active_tab {
                    if self.split_tabs.is_empty() {
                        return;
                    }
                    self.split_axis = axis;
                } else {
                    if !self.split_tabs.contains(&target) && self.split_tabs.len() >= 3 {
                        self.set_status(StatusKind::Warn, "at most four editor panes".to_string());
                        return;
                    }

                    self.split_axis = axis;
                    match side {
                        SplitSide::Left | SplitSide::Above => {
                            let previous = self.active_tab;
                            self.activate_tab(target, cx);
                            self.split_tabs.retain(|index| *index != target);
                            if previous != target && !self.split_tabs.contains(&previous) {
                                self.split_tabs.insert(0, previous);
                            }
                        }
                        SplitSide::Right | SplitSide::Below => {
                            if !self.split_tabs.contains(&target) {
                                self.split_tabs.push(target);
                            }
                        }
                    }
                }

                self.split_preview = false;
                let axis_name = match self.split_axis {
                    SplitAxis::Horizontal => "horizontal",
                    SplitAxis::Vertical => "vertical",
                };
                self.set_status(
                    StatusKind::Info,
                    format!("{axis_name} split: {} panes", self.split_tabs.len() + 1),
                );
            }
            TabDrop::Detach => {
                if same_window {
                    self.detach_tab(target, window, cx);
                    return;
                }
                self.activate_tab(target, cx);
                self.set_status(StatusKind::Info, self.tabs[target].title.clone());
            }
        }

        self.normalize_splits();
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    fn drop_on_tab_bar(&mut self, drag: &TabDrag, window: &mut Window, cx: &mut Context<Self>) {
        self.accept_tab(drag, TabDrop::Merge, window, cx);
    }

    fn drop_on_window(&mut self, drag: &TabDrag, window: &mut Window, cx: &mut Context<Self>) {
        let same_window = drag.source.entity_id() == cx.entity().entity_id();
        self.accept_tab(
            drag,
            if same_window {
                TabDrop::Detach
            } else {
                TabDrop::Merge
            },
            window,
            cx,
        );
    }

    /// While a tab is dragged, the editor area exposes four edges: left/right split the
    /// editors horizontally, top/bottom stack them vertically.
    fn tab_drop_overlay(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let edge = px(84.0);
        let edge_tint = theme.accent.opacity(0.16);
        let center_tint = theme.accent.opacity(0.06);
        let center_side = match self.split_axis {
            SplitAxis::Horizontal => SplitSide::Right,
            SplitAxis::Vertical => SplitSide::Below,
        };

        div()
            .id("tab-drop-overlay")
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(edge)
                    .w_full()
                    .bg(edge_tint)
                    .drag_over::<TabDrag>(move |style, _drag, _window, _cx| {
                        style.bg(theme.accent.opacity(0.35))
                    })
                    .on_drop::<TabDrag>(cx.listener(|this, drag: &TabDrag, window, cx| {
                        this.accept_tab(drag, TabDrop::Split(SplitSide::Above), window, cx);
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .child(
                        div()
                            .w(edge)
                            .h_full()
                            .bg(edge_tint)
                            .drag_over::<TabDrag>(move |style, _drag, _window, _cx| {
                                style.bg(theme.accent.opacity(0.35))
                            })
                            .on_drop::<TabDrag>(cx.listener(|this, drag: &TabDrag, window, cx| {
                                this.accept_tab(drag, TabDrop::Split(SplitSide::Left), window, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .bg(center_tint)
                            .drag_over::<TabDrag>(move |style, _drag, _window, _cx| {
                                style.bg(theme.accent.opacity(0.3))
                            })
                            .on_drop::<TabDrag>(cx.listener(
                                move |this, drag: &TabDrag, window, cx| {
                                    this.accept_tab(drag, TabDrop::Split(center_side), window, cx);
                                },
                            )),
                    )
                    .child(
                        div()
                            .w(edge)
                            .h_full()
                            .bg(edge_tint)
                            .drag_over::<TabDrag>(move |style, _drag, _window, _cx| {
                                style.bg(theme.accent.opacity(0.35))
                            })
                            .on_drop::<TabDrag>(cx.listener(|this, drag: &TabDrag, window, cx| {
                                this.accept_tab(drag, TabDrop::Split(SplitSide::Right), window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .h(edge)
                    .w_full()
                    .bg(edge_tint)
                    .drag_over::<TabDrag>(move |style, _drag, _window, _cx| {
                        style.bg(theme.accent.opacity(0.35))
                    })
                    .on_drop::<TabDrag>(cx.listener(|this, drag: &TabDrag, window, cx| {
                        this.accept_tab(drag, TabDrop::Split(SplitSide::Below), window, cx);
                    })),
            )
            .into_any_element()
    }

    /// A tab dragged outside every drop zone pops out into its own window.
    fn on_drag_release_out(
        &mut self,
        _: &gpui::MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resize = None;
        if let Some(drag) = self.dragging.take() {
            if cx.has_active_drag() {
                cx.stop_active_drag(window);
                self.detach_tab(drag.index, window, cx);
            }
        }
    }

    /// If a drag ended outside the window, the platform may not deliver the mouse-up.
    /// The next button-less mouse movement releases the stale drag so it cannot keep
    /// covering the editor.
    fn clear_stuck_drag_move(
        &mut self,
        event: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.resize.is_some() {
            if event.pressed_button.is_none() {
                self.resize = None;
                self.set_editors_resizing(false, cx);
                cx.notify();
            } else {
                self.apply_resize(event, cx);
            }
        }

        if event.pressed_button.is_none() && cx.has_active_drag() {
            cx.stop_active_drag(window);
            self.dragging = None;
            cx.notify();
        }

        // A tab dragged in from another window is a global active drag; repaint so this
        // window's drop zones light up and its hit-testing stays current.
        if cx.has_active_drag() && self.dragging.is_none() {
            cx.notify();
        }
    }

    fn clear_stuck_drag_down(
        &mut self,
        _: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cx.has_active_drag() {
            cx.stop_active_drag(window);
            self.dragging = None;
            cx.notify();
        }
    }

    /// Build a new Dragon Glass window around the supplied tabs, sharing this app's vault
    /// configuration, theme and workspace list.
    fn spawn_window_with_tabs(
        &mut self,
        tabs: Vec<Tab>,
        active_tab: usize,
        minimal: bool,
        cx: &mut Context<Self>,
    ) {
        let active_tab = active_tab.min(tabs.len().saturating_sub(1));
        let Some(active_tab_ref) = tabs.get(active_tab) else {
            return;
        };
        let editor = active_tab_ref.editor.clone();
        let active = active_tab_ref.relative.clone();
        let title = match (minimal, active.as_deref().map(tab_title)) {
            (true, Some(title)) => title,
            (false, Some(title)) => format!("Dragon Glass — {title}"),
            (_, None) => "Dragon Glass".to_string(),
        };

        let parent_for_close = cx.entity().downgrade();
        let sibling = cx.new(|cx| {
            let app = DragonGlassApp {
                editor: editor.clone(),
                tabs,
                closed_tabs: Vec::new(),
                zen: false,
                nav_history: Vec::new(),
                nav_index: 0,
                active_tab,
                split_tabs: Vec::new(),
                split_axis: SplitAxis::Horizontal,
                minimal,
                close_requested: false,
                dragging: None,
                shown_title: title.clone(),
                cache_epoch: 1,
                vault_root: self.vault_root.clone(),
                tree: self.tree.clone(),
                file_rows: self.file_rows.clone(),
                collapsed_dirs: self.collapsed_dirs.clone(),
                collapsed_outline: HashSet::new(),
                note_refs: self.note_refs.clone(),
                note_index: self.note_index.clone(),
                note_suggestions: self.note_suggestions.clone(),
                tag_suggestions: self.tag_suggestions.clone(),
                properties: Vec::new(),
                headings: Rc::new(Vec::new()),
                headings_version: (u64::MAX, u64::MAX),
                preview_content: ReadingCache::new(),
                bookmarks: self.bookmarks.clone(),
                theme: self.theme,
                active,
                vault_path: self.vault_path.clone(),
                notes: self.notes,
                links: self.links,
                git_status: self.git_status.clone(),
                status: None,
                pending_draft: None,
                derived_at: None,
                derived_armed: false,
                preview_at: None,
                preview_armed: false,
                preview_epoch: u64::MAX,
                status_timer_armed: false,
                switcher: None,
                switcher_focus: cx.focus_handle(),
                focus_handle: cx.focus_handle(),
                quit_prompt: false,
                quit_after_prompt: false,
                // Popped-out windows hand their tabs back to the parent on close; the parent
                // owns the save prompt, so this window can close immediately.
                allow_close: true,
                show_settings: false,
                pending_setting: None,
                settings_focus: cx.focus_handle(),
                tree_menu: None,
                pending_dir: None,
                pending_path: None,
                pending_save_as: false,
                tab_menu: None,
                scroll_sync_generation: 0,
                last_editor_progress: -1.0,
                last_preview_progress: -1.0,
                outline_scroll: UniformListScrollHandle::new(),
                outline_current: None,
                conflict: false,
                watcher: None,
                update_source: self.update_source.clone(),
                update_available: self.update_available.clone(),
                full_graph: self.full_graph.clone(),
                graph_view: None,
                show_graph: false,
                relations: self.relations.clone(),
                show_relations: self.show_relations && !minimal,
                sidebar_width: self.sidebar_width,
                outline_width: self.outline_width,
                relations_width: self.relations_width,
                resize: None,
                show_diff: false,
                diff_title: String::new(),
                diff_summary: String::new(),
                diff_rows: Rc::new(Vec::new()),
                review_issues: Vec::new(),
                pending_shared: None,
                pending_template: None,
                stats: self.stats,
                stats_version: (u64::MAX, u64::MAX),
                query_results: Rc::new(Vec::new()),
                show_query: false,
                split_preview: self.split_preview && !minimal,
                plugins: self.plugins.clone(),
                plugin_commands: self.plugin_commands.clone(),
                workspaces: self.workspaces.clone(),
                user_themes: self.user_themes.clone(),
                bindings: self.bindings.clone(),
                base_theme: self.base_theme,
                snippet_overrides: self.snippet_overrides.clone(),
                config: self.config.clone(),
            };
            app.install_quit_saver(cx);
            app
        });

        let graph_view = cx.new(|cx| GraphView::new(sibling.downgrade(), cx));
        sibling.update(cx, |app, cx| {
            app.graph_view = Some(graph_view);
            app.watch_vault(cx);
            // Editors moved into this window must ask this window for Save As, not the parent.
            app.configure_editors(cx);
        });

        let bounds = Bounds::centered(None, size(px(1000.0), px(640.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some(title.into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let view = sibling.clone();
        let returns = sibling.downgrade();
        let _ = cx.open_window(options, move |window, cx| {
            // Closing a popped-out window returns its tabs to the window it came from, so
            // closing the window is the merge gesture: no separate button is needed.
            let child = returns.clone();
            let target = parent_for_close.clone();
            window.on_window_should_close(cx, move |_window, cx| {
                let mut tabs = Vec::new();
                if let Some(child) = child.upgrade() {
                    tabs = child.update(cx, |app, _cx| app.take_all_tabs());
                }
                if !tabs.is_empty() {
                    if let Some(target) = target.upgrade() {
                        target.update(cx, |app, cx| app.adopt_tabs(tabs, cx));
                    }
                }
                true
            });
            view.clone()
        });
    }

    /// Watch the vault for external edits. Event-driven: the watcher thread blocks, so idle
    /// CPU stays at zero. Each window gets its own watcher so detached tabs still reconcile.
    fn watch_vault(&mut self, cx: &mut Context<Self>) {
        let (change_tx, change_rx) = smol::channel::unbounded::<String>();
        let watcher =
            match notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event {
                    for path in event.paths {
                        if path
                            .extension()
                            .map(|extension| extension.eq_ignore_ascii_case("md"))
                            .unwrap_or(false)
                        {
                            let _ = change_tx.send_blocking(path.to_string_lossy().to_string());
                        }
                    }
                }
            }) {
                Ok(mut watcher) => {
                    use notify::Watcher as _;
                    let _ = watcher.watch(&self.vault_root, notify::RecursiveMode::Recursive);
                    Some(watcher)
                }
                Err(_) => None,
            };

        self.watcher = watcher;
        cx.spawn(async move |this, cx| {
            while let Ok(changed) = change_rx.recv().await {
                let alive = this
                    .update(cx, |app, cx| app.on_external_change(&changed, cx))
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    fn detach_tab_action(&mut self, _: &DetachTab, window: &mut Window, cx: &mut Context<Self>) {
        self.detach_tab(self.active_tab, window, cx);
    }

    fn toggle_split_editor(&mut self, cx: &mut Context<Self>) {
        if !self.split_tabs.is_empty() {
            self.split_tabs.clear();
            self.set_status(StatusKind::Info, "single editor".to_string());
        } else if self.tabs.len() < 2 {
            self.set_status(
                StatusKind::Warn,
                "open another note to use split editing".to_string(),
            );
        } else {
            let next = (self.active_tab + 1) % self.tabs.len();
            self.split_tabs.push(next);
            self.split_axis = SplitAxis::Horizontal;
            self.split_preview = false;
            self.set_status(
                StatusKind::Info,
                format!("two editors: {}", self.tabs[next].title),
            );
        }
        cx.notify();
    }

    fn toggle_split_preview(&mut self, cx: &mut Context<Self>) {
        self.split_preview = !self.split_preview;
        if self.split_preview && self.config.scroll_sync {
            self.start_scroll_sync(cx);
        }
        if self.split_preview {
            self.split_tabs.clear();
            self.editor.update(cx, |editor, cx| {
                if editor.is_reading() {
                    editor.set_reading_mode(false);
                    cx.notify();
                }
            });
        }
        self.set_status(
            StatusKind::Info,
            format!(
                "preview pane: {}",
                if self.split_preview { "on" } else { "off" }
            ),
        );
        cx.notify();
    }

    fn toggle_split_editor_action(
        &mut self,
        _: &ToggleSplitEditor,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_split_editor(cx);
    }

    fn toggle_side_by_side_action(
        &mut self,
        _: &ToggleSideBySide,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_split_preview(cx);
    }

    /// The theme with snippets layered on, if enabled.
    fn effective_theme(&self) -> Theme {
        let mut theme = if self.config.snippets {
            self.base_theme.with_overrides(&self.snippet_overrides)
        } else {
            self.base_theme
        };
        if let Some(accent) = self
            .config
            .accent
            .as_deref()
            .and_then(crate::theme::parse_hex)
        {
            theme = theme.with_accent(accent);
        }
        theme
    }

    /// Recompute the effective theme and push it to every view.
    fn refresh_theme(&mut self, cx: &mut Context<Self>) {
        let theme = self.effective_theme();
        self.theme = theme;
        cx.set_global(theme);
        // Token colours are baked into the shaped rows, so every open editor must
        // re-shape before the next frame.
        for tab in &self.tabs {
            tab.editor
                .update(cx, |editor, _| editor.invalidate_layout());
        }
        self.editor.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn toggle_snippets(&mut self, cx: &mut Context<Self>) {
        self.config.snippets = !self.config.snippets;
        let message = match config::save(&self.vault_root, &self.config) {
            Ok(()) => format!(
                "snippets: {}",
                if self.config.snippets { "on" } else { "off" }
            ),
            Err(e) => format!("snippets toggled (not saved: {e})"),
        };
        self.refresh_theme(cx);
        self.set_status(StatusKind::Info, message);
        cx.notify();
    }

    /// Apply the reading typography after the config changed, and repaint every reader.
    fn refresh_reader_settings(&mut self, cx: &mut Context<Self>) {
        cx.set_global(ReaderSettings {
            body_size: self.config.reader_size as f32,
            measure: self.config.reader_width as f32,
            line_height: self.config.reader_line_height as f32 / 100.0,
            font: ReaderFont::from_name(&self.config.reader_font),
        });
        let editors: Vec<Entity<Editor>> = self.tabs.iter().map(|tab| tab.editor.clone()).collect();
        for editor in editors {
            editor.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }

    /// Cycle the reading body size / measure through a short list of comfortable values.
    fn cycle_reader_setting(&mut self, width: bool, cx: &mut Context<Self>) {
        if width {
            const STEPS: [u16; 5] = [640, 720, 780, 900, 1040];
            let next = STEPS
                .iter()
                .position(|step| *step == self.config.reader_width)
                .map(|index| STEPS[(index + 1) % STEPS.len()])
                .unwrap_or(STEPS[0]);
            self.config.reader_width = next;
        } else {
            const STEPS: [u16; 8] = [13, 14, 15, 16, 17, 18, 20, 22];
            let next = STEPS
                .iter()
                .position(|step| *step == self.config.reader_size)
                .map(|index| STEPS[(index + 1) % STEPS.len()])
                .unwrap_or(STEPS[2]);
            self.config.reader_size = next;
        }
        let (kind, message) = match config::save(&self.vault_root, &self.config) {
            Ok(()) => (
                StatusKind::Info,
                if width {
                    format!("reading width: {} px", self.config.reader_width)
                } else {
                    format!("reading text: {} px", self.config.reader_size)
                },
            ),
            Err(error) => (
                StatusKind::Warn,
                format!("reader setting changed (not saved: {error})"),
            ),
        };
        self.refresh_reader_settings(cx);
        self.set_status(kind, message);
        cx.notify();
    }

    /// Follow the editor's scroll position in the split preview, or stop following.
    fn toggle_scroll_sync(&mut self, cx: &mut Context<Self>) {
        self.config.scroll_sync = !self.config.scroll_sync;
        let message = match config::save(&self.vault_root, &self.config) {
            Ok(()) => format!(
                "preview scroll sync: {}",
                if self.config.scroll_sync { "on" } else { "off" }
            ),
            Err(error) => format!("scroll sync changed (not saved: {error})"),
        };
        if self.config.scroll_sync && self.split_preview {
            self.start_scroll_sync(cx);
        } else if !self.config.scroll_sync {
            // Bump the generation so the running loop stops on its next tick.
            self.scroll_sync_generation = self.scroll_sync_generation.wrapping_add(1);
        }
        self.set_status(StatusKind::Info, message);
        cx.notify();
    }

    /// Keep the split preview and the editor at the same place while it is open.
    ///
    /// Two-way: whichever side moved last drives the other. The loop stops as soon as the
    /// setting is turned off or the preview closes, so idle CPU stays at zero otherwise.
    fn start_scroll_sync(&mut self, cx: &mut Context<Self>) {
        self.scroll_sync_generation = self.scroll_sync_generation.wrapping_add(1);
        let generation = self.scroll_sync_generation;
        self.last_editor_progress = -1.0;
        self.last_preview_progress = -1.0;
        cx.spawn(async move |this, cx| loop {
            Timer::after(std::time::Duration::from_millis(150)).await;
            let keep = this
                .update(cx, |app, cx| {
                    if !app.config.scroll_sync
                        || app.scroll_sync_generation != generation
                        || !app.split_preview
                    {
                        return false;
                    }
                    let editor_progress = app.editor.read(cx).scroll_progress();
                    let preview_progress = app.preview_content.progress();
                    let editor_moved = (editor_progress - app.last_editor_progress).abs() >= 0.005;
                    let preview_moved =
                        (preview_progress - app.last_preview_progress).abs() >= 0.005;
                    if editor_moved {
                        // The editor leads: scroll the preview, then record where the
                        // block-based preview actually landed so it does not fight back.
                        app.preview_content.scroll_to_progress(editor_progress);
                        app.last_editor_progress = editor_progress;
                        app.last_preview_progress = app.preview_content.progress();
                        cx.notify();
                    } else if preview_moved {
                        // The preview leads: mirror its fraction in the editor.
                        app.last_preview_progress = preview_progress;
                        app.last_editor_progress = preview_progress;
                        let editor = app.editor.clone();
                        editor.update(cx, |editor, cx| {
                            editor.set_scroll_progress(preview_progress);
                            cx.notify();
                        });
                    }
                    true
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        })
        .detach();
    }

    /// Show or hide a side panel, persisting the choice with the vault.
    fn toggle_panel(&mut self, panel: &str, cx: &mut Context<Self>) {
        match panel {
            "sidebar" => self.config.show_sidebar = !self.config.show_sidebar,
            "outline" => self.config.show_outline = !self.config.show_outline,
            "relations" => self.config.show_relations = !self.config.show_relations,
            _ => return,
        }
        let on = match panel {
            "sidebar" => self.config.show_sidebar,
            "outline" => self.config.show_outline,
            _ => self.config.show_relations,
        };
        let message = match config::save(&self.vault_root, &self.config) {
            Ok(()) => format!("{panel}: {}", if on { "shown" } else { "hidden" }),
            Err(error) => format!("{panel} changed (not saved: {error})"),
        };
        self.set_status(StatusKind::Info, message);
        cx.notify();
    }

    /// Cycle the reading line height through comfortable values.
    fn cycle_reader_line_height(&mut self, cx: &mut Context<Self>) {
        const STEPS: [u16; 6] = [130, 140, 150, 160, 175, 190];
        let next = STEPS
            .iter()
            .position(|step| *step == self.config.reader_line_height)
            .map(|index| STEPS[(index + 1) % STEPS.len()])
            .unwrap_or(150);
        self.config.reader_line_height = next;
        let message = match config::save(&self.vault_root, &self.config) {
            Ok(()) => format!("reading line height: {next}%"),
            Err(error) => format!("reader setting changed (not saved: {error})"),
        };
        self.refresh_reader_settings(cx);
        self.set_status(StatusKind::Info, message);
        cx.notify();
    }

    /// Cycle the reading typeface: serif, sans, mono.
    fn cycle_reader_font(&mut self, cx: &mut Context<Self>) {
        let next = match self.config.reader_font.as_str() {
            "serif" => "sans",
            "sans" => "mono",
            _ => "serif",
        };
        self.config.reader_font = next.to_string();
        let message = match config::save(&self.vault_root, &self.config) {
            Ok(()) => format!("reading typeface: {next}"),
            Err(error) => format!("reader setting changed (not saved: {error})"),
        };
        self.refresh_reader_settings(cx);
        self.set_status(StatusKind::Info, message);
        cx.notify();
    }

    /// Turn note autosave on or off for this vault and every open editor.
    fn toggle_autosave(&mut self, cx: &mut Context<Self>) {
        self.config.autosave = !self.config.autosave;
        let autosave = self.config.autosave;
        let message = match config::save(&self.vault_root, &self.config) {
            Ok(()) => format!("autosave: {}", if autosave { "on" } else { "off" }),
            Err(e) => format!("autosave toggled (not saved: {e})"),
        };
        for tab in &self.tabs {
            tab.editor
                .update(cx, |editor, _| editor.set_autosave(autosave));
        }
        self.set_status(StatusKind::Info, message);
        cx.notify();
    }

    fn toggle_vim(&mut self, cx: &mut Context<Self>) {
        self.config.vim_mode = !self.config.vim_mode;
        let enabled = self.config.vim_mode;
        for tab in &self.tabs {
            tab.editor
                .update(cx, |editor, _| editor.set_vim_enabled(enabled));
        }
        let message = match config::save(&self.vault_root, &self.config) {
            Ok(()) => format!("Vim mode: {}", if enabled { "on" } else { "off" }),
            Err(error) => format!(
                "Vim mode: {} (not saved: {error})",
                if enabled { "on" } else { "off" }
            ),
        };
        self.set_status(StatusKind::Info, message);
        cx.notify();
    }

    /// Open the input panel for one setting, seeded with its current value.
    fn open_setting(&mut self, key: SettingKey, window: &mut Window, cx: &mut Context<Self>) {
        let value = match key {
            SettingKey::FontSize => self.config.font_size.to_string(),
            SettingKey::Accent => self.config.accent.clone().unwrap_or_default(),
            SettingKey::AttachmentsFolder => self.config.attachments_folder.clone(),
            SettingKey::TemplatesFolder => self.config.templates_folder.clone(),
            SettingKey::DailyFolder => self.config.daily_folder.clone(),
            SettingKey::WeeklyFolder => self.config.weekly_folder.clone(),
            SettingKey::MonthlyFolder => self.config.monthly_folder.clone(),
        };
        self.pending_setting = Some(key);
        self.open_panel(SwitcherMode::SettingInput, window, cx);
        if let Some(panel) = self.switcher.as_mut() {
            panel.query = value;
        }
        cx.notify();
    }

    fn apply_setting(&mut self, key: SettingKey, value: &str, cx: &mut Context<Self>) {
        match key {
            SettingKey::FontSize => match value.trim().parse::<u16>() {
                Ok(size) => {
                    let size = size.clamp(9, 32);
                    self.config.font_size = size;
                    self.apply_font_size(cx);
                }
                Err(_) => {
                    self.set_status(
                        StatusKind::Warn,
                        "font size must be a number from 9 to 32".to_string(),
                    );
                    cx.notify();
                    return;
                }
            },
            SettingKey::Accent => {
                let value = value.trim().trim_start_matches('#');
                self.config.accent = if value.is_empty() {
                    None
                } else if crate::theme::parse_hex(value).is_some() {
                    Some(value.to_string())
                } else {
                    self.set_status(
                        StatusKind::Warn,
                        "accent must be a hex colour such as 4ea1ff".to_string(),
                    );
                    cx.notify();
                    return;
                };
                self.refresh_theme(cx);
            }
            SettingKey::AttachmentsFolder => {
                let folder = value.trim();
                if !folder.is_empty() {
                    self.config.attachments_folder = folder.to_string();
                    for tab in &self.tabs {
                        tab.editor.update(cx, |editor, _| {
                            editor.set_attachments_folder(folder);
                        });
                    }
                }
            }
            SettingKey::TemplatesFolder => {
                if !value.trim().is_empty() {
                    self.config.templates_folder = value.trim().to_string();
                }
            }
            SettingKey::DailyFolder => {
                if !value.trim().is_empty() {
                    self.config.daily_folder = value.trim().to_string();
                }
            }
            SettingKey::WeeklyFolder => {
                if !value.trim().is_empty() {
                    self.config.weekly_folder = value.trim().to_string();
                }
            }
            SettingKey::MonthlyFolder => {
                if !value.trim().is_empty() {
                    self.config.monthly_folder = value.trim().to_string();
                }
            }
        }

        self.set_status(
            StatusKind::Error,
            match config::save(&self.vault_root, &self.config) {
                Ok(()) => format!("{}: saved", key.label()),
                Err(error) => format!("{}: not saved ({error})", key.label()),
            },
        );
        cx.notify();
    }

    fn apply_font_size(&mut self, cx: &mut Context<Self>) {
        let size = px(self.config.font_size as f32);
        for tab in &self.tabs {
            tab.editor
                .update(cx, |editor, _| editor.set_font_size(size));
        }
        self.editor.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    /// Dispatch a click on the settings page.
    fn run_setting_action(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            "theme" => self.toggle_theme_now(cx),
            "vim" => self.toggle_vim(cx),
            "snippets" => self.toggle_snippets(cx),
            "autosave" => self.toggle_autosave(cx),
            "reader-size" => self.cycle_reader_setting(false, cx),
            "reader-width" => self.cycle_reader_setting(true, cx),
            "reader-line" => self.cycle_reader_line_height(cx),
            "reader-font" => self.cycle_reader_font(cx),
            "scroll-sync" => self.toggle_scroll_sync(cx),
            "sidebar" => self.toggle_panel("sidebar", cx),
            "outline" => self.toggle_panel("outline", cx),
            "relations" => self.toggle_panel("relations", cx),
            "font" => self.open_setting(SettingKey::FontSize, window, cx),
            "accent" => self.open_setting(SettingKey::Accent, window, cx),
            "attachments" => self.open_setting(SettingKey::AttachmentsFolder, window, cx),
            "templates" => self.open_setting(SettingKey::TemplatesFolder, window, cx),
            "daily" => self.open_setting(SettingKey::DailyFolder, window, cx),
            "weekly" => self.open_setting(SettingKey::WeeklyFolder, window, cx),
            "monthly" => self.open_setting(SettingKey::MonthlyFolder, window, cx),
            "save-all" => {
                let result = self.save_all(cx);
                let (kind, message) = match result {
                    Ok(saved) => (StatusKind::Success, format!("saved {saved} note(s)")),
                    Err(error) => (StatusKind::Error, format!("could not save: {error}")),
                };
                self.set_status(kind, message);
                cx.notify();
            }
            "updates" => {
                self.check_updates();
                cx.notify();
            }
            "vault" => {
                self.show_settings = false;
                vault_picker::VaultPicker::open(cx, None, Rc::new(open_vault_in_new_process));
            }
            "close" => {
                self.show_settings = false;
                window.focus(&self.editor.focus_handle(cx));
                cx.notify();
            }
            _ => {}
        }
    }

    /// The settings page, shown in place of the editor while `show_settings` is set.
    fn settings_view(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let entity = cx.entity();
        let row = move |id: &str,
                        label: &str,
                        value: String,
                        action: &'static str,
                        theme: Theme|
              -> AnyElement {
            let entity = entity.clone();
            div()
                .id(ElementId::Name(id.to_string().into()))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .h(px(36.0))
                .px_3()
                .rounded_md()
                .cursor_pointer()
                .text_sm()
                .text_color(theme.text)
                .hover(move |style| style.bg(theme.hover))
                .child(label.to_string())
                .child(div().text_color(theme.muted).child(if value.is_empty() {
                    "not set".to_string()
                } else {
                    value
                }))
                .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                    entity.update(cx, |this, cx| this.run_setting_action(action, window, cx));
                })
                .into_any_element()
        };

        let heading = |text: &str, theme: Theme| -> AnyElement {
            div()
                .pt_4()
                .pb_1()
                .text_xs()
                .text_color(theme.muted)
                .child(text.to_string())
                .into_any_element()
        };

        // Read-only reference rows: the editor no longer prints shortcut hints on screen.
        let info = |label: &str, value: &str, theme: Theme| -> AnyElement {
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .h(px(28.0))
                .px_3()
                .text_sm()
                .text_color(theme.text)
                .child(label.to_string())
                .child(div().text_color(theme.muted).child(value.to_string()))
                .into_any_element()
        };

        let accent = self
            .config
            .accent
            .clone()
            .unwrap_or_else(|| "theme default".to_string());
        let user_themes = if self.user_themes.is_empty() {
            "none installed".to_string()
        } else {
            self.user_themes
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };

        div()
            .id("settings-page")
            .track_focus(&self.settings_focus)
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .max_w(px(720.0))
                    .mx_auto()
                    .px_6()
                    .py_6()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_xl().text_color(theme.text).child("Settings"))
                            .child(
                                div()
                                    .id("settings-close")
                                    .px_3()
                                    .py_1()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .bg(theme.chip)
                                    .text_sm()
                                    .text_color(theme.muted)
                                    .hover(move |style| style.bg(theme.hover))
                                    .child("Close (Esc)")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _event, window, cx| {
                                            this.run_setting_action("close", window, cx);
                                        }),
                                    ),
                            ),
                    )
                    .child(heading("Appearance", theme))
                    .child(row(
                        "setting-theme",
                        "Theme",
                        self.theme.name().to_string(),
                        "theme",
                        theme,
                    ))
                    .child(row(
                        "setting-accent",
                        "Accent colour",
                        accent,
                        "accent",
                        theme,
                    ))
                    .child(row(
                        "setting-font",
                        "Editor font size",
                        format!("{} px", self.config.font_size),
                        "font",
                        theme,
                    ))
                    .child(row(
                        "setting-themes",
                        "Installed themes",
                        user_themes,
                        "none",
                        theme,
                    ))
                    .child(heading("Panels", theme))
                    .child(row(
                        "setting-sidebar",
                        "Sidebar",
                        if self.config.show_sidebar {
                            "on"
                        } else {
                            "off"
                        }
                        .to_string(),
                        "sidebar",
                        theme,
                    ))
                    .child(row(
                        "setting-outline",
                        "Outline",
                        if self.config.show_outline {
                            "on"
                        } else {
                            "off"
                        }
                        .to_string(),
                        "outline",
                        theme,
                    ))
                    .child(row(
                        "setting-relations",
                        "Relations",
                        if self.config.show_relations {
                            "on"
                        } else {
                            "off"
                        }
                        .to_string(),
                        "relations",
                        theme,
                    ))
                    .child(heading("Editor", theme))
                    .child(row(
                        "setting-vim",
                        "Vim mode",
                        if self.config.vim_mode { "on" } else { "off" }.to_string(),
                        "vim",
                        theme,
                    ))
                    .child(row(
                        "setting-snippets",
                        "Snippets",
                        if self.config.snippets { "on" } else { "off" }.to_string(),
                        "snippets",
                        theme,
                    ))
                    .child(row(
                        "setting-autosave",
                        "Autosave notes",
                        if self.config.autosave { "on" } else { "off" }.to_string(),
                        "autosave",
                        theme,
                    ))
                    .child(row(
                        "setting-reader-size",
                        "Reading text size",
                        format!("{} px", self.config.reader_size),
                        "reader-size",
                        theme,
                    ))
                    .child(row(
                        "setting-reader-width",
                        "Reading width",
                        format!("{} px", self.config.reader_width),
                        "reader-width",
                        theme,
                    ))
                    .child(row(
                        "setting-reader-line",
                        "Reading line height",
                        format!("{:.0}%", self.config.reader_line_height as f32),
                        "reader-line",
                        theme,
                    ))
                    .child(row(
                        "setting-reader-font",
                        "Reading typeface",
                        self.config.reader_font.clone(),
                        "reader-font",
                        theme,
                    ))
                    .child(heading("Editor shortcuts", theme))
                    .child(info("Bold", &platform::label("secondary-b"), theme))
                    .child(info("Italic", &platform::label("secondary-i"), theme))
                    .child(info(
                        "Strikethrough",
                        &platform::label("secondary-shift-x"),
                        theme,
                    ))
                    .child(info(
                        "Highlight",
                        &platform::label("secondary-shift-h"),
                        theme,
                    ))
                    .child(info(
                        "Inline code",
                        &platform::label("secondary-shift-c"),
                        theme,
                    ))
                    .child(info(
                        "Wikilink picker",
                        &platform::label("secondary-k"),
                        theme,
                    ))
                    .child(info("Link selected text", "type [[", theme))
                    .child(info("Autocomplete", "↑ / ↓ then Enter", theme))
                    .child(heading("Files", theme))
                    .child(row(
                        "setting-attachments",
                        "Attachments folder",
                        self.config.attachments_folder.clone(),
                        "attachments",
                        theme,
                    ))
                    .child(row(
                        "setting-templates",
                        "Templates folder",
                        self.config.templates_folder.clone(),
                        "templates",
                        theme,
                    ))
                    .child(row(
                        "setting-daily",
                        "Daily notes folder",
                        self.config.daily_folder.clone(),
                        "daily",
                        theme,
                    ))
                    .child(row(
                        "setting-weekly",
                        "Weekly notes folder",
                        self.config.weekly_folder.clone(),
                        "weekly",
                        theme,
                    ))
                    .child(row(
                        "setting-monthly",
                        "Monthly notes folder",
                        self.config.monthly_folder.clone(),
                        "monthly",
                        theme,
                    ))
                    .child(heading("Data", theme))
                    .child(row(
                        "setting-save-all",
                        "Save all open notes",
                        format!("{} open", self.tabs.len()),
                        "save-all",
                        theme,
                    ))
                    .child(row(
                        "setting-updates",
                        "Updates",
                        self.update_available
                            .clone()
                            .unwrap_or_else(|| "check for updates".to_string()),
                        "updates",
                        theme,
                    ))
                    .child(row(
                        "setting-vault",
                        "Open another vault",
                        self.vault_path.clone(),
                        "vault",
                        theme,
                    ))
                    .child(row(
                        "setting-git",
                        "Git",
                        git_label(&self.git_status),
                        "none",
                        theme,
                    ))
                    .child(row(
                        "setting-version",
                        "Version",
                        env!("CARGO_PKG_VERSION").to_string(),
                        "none",
                        theme,
                    )),
            )
            .into_any_element()
    }

    fn apply_user_theme(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some((name, theme)) = self.user_themes.get(index).cloned() else {
            return;
        };

        self.base_theme = theme;
        cx.set_global(theme);
        self.refresh_theme(cx);
        let (kind, message) = match save_theme(&self.vault_root, theme, Some(&name)) {
            Ok(()) => (StatusKind::Success, format!("theme: {name}")),
            Err(e) => (StatusKind::Error, format!("theme: {name} (not saved: {e})")),
        };
        self.set_status(kind, message);
        self.editor.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn toggle_theme(&mut self, _: &ToggleTheme, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_theme_now(cx);
        let _ = window;
    }

    fn toggle_vim_action(&mut self, _: &ToggleVim, _window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_vim(cx);
    }

    fn check_updates(&mut self) {
        let Some(source) = self.update_source.clone() else {
            self.set_status(
                StatusKind::Warn,
                "update checks are not configured (set DRAGON_GLASS_UPDATE_URL)",
            );
            return;
        };
        match update::latest_if_newer(env!("CARGO_PKG_VERSION"), &source) {
            Ok(Some(latest)) => {
                self.update_available = Some(latest.clone());
                self.set_status(StatusKind::Info, format!("update available: {latest}"));
            }
            Ok(None) => {
                self.update_available = None;
                self.set_status(StatusKind::Info, "up to date");
            }
            Err(e) => self.set_status(StatusKind::Error, format!("update check failed: {e}")),
        }
    }

    fn toggle_theme_now(&mut self, cx: &mut Context<Self>) {
        self.base_theme = self.base_theme.toggled();
        self.refresh_theme(cx);
        let message = match save_theme(&self.vault_root, self.base_theme, None) {
            Ok(()) => format!("theme: {}", self.theme.name()),
            Err(e) => format!("theme: {} (not saved: {e})", self.theme.name()),
        };
        self.set_status(StatusKind::Info, message);
        self.editor.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn refresh_properties(&mut self, cx: &mut Context<Self>) {
        self.properties = self.editor.update(cx, |editor, _cx| editor.properties());
    }

    fn toggle_bookmark(
        &mut self,
        _: &ToggleBookmark,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_bookmark_now(cx);
    }

    fn toggle_bookmark_now(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else {
            return;
        };

        let added = bookmarks::toggle(&mut self.bookmarks, &active);
        self.set_status(
            StatusKind::Error,
            match bookmarks::save(&self.vault_root, &self.bookmarks) {
                Ok(()) if added => format!("bookmarked {active}"),
                Ok(()) => format!("removed bookmark {active}"),
                Err(e) => format!("could not save bookmarks: {e}"),
            },
        );
        cx.notify();
    }

    fn open_properties(&mut self, _: &OpenProperties, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_properties(cx);
        self.open_panel(SwitcherMode::PropertiesMenu, window, cx);
    }

    fn apply_properties(&mut self, properties: &[Property], cx: &mut Context<Self>) {
        let result = self.editor.update(cx, |editor, cx| {
            let result = editor.set_properties(properties);
            cx.notify();
            result
        });

        match result {
            Ok(()) => {
                self.properties = properties.to_vec();
                self.cache_epoch = self.cache_epoch.wrapping_add(1);
                self.set_status(StatusKind::Info, "properties updated".to_string());
            }
            Err(e) => {
                self.set_status(StatusKind::Error, format!("could not save properties: {e}"));
            }
        }
    }

    fn run_git_action(&mut self, action: GitAction) {
        let root = self.vault_root.clone();
        self.set_status(
            StatusKind::Error,
            match action {
                GitAction::Init => match git::init(&root) {
                    Ok(()) => "initialized git repository".to_string(),
                    Err(e) => format!("git init failed: {e}"),
                },
                GitAction::Pull => match git::pull(&root) {
                    Ok(outcome) => describe_outcome(outcome),
                    Err(e) => format!("git pull failed: {e}"),
                },
                GitAction::Push => match git::push(&root) {
                    Ok(outcome) => describe_outcome(outcome),
                    Err(e) => format!("git push failed: {e}"),
                },
                GitAction::Refresh => "status refreshed".to_string(),
                GitAction::Commit => "unreachable".to_string(),
            },
        );
        self.refresh_git();
    }

    fn commit(&mut self, message: &str) {
        let message = message.trim();
        if message.is_empty() {
            return;
        }
        self.set_status(
            StatusKind::Error,
            match git::commit_all(&self.vault_root, message) {
                Ok(true) => format!("committed: {message}"),
                Ok(false) => "nothing to commit".to_string(),
                Err(e) => format!("git commit failed: {e}"),
            },
        );
        self.refresh_git();
    }

    fn on_switcher_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut panel) = self.switcher.take() else {
            return;
        };
        let mut query_changed = false;

        // Capturing a new shortcut for the hotkey editor.
        if panel.mode == SwitcherMode::HotkeyCapture {
            if event.keystroke.key == "escape" {
                self.close_panel(window, cx);
                return;
            }

            let key = event.keystroke.unparse().to_lowercase();
            if let Some(action) = panel.capture_action.clone() {
                keymap::upsert(
                    &mut self.bindings,
                    keymap::Binding {
                        key: key.clone(),
                        action: action.clone(),
                    },
                );
                let saved = keymap::save(&self.vault_root, &self.bindings);
                bind_named(cx, &key, &action);

                let (kind, message) = match saved {
                    Ok(()) => (StatusKind::Success, format!("bound {key} to {action}")),
                    Err(e) => (
                        StatusKind::Warn,
                        format!("bound {key} to {action} (not saved: {e})"),
                    ),
                };
                self.set_status(kind, message);
            }
            self.close_panel(window, cx);
            return;
        }

        match event.keystroke.key.as_str() {
            "escape" => {
                self.close_panel(window, cx);
                return;
            }
            "enter" => {
                if panel.mode == SwitcherMode::CommitInput {
                    let message = panel.query.clone();
                    self.commit(&message);
                    self.close_panel(window, cx);
                    return;
                }

                if panel.mode == SwitcherMode::PropertyInput {
                    let query = panel.query.clone();
                    let mut properties = self.properties.clone();

                    if let Some(key) = panel.property_key.clone() {
                        if let Some(property) = properties.iter_mut().find(|p| p.key == key) {
                            property.value = PropertyValue::from_input(&query);
                        }
                    } else if let Some((key, value)) = query.split_once(':') {
                        let key = key.trim().to_string();
                        if !key.is_empty() {
                            let value = PropertyValue::from_input(value.trim());
                            match properties.iter_mut().find(|p| p.key == key) {
                                Some(property) => property.value = value,
                                None => properties.push(Property { key, value }),
                            }
                        }
                    }

                    self.apply_properties(&properties, cx);
                    self.close_panel(window, cx);
                    return;
                }

                if panel.mode == SwitcherMode::QueryInput {
                    let expression = panel.query.clone();
                    self.query_results = Rc::new(query::run(&self.note_refs, &expression));
                    self.show_query = true;
                    self.show_graph = false;
                    self.show_diff = false;
                    self.set_status(
                        StatusKind::Info,
                        format!("query: {} rows", self.query_results.len()),
                    );
                    self.close_panel(window, cx);
                    return;
                }

                if panel.mode == SwitcherMode::WorkspaceName {
                    let name = panel.query.clone();
                    self.save_workspace(&name, cx);
                    self.close_panel(window, cx);
                    return;
                }

                if panel.mode == SwitcherMode::GraphFilterInput {
                    let filter = panel.query.clone();
                    self.open_graph(window, cx);
                    if let Some(view) = self.graph_view.clone() {
                        view.update(cx, |view, cx| {
                            view.set_filter(&filter);
                            cx.notify();
                        });
                    }
                    self.set_status(StatusKind::Info, format!("graph filter: {filter}"));
                    self.close_panel(window, cx);
                    return;
                }

                if panel.mode == SwitcherMode::SharedName {
                    let slug = panel.query.clone();
                    let content = self.pending_shared.take();
                    self.close_panel(window, cx);
                    if let Some(content) = content {
                        match shared::save(&self.vault_root, &slug, &content) {
                            Ok(block) => {
                                let reference = format!("{{{{shared:{}}}}}", block.slug);
                                self.editor.update(cx, |editor, cx| {
                                    editor.replace_selection_or_insert(&reference);
                                    cx.notify();
                                });
                                window.focus(&self.editor.focus_handle(cx));
                                self.set_status(
                                    StatusKind::Success,
                                    format!("shared block created: {}", block.slug),
                                );
                            }
                            Err(error) => {
                                self.set_status(
                                    StatusKind::Error,
                                    format!("could not save shared block: {error}"),
                                );
                            }
                        }
                    }
                    cx.notify();
                    return;
                }

                if panel.mode == SwitcherMode::NewNoteName {
                    let name = panel.query.clone();
                    let template = self.pending_template.take();
                    let dir = self.pending_dir.take();
                    let save_as = self.pending_save_as;
                    self.pending_save_as = false;
                    self.close_panel(window, cx);
                    if save_as {
                        self.save_active_as(&name, window, cx);
                    } else if let Some(template) = template {
                        self.create_from_template(&name, &template, window, cx);
                    } else {
                        self.create_note_in(dir.as_deref(), &name, window, cx);
                    }
                    return;
                }

                if panel.mode == SwitcherMode::NewFolderName {
                    let name = panel.query.clone();
                    let dir = self.pending_dir.take();
                    self.close_panel(window, cx);
                    self.create_folder_in(dir.as_deref(), &name, cx);
                    return;
                }

                if panel.mode == SwitcherMode::RenameInput {
                    let to = panel.query.clone();
                    let from = self.pending_path.take();
                    self.close_panel(window, cx);
                    if let Some(from) = from {
                        self.rename_entry(&from, &to, cx);
                    }
                    return;
                }

                if panel.mode == SwitcherMode::DeleteConfirm {
                    let path = self.pending_path.take();
                    self.close_panel(window, cx);
                    if let Some(path) = path {
                        self.delete_entry(&path, window, cx);
                    }
                    return;
                }

                if panel.mode == SwitcherMode::SettingInput {
                    let value = panel.query.clone();
                    let key = self.pending_setting.take();
                    self.close_panel(window, cx);
                    if let Some(key) = key {
                        self.apply_setting(key, &value, cx);
                    }
                    return;
                }

                if panel.mode == SwitcherMode::RelationInput {
                    let target = panel.query.clone();
                    self.close_panel(window, cx);
                    self.add_relation(&target, cx);
                    return;
                }

                if let Some(item) = panel.results.get(panel.selected) {
                    match &item.item {
                        Item::Note { relative } => {
                            let relative = relative.clone();
                            self.open_note(&relative, window, cx);
                            self.close_panel(window, cx);
                            return;
                        }
                        Item::NoteAt { relative, line } => {
                            let relative = relative.clone();
                            let line = *line;
                            let query = panel.query.clone();
                            self.open_note(&relative, window, cx);
                            self.close_panel(window, cx);
                            let editor = self.editor.clone();
                            editor.update(cx, |editor, cx| {
                                // A search hit is an editing target: show the source line.
                                editor.set_reading_mode(false);
                                editor.select_line_text(line.saturating_sub(1), &query);
                                cx.notify();
                            });
                            self.set_status(StatusKind::Info, format!("{relative}:{line}"));
                            return;
                        }
                        Item::Git(action) => {
                            let action = *action;
                            if action == GitAction::Commit {
                                panel.mode = SwitcherMode::CommitInput;
                                panel.query.clear();
                                panel.results.clear();
                                panel.selected = 0;
                                self.switcher = Some(panel);
                                cx.notify();
                                return;
                            }
                            self.run_git_action(action);
                            self.close_panel(window, cx);
                            return;
                        }
                        Item::Property { key } => {
                            let existing = key.clone();
                            let query = existing
                                .as_ref()
                                .and_then(|key| {
                                    self.properties
                                        .iter()
                                        .find(|p| &p.key == key)
                                        .map(|p| p.value.display())
                                })
                                .unwrap_or_default();

                            panel.mode = SwitcherMode::PropertyInput;
                            panel.property_key = existing;
                            panel.query = query;
                            panel.results.clear();
                            panel.selected = 0;
                            self.switcher = Some(panel);
                            cx.notify();
                            return;
                        }
                        Item::DeleteProperty { key } => {
                            let key = key.clone();
                            let properties: Vec<Property> = self
                                .properties
                                .iter()
                                .filter(|p| p.key != key)
                                .cloned()
                                .collect();
                            self.apply_properties(&properties, cx);
                            self.close_panel(window, cx);
                            return;
                        }
                        Item::Hotkey(action) => {
                            panel.mode = SwitcherMode::HotkeyCapture;
                            panel.capture_action = Some(action.clone());
                            panel.query.clear();
                            panel.results.clear();
                            panel.selected = 0;
                            self.switcher = Some(panel);
                            cx.notify();
                            return;
                        }
                        Item::Workspace(name) => {
                            let name = name.clone();
                            self.apply_workspace(&name, window, cx);
                            self.close_panel(window, cx);
                            return;
                        }
                        Item::VersionSave(label) => {
                            let label = label.clone();
                            self.close_panel(window, cx);
                            self.save_version(&label, cx);
                            return;
                        }
                        Item::VersionCompare { left, right } => {
                            let left = left.clone();
                            let right = right.clone();
                            self.close_panel(window, cx);
                            self.compare_version(left, right, cx);
                            return;
                        }
                        Item::VersionPromote(id) => {
                            let id = id.clone();
                            self.close_panel(window, cx);
                            self.promote_version(&id, window, cx);
                            return;
                        }
                        Item::VersionDelete(id) => {
                            let id = id.clone();
                            self.close_panel(window, cx);
                            self.delete_version(&id, cx);
                            return;
                        }
                        Item::Template(relative) => {
                            let relative = relative.clone();
                            self.close_panel(window, cx);
                            self.insert_template(&relative, window, cx);
                            return;
                        }
                        Item::TemplateNew(relative) => {
                            let relative = relative.clone();
                            let template =
                                templates::list(&self.vault_root, &self.config.templates_folder)
                                    .into_iter()
                                    .find(|template| template.relative == relative);
                            if let Some(template) = template {
                                self.pending_template = Some(template);
                                panel.mode = SwitcherMode::NewNoteName;
                                panel.query.clear();
                                panel.results.clear();
                                panel.selected = 0;
                                self.switcher = Some(panel);
                                cx.notify();
                            }
                            return;
                        }
                        Item::Shared(slug) => {
                            let slug = slug.clone();
                            self.close_panel(window, cx);
                            self.insert_shared(&slug, window, cx);
                            return;
                        }
                        Item::SharedEdit(slug) => {
                            let relative =
                                format!(".dragon-glass/shared/{}.md", shared::sanitize(slug));
                            self.open_note(&relative, window, cx);
                            self.close_panel(window, cx);
                            return;
                        }
                        Item::Issue(line) => {
                            let line = *line;
                            self.close_panel(window, cx);
                            self.editor.update(cx, |editor, cx| {
                                editor.jump_to_line(line.saturating_sub(1));
                                cx.notify();
                            });
                            window.focus(&self.editor.focus_handle(cx));
                            return;
                        }
                        Item::HistoryCompare(id) => {
                            let id = id.clone();
                            self.close_panel(window, cx);
                            self.compare_snapshot(&id, cx);
                            return;
                        }
                        Item::HistoryRestore(id) => {
                            let id = id.clone();
                            self.close_panel(window, cx);
                            self.restore_snapshot(&id, window, cx);
                            return;
                        }
                        Item::Tree(action) => {
                            let action = *action;
                            let Some(target) = self.tree_menu.clone() else {
                                self.close_panel(window, cx);
                                return;
                            };
                            let menu_dir = if target.is_dir {
                                target.path.clone()
                            } else {
                                target
                                    .path
                                    .rsplit_once('/')
                                    .map(|(dir, _)| dir.to_string())
                                    .unwrap_or_default()
                            };

                            match action {
                                TreeAction::Open => {
                                    self.close_panel(window, cx);
                                    if target.is_dir {
                                        self.toggle_dir(&target.path, cx);
                                    } else {
                                        self.open_note(&target.path, window, cx);
                                    }
                                    return;
                                }
                                TreeAction::NewNote => {
                                    self.pending_dir = Some(menu_dir);
                                    self.pending_template = None;
                                    panel.mode = SwitcherMode::NewNoteName;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results.clear();
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                TreeAction::NewFolder => {
                                    self.pending_dir = Some(menu_dir);
                                    panel.mode = SwitcherMode::NewFolderName;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results.clear();
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                TreeAction::Rename => {
                                    self.pending_path = Some(target.path.clone());
                                    panel.mode = SwitcherMode::RenameInput;
                                    panel.query = target.path;
                                    panel.property_key = None;
                                    panel.results.clear();
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                TreeAction::Delete => {
                                    if panel.mode == SwitcherMode::DeleteConfirm {
                                        let path = target.path;
                                        self.close_panel(window, cx);
                                        self.delete_entry(&path, window, cx);
                                        return;
                                    }
                                    self.pending_path = Some(target.path.clone());
                                    panel.mode = SwitcherMode::DeleteConfirm;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results = vec![SwitcherItem {
                                        label: format!("Delete {}", target.path),
                                        detail: "moves to .dragon-glass/trash".to_string(),
                                        item: Item::Tree(TreeAction::Delete),
                                    }];
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                            }
                        }
                        Item::Command(command) => {
                            let command = *command;

                            let next_mode = match command {
                                CommandId::QuickSwitch => Some(SwitcherMode::QuickSwitch),
                                CommandId::Search => Some(SwitcherMode::Search),
                                CommandId::Git => {
                                    self.refresh_git();
                                    Some(SwitcherMode::GitMenu)
                                }
                                CommandId::Properties => {
                                    self.refresh_properties(cx);
                                    Some(SwitcherMode::PropertiesMenu)
                                }
                                _ => None,
                            };

                            if let Some(mode) = next_mode {
                                panel.mode = mode;
                                panel.query.clear();
                                panel.property_key = None;
                                panel.results = self.compute_results(mode, "");
                                panel.selected = 0;
                                self.switcher = Some(panel);
                                cx.notify();
                                return;
                            }

                            match command {
                                CommandId::Daily => {
                                    self.create_periodic(Periodic::Daily, window, cx)
                                }
                                CommandId::Weekly => {
                                    self.create_periodic(Periodic::Weekly, window, cx)
                                }
                                CommandId::Monthly => {
                                    self.create_periodic(Periodic::Monthly, window, cx)
                                }
                                CommandId::Bookmark => self.toggle_bookmark_now(cx),
                                CommandId::ToggleReading => {
                                    self.editor.update(cx, |editor, cx| {
                                        editor.toggle_reading_mode();
                                        cx.notify();
                                    });
                                    self.remember_session(cx);
                                }
                                CommandId::FindReplace => {
                                    self.close_panel(window, cx);
                                    self.editor.update(cx, |editor, cx| {
                                        editor.open_find(true);
                                        cx.notify();
                                    });
                                    window.focus(&self.editor.focus_handle(cx));
                                    return;
                                }
                                CommandId::Zen => {
                                    self.zen = !self.zen;
                                    self.set_status(
                                        StatusKind::Info,
                                        if self.zen { "zen mode" } else { "zen mode off" },
                                    );
                                    cx.notify();
                                    return;
                                }
                                CommandId::ToggleVim => {
                                    self.toggle_vim(cx);
                                    self.close_panel(window, cx);
                                    return;
                                }
                                CommandId::ToggleFold => {
                                    self.close_panel(window, cx);
                                    self.editor.update(cx, |editor, cx| {
                                        editor.toggle_fold_at_cursor();
                                        cx.notify();
                                    });
                                    window.focus(&self.editor.focus_handle(cx));
                                    return;
                                }
                                CommandId::Save => self.editor.update(cx, |editor, cx| {
                                    let _ = editor.save();
                                    cx.notify();
                                }),
                                CommandId::ToggleTheme => self.toggle_theme_now(cx),
                                CommandId::CheckUpdates => self.check_updates(),
                                CommandId::OpenGraph => self.open_graph(window, cx),
                                CommandId::RunQuery => {
                                    panel.mode = SwitcherMode::QueryInput;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results.clear();
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::Plugin(index) => self.run_plugin_command(index, cx),
                                CommandId::UserTheme(index) => self.apply_user_theme(index, cx),
                                CommandId::NewWindow => self.open_new_window(window, cx),
                                CommandId::OpenVault => {
                                    self.close_panel(window, cx);
                                    open_vault_picker(cx, None, Rc::new(open_vault_in_new_process));
                                    return;
                                }
                                CommandId::DetachTab => {
                                    self.detach_tab(self.active_tab, window, cx);
                                    self.close_panel(window, cx);
                                    return;
                                }
                                CommandId::ToggleSplitEditor => {
                                    self.toggle_split_editor(cx);
                                    self.close_panel(window, cx);
                                    return;
                                }
                                CommandId::CloseTab => {
                                    self.close_tab(self.active_tab, window, cx);
                                    self.close_panel(window, cx);
                                    return;
                                }
                                CommandId::NextTab => {
                                    self.cycle_tab(1, window, cx);
                                    self.close_panel(window, cx);
                                    return;
                                }
                                CommandId::PrevTab => {
                                    self.cycle_tab(-1, window, cx);
                                    self.close_panel(window, cx);
                                    return;
                                }
                                CommandId::GraphFilter => {
                                    panel.mode = SwitcherMode::GraphFilterInput;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results.clear();
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::GraphDepth => {
                                    if let Some(view) = self.graph_view.clone() {
                                        let next = view.read(cx).depth() % 3 + 1;
                                        self.open_graph(window, cx);
                                        view.update(cx, |view, cx| {
                                            view.set_depth(next);
                                            cx.notify();
                                        });
                                        self.set_status(
                                            StatusKind::Info,
                                            format!("graph depth: {next}"),
                                        );
                                    }
                                    self.close_panel(window, cx);
                                    return;
                                }
                                CommandId::ToggleSnippets => self.toggle_snippets(cx),
                                CommandId::TogglePreview => self.toggle_split_preview(cx),
                                CommandId::DetachPreview => {
                                    let opener = self.link_opener(cx);
                                    preview_window::PreviewWindow::open(
                                        self.editor.clone(),
                                        self.vault_root.clone(),
                                        opener,
                                        cx,
                                    );
                                    self.set_status(
                                        StatusKind::Info,
                                        "preview opened in its own window".to_string(),
                                    );
                                    cx.notify();
                                }
                                CommandId::Hotkeys => {
                                    panel.mode = SwitcherMode::HotkeyMenu;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results =
                                        self.compute_results(SwitcherMode::HotkeyMenu, "");
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::SaveWorkspace => {
                                    panel.mode = SwitcherMode::WorkspaceName;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results.clear();
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::LoadWorkspace => {
                                    panel.mode = SwitcherMode::WorkspaceMenu;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results =
                                        self.compute_results(SwitcherMode::WorkspaceMenu, "");
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::Versions => {
                                    panel.mode = SwitcherMode::VersionMenu;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results =
                                        self.compute_results(SwitcherMode::VersionMenu, "");
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::SaveVersionA | CommandId::SaveVersionB => {
                                    let label = if command == CommandId::SaveVersionA {
                                        "A"
                                    } else {
                                        "B"
                                    };
                                    self.close_panel(window, cx);
                                    self.save_version(label, cx);
                                    return;
                                }
                                CommandId::InsertTemplate => {
                                    panel.mode = SwitcherMode::TemplateMenu;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results =
                                        self.compute_results(SwitcherMode::TemplateMenu, "");
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::NewFromTemplate => {
                                    panel.mode = SwitcherMode::TemplateNewMenu;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results =
                                        self.compute_results(SwitcherMode::TemplateNewMenu, "");
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::InsertShared => {
                                    panel.mode = SwitcherMode::SharedMenu;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results =
                                        self.compute_results(SwitcherMode::SharedMenu, "");
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::EditShared => {
                                    panel.mode = SwitcherMode::SharedEditMenu;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results =
                                        self.compute_results(SwitcherMode::SharedEditMenu, "");
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::ExtractShared => {
                                    self.extract_shared(window, cx);
                                    return;
                                }
                                CommandId::ReloadNote => {
                                    self.editor.update(cx, |editor, cx| {
                                        let _ = editor.reload();
                                        cx.notify();
                                    });
                                    self.cache_epoch = self.cache_epoch.wrapping_add(1);
                                    self.refresh_properties(cx);
                                    self.close_panel(window, cx);
                                    self.set_status(
                                        StatusKind::Info,
                                        "reloaded from disk".to_string(),
                                    );
                                    return;
                                }
                                CommandId::ReviewDocument => {
                                    self.review_document(window, cx);
                                    return;
                                }
                                CommandId::Relations => {
                                    self.close_panel(window, cx);
                                    self.relations = relations::build(&self.note_refs);
                                    self.show_relations = true;
                                    self.set_status(
                                        StatusKind::Info,
                                        "document relations".to_string(),
                                    );
                                    cx.notify();
                                    return;
                                }
                                CommandId::RelationGraph => {
                                    self.close_panel(window, cx);
                                    self.open_relation_graph(window, cx);
                                    return;
                                }
                                CommandId::MoveBlockUp => {
                                    let moved = self.editor.update(cx, |editor, cx| {
                                        let moved = editor.move_block_up();
                                        cx.notify();
                                        moved
                                    });
                                    self.close_panel(window, cx);
                                    self.set_status(
                                        StatusKind::Info,
                                        if moved {
                                            "block moved up".to_string()
                                        } else {
                                            "nothing to move up".to_string()
                                        },
                                    );
                                    return;
                                }
                                CommandId::MoveBlockDown => {
                                    let moved = self.editor.update(cx, |editor, cx| {
                                        let moved = editor.move_block_down();
                                        cx.notify();
                                        moved
                                    });
                                    self.close_panel(window, cx);
                                    self.set_status(
                                        StatusKind::Info,
                                        if moved {
                                            "block moved down".to_string()
                                        } else {
                                            "nothing to move down".to_string()
                                        },
                                    );
                                    return;
                                }
                                CommandId::DuplicateBlock => {
                                    let changed = self.editor.update(cx, |editor, cx| {
                                        let changed = editor.duplicate_block();
                                        cx.notify();
                                        changed
                                    });
                                    self.close_panel(window, cx);
                                    self.set_status(
                                        StatusKind::Info,
                                        if changed {
                                            "block duplicated".to_string()
                                        } else {
                                            "no block here".to_string()
                                        },
                                    );
                                    return;
                                }
                                CommandId::DeleteBlock => {
                                    let changed = self.editor.update(cx, |editor, cx| {
                                        let changed = editor.delete_block();
                                        cx.notify();
                                        changed
                                    });
                                    self.close_panel(window, cx);
                                    self.set_status(
                                        StatusKind::Info,
                                        if changed {
                                            "block deleted".to_string()
                                        } else {
                                            "no block here".to_string()
                                        },
                                    );
                                    return;
                                }
                                CommandId::AddRelation => {
                                    panel.mode = SwitcherMode::RelationInput;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results.clear();
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::Settings => {
                                    self.close_panel(window, cx);
                                    self.show_settings = true;
                                    self.show_graph = false;
                                    self.show_query = false;
                                    self.show_diff = false;
                                    self.set_status(StatusKind::Info, "settings".to_string());
                                    window.focus(&self.settings_focus);
                                    cx.notify();
                                    return;
                                }
                                CommandId::NewNote => {
                                    let dir = self
                                        .active
                                        .as_deref()
                                        .and_then(|active| active.rsplit_once('/'))
                                        .map(|(dir, _)| dir.to_string())
                                        .unwrap_or_default();
                                    self.pending_dir = Some(dir);
                                    self.pending_template = None;
                                    panel.mode = SwitcherMode::NewNoteName;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results.clear();
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::NewFolder => {
                                    let dir = self
                                        .active
                                        .as_deref()
                                        .and_then(|active| active.rsplit_once('/'))
                                        .map(|(dir, _)| dir.to_string())
                                        .unwrap_or_default();
                                    self.pending_dir = Some(dir);
                                    panel.mode = SwitcherMode::NewFolderName;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results.clear();
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::SaveAs => {
                                    self.pending_save_as = true;
                                    self.pending_template = None;
                                    panel.mode = SwitcherMode::NewNoteName;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results.clear();
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::RenameNote => {
                                    let Some(relative) = self.active.clone() else {
                                        self.close_panel(window, cx);
                                        self.set_status(
                                            StatusKind::Warn,
                                            "open a note before renaming".to_string(),
                                        );
                                        cx.notify();
                                        return;
                                    };
                                    self.pending_path = Some(relative.clone());
                                    panel.mode = SwitcherMode::RenameInput;
                                    panel.query = relative;
                                    panel.property_key = None;
                                    panel.results.clear();
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::DeleteNote => {
                                    let Some(relative) = self.active.clone() else {
                                        self.close_panel(window, cx);
                                        self.set_status(
                                            StatusKind::Warn,
                                            "open a note before deleting".to_string(),
                                        );
                                        cx.notify();
                                        return;
                                    };
                                    self.tree_menu = Some(TreeTarget {
                                        path: relative.clone(),
                                        is_dir: false,
                                    });
                                    self.pending_path = Some(relative.clone());
                                    panel.mode = SwitcherMode::DeleteConfirm;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results = vec![SwitcherItem {
                                        label: format!("Delete {relative}"),
                                        detail: "moves to .dragon-glass/trash".to_string(),
                                        item: Item::Tree(TreeAction::Delete),
                                    }];
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                CommandId::History => {
                                    panel.mode = SwitcherMode::HistoryMenu;
                                    panel.query.clear();
                                    panel.property_key = None;
                                    panel.results =
                                        self.compute_results(SwitcherMode::HistoryMenu, "");
                                    panel.selected = 0;
                                    self.switcher = Some(panel);
                                    cx.notify();
                                    return;
                                }
                                _ => {}
                            }
                            self.close_panel(window, cx);
                            return;
                        }
                    }
                }
                self.close_panel(window, cx);
                return;
            }
            "up" => {
                panel.selected = panel.selected.saturating_sub(1);
            }
            "down" => {
                if panel.selected + 1 < panel.results.len() {
                    panel.selected += 1;
                }
            }
            "backspace" => {
                panel.query.pop();
                query_changed = true;
            }
            _ => {
                if let Some(text) = event.keystroke.key_char.as_ref() {
                    if !event.keystroke.modifiers.control && !event.keystroke.modifiers.platform {
                        panel.query.push_str(text);
                        query_changed = true;
                    }
                }
            }
        }

        if query_changed {
            panel.results = self.compute_results(panel.mode, &panel.query);
            panel.selected = 0;
        }

        self.switcher = Some(panel);
        cx.notify();
    }
}

fn query_view(rows: Rc<Vec<query::Row>>, theme: Theme) -> AnyElement {
    let count = rows.len();
    let list = uniform_list("query-rows", count, move |range, _window, _cx| {
        range
            .filter_map(|i| rows.get(i).map(|row| (i, row)))
            .map(|(i, row)| {
                let properties = row
                    .values
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
                    .join("  ");

                div()
                    .id(ElementId::Name(format!("query-row-{i}").into()))
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(24.0))
                    .gap_4()
                    .px_3()
                    .text_sm()
                    .child(div().w(px(340.0)).child(row.relative.clone()))
                    .child(properties)
                    .into_any_element()
            })
            .collect::<Vec<AnyElement>>()
    })
    .flex_1()
    .min_h(px(0.0));

    div()
        .flex()
        .flex_col()
        .size_full()
        .py_2()
        .child(
            div()
                .flex()
                .flex_row()
                .gap_4()
                .px_3()
                .py_1()
                .text_xs()
                .text_color(theme.muted)
                .child(
                    div()
                        .w(px(340.0))
                        .text_color(theme.star)
                        .child(format!("{count} rows")),
                )
                .child("note  /  properties"),
        )
        .child(list)
        .into_any_element()
}

fn diff_view(
    title: String,
    summary: String,
    rows: Rc<Vec<diff::Line>>,
    theme: Theme,
) -> AnyElement {
    let count = rows.len();
    let list = uniform_list("diff-rows", count, move |range, _window, _cx| {
        range
            .filter_map(|i| rows.get(i).map(|row| (i, row)))
            .map(|(i, row)| {
                let color = match row.kind {
                    diff::LineKind::Added => theme.code_text,
                    diff::LineKind::Removed => theme.warn,
                    diff::LineKind::Context => theme.text_dim,
                };
                let marker = row.kind.marker().to_string();
                let numbers = format!(
                    "{:>4} {:>4}",
                    row.old_line.unwrap_or(0),
                    row.new_line.unwrap_or(0)
                );

                div()
                    .id(ElementId::Name(format!("diff-row-{i}").into()))
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(20.0))
                    .px_3()
                    .text_sm()
                    .child(div().w(px(76.0)).text_color(theme.muted).child(numbers))
                    .child(div().w(px(16.0)).text_color(color).child(marker))
                    .child(div().flex_1().text_color(color).child(row.text.clone()))
                    .into_any_element()
            })
            .collect::<Vec<AnyElement>>()
    })
    .flex_1()
    .min_h(px(0.0));

    div()
        .flex()
        .flex_col()
        .size_full()
        .py_2()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_4()
                .px_3()
                .py_1()
                .text_xs()
                .text_color(theme.muted)
                .child(div().text_color(theme.star).child(title))
                .child(summary)
                .child(div().flex_1())
                .child("Esc to close"),
        )
        .child(list)
        .into_any_element()
}

/// Fill the standard template variables for a note.
fn render_template(template: &str, root: &Path, relative: Option<&str>, title: &str) -> String {
    let now = chrono::Local::now();
    let date = now.format("%Y-%m-%d").to_string();
    let time = now.format("%H:%M").to_string();
    let weekday = now.format("%A").to_string();
    let vault = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("vault")
        .to_string();
    let folder = relative
        .and_then(|relative| relative.rsplit_once('/'))
        .map(|(folder, _)| folder)
        .unwrap_or("")
        .to_string();

    templates::render(
        template,
        &[
            ("title", title),
            ("date", &date),
            ("time", &time),
            ("weekday", &weekday),
            ("vault", &vault),
            ("folder", &folder),
        ],
    )
}

fn note_display(relative: &str) -> String {
    relative
        .rsplit('/')
        .next()
        .unwrap_or(relative)
        .trim_end_matches(".md")
        .trim_end_matches(".markdown")
        .to_string()
}

fn sanitize_filename(name: &str) -> String {
    let mut out: String = name
        .trim()
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            other => other,
        })
        .collect();
    out = out.trim().trim_end_matches('.').to_string();
    if out.is_empty() {
        "Untitled".to_string()
    } else {
        out
    }
}

fn describe_workspace(workspace: &workspace::Workspace) -> String {
    let mut parts = Vec::new();
    if workspace.graph {
        parts.push("graph");
    }
    if workspace.reading {
        parts.push("reading");
    }
    if let Some(note) = &workspace.note {
        parts.push(note);
    }
    parts.join(" · ")
}

fn git_item(label: &str, detail: &str, action: GitAction) -> SwitcherItem {
    SwitcherItem {
        label: label.to_string(),
        detail: detail.to_string(),
        item: Item::Git(action),
    }
}

fn command_item(label: &str, hint: &str, command: CommandId) -> SwitcherItem {
    SwitcherItem {
        label: label.to_string(),
        // Hints are portable keystrokes; show them the way this platform writes them.
        detail: if hint.is_empty() {
            String::new()
        } else {
            platform::label(hint)
        },
        item: Item::Command(command),
    }
}

fn load_theme(root: &Path, user_themes: &[(String, Theme)]) -> Theme {
    let Ok(text) = std::fs::read_to_string(root.join(".dragon-glass/theme.txt")) else {
        return Theme::dark();
    };
    let value = text.trim();

    if let Some(name) = value.strip_prefix("theme:") {
        return user_themes
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, theme)| *theme)
            .unwrap_or_else(Theme::dark);
    }

    match value {
        "light" => Theme::light(),
        _ => Theme::dark(),
    }
}

/// User themes installed under `.dragon-glass/themes/<id>/theme.txt`.
fn load_user_themes(root: &Path) -> Vec<(String, Theme)> {
    let Ok(entries) = std::fs::read_dir(store::themes_dir(root)) else {
        return Vec::new();
    };

    let mut themes: Vec<(String, Theme)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let fallback = entry.file_name().to_string_lossy().to_string();
            let text = std::fs::read_to_string(entry.path().join("theme.txt")).ok()?;
            let parsed = store::parse_theme(&text, &fallback);
            Some((
                parsed.name.clone(),
                Theme::from_colors(parsed.dark, &parsed.colors),
            ))
        })
        .collect();

    themes.sort_by(|a, b| a.0.cmp(&b.0));
    themes
}

fn save_theme(root: &Path, theme: Theme, name: Option<&str>) -> std::io::Result<()> {
    let dir = root.join(".dragon-glass");
    std::fs::create_dir_all(&dir)?;
    let value = match name {
        Some(name) => format!("theme:{name}"),
        None => theme.name().to_string(),
    };
    storage::write_atomic(&dir.join("theme.txt"), &format!("{value}\n"))
}

fn describe_outcome(outcome: git::Outcome) -> String {
    match outcome {
        git::Outcome::UpToDate => "already up to date".to_string(),
        git::Outcome::Updated => "updated".to_string(),
        git::Outcome::NoRemote => "no remote configured".to_string(),
        git::Outcome::Conflicts(files) => {
            format!(
                "conflicts in {} file(s) — resolve in a terminal",
                files.len()
            )
        }
    }
}

impl Focusable for DragonGlassApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

struct Row {
    depth: usize,
    name: String,
    path: Option<String>,
    is_dir: bool,
    /// A folder whose children are hidden.
    collapsed: bool,
}

fn flatten(node: &TreeNode, depth: usize, collapsed: &HashSet<String>, out: &mut Vec<Row>) {
    for child in &node.children {
        match child.kind {
            NodeKind::Dir => {
                let is_collapsed = collapsed.contains(&child.path);
                out.push(Row {
                    depth,
                    name: child.name.clone(),
                    path: Some(child.path.clone()),
                    is_dir: true,
                    collapsed: is_collapsed,
                });
                if !is_collapsed {
                    flatten(child, depth + 1, collapsed, out);
                }
            }
            NodeKind::Note => out.push(Row {
                depth,
                name: child.name.clone(),
                path: Some(child.path.clone()),
                is_dir: false,
                collapsed: false,
            }),
        }
    }
}

/// The outline headings that are currently visible, as `(index, heading)` pairs.
///
/// A folded heading hides the headings nested under it, up to the next heading of the
/// same or a higher level.
fn visible_headings(
    headings: &[markdown::Heading],
    collapsed: &HashSet<usize>,
) -> Vec<(usize, markdown::Heading)> {
    let mut visible = Vec::new();
    let mut hide_until_level: Option<u8> = None;

    for (index, heading) in headings.iter().enumerate() {
        if let Some(level) = hide_until_level {
            if heading.level <= level {
                hide_until_level = None;
            }
        }
        if hide_until_level.is_some() {
            continue;
        }

        visible.push((index, heading.clone()));
        if collapsed.contains(&index) {
            hide_until_level = Some(heading.level);
        }
    }

    visible
}

fn git_label(status: &Option<git::Status>) -> String {
    let Some(status) = status else {
        return "git: none".to_string();
    };

    let mut parts = vec![format!("git:{}", status.branch)];
    if status.changed + status.untracked > 0 {
        parts.push(format!("\u{00B1}{}", status.changed + status.untracked));
    }
    if status.ahead > 0 {
        parts.push(format!("\u{2191}{}", status.ahead));
    }
    if status.behind > 0 {
        parts.push(format!("\u{2193}{}", status.behind));
    }
    if !status.conflicted.is_empty() {
        parts.push(format!("!{}", status.conflicted.len()));
    }
    parts.join(" ")
}

impl Render for DragonGlassApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.close_requested {
            window.remove_window();
            return div();
        }

        self.expire_status(cx);

        let title = self.desired_window_title(cx);
        if self.shown_title != title {
            window.set_window_title(&title);
            self.shown_title = title;
        }

        let theme = *cx.global::<Theme>();

        // Stats and the outline both need the document text. Fetch it once per change
        // rather than once per consumer: copying a large note is not free on every
        // keystroke.
        let stats_version = self.editor.read(cx).document_version();
        let headings_version = (self.cache_epoch, stats_version.1);
        let stats_stale = stats_version != self.stats_version;
        let headings_stale = headings_version != self.headings_version;
        if stats_stale || headings_stale {
            // Whole-document work (word counts, outline) is throttled to ~7 times a second
            // while typing: the first stale frame arms a timer, the timer repaints once the
            // window has passed, and continuous typing cannot recompute on every keystroke.
            let now = std::time::Instant::now();
            let due = self.derived_at.is_none_or(|last| {
                now.duration_since(last) >= std::time::Duration::from_millis(150)
            });
            if due {
                let text = self.editor.read(cx).text();
                if stats_stale {
                    self.stats = stats::compute(&text);
                    self.stats_version = stats_version;
                }
                if headings_stale {
                    self.headings = Rc::new(markdown::headings(&text));
                    self.headings_version = headings_version;
                }
                self.derived_at = Some(now);
            } else if !self.derived_armed {
                self.derived_armed = true;
                let elapsed = self
                    .derived_at
                    .map(|last| now.duration_since(last))
                    .unwrap_or_default();
                let wait = std::time::Duration::from_millis(150).saturating_sub(elapsed);
                cx.spawn(async move |this, cx| {
                    Timer::after(wait).await;
                    let _ = this.update(cx, |app, cx| {
                        app.derived_armed = false;
                        cx.notify();
                    });
                })
                .detach();
            }
        }

        // A preview pane beside the editor, sharing the same document text.
        let preview: Option<AnyElement> = if self.split_preview
            && self.split_tabs.is_empty()
            && !self.show_settings
            && !self.show_graph
            && !self.show_diff
            && !self.show_query
            && !self.editor.read(cx).is_reading()
        {
            let version = (self.cache_epoch, self.editor.read(cx).document_version().1);
            if self.preview_content.needs_update(version) {
                // Reparsing and re-typesetting the whole note on every keystroke is what
                // makes a split preview stutter, so it catches up ~150 ms after typing
                // stops. The preview shows the previous revision in the meantime.
                let now = std::time::Instant::now();
                // A different note updates at once; edits to the same note wait for the
                // debounce so the previous render stays on screen instead of flickering.
                let due = self.preview_epoch != version.0
                    || self.preview_at.is_none_or(|last| {
                        now.duration_since(last) >= std::time::Duration::from_millis(150)
                    });
                if due {
                    let text = self.editor.read(cx).text();
                    let root = self.vault_root.clone();
                    self.preview_content.update(version, &text, Some(&root));
                    self.preview_at = Some(now);
                    self.preview_epoch = version.0;
                } else if !self.preview_armed {
                    self.preview_armed = true;
                    let elapsed = self
                        .preview_at
                        .map(|last| now.duration_since(last))
                        .unwrap_or_default();
                    let wait = std::time::Duration::from_millis(150).saturating_sub(elapsed);
                    cx.spawn(async move |this, cx| {
                        Timer::after(wait).await;
                        let _ = this.update(cx, |app, cx| {
                            app.preview_armed = false;
                            cx.notify();
                        });
                    })
                    .detach();
                }
            }

            let style = window.text_style();
            let mut template_font = style.font();
            if let Some(reader) = cx.try_global::<ReaderSettings>() {
                if let Some(family) = reader.font.family() {
                    template_font.family = family.into();
                }
            }
            let template = gpui::TextRun {
                len: 0,
                font: template_font,
                color: style.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };

            let opener = self.link_opener(cx);
            let fold: reading::FoldToggle = {
                let weak = cx.entity().downgrade();
                Rc::new(move |index: usize, window: &mut Window, cx: &mut App| {
                    let _ = weak.update(cx, |app, cx| {
                        app.preview_content.toggle_fold(index);
                        cx.notify();
                    });
                    window.refresh();
                })
            };
            Some(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .border_l_1()
                    .border_color(theme.border)
                    .child(self.preview_content.element(
                        Some(self.vault_root.clone()),
                        theme,
                        template,
                        Some(opener),
                        Some(fold),
                        cx,
                    ))
                    .into_any_element(),
            )
        } else {
            None
        };

        // A conflict is resolved once the editor is clean again (saved or reloaded).
        if self.conflict && !self.editor.read(cx).is_dirty() {
            self.conflict = false;
        }

        let rows = self.file_rows.clone();
        let file_count = rows.len();
        let me_for_list = cx.entity();
        let active_for_list = self.active.clone();
        let file_list = uniform_list("file-list", file_count, move |range, _window, _cx| {
            range
                .filter_map(|i| rows.get(i).map(|row| (i, row)))
                .map(|(i, row)| {
                    let indent = px(8.0 + row.depth as f32 * 12.0);
                    let is_active =
                        !row.is_dir && row.path.as_deref() == active_for_list.as_deref();
                    let color = if row.is_dir {
                        theme.text_dim
                    } else if is_active {
                        theme.accent_text
                    } else {
                        theme.text
                    };

                    let mut base = div()
                        .id(ElementId::Name(format!("row-{i}").into()))
                        .flex()
                        .items_center()
                        .gap_1()
                        .h(px(24.0))
                        .w_full()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .rounded_sm()
                        .pl(indent)
                        .pr_2()
                        .text_sm()
                        .cursor_pointer()
                        .text_color(color);

                    if row.is_dir {
                        base = base
                            .font_weight(gpui::FontWeight::BOLD)
                            .child(
                                div()
                                    .w(px(12.0))
                                    .flex_shrink_0()
                                    .text_color(theme.muted)
                                    .child(if row.collapsed {
                                        "\u{25B8}"
                                    } else {
                                        "\u{25BE}"
                                    }),
                            )
                            .child(row.name.clone());
                    } else {
                        base = base.child(row.name.clone());
                    }

                    if is_active {
                        base = base.bg(theme.accent);
                    }

                    if row.is_dir {
                        let path = row.path.clone().unwrap_or_default();
                        let menu_path = path.clone();
                        let me = me_for_list.clone();
                        let menu_me = me_for_list.clone();
                        base.hover(move |style| style.bg(theme.hover))
                            .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                                me.update(cx, |this, cx| this.toggle_dir(&path, cx));
                            })
                            .on_mouse_down(MouseButton::Right, move |_ev, window, cx| {
                                cx.stop_propagation();
                                menu_me.update(cx, |this, cx| {
                                    this.open_tree_menu(menu_path.clone(), true, window, cx);
                                });
                            })
                            .into_any_element()
                    } else {
                        match &row.path {
                            Some(path) => {
                                let path = path.clone();
                                let menu_path = path.clone();
                                let me = me_for_list.clone();
                                let menu_me = me_for_list.clone();
                                base.hover(move |style| style.bg(theme.hover))
                                    .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                                        me.update(cx, |this, cx| {
                                            this.open_note(&path, window, cx);
                                        });
                                    })
                                    .on_mouse_down(MouseButton::Right, move |_ev, window, cx| {
                                        cx.stop_propagation();
                                        menu_me.update(cx, |this, cx| {
                                            this.open_tree_menu(
                                                menu_path.clone(),
                                                false,
                                                window,
                                                cx,
                                            );
                                        });
                                    })
                                    .into_any_element()
                            }
                            None => base.into_any_element(),
                        }
                    }
                })
                .collect::<Vec<AnyElement>>()
        })
        .flex_1()
        .min_h(px(0.0));

        let me = cx.entity();

        let bookmark_rows: Vec<AnyElement> = self
            .bookmarks
            .iter()
            .enumerate()
            .map(|(i, relative)| {
                let relative = relative.clone();
                let me = me.clone();
                let name = relative
                    .rsplit('/')
                    .next()
                    .unwrap_or(&relative)
                    .trim_end_matches(".md")
                    .to_string();
                div()
                    .id(ElementId::Name(format!("bookmark-{i}").into()))
                    .flex()
                    .items_center()
                    .h(px(24.0))
                    .w_full()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .rounded_sm()
                    .pl(px(16.0))
                    .pr_2()
                    .text_sm()
                    .cursor_pointer()
                    .text_color(theme.star)
                    .hover(move |style| style.bg(theme.hover))
                    .child(format!("\u{2605} {name}"))
                    .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                        me.update(cx, |this, cx| {
                            this.open_note(&relative, window, cx);
                        });
                    })
                    .into_any_element()
            })
            .collect();

        let sidebar = div()
            .id("sidebar")
            .on_drop::<TabDrag>(cx.listener(Self::drop_on_window))
            .drag_over::<TabDrag>(move |style, _drag, _window, _cx| {
                style.bg(theme.accent.opacity(0.25))
            })
            .flex()
            .flex_col()
            .w(px(self.sidebar_width))
            .h_full()
            .bg(theme.panel)
            .border_r_1()
            .border_color(theme.border)
            .py_2()
            .when(!bookmark_rows.is_empty(), move |d| {
                d.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(theme.muted)
                        .child("BOOKMARKS"),
                )
                .children(bookmark_rows)
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .pt_2()
                    .pb_1()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(
                        div().flex().items_center().gap_2().child("FILES").child(
                            div()
                                .id("new-note-button")
                                .px_1()
                                .rounded_sm()
                                .cursor_pointer()
                                .hover(move |style| style.bg(theme.hover).text_color(theme.text))
                                .child("+")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _ev, window, cx| {
                                        let dir = this
                                            .active
                                            .as_deref()
                                            .and_then(|active| active.rsplit_once('/'))
                                            .map(|(dir, _)| dir.to_string())
                                            .unwrap_or_default();
                                        this.pending_dir = Some(dir);
                                        this.pending_template = None;
                                        this.open_panel(SwitcherMode::NewNoteName, window, cx);
                                    }),
                                ),
                        ),
                    )
                    .child(format!("{}", self.notes)),
            )
            .child(file_list);

        let headings = self.headings.clone();
        // Highlight the section the reader is in.
        let reading_heading = if self.editor.read(cx).is_reading() {
            self.editor.read(cx).reading_outline_state().0
        } else {
            None
        };

        let outline: Option<AnyElement> = if headings.is_empty() {
            None
        } else {
            let me = cx.entity();
            let outline_rows: Vec<(usize, markdown::Heading, bool, bool)> =
                visible_headings(&headings, &self.collapsed_outline)
                    .into_iter()
                    .map(|(index, heading)| {
                        let foldable = headings
                            .get(index + 1)
                            .is_some_and(|next| next.level > heading.level);
                        let collapsed = self.collapsed_outline.contains(&index);
                        (index, heading, foldable, collapsed)
                    })
                    .collect();
            let row_count = outline_rows.len();
            // Keep the active section visible as the reader scrolls.
            if reading_heading != self.outline_current {
                self.outline_current = reading_heading.clone();
                if let Some(name) = &reading_heading {
                    if let Some(row) = outline_rows
                        .iter()
                        .position(|(_, heading, _, _)| &heading.text == name)
                    {
                        self.outline_scroll
                            .scroll_to_item(row, ScrollStrategy::Center);
                    }
                }
            }
            let current_heading = reading_heading.clone();
            let rows = uniform_list("outline-list", row_count, move |range, _window, _cx| {
                range
                    .filter_map(|i| outline_rows.get(i))
                    .map(|(index, heading, foldable, collapsed)| {
                        let index = *index;
                        let foldable = *foldable;
                        let collapsed = *collapsed;
                        let indent = px(8.0 + heading.level.saturating_sub(1) as f32 * 10.0);
                        let line = heading.line;
                        let heading_text = heading.text.clone();
                        let is_current = current_heading.as_deref() == Some(heading.text.as_str());
                        let me = me.clone();

                        let mut chevron = div()
                            .id(ElementId::Name(format!("outline-fold-{index}").into()))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(14.0))
                            .flex_shrink_0()
                            .text_color(theme.muted)
                            .child(if foldable {
                                if collapsed {
                                    "\u{25B8}"
                                } else {
                                    "\u{25BE}"
                                }
                            } else {
                                ""
                            });
                        if foldable {
                            let toggle = me.clone();
                            chevron = chevron
                                .cursor_pointer()
                                .hover(move |style| style.text_color(theme.text))
                                .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                                    cx.stop_propagation();
                                    toggle.update(cx, |this, cx| this.toggle_outline(index, cx));
                                });
                        }

                        div()
                            .id(ElementId::Name(format!("outline-{index}").into()))
                            .flex()
                            .items_center()
                            .gap_1()
                            .h(px(24.0))
                            .mx_1()
                            .rounded_sm()
                            .pl(indent)
                            .pr_2()
                            .text_sm()
                            .cursor_pointer()
                            .text_color(if is_current {
                                theme.accent_text
                            } else {
                                theme.text
                            })
                            .when(is_current, |row| row.bg(theme.chip))
                            .hover(move |style| style.bg(theme.hover))
                            .child(chevron)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .child(heading.text.clone()),
                            )
                            .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                                me.update(cx, |this, cx| {
                                    this.reveal_outline_heading(index);
                                    this.remember_nav(cx);
                                    this.editor.update(cx, |editor, cx| {
                                        editor.jump_to_heading(
                                            line.saturating_sub(1),
                                            &heading_text,
                                            index,
                                        );
                                        cx.notify();
                                    });
                                    window.focus(&this.editor.focus_handle(cx));
                                    cx.notify();
                                });
                            })
                            .into_any_element()
                    })
                    .collect::<Vec<AnyElement>>()
            })
            .flex_1()
            .min_h(px(0.0))
            .track_scroll(self.outline_scroll.clone());

            Some(
                div()
                    .id("outline")
                    .on_drop::<TabDrag>(cx.listener(Self::drop_on_window))
                    .flex()
                    .flex_col()
                    .w(px(self.outline_width))
                    .h_full()
                    .bg(theme.panel)
                    .border_l_1()
                    .border_color(theme.border)
                    .py_2()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(theme.muted)
                            .child("TABLE OF CONTENTS"),
                    )
                    .child(rows)
                    .into_any_element(),
            )
        };

        let relations_panel: Option<AnyElement> = if self.show_relations {
            let me = cx.entity();
            let active = self.active.clone();
            let entries = active
                .as_deref()
                .and_then(|relative| relations::for_note(&self.relations, relative));

            let mut rows: Vec<AnyElement> = Vec::new();
            let section = |title: &str, rows: &mut Vec<AnyElement>| {
                rows.push(
                    div()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .text_color(theme.muted)
                        .child(title.to_uppercase())
                        .into_any_element(),
                );
            };

            if let Some(entries) = entries {
                if !entries.outgoing.is_empty() {
                    section("Relations", &mut rows);
                    for relation in entries.outgoing.iter().take(30) {
                        let target = relation.target.clone();
                        let me = me.clone();
                        let label = note_display(&target);
                        let detail = match &relation.label {
                            Some(label) => format!("{} · {label}", relation.kind.label()),
                            None => relation.kind.label().to_string(),
                        };
                        rows.push(
                            div()
                                .id(ElementId::Name(
                                    format!("relation-{}-{}", relation.kind.label(), target).into(),
                                ))
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .h(px(24.0))
                                .mx_1()
                                .rounded_sm()
                                .pl(px(14.0))
                                .pr_2()
                                .text_sm()
                                .cursor_pointer()
                                .text_color(theme.text)
                                .hover(move |style| style.bg(theme.hover))
                                .child(div().child(label))
                                .child(div().text_xs().text_color(theme.muted).child(detail))
                                .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                                    me.update(cx, |this, cx| {
                                        this.open_note(&target, window, cx);
                                    });
                                })
                                .into_any_element(),
                        );
                    }
                }

                if !entries.incoming.is_empty() {
                    section("Backlinks", &mut rows);
                    for relation in entries.incoming.iter().take(30) {
                        let target = relation.target.clone();
                        let me = me.clone();
                        let label = note_display(&target);
                        let detail = relation.kind.label();
                        rows.push(
                            div()
                                .id(ElementId::Name(
                                    format!("backlink-{}-{}", relation.kind.label(), target).into(),
                                ))
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .h(px(24.0))
                                .mx_1()
                                .rounded_sm()
                                .pl(px(14.0))
                                .pr_2()
                                .text_sm()
                                .cursor_pointer()
                                .text_color(theme.text)
                                .hover(move |style| style.bg(theme.hover))
                                .child(div().child(label))
                                .child(div().text_xs().text_color(theme.muted).child(detail))
                                .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                                    me.update(cx, |this, cx| {
                                        this.open_note(&target, window, cx);
                                    });
                                })
                                .into_any_element(),
                        );
                    }
                }

                if !entries.shared_tags.is_empty() {
                    section("Shared tags", &mut rows);
                    for (tag, notes) in entries.shared_tags.iter().take(20) {
                        let others: Vec<String> = notes
                            .iter()
                            .take(3)
                            .map(|note| note_display(note))
                            .collect();
                        rows.push(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .h(px(24.0))
                                .mx_1()
                                .rounded_sm()
                                .pl(px(14.0))
                                .pr_2()
                                .text_sm()
                                .text_color(theme.text_dim)
                                .child(format!("#{tag}"))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted)
                                        .child(others.join(", ")),
                                )
                                .into_any_element(),
                        );
                    }
                }
            }

            if rows.is_empty() {
                rows.push(
                    div()
                        .px_3()
                        .py_2()
                        .text_sm()
                        .text_color(theme.muted)
                        .child(if active.is_some() {
                            "no relations yet"
                        } else {
                            "no note open"
                        })
                        .into_any_element(),
                );
            }

            Some(
                div()
                    .id("relations")
                    .on_drop::<TabDrag>(cx.listener(Self::drop_on_window))
                    .flex()
                    .flex_col()
                    .w(px(self.relations_width))
                    .h_full()
                    .bg(theme.panel)
                    .border_l_1()
                    .border_color(theme.border)
                    .py_2()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_2()
                            .pb_1()
                            .text_xs()
                            .text_color(theme.muted)
                            .child("DOCUMENT RELATIONS")
                            .child("esc"),
                    )
                    .children(rows)
                    .into_any_element(),
            )
        } else {
            None
        };

        let panel: Option<AnyElement> = self.switcher.as_ref().map(|panel| {
            let prefix = match panel.mode {
                SwitcherMode::QuickSwitch => ">",
                SwitcherMode::Search => "find:",
                SwitcherMode::GitMenu => "git:",
                SwitcherMode::CommitInput => "commit:",
                SwitcherMode::PropertiesMenu => "properties:",
                SwitcherMode::PropertyInput => "value:",
                SwitcherMode::CommandsMenu => "command:",
                SwitcherMode::QueryInput => "query:",
                SwitcherMode::WorkspaceName => "workspace name:",
                SwitcherMode::WorkspaceMenu => "workspace:",
                SwitcherMode::HotkeyMenu => "shortcut:",
                SwitcherMode::HotkeyCapture => "press a key:",
                SwitcherMode::GraphFilterInput => "graph filter:",
                SwitcherMode::VersionMenu => "versions:",
                SwitcherMode::TemplateMenu => "template:",
                SwitcherMode::TemplateNewMenu => "new note template:",
                SwitcherMode::SharedMenu => "shared block:",
                SwitcherMode::SharedEditMenu => "edit shared block:",
                SwitcherMode::ReviewMenu => "review:",
                SwitcherMode::NewNoteName => {
                    if self.pending_save_as {
                        "save as:"
                    } else {
                        "note name:"
                    }
                }
                SwitcherMode::SharedName => "shared block name:",
                SwitcherMode::RelationInput => "related note:",
                SwitcherMode::NewFolderName => "folder name:",
                SwitcherMode::RenameInput => "rename to:",
                SwitcherMode::DeleteConfirm => "delete:",
                SwitcherMode::FileMenu => "file:",
                SwitcherMode::HistoryMenu => "history:",
                SwitcherMode::SettingInput => "setting:",
            };
            let rows = panel.results.iter().enumerate().map(|(i, item)| {
                let mut row = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .h(px(28.0))
                    .px_3()
                    .text_sm()
                    .child(item.label.clone())
                    .child(div().text_color(theme.muted).child(item.detail.clone()));
                if i == panel.selected {
                    row = row.bg(theme.accent);
                } else {
                    row = row.hover(move |style| style.bg(theme.hover));
                }
                row
            });

            div()
                .id("switcher")
                .key_context("Switcher")
                .track_focus(&self.switcher_focus)
                .on_key_down(cx.listener(Self::on_switcher_key))
                .flex()
                .flex_col()
                .bg(theme.input_bg)
                .border_b_1()
                .border_color(theme.border)
                .max_h(px(320.0))
                .overflow_y_scroll()
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(theme.border)
                        .text_color(theme.text)
                        .child(format!("{prefix} {}", panel.query)),
                )
                .children(rows)
                .into_any_element()
        });

        let quit_bar = self.quit_prompt.then(|| self.quit_prompt_bar(theme, cx));

        let tab_host = cx.entity();
        let drop_overlay = if cx.has_active_drag() {
            Some(self.tab_drop_overlay(theme, cx))
        } else {
            None
        };
        let center = div()
            .flex_1()
            // `flex_1` alone lets the outline panel keep its width; `w_full` would make
            // this pane claim the whole row and push the outline off-screen.
            .min_w(px(0.0))
            .relative()
            .child(self.center_view(theme, preview, cx))
            .children(drop_overlay);

        if self.minimal {
            let minimal_title = self
                .tabs
                .get(self.active_tab)
                .map(|tab| tab.title.clone())
                .unwrap_or_default();

            return div()
                .flex()
                .flex_col()
                .size_full()
                .bg(theme.bg)
                .text_color(theme.text)
                .on_drop::<TabDrag>(cx.listener(Self::drop_on_window))
                .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_drag_release_out))
                .on_mouse_move(cx.listener(Self::clear_stuck_drag_move))
                .on_mouse_down(MouseButton::Left, cx.listener(Self::clear_stuck_drag_down))
                .on_mouse_down(
                    MouseButton::Navigate(NavigationDirection::Back),
                    cx.listener(Self::nav_button),
                )
                .on_mouse_down(
                    MouseButton::Navigate(NavigationDirection::Forward),
                    cx.listener(Self::nav_button),
                )
                .on_action(cx.listener(Self::open_switcher))
                .on_action(cx.listener(Self::open_search))
                .on_action(cx.listener(Self::open_git))
                .on_action(cx.listener(Self::open_relations))
                .on_action(cx.listener(Self::open_daily))
                .on_action(cx.listener(Self::open_weekly))
                .on_action(cx.listener(Self::open_monthly))
                .on_action(cx.listener(Self::open_properties))
                .on_action(cx.listener(Self::toggle_bookmark))
                .on_action(cx.listener(Self::open_commands))
                .on_action(cx.listener(Self::toggle_theme))
                .on_action(cx.listener(Self::toggle_vim_action))
                .on_action(cx.listener(Self::close_overlay))
                .on_action(cx.listener(Self::new_window))
                .on_action(cx.listener(Self::toggle_side_by_side_action))
                .on_action(cx.listener(Self::detach_tab_action))
                .on_action(cx.listener(Self::toggle_split_editor_action))
                .on_action(cx.listener(Self::close_tab_action))
                .on_action(cx.listener(Self::next_tab))
                .on_action(cx.listener(Self::prev_tab))
                .children(panel)
                .child(
                    div()
                        .id("minimal-tab")
                        .flex()
                        .items_center()
                        .h(px(28.0))
                        .px_3()
                        .bg(theme.header)
                        .border_b_1()
                        .border_color(theme.border)
                        .text_sm()
                        .text_color(theme.muted)
                        .child(minimal_title.clone())
                        .child(div().flex_1())
                        .on_drag(
                            TabDrag {
                                source: cx.entity().downgrade(),
                                index: self.active_tab,
                                title: minimal_title.clone(),
                            },
                            |drag, _offset, _window, cx| {
                                let _ = drag.source.update(cx, |app, cx| {
                                    app.dragging = Some(drag.clone());
                                    cx.notify();
                                });
                                cx.new(|_| DragGhost {
                                    title: drag.title.clone(),
                                })
                            },
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(center),
                );
        }

        // A conflict marker clears itself once the note is saved or reverted: the buffer
        // then matches the disk again, so there is nothing left to warn about.
        for tab in &mut self.tabs {
            if tab.conflict && !tab.editor.read(cx).is_dirty() {
                tab.conflict = false;
            }
        }

        // A dirty buffer whose file changed on disk gets an explicit choice instead of a
        // silent overwrite: reload, keep mine, compare, or save a copy.
        let dirty_active = self
            .tabs
            .get(self.active_tab)
            .is_some_and(|tab| tab.editor.read(cx).is_dirty());
        let conflict_bar = if self.conflict && dirty_active {
            let button = |id: &'static str, label: &'static str| {
                div()
                    .id(id)
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(theme.chip)
                    .text_xs()
                    .text_color(theme.text)
                    .cursor_pointer()
                    .hover(move |style| style.bg(theme.hover))
                    .child(label)
            };
            let deleted = self
                .active
                .as_ref()
                .is_some_and(|relative| !self.vault_root.join(relative).exists());
            Some(
                div()
                    .id("conflict-bar")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .h(px(36.0))
                    .px_3()
                    .flex_shrink_0()
                    .bg(theme.panel)
                    .border_b_1()
                    .border_color(theme.error)
                    .text_sm()
                    .child(div().text_color(theme.error).child(if deleted {
                        "This note was deleted on disk."
                    } else {
                        "This note changed on disk."
                    }))
                    .child(button("conflict-reload", "Reload").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _event, window, cx| {
                            this.reload_conflicted(&ReloadConflicted, window, cx);
                        }),
                    ))
                    .child(button("conflict-keep", "Keep mine").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _event, window, cx| {
                            this.keep_mine(&KeepMine, window, cx);
                        }),
                    ))
                    .child(button("conflict-compare", "Compare").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _event, window, cx| {
                            this.compare_conflicted(&CompareConflicted, window, cx);
                        }),
                    ))
                    .child(button("conflict-copy", "Save as copy").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _event, window, cx| {
                            this.save_conflict_copy(&SaveConflictCopy, window, cx);
                        }),
                    )),
            )
        } else {
            None
        };

        // A draft that differs from the note on disk is offered back rather than applied
        // silently.
        let draft_bar = self.pending_draft.as_ref().map(|_| {
            let button = |id: &'static str, label: &'static str| {
                div()
                    .id(id)
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(theme.chip)
                    .text_xs()
                    .text_color(theme.text)
                    .cursor_pointer()
                    .hover(move |style| style.bg(theme.hover))
                    .child(label)
            };
            div()
                .id("draft-bar")
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .h(px(36.0))
                .px_3()
                .flex_shrink_0()
                .bg(theme.panel)
                .border_b_1()
                .border_color(theme.warn)
                .text_sm()
                .child(
                    div()
                        .text_color(theme.warn)
                        .child("An unsaved draft from an earlier session differs from this note."),
                )
                .child(button("draft-recover", "Recover").on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _event, window, cx| {
                        this.recover_draft(&RecoverDraft, window, cx);
                    }),
                ))
                .child(button("draft-compare", "Compare").on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _event, window, cx| {
                        this.compare_draft(&CompareDraft, window, cx);
                    }),
                ))
                .child(button("draft-discard", "Discard").on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _event, window, cx| {
                        this.discard_draft(&DiscardDraft, window, cx);
                    }),
                ))
        });

        let tab_bar = div()
            .id("tab-bar")
            .on_drop::<TabDrag>(cx.listener(Self::drop_on_tab_bar))
            .drag_over::<TabDrag>(move |style, _drag, _window, _cx| style.bg(theme.accent))
            .flex()
            .flex_row()
            .items_center()
            .h(px(34.0))
            .bg(theme.header)
            .border_b_1()
            .border_color(theme.border)
            .overflow_x_scroll()
            .children(self.tabs.iter().enumerate().map(|(index, tab)| {
                let is_active = index == self.active_tab;
                let is_secondary = self.split_tabs.contains(&index);
                let dirty = tab.editor.read(cx).is_dirty();
                let title = tab.title.clone();
                let activate = tab_host.clone();
                let close = tab_host.clone();
                let middle = tab_host.clone();

                div()
                    .id(ElementId::Name(format!("tab-{index}").into()))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(34.0))
                    .pl_3()
                    .pr_1()
                    .cursor_pointer()
                    .border_r_1()
                    .border_color(theme.border)
                    .text_sm()
                    .bg(if is_active {
                        theme.bg
                    } else if is_secondary {
                        theme.chip
                    } else {
                        theme.header
                    })
                    .text_color(if is_active {
                        theme.text
                    } else if is_secondary {
                        theme.star
                    } else {
                        theme.muted
                    })
                    .when(!is_active, move |d| {
                        d.hover(move |style| style.bg(theme.hover).text_color(theme.text))
                    })
                    .child(title.clone())
                    .when(dirty, |d| {
                        d.child(div().text_color(theme.warn).child("\u{25CF}"))
                    })
                    .when(tab.conflict, |d| {
                        d.child(div().text_color(theme.error).child("\u{26A0}"))
                    })
                    .on_drag(
                        TabDrag {
                            source: cx.entity().downgrade(),
                            index,
                            title: title.clone(),
                        },
                        |drag, _offset, _window, cx| {
                            let _ = drag.source.update(cx, |app, cx| {
                                app.dragging = Some(drag.clone());
                                cx.notify();
                            });
                            cx.new(|_| DragGhost {
                                title: drag.title.clone(),
                            })
                        },
                    )
                    .child(
                        div()
                            .id(ElementId::Name(format!("tab-close-{index}").into()))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(18.0))
                            .h(px(18.0))
                            .rounded_sm()
                            .text_color(theme.muted)
                            .hover(move |style| style.bg(theme.hover).text_color(theme.warn))
                            .child("\u{00D7}")
                            .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                                cx.stop_propagation();
                                close.update(cx, |this, cx| this.close_tab(index, window, cx));
                            })
                            .on_click(move |_ev, _window, cx| cx.stop_propagation()),
                    )
                    .on_click(move |_ev, window, cx| {
                        activate.update(cx, |this, cx| {
                            this.activate_tab(index, cx);
                            let handle = this.editor.focus_handle(cx);
                            window.focus(&handle);
                        });
                    })
                    .on_mouse_down(MouseButton::Middle, move |_ev, window, cx| {
                        cx.stop_propagation();
                        middle.update(cx, |this, cx| this.close_tab(index, window, cx));
                    })
                    .into_any_element()
            }))
            .child(
                div()
                    .id("tab-overflow")
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(28.0))
                    .h(px(34.0))
                    .text_color(theme.muted)
                    .cursor_pointer()
                    .hover(move |style| style.bg(theme.hover).text_color(theme.text))
                    .child("\u{25BE}")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                            cx.stop_propagation();
                            this.tab_menu = Some(event.position);
                            cx.notify();
                        }),
                    ),
            );

        // The tab bar scrolls horizontally; the overflow menu lists every tab so a tab
        // scrolled out of view is still reachable.
        let tab_menu = self.tab_menu.map(|position| {
            let rows: Vec<AnyElement> = self
                .tabs
                .iter()
                .enumerate()
                .map(|(index, tab)| {
                    let title = tab.title.clone();
                    let dirty = tab.editor.read(cx).is_dirty();
                    let active = index == self.active_tab;
                    let host = tab_host.clone();
                    div()
                        .id(ElementId::Name(format!("tab-menu-{index}").into()))
                        .flex()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .px_3()
                        .py_1()
                        .text_sm()
                        .cursor_pointer()
                        .when(active, |d| d.bg(theme.chip).text_color(theme.text))
                        .when(!active, |d| d.text_color(theme.muted))
                        .hover(move |style| style.bg(theme.hover).text_color(theme.text))
                        .child(title)
                        .when(dirty, |d| {
                            d.child(div().text_color(theme.warn).child("\u{25CF}"))
                        })
                        .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                            host.update(cx, |this, cx| {
                                this.tab_menu = None;
                                this.activate_tab(index, cx);
                                let handle = this.editor.focus_handle(cx);
                                window.focus(&handle);
                            });
                        })
                        .into_any_element()
                })
                .collect();

            anchored()
                .position(position)
                .anchor(Corner::TopLeft)
                .snap_to_window_with_margin(px(8.0))
                .child(
                    div()
                        .id("tab-menu")
                        .flex()
                        .flex_col()
                        .w(px(240.0))
                        .max_h(px(320.0))
                        .overflow_y_scroll()
                        .py_1()
                        .rounded_md()
                        .bg(theme.panel)
                        .border_1()
                        .border_color(theme.border)
                        .text_color(theme.text)
                        .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                            cx.stop_propagation();
                        })
                        .children(rows),
                )
                .into_any_element()
        });

        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg)
            .text_color(theme.text)
            .on_drop::<TabDrag>(cx.listener(Self::drop_on_window))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_drag_release_out))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::finish_resize))
            .on_mouse_move(cx.listener(Self::clear_stuck_drag_move))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::clear_stuck_drag_down))
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Back),
                cx.listener(Self::nav_button),
            )
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Forward),
                cx.listener(Self::nav_button),
            )
            .on_action(cx.listener(Self::open_switcher))
            .on_action(cx.listener(Self::open_search))
            .on_action(cx.listener(Self::open_git))
            .on_action(cx.listener(Self::open_relations))
            .on_action(cx.listener(Self::open_daily))
            .on_action(cx.listener(Self::open_weekly))
            .on_action(cx.listener(Self::open_monthly))
            .on_action(cx.listener(Self::open_properties))
            .on_action(cx.listener(Self::toggle_bookmark))
            .on_action(cx.listener(Self::open_commands))
            .on_action(cx.listener(Self::open_settings_action))
            .on_action(cx.listener(Self::toggle_theme))
            .on_action(cx.listener(Self::toggle_vim_action))
            .on_action(cx.listener(Self::close_overlay))
            .on_action(cx.listener(Self::new_window))
            .on_action(cx.listener(Self::toggle_side_by_side_action))
            .on_action(cx.listener(Self::detach_tab_action))
            .on_action(cx.listener(Self::toggle_split_editor_action))
            .on_action(cx.listener(Self::close_tab_action))
            .on_action(cx.listener(Self::quit_action))
            .on_action(cx.listener(Self::reopen_closed_tab_action))
            .on_action(cx.listener(Self::save_as_action))
            .on_action(cx.listener(Self::reload_conflicted))
            .on_action(cx.listener(Self::keep_mine))
            .on_action(cx.listener(Self::compare_conflicted))
            .on_action(cx.listener(Self::save_conflict_copy))
            .on_action(cx.listener(Self::recover_draft))
            .on_action(cx.listener(Self::compare_draft))
            .on_action(cx.listener(Self::discard_draft))
            .on_action(cx.listener(Self::toggle_zen))
            .on_action(cx.listener(Self::nav_back))
            .on_action(cx.listener(Self::nav_forward))
            .on_action(cx.listener(Self::copy_selection))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .h(px(40.0))
                    .px_4()
                    .bg(theme.header)
                    .border_b_1()
                    .border_color(theme.border)
                    .text_sm()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(theme.text)
                            .child("Dragon Glass"),
                    )
                    .child(div().text_color(theme.muted).child(self.vault_path.clone()))
                    .child(
                        div()
                            .text_color(theme.text_dim)
                            .child(git_label(&self.git_status)),
                    )
                    .child(
                        div()
                            .text_color(theme.muted)
                            .child(format!("{} notes", self.notes)),
                    )
                    .child(
                        div()
                            .text_color(theme.muted)
                            .child(format!("{} links", self.links)),
                    )
                    .child(div().text_color(theme.muted).child(format!(
                        "{} words · {} min",
                        self.stats.words, self.stats.reading_minutes
                    )))
                    .when(self.conflict, |d| {
                        d.child(div().text_color(theme.warn).child("changed on disk"))
                    })
                    .when_some(self.update_available.clone(), |d, latest| {
                        d.child(
                            div()
                                .text_color(theme.star)
                                .child(format!("update available: {latest}")),
                        )
                    })
                    .child(div().flex_1())
                    .when_some(self.status.clone(), |d, status| {
                        d.child(
                            div()
                                .id("status-line")
                                .max_w(px(520.0))
                                .overflow_hidden()
                                .text_color(match status.kind {
                                    StatusKind::Info => theme.muted,
                                    StatusKind::Success => theme.text_dim,
                                    StatusKind::Warn => theme.warn,
                                    StatusKind::Error => theme.error,
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.clear_status();
                                    cx.notify();
                                }))
                                .child(status.text),
                        )
                    }),
            )
            .children(panel)
            .children(quit_bar)
            .children(conflict_bar)
            .children(draft_bar)
            .when(!self.zen, |d| d.child(tab_bar))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .when(!self.zen && self.config.show_sidebar, |d| {
                        d.child(sidebar)
                            .child(resize_handle(ResizePanel::Sidebar, theme, cx))
                    })
                    .child(center)
                    .children(if self.zen || !self.config.show_outline {
                        None
                    } else {
                        outline.map(|panel| {
                            let handle = resize_handle(ResizePanel::Outline, theme, cx);
                            div()
                                .flex()
                                .flex_row()
                                .h_full()
                                .child(handle)
                                .child(panel)
                                .into_any_element()
                        })
                    })
                    .children(if self.zen || !self.config.show_relations {
                        None
                    } else {
                        relations_panel.map(|panel| {
                            let handle = resize_handle(ResizePanel::Relations, theme, cx);
                            div()
                                .flex()
                                .flex_row()
                                .h_full()
                                .child(handle)
                                .child(panel)
                                .into_any_element()
                        })
                    }),
            )
            .when(self.tab_menu.is_some(), |d| {
                d.child(
                    div()
                        .id("tab-menu-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _event, _window, cx| {
                                this.tab_menu = None;
                                cx.notify();
                            }),
                        ),
                )
            })
            .children(tab_menu)
    }
}

/// Build the macOS menu bar. The menu items dispatch the same actions as the shortcuts,
/// so the two can never disagree.
#[cfg(target_os = "macos")]
fn install_menus(cx: &mut App) {
    use gpui::{Menu, MenuItem, OsAction, SystemMenuType};

    let menus = vec![
        Menu {
            name: "Dragon Glass".into(),
            items: vec![
                MenuItem::action("Settings\u{2026}", OpenSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Dragon Glass", HideApp),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Dragon Glass", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Window", NewWindow),
                MenuItem::action("Move Tab to New Window", DetachTab),
                MenuItem::action("Close Tab", CloseTab),
                MenuItem::separator(),
                MenuItem::action("Save", editor::Save),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", editor::Undo, OsAction::Undo),
                MenuItem::os_action("Redo", editor::Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", editor::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", editor::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", editor::Paste, OsAction::Paste),
                MenuItem::os_action("Select All", editor::SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Find", editor::OpenFind),
                MenuItem::action("Find and Replace", editor::OpenFindReplace),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Reading Mode", editor::ToggleReading),
                MenuItem::action("Toggle Theme", ToggleTheme),
                MenuItem::separator(),
                MenuItem::action("Graph View", OpenGraph),
                MenuItem::action("Document Relations", OpenRelations),
                MenuItem::separator(),
                MenuItem::action("Read and Write Side by Side", ToggleSideBySide),
                MenuItem::action("Two Editors Side by Side", ToggleSplitEditor),
            ],
        },
        Menu {
            name: "Go".into(),
            items: vec![
                MenuItem::action("Command Palette", OpenCommands),
                MenuItem::action("Quick Switcher", OpenSwitcher),
                MenuItem::action("Search", OpenSearch),
                MenuItem::separator(),
                MenuItem::action("Daily Note", OpenDaily),
                MenuItem::action("Weekly Note", OpenWeekly),
                MenuItem::action("Monthly Note", OpenMonthly),
            ],
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Next Tab", NextTab),
                MenuItem::action("Previous Tab", PrevTab),
            ],
        },
    ];

    cx.set_menus(menus);
}

/// Handle `dragonglass://` URLs delivered by Launch Services while the app runs, and ask
/// the system to route the scheme here when the app is installed as a bundle.
#[cfg(target_os = "macos")]
fn install_url_handler(cx: &mut App, urls: smol::channel::Receiver<Vec<String>>) {
    // The platform callback has no app handle, so it feeds a channel and a foreground
    // task does the work. Waiting on a channel costs nothing while idle.
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        while let Ok(batch) = urls.recv().await {
            for url in batch {
                open_uri(&url, cx);
            }
        }
    })
    .detach();

    // This fails for a binary run outside a bundle, which is fine: argv still works.
    cx.register_url_scheme("dragonglass").detach();
}

/// Open the vault and note named by a `dragonglass://` URL.
#[cfg(target_os = "macos")]
fn open_uri(url: &str, app: &gpui::AsyncApp) {
    let Some(parsed) = uri::parse(url) else {
        return;
    };
    let vault = parsed.vault.map(PathBuf::from);
    let note = parsed.note;

    let _ = app.update(move |cx| {
        // A cold launch through the URL scheme shows the chooser first; replace it.
        close_vault_picker(cx);
        open_request(vault, note, cx);
    });
}

thread_local! {
    /// The last "open a vault" window. A `dragonglass://` URL that arrives after launch
    /// closes it, so the app shows the requested vault instead of two windows.
    static VAULT_PICKER: RefCell<Option<gpui::WindowHandle<vault_picker::VaultPicker>>> =
        const { RefCell::new(None) };
}

/// Show the vault chooser and remember its window.
fn open_vault_picker(cx: &mut App, error: Option<String>, open: vault_picker::OpenVault) {
    let window = vault_picker::VaultPicker::open(cx, error, open);
    VAULT_PICKER.with(|slot| *slot.borrow_mut() = Some(window));
}

/// Close the chooser, if one is open.
/// Close the chooser when a `dragonglass://` URL replaces it.
#[cfg(target_os = "macos")]
fn close_vault_picker(cx: &mut App) {
    let Some(window) = VAULT_PICKER.with(|slot| slot.borrow_mut().take()) else {
        return;
    };
    let _ = window.update(cx, |_picker, window, _cx| window.remove_window());
}

fn main() {
    // The first argument may be a vault path or a `dragonglass://` URI, so the app can be
    // registered as the OS handler for the scheme. Without one, reopen the last vault; on
    // a first run, ask which folder to open.
    let requested = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("DRAGON_GLASS_VAULT").ok())
        .filter(|argument| !argument.trim().is_empty());

    let (requested_vault, uri_note) = match requested {
        Some(argument) => match uri::parse(&argument) {
            Some(parsed) => (parsed.vault.map(PathBuf::from), parsed.note),
            None => {
                let path = vaults::canonical(Path::new(&argument));
                if path.is_dir() {
                    (Some(path), None)
                } else if path.is_file() {
                    // macOS/Linux/Windows file associations pass a note path. Prefer the
                    // deepest recent vault that contains it; otherwise treat its folder as
                    // the vault.
                    let registry = vaults::Registry::load();
                    let vault = registry
                        .recent()
                        .iter()
                        .filter(|vault| path.starts_with(vault))
                        .max_by_key(|vault| vault.components().count())
                        .cloned()
                        .or_else(|| path.parent().map(Path::to_path_buf));
                    let note = vault.as_ref().and_then(|vault| {
                        path.strip_prefix(vault)
                            .ok()
                            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                    });
                    (vault, note)
                } else {
                    (Some(path), None)
                }
            }
        },
        None => (None, None),
    };

    // URLs opened while the app runs arrive through a platform callback with no app
    // handle. Feed a channel, then handle them on the foreground task in `run`.
    #[cfg(target_os = "macos")]
    let (url_tx, url_rx) = smol::channel::unbounded::<Vec<String>>();

    let application = Application::new();
    #[cfg(target_os = "macos")]
    application.on_open_urls(move |urls| {
        let _ = url_tx.try_send(urls);
    });

    application.run(move |cx: &mut App| {
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
        // Standard application menu items on macOS. They are harmless elsewhere.
        cx.on_action(|_: &HideApp, cx: &mut App| cx.hide());
        cx.on_action(|_: &HideOthers, cx: &mut App| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx: &mut App| cx.unhide_other_apps());

        // Shortcuts are global; a vault's saved overrides replace them once it opens.
        editor::init(cx);
        // Defaults use `secondary`, which GPUI resolves to Command on macOS and Control
        // elsewhere. Tab switching stays on Control everywhere: Command-Tab is the macOS
        // application switcher.
        for (action, default, _) in HOTKEYS {
            bind_named(cx, &platform::normalize(default), action);
        }
        cx.bind_keys([
            KeyBinding::new("escape", CloseOverlay, None),
            KeyBinding::new("ctrl-pagedown", NextTab, None),
            KeyBinding::new("ctrl-pageup", PrevTab, None),
            KeyBinding::new("secondary-q", Quit, None),
            KeyBinding::new("secondary-alt-v", ToggleVim, None),
        ]);
        // Command-H is Hide on macOS and Ctrl-H is find/replace elsewhere.
        #[cfg(target_os = "macos")]
        cx.bind_keys([
            KeyBinding::new("cmd-h", HideApp, None),
            KeyBinding::new("cmd-alt-h", HideOthers, None),
        ]);

        #[cfg(target_os = "macos")]
        {
            install_menus(cx);
            install_url_handler(cx, url_rx);
        }

        // An explicit path (a shortcut, the file manager, or a `dragonglass://` URI) wins.
        // If it cannot be opened, fall back to the chooser rather than exiting silently.
        open_request(requested_vault.clone(), uri_note.clone(), cx);
    });
}

/// Open the requested vault, or the last vault, or the chooser.
fn open_request(vault: Option<PathBuf>, note: Option<String>, cx: &mut App) {
    if let Some(vault) = vault {
        if !open_vault_window(vault.clone(), note, cx) {
            open_vault_picker(
                cx,
                Some(format!("Could not open {}.", vault.display())),
                Rc::new(open_vault_at_startup),
            );
        }
        return;
    }

    // A URI that names no vault still supplies a note.
    match vaults::Registry::load().last().map(Path::to_path_buf) {
        Some(last) if last.is_dir() => {
            if !open_vault_window(last.clone(), note, cx) {
                open_vault_picker(
                    cx,
                    Some(format!("Could not open {}.", last.display())),
                    Rc::new(open_vault_at_startup),
                );
            }
        }
        Some(last) => open_vault_picker(
            cx,
            Some(format!(
                "The last vault, {}, is no longer available.",
                last.display()
            )),
            Rc::new(open_vault_at_startup),
        ),
        None => open_vault_picker(cx, None, Rc::new(open_vault_at_startup)),
    }
}

/// Open a vault chosen in the startup picker, falling back to the picker on failure.
fn open_vault_at_startup(vault_path: PathBuf, cx: &mut App) {
    if !open_vault_window(vault_path.clone(), None, cx) {
        open_vault_picker(
            cx,
            Some(format!("Could not open {}.", vault_path.display())),
            Rc::new(open_vault_at_startup),
        );
    }
}

/// Open a vault chosen while the app is running in a second process.
///
/// Each window owns its vault, theme and keymap, so switching vaults is a fresh launch
/// rather than a reset of the current window. The current vault stays open beside it.
fn open_vault_in_new_process(vault_path: PathBuf, cx: &mut App) {
    let spawned = spawn_vault_process(&vault_path);

    if let Err(error) = spawned {
        open_vault_picker(
            cx,
            Some(format!("Could not open {}: {error}", vault_path.display())),
            Rc::new(open_vault_in_new_process),
        );
    }
}

/// Launch another Dragon Glass process for a second vault.
///
/// Inside a macOS `.app`, launching the inner executable directly bypasses LaunchServices;
/// use `open -n` on the bundle instead so the new instance gets a Dock icon, menu and
/// activation. Dev builds fall back to running the binary.
fn spawn_vault_process(vault_path: &Path) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;

    if cfg!(target_os = "macos") {
        let bundle = exe.ancestors().find(|candidate| {
            candidate
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        });
        if let Some(bundle) = bundle {
            return std::process::Command::new("open")
                .arg("-n")
                .arg(bundle)
                .arg("--args")
                .arg(vault_path)
                .spawn()
                .map(|_| ());
        }
    }

    std::process::Command::new(exe)
        .arg(vault_path)
        .spawn()
        .map(|_| ())
}

/// Build the main window for `vault_path`. Returns false when the vault cannot be opened.
fn open_vault_window(vault_path: PathBuf, uri_note: Option<String>, cx: &mut App) -> bool {
    let vault = match Vault::open(&vault_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("could not open vault {}: {e}", vault_path.display());
            return false;
        }
    };

    // Remember it before doing any other work, so a crash later still leaves the vault
    // at the top of the list next time.
    let mut registry = vaults::Registry::load();
    let _ = registry.record(vault.root());

    let note_refs = vault.notes();
    let note_count = note_refs.len();

    // One pass over the notes for both the link count and the "most linked note" pick.
    let mut link_count = 0usize;
    let mut best_links = 0usize;
    let mut first: Option<&NoteRef> = None;
    for note_ref in &note_refs {
        if let Ok(note) = Note::load(&note_ref.path) {
            let links = note.links().len();
            link_count += links;
            if links >= best_links {
                best_links = links;
                first = Some(note_ref);
            }
        }
    }

    let tree = tree::build_with_dirs(&note_refs, &vault.folders());
    let mut initial_rows = Vec::new();
    flatten(&tree, 0, &HashSet::new(), &mut initial_rows);
    let file_rows = Rc::new(initial_rows);
    let vault_root = vault.root().to_path_buf();
    let initial_session = session::load(&vault_root);
    let initial_bookmarks = bookmarks::load(&vault_root);
    let initial_user_themes = load_user_themes(&vault_root);
    let initial_config = config::load(&vault_root);
    let initial_theme = {
        let mut theme = load_theme(&vault_root, &initial_user_themes);
        if let Some(accent) = initial_config
            .accent
            .as_deref()
            .and_then(crate::theme::parse_hex)
        {
            theme = theme.with_accent(accent);
        }
        theme
    };
    let initial_snippets = store::load_overrides(&store::snippets_dir(&vault_root));
    let initial_graph = graph::build(&note_refs);
    let initial_suggestions: Rc<Vec<(String, String)>> = Rc::new(
        note_refs
            .iter()
            .map(|note| (note.path_no_ext().to_string(), note.stem().to_string()))
            .collect(),
    );
    let initial_tags: Rc<Vec<String>> = Rc::new(tags::vault_tags(&note_refs));
    let initial_plugins = plugins::discover(&vault_root);
    let initial_workspaces = workspace::load(&vault_root);
    let initial_bindings = keymap::load(&vault_root);
    let initial_plugin_commands: Vec<(usize, String, String)> = initial_plugins
        .iter()
        .enumerate()
        .flat_map(|(index, plugin)| {
            plugin
                .manifest
                .commands
                .iter()
                .map(move |command| (index, command.id.clone(), command.title.clone()))
        })
        .collect();
    let update_source = std::env::var("DRAGON_GLASS_UPDATE_URL").ok();
    let update_available = update_source.as_deref().and_then(|source| {
        update::latest_if_newer(env!("CARGO_PKG_VERSION"), source)
            .ok()
            .flatten()
    });
    let initial_git = if git::is_repo(&vault_root) {
        git::status(&vault_root).ok()
    } else {
        None
    };
    let display_path = vaults::canonical(&vault_path).to_string_lossy().to_string();

    // Tabs to reopen: an explicit URI note wins, then the saved session's notes that still
    // exist, then the most linked note.
    let wanted = restored_tabs(
        &initial_session,
        uri_note.as_deref(),
        first.map(|note| note.relative.as_str()),
        |note| note_refs.iter().any(|candidate| candidate.relative == note),
    );

    // Saved shortcut overrides win over the defaults registered above.
    for binding in &initial_bindings {
        bind_named(cx, &binding.key, &binding.action);
    }

    let forced_reading = std::env::var("DRAGON_GLASS_READING").is_ok();
    // The saved active tab decides which note opens first; the others load when activated,
    // so a session with many tabs does not read every file before the window appears.
    let initial_active = if wanted.is_empty() {
        0
    } else {
        initial_session.active.min(wanted.len() - 1)
    };
    let mut initial_tabs: Vec<Tab> = Vec::new();
    for (index, relative) in wanted.iter().enumerate() {
        let editor = cx.new(Editor::new);
        let saved = initial_session
            .tabs
            .iter()
            .find(|tab| &tab.note == relative);
        let active = index == initial_active;
        let loaded = editor.update(cx, |editor, _cx| {
            editor.set_suggestions((*initial_suggestions).clone(), (*initial_tags).clone());
            editor.set_vault_root(&vault_root);
            editor.set_attachments_folder(&initial_config.attachments_folder);
            editor.set_vim_enabled(initial_config.vim_mode);
            editor.set_font_size(px(initial_config.font_size as f32));
            if active {
                let loaded = editor.load(&vault_root.join(relative)).is_ok();
                if forced_reading || saved.is_some_and(|tab| tab.reading) {
                    editor.set_reading_mode(true);
                }
                if let Some(saved) = saved {
                    editor.set_folded_headings(&saved.folded);
                    editor.set_cursor_position(saved.line, saved.col);
                    editor.set_selection_anchor(saved.anchor);
                    editor.set_scroll_offset(saved.scroll);
                    editor.scroll_reading_to(saved.reading_block);
                }
                loaded
            } else {
                // Reading and cursor are applied when the tab is activated.
                true
            }
        });
        initial_tabs.push(Tab {
            relative: Some(relative.clone()),
            editor,
            title: tab_title(relative),
            conflict: false,
            load_error: if loaded {
                None
            } else {
                Some("the file could not be read".to_string())
            },
            pending_load: !active,
        });
    }
    if initial_tabs.is_empty() {
        let editor = cx.new(Editor::new);
        editor.update(cx, |editor, _cx| {
            editor.set_suggestions((*initial_suggestions).clone(), (*initial_tags).clone());
            editor.set_vault_root(&vault_root);
            editor.set_attachments_folder(&initial_config.attachments_folder);
            editor.set_vim_enabled(initial_config.vim_mode);
            editor.set_font_size(px(initial_config.font_size as f32));
        });
        initial_tabs.push(Tab {
            relative: None,
            editor,
            title: "Untitled".to_string(),
            conflict: false,
            load_error: None,
            pending_load: false,
        });
    }

    let editor = initial_tabs[initial_active].editor.clone();
    let first_rel = initial_tabs[initial_active].relative.clone();
    let initial_properties = editor.update(cx, |editor, _cx| editor.properties());
    let initial_cache_epoch = 1u64;
    let initial_headings_version = (initial_cache_epoch, editor.read(cx).document_version().1);
    let initial_text = editor.read(cx).text();
    let initial_headings = Rc::new(markdown::headings(&initial_text));
    let initial_stats = stats::compute(&initial_text);
    let note_index = resolve::NoteIndex::new(&note_refs);
    cx.set_global(initial_theme);
    cx.set_global(FocusMode::default());
    cx.set_global(ReaderSettings {
        body_size: initial_config.reader_size as f32,
        measure: initial_config.reader_width as f32,
        line_height: initial_config.reader_line_height as f32 / 100.0,
        font: ReaderFont::from_name(&initial_config.reader_font),
    });

    let bounds = Bounds::centered(None, size(px(1000.0), px(640.0)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some("Dragon Glass".into()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let app_view = cx.new(|cx| {
        let app = DragonGlassApp {
            editor: editor.clone(),
            tabs: initial_tabs,
            closed_tabs: Vec::new(),
            zen: false,
            nav_history: Vec::new(),
            nav_index: 0,
            active_tab: initial_active,
            split_tabs: Vec::new(),
            split_axis: SplitAxis::Horizontal,
            minimal: false,
            close_requested: false,
            dragging: None,
            shown_title: "Dragon Glass".to_string(),
            cache_epoch: initial_cache_epoch,
            vault_root: vault_root.clone(),
            tree: tree.clone(),
            file_rows: file_rows.clone(),
            collapsed_dirs: HashSet::new(),
            collapsed_outline: HashSet::new(),
            note_refs: note_refs.clone(),
            note_index,
            note_suggestions: initial_suggestions,
            tag_suggestions: initial_tags,
            properties: initial_properties,
            headings: initial_headings,
            headings_version: initial_headings_version,
            preview_content: ReadingCache::new(),
            bookmarks: initial_bookmarks,
            theme: initial_theme,
            active: first_rel.clone(),
            vault_path: display_path.clone(),
            notes: note_count,
            links: link_count,
            git_status: initial_git.clone(),
            status: None,
            pending_draft: None,
            derived_at: None,
            derived_armed: false,
            preview_at: None,
            preview_armed: false,
            preview_epoch: u64::MAX,
            status_timer_armed: false,
            switcher: None,
            switcher_focus: cx.focus_handle(),
            focus_handle: cx.focus_handle(),
            quit_prompt: false,
            quit_after_prompt: false,
            allow_close: false,
            show_settings: false,
            pending_setting: None,
            settings_focus: cx.focus_handle(),
            tree_menu: None,
            pending_dir: None,
            pending_path: None,
            pending_save_as: false,
            tab_menu: None,
            scroll_sync_generation: 0,
            last_editor_progress: -1.0,
            last_preview_progress: -1.0,
            outline_scroll: UniformListScrollHandle::new(),
            outline_current: None,
            conflict: false,
            watcher: None,
            update_source,
            update_available,
            full_graph: initial_graph.clone(),
            graph_view: None,
            show_graph: false,
            relations: Vec::new(),
            show_relations: false,
            sidebar_width: 240.0,
            outline_width: 200.0,
            relations_width: 240.0,
            resize: None,
            show_diff: false,
            diff_title: String::new(),
            diff_summary: String::new(),
            diff_rows: Rc::new(Vec::new()),
            review_issues: Vec::new(),
            pending_shared: None,
            pending_template: None,
            stats: initial_stats,
            stats_version: initial_headings_version,
            query_results: Rc::new(Vec::new()),
            show_query: false,
            split_preview: std::env::var("DRAGON_GLASS_PREVIEW").is_ok(),
            plugins: initial_plugins,
            plugin_commands: initial_plugin_commands,
            workspaces: initial_workspaces,
            user_themes: initial_user_themes,
            bindings: initial_bindings,
            base_theme: initial_theme,
            snippet_overrides: initial_snippets,
            config: initial_config,
        };
        app.install_quit_saver(cx);
        app
    });

    let graph_view_entity = cx.new(|cx| GraphView::new(app_view.downgrade(), cx));
    app_view.update(cx, |app, cx| {
        app.graph_view = Some(graph_view_entity.clone());
        let opener = app.link_opener(cx);
        let editor = app.editor.clone();
        editor.update(cx, |editor, _| editor.set_link_opener(opener));
        // Tabs restored from the session need the same callbacks as newly created ones.
        app.configure_editors(cx);
        app.check_recovery(cx);
    });

    app_view.update(cx, |this, cx| this.watch_vault(cx));
    // Pin the note we opened, so the next launch reopens the same one.
    app_view.update(cx, |this, cx| this.remember_session(cx));

    let view_for_window = app_view.clone();
    let window = cx
        .open_window(options, move |window, cx| {
            // Persist the reading mode too, including changes made with Ctrl+E, which
            // the editor handles without telling the app.
            let saver = view_for_window.clone();
            window.on_window_should_close(cx, move |window, cx| {
                saver.update(cx, |app, cx| app.request_close(window, cx))
            });
            view_for_window
        })
        .expect("failed to open window");

    let _ = window.update(cx, |view, window, cx| {
        if view.config.scroll_sync && view.split_preview {
            view.start_scroll_sync(cx);
        }
        window.focus(&view.editor.focus_handle(cx));
        cx.activate(true);
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_links_resolve_from_the_note_folder() {
        assert_eq!(
            relative_link_target("../Sibling", Some("notes/deep")),
            "notes/Sibling"
        );
        assert_eq!(
            relative_link_target("./Child", Some("notes")),
            "notes/Child"
        );
        assert_eq!(relative_link_target("../../Top", Some("a/b")), "Top");
        // Escaping the vault clamps at the root rather than producing `..`.
        assert_eq!(relative_link_target("../../../Top", Some("a")), "Top");
        // Non-relative targets keep their global resolution.
        assert_eq!(relative_link_target("Sibling", Some("notes")), "Sibling");
    }

    fn tab(note: &str) -> session::TabSession {
        session::TabSession {
            note: note.to_string(),
            ..session::TabSession::default()
        }
    }

    #[test]
    fn session_tabs_restore_only_notes_that_still_exist() {
        let session = session::Session {
            version: 2,
            active: 0,
            tabs: vec![
                tab("a.md"),
                tab(""),
                tab("missing.md"),
                tab("b.md"),
                tab("a.md"),
            ],
        };
        let tabs = restored_tabs(&session, None, Some("fallback.md"), |note| {
            !note.is_empty() && note != "missing.md"
        });
        assert_eq!(tabs, vec!["a.md", "b.md"]);
    }

    #[test]
    fn an_explicit_uri_note_wins_over_the_session() {
        let session = session::Session {
            version: 2,
            active: 0,
            tabs: vec![tab("a.md")],
        };
        let tabs = restored_tabs(&session, Some("uri.md"), Some("fallback.md"), |_| true);
        assert_eq!(tabs, vec!["uri.md"]);
    }

    #[test]
    fn an_empty_session_falls_back_to_one_note() {
        let session = session::Session::default();
        let tabs = restored_tabs(&session, None, Some("linked.md"), |_| true);
        assert_eq!(tabs, vec!["linked.md"]);

        let tabs = restored_tabs(&session, None, None, |_| true);
        assert!(tabs.is_empty());
    }

    fn heading(level: u8, line: usize) -> markdown::Heading {
        markdown::Heading {
            level,
            text: format!("h{line}"),
            line,
        }
    }

    #[test]
    fn a_folded_outline_heading_hides_only_its_children() {
        let headings = vec![
            heading(1, 1),
            heading(2, 2),
            heading(3, 4),
            heading(2, 6),
            heading(1, 8),
        ];
        let mut collapsed = HashSet::new();
        collapsed.insert(0);

        let visible: Vec<usize> = visible_headings(&headings, &collapsed)
            .into_iter()
            .map(|(index, _)| index)
            .collect();

        assert_eq!(visible, vec![0, 4]);
    }

    #[test]
    fn folding_a_nested_outline_heading_leaves_the_parent_visible() {
        let headings = vec![heading(1, 1), heading(2, 2), heading(3, 4), heading(2, 6)];
        let mut collapsed = HashSet::new();
        collapsed.insert(1);

        let visible: Vec<usize> = visible_headings(&headings, &collapsed)
            .into_iter()
            .map(|(index, _)| index)
            .collect();

        assert_eq!(visible, vec![0, 1, 3]);
    }
}
