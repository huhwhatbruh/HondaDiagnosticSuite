//! Whether the startup animation plays (qml/Intro.qml): on the first start of each day, unless the person turned it off.
//! A screenshot run never plays it. `HONDASCAN_INTRO=always` plays it on every start, to look at it and for the package check.
use crate::settings::Settings;

const SEQUENCE: &str = include_str!("../assets/intro/sequence.json");

/// assets/intro/sequence.json, or "" when this tree has no frames: the published source does not carry them (public-export
/// overlays a sequence with 0 files), so there the animation never plays and the sidebar has no switch for it.
pub fn sequence() -> &'static str {
    if has_frames(SEQUENCE) { SEQUENCE } else { "" }
}

fn has_frames(sequence: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(sequence).ok().and_then(|v| v["files"].as_u64()).is_some_and(|n| n > 0)
}

/// `today` is the local date as `YYYY-MM-DD`.
pub fn due(s: &Settings, today: &str, always: bool, screenshot: bool) -> bool {
    always || (!screenshot && !s.intro_off && s.intro_day != today)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn played_on(day: &str) -> Settings { Settings { intro_day: day.into(), ..Default::default() } }

    #[test]
    fn it_plays_on_the_first_start_of_a_day_only() {
        assert!(due(&Settings::default(), "2026-10-30", false, false), "a first ever start plays it");
        assert!(!due(&played_on("2026-10-30"), "2026-10-30", false, false), "the second start of the day does not");
        assert!(due(&played_on("2026-10-30"), "2026-10-31", false, false), "the next day does");
    }

    #[test]
    fn turned_off_it_never_plays_and_a_screenshot_run_skips_it() {
        let off = Settings { intro_off: true, ..Default::default() };
        assert!(!due(&off, "2026-10-30", false, false));
        assert!(!due(&Settings::default(), "2026-10-30", false, true));
    }

    #[test]
    fn a_tree_without_frames_has_no_animation() {
        assert!(has_frames(r#"{"fps":60,"files":84,"seq":[0,1]}"#));
        assert!(!has_frames(r#"{"fps":60,"files":0,"seq":[]}"#));
        assert!(!has_frames(""));
    }

    #[test]
    fn always_plays_it_whatever_else_is_set() {
        let off = Settings { intro_off: true, intro_day: "2026-10-30".into(), ..Default::default() };
        assert!(due(&off, "2026-10-30", true, true));
    }
}
