//! The "thinking" indicator shown while a reply has not started streaming:
//! a word whose letters scramble and then lock in from left to right, on a
//! loop. Frames are capped (`FPS`), and the toolkit renders the first frame,
//! the plain word, with no animation when reduce-motion is on.

use crate::theme::TextRole;
use crate::typography::Typography as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    div, px, Animation, AnimationExt as _, App, ElementId, IntoElement, ParentElement as _,
    Styled as _,
};
use std::time::Duration;

const WORD: &str = "Thinking";
const CYCLE_MS: u64 = 2200;
const FPS: f32 = 20.0;
/// Letters of similar width, so the word does not jitter while it scrambles.
const GLYPHS: &[u8] = b"aceghnopqsuvxyz";
/// Progress at which every letter has locked in, and at which the hold ends.
const LOCKED_AT: f32 = 0.55;
const HOLD_UNTIL: f32 = 0.85;

/// `word` at `progress` (0..1 of one cycle): letters before the lock-in
/// point show as typed, the rest are random glyphs that change every frame.
pub fn scramble(word: &str, progress: f32) -> String {
    let n = word.chars().count().max(1);
    let locked = if progress >= HOLD_UNTIL {
        0
    } else {
        ((progress / LOCKED_AT).min(1.0) * n as f32).floor() as usize
    };
    let tick = (progress * CYCLE_MS as f32 / (1000.0 / FPS)) as u32;
    word.chars()
        .enumerate()
        .map(|(i, c)| {
            if i < locked || c == ' ' {
                c
            } else {
                glyph(i as u32, tick, c.is_uppercase())
            }
        })
        .collect()
}

fn glyph(index: u32, tick: u32, upper: bool) -> char {
    let h = index.wrapping_mul(2_654_435_761) ^ tick.wrapping_mul(40_503).wrapping_add(index);
    let c = GLYPHS[(h >> 3) as usize % GLYPHS.len()] as char;
    if upper {
        c.to_ascii_uppercase()
    } else {
        c
    }
}

/// The animated word, in the muted caption colour.
pub fn indicator(id: impl Into<ElementId>, app: &App) -> impl IntoElement {
    div()
        .min_w(px(88.0))
        .typeset(TextRole::Label, app)
        .text_color(app.theme().colors.muted_foreground)
        .with_animation(
            id,
            Animation::new(Duration::from_millis(CYCLE_MS))
                .repeat()
                .with_max_fps(FPS),
            |el, progress| el.child(scramble(WORD, progress)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_as_the_plain_word_locks_in_left_to_right_and_rescrambles() {
        assert_eq!(scramble(WORD, 0.0).chars().count(), WORD.chars().count());
        let mid = scramble(WORD, 0.3);
        assert!(mid.starts_with("Thi"), "{mid}");
        assert_eq!(scramble(WORD, 0.6), WORD);
        assert_ne!(scramble(WORD, 0.9), WORD);
    }
}
