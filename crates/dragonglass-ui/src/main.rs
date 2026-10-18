//! Dragon Glass UI (Phase 0/1).
//!
//! Opens a vault, shows its directory tree, loads notes into the in-house editor, and
//! provides a quick switcher (Ctrl+O) and full-text search (Ctrl+Shift+F).

mod editor;
mod reading;

use std::path::PathBuf;

use dragonglass_core::{search, NodeKind, Note, NoteRef, TreeNode, Vault};
use editor::Editor;
use gpui::{
    actions, div, prelude::*, px, rgb, size, AnyElement, App, Application, Bounds, Context,
    ElementId, FocusHandle, Focusable, KeyBinding, KeyDownEvent, MouseButton, Render, Window,
    WindowBounds, WindowOptions,
};

actions!(app_actions, [OpenSwitcher, OpenSearch]);

#[derive(Clone, Copy, PartialEq, Eq)]
enum SwitcherMode {
    QuickSwitch,
    Search,
}

struct SwitcherItem {
    label: String,
    detail: String,
    relative: String,
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

    fn compute_results(&self, mode: SwitcherMode, query: &str) -> Vec<SwitcherItem> {
        match mode {
            SwitcherMode::QuickSwitch => search::fuzzy_notes(&self.note_refs, query, 20)
                .into_iter()
                .map(|note| SwitcherItem {
                    label: note.stem().to_string(),
                    detail: note.relative.clone(),
                    relative: note.relative.clone(),
                })
                .collect(),
            SwitcherMode::Search => search::full_text(&self.note_refs, query, 50)
                .into_iter()
                .map(|hit| SwitcherItem {
                    label: format!("{}:{}", hit.relative, hit.line_number),
                    detail: hit.line,
                    relative: hit.relative,
                })
                .collect(),
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
                if let Some(item) = panel.results.get(panel.selected) {
                    let relative = item.relative.clone();
                    self.open_note(&relative, cx);
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

fn label_color(is_dir: bool, is_active: bool) -> gpui::Hsla {
    if is_dir {
        rgb(0x8a8a8a).into()
    } else if is_active {
        rgb(0xffffff).into()
    } else {
        rgb(0xd4d4d4).into()
    }
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
                let mut base = div()
                    .id(ElementId::Name(format!("row-{i}").into()))
                    .pl(indent)
                    .pr_2()
                    .py_1()
                    .text_sm()
                    .cursor_pointer()
                    .text_color(label_color(row.is_dir, is_active))
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

        let panel: Option<AnyElement> = self.switcher.as_ref().map(|panel| {
            let prefix = match panel.mode {
                SwitcherMode::QuickSwitch => ">",
                SwitcherMode::Search => "find:",
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
                    .child(format!("{} notes / {} links", self.notes, self.links)),
            )
            .children(panel)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(sidebar)
                    .child(div().flex_1().w_full().p_2().child(self.editor.clone())),
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
