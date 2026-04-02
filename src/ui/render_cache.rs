use ratatui::{buffer::Buffer, layout::Rect};

const SKIP_CELL: ratatui::buffer::Cell = {
    let mut cell = ratatui::buffer::Cell::EMPTY;
    let _ = cell.set_skip(true);
    cell
};

pub(crate) fn mark_area_skipped(buffer: &mut Buffer, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let width = area.width as usize;
    let buffer_width = buffer.area.width as usize;
    let offset_x = area.x.saturating_sub(buffer.area.x) as usize;
    let offset_y = area.y.saturating_sub(buffer.area.y) as usize;

    for row in 0..area.height as usize {
        let start = (offset_y + row) * buffer_width + offset_x;
        buffer.content[start..start + width].fill(SKIP_CELL.clone());
    }
}
