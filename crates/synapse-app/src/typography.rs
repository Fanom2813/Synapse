//! Applies a [`TextRole`] to any GPUI element, so type is set in one place.

use crate::theme::TextRole;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{px, relative, App, FontFeatures, FontWeight, Styled};
use std::sync::Arc;

pub trait Typography: Styled + Sized {
    /// Family, size, weight and leading for `role`, scaled by the user's text
    /// size setting (the theme's base size is 14 px at 100 %).
    fn typeset(self, role: TextRole, cx: &App) -> Self {
        let spec = role.spec();
        let scale = f32::from(cx.theme().font_size) / 14.0;
        // Light text on a dark ground looks thinner, so give it a little more air.
        let leading = spec.line_height + if cx.theme().is_dark() { 0.05 } else { 0.0 };
        self.font_family(spec.font.family())
            .text_size(px(spec.size * scale))
            .font_weight(FontWeight(f32::from(spec.weight)))
            .line_height(relative(leading))
    }

    /// Tabular figures, so costs, counts and percentages line up and do not jitter.
    fn figures(self) -> Self {
        self.font_features(FontFeatures(Arc::new(vec![("tnum".to_string(), 1)])))
    }
}

impl<T: Styled + Sized> Typography for T {}
