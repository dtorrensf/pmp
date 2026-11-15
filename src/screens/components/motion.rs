// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Time-driven motion: eased bars and gauges, a braille spinner, toast
//! entrance and a short style pulse for celebrations.
//!
//! Animation is driven by the time of the frame being drawn. The event loop
//! publishes that time with [`set_frame_time`] before every tick and draw,
//! measured from a monotonic [`Instant`](std::time::Instant). Outside the
//! event loop (tests, one-off renders) no frame time is published, so every
//! animation resolves to its final state: snapshots stay deterministic and
//! behave as frame 0.
//!
//! Setting `PMP_REDUCED_MOTION` to a non-empty value (other than `0`) turns
//! every animation off: values jump to their target, the spinner keeps a
//! static icon and nothing pulses.

use std::cell::Cell;
use std::f64::consts::TAU;
use std::sync::OnceLock;
use std::time::Duration;

use ratatui::style::Color;

use crate::screens::components::glyphs;

/// Environment variable that turns animations off when set.
pub const REDUCED_MOTION_ENV: &str = "PMP_REDUCED_MOTION";

/// How long a bar or gauge takes to reach a new value.
pub const EASE_DURATION: Duration = Duration::from_millis(350);
/// Time each spinner frame stays on screen.
pub const SPINNER_STEP: Duration = Duration::from_millis(100);
/// How long a version celebration (100 % or compaction) lasts.
pub const CELEBRATION_DURATION: Duration = Duration::from_millis(1200);
/// How long a toast takes to slide in.
pub const TOAST_ENTRANCE: Duration = Duration::from_millis(300);
/// Period of the accent pulse on focused elements.
const PULSE_PERIOD: Duration = Duration::from_millis(2400);
/// Largest share of the way to white the pulse reaches. Kept small so the
/// pulse is felt, not seen.
const PULSE_DEPTH: f64 = 0.25;

const SPINNER_BRAILLE: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_ASCII: [&str; 4] = ["|", "/", "-", "\\"];

thread_local! {
    /// Time of the frame being drawn, or `None` when nothing is animating.
    static FRAME_TIME: Cell<Option<Duration>> = const { Cell::new(None) };
}

/// Publishes the time of the frame being drawn. `None` turns animations off.
pub fn set_frame_time(time: Option<Duration>) {
    FRAME_TIME.with(|frame| frame.set(time));
}

/// The time of the frame being drawn, if the event loop published one.
pub fn frame_time() -> Option<Duration> {
    FRAME_TIME.with(Cell::get)
}

/// [`frame_time`] with `None` read as frame 0.
fn now() -> Duration {
    frame_time().unwrap_or_default()
}

/// `true` when animations are turned off through `PMP_REDUCED_MOTION`.
pub fn reduced() -> bool {
    static REDUCED: OnceLock<bool> = OnceLock::new();
    *REDUCED.get_or_init(|| reduced_in(&|key| std::env::var(key).ok()))
}

/// `true` when the given lookup yields a `PMP_REDUCED_MOTION` value other
/// than empty or `0`.
fn reduced_in(lookup: &impl Fn(&str) -> Option<String>) -> bool {
    lookup(REDUCED_MOTION_ENV).is_some_and(|value| !value.is_empty() && value != "0")
}

/// `true` while the event loop is publishing frame times and motion is
/// allowed. Everything animated checks this before moving.
pub fn animating() -> bool {
    frame_time().is_some() && !reduced()
}

/// Glyph of the spinner shown next to tasks in progress. The first frame is
/// used when nothing is animating, so renders stay deterministic.
pub fn spinner_glyph() -> &'static str {
    if reduced() {
        return glyphs::glyphs().status_in_progress;
    }
    let frames: &[&str] = if glyphs::ascii_requested() {
        &SPINNER_ASCII
    } else {
        &SPINNER_BRAILLE
    };
    let step = now().as_millis() / SPINNER_STEP.as_millis();
    frames[step as usize % frames.len()]
}

/// A value that moves smoothly toward its target with ease-out cubic easing.
///
/// Changing the target starts a new movement from wherever the value is now,
/// so rapid updates never make it jump.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tween {
    from: f64,
    to: f64,
    started: Duration,
}

impl Tween {
    /// A value that is already at `value`, with no movement pending.
    pub fn settled(value: f64) -> Tween {
        Tween {
            from: value,
            to: value,
            started: Duration::ZERO,
        }
    }

    /// The value the tween is heading to.
    pub fn target(&self) -> f64 {
        self.to
    }

    /// Starts moving toward `target` from the current position. Does nothing
    /// when `target` is already the goal.
    pub fn set(&mut self, target: f64) {
        if target == self.to {
            return;
        }
        let now = now();
        self.from = self.value_at(now);
        self.to = target;
        self.started = now;
    }

    /// The value to draw in the current frame.
    pub fn value(&self) -> f64 {
        self.value_at(now())
    }

    fn value_at(&self, now: Duration) -> f64 {
        if !animating() {
            return self.to;
        }
        let elapsed = now.saturating_sub(self.started).as_secs_f64();
        let t = (elapsed / EASE_DURATION.as_secs_f64()).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        self.from + (self.to - self.from) * eased
    }
}

/// Moves an RGB color a little toward white on a slow sine wave. Colors that
/// are not RGB (the default ANSI palette, which the terminal draws itself)
/// cannot be blended and are returned unchanged.
pub fn pulse_color(color: Color) -> Color {
    if !animating() {
        return color;
    }
    let Color::Rgb(r, g, b) = color else {
        return color;
    };
    let phase = now().as_secs_f64() / PULSE_PERIOD.as_secs_f64() * TAU;
    let amount = (1.0 - phase.cos()) / 2.0 * PULSE_DEPTH;
    let lift = |channel: u8| {
        let channel = channel as f64;
        (channel + (255.0 - channel) * amount).round() as u8
    };
    Color::Rgb(lift(r), lift(g), lift(b))
}

/// `true` while a celebration that started at `since` is still playing.
pub fn celebrating(since: Option<Duration>) -> bool {
    animating() && since.is_some_and(|start| now().saturating_sub(start) < CELEBRATION_DURATION)
}

/// Progress of a toast entrance, from 0.0 (just appeared) to 1.0 (in place).
/// Always 1.0 when nothing is animating.
pub fn entrance_progress(age: Duration) -> f64 {
    if !animating() {
        return 1.0;
    }
    (age.as_secs_f64() / TOAST_ENTRANCE.as_secs_f64()).clamp(0.0, 1.0)
}

/// Eases an entrance progress value so it starts fast and settles softly.
pub fn ease_out(progress: f64) -> f64 {
    1.0 - (1.0 - progress.clamp(0.0, 1.0)).powi(3)
}

/// Runs `body` with the frame time forced to `time`, restoring the previous
/// value afterwards. Lets tests exercise animations at a chosen instant.
#[cfg(test)]
pub fn with_frame_time<T>(time: Option<Duration>, body: impl FnOnce() -> T) -> T {
    let previous = frame_time();
    set_frame_time(time);
    let result = body();
    set_frame_time(previous);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn without_frame_time_everything_is_static() {
        assert!(!animating());
        let mut tween = Tween::settled(0.0);
        tween.set(1.0);
        assert_eq!(tween.value(), 1.0);
        assert_eq!(spinner_glyph(), SPINNER_BRAILLE[0]);
        assert_eq!(entrance_progress(ms(0)), 1.0);
        assert!(!celebrating(Some(ms(0))));
    }

    #[test]
    fn reduced_motion_is_read_from_the_environment() {
        assert!(reduced_in(&|_| Some("1".to_string())));
        assert!(reduced_in(&|_| Some("yes".to_string())));
        assert!(!reduced_in(&|_| Some("0".to_string())));
        assert!(!reduced_in(&|_| Some(String::new())));
        assert!(!reduced_in(&|_| None));
    }

    #[test]
    fn tween_moves_with_ease_out_and_lands_on_target() {
        with_frame_time(Some(ms(0)), || {
            let mut tween = Tween::settled(0.0);
            tween.set(1.0);
            assert_eq!(tween.target(), 1.0);
            let mid = with_frame_time(Some(EASE_DURATION / 2), || tween.value());
            assert!(mid > 0.8 && mid < 1.0, "ease-out: {mid}");
            let end = with_frame_time(Some(EASE_DURATION), || tween.value());
            assert!((end - 1.0).abs() < 1e-9);
        });
    }

    #[test]
    fn retargeting_mid_movement_starts_from_the_current_position() {
        with_frame_time(Some(ms(0)), || {
            let mut tween = Tween::settled(0.0);
            tween.set(1.0);
            let mut halfway = with_frame_time(Some(EASE_DURATION / 2), || {
                tween.set(0.0);
                tween
            });
            // The new movement starts where the old one was, not at 1.0.
            let start = with_frame_time(Some(EASE_DURATION / 2), || halfway.value());
            assert!((start - tween.value_at(EASE_DURATION / 2)).abs() < 1e-9);
            halfway.set(0.0);
            assert_eq!(halfway.target(), 0.0);
        });
    }

    #[test]
    fn setting_the_same_target_keeps_the_movement() {
        with_frame_time(Some(ms(0)), || {
            let mut tween = Tween::settled(0.0);
            tween.set(1.0);
            let before = tween;
            tween.set(1.0);
            assert_eq!(tween, before);
        });
    }

    #[test]
    fn spinner_advances_one_frame_per_step() {
        with_frame_time(Some(ms(0)), || {
            assert_eq!(spinner_glyph(), SPINNER_BRAILLE[0]);
        });
        with_frame_time(Some(SPINNER_STEP), || {
            assert_eq!(spinner_glyph(), SPINNER_BRAILLE[1]);
        });
        let wrapped = Duration::from_millis(SPINNER_STEP.as_millis() as u64 * 10);
        with_frame_time(Some(wrapped), || {
            assert_eq!(spinner_glyph(), SPINNER_BRAILLE[0]);
        });
    }

    #[test]
    fn pulse_only_changes_rgb_colors() {
        with_frame_time(Some(ms(600)), || {
            assert_eq!(pulse_color(Color::Cyan), Color::Cyan);
            let pulsed = pulse_color(Color::Rgb(0, 0, 0));
            assert_ne!(pulsed, Color::Rgb(0, 0, 0));
        });
        assert_eq!(pulse_color(Color::Rgb(0, 0, 0)), Color::Rgb(0, 0, 0));
    }

    #[test]
    fn celebration_lasts_its_duration_only() {
        let start = ms(1000);
        with_frame_time(Some(start), || assert!(celebrating(Some(start))));
        let inside = start + CELEBRATION_DURATION - ms(1);
        with_frame_time(Some(inside), || assert!(celebrating(Some(start))));
        let after = start + CELEBRATION_DURATION;
        with_frame_time(Some(after), || assert!(!celebrating(Some(start))));
        with_frame_time(Some(start), || assert!(!celebrating(None)));
    }

    #[test]
    fn toast_entrance_completes_after_its_duration() {
        with_frame_time(Some(ms(0)), || {
            assert_eq!(entrance_progress(ms(0)), 0.0);
            assert!((entrance_progress(TOAST_ENTRANCE / 2) - 0.5).abs() < 1e-9);
            assert_eq!(entrance_progress(TOAST_ENTRANCE * 2), 1.0);
            assert_eq!(ease_out(0.0), 0.0);
            assert_eq!(ease_out(1.0), 1.0);
            assert!(ease_out(0.5) > 0.5);
        });
    }
}
