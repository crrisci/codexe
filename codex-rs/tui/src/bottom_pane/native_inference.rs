//! Provider metrics stay visible while transcript streaming hides ordinary status.

use crate::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::render::renderable::Renderable;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct NativeInference {
    pub message: String,
}

impl Renderable for NativeInference {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let line = Line::from(vec!["Xenolith · ".dim(), self.message.clone().into()]);
        truncate_line_with_ellipsis_if_overflow(line, usize::from(area.width)).render(area, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        1
    }
}
