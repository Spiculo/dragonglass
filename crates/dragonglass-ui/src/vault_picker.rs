//! The vault chooser.
//!
//! Shown at launch when there is no vault to reopen, and from the "Open vault" command
//! while the app is running. It lists the vaults opened before (kept in
//! [`dragonglass_core::vaults`]) and can browse for a new folder.
//!
//! Opening a vault is delegated to a callback: at launch it builds the main window in this
//! process, while a running app starts a second process so every window keeps its own
//! vault, theme and keymap.

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    div, prelude::*, px, size, App, Bounds, ClickEvent, Context, FocusHandle, KeyDownEvent,
    PathPromptOptions, Render, SharedString, Window, WindowBounds, WindowOptions,
};

use dragonglass_core::vaults;

use crate::theme::Theme;

/// What to do once a folder has been chosen.
pub type OpenVault = Rc<dyn Fn(PathBuf, &mut App)>;

pub struct VaultPicker {
    registry: vaults::Registry,
    error: Option<SharedString>,
    focus_handle: FocusHandle,
    open: OpenVault,
}

impl VaultPicker {
    /// Open the chooser. `error` is shown above the list, e.g. when the last vault has
    /// been moved or deleted. Returns the window, so a late `dragonglass://` URL can
    /// replace the chooser with the vault the URL names.
    pub fn open(
        cx: &mut App,
        error: Option<String>,
        open: OpenVault,
    ) -> gpui::WindowHandle<VaultPicker> {
        // The palette is per-vault, and at launch no vault has been read yet.
        if !cx.has_global::<Theme>() {
            cx.set_global(Theme::dark());
        }

        let view = cx.new(|cx| VaultPicker {
            registry: vaults::Registry::load(),
            error: error.map(SharedString::from),
            focus_handle: cx.focus_handle(),
            open,
        });

        let bounds = Bounds::centered(None, size(px(560.0), px(520.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Dragon Glass \u{2014} Open a vault".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let focus_handle = view.read(cx).focus_handle.clone();
        let view_for_window = view.clone();
        let window = cx
            .open_window(options, move |_window, _cx| view_for_window.clone())
            .expect("failed to open the vault picker");
        let _ = window.update(cx, |_view, window, _cx| window.focus(&focus_handle));
        window
    }

    /// Validate and open `path`, then close the chooser.
    fn choose(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = vaults::canonical(&path);
        if !path.is_dir() {
            self.error = Some(format!("{} is not a folder", path.display()).into());
            cx.notify();
            return;
        }

        // The main app records the path once the vault actually opens, so a failed launch
        // does not push a broken entry to the front of the list.
        let open = self.open.clone();
        open(path, cx);
        // Keep at least one window alive while the main one is being built.
        window.remove_window();
    }

    fn forget(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Err(error) = self.registry.forget(&path) {
            self.error = Some(format!("could not update the vault list: {error}").into());
        }
        cx.notify();
    }

    fn browse(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open folder as a vault".into()),
        });

        cx.spawn_in(window, async move |this, cx| {
            let picked = match receiver.await {
                Ok(Ok(Some(mut paths))) => paths.pop(),
                Ok(Ok(None)) | Err(_) => None,
                Ok(Err(error)) => {
                    let _ = this.update_in(cx, |picker, _window, cx| {
                        picker.error =
                            Some(format!("could not open the folder picker: {error}").into());
                        cx.notify();
                    });
                    None
                }
            };

            if let Some(path) = picked {
                let _ = this.update_in(cx, |picker, window, cx| picker.choose(path, window, cx));
            }
        })
        .detach();
    }

    fn close(&mut self, _: &KeyDownEvent, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }
}

impl Render for VaultPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.global::<Theme>();
        let recents = self.registry.recent().to_vec();

        let rows = recents.iter().enumerate().map(|(index, path)| {
            let missing = !path.is_dir();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            let detail = path.to_string_lossy().to_string();
            let open_path = path.clone();
            let forget_path = path.clone();

            div()
                .id(SharedString::from(format!("vault-{index}")))
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded_md()
                .cursor_pointer()
                .when(missing, |row| row.opacity(0.55))
                .hover(move |style| style.bg(theme.hover))
                .on_click(cx.listener(move |picker, _event, window, cx| {
                    picker.choose(open_path.clone(), window, cx);
                }))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_sm().text_color(theme.text).child(name))
                        .child(div().text_xs().text_color(theme.muted).child(if missing {
                            format!("{detail} (missing)")
                        } else {
                            detail
                        })),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("vault-forget-{index}")))
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .text_xs()
                        .text_color(theme.muted)
                        .hover(move |style| style.text_color(theme.warn))
                        .child("remove")
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |picker, _event, _window, cx| {
                                cx.stop_propagation();
                                picker.forget(forget_path.clone(), cx);
                            }),
                        )
                        .on_click(move |_event, _window, cx| cx.stop_propagation()),
                )
                .into_any_element()
        });

        div()
            .id("vault-picker")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::close))
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg)
            .text_color(theme.text)
            .p(px(24.0))
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_xl().child("Open a vault"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted)
                            .child("A vault is a folder of markdown notes. Dragon Glass keeps the folder untouched."),
                    ),
            )
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(theme.chip)
                        .text_sm()
                        .text_color(theme.warn)
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child("RECENT VAULTS"),
                    )
                    .child(
                        div()
                            .id("vault-browse")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.chip)
                            .text_sm()
                            .text_color(theme.text)
                            .hover(move |style| style.bg(theme.hover))
                            .cursor_pointer()
                            .child("Browse\u{2026}")
                            .on_click(cx.listener(Self::browse)),
                    ),
            )
            .child(
                div()
                    .id("vault-recents")
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(recents.is_empty(), |el| {
                        el.child(
                            div()
                                .px_3()
                                .py_4()
                                .text_sm()
                                .text_color(theme.muted)
                                .child("No vaults yet. Choose the folder that holds your notes."),
                        )
                    })
                    .children(rows),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted)
                    .child("Tip: open several vaults at once \u{2014} each one gets its own window."),
            )
    }
}
