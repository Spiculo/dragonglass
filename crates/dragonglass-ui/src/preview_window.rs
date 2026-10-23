//! A detached window showing the reading view of a note, so a preview can sit on another
//! monitor while the editor stays put.

use std::path::PathBuf;

use gpui::{
    div, prelude::*, px, size, App, Bounds, Context, Entity, FocusHandle, Render, TextRun, Window,
    WindowBounds, WindowOptions,
};

use crate::editor::Editor;
use crate::theme::Theme;

pub struct PreviewWindow {
    editor: Entity<Editor>,
    vault_root: PathBuf,
    focus_handle: FocusHandle,
}

impl PreviewWindow {
    /// Open a preview window for `editor`.
    pub fn open(editor: Entity<Editor>, vault_root: PathBuf, cx: &mut App) {
        let view = cx.new(|cx| PreviewWindow {
            editor,
            vault_root,
            focus_handle: cx.focus_handle(),
        });

        let bounds = Bounds::centered(None, size(px(720.0), px(820.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Dragon Glass — Preview".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let view_for_window = view.clone();
        let _ = cx.open_window(options, move |_window, _cx| view_for_window.clone());
    }
}

impl Render for PreviewWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        let style = window.text_style();
        let template = TextRun {
            len: 0,
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        let text = self.editor.read(cx).text();

        div()
            .id("preview-window")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .bg(theme.bg)
            .text_color(theme.text)
            .child(crate::reading::reading_view(
                &text,
                Some(&self.vault_root),
                0,
                theme,
                &template,
            ))
    }
}
