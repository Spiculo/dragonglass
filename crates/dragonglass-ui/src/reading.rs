//! Reading mode: render parsed markdown blocks as styled GPUI elements.

use dragonglass_core::markdown::{self, Block};
use gpui::{div, prelude::*, px, relative, rgb, AnyElement, IntoElement};

pub fn reading_view(src: &str) -> impl IntoElement {
    let blocks = markdown::parse_blocks(src);
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .text_color(rgb(0xd4d4d4))
        .children(blocks.into_iter().map(render_block))
}

fn render_block(block: Block) -> AnyElement {
    match block {
        Block::Heading { level, text } => {
            let size = match level {
                1 => 30.0,
                2 => 24.0,
                3 => 20.0,
                _ => 17.0,
            };
            div()
                .text_size(px(size))
                .font_weight(gpui::FontWeight::BOLD)
                .child(text)
                .into_any_element()
        }
        Block::Paragraph(text) => div()
            .text_size(px(15.0))
            .line_height(relative(1.5))
            .child(text.replace('\n', " "))
            .into_any_element(),
        Block::ListItem {
            depth,
            marker,
            text,
            ..
        } => div()
            .flex()
            .flex_row()
            .gap_2()
            .pl(px(8.0 + depth as f32 * 16.0))
            .child(div().text_color(rgb(0x8a8a8a)).child(marker))
            .child(div().text_size(px(15.0)).child(text))
            .into_any_element(),
        Block::Code { text, .. } => div()
            .bg(rgb(0x111111))
            .p_3()
            .rounded_md()
            .text_size(px(13.0))
            .text_color(rgb(0xb8d4b8))
            .child(text)
            .into_any_element(),
        Block::Quote(text) => div()
            .border_l_1()
            .border_color(rgb(0x555555))
            .pl_3()
            .text_color(rgb(0xa8a8a8))
            .child(text.replace('\n', " "))
            .into_any_element(),
        Block::Rule => div()
            .h(px(1.0))
            .w_full()
            .bg(rgb(0x3a3a3a))
            .into_any_element(),
    }
}
