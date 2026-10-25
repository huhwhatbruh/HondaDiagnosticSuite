//! The installation check, shown on the welcome screen, before the first Connect and nowhere else: what the backend found on
//! this computer (hondascan selfcheck.rs, `self_check`), plus the two things only the app knows — whether the backend is this
//! app's own version, and whether the settings folder can be written. It runs by itself at every start. When all is well it is
//! one quiet line; only a failure spells itself out. Warnings (an optional piece missing) never block anything.
use crate::api::SelfCheck;
use crate::app::App;
use crate::theme::*;
use eframe::egui::{self, RichText, Ui};
use egui_phosphor::regular as icon;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Now in Unix seconds: the timestamp the check records.
pub fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Whether a check counts as passed. A backend that is not exactly this app's version never passes: its replies may lack
/// the fields the app relies on, and the two are shipped together.
pub fn passed(check: &SelfCheck, writable: bool) -> bool { check.ok && check.version == APP_VERSION && writable }

/// One line of the checklist: `status` is the backend's `ok`, `warn` or `fail`.
struct Line { status: &'static str, title: String, detail: String }

fn line(status: &'static str, title: &str, detail: impl Into<String>) -> Line { Line { status, title: title.into(), detail: detail.into() } }

fn status_of(s: &str) -> &'static str { match s { "ok" => "ok", "warn" => "warn", _ => "fail" } }

/// Every line of the checklist, in the order shown: the backend's own, then the app's two. `None` while the reply is pending.
fn lines(app: &App) -> Option<Vec<Line>> {
    let mut out: Vec<Line> = match (&app.self_check, &app.self_check_err) {
        (Some(sc), _) => sc.checks.iter().map(|c| line(status_of(&c.status), &c.title, c.detail.clone())).collect(),
        (None, Some(e)) => vec![line("fail", "The scanner did not answer the check",
            format!("It did not start, or it is older than the app and does not know this check. Reinstall or update hondascan. ({e})"))],
        (None, None) => return None,
    };
    // the version the check reported, else the one `hello` did (an older backend answers hello but not the check); a backend
    // that never started has none, and its failure is already the line above
    let backend = app.self_check.as_ref().map(|s| s.version.clone()).or_else(|| Some(app.backend_version.clone()).filter(|v| !v.is_empty()));
    if let Some(backend) = backend {
        out.push(line(if backend == APP_VERSION { "ok" } else { "fail" }, "Backend version matches the app",
            format!("app {APP_VERSION} · backend {backend}. They are shipped together; a mismatch can be a partial update.")));
    }
    out.push(match (app.settings.is_some(), app.settings_writable) {
        (false, _) => line("warn", "Settings file writable", "the data folder is not known yet"),
        (true, true) => line("ok", "Settings file writable", "the adapter choice and the check result can be saved"),
        (true, false) => line("fail", "Settings file writable", "the settings folder cannot be written: the adapter choice and the check result would be lost"),
    });
    Some(out)
}

/// The mark for one line. Phosphor icons, not text glyphs: the bundled fonts have no ✓ and draw a box for it. A warning is
/// an optional piece, so it is a quiet grey "i", not an amber alarm.
fn mark(status: &str) -> (&'static str, egui::Color32) {
    match status { "ok" => (icon::CHECK_CIRCLE, OK), "warn" => (icon::INFO, MUTED_FG), _ => (icon::X_CIRCLE, FAULT) }
}

fn row(ui: &mut Ui, l: &Line) {
    let (m, col) = mark(l.status);
    ui.horizontal_top(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(m).size(15.0).color(col));
        ui.vertical(|ui| {
            ui.label(RichText::new(&l.title).size(13.0));
            if !l.detail.is_empty() { ui.label(RichText::new(&l.detail).size(11.5).color(MUTED_FG)); }
        });
    });
}

/// The check on the welcome screen: a single status line, the failures spelled out under it, everything else under "Details".
pub fn show(ui: &mut Ui, app: &App) {
    let Some(all) = lines(app) else {
        ui.horizontal(|ui| { ui.spinner(); ui.label(RichText::new("Checking this computer…").size(12.0).color(MUTED_FG)); });
        return;
    };
    let failed: Vec<&Line> = all.iter().filter(|l| l.status == "fail").collect();
    let (m, col, text) = if failed.is_empty() { (icon::CHECK_CIRCLE, OK, "Ready to scan") } else { (icon::X_CIRCLE, FAULT, "Something needs fixing before you scan") };
    ui.horizontal(|ui| {
        ui.label(RichText::new(m).size(16.0).color(col));
        ui.label(RichText::new(text).font(bold(13.0)));
    });
    for l in &failed { row(ui, l); }
    egui::CollapsingHeader::new(RichText::new("Details").size(11.5).color(MUTED_FG)).id_salt("installation-details").show(ui, |ui| {
        for l in &all { row(ui, l); }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(ok: bool, version: &str) -> SelfCheck { SelfCheck { ok, version: version.into(), ..Default::default() } }

    #[test]
    fn a_check_passes_only_when_the_backend_is_this_app_and_the_folder_is_writable() {
        assert!(passed(&check(true, APP_VERSION), true));
        assert!(!passed(&check(true, APP_VERSION), false), "a folder that cannot be written loses the result");
        assert!(!passed(&check(false, APP_VERSION), true), "one failed check is a failed check");
        assert!(!passed(&check(true, "0.0.9"), true), "an older backend does not pass");
    }

    #[test]
    fn the_timestamp_is_the_clock_the_settings_and_audit_use() {
        let now = unix_now();
        assert!(now > 1_700_000_000.0, "Unix seconds, not milliseconds: {now}");
    }

    #[test]
    fn an_unknown_status_is_a_failure_and_a_warning_is_not() {
        assert_eq!(status_of("ok"), "ok");
        assert_eq!(status_of("warn"), "warn");
        assert_eq!(status_of("anything else"), "fail", "a status the app does not know must not read as fine");
    }

    #[test]
    fn marks_are_icons_from_the_icon_font_not_text_glyphs() {
        // the bundled Noto fonts have no ✓ (U+2713); Phosphor's glyphs live in the private-use area the icon font covers
        for s in ["ok", "warn", "fail"] {
            let c = mark(s).0.chars().next().unwrap() as u32;
            assert!((0xE000..=0xF8FF).contains(&c), "{s}: U+{c:04X} is not a Phosphor private-use glyph");
        }
    }
}
