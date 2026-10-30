//! A detached window showing the reading view of a note, so a preview can sit on another
//! monitor while the editor stays put.

use std::path::PathBuf;

use gpui::{
    div, prelude::*, px, size, App, Bounds, Context, Entity, FocusHandle, Render, Subscription,
    TextRun, Window, WindowBounds, WindowOptions,
};

use crate::editor::Editor;
use crate::reading::{LinkOpener, ReadingCache};
use crate::theme::Theme;

pub struct PreviewWindow {
    editor: Entity<Editor>,
    vault_root: PathBuf,
    reading: ReadingCache,
    opener: LinkOpener,
    /// Keeps the preview in sync while the note is edited.
    _editor_subscription: Option<Subscription>,
    focus_handle: FocusHandle,
}

impl PreviewWindow {
    /// Open a preview window for `editor`.
    pub fn open(editor: Entity<Editor>, vault_root: PathBuf, opener: LinkOpener, cx: &mut App) {
        let view = cx.new(|cx| PreviewWindow {
            editor: editor.clone(),
            vault_root,
            reading: ReadingCache::new(),
            opener,
            _editor_subscription: None,
            focus_handle: cx.focus_handle(),
        });

        let view_for_observer = view.clone();
        let subscription = cx.observe(&editor, move |editor, cx| {
            let version = editor.read(cx).document_version();
            view_for_observer.update(cx, |this, cx| {
                if this.reading.needs_update(version) {
                    cx.notify();
                }
            });
        });
        view.update(cx, |this, _| this._editor_subscription = Some(subscription));

        let bounds = Bounds::centered(None, size(px(720.0), px(820.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Dragon Glass \u{2014} Preview".into()),
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

        let version = self.editor.read(cx).document_version();
        if self.reading.needs_update(version) {
            let text = self.editor.read(cx).text();
            let root = self.vault_root.clone();
            self.reading.update(version, &text, Some(&root));
        }

        div()
            .id("preview-window")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(theme.bg)
            .text_color(theme.text)
            .child(self.reading.element(
                Some(self.vault_root.clone()),
                theme,
                template,
                Some(self.opener.clone()),
                cx,
            ))
    }
}
