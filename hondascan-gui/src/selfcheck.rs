//! The installation check, shown on the welcome screen, before the first Connect and nowhere else: what the backend
//! found on this computer (hondascan selfcheck.rs, `self_check`), plus the two things only the app knows — whether the
//! backend is this app's own version, and whether the settings folder can be written. It runs by itself at every start.
//! When all is well it is one quiet line; only a failure spells itself out. Warnings (an optional piece missing) never
//! block anything. QML only draws the JSON `selfcheck_json` makes.
use crate::api::SelfCheck;
use serde_json::{json, Value};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Now in Unix seconds: the timestamp the check records.
pub fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Whether a check counts as passed. A backend that is not exactly this app's version never passes: its replies may lack
/// the fields the app relies on, and the two are shipped together.
pub fn passed(check: &SelfCheck, writable: bool) -> bool { check.ok && check.version == APP_VERSION && writable }

/// The check's inputs: the backend's reply, what only the app knows.
pub struct SelfCheckState {
    pub check: Option<SelfCheck>,   // the backend's self_check reply, when it arrived
    pub err: Option<String>,        // why it did not: the backend did not answer the check
    pub backend_version: String,    // from `hello` (an older backend answers hello but not the check)
    pub settings_known: bool,       // the data folder is known yet (the settings opened)
    pub settings_writable: bool,
}

/// One line of the checklist: `status` is the backend's `ok`, `warn` or `fail`.
struct Line { status: &'static str, title: String, detail: String }

fn line(status: &'static str, title: &str, detail: impl Into<String>) -> Line { Line { status, title: title.into(), detail: detail.into() } }

fn status_of(s: &str) -> &'static str { match s { "ok" => "ok", "warn" => "warn", _ => "fail" } }

/// The tone and the text shape for one line's status: tones in sysview's convention (0 ok, 1 warn, 2 fault). The egui
/// drew Phosphor icons (CHECK_CIRCLE / INFO / X_CIRCLE) in the icon font — the bundled fonts have no ✓ and draw a box
/// for it — where JSON needs text the shapes are "✓", "i", "✗". A warning is an optional piece, so it is a quiet grey
/// "i", not an amber alarm.
fn shape(status: &str) -> (u8, &'static str) {
    match status_of(status) { "ok" => (0, "✓"), "warn" => (1, "i"), _ => (2, "✗") }
}

/// Every line of the checklist, in the order shown: the backend's own, then the app's two. `None` while the reply is pending.
fn lines(state: &SelfCheckState) -> Option<Vec<Line>> {
    let mut out: Vec<Line> = match (&state.check, &state.err) {
        (Some(sc), _) => sc.checks.iter().map(|c| line(status_of(&c.status), &c.title, c.detail.clone())).collect(),
        (None, Some(e)) => vec![line("fail", "The scanner did not answer the check",
            format!("It did not start, or it is older than the app and does not know this check. Reinstall or update hondascan. ({e})"))],
        (None, None) => return None,
    };
    // the version the check reported, else the one `hello` did (an older backend answers hello but not the check); a backend
    // that never started has none, and its failure is already the line above
    let backend = state.check.as_ref().map(|s| s.version.clone()).or_else(|| Some(state.backend_version.clone()).filter(|v| !v.is_empty()));
    if let Some(backend) = backend {
        out.push(line(if backend == APP_VERSION { "ok" } else { "fail" }, "Backend version matches the app",
            format!("app {APP_VERSION} · backend {backend}. They are shipped together; a mismatch can be a partial update.")));
    }
    out.push(match (state.settings_known, state.settings_writable) {
        (false, _) => line("warn", "Settings file writable", "the data folder is not known yet"),
        (true, true) => line("ok", "Settings file writable", "the adapter choice and the check result can be saved"),
        (true, false) => line("fail", "Settings file writable", "the settings folder cannot be written: the adapter choice and the check result would be lost"),
    });
    Some(out)
}

/// What QML draws of one line: its tone and shape, the title and the detail (an empty string when the backend sent none;
/// the egui's `row` skipped an empty detail instead).
fn row(l: &Line) -> Value {
    let (tone, s) = shape(l.status);
    json!({ "tone": tone, "shape": s, "title": l.title, "detail": l.detail })
}

/// The check as QML draws it on the welcome screen: while the reply is pending, the spinner's text; otherwise a single
/// status line, the failures spelled out as `rows` under it (the egui showed failed lines open), every line also in
/// `details` in the egui's order — the collapsing "Details" expander is QML's drawing.
pub fn selfcheck_json(state: &SelfCheckState) -> Value {
    let Some(all) = lines(state) else {
        return json!({ "pending": true, "waiting": "Checking this computer…", "headline": Value::Null, "rows": [], "details": [] });
    };
    let failed: Vec<&Line> = all.iter().filter(|l| l.status == "fail").collect();
    let (tone, s, text) = if failed.is_empty() { (0, "✓", "Ready to scan") } else { (2, "✗", "Something needs fixing before you scan") };
    json!({
        "pending": false,
        "waiting": Value::Null,
        "headline": { "tone": tone, "shape": s, "text": text },
        "rows": failed.iter().map(|l| row(l)).collect::<Vec<_>>(),
        "details": all.iter().map(row).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Check;

    fn check(ok: bool, version: &str) -> SelfCheck { SelfCheck { ok, version: version.into(), ..Default::default() } }

    fn state(check: Option<SelfCheck>, err: Option<String>, backend: &str, known: bool, writable: bool) -> SelfCheckState {
        SelfCheckState { check, err, backend_version: backend.into(), settings_known: known, settings_writable: writable }
    }

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
    fn a_check_shows_a_shape_as_well_as_a_colour() {
        // the egui's mark was Phosphor's own glyphs (CHECK_CIRCLE / INFO / X_CIRCLE), private-use code points the icon
        // font covers and plain text has no ✓ for; the JSON keeps text shapes instead, so a shape names the status even
        // where colour is not available
        let ok = shape("ok");
        let warn = shape("warn");
        let bad = shape("anything else");
        assert_eq!(ok, (0, "✓"));
        assert_eq!(warn, (1, "i"), "a warning is an optional piece, so it is a quiet grey 'i', not an amber alarm");
        assert_eq!(bad, (2, "✗"));
        assert_ne!(ok, warn);
        assert_ne!(warn, bad);
        assert_ne!(ok, bad);
    }

    #[test]
    fn the_check_json_says_what_the_welcome_screen_draws() {
        // pending: no reply yet and none refused — the spinner and its muted text, nothing else
        let j = selfcheck_json(&state(None, None, "", false, false));
        assert_eq!((j["pending"].as_bool(), j["waiting"].as_str(), j["headline"].is_null()),
            (Some(true), Some("Checking this computer…"), true));
        assert!(j["rows"].as_array().unwrap().is_empty() && j["details"].as_array().unwrap().is_empty());

        // the backend answered, all green but one warn, and it is exactly this app's version; the folder is writable:
        // rows stay empty (a warn is not a failure), the details keep every line in the egui's order
        let ok = SelfCheck { ok: true, version: APP_VERSION.into(), checks: vec![
            Check { title: "Adapter found".into(), status: "ok".into(), ..Default::default() },
            Check { title: "Udev rule".into(), status: "warn".into(), detail: "setup step skipped".into(), ..Default::default() },
        ], ..Default::default() };
        let j = selfcheck_json(&state(Some(ok), None, "", true, true));
        assert_eq!(j["headline"], json!({ "tone": 0, "shape": "✓", "text": "Ready to scan" }), "a warn never blocks");
        assert!(j["rows"].as_array().unwrap().is_empty());
        let details = j["details"].as_array().unwrap();
        assert_eq!(details.len(), 4, "the backend's two, then the app's two");
        let bid = |i: usize| &details[i];
        assert_eq!((bid(0)["tone"].as_i64(), bid(0)["shape"].as_str(), bid(0)["title"].as_str(), bid(0)["detail"].as_str()),
            (Some(0), Some("✓"), Some("Adapter found"), Some("")), "a detail the backend did not send stays an empty string");
        assert_eq!((bid(1)["tone"].as_i64(), bid(1)["shape"].as_str()), (Some(1), Some("i")), "the warn's quiet 'i'");
        let want = format!("app {APP_VERSION} · backend {APP_VERSION}. They are shipped together; a mismatch can be a partial update.");
        assert_eq!((bid(2)["title"].as_str(), bid(2)["detail"].as_str()), (Some("Backend version matches the app"), Some(want.as_str())));
        assert_eq!((bid(3)["tone"].as_i64(), bid(3)["title"].as_str(), bid(3)["detail"].as_str()),
            (Some(0), Some("Settings file writable"), Some("the adapter choice and the check result can be saved")));

        // the settings not opened yet: a warn that names it, and the headline still says what it says
        let ok = check(true, APP_VERSION);
        let j = selfcheck_json(&state(Some(ok), None, "", false, true));
        let details = j["details"].as_array().unwrap();
        assert_eq!((details.last().unwrap()["tone"].as_i64(), details.last().unwrap()["detail"].as_str()),
            (Some(1), Some("the data folder is not known yet")));
        assert_eq!(j["headline"]["text"], "Ready to scan");

        // a folder that cannot be written is the failure the welcome screen spells out, open under the headline
        let ok = check(true, APP_VERSION);
        let j = selfcheck_json(&state(Some(ok), None, "", true, false));
        let rows = j["rows"].as_array().unwrap();
        assert_eq!((rows.len(), rows[0]["tone"].as_i64(), rows[0]["title"].as_str()), (1, Some(2), Some("Settings file writable")));
        assert_eq!(rows[0]["detail"], "the settings folder cannot be written: the adapter choice and the check result would be lost");
        assert_eq!((j["headline"]["tone"].as_i64(), j["headline"]["shape"].as_str(), j["headline"]["text"].as_str()),
            (Some(2), Some("✗"), Some("Something needs fixing before you scan")));

        // an older backend that did not answer the check: the no-reply failure line (the error says why) and the
        // version line from `hello` are the failures under the headline; the fine settings line is in Details only
        let j = selfcheck_json(&state(None, Some("backend unresponsive".into()), "0.9.9", true, true));
        let rows = j["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0]["tone"].as_i64(), rows[0]["shape"].as_str(), rows[0]["title"].as_str()),
            (Some(2), Some("✗"), Some("The scanner did not answer the check")));
        assert_eq!(rows[0]["detail"],
            "It did not start, or it is older than the app and does not know this check. Reinstall or update hondascan. (backend unresponsive)");
        let want = format!("app {APP_VERSION} · backend 0.9.9. They are shipped together; a mismatch can be a partial update.");
        assert_eq!((rows[1]["tone"].as_i64(), rows[1]["title"].as_str(), rows[1]["detail"].as_str()),
            (Some(2), Some("Backend version matches the app"), Some(want.as_str())), "an older backend does not match");
        let details = j["details"].as_array().unwrap();
        assert_eq!(details.len(), 3);
        assert_eq!((details[2]["tone"].as_i64(), details[2]["detail"].as_str()), (Some(0), Some("the adapter choice and the check result can be saved")));
        assert_eq!(j["headline"]["text"], "Something needs fixing before you scan");

        // a backend that never started has no version anywhere, and its failure is already the no-reply line above
        let j = selfcheck_json(&state(None, Some("did not start".into()), "", true, true));
        let rows = j["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "no `hello` version: the version line is absent");
        let details = j["details"].as_array().unwrap();
        assert_eq!(details.len(), 2);
        assert_eq!(details[1]["title"], "Settings file writable");
    }
}
