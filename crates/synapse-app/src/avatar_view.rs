//! An agent's avatar as a GPUI image: the stored avatar JSON (blob or pack
//! style) rendered to SVG by `avatar_packs::render_avatar`.

use crate::avatar_packs::render_avatar;
use gpui_kit::{img, px, Image, ImageFormat, Img, Styled as _};
use std::sync::Arc;

/// Avatar image `size` px square. The same JSON gives the same image id, so
/// GPUI rasterises it once and reuses it.
pub fn avatar_img(avatar_json: &str, name: &str, size: f32) -> Img {
    let svg = render_avatar(avatar_json, name, (size * 2.0) as u32);
    let image = Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes()));
    img(image).size(px(size)).flex_none()
}
