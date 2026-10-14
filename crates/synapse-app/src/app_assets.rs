//! Embedded assets: GPUI Kit's Lucide icons (UI.md §7) and the UI.md §6 fonts.
//!
//! GPUI Kit's `Assets` carry its 101 built-in icons; `ExtraIcons` adds the few
//! more we use (see `icons.rs`). Only the listed SVGs are embedded.

use gpui_kit::assets::{icon_assets, Assets};
use gpui_kit::{App, AssetSource, SharedString};
use std::borrow::Cow;

icon_assets!(
    ExtraIcons,
    [
        Activity,
        Clock,
        Cloud,
        Command,
        Contrast,
        CornerUpRight,
        Flag,
        Hand,
        Library,
        Lightbulb,
        MessageCircleMore,
        MessagesSquare,
        Power,
        Send,
        ShieldCheck,
        Users,
        Waypoints,
        Zap,
    ]
);

const MARK_SVG: &[u8] = include_bytes!("../../../assets/brand/synapse-mark.svg");

/// Asset source for the app: built-in icons, our extras, and the brand mark.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if path == crate::brand_mark::MARK_PATH {
            return Ok(Some(Cow::Borrowed(MARK_SVG)));
        }
        if let Some(bytes) = crate::cli_logo::load(path) {
            return Ok(Some(bytes));
        }
        match ExtraIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut out = Assets.list(path)?;
        out.extend(ExtraIcons.list(path)?);
        if crate::brand_mark::MARK_PATH.starts_with(path) {
            out.push(crate::brand_mark::MARK_PATH.into());
        }
        out.extend(crate::cli_logo::list(path).map(SharedString::from));
        out.sort();
        out.dedup();
        Ok(out)
    }
}

/// Bundled fonts (OFL, see `assets/fonts/`), weights pinned by `scripts/subset-fonts`.
const FONTS: [&[u8]; 9] = [
    include_bytes!("../../../assets/fonts/BricolageGrotesque-600.ttf"),
    include_bytes!("../../../assets/fonts/BricolageGrotesque-800.ttf"),
    include_bytes!("../../../assets/fonts/AtkinsonHyperlegibleNext-400.ttf"),
    include_bytes!("../../../assets/fonts/AtkinsonHyperlegibleNext-500.ttf"),
    include_bytes!("../../../assets/fonts/AtkinsonHyperlegibleNext-700.ttf"),
    include_bytes!("../../../assets/fonts/Newsreader-400.ttf"),
    include_bytes!("../../../assets/fonts/Newsreader-700.ttf"),
    include_bytes!("../../../assets/fonts/AtkinsonHyperlegibleMono-400.ttf"),
    include_bytes!("../../../assets/fonts/AtkinsonHyperlegibleMono-700.ttf"),
];

/// Register the bundled fonts with GPUI's text system. Call once at startup.
pub fn register_fonts(cx: &App) {
    let fonts = FONTS.iter().map(|b| Cow::Borrowed(*b)).collect();
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("could not load bundled fonts ({e}); using system fonts");
    }
}
