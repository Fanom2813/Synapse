//! Window frame (UI.md §1b): the OS title bar is hidden and the app draws its
//! own, keeping the native window buttons. Shared by every window we open so
//! they all look like part of the app.

use gpui_kit::component::TitleBar;
use gpui_kit::{point, px, size, App, Bounds, TitlebarOptions, WindowBounds, WindowOptions};

/// Height of our title bar strip.
pub const BAR_HEIGHT: f32 = 44.0;

/// Options for an app window: centered at `width` x `height`, transparent
/// native title bar with the traffic lights inset to line up with our bar.
/// The title bar component owns dragging and double-click zoom.
pub fn window_options(cx: &App, width: f32, height: f32) -> WindowOptions {
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("Synapse".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(16.0), px(15.0))),
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(width), px(height)),
            cx,
        ))),
        window_min_size: Some(size(px(720.0), px(520.0))),
        ..TitleBar::window_options()
    }
}
