//! hondascan, native: the same scanner (hondascan/, Rust, over JSON lines) behind an egui front end.
mod api;
mod app;
mod backend;
mod format;
mod live;
mod motion;
mod names;
mod ranges;
mod settings;
mod systems;
mod theme;
mod ui;
mod widgets;

use eframe::egui;

/// `HONDASCAN_SIZE=WxH` overrides the starting size (for screenshots of long screens).
fn size() -> [f32; 2] {
    std::env::var("HONDASCAN_SIZE").ok().and_then(|v| v.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))).unwrap_or([1360.0, 860.0])
}

fn main() -> eframe::Result<()> {
    // the logo, decoded once: the window icon here, the spinning model in the app
    let logo = image::load_from_memory(include_bytes!("../assets/icon.png")).ok().map(|i| i.to_rgba8());
    // the window icon: the logo scaled to fit a square, its transparent background kept
    let icon = logo.as_ref().map(|i| {
        let i = image::imageops::resize(i, 256, (256.0 * i.height() as f32 / i.width() as f32) as u32, image::imageops::FilterType::Lanczos3);
        let mut sq = image::RgbaImage::new(256, 256);
        image::imageops::overlay(&mut sq, &i, 0, ((256 - i.height()) / 2) as i64);
        egui::IconData { width: 256, height: 256, rgba: sq.into_raw() }
    });
    let mut vp = egui::ViewportBuilder::default()
        .with_title("hondascan").with_decorations(false)
        .with_inner_size(size()).with_min_inner_size([1024.0, 680.0]);
    if let Some(i) = icon { vp = vp.with_icon(i); }
    eframe::run_native("hondascan", eframe::NativeOptions { viewport: vp, multisampling: std::env::var("HONDASCAN_MSAA").ok().and_then(|v| v.parse().ok()).unwrap_or(4), ..Default::default() }, Box::new(move |cc| Ok(Box::new(app::App::new(cc, logo)))))
}
