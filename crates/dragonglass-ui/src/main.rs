//! Dragon Glass UI spike (Phase 0/1).
//!
//! Opens a vault, loads a note into the in-house editor, and shows vault stats. Throwaway
//! wiring around `dragonglass-core` and the `editor` view.

mod editor;

use dragonglass_core::{Note, Vault};
use editor::Editor;
use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context, FocusHandle, Focusable,
    Render, Window, WindowBounds, WindowOptions,
};

struct DragonGlassApp {
    editor: gpui::Entity<Editor>,
    vault_path: String,
    notes: usize,
    links: usize,
    focus_handle: FocusHandle,
}

impl Focusable for DragonGlassApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DragonGlassApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
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
            .child(div().flex_1().w_full().p_2().child(self.editor.clone()))
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

    // Open the note with the most links, so there is something worth looking at.
    let first_note = note_refs
        .iter()
        .max_by_key(|r| Note::load(&r.path).map(|n| n.links().len()).unwrap_or(0))
        .map(|r| r.path.clone());

    let display_path = vault_path.clone();

    Application::new().run(move |cx: &mut App| {
        editor::init(cx);

        let editor = cx.new(Editor::new);
        if let Some(path) = &first_note {
            editor.update(cx, |editor, _cx| {
                let _ = editor.load(path);
            });
        }

        let bounds = Bounds::centered(None, size(px(900.0), px(600.0)), cx);
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
