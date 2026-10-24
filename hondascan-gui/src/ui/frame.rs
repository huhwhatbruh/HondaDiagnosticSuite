//! A borderless window has no frame to grab, so the edges and corners of the window start the resize themselves
//! (the window manager does the dragging: `ViewportCommand::BeginResize`).
use eframe::egui::{Context, CursorIcon, ResizeDirection, ViewportCommand};

const EDGE: f32 = 6.0;
const CORNER: f32 = 14.0;

pub fn resize_edges(ctx: &Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)) { return; }
    let Some(p) = ctx.pointer_latest_pos() else { return };
    let r = ctx.content_rect();
    let (l, rt, t, b) = (p.x - r.left() < EDGE, r.right() - p.x < EDGE, p.y - r.top() < EDGE, r.bottom() - p.y < EDGE);
    let (lc, rc, tc, bc) = (p.x - r.left() < CORNER, r.right() - p.x < CORNER, p.y - r.top() < CORNER, r.bottom() - p.y < CORNER);
    use ResizeDirection::*;
    let (dir, cur) = match () {
        _ if (l && tc) || (t && lc) => (NorthWest, CursorIcon::ResizeNorthWest),
        _ if (rt && tc) || (t && rc) => (NorthEast, CursorIcon::ResizeNorthEast),
        _ if (l && bc) || (b && lc) => (SouthWest, CursorIcon::ResizeSouthWest),
        _ if (rt && bc) || (b && rc) => (SouthEast, CursorIcon::ResizeSouthEast),
        _ if l => (West, CursorIcon::ResizeWest),
        _ if rt => (East, CursorIcon::ResizeEast),
        _ if t => (North, CursorIcon::ResizeNorth),
        _ if b => (South, CursorIcon::ResizeSouth),
        _ => return,
    };
    ctx.set_cursor_icon(cur);
    if ctx.input(|i| i.pointer.primary_pressed()) { ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir)); }
}
