//! The Synapse mark: in-app image and the tray bitmap. Source art lives in
//! `assets/brand/`; `scripts/gen-icons` renders the raster pieces from it.

use gpui_kit::{img, px, Img, Styled as _};

/// Asset path the app's asset source serves the colour mark under.
pub const MARK_PATH: &str = "brand/synapse-mark.svg";

/// The colour mark at `size` px square.
pub fn mark(size: f32) -> Img {
    img(MARK_PATH).size(px(size))
}

/// Menu-bar size: the OS shows the bitmap at its pixel size, so keep it small.
const TRAY_SIZE: u32 = 22;
const DOT_INSET: i32 = 5;
const TRAY_MARK: &[u8] = include_bytes!("../../../assets/brand/tray-22.rgba");

/// Tray bitmap (RGBA, 22 px): the mark, with a status dot in the corner when
/// something needs attention (`None` = idle, mark only).
pub fn tray_rgba(status_dot: Option<[u8; 3]>) -> Vec<u8> {
    let mut rgba = TRAY_MARK.to_vec();
    if let Some(rgb) = status_dot {
        let (cx, cy) = (TRAY_SIZE as i32 - DOT_INSET, TRAY_SIZE as i32 - DOT_INSET);
        for y in 0..TRAY_SIZE as i32 {
            for x in 0..TRAY_SIZE as i32 {
                let d2 = (x - cx).pow(2) + (y - cy).pow(2);
                let at = ((y as u32 * TRAY_SIZE + x as u32) * 4) as usize;
                if d2 <= 5 * 5 {
                    // Dark ring so the dot stays visible on any menu bar.
                    rgba[at..at + 4].copy_from_slice(&[22, 27, 38, 255]);
                }
                if d2 <= 3 * 3 {
                    rgba[at..at + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
                }
            }
        }
    }
    rgba
}

pub const TRAY_PX: u32 = TRAY_SIZE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_bitmap_is_a_full_rgba_square_with_the_dot_drawn() {
        let idle = tray_rgba(None);
        assert_eq!(idle.len(), (TRAY_PX * TRAY_PX * 4) as usize);
        assert!(idle.chunks(4).any(|p| p[3] > 0), "mark must not be empty");
        let busy = tray_rgba(Some([46, 160, 90]));
        let at =
            (((TRAY_PX as usize - DOT_INSET as usize) * TRAY_PX as usize + (TRAY_PX as usize - DOT_INSET as usize)) * 4) as usize;
        assert_eq!(&busy[at..at + 4], &[46, 160, 90, 255]);
        assert_ne!(idle, busy);
    }
}
