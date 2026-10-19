//! Dragon Glass UI (Phase 0/1).
//!
//! Opens a vault, shows its directory tree, loads notes into the in-house editor, and
//! provides a quick switcher (Ctrl+O), full-text search (Ctrl+Shift+F) and a git menu
//! (Ctrl+G).

mod editor;
mod reading;

use std::path::PathBuf;

use dragonglass_core::{
    git, markdown, search, storage, templates, NodeKind, Note, NoteRef, TreeNode, Vault,
};
use editor::Editor;
use gpui::{
    actions, div, prelude::*, px, rgb, size, AnyElement, App, Application, Bounds, Context,
    ElementId, FocusHandle, Focusable, KeyBinding, KeyDownEvent, MouseButton, Render, Window,
    WindowBounds, WindowOptions,
};

actions!(app_actions, [OpenSwitcher, OpenSearch, OpenGit, OpenDaily]);

#[derive(Clone, Copy, PartialEq, Eq)]
enum SwitcherMode {
    QuickSwitch,
    Search,
    GitMenu,
    CommitInput,
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
}

struct DragonGlassApp {
    editor: gpui::Entity<Editor>,
    vault_root: PathBuf,
    tree: TreeNode,
    note_refs: Vec<NoteRef>,
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
        let now = chrono::Local::now();
        let date = now.format("%Y-%m-%d").to_string();
        let time = now.format("%H:%M").to_string();
        let relative = format!("Daily/{date}.md");
        let path = self.vault_root.join(&relative);

        if !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let template_path = self.vault_root.join("Templates/Daily.md");
            let content = match std::fs::read_to_string(&template_path) {
                Ok(template) => templates::render(
                    &template,
                    &[
                        ("date", date.as_str()),
                        ("time", time.as_str()),
                        ("title", date.as_str()),
                    ],
                ),
                Err(_) => format!("# {date}\n\n"),
            };

            if let Err(e) = storage::write_atomic(&path, &content) {
                self.status_message = Some(format!("could not create daily note: {e}"));
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
        }
    }

    fn open_panel(&mut self, mode: SwitcherMode, window: &mut Window, cx: &mut Context<Self>) {
        self.switcher = Some(Switcher {
            mode,
            query: String::new(),
            results: self.compute_results(mode, ""),
            selected: 0,
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

fn git_item(label: &str, detail: &str, action: GitAction) -> SwitcherItem {
    SwitcherItem {
        label: label.to_string(),
        detail: detail.to_string(),
        item: Item::Git(action),
    }
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
        let mut rows = Vec::new();
        flatten(&self.tree, 0, &mut rows);
        let me = cx.entity();
        let active = self.active.clone();

        let sidebar = div()
            .id("sidebar")
            .w(px(240.0))
            .h_full()
            .overflow_y_scroll()
            .bg(rgb(0x181818))
            .py_2()
            .children(rows.into_iter().enumerate().map(move |(i, row)| {
                let indent = px(8.0 + row.depth as f32 * 12.0);
                let is_active = !row.is_dir && row.path.as_deref() == active.as_deref();
                let color = if row.is_dir {
                    rgb(0x8a8a8a)
                } else if is_active {
                    rgb(0xffffff)
                } else {
                    rgb(0xd4d4d4)
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
                    base = base.bg(rgb(0x2a3b57));
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
                    .text_color(rgb(0xd4d4d4))
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
                    .bg(rgb(0x181818))
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
                    .child(div().text_color(rgb(0x808080)).child(item.detail.clone()));
                if i == panel.selected {
                    row = row.bg(rgb(0x2a3b57));
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
                .bg(rgb(0x232323))
                .border_b_1()
                .border_color(rgb(0x3a3a3a))
                .max_h(px(320.0))
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .text_color(rgb(0xffffff))
                        .child(format!("{prefix} {}", panel.query)),
                )
                .children(rows)
                .into_any_element()
        });

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xe6e6e6))
            .on_action(cx.listener(Self::open_switcher))
            .on_action(cx.listener(Self::open_search))
            .on_action(cx.listener(Self::open_git))
            .on_action(cx.listener(Self::open_daily))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_4()
                    .px_4()
                    .py_2()
                    .bg(rgb(0x252526))
                    .text_sm()
                    .child("Dragon Glass")
                    .child(self.vault_path.clone())
                    .child(git_label(&self.git_status))
                    .child(format!("{} notes / {} links", self.notes, self.links))
                    .child(
                        div()
                            .text_color(rgb(0xa0a0a0))
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
                    .child(div().flex_1().w_full().p_2().child(self.editor.clone()))
                    .children(outline),
            )
    }
}

fn main() {
    let vault_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "test-vault".to_string());

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
    let first_path = first.map(|r| r.path.clone());
    let first_rel = first.map(|r| r.relative.clone());

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
        ]);

        let editor = cx.new(Editor::new);
        if let Some(path) = &first_path {
            editor.update(cx, |editor, _cx| {
                let _ = editor.load(path);
            });
        }

        let bounds = Bounds::centered(None, size(px(1000.0), px(640.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Dragon Glass".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let editor_for_window = editor.clone();
        let window = cx
            .open_window(options, move |_window, cx| {
                cx.new(|cx| DragonGlassApp {
                    editor: editor_for_window.clone(),
                    vault_root: vault_root.clone(),
                    tree: tree.clone(),
                    note_refs: note_refs.clone(),
                    active: first_rel.clone(),
                    vault_path: display_path.clone(),
                    notes: note_count,
                    links: link_count,
                    git_status: initial_git.clone(),
                    status_message: None,
                    switcher: None,
                    switcher_focus: cx.focus_handle(),
                    focus_handle: cx.focus_handle(),
                })
            })
            .expect("failed to open window");

        let _ = window.update(cx, |view, window, cx| {
            window.focus(&view.editor.focus_handle(cx));
            cx.activate(true);
        });
    });
}
