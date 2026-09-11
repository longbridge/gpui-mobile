//! GPUI Kit's Markdown TextView rendered by the native mobile platform.

use gpui_kit::component::{text::TextView, ActiveTheme};
use gpui_kit::{div, prelude::*, App};

const MARKDOWN: &str = include_str!("markdown.md");

pub fn render(cx: &App) -> impl IntoElement {
    div()
        .w_full()
        .min_w_0()
        .p_4()
        .bg(cx.theme().background)
        .text_color(cx.theme().foreground)
        .child(
            TextView::markdown("kit-markdown-example", MARKDOWN)
                .selectable(true)
                .scrollable(false),
        )
}
