//! Dragon Glass UI spike (Phase 0/1).
//!
//! Opens a vault, shows its directory tree, and loads notes into the in-house editor.

mod editor;

use std::path::PathBuf;

use dragonglass_core::{NodeKind, Note, TreeNode, Vault};
use editor::Editor;
use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context, ElementId, FocusHandle,
    Focusable, MouseButton, Render, Window, WindowBounds, WindowOptions,
};

struct DragonGlassApp {
    editor: gpui::Entity<Editor>,
    vault_root: PathBuf,
    tree: TreeNode,
    active: Option<String>,
    vault_path: String,
    notes: usize,
    links: usize,
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

impl Render for DragonGlassApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = Vec::new();
        flatten(&self.tree, 0, &mut rows);
        let me = cx.entity();
        let active = self.active.clone();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xe6e6e6))
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
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(
                        div()
                            .id("sidebar")
                            .w(px(240.0))
                            .h_full()
                            .overflow_y_scroll()
                            .bg(rgb(0x181818))
                            .py_2()
                            .children(rows.into_iter().enumerate().map(move |(i, row)| {
                                let indent = px(8.0 + row.depth as f32 * 12.0);
                                let mut base = div()
                                    .id(ElementId::Name(format!("row-{i}").into()))
                                    .pl(indent)
                                    .pr_2()
                                    .py_1()
                                    .text_sm()
                                    .cursor_pointer()
                                    .child(row.name.clone());

                                if row.is_dir {
                                    base = base.text_color(rgb(0x8a8a8a));
                                } else if row.path.as_deref() == active.as_deref() {
                                    base = base.bg(rgb(0x2a3b57)).text_color(rgb(0xffffff));
                                } else {
                                    base = base.text_color(rgb(0xd4d4d4));
                                }

                                match row.path {
                                    Some(path) => {
                                        let me = me.clone();
                                        base.on_mouse_down(
                                            MouseButton::Left,
                                            move |_ev, _window, cx| {
                                                me.update(cx, |this, cx| {
                                                    this.open_note(&path, cx);
                                                });
                                            },
                                        )
                                    }
                                    None => base,
                                }
                            })),
                    )
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
                    active: first_rel.clone(),
                    vault_path: display_path.clone(),
                    notes: note_count,
                    links: link_count,
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
