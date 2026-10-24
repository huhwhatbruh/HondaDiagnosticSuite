//! Developer aids, off unless an environment variable asks: a frame-time profile and a screenshot-and-quit.
use super::App;
use eframe::egui;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// `HONDASCAN_PROFILE=1`: how long drawing a frame takes (the app's own work: layout, text, shapes; not the GPU), printed
/// to stderr every 5 s with the frame rate. Off by default.
pub(super) struct Profile { since: Instant, frames: u32, total: Duration, max: Duration, causes: HashMap<String, u32>, last_frame: u64, frame_nrs: u32 }

impl Profile {
    pub(super) fn new() -> Self {
        Profile { since: Instant::now(), frames: 0, total: Duration::ZERO, max: Duration::ZERO, causes: HashMap::new(), last_frame: 0, frame_nrs: 0 }
    }

    pub(super) fn add(&mut self, d: Duration, causes: Vec<egui::RepaintCause>, frame_nr: u64) {
        self.frames += 1;
        if frame_nr != self.last_frame { self.frame_nrs += 1; self.last_frame = frame_nr; }
        for c in causes { *self.causes.entry(format!("{}:{}", c.file.rsplit('/').next().unwrap_or(c.file), c.line)).or_default() += 1; }
        self.total += d;
        self.max = self.max.max(d);
        if d >= Duration::from_millis(8) { eprintln!("slow frame: {:.1} ms (frame {})", d.as_secs_f64() * 1000.0, frame_nr); }
        if self.since.elapsed() >= Duration::from_secs(5) {
            let secs = self.since.elapsed().as_secs_f64();
            eprintln!("frame: {:.2} ms avg, {:.2} ms max, {:.1} frames/s, app work {:.1} % of a core", self.total.as_secs_f64() * 1000.0 / self.frames as f64,
                self.max.as_secs_f64() * 1000.0, self.frames as f64 / secs, self.total.as_secs_f64() / secs * 100.0);
            let mut top: Vec<_> = self.causes.iter().collect();
            top.sort_by(|a, b| b.1.cmp(a.1));
            eprintln!("  ui() calls {} , real frames {} in {:.1} s", self.frames, self.frame_nrs, secs);
            eprintln!("  repaint requested by: {}", top.iter().take(6).map(|(k, n)| format!("{k} x{n}")).collect::<Vec<_>>().join(", "));
            *self = Profile::new();
        }
    }
}

/// `HONDASCAN_SCREENSHOT=file.png [HONDASCAN_SCREENSHOT_AFTER=seconds]`: save the window once and quit (for checking the
/// look without a screen capture tool).
pub(super) struct Shot { pub path: String, pub after: f64, pub sent: bool }

impl App {
    pub(super) fn shoot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &mut self.shot else { return };
        if !shot.sent && ctx.input(|i| i.time) > shot.after {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            shot.sent = true;
        }
        let img = ctx.input(|i| i.events.iter().find_map(|e| match e { egui::Event::Screenshot { image, .. } => Some(image.clone()), _ => None }));
        if let Some(img) = img {
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            if let Some(buf) = image::RgbaImage::from_raw(img.width() as u32, img.height() as u32, rgba) { let _ = buf.save(&shot.path); }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if shot.sent { ctx.request_repaint(); }
    }
}
