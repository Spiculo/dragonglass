//! Dragon Glass UI (Phase 0/1).
//!
//! Opens a vault, shows its directory tree, loads notes into the in-house editor, and
//! provides a quick switcher (Ctrl+O), full-text search (Ctrl+Shift+F) and a git menu
//! (Ctrl+G).

mod editor;
mod graph_view;
mod reading;
mod theme;

use std::path::{Path, PathBuf};

use dragonglass_core::frontmatter::{Property, PropertyValue};
use dragonglass_core::{
    bookmarks, config, conflict, git, graph, keymap, markdown, query, search, storage, store,
    templates, update, uri, workspace, NodeKind, Note, NoteRef, TreeNode, Vault,
};

/// Actions the hotkey editor can rebind: `(action, default key, label)`.
const HOTKEYS: &[(&str, &str, &str)] = &[
    ("OpenCommands", "ctrl+p", "Command palette"),
    ("OpenSwitcher", "ctrl+o", "Quick switcher"),
    ("OpenSearch", "ctrl+shift+f", "Search"),
    ("OpenGraph", "ctrl+shift+g", "Graph view"),
    ("OpenDaily", "ctrl+shift+d", "Daily note"),
    ("OpenWeekly", "ctrl+shift+w", "Weekly note"),
    ("OpenMonthly", "ctrl+shift+m", "Monthly note"),
    ("OpenProperties", "ctrl+shift+p", "Properties"),
    ("ToggleBookmark", "ctrl+shift+b", "Toggle bookmark"),
    ("OpenGit", "ctrl+g", "Git"),
    ("ToggleTheme", "ctrl+shift+l", "Toggle theme"),
    ("NewWindow", "ctrl+n", "New window"),
];

/// Bind a saved override by action name.
fn bind_named(cx: &mut App, key: &str, action: &str) {
    match action {
        "OpenCommands" => cx.bind_keys([KeyBinding::new(key, OpenCommands, None)]),
        "OpenSwitcher" => cx.bind_keys([KeyBinding::new(key, OpenSwitcher, None)]),
        "OpenSearch" => cx.bind_keys([KeyBinding::new(key, OpenSearch, None)]),
        "OpenGraph" => cx.bind_keys([KeyBinding::new(key, OpenGraph, None)]),
        "OpenDaily" => cx.bind_keys([KeyBinding::new(key, OpenDaily, None)]),
        "OpenWeekly" => cx.bind_keys([KeyBinding::new(key, OpenWeekly, None)]),
        "OpenMonthly" => cx.bind_keys([KeyBinding::new(key, OpenMonthly, None)]),
        "OpenProperties" => cx.bind_keys([KeyBinding::new(key, OpenProperties, None)]),
        "ToggleBookmark" => cx.bind_keys([KeyBinding::new(key, ToggleBookmark, None)]),
        "OpenGit" => cx.bind_keys([KeyBinding::new(key, OpenGit, None)]),
        "ToggleTheme" => cx.bind_keys([KeyBinding::new(key, ToggleTheme, None)]),
        "NewWindow" => cx.bind_keys([KeyBinding::new(key, NewWindow, None)]),
        _ => {}
    }
}
use dragonglass_plugins as plugins;
use editor::Editor;
use gpui::{
    actions, div, prelude::*, px, size, AnyElement, App, Application, Bounds, Context, ElementId,
    FocusHandle, Focusable, KeyBinding, KeyDownEvent, MouseButton, Render, Window, WindowBounds,
    WindowOptions,
};
use graph_view::GraphView;
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
        OpenWeekly,
        OpenMonthly,
        CloseQuery,
        RunQuery,
        NewWindow
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
    Hotkeys,
    Plugin(usize),
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

struct DragonGlassApp {
    editor: gpui::Entity<Editor>,
    vault_root: PathBuf,
    tree: TreeNode,
    note_refs: Vec<NoteRef>,
    properties: Vec<Property>,
    bookmarks: Vec<String>,
    theme: Theme,
    conflict: bool,
    watcher: Option<notify::RecommendedWatcher>,
    update_source: Option<String>,
    update_available: Option<String>,
    full_graph: graph::Graph,
    graph_view: Option<gpui::Entity<GraphView>>,
    show_graph: bool,
    query_results: Vec<query::Row>,
    show_query: bool,
    /// Installed plugins, and a flattened list of `(plugin index, command id, title)`.
    plugins: Vec<plugins::PluginInfo>,
    plugin_commands: Vec<(usize, String, String)>,
    workspaces: Vec<workspace::Workspace>,
    user_themes: Vec<(String, Theme)>,
    bindings: Vec<keymap::Binding>,
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
    fn open_note(&mut self, relative: &str, cx: &mut Context<Self>) {
        let path = self.vault_root.join(relative);
        self.editor.update(cx, |editor, cx| {
            // Don't lose edits when switching notes.
            if editor.is_dirty() {
                let _ = editor.save();
            }
            if editor.load(&path).is_ok() {
                cx.notify();
            }
        });
        self.active = Some(relative.to_string());
        self.conflict = false;
        self.refresh_properties(cx);
        cx.notify();
    }

    /// React to a note changing on disk while it is open.
    fn on_external_change(&mut self, changed: &str, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else {
            return;
        };
        let active_path = self.vault_root.join(&active);

        let same_file = match (
            active_path.canonicalize(),
            Path::new(changed).canonicalize(),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => active_path.to_string_lossy() == changed,
        };
        if !same_file {
            return;
        }

        eprintln!("[dragon-glass] external change to open note: {changed}");

        let Ok(disk) = std::fs::read_to_string(&active_path) else {
            return;
        };
        let (buffer, dirty) = {
            let editor = self.editor.read(cx);
            (editor.text(), editor.is_dirty())
        };

        match conflict::classify(&buffer, dirty, &disk) {
            conflict::Resolution::Ignore => {}
            conflict::Resolution::Reload => {
                self.editor.update(cx, |editor, cx| {
                    let _ = editor.reload();
                    cx.notify();
                });
                self.conflict = false;
                self.status_message = Some("reloaded from disk (changed externally)".to_string());
                cx.notify();
            }
            conflict::Resolution::Conflict => {
                self.conflict = true;
                self.status_message =
                    Some("changed on disk — saving will overwrite the file".to_string());
                cx.notify();
            }
        }
    }

    fn refresh_git(&mut self) {
        self.git_status = if git::is_repo(&self.vault_root) {
            git::status(&self.vault_root).ok()
        } else {
            None
        };
    }

    /// Re-scan the vault after files are created or removed.
    fn refresh_vault(&mut self) {
        if let Ok(vault) = Vault::open(&self.vault_root) {
            self.note_refs = vault.notes();
            self.tree = vault.tree();
            self.notes = self.note_refs.len();
        }
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
            self.refresh_vault();
        }

        self.open_note(&relative, cx);
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
                    detail: keymap::key_for_action(&self.bindings, action)
                        .unwrap_or_else(|| (*default).to_string()),
                    item: Item::Hotkey((*action).to_string()),
                })
                .collect(),
            SwitcherMode::HotkeyCapture => Vec::new(),
            SwitcherMode::CommandsMenu => vec![
                command_item("Quick switcher", "ctrl+o", CommandId::QuickSwitch),
                command_item("Search", "ctrl+shift+f", CommandId::Search),
                command_item("Daily note", "ctrl+shift+d", CommandId::Daily),
                command_item("Weekly note", "ctrl+shift+w", CommandId::Weekly),
                command_item("Monthly note", "ctrl-shift-m", CommandId::Monthly),
                command_item("Properties", "ctrl+shift+p", CommandId::Properties),
                command_item("Toggle bookmark", "ctrl+shift+b", CommandId::Bookmark),
                command_item("Git", "ctrl+g", CommandId::Git),
                command_item("Toggle reading mode", "ctrl+e", CommandId::ToggleReading),
                command_item("Save", "ctrl+s", CommandId::Save),
                command_item("Toggle theme", "ctrl+shift+l", CommandId::ToggleTheme),
                command_item("Check for updates", "", CommandId::CheckUpdates),
                command_item("Graph view", "ctrl+shift+g", CommandId::OpenGraph),
                command_item("Run a query", "", CommandId::RunQuery),
                command_item("Save workspace", "", CommandId::SaveWorkspace),
                command_item("Keyboard shortcuts", "", CommandId::Hotkeys),
                command_item("Load workspace", "", CommandId::LoadWorkspace),
                command_item("New window", "ctrl+n", CommandId::NewWindow),
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
            self.open_note(note, cx);
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
        window.focus(&view.focus_handle(cx));
        cx.notify();
    }

    fn close_graph(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.show_graph {
            return;
        }
        self.show_graph = false;
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    /// The centre pane: the graph when open, otherwise the editor.
    fn center_view(&self, theme: Theme) -> AnyElement {
        if self.show_query {
            return query_view(&self.query_results, theme);
        }
        if self.show_graph {
            if let Some(view) = &self.graph_view {
                return view.clone().into_any_element();
            }
        }
        self.editor.clone().into_any_element()
    }

    fn close_query(&mut self, _: &CloseQuery, window: &mut Window, cx: &mut Context<Self>) {
        if !self.show_query {
            return;
        }
        self.show_query = false;
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    fn new_window(&mut self, _: &NewWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.open_new_window(window, cx);
    }

    /// Open another window on the same vault, with its own editor and panes.
    fn open_new_window(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let editor = cx.new(Editor::new);
        editor.update(cx, |editor, _cx| {
            editor.set_vault_root(&self.vault_root);
            if let Some(active) = &self.active {
                let _ = editor.load(&self.vault_root.join(active));
            }
        });

        let sibling = cx.new(|cx| DragonGlassApp {
            editor: editor.clone(),
            vault_root: self.vault_root.clone(),
            tree: self.tree.clone(),
            note_refs: self.note_refs.clone(),
            properties: Vec::new(),
            bookmarks: self.bookmarks.clone(),
            theme: self.theme,
            active: self.active.clone(),
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
            query_results: Vec::new(),
            show_query: false,
            plugins: self.plugins.clone(),
            plugin_commands: self.plugin_commands.clone(),
            workspaces: self.workspaces.clone(),
            user_themes: self.user_themes.clone(),
            bindings: self.bindings.clone(),
            config: self.config.clone(),
        });

        let graph_view = cx.new(|cx| GraphView::new(sibling.downgrade(), cx));
        sibling.update(cx, |app, _cx| {
            app.graph_view = Some(graph_view);
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

        let view = sibling.clone();
        let _ = cx.open_window(options, move |_window, _cx| view.clone());
    }

    fn apply_user_theme(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some((name, theme)) = self.user_themes.get(index).cloned() else {
            return;
        };

        self.theme = theme;
        cx.set_global(theme);
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
        self.theme = self.theme.toggled();
        cx.set_global(self.theme);
        let message = match save_theme(&self.vault_root, self.theme, None) {
            Ok(()) => format!("theme: {}", self.theme.name()),
            Err(e) => format!("theme: {} (not saved: {e})", self.theme.name()),
        };
        self.status_message = Some(message);
        self.editor.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn refresh_properties(&mut self, cx: &mut Context<Self>) {
        self.properties = self.editor.read(cx).properties();
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
                    self.query_results = query::run(&self.note_refs, &expression);
                    self.show_query = true;
                    self.show_graph = false;
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

                if let Some(item) = panel.results.get(panel.selected) {
                    match &item.item {
                        Item::Note { relative } => {
                            let relative = relative.clone();
                            self.open_note(&relative, cx);
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
                                CommandId::ToggleReading => self.editor.update(cx, |editor, cx| {
                                    editor.toggle_reading_mode();
                                    cx.notify();
                                }),
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

fn query_view(rows: &[query::Row], theme: Theme) -> AnyElement {
    let mut items: Vec<AnyElement> = Vec::with_capacity(rows.len() + 1);
    items.push(
        div()
            .flex()
            .flex_row()
            .gap_4()
            .px_3()
            .py_1()
            .text_xs()
            .text_color(theme.muted)
            .child(div().w(px(340.0)).child("note"))
            .child("properties")
            .into_any_element(),
    );

    for row in rows {
        let properties = row
            .values
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("  ");

        items.push(
            div()
                .flex()
                .flex_row()
                .gap_4()
                .px_3()
                .py_1()
                .text_sm()
                .child(div().w(px(340.0)).child(row.relative.clone()))
                .child(properties)
                .into_any_element(),
        );
    }

    div()
        .id("query-scroll")
        .flex()
        .flex_col()
        .size_full()
        .overflow_y_scroll()
        .py_2()
        .child(
            div()
                .px_3()
                .py_1()
                .text_xs()
                .text_color(theme.star)
                .child(format!("{} rows", rows.len())),
        )
        .children(items)
        .into_any_element()
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
        detail: hint.to_string(),
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();

        // A conflict is resolved once the editor is clean again (saved or reloaded).
        if self.conflict && !self.editor.read(cx).is_dirty() {
            self.conflict = false;
        }

        let mut rows = Vec::new();
        flatten(&self.tree, 0, &mut rows);
        let me = cx.entity();
        let active = self.active.clone();

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
                    .pl(px(16.0))
                    .pr_2()
                    .py_1()
                    .text_sm()
                    .cursor_pointer()
                    .text_color(theme.star)
                    .child(format!("\u{2605} {name}"))
                    .on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                        me.update(cx, |this, cx| {
                            this.open_note(&relative, cx);
                        });
                    })
                    .into_any_element()
            })
            .collect();

        let sidebar = div()
            .id("sidebar")
            .w(px(240.0))
            .h_full()
            .overflow_y_scroll()
            .bg(theme.panel)
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
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(theme.muted)
                    .child("FILES"),
            )
            .children(rows.into_iter().enumerate().map(move |(i, row)| {
                let indent = px(8.0 + row.depth as f32 * 12.0);
                let is_active = !row.is_dir && row.path.as_deref() == active.as_deref();
                let color = if row.is_dir {
                    theme.muted
                } else if is_active {
                    theme.accent_text
                } else {
                    theme.text
                };

                let mut base = div()
                    .id(ElementId::Name(format!("row-{i}").into()))
                    .pl(indent)
                    .pr_2()
                    .py_1()
                    .text_sm()
                    .cursor_pointer()
                    .text_color(color)
                    .child(row.name.clone());

                if is_active {
                    base = base.bg(theme.accent);
                }

                match row.path {
                    Some(path) => {
                        let me = me.clone();
                        base.on_mouse_down(MouseButton::Left, move |_ev, _window, cx| {
                            me.update(cx, |this, cx| {
                                this.open_note(&path, cx);
                            });
                        })
                    }
                    None => base,
                }
            }));

        let headings = {
            let editor = self.editor.read(cx);
            markdown::headings(&editor.text())
        };

        let outline: Option<AnyElement> = if headings.is_empty() {
            None
        } else {
            let me = cx.entity();
            let rows = headings.iter().enumerate().map(|(i, heading)| {
                let indent = px(8.0 + heading.level.saturating_sub(1) as f32 * 10.0);
                let line = heading.line;
                let me = me.clone();
                div()
                    .id(ElementId::Name(format!("outline-{i}").into()))
                    .pl(indent)
                    .pr_2()
                    .py_1()
                    .text_sm()
                    .cursor_pointer()
                    .text_color(theme.text)
                    .child(heading.text.clone())
                    .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
                        me.update(cx, |this, cx| {
                            this.editor.update(cx, |editor, cx| {
                                editor.jump_to_line(line.saturating_sub(1));
                                cx.notify();
                            });
                            window.focus(&this.editor.focus_handle(cx));
                            cx.notify();
                        });
                    })
            });

            Some(
                div()
                    .id("outline")
                    .w(px(200.0))
                    .h_full()
                    .overflow_y_scroll()
                    .bg(theme.panel)
                    .py_2()
                    .children(rows)
                    .into_any_element(),
            )
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
            };
            let rows = panel.results.iter().enumerate().map(|(i, item)| {
                let mut row = div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .px_3()
                    .py_1()
                    .text_sm()
                    .child(item.label.clone())
                    .child(div().text_color(theme.muted).child(item.detail.clone()));
                if i == panel.selected {
                    row = row.bg(theme.accent);
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
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .text_color(theme.text)
                        .child(format!("{prefix} {}", panel.query)),
                )
                .children(rows)
                .into_any_element()
        });

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg)
            .text_color(theme.text)
            .on_action(cx.listener(Self::open_switcher))
            .on_action(cx.listener(Self::open_search))
            .on_action(cx.listener(Self::open_git))
            .on_action(cx.listener(Self::open_daily))
            .on_action(cx.listener(Self::open_weekly))
            .on_action(cx.listener(Self::open_monthly))
            .on_action(cx.listener(Self::open_properties))
            .on_action(cx.listener(Self::toggle_bookmark))
            .on_action(cx.listener(Self::open_commands))
            .on_action(cx.listener(Self::toggle_theme))
            .on_action(cx.listener(Self::close_query))
            .on_action(cx.listener(Self::new_window))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_4()
                    .px_4()
                    .py_2()
                    .bg(theme.header)
                    .text_sm()
                    .child("Dragon Glass")
                    .child(self.vault_path.clone())
                    .child(git_label(&self.git_status))
                    .child(format!("{} notes / {} links", self.notes, self.links))
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
                    .child(
                        div()
                            .text_color(theme.muted)
                            .child(self.status_message.clone().unwrap_or_default()),
                    ),
            )
            .children(panel)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(sidebar)
                    .child(div().flex_1().w_full().p_2().child(self.center_view(theme)))
                    .children(outline),
            )
    }
}

fn main() {
    // The first argument may be a vault path or a `dragonglass://` URI, so the app can be
    // registered as the OS handler for the scheme.
    let argument = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "test-vault".to_string());
    let (vault_path, uri_note) = match uri::parse(&argument) {
        Some(parsed) => (
            parsed.vault.clone().unwrap_or_else(|| ".".to_string()),
            parsed.note.clone(),
        ),
        None => (argument, None),
    };

    let vault = match Vault::open(&vault_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("could not open vault {vault_path:?}: {e}");
            std::process::exit(1);
        }
    };

    let note_refs = vault.notes();
    let note_count = note_refs.len();
    let link_count: usize = note_refs
        .iter()
        .filter_map(|r| Note::load(&r.path).ok())
        .map(|n| n.links().len())
        .sum();

    let tree = vault.tree();
    let vault_root = vault.root().to_path_buf();
    let initial_bookmarks = bookmarks::load(&vault_root);
    let initial_user_themes = load_user_themes(&vault_root);
    let initial_theme = load_theme(&vault_root, &initial_user_themes);
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
    let display_path = vault_path.clone();

    // Open the note with the most links, so there is something worth looking at.
    let first = note_refs
        .iter()
        .max_by_key(|r| Note::load(&r.path).map(|n| n.links().len()).unwrap_or(0));

    // A URI note wins over the default; otherwise open the note with the most links.
    let first_rel = uri_note
        .filter(|note| {
            note_refs
                .iter()
                .any(|candidate| &candidate.relative == note)
        })
        .or_else(|| first.map(|r| r.relative.clone()));
    let first_path = first_rel.as_ref().map(|relative| vault_root.join(relative));

    Application::new().run(move |cx: &mut App| {
        editor::init(cx);
        cx.bind_keys([
            KeyBinding::new("ctrl-o", OpenSwitcher, None),
            KeyBinding::new("cmd-o", OpenSwitcher, None),
            KeyBinding::new("ctrl-shift-f", OpenSearch, None),
            KeyBinding::new("cmd-shift-f", OpenSearch, None),
            KeyBinding::new("ctrl-g", OpenGit, None),
            KeyBinding::new("cmd-g", OpenGit, None),
            KeyBinding::new("ctrl-shift-d", OpenDaily, None),
            KeyBinding::new("cmd-shift-d", OpenDaily, None),
            KeyBinding::new("ctrl-shift-w", OpenWeekly, None),
            KeyBinding::new("cmd-shift-w", OpenWeekly, None),
            KeyBinding::new("ctrl-shift-m", OpenMonthly, None),
            KeyBinding::new("cmd-shift-m", OpenMonthly, None),
            KeyBinding::new("ctrl-shift-p", OpenProperties, None),
            KeyBinding::new("cmd-shift-p", OpenProperties, None),
            KeyBinding::new("ctrl-shift-b", ToggleBookmark, None),
            KeyBinding::new("cmd-shift-b", ToggleBookmark, None),
            KeyBinding::new("ctrl-p", OpenCommands, None),
            KeyBinding::new("cmd-p", OpenCommands, None),
            KeyBinding::new("ctrl-shift-l", ToggleTheme, None),
            KeyBinding::new("cmd-shift-l", ToggleTheme, None),
            KeyBinding::new("ctrl-shift-g", OpenGraph, None),
            KeyBinding::new("cmd-shift-g", OpenGraph, None),
            KeyBinding::new("escape", CloseQuery, None),
            KeyBinding::new("ctrl-n", NewWindow, None),
            KeyBinding::new("cmd-n", NewWindow, None),
        ]);
        graph_view::init(cx);

        // Saved shortcut overrides win over the defaults registered above.
        for binding in &initial_bindings {
            bind_named(cx, &binding.key, &binding.action);
        }

        let editor = cx.new(Editor::new);
        if let Some(path) = &first_path {
            editor.update(cx, |editor, _cx| {
                let _ = editor.load(path);
            });
        }
        let initial_properties = editor.read(cx).properties();
        editor.update(cx, |editor, _cx| editor.set_vault_root(&vault_root));
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
            vault_root: vault_root.clone(),
            tree: tree.clone(),
            note_refs: note_refs.clone(),
            properties: initial_properties,
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
            query_results: Vec::new(),
            show_query: false,
            plugins: initial_plugins,
            plugin_commands: initial_plugin_commands,
            workspaces: initial_workspaces,
            user_themes: initial_user_themes,
            bindings: initial_bindings,
            config: initial_config,
        });

        let graph_view_entity = cx.new(|cx| GraphView::new(app_view.downgrade(), cx));
        app_view.update(cx, |app, _cx| {
            app.graph_view = Some(graph_view_entity.clone());
        });

        // Watch the vault so external edits to the open note are noticed. This is
        // event-driven: the watcher thread blocks, so idle CPU stays at zero.
        let (change_tx, change_rx) = smol::channel::unbounded::<String>();
        let watcher =
            match notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event {
                    for path in event.paths {
                        if path
                            .extension()
                            .map(|e| e.eq_ignore_ascii_case("md"))
                            .unwrap_or(false)
                        {
                            let _ = change_tx.send_blocking(path.to_string_lossy().to_string());
                        }
                    }
                }
            }) {
                Ok(mut watcher) => {
                    use notify::Watcher as _;
                    let _ = watcher.watch(&vault_root, notify::RecursiveMode::Recursive);
                    Some(watcher)
                }
                Err(_) => None,
            };

        app_view.update(cx, |this, cx| {
            this.watcher = watcher;
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
        });

        let view_for_window = app_view.clone();
        let window = cx
            .open_window(options, move |_window, _cx| view_for_window.clone())
            .expect("failed to open window");

        let _ = window.update(cx, |view, window, cx| {
            window.focus(&view.editor.focus_handle(cx));
            cx.activate(true);
        });
    });
}
