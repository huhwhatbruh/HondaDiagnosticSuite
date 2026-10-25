//! The app's motion, in one place: one easing, a few durations, and Reduce Motion (HIG Accessibility: "be cautious with
//! fast-moving and blinking animations"): `HONDASCAN_REDUCE_MOTION=1`, or the desktop's own setting (GNOME: animations off)
//! turns all of it off. Every duration becomes 0, so each animation lands on its final frame at once, the turntable stops
//! turning on its own and the pulsing dots hold still: for people who want that, and stable screenshots.
//! egui asks for a repaint only while one of these is still moving, so a still screen costs nothing.
use eframe::egui::{Color32, Context, Id};
use std::sync::OnceLock;

/// `gsettings get org.gnome.desktop.interface enable-animations` prints `false` when animations are off.
fn animations_off(gsettings_output: &str) -> bool { gsettings_output.trim() == "false" }

pub fn reduced() -> bool {
    static R: OnceLock<bool> = OnceLock::new();
    *R.get_or_init(|| {
        if let Some(v) = std::env::var_os("HONDASCAN_REDUCE_MOTION") { return v != "0"; }
        // read once at start-up; a missing gsettings (not GNOME) or an unreadable answer means animations stay on
        std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "enable-animations"]).output()
            .ok().filter(|o| o.status.success()).is_some_and(|o| animations_off(&String::from_utf8_lossy(&o.stdout)))
    })
}

/// A duration in seconds, or 0 with reduced motion.
fn secs(s: f32) -> f32 { if reduced() { 0.0 } else { s } }

/// The shared easing (the web app's --ease, cubic-bezier(0.32, 0.72, 0, 1)), approximated: a fast start that settles.
pub fn ease(t: f32) -> f32 { 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(4) }

/// 0..1 following `on`, for hover and selection fades (110 ms).
pub fn fade(ctx: &Context, id: Id, on: bool) -> f32 { ctx.animate_bool_with_time_and_easing(id, on, secs(0.11), ease) }

/// 0..1 for the open/closed state of something that grows and shrinks (190 ms).
pub fn open(ctx: &Context, id: Id, on: bool) -> f32 { ctx.animate_bool_with_time_and_easing(id, on, secs(0.19), ease) }

/// `target`, approached over 170 ms from wherever the last frame left it (a sliding underline, a gauge).
pub fn settle(ctx: &Context, id: Id, target: f32) -> f32 { ctx.animate_value_with_time(id, target, secs(0.17)) }

/// Progress 0..1 of a screen coming in (240 ms) since `born`; asks for repaints until it is 1.
pub fn entered(ctx: &Context, born: f64) -> f32 {
    let d = secs(0.24) as f64;
    let t = if d == 0.0 { 1.0 } else { ((ctx.input(|i| i.time) - born) / d).clamp(0.0, 1.0) as f32 };
    if t < 1.0 { ctx.request_repaint(); }
    t
}

/// `a` towards `b` by `t`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 { a.lerp_to_gamma(b, t) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gsettings_false_means_animations_are_off() {
        assert!(animations_off("false\n"));
        assert!(!animations_off("true\n"));
        assert!(!animations_off(""), "an unreadable answer leaves animations on");
    }

    #[test]
    fn the_easing_starts_at_zero_ends_at_one_and_never_goes_back() {
        assert_eq!((ease(0.0), ease(1.0)), (0.0, 1.0));
        assert_eq!((ease(-1.0), ease(2.0)), (0.0, 1.0), "outside the range it holds the end");
        let v: Vec<f32> = (0..=20).map(|i| ease(i as f32 / 20.0)).collect();
        assert!(v.windows(2).all(|w| w[1] >= w[0]));
        assert!(ease(0.25) > 0.5, "a fast start that settles");
    }
}
