//! Dragon Glass UI spike (Phase 0).
//!
//! Goal: prove GPUI builds and opens a native window on this platform, and that it can
//! drive `dragonglass-core`. Everything below is throwaway scaffolding.

use dragonglass_core::{Note, Vault};
use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context, Render, Window,
    WindowBounds, WindowOptions,
};

struct DragonGlassApp {
    vault_path: String,
    notes: usize,
    links: usize,
}

impl Render for DragonGlassApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_6()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xe6e6e6))
            .child(div().text_2xl().child("Dragon Glass"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x9a9a9a))
                    .child("Phase 0 spike"),
            )
            .child(div().mt_4().child(format!("vault: {}", self.vault_path)))
            .child(div().child(format!("notes: {}", self.notes)))
            .child(div().child(format!("links: {}", self.links)))
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

    let display_path = vault_path.clone();

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(720.0), px(480.0)), cx);

        let mut options = WindowOptions::default();
        options.window_bounds = Some(WindowBounds::Windowed(bounds));
        if let Some(titlebar) = options.titlebar.as_mut() {
            titlebar.title = Some("Dragon Glass".into());
        }

        cx.open_window(options, |_, cx| {
            cx.new(|_| DragonGlassApp {
                vault_path: display_path.clone(),
                notes: note_count,
                links: link_count,
            })
        })
        .expect("failed to open window");

        cx.activate(true);
    });
}
