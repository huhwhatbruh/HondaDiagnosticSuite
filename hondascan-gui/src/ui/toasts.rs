//! Toasts: slide in from the right on the app's one easing (`motion::ease`), fade out, never more than three. Newest at the bottom.
use crate::motion::ease;
use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Color32, Context, Id, Order, Stroke};

#[derive(Clone, Copy, PartialEq)]
pub enum Level { Info, Success, Warn, Error }

struct Toast { id: String, level: Level, title: String, text: String, born: f64, ttl: f64 }

#[derive(Default)]
pub struct Toasts { items: Vec<Toast> }

const IN: f64 = 0.45;
const OUT: f64 = 0.25;


impl Toasts {
    /// Show a toast; one with the same id replaces the old one in place (no stacking of repeats).
    pub fn push(&mut self, ctx: &Context, id: &str, level: Level, title: &str, text: &str, ttl: f64) {
        let now = ctx.input(|i| i.time);
        self.items.retain(|t| t.id != id);
        self.items.push(Toast { id: id.into(), level, title: title.into(), text: text.into(), born: now, ttl });
        if self.items.len() > 3 { self.items.remove(0); }
        ctx.request_repaint();
    }

    pub fn show(&mut self, ctx: &Context) {
        let now = ctx.input(|i| i.time);
        self.items.retain(|t| now - t.born < t.ttl + OUT);
        let screen = ctx.content_rect();
        let mut y = screen.bottom() - 16.0;
        for t in self.items.iter().rev() {
            let age = now - t.born;
            let (slide, alpha) = if age < IN { (1.0 - ease((age / IN) as f32), ((age / IN) as f32).clamp(0.0, 1.0).powf(0.7)) }
                else if age > t.ttl { (0.0, 1.0 - ((age - t.ttl) / OUT) as f32) } else { (0.0, 1.0) };
            let edge = match t.level { Level::Error => FAULT, Level::Warn => WARN, Level::Success => OK, Level::Info => MUTED_FG };
            let a = |c: Color32| c.gamma_multiply(alpha);
            let galley = ctx.fonts_mut(|f| f.layout(t.text.clone(), sans(13.0), a(MUTED_FG), 330.0));
            let head = ctx.fonts_mut(|f| f.layout_no_wrap(t.title.clone(), bold(13.0), a(FOREGROUND)));
            let h = if t.title.is_empty() { galley.size().y } else { head.size().y + 3.0 + galley.size().y } + 24.0;
            let (w, x) = (366.0, screen.right() - 16.0 + slide * 40.0);
            let rect = egui::Rect::from_min_size(pos2(x - w, y - h), vec2(w, h));
            egui::Area::new(Id::new(("toast", &t.id))).order(Order::Tooltip).fixed_pos(rect.min).interactable(false).show(ctx, |ui| {
                let p = ui.painter();
                p.rect_filled(rect.translate(vec2(0.0, 6.0)).expand(2.0), 0.0, Color32::from_black_alpha((90.0 * alpha) as u8));
                p.rect_filled(rect, 0.0, a(POPOVER));
                p.rect_stroke(rect, 0.0, Stroke::new(1.0, a(Color32::BLACK)), egui::StrokeKind::Inside);
                p.rect_filled(egui::Rect::from_min_size(rect.min, vec2(3.0, h)), 0.0, a(edge));
                let mut at = rect.min + vec2(16.0, 12.0);
                if !t.title.is_empty() { let hh = head.size().y; p.galley(at, head, Color32::WHITE); at.y += hh + 3.0; }
                p.galley(at, galley, MUTED_FG);
            });
            y -= h + 10.0;
        }
        // every frame only while a toast slides or fades; otherwise sleep until the next one starts to leave
        let moving = self.items.iter().any(|t| { let a = now - t.born; a < IN || a > t.ttl });
        if moving { ctx.request_repaint(); }
        else if let Some(next) = self.items.iter().map(|t| t.born + t.ttl - now).reduce(f64::min) { ctx.request_repaint_after(std::time::Duration::from_secs_f64(next.max(0.0))); }
    }
}
