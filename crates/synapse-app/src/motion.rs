//! Micro-interactions from UI.md §5, all derived from [`MotionTokens`].
//!
//! Toolkit-free so it is unit testable. The GPUI view layer maps
//! [`MicroInteraction::spec`] to `with_animation` / GPUI Kit springs; this
//! module guarantees one consistent feel: no inline durations, interruptible
//! springs, exit at 60 % of enter, stagger capped at 6, reduced motion as a
//! short cross-fade, and zero frames when hidden, off-screen or idle.

use crate::theme::{MotionTokens, Spring};

/// Idle settles to zero frames after 30 s (UI.md §4, PERFORMANCE.md).
pub const IDLE_SETTLE_SECS: u64 = 30;
/// Celebration confetti limits: 3–5 dots, ≤ 600 ms, once, never with
/// reduced motion (UI.md §5).
pub const CELEBRATION_MAX_MS: u32 = 600;
/// Citation preview waits for 300 ms hover intent (UI.md §5).
pub const CITATION_HOVER_INTENT_MS: u32 = 300;
/// Panel content fades in 50 ms after the slide starts (UI.md §5).
pub const PANEL_CONTENT_DELAY_MS: u32 = 50;

/// Easing named by UI.md §5 token table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Easing {
    EaseOut,
    EaseInOut,
    EaseOutBack,
}

impl Easing {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EaseOut => "ease-out",
            Self::EaseInOut => "ease-in-out",
            Self::EaseOutBack => "ease-out-back",
        }
    }
}

/// Resolved animation spec for one micro-interaction enter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSpec {
    pub duration_ms: u32,
    pub easing: Easing,
    /// Spring when the interaction settles with one (else `None` = tween).
    pub spring: Option<Spring>,
    /// Enter moves this far (fade + 4–8 px rise); reduced motion forces 0.
    pub rise_px: f32,
}

impl MotionSpec {
    /// Faster fade for exits: 60 % of the enter duration.
    pub fn exit_ms(self, tokens: MotionTokens) -> u32 {
        tokens.exit_ms(self.duration_ms)
    }

    /// Reduced-motion form: short cross-fade, no movement, no spring.
    pub fn reduced(self) -> Self {
        Self {
            duration_ms: self.duration_ms.min(140),
            easing: Easing::EaseOut,
            spring: None,
            rise_px: 0.0,
        }
    }
}

/// Every micro-interaction bullet in UI.md §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicroInteraction {
    ButtonHover,
    ButtonPress,
    SendMorph,
    ComposerGrow,
    MentionMenu,
    MessageEnter,
    AvatarHop,
    TypingShimmer,
    DelegateExpand,
    ProgressEase,
    ApprovalEnter,
    ApprovalCollapse,
    CitationHover,
    SteeringLine,
    MapEdge,
    MapNode,
    PanelResize,
    PanelOpen,
    TabIndicator,
    RunPill,
    ProgressLine,
    BadgeBounce,
    ListStagger,
    ListReorder,
    ToastEnter,
    Celebration,
}

impl MicroInteraction {
    pub fn all() -> &'static [MicroInteraction] {
        use MicroInteraction::*;
        &[
            ButtonHover,
            ButtonPress,
            SendMorph,
            ComposerGrow,
            MentionMenu,
            MessageEnter,
            AvatarHop,
            TypingShimmer,
            DelegateExpand,
            ProgressEase,
            ApprovalEnter,
            ApprovalCollapse,
            CitationHover,
            SteeringLine,
            MapEdge,
            MapNode,
            PanelResize,
            PanelOpen,
            TabIndicator,
            RunPill,
            ProgressLine,
            BadgeBounce,
            ListStagger,
            ListReorder,
            ToastEnter,
            Celebration,
        ]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ButtonHover => "button_hover",
            Self::ButtonPress => "button_press",
            Self::SendMorph => "send_morph",
            Self::ComposerGrow => "composer_grow",
            Self::MentionMenu => "mention_menu",
            Self::MessageEnter => "message_enter",
            Self::AvatarHop => "avatar_hop",
            Self::TypingShimmer => "typing_shimmer",
            Self::DelegateExpand => "delegate_expand",
            Self::ProgressEase => "progress_ease",
            Self::ApprovalEnter => "approval_enter",
            Self::ApprovalCollapse => "approval_collapse",
            Self::CitationHover => "citation_hover",
            Self::SteeringLine => "steering_line",
            Self::MapEdge => "map_edge",
            Self::MapNode => "map_node",
            Self::PanelResize => "panel_resize",
            Self::PanelOpen => "panel_open",
            Self::TabIndicator => "tab_indicator",
            Self::RunPill => "run_pill",
            Self::ProgressLine => "progress_line",
            Self::BadgeBounce => "badge_bounce",
            Self::ListStagger => "list_stagger",
            Self::ListReorder => "list_reorder",
            Self::ToastEnter => "toast_enter",
            Self::Celebration => "celebration",
        }
    }

    /// Spec derived from tokens only — never an inline number.
    pub fn spec(self, t: MotionTokens) -> MotionSpec {
        match self {
            Self::ButtonHover => MotionSpec {
                duration_ms: t.instant_ms,
                easing: Easing::EaseOut,
                spring: None,
                rise_px: 0.0,
            },
            Self::ButtonPress => MotionSpec {
                duration_ms: t.instant_ms,
                easing: Easing::EaseOut,
                spring: Some(t.spring_bouncy),
                rise_px: 0.0,
            },
            Self::SendMorph => MotionSpec {
                duration_ms: t.standard_ms,
                easing: Easing::EaseOut,
                spring: Some(t.spring_bouncy),
                rise_px: 0.0,
            },
            Self::ComposerGrow => MotionSpec {
                duration_ms: t.quick_ms,
                easing: Easing::EaseOut,
                spring: Some(t.spring_soft),
                rise_px: 0.0,
            },
            Self::MentionMenu | Self::MessageEnter | Self::PanelOpen | Self::RunPill => {
                MotionSpec {
                    duration_ms: t.standard_ms,
                    easing: Easing::EaseInOut,
                    spring: None,
                    rise_px: 6.0,
                }
            }
            Self::AvatarHop | Self::BadgeBounce | Self::Celebration => MotionSpec {
                duration_ms: t.standard_ms,
                easing: Easing::EaseOut,
                spring: Some(t.spring_bouncy),
                rise_px: 4.0,
            },
            Self::TypingShimmer | Self::ProgressEase | Self::ProgressLine => MotionSpec {
                duration_ms: t.standard_ms,
                easing: Easing::EaseInOut,
                spring: None,
                rise_px: 0.0,
            },
            Self::DelegateExpand | Self::ApprovalEnter => MotionSpec {
                duration_ms: t.emphasized_ms,
                easing: Easing::EaseOutBack,
                spring: None,
                rise_px: 8.0,
            },
            Self::ApprovalCollapse | Self::CitationHover => MotionSpec {
                duration_ms: t.quick_ms,
                easing: Easing::EaseOut,
                spring: None,
                rise_px: 4.0,
            },
            Self::SteeringLine | Self::MapEdge => MotionSpec {
                duration_ms: t.standard_ms,
                easing: Easing::EaseInOut,
                spring: None,
                rise_px: 0.0,
            },
            Self::MapNode | Self::PanelResize | Self::ListReorder => MotionSpec {
                duration_ms: t.standard_ms,
                easing: Easing::EaseOut,
                spring: Some(t.spring_soft),
                rise_px: 0.0,
            },
            Self::TabIndicator => MotionSpec {
                duration_ms: t.quick_ms,
                easing: Easing::EaseInOut,
                spring: Some(t.spring_soft),
                rise_px: 0.0,
            },
            Self::ListStagger | Self::ToastEnter => MotionSpec {
                duration_ms: t.standard_ms,
                easing: Easing::EaseOut,
                spring: None,
                rise_px: 6.0,
            },
        }
    }

    /// Reduced motion: ambient / movement interactions become a short
    /// cross-fade or are skipped entirely (`None` = do not animate).
    pub fn reduced_spec(self, t: MotionTokens) -> Option<MotionSpec> {
        match self {
            // Skipped with reduced motion.
            Self::TypingShimmer | Self::AvatarHop | Self::Celebration => None,
            _ => Some(self.spec(t).reduced()),
        }
    }
}

/// Frame-budget gate shared by all animated views (UI.md §5 principles).
#[derive(Debug, Clone, Copy)]
pub struct FrameGate {
    pub window_visible: bool,
    pub on_screen: bool,
    pub reduce_motion: bool,
    /// Seconds since the view last changed; idle settles to zero frames.
    pub idle_secs: u64,
}

impl Default for FrameGate {
    fn default() -> Self {
        Self {
            window_visible: true,
            on_screen: true,
            reduce_motion: false,
            idle_secs: 0,
        }
    }
}

impl FrameGate {
    /// No frames when hidden, off-screen, or idle past 30 s.
    pub fn should_animate(self) -> bool {
        if !self.window_visible || !self.on_screen {
            return false;
        }
        if self.idle_secs >= IDLE_SETTLE_SECS {
            return false;
        }
        true
    }

    /// Spec to play for `interaction`, or `None` when no frames should run
    /// (gated off, or skipped under reduced motion).
    pub fn spec_for(
        self,
        interaction: MicroInteraction,
        tokens: MotionTokens,
    ) -> Option<MotionSpec> {
        if !self.should_animate() {
            return None;
        }
        if self.reduce_motion {
            return interaction.reduced_spec(tokens);
        }
        Some(interaction.spec(tokens))
    }
}

/// Celebration confetti: 3–5 dots, ≤ 600 ms, once, never with reduced motion.
pub fn celebration_dots(reduce_motion: bool, seed: u64) -> Vec<(f32, f32)> {
    if reduce_motion {
        return Vec::new();
    }
    let n = 3 + (seed % 3) as usize;
    (0..n)
        .map(|i| {
            let a = (seed.wrapping_add((i as u64).wrapping_mul(0x9e3779b97f4a7c15)) % 360) as f32;
            let r = 12.0 + ((seed >> (i * 4)) % 16) as f32;
            (a.to_radians().cos() * r, a.to_radians().sin() * r - 8.0)
        })
        .collect()
}

#[cfg(test)]
mod motion_tests {
    use super::*;
    use crate::theme::MotionTokens;

    #[test]
    fn motion_tokens_match_documented_values() {
        let t = MotionTokens::default();
        assert_eq!(t.instant_ms, 80);
        assert_eq!(t.quick_ms, 140);
        assert_eq!(t.standard_ms, 200);
        assert_eq!(t.emphasized_ms, 280);
        assert_eq!(
            (t.spring_soft.stiffness, t.spring_soft.damping),
            (170.0, 26.0)
        );
        assert_eq!(
            (t.spring_bouncy.stiffness, t.spring_bouncy.damping),
            (260.0, 18.0)
        );
        assert_eq!((t.stagger_ms, t.stagger_max), (30, 6));
    }

    #[test]
    fn motion_exit_is_sixty_percent_and_stagger_caps() {
        let t = MotionTokens::default();
        assert_eq!(t.exit_ms(200), 120);
        assert_eq!(t.exit_ms(280), 168);
        for m in MicroInteraction::all() {
            let s = m.spec(t);
            assert_eq!(s.exit_ms(t), s.duration_ms * 60 / 100, "{:?}", m);
            assert!(s.exit_ms(t) < s.duration_ms, "{:?}", m);
        }
        assert_eq!(t.stagger_delay_ms(0), 0);
        assert_eq!(t.stagger_delay_ms(2), 60);
        assert_eq!(t.stagger_delay_ms(5), 150);
        assert_eq!(t.stagger_delay_ms(6), 150, "capped at 6 items");
        assert_eq!(t.stagger_delay_ms(99), 150);
    }

    #[test]
    fn motion_micro_interactions_use_tokens_only() {
        let t = MotionTokens::default();
        let allowed = [t.instant_ms, t.quick_ms, t.standard_ms, t.emphasized_ms];
        for m in MicroInteraction::all() {
            let s = m.spec(t);
            assert!(
                allowed.contains(&s.duration_ms),
                "{:?} uses inline duration {}",
                m,
                s.duration_ms
            );
            // Only position/scale/opacity/colour: rise is 0–8 px.
            assert!((0.0..=8.0).contains(&s.rise_px), "{:?}", m);
            if let Some(spring) = s.spring {
                assert!(
                    spring == t.spring_soft || spring == t.spring_bouncy,
                    "{:?} uses inline spring",
                    m
                );
            }
        }
        // Spot-check the documented mapping.
        assert_eq!(
            MicroInteraction::ButtonHover.spec(t).duration_ms,
            t.instant_ms
        );
        assert_eq!(
            MicroInteraction::MessageEnter.spec(t).duration_ms,
            t.standard_ms
        );
        assert_eq!(
            MicroInteraction::ApprovalEnter.spec(t).duration_ms,
            t.emphasized_ms
        );
        assert_eq!(
            MicroInteraction::SendMorph.spec(t).spring,
            Some(t.spring_bouncy)
        );
        assert_eq!(
            MicroInteraction::MapNode.spec(t).spring,
            Some(t.spring_soft)
        );
    }

    #[test]
    fn motion_reduced_replaces_movement_with_fade() {
        let t = MotionTokens::default();
        for m in MicroInteraction::all() {
            match m.reduced_spec(t) {
                None => assert!(
                    matches!(
                        m,
                        MicroInteraction::TypingShimmer
                            | MicroInteraction::AvatarHop
                            | MicroInteraction::Celebration
                    ),
                    "{:?} must only skip documented ambient motion",
                    m
                ),
                Some(s) => {
                    assert_eq!(s.rise_px, 0.0, "{:?} moves under reduce-motion", m);
                    assert_eq!(s.spring, None, "{:?} springs under reduce-motion", m);
                    assert!(s.duration_ms <= 140, "{:?} too long reduced", m);
                }
            }
        }
        // Celebration confetti never runs with reduced motion.
        assert!(celebration_dots(true, 7).is_empty());
        let dots = celebration_dots(false, 7);
        assert!((3..=5).contains(&dots.len()));
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn motion_gate_stops_when_hidden_offscreen_or_idle() {
        let t = MotionTokens::default();
        assert!(FrameGate::default()
            .spec_for(MicroInteraction::MessageEnter, t)
            .is_some());
        for gate in [
            FrameGate {
                window_visible: false,
                ..FrameGate::default()
            },
            FrameGate {
                on_screen: false,
                ..FrameGate::default()
            },
            FrameGate {
                idle_secs: IDLE_SETTLE_SECS,
                ..FrameGate::default()
            },
            FrameGate {
                idle_secs: 3600,
                ..FrameGate::default()
            },
        ] {
            assert!(!gate.should_animate(), "{gate:?}");
            assert!(
                gate.spec_for(MicroInteraction::MessageEnter, t).is_none(),
                "{gate:?}"
            );
        }
        // Reduced motion serves the fade form through the same gate.
        let gate = FrameGate {
            reduce_motion: true,
            ..FrameGate::default()
        };
        let spec = gate
            .spec_for(MicroInteraction::MessageEnter, t)
            .expect("reduced still cross-fades");
        assert_eq!(spec.rise_px, 0.0);
        assert!(
            gate.spec_for(MicroInteraction::Celebration, t).is_none(),
            "celebration skipped with reduce motion"
        );
        assert_eq!(CITATION_HOVER_INTENT_MS, 300);
        assert_eq!(PANEL_CONTENT_DELAY_MS, 50);
        assert!(CELEBRATION_MAX_MS <= 600);
    }
}
