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

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use dragonglass_core::frontmatter::{Property, PropertyValue};
use dragonglass_core::{
    bookmarks, config, conflict, diff, git, graph, keymap, lint, markdown, query, relations,
    resolve, search, session, shared, stats, storage, store, templates, tree, update, uri, vaults,
    versions, workspace, NodeKind, Note, NoteRef, TreeNode, Vault,
};

/// Actions the hotkey editor can rebind: `(action, default key, label)`.
const HOTKEYS: &[(&str, &str, &str)] = &[
    ("OpenCommands", "secondary+p", "Command palette"),
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
    ("NextTab", "ctrl-tab", "Next tab"),
    ("PrevTab", "ctrl-shift-tab", "Previous tab"),
];

/// Bind a saved override by action name.
fn bind_named(cx: &mut App, key: &str, action: &str) {
    // Accept both `ctrl+shift+g` and `ctrl-shift-g`; GPUI only understands `-`.
    let key = platform::normalize(key);
    let key = key.as_str();
    match action {
        "OpenCommands" => cx.bind_keys([KeyBinding::new(key, OpenCommands, None)]),
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
        "NextTab" => cx.bind_keys([KeyBinding::new(key, NextTab, None)]),
        "PrevTab" => cx.bind_keys([KeyBinding::new(key, PrevTab, None)]),
        _ => {}
    }
}
use dragonglass_plugins as plugins;
use editor::Editor;
use gpui::{
    actions, div, prelude::*, px, size, uniform_list, AnyElement, App, Application, Bounds,
    Context, CursorStyle, ElementId, Entity, FocusHandle, Focusable, KeyBinding, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Render, WeakEntity, Window,
    WindowBounds, WindowOptions,
};
use graph_view::GraphView;
use reading::ReadingCache;
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
        ToggleTheme,
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
        NextTab,
        PrevTab,
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
    Note { relative: String },
    Git(GitAction),
    Property { key: Option<String> },
    DeleteProperty { key: String },
    Command(CommandId),
    Workspace(String),
    Hotkey(String),
    VersionSave(String),
    VersionCompare { left: Option<String>, right: String },
    VersionPromote(String),
    VersionDelete(String),
    Template(String),
    TemplateNew(String),
    Shared(String),
    SharedEdit(String),
    Issue(usize),
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

/// One open note. Each tab owns its editor, so cursor, scroll and unsaved edits are
/// preserved when switching between notes.
struct Tab {
    relative: Option<String>,
    editor: Entity<Editor>,
    title: String,
    /// Whether the file changed on disk while this tab has unsaved edits.
    conflict: bool,
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
    path.split('/')
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .map(|segment| {
            segment
                .chars()
                .map(|ch| match ch {
                    ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\\' => '-',
                    other => other,
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
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
    /// Bumped when the active editor changes so document-derived caches invalidate even
    /// when two editors share the same buffer revision.
    cache_epoch: u64,
    vault_root: PathBuf,
    tree: TreeNode,
    /// Flattened tree rows for the sidebar. Rebuilt when the vault changes, never in
    /// `render`, so scrolling does not recursively walk the tree each frame.
    file_rows: Rc<Vec<Row>>,
    note_refs: Vec<NoteRef>,
    /// Link resolution for the whole vault: paths, stems, frontmatter names and aliases.
    note_index: resolve::NoteIndex,
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
    status_message: Option<String>,
    switcher: Option<Switcher>,
    switcher_focus: FocusHandle,
    focus_handle: FocusHandle,
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
        if target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with("mailto:")
        {
            cx.open_url(target);
            return;
        }

        let target = target.split('#').next().unwrap_or(target).trim();
        if target.is_empty() {
            return;
        }

        match self.note_index.resolve_relative(target) {
            Some(relative) => {
                let relative = relative.to_string();
                self.open_note(&relative, window, cx);
            }
            None => {
                // An unresolved wikilink creates the note, the way Obsidian does.
                let is_asset = Path::new(target)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| {
                        !ext.eq_ignore_ascii_case("md") && !ext.eq_ignore_ascii_case("markdown")
                    })
                    .unwrap_or(false);
                if is_asset {
                    self.status_message = Some(format!("unresolved link: {target}"));
                    cx.notify();
                    return;
                }
                self.create_linked_note(target, window, cx);
            }
        }
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
                self.status_message = Some(format!("created note: {relative}"));
                self.open_note(&relative, window, cx);
            }
            Err(error) => {
                self.status_message = Some(format!("could not create note: {error}"));
                cx.notify();
            }
        }
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
        editor.update(cx, |editor, _cx| {
            editor.set_vault_root(&root);
            editor.set_attachments_folder(&folder);
            editor.set_link_opener(opener);
        });
        editor
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
        let saved = session::Session {
            note: self.active.clone(),
            reading: self.editor.read(cx).is_reading(),
        };
        let _ = session::save(&self.vault_root, &saved);
    }

    /// Switch the active tab, syncing the editor, outline and properties.
    fn activate_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }

        self.active_tab = index;
        self.editor = self.tabs[index].editor.clone();
        self.active = self.tabs[index].relative.clone();
        self.conflict = self.tabs[index].conflict;
        self.cache_epoch = self.cache_epoch.wrapping_add(1);
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

        let title = tab_title(relative);
        if let Some(index) = empty {
            let editor = self.tabs[index].editor.clone();
            if !self.load_into(&editor, relative, cx) {
                self.status_message = Some(format!("could not open {relative}"));
                cx.notify();
                return;
            }
            self.tabs[index].relative = Some(relative.to_string());
            self.tabs[index].title = title;
            self.tabs[index].conflict = false;
            self.activate_tab(index, cx);
        } else {
            let editor = self.new_editor(cx);
            if !self.load_into(&editor, relative, cx) {
                self.status_message = Some(format!("could not open {relative}"));
                cx.notify();
                return;
            }
            self.tabs.push(Tab {
                relative: Some(relative.to_string()),
                editor: editor.clone(),
                title,
                conflict: false,
            });
            self.active_tab = self.tabs.len() - 1;
            self.editor = editor;
            self.active = Some(relative.to_string());
            self.conflict = false;
            self.cache_epoch = self.cache_epoch.wrapping_add(1);
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
    }

    /// Close a tab. Dirty notes are saved first so closing never loses edits.
    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }

        let save_result = self.tabs[index].editor.update(cx, |editor, _cx| {
            if editor.is_dirty() {
                editor.save()
            } else {
                Ok(())
            }
        });
        if let Err(error) = save_result {
            self.status_message = Some(format!("could not save before closing: {error}"));
            cx.notify();
            return;
        }

        let was_active = index == self.active_tab;
        let title = self.tabs[index].title.clone();
        self.tabs.remove(index);
        self.reindex_splits_after_remove(index);

        if self.tabs.is_empty() {
            let editor = self.new_editor(cx);
            self.tabs.push(Tab {
                relative: None,
                editor,
                title: "Untitled".to_string(),
                conflict: false,
            });
            self.activate_tab(0, cx);
        } else if was_active {
            let next = index.min(self.tabs.len() - 1);
            self.activate_tab(next, cx);
        } else if index < self.active_tab {
            self.active_tab -= 1;
        }

        self.normalize_splits();
        self.status_message = Some(format!("closed {title}"));
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
                self.status_message =
                    Some("changed on disk - saving will overwrite the file".to_string());
            } else if reloaded {
                self.status_message = Some("reloaded from disk (changed externally)".to_string());
            }
        } else if conflicted {
            self.status_message = Some("a note changed on disk in another tab".to_string());
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
            self.tree = tree::build(&self.note_refs);
            self.notes = self.note_refs.len();
            self.full_graph = graph::build(&self.note_refs);
            if self.show_relations {
                self.relations = relations::build(&self.note_refs);
            }
        }
        // A rescan is also when attachments may have appeared or disappeared.
        self.editor
            .update(cx, |editor, _| editor.invalidate_attachments());
        let mut rows = Vec::new();
        flatten(&self.tree, 0, &mut rows);
        self.file_rows = Rc::new(rows);
        self.refresh_git();
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
                self.status_message = Some(format!("could not create note: {e}"));
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
                    item: Item::Note {
                        relative: hit.relative,
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
                command_item("Save", "secondary+s", CommandId::Save),
                command_item("Toggle theme", "secondary+shift+l", CommandId::ToggleTheme),
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
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    fn open_switcher(&mut self, _: &OpenSwitcher, window: &mut Window, cx: &mut Context<Self>) {
        self.open_panel(SwitcherMode::QuickSwitch, window, cx);
    }

    fn open_search(&mut self, _: &OpenSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.open_panel(SwitcherMode::Search, window, cx);
    }

    fn open_git(&mut self, _: &OpenGit, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_git();
        self.open_panel(SwitcherMode::GitMenu, window, cx);
    }

    fn open_commands(&mut self, _: &OpenCommands, window: &mut Window, cx: &mut Context<Self>) {
        self.open_panel(SwitcherMode::CommandsMenu, window, cx);
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

        self.status_message = Some(match workspace::save(&self.vault_root, &self.workspaces) {
            Ok(()) => format!("workspace saved: {name}"),
            Err(e) => format!("could not save workspace: {e}"),
        });
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

        self.status_message = Some(format!("workspace: {name}"));
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

        self.status_message = Some({
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
            self.status_message = Some("document relations".to_string());
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
        self.status_message = Some(format!(
            "relation graph: {} notes / {} relations",
            enriched.nodes.len(),
            relations
        ));
        window.focus(&view.focus_handle(cx));
        cx.notify();
    }

    fn save_version(&mut self, label: &str, cx: &mut Context<Self>) {
        let Some(relative) = self.active.clone() else {
            self.status_message = Some("open a note before saving a version".to_string());
            cx.notify();
            return;
        };

        let content = self.editor.read(cx).text();
        self.status_message = Some(
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
            self.status_message = Some("open a note before comparing versions".to_string());
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
                self.status_message = Some(format!("could not compare versions: {error}"));
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
        self.status_message = Some(format!("version diff: {}", self.diff_summary));
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

        self.status_message = Some(match versions::promote(&self.vault_root, &relative, id) {
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
                format!("promoted version {id} to {relative}")
            }
            Err(error) => format!("could not promote version: {error}"),
        });
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    fn delete_version(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(relative) = self.active.clone() else {
            return;
        };
        self.status_message = Some(match versions::remove(&self.vault_root, &relative, id) {
            Ok(()) => format!("deleted version {id}"),
            Err(error) => format!("could not delete version: {error}"),
        });
        cx.notify();
    }

    fn insert_template(&mut self, relative: &str, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.vault_root.join(relative);
        let template = match std::fs::read_to_string(&path) {
            Ok(template) => template,
            Err(error) => {
                self.status_message = Some(format!("could not read template: {error}"));
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
        self.status_message = Some(format!("inserted template {relative}"));
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
            self.status_message = Some("a note needs a name".to_string());
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
            self.status_message = Some(format!("{relative} already exists"));
            cx.notify();
            return;
        }

        let template_text = match std::fs::read_to_string(&template.path) {
            Ok(text) => text,
            Err(error) => {
                self.status_message = Some(format!("could not read template: {error}"));
                cx.notify();
                return;
            }
        };
        let rendered = render_template(&template_text, &self.vault_root, Some(&relative), name);

        if let Err(error) = storage::write_atomic(&path, &rendered) {
            self.status_message = Some(format!("could not create note: {error}"));
            cx.notify();
            return;
        }

        self.refresh_vault(cx);
        self.open_note(&relative, window, cx);
        self.status_message = Some(format!("created {relative} from {}", template.name));
        cx.notify();
    }

    fn insert_shared(&mut self, slug: &str, window: &mut Window, cx: &mut Context<Self>) {
        let reference = format!("{{{{shared:{}}}}}", shared::sanitize(slug));
        self.editor.update(cx, |editor, cx| {
            editor.replace_selection_or_insert(&reference);
            cx.notify();
        });
        window.focus(&self.editor.focus_handle(cx));
        self.status_message = Some(format!("inserted shared block {slug}"));
        cx.notify();
    }

    fn extract_shared(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.clone();
        let text = editor
            .read(cx)
            .selection_text()
            .or_else(|| editor.read(cx).current_block_text());

        let Some(text) = text else {
            self.status_message =
                Some("select text or place the caret in a block first".to_string());
            cx.notify();
            return;
        };
        if text.trim().is_empty() {
            self.status_message = Some("nothing to extract".to_string());
            cx.notify();
            return;
        }

        self.pending_shared = Some(text);
        self.open_panel(SwitcherMode::SharedName, window, cx);
        self.status_message = Some("name the shared block".to_string());
    }

    fn review_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).text();
        self.review_issues = lint::review_with(&text, &self.note_index);
        if self.review_issues.is_empty() {
            self.status_message = Some("review: no issues found".to_string());
            cx.notify();
            return;
        }

        self.status_message = Some(format!("review: {} issue(s)", self.review_issues.len()));
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
        self.status_message = Some(format!("related: {target}"));
    }

    /// The centre pane: the graph, the query table, a diff, or the editor (optionally with
    /// a preview pane beside it).
    fn center_view(&self, theme: Theme, preview: Option<AnyElement>) -> AnyElement {
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
        if !self.split_tabs.is_empty() {
            return match self.split_axis {
                SplitAxis::Horizontal => {
                    let mut row = div()
                        .flex()
                        .flex_row()
                        .size_full()
                        .child(div().flex_1().min_w(px(0.0)).child(self.editor.clone()));
                    for index in &self.split_tabs {
                        if let Some(tab) = self.tabs.get(*index) {
                            row = row
                                .child(div().w(px(1.0)).h_full().bg(theme.border).flex_shrink_0())
                                .child(div().flex_1().min_w(px(0.0)).child(tab.editor.clone()));
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
                        if let Some(tab) = self.tabs.get(*index) {
                            column = column
                                .child(div().h(px(1.0)).w_full().bg(theme.border).flex_shrink_0())
                                .child(div().flex_1().min_h(px(0.0)).child(tab.editor.clone()));
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

    /// Escape closes whatever is on top: the panel, then the query view, then the graph.
    fn close_overlay(&mut self, _: &CloseOverlay, window: &mut Window, cx: &mut Context<Self>) {
        if self.switcher.is_some() {
            self.close_panel(window, cx);
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
        }
    }

    fn new_window(&mut self, _: &NewWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.open_new_window(window, cx);
    }

    fn close_tab_action(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.switcher.is_none() {
            self.close_tab(self.active_tab, window, cx);
        }
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
        editor.update(cx, |editor, _cx| {
            editor.set_vault_root(&self.vault_root);
            editor.set_attachments_folder(&self.config.attachments_folder);
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
        self.status_message = Some(format!("moved {title} to its own window"));
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
        self.status_message = Some(format!("returned {}", self.tabs[first].title));
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
                self.status_message = Some(self.tabs[target].title.clone());
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
                        self.status_message = Some("at most four editor panes".to_string());
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
                self.status_message = Some(format!(
                    "{axis_name} split: {} panes",
                    self.split_tabs.len() + 1
                ));
            }
            TabDrop::Detach => {
                if same_window {
                    self.detach_tab(target, window, cx);
                    return;
                }
                self.activate_tab(target, cx);
                self.status_message = Some(self.tabs[target].title.clone());
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
        let sibling = cx.new(|cx| DragonGlassApp {
            editor: editor.clone(),
            tabs,
            active_tab,
            split_tabs: Vec::new(),
            split_axis: SplitAxis::Horizontal,
            minimal,
            close_requested: false,
            dragging: None,
            cache_epoch: 1,
            vault_root: self.vault_root.clone(),
            tree: self.tree.clone(),
            file_rows: self.file_rows.clone(),
            note_refs: self.note_refs.clone(),
            note_index: self.note_index.clone(),
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
            status_message: None,
            switcher: None,
            switcher_focus: cx.focus_handle(),
            focus_handle: cx.focus_handle(),
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
        });

        let graph_view = cx.new(|cx| GraphView::new(sibling.downgrade(), cx));
        sibling.update(cx, |app, cx| {
            app.graph_view = Some(graph_view);
            app.watch_vault(cx);
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
                        let _ = target.update(cx, |app, cx| app.adopt_tabs(tabs, cx));
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
            self.status_message = Some("single editor".to_string());
        } else if self.tabs.len() < 2 {
            self.status_message = Some("open another note to use split editing".to_string());
        } else {
            let next = (self.active_tab + 1) % self.tabs.len();
            self.split_tabs.push(next);
            self.split_axis = SplitAxis::Horizontal;
            self.split_preview = false;
            self.status_message = Some(format!("two editors: {}", self.tabs[next].title));
        }
        cx.notify();
    }

    fn toggle_split_preview(&mut self, cx: &mut Context<Self>) {
        self.split_preview = !self.split_preview;
        if self.split_preview {
            self.split_tabs.clear();
            self.editor.update(cx, |editor, cx| {
                if editor.is_reading() {
                    editor.set_reading_mode(false);
                    cx.notify();
                }
            });
        }
        self.status_message = Some(format!(
            "preview pane: {}",
            if self.split_preview { "on" } else { "off" }
        ));
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
        if self.config.snippets {
            self.base_theme.with_overrides(&self.snippet_overrides)
        } else {
            self.base_theme
        }
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
        self.status_message = Some(message);
        cx.notify();
    }

    fn apply_user_theme(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some((name, theme)) = self.user_themes.get(index).cloned() else {
            return;
        };

        self.base_theme = theme;
        cx.set_global(theme);
        self.refresh_theme(cx);
        self.status_message = Some(match save_theme(&self.vault_root, theme, Some(&name)) {
            Ok(()) => format!("theme: {name}"),
            Err(e) => format!("theme: {name} (not saved: {e})"),
        });
        self.editor.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn toggle_theme(&mut self, _: &ToggleTheme, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_theme_now(cx);
        let _ = window;
    }

    fn check_updates(&mut self) {
        self.status_message = Some(match &self.update_source {
            None => "update checks are not configured (set DRAGON_GLASS_UPDATE_URL)".to_string(),
            Some(source) => match update::latest_if_newer(env!("CARGO_PKG_VERSION"), source) {
                Ok(Some(latest)) => {
                    self.update_available = Some(latest.clone());
                    format!("update available: {latest}")
                }
                Ok(None) => {
                    self.update_available = None;
                    "up to date".to_string()
                }
                Err(e) => format!("update check failed: {e}"),
            },
        });
    }

    fn toggle_theme_now(&mut self, cx: &mut Context<Self>) {
        self.base_theme = self.base_theme.toggled();
        self.refresh_theme(cx);
        let message = match save_theme(&self.vault_root, self.base_theme, None) {
            Ok(()) => format!("theme: {}", self.theme.name()),
            Err(e) => format!("theme: {} (not saved: {e})", self.theme.name()),
        };
        self.status_message = Some(message);
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
        self.status_message = Some(match bookmarks::save(&self.vault_root, &self.bookmarks) {
            Ok(()) if added => format!("bookmarked {active}"),
            Ok(()) => format!("removed bookmark {active}"),
            Err(e) => format!("could not save bookmarks: {e}"),
        });
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
                self.status_message = Some("properties updated".to_string());
            }
            Err(e) => self.status_message = Some(format!("could not save properties: {e}")),
        }
    }

    fn run_git_action(&mut self, action: GitAction) {
        let root = self.vault_root.clone();
        self.status_message = Some(match action {
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
        });
        self.refresh_git();
    }

    fn commit(&mut self, message: &str) {
        let message = message.trim();
        if message.is_empty() {
            return;
        }
        self.status_message = Some(match git::commit_all(&self.vault_root, message) {
            Ok(true) => format!("committed: {message}"),
            Ok(false) => "nothing to commit".to_string(),
            Err(e) => format!("git commit failed: {e}"),
        });
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

                self.status_message = Some(match saved {
                    Ok(()) => format!("bound {key} to {action}"),
                    Err(e) => format!("bound {key} to {action} (not saved: {e})"),
                });
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
                    self.status_message = Some(format!("query: {} rows", self.query_results.len()));
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
                    self.status_message = Some(format!("graph filter: {filter}"));
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
                                self.status_message =
                                    Some(format!("shared block created: {}", block.slug));
                            }
                            Err(error) => {
                                self.status_message =
                                    Some(format!("could not save shared block: {error}"));
                            }
                        }
                    }
                    cx.notify();
                    return;
                }

                if panel.mode == SwitcherMode::NewNoteName {
                    let name = panel.query.clone();
                    let template = self.pending_template.take();
                    self.close_panel(window, cx);
                    if let Some(template) = template {
                        self.create_from_template(&name, &template, window, cx);
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
                                        self.status_message = Some(format!("graph depth: {next}"));
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
                                    self.status_message =
                                        Some("preview opened in its own window".to_string());
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
                                    self.status_message = Some("reloaded from disk".to_string());
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
                                    self.status_message = Some("document relations".to_string());
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
                                    self.status_message = Some(if moved {
                                        "block moved up".to_string()
                                    } else {
                                        "nothing to move up".to_string()
                                    });
                                    return;
                                }
                                CommandId::MoveBlockDown => {
                                    let moved = self.editor.update(cx, |editor, cx| {
                                        let moved = editor.move_block_down();
                                        cx.notify();
                                        moved
                                    });
                                    self.close_panel(window, cx);
                                    self.status_message = Some(if moved {
                                        "block moved down".to_string()
                                    } else {
                                        "nothing to move down".to_string()
                                    });
                                    return;
                                }
                                CommandId::DuplicateBlock => {
                                    let changed = self.editor.update(cx, |editor, cx| {
                                        let changed = editor.duplicate_block();
                                        cx.notify();
                                        changed
                                    });
                                    self.close_panel(window, cx);
                                    self.status_message = Some(if changed {
                                        "block duplicated".to_string()
                                    } else {
                                        "no block here".to_string()
                                    });
                                    return;
                                }
                                CommandId::DeleteBlock => {
                                    let changed = self.editor.update(cx, |editor, cx| {
                                        let changed = editor.delete_block();
                                        cx.notify();
                                        changed
                                    });
                                    self.close_panel(window, cx);
                                    self.status_message = Some(if changed {
                                        "block deleted".to_string()
                                    } else {
                                        "no block here".to_string()
                                    });
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
}

fn flatten(node: &TreeNode, depth: usize, out: &mut Vec<Row>) {
    for child in &node.children {
        match child.kind {
            NodeKind::Dir => {
                out.push(Row {
                    depth,
                    name: child.name.clone(),
                    path: None,
                    is_dir: true,
                });
                flatten(child, depth + 1, out);
            }
            NodeKind::Note => out.push(Row {
                depth,
                name: child.name.clone(),
                path: Some(child.path.clone()),
                is_dir: false,
            }),
        }
    }
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

        let theme = *cx.global::<Theme>();

        // Stats and the outline both need the document text. Fetch it once per change
        // rather than once per consumer: copying a large note is not free on every
        // keystroke.
        let stats_version = self.editor.read(cx).document_version();
        let headings_version = (self.cache_epoch, stats_version.1);
        let stats_stale = stats_version != self.stats_version;
        let headings_stale = headings_version != self.headings_version;
        if stats_stale || headings_stale {
            let text = self.editor.read(cx).text();
            if stats_stale {
                self.stats = stats::compute(&text);
                self.stats_version = stats_version;
            }
            if headings_stale {
                self.headings = Rc::new(markdown::headings(&text));
                self.headings_version = headings_version;
            }
        }

        // A preview pane beside the editor, sharing the same document text.
        let preview: Option<AnyElement> = if self.split_preview
            && self.split_tabs.is_empty()
            && !self.show_graph
            && !self.show_diff
            && !self.show_query
            && !self.editor.read(cx).is_reading()
        {
            let version = (self.cache_epoch, self.editor.read(cx).document_version().1);
            if self.preview_content.needs_update(version) {
                let text = self.editor.read(cx).text();
                let root = self.vault_root.clone();
                self.preview_content.update(version, &text, Some(&root));
            }

            let style = window.text_style();
            let template = gpui::TextRun {
                len: 0,
                font: style.font(),
                color: style.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };

            let opener = self.link_opener(cx);
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
                        .h(px(24.0))
                        .w_full()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .rounded_sm()
                        .pl(indent)
                        .pr_2()
                        .text_sm()
                        .cursor_pointer()
                        .text_color(color)
                        .child(row.name.clone());

                    if is_active {
                        base = base.bg(theme.accent);
                    }

                    match &row.path {
                        Some(path) => {
                            let path = path.clone();
                            let me = me_for_list.clone();
                            base.hover(move |style| style.bg(theme.hover))
                                .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                                    me.update(cx, |this, cx| {
                                        this.open_note(&path, window, cx);
                                    });
                                })
                                .into_any_element()
                        }
                        None => base.font_weight(gpui::FontWeight::BOLD).into_any_element(),
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
                    .child("FILES")
                    .child(format!("{}", self.notes)),
            )
            .child(file_list);

        let headings = self.headings.clone();

        let outline: Option<AnyElement> = if headings.is_empty() {
            None
        } else {
            let me = cx.entity();
            let heading_count = headings.len();
            let rows = uniform_list("outline-list", heading_count, move |range, _window, _cx| {
                range
                    .filter_map(|i| headings.get(i).map(|heading| (i, heading)))
                    .map(|(i, heading)| {
                        let indent = px(8.0 + heading.level.saturating_sub(1) as f32 * 10.0);
                        let line = heading.line;
                        let heading_text = heading.text.clone();
                        let me = me.clone();
                        div()
                            .id(ElementId::Name(format!("outline-{i}").into()))
                            .flex()
                            .items_center()
                            .h(px(24.0))
                            .mx_1()
                            .rounded_sm()
                            .pl(indent)
                            .pr_2()
                            .text_sm()
                            .cursor_pointer()
                            .text_color(theme.text)
                            .hover(move |style| style.bg(theme.hover))
                            .child(heading.text.clone())
                            .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                                me.update(cx, |this, cx| {
                                    this.editor.update(cx, |editor, cx| {
                                        editor.jump_to_heading(
                                            line.saturating_sub(1),
                                            &heading_text,
                                            i,
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
            .min_h(px(0.0));

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
                SwitcherMode::NewNoteName => "note name:",
                SwitcherMode::SharedName => "shared block name:",
                SwitcherMode::RelationInput => "related note:",
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
            .child(self.center_view(theme, preview))
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
            }));

        div()
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
            .on_action(cx.listener(Self::close_overlay))
            .on_action(cx.listener(Self::new_window))
            .on_action(cx.listener(Self::toggle_side_by_side_action))
            .on_action(cx.listener(Self::detach_tab_action))
            .on_action(cx.listener(Self::toggle_split_editor_action))
            .on_action(cx.listener(Self::close_tab_action))
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
                    .child(
                        div()
                            .text_color(theme.muted)
                            .child(self.status_message.clone().unwrap_or_default()),
                    ),
            )
            .children(panel)
            .child(tab_bar)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(sidebar)
                    .child(resize_handle(ResizePanel::Sidebar, theme, cx))
                    .child(center)
                    .children(outline.map(|panel| {
                        let handle = resize_handle(ResizePanel::Outline, theme, cx);
                        div()
                            .flex()
                            .flex_row()
                            .h_full()
                            .child(handle)
                            .child(panel)
                            .into_any_element()
                    }))
                    .children(relations_panel.map(|panel| {
                        let handle = resize_handle(ResizePanel::Relations, theme, cx);
                        div()
                            .flex()
                            .flex_row()
                            .h_full()
                            .child(handle)
                            .child(panel)
                            .into_any_element()
                    })),
            )
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
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit Dragon Glass", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Window", NewWindow),
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
            None => (Some(PathBuf::from(argument)), None),
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
    let spawned = std::env::current_exe().and_then(|exe| {
        std::process::Command::new(exe)
            .arg(&vault_path)
            .spawn()
            .map(|_| ())
    });

    if let Err(error) = spawned {
        open_vault_picker(
            cx,
            Some(format!("Could not open {}: {error}", vault_path.display())),
            Rc::new(open_vault_in_new_process),
        );
    }
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

    let tree = tree::build(&note_refs);
    let mut initial_rows = Vec::new();
    flatten(&tree, 0, &mut initial_rows);
    let file_rows = Rc::new(initial_rows);
    let vault_root = vault.root().to_path_buf();
    let initial_session = session::load(&vault_root);
    let initial_bookmarks = bookmarks::load(&vault_root);
    let initial_user_themes = load_user_themes(&vault_root);
    let initial_theme = load_theme(&vault_root, &initial_user_themes);
    let initial_snippets = store::load_overrides(&store::snippets_dir(&vault_root));
    let initial_graph = graph::build(&note_refs);
    let initial_config = config::load(&vault_root);
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

    // A URI note wins, then the note that was open last time, then the most linked note.
    let first_rel = uri_note
        .filter(|note| {
            note_refs
                .iter()
                .any(|candidate| &candidate.relative == note)
        })
        .or_else(|| {
            initial_session.note.clone().filter(|note| {
                note_refs
                    .iter()
                    .any(|candidate| &candidate.relative == note)
            })
        })
        .or_else(|| first.map(|r| r.relative.clone()));
    let first_path = first_rel.as_ref().map(|relative| vault_root.join(relative));

    // Saved shortcut overrides win over the defaults registered above.
    for binding in &initial_bindings {
        bind_named(cx, &binding.key, &binding.action);
    }
    for binding in &initial_bindings {
        bind_named(cx, &binding.key, &binding.action);
    }

    let editor = cx.new(Editor::new);
    if let Some(path) = &first_path {
        editor.update(cx, |editor, _cx| {
            let _ = editor.load(path);
            // Handy for screenshots and for exercising the reading view.
            let reading = std::env::var("DRAGON_GLASS_READING").is_ok() || initial_session.reading;
            if reading {
                editor.set_reading_mode(true);
            }
        });
    }
    let initial_properties = editor.update(cx, |editor, _cx| editor.properties());
    let initial_cache_epoch = 1u64;
    let initial_headings_version = (initial_cache_epoch, editor.read(cx).document_version().1);
    let initial_text = editor.read(cx).text();
    let initial_headings = Rc::new(markdown::headings(&initial_text));
    let initial_stats = stats::compute(&initial_text);
    editor.update(cx, |editor, _cx| {
        editor.set_vault_root(&vault_root);
        editor.set_attachments_folder(&initial_config.attachments_folder);
    });
    let initial_tabs = vec![Tab {
        relative: first_rel.clone(),
        editor: editor.clone(),
        title: first_rel
            .as_deref()
            .map(tab_title)
            .unwrap_or_else(|| "Untitled".to_string()),
        conflict: false,
    }];
    let note_index = resolve::NoteIndex::new(&note_refs);
    cx.set_global(initial_theme);

    let bounds = Bounds::centered(None, size(px(1000.0), px(640.0)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some("Dragon Glass".into()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let app_view = cx.new(|cx| DragonGlassApp {
        editor: editor.clone(),
        tabs: initial_tabs,
        active_tab: 0,
        split_tabs: Vec::new(),
        split_axis: SplitAxis::Horizontal,
        minimal: false,
        close_requested: false,
        dragging: None,
        cache_epoch: initial_cache_epoch,
        vault_root: vault_root.clone(),
        tree: tree.clone(),
        file_rows: file_rows.clone(),
        note_refs: note_refs.clone(),
        note_index,
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
        status_message: None,
        switcher: None,
        switcher_focus: cx.focus_handle(),
        focus_handle: cx.focus_handle(),
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
    });

    let graph_view_entity = cx.new(|cx| GraphView::new(app_view.downgrade(), cx));
    app_view.update(cx, |app, cx| {
        app.graph_view = Some(graph_view_entity.clone());
        let opener = app.link_opener(cx);
        let editor = app.editor.clone();
        editor.update(cx, |editor, _| editor.set_link_opener(opener));
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
            window.on_window_should_close(cx, move |_window, cx| {
                saver.update(cx, |app, cx| app.remember_session(cx));
                true
            });
            view_for_window
        })
        .expect("failed to open window");

    let _ = window.update(cx, |view, window, cx| {
        window.focus(&view.editor.focus_handle(cx));
        cx.activate(true);
    });
    true
}
