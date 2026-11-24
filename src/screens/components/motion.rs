// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Time-driven motion: eased bars and gauges, a braille spinner, toast
//! entrance and a short style pulse for celebrations.
//!
//! Animation is driven by an explicit [`Motion`] value carried by the
//! [`UiContext`](crate::screens::ui::UiContext). The event loop builds one
//! context per frame from a monotonic [`Instant`](std::time::Instant); renders
//! made outside the loop (tests, one-off renders) use a motion with no frame
//! time, so every animation resolves to its final state and snapshots stay
//! deterministic.
//!
//! Setting `PMP_REDUCED_MOTION` to a non-empty value (other than `0`) turns
//! every animation off: values jump to their target, the spinner keeps a
//! static icon and nothing pulses. The variable is resolved once when the UI
//! context is built, never read from a render.

use std::f64::consts::TAU;
use std::time::Duration;

use ratatui::style::Color;

use crate::screens::components::glyphs::Glyphs;

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

/// The time source and the reduced-motion preference of one render.
///
/// It is cheap to copy, so it travels by value next to the theme and the glyph
/// set inside [`UiContext`](crate::screens::ui::UiContext).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Motion {
    frame_time: Option<Duration>,
    reduced: bool,
}

impl Motion {
    /// Motion with no frame time: animations resolve to their final state.
    pub const fn still() -> Motion {
        Motion {
            frame_time: None,
            reduced: false,
        }
    }

    /// Motion with animations turned off through `PMP_REDUCED_MOTION`.
    pub const fn reduced() -> Motion {
        Motion {
            frame_time: None,
            reduced: true,
        }
    }

    /// The same preference with the time of the frame being drawn.
    pub const fn at(self, frame_time: Duration) -> Motion {
        Motion {
            frame_time: Some(frame_time),
            ..self
        }
    }

    /// The same motion with the reduced-motion preference replaced.
    pub const fn with_reduced(self, reduced: bool) -> Motion {
        Motion { reduced, ..self }
    }

    /// Time of the frame being drawn, if the event loop published one.
    pub fn frame_time(&self) -> Option<Duration> {
        self.frame_time
    }

    /// `true` when the user asked for reduced motion.
    pub fn is_reduced(&self) -> bool {
        self.reduced
    }

    /// `true` while the event loop is publishing frame times and motion is
    /// allowed. Everything animated checks this before moving.
    pub fn animating(&self) -> bool {
        self.frame_time.is_some() && !self.reduced
    }

    /// [`Self::frame_time`] with `None` read as frame 0.
    pub fn now(&self) -> Duration {
        self.frame_time.unwrap_or_default()
    }

    /// Glyph of the spinner shown next to tasks in progress. Reduced motion
    /// shows the static status icon; when nothing is animating the first frame
    /// is used, so renders stay deterministic.
    pub fn spinner_glyph(&self, glyphs: Glyphs) -> &'static str {
        if self.reduced {
            return glyphs.status_in_progress;
        }
        let frames: &[&str] = if glyphs.is_ascii() {
            &SPINNER_ASCII
        } else {
            &SPINNER_BRAILLE
        };
        let step = if self.animating() {
            self.now().as_millis() / SPINNER_STEP.as_millis()
        } else {
            0
        };
        frames[step as usize % frames.len()]
    }

    /// Moves an RGB color a little toward white on a slow sine wave. Colors
    /// that are not RGB (the default ANSI palette, which the terminal draws
    /// itself) cannot be blended and are returned unchanged.
    pub fn pulse_color(&self, color: Color) -> Color {
        if !self.animating() {
            return color;
        }
        let Color::Rgb(r, g, b) = color else {
            return color;
        };
        let phase = self.now().as_secs_f64() / PULSE_PERIOD.as_secs_f64() * TAU;
        let amount = (1.0 - phase.cos()) / 2.0 * PULSE_DEPTH;
        let lift = |channel: u8| {
            let channel = channel as f64;
            (channel + (255.0 - channel) * amount).round() as u8
        };
        Color::Rgb(lift(r), lift(g), lift(b))
    }

    /// `true` while a celebration that started at `since` is still playing.
    pub fn celebrating(&self, since: Option<Duration>) -> bool {
        self.animating()
            && since.is_some_and(|start| self.now().saturating_sub(start) < CELEBRATION_DURATION)
    }

    /// Progress of a toast entrance, from 0.0 (just appeared) to 1.0 (in
    /// place). Always 1.0 when nothing is animating.
    pub fn entrance_progress(&self, age: Duration) -> f64 {
        if !self.animating() {
            return 1.0;
        }
        (age.as_secs_f64() / TOAST_ENTRANCE.as_secs_f64()).clamp(0.0, 1.0)
    }
}

impl Default for Motion {
    fn default() -> Self {
        Self::still()
    }
}

/// The `PMP_REDUCED_MOTION` preference from a lookup: `Some(true)` to reduce,
/// `Some(false)` when the variable explicitly disables it (`0`) and `None` when
/// it is unset or empty.
pub fn reduced_requested(lookup: &impl Fn(&str) -> Option<String>) -> Option<bool> {
    lookup(REDUCED_MOTION_ENV)
        .filter(|value| !value.is_empty())
        .map(|value| value != "0")
}

/// A value that moves smoothly toward its target with ease-out cubic easing.
///
/// Changing the target starts a new movement from wherever the value is now,
/// so rapid updates never make it jump. Both ends of the movement are read
/// from the [`Motion`] passed in, never from global state.
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
    pub fn set(&mut self, target: f64, motion: &Motion) {
        if target == self.to {
            return;
        }
        let now = motion.now();
        self.from = self.value_at(now, motion);
        self.to = target;
        self.started = now;
    }

    /// The value to draw in the current frame.
    pub fn value(&self, motion: &Motion) -> f64 {
        self.value_at(motion.now(), motion)
    }

    fn value_at(&self, now: Duration, motion: &Motion) -> f64 {
        if !motion.animating() {
            return self.to;
        }
        let elapsed = now.saturating_sub(self.started).as_secs_f64();
        let t = (elapsed / EASE_DURATION.as_secs_f64()).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        self.from + (self.to - self.from) * eased
    }
}

/// Eases an entrance progress value so it starts fast and settles softly.
pub fn ease_out(progress: f64) -> f64 {
    1.0 - (1.0 - progress.clamp(0.0, 1.0)).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn at(value: u64) -> Motion {
        Motion::still().at(ms(value))
    }

    #[test]
    fn without_frame_time_everything_is_static() {
        let motion = Motion::still();
        assert!(!motion.animating());
        let mut tween = Tween::settled(0.0);
        tween.set(1.0, &motion);
        assert_eq!(tween.value(&motion), 1.0);
        assert_eq!(motion.spinner_glyph(Glyphs::unicode()), SPINNER_BRAILLE[0]);
        assert_eq!(motion.entrance_progress(ms(0)), 1.0);
        assert!(!motion.celebrating(Some(ms(0))));
    }

    #[test]
    fn reduced_motion_behaves_like_no_frame_time() {
        let motion = Motion::reduced().at(ms(600));
        assert!(!motion.animating());
        let mut tween = Tween::settled(0.0);
        tween.set(1.0, &motion);
        assert_eq!(tween.value(&motion), 1.0);
        assert_eq!(motion.spinner_glyph(Glyphs::unicode()), "◐");
        assert_eq!(motion.pulse_color(Color::Rgb(0, 0, 0)), Color::Rgb(0, 0, 0));
    }

    #[test]
    fn reduced_motion_is_read_from_the_lookup() {
        assert_eq!(reduced_requested(&|_| Some("1".to_string())), Some(true));
        assert_eq!(reduced_requested(&|_| Some("yes".to_string())), Some(true));
        assert_eq!(reduced_requested(&|_| Some("0".to_string())), Some(false));
        assert_eq!(reduced_requested(&|_| Some(String::new())), None);
        assert_eq!(reduced_requested(&|_| None), None);
    }

    #[test]
    fn tween_moves_with_ease_out_and_lands_on_target() {
        let start = at(0);
        let mut tween = Tween::settled(0.0);
        tween.set(1.0, &start);
        assert_eq!(tween.target(), 1.0);
        let mid = tween.value(&at(EASE_DURATION.as_millis() as u64 / 2));
        assert!(mid > 0.8 && mid < 1.0, "ease-out: {mid}");
        let end = tween.value(&at(EASE_DURATION.as_millis() as u64));
        assert!((end - 1.0).abs() < 1e-9);
    }

    #[test]
    fn retargeting_mid_movement_starts_from_the_current_position() {
        let start = at(0);
        let mut tween = Tween::settled(0.0);
        tween.set(1.0, &start);
        let halfway_time = at(EASE_DURATION.as_millis() as u64 / 2);
        let before = tween.value(&halfway_time);
        tween.set(0.0, &halfway_time);
        // The new movement starts where the old one was, not at 1.0.
        let after = tween.value(&halfway_time);
        assert!(
            (before - after).abs() < 1e-9,
            "before {before}, after {after}"
        );
        assert!(after < 1.0);
        assert_eq!(tween.target(), 0.0);
    }

    #[test]
    fn setting_the_same_target_keeps_the_movement() {
        let start = at(0);
        let mut tween = Tween::settled(0.0);
        tween.set(1.0, &start);
        let before = tween;
        tween.set(1.0, &at(100));
        assert_eq!(tween, before);
    }

    #[test]
    fn spinner_advances_one_frame_per_step() {
        assert_eq!(at(0).spinner_glyph(Glyphs::unicode()), SPINNER_BRAILLE[0]);
        assert_eq!(
            at(SPINNER_STEP.as_millis() as u64).spinner_glyph(Glyphs::unicode()),
            SPINNER_BRAILLE[1]
        );
        let wrapped = SPINNER_STEP.as_millis() as u64 * 10;
        assert_eq!(
            at(wrapped).spinner_glyph(Glyphs::unicode()),
            SPINNER_BRAILLE[0]
        );
        assert_eq!(
            at(SPINNER_STEP.as_millis() as u64).spinner_glyph(Glyphs::ascii()),
            SPINNER_ASCII[1]
        );
    }

    #[test]
    fn pulse_only_changes_rgb_colors() {
        let motion = at(600);
        assert_eq!(motion.pulse_color(Color::Cyan), Color::Cyan);
        assert_ne!(motion.pulse_color(Color::Rgb(0, 0, 0)), Color::Rgb(0, 0, 0));
        assert_eq!(
            Motion::still().pulse_color(Color::Rgb(0, 0, 0)),
            Color::Rgb(0, 0, 0)
        );
    }

    #[test]
    fn celebration_lasts_its_duration_only() {
        let start = ms(1000);
        assert!(at(1000).celebrating(Some(start)));
        let inside = start + CELEBRATION_DURATION - ms(1);
        assert!(Motion::still().at(inside).celebrating(Some(start)));
        let after = start + CELEBRATION_DURATION;
        assert!(!Motion::still().at(after).celebrating(Some(start)));
        assert!(!at(1000).celebrating(None));
    }

    #[test]
    fn toast_entrance_completes_after_its_duration() {
        let motion = at(0);
        assert_eq!(motion.entrance_progress(ms(0)), 0.0);
        assert!((motion.entrance_progress(TOAST_ENTRANCE / 2) - 0.5).abs() < 1e-9);
        assert_eq!(motion.entrance_progress(TOAST_ENTRANCE * 2), 1.0);
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out(0.5) > 0.5);
    }
}
