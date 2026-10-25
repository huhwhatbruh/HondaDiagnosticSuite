//! The window's own frame, drawn by the app (the window has no native one): rounded corners and a rim as a Mac window has
//! (HIG Windows: custom frames must match the system's look), square when maximised; and the edges and corners start a
//! resize themselves (the window manager does the dragging: `ViewportCommand::BeginResize`).
use crate::theme::{BACKGROUND, OUTLINE, RADIUS_FRAME};
use eframe::egui::{Context, CornerRadius, CursorIcon, LayerId, Order, ResizeDirection, Stroke, StrokeKind, ViewportCommand};

fn filling_screen(ctx: &Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false))
}

/// The corner radius of the window: 0 when it fills the screen.
pub fn radius(ctx: &Context) -> u8 { if filling_screen(ctx) { 0 } else { RADIUS_FRAME } }

/// Only the given corners of the window's radius, for a panel that sits in those corners.
pub fn corners(ctx: &Context, nw: bool, ne: bool, sw: bool, se: bool) -> CornerRadius {
    let r = radius(ctx);
    let c = |on: bool| if on { r } else { 0 };
    CornerRadius { nw: c(nw), ne: c(ne), sw: c(sw), se: c(se) }
}

/// The window's floor, under every panel: the surface is transparent, so outside this rounded rectangle the desktop shows.
pub fn backdrop(ctx: &Context) {
    ctx.layer_painter(LayerId::background()).rect_filled(ctx.content_rect(), radius(ctx), BACKGROUND);
}

/// The rim around the whole window, above everything.
pub fn rim(ctx: &Context) {
    if filling_screen(ctx) { return; }
    ctx.layer_painter(LayerId::new(Order::Foreground, "window-rim".into()))
        .rect_stroke(ctx.content_rect(), RADIUS_FRAME, Stroke::new(1.0, OUTLINE), StrokeKind::Inside);
}

const EDGE: f32 = 6.0;
const CORNER: f32 = 14.0;

pub fn resize_edges(ctx: &Context) {
    if filling_screen(ctx) { return; }
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
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        ctx.stop_dragging();   // the window manager takes the release (see topbar.rs)
    }
}
