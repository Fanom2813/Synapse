//! Enter animation shared by screens: fade + short slide with a capped
//! stagger (UI.md §5). Durations come from [`MotionTokens`]; reduced motion
//! becomes a short cross-fade with no movement. Plays once per element id, so
//! put the step / screen in the id to replay it on change.

use crate::theme::{MotionTokens, Spring};
use gpui_kit::*;
use std::time::Duration;

/// How a screen's content arrives. Build once per render from the appearance.
#[derive(Debug, Clone, Copy)]
pub struct Reveal {
    pub tokens: MotionTokens,
    pub reduce_motion: bool,
}

impl Reveal {
    pub fn new(tokens: MotionTokens, reduce_motion: bool) -> Self {
        Self {
            tokens,
            reduce_motion,
        }
    }

    /// Fade in while sliding `(dx, dy)` px to rest; `index` staggers siblings.
    pub fn enter<E>(
        self,
        el: E,
        id: impl Into<ElementId>,
        index: u32,
        (dx, dy): (f32, f32),
    ) -> impl IntoElement
    where
        E: Styled + IntoElement + 'static,
    {
        let t = self.tokens;
        let (dur, delay, dx, dy) = if self.reduce_motion {
            (t.quick_ms, 0, 0.0, 0.0)
        } else {
            (t.emphasized_ms, t.stagger_delay_ms(index), dx, dy)
        };
        let total = (dur + delay) as f32;
        let (dur_f, delay_f) = (dur as f32, delay as f32);
        el.relative().with_animation(
            id,
            Animation::new(Duration::from_millis((dur + delay) as u64)).with_easing(move |p| {
                ease_out_quint(((p * total - delay_f) / dur_f).clamp(0.0, 1.0))
            }),
            move |el, eased| {
                el.opacity(eased)
                    .left(px(dx * (1.0 - eased)))
                    .top(px(dy * (1.0 - eased)))
            },
        )
    }
}

impl Reveal {
    /// Soft spring (progress, settle) as a GPUI config; reduced motion snaps.
    pub fn spring_soft(self) -> (SpringConfig, SpringPlayback) {
        self.spring(self.tokens.spring_soft)
    }

    /// Quick, no-overshoot spring for panels and sidebars; reduced motion snaps.
    pub fn spring_snappy(self) -> (SpringConfig, SpringPlayback) {
        self.spring(self.tokens.spring_snappy)
    }

    /// Bouncy spring (marks, badges); reduced motion snaps.
    pub fn spring_bouncy(self) -> (SpringConfig, SpringPlayback) {
        self.spring(self.tokens.spring_bouncy)
    }

    fn spring(self, s: Spring) -> (SpringConfig, SpringPlayback) {
        let playback = if self.reduce_motion {
            SpringPlayback::Completed
        } else {
            SpringPlayback::Running
        };
        (SpringConfig::new(s.stiffness, s.damping, 1.0), playback)
    }
}

fn ease_out_quint(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(5)
}

#[cfg(test)]
mod tests {
    use super::ease_out_quint;

    #[test]
    fn ease_out_quint_runs_zero_to_one_and_decelerates() {
        assert_eq!(ease_out_quint(0.0), 0.0);
        assert_eq!(ease_out_quint(1.0), 1.0);
        assert!(ease_out_quint(0.5) > 0.9);
    }
}
