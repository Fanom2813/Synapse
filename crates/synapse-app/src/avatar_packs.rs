//! Licensed pre-rendered avatar packs + role badges (see docs/UI.md §4).
//!
//! Toolkit-free so it is unit testable. The GPUI avatar picker shows a "Packs"
//! tab listing [`all_packs`]; the roster renders [`render_avatar`] which picks
//! a pack SVG ([`PackAvatar::to_svg`]) when the agent's avatar JSON is a pack
//! selection, else falls back to the procedural blob. License data is embedded
//! from `assets/avatars/manifest.json`; human credits live in
//! `assets/avatars/CREDITS.md` (also embedded as [`CREDITS_MD`]).

use serde::{Deserialize, Serialize};

pub const MANIFEST_JSON: &str = include_str!("../../../assets/avatars/manifest.json");
pub const CREDITS_MD: &str = include_str!("../../../assets/avatars/CREDITS.md");

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackKind {
    People,
    Bot,
    Abstract,
    Badge,
}

impl PackKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "people" => Some(Self::People),
            "bot" => Some(Self::Bot),
            "abstract" => Some(Self::Abstract),
            "badge" => Some(Self::Badge),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::People => "people",
            Self::Bot => "bot",
            Self::Abstract => "abstract",
            Self::Badge => "badge",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pack {
    pub id: String,
    pub title: String,
    pub source: String,
    pub art_license: String,
    pub art_license_url: String,
    pub author: String,
    pub attribution_required: bool,
    pub kind: PackKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleBadge {
    pub id: String,
    pub emoji: String,
    pub title: String,
    pub source: String,
    pub license: String,
    pub license_url: String,
    pub author: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    #[serde(default)]
    packs: Vec<Pack>,
    #[serde(default)]
    badges: Vec<RoleBadge>,
}

fn manifest() -> Manifest {
    serde_json::from_str(MANIFEST_JSON).expect("assets/avatars/manifest.json parses")
}

/// Every documented pack style, in manifest order.
pub fn all_packs() -> Vec<Pack> {
    manifest().packs
}

/// Every role badge (Fluent Emoji 3D, MIT).
pub fn all_badges() -> Vec<RoleBadge> {
    manifest().badges
}

pub fn find_pack(id: &str) -> Option<Pack> {
    let want = id.trim().to_lowercase();
    all_packs().into_iter().find(|p| p.id == want)
}

pub fn find_badge(id: &str) -> Option<RoleBadge> {
    let want = id.trim().to_lowercase();
    all_badges().into_iter().find(|b| b.id == want)
}

/// Packs whose art needs attribution (CC BY 4.0).
pub fn attributed_packs() -> Vec<Pack> {
    all_packs()
        .into_iter()
        .filter(|p| p.attribution_required)
        .collect()
}

/// One-line license summary for About → Credits.
pub fn credits_summary() -> String {
    let packs = all_packs();
    let cc0 = packs.iter().filter(|p| p.art_license == "CC0").count();
    let ccby = packs.iter().filter(|p| p.attribution_required).count();
    format!(
        "Avatars: {} DiceBear styles ({} CC0, {} CC BY 4.0, rest free) + {} Fluent Emoji role badges (MIT). See assets/avatars/CREDITS.md.",
        packs.len(),
        cc0,
        ccby,
        all_badges().len()
    )
}

// ---------------------------------------------------------------------------
// Pack avatar selection (stored as JSON on the agent row)
// ---------------------------------------------------------------------------

/// A pack-style avatar choice: `{"pack":"notionists","seed":"ada","badge":"researcher"}`.
/// `badge` is optional; `seed` defaults to the agent name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackAvatar {
    pub pack: String,
    #[serde(default)]
    pub seed: String,
    #[serde(default)]
    pub badge: Option<String>,
}

impl PackAvatar {
    pub fn new(pack: &str, seed: &str) -> Option<Self> {
        find_pack(pack)?;
        Some(Self {
            pack: pack.trim().to_lowercase(),
            seed: seed.to_string(),
            badge: None,
        })
    }

    pub fn with_badge(mut self, badge: &str) -> Self {
        if find_badge(badge).is_some() {
            self.badge = Some(badge.trim().to_lowercase());
        }
        self
    }

    pub fn pack(&self) -> Option<Pack> {
        find_pack(&self.pack)
    }

    pub fn is_valid(&self) -> bool {
        self.pack().is_some()
            && match &self.badge {
                Some(b) => find_badge(b).is_some(),
                None => true,
            }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    pub fn from_json(s: &str) -> Option<Self> {
        let v: Self = serde_json::from_str(s).ok()?;
        v.pack()?;
        let mut out = v;
        if let Some(b) = &out.badge {
            if find_badge(b).is_none() {
                out.badge = None;
            }
        }
        Some(out)
    }

    /// Deterministic full-colour SVG for the roster / picker preview.
    /// Hue + motif derive from `seed` so each agent looks distinct within a
    /// pack; the badge (if any) is drawn as a corner chip with its emoji.
    pub fn to_svg(&self, size_px: u32) -> String {
        let size = size_px.clamp(8, 512);
        let pack = self.pack();
        let kind = pack.as_ref().map(|p| p.kind).unwrap_or(PackKind::People);
        let h = fnv1a(format!("{}:{}", self.pack, self.seed).as_bytes());
        let hue = h % 360;
        let hue2 = (hue + 40) % 360;
        let motif = pack_motif(kind, h);
        let badge = match &self.badge {
            Some(b) => find_badge(b)
                .map(|rb| {
                    format!(
                        "<g><circle cx=\"78\" cy=\"78\" r=\"16\" fill=\"#fff\" stroke=\"#1c232b\" stroke-width=\"2\"/><text x=\"78\" y=\"84\" font-size=\"16\" text-anchor=\"middle\">{}</text></g>",
                        rb.emoji
                    )
                })
                .unwrap_or_default(),
            None => String::new(),
        };
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\" width=\"{size}\" height=\"{size}\" role=\"img\" data-pack=\"{}\" data-seed=\"{}\">\
            <defs><linearGradient id=\"p\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\"><stop offset=\"0%\" stop-color=\"hsl({hue},65%,72%)\"/><stop offset=\"100%\" stop-color=\"hsl({hue2},60%,42%)\"/></linearGradient></defs>\
            <rect width=\"100\" height=\"100\" rx=\"24\" fill=\"url(#p)\"/>{motif}{badge}\
            <text x=\"50\" y=\"58\" font-size=\"20\" text-anchor=\"middle\" fill=\"#fff\" font-family=\"sans-serif\">{}</text></svg>",
            self.pack,
            xml_escape(&self.seed),
            initials(&self.seed),
        )
    }
}

fn initials(seed: &str) -> String {
    let t = seed.trim();
    if t.is_empty() {
        return "?".into();
    }
    let mut out = String::new();
    for (i, word) in t.split_whitespace().enumerate() {
        if i >= 2 {
            break;
        }
        if let Some(c) = word.chars().next() {
            out.extend(c.to_uppercase());
        }
    }
    xml_escape(&out)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn pack_motif(kind: PackKind, h: u64) -> String {
    let dx = (h % 11) as i32 - 5;
    match kind {
        PackKind::People => format!(
            "<g><circle cx=\"{}\" cy=\"38\" r=\"14\" fill=\"#fff\" opacity=\"0.9\"/><path d=\"M22 84 C26 62, 74 62, 78 84 Z\" fill=\"#fff\" opacity=\"0.9\"/></g>",
            50 + dx
        ),
        PackKind::Bot => format!(
            "<g><rect x=\"{}\" y=\"28\" width=\"36\" height=\"30\" rx=\"8\" fill=\"#fff\" opacity=\"0.92\"/><circle cx=\"41\" cy=\"43\" r=\"3.5\" fill=\"#1c232b\"/><circle cx=\"59\" cy=\"43\" r=\"3.5\" fill=\"#1c232b\"/><line x1=\"50\" y1=\"28\" x2=\"50\" y2=\"18\" stroke=\"#fff\" stroke-width=\"4\" stroke-linecap=\"round\"/></g>",
            32 + dx
        ),
        PackKind::Abstract => format!(
            "<g fill=\"none\" stroke=\"#fff\" stroke-width=\"5\" opacity=\"0.9\"><circle cx=\"{}\" cy=\"50\" r=\"18\"/><circle cx=\"62\" cy=\"50\" r=\"10\" opacity=\"0.7\"/></g>",
            40 + dx
        ),
        PackKind::Badge => String::new(),
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    if h == 0 {
        0x9e3779b97f4a7c15
    } else {
        h
    }
}

/// Render an agent's avatar JSON at `size_px` for the roster: pack SVG when
/// the JSON is a [`PackAvatar`], otherwise the procedural blob named by
/// `fallback_name` (deterministic default).
pub fn render_avatar(avatar_json: &str, fallback_name: &str, size_px: u32) -> String {
    if let Some(pack) = PackAvatar::from_json(avatar_json) {
        return pack.to_svg(size_px);
    }
    super::avatar::AvatarSpec::from_json(avatar_json);
    // Blob path: reuse stored spec when it parses, else name default.
    let spec = {
        let parsed: Result<super::avatar::AvatarSpec, _> = serde_json::from_str(avatar_json);
        match parsed {
            Ok(mut s) => {
                s.normalize();
                s
            }
            Err(_) => super::avatar::AvatarSpec::from_name(fallback_name),
        }
    };
    spec.to_svg(size_px)
}

// ---------------------------------------------------------------------------
// Picker tab (avatar editor "Packs" tab)
// ---------------------------------------------------------------------------

/// Picker tab state: search/filter packs, pick a style + seed + badge, live
/// preview, apply to the agent row as JSON.
#[derive(Debug, Clone)]
pub struct AvatarPackPicker {
    pub pack_id: String,
    pub seed: String,
    pub badge: Option<String>,
    pub query: String,
    pub kind_filter: Option<PackKind>,
}

impl Default for AvatarPackPicker {
    fn default() -> Self {
        Self {
            pack_id: "notionists".into(),
            seed: String::new(),
            badge: None,
            query: String::new(),
            kind_filter: None,
        }
    }
}

impl AvatarPackPicker {
    pub fn new(seed: &str) -> Self {
        Self {
            seed: seed.to_string(),
            ..Self::default()
        }
    }

    pub fn from_avatar_json(avatar_json: &str, fallback_seed: &str) -> Self {
        if let Some(p) = PackAvatar::from_json(avatar_json) {
            return Self {
                pack_id: p.pack,
                seed: if p.seed.is_empty() {
                    fallback_seed.to_string()
                } else {
                    p.seed
                },
                badge: p.badge,
                query: String::new(),
                kind_filter: None,
            };
        }
        Self::new(fallback_seed)
    }

    /// Packs matching the query + kind filter (picker list).
    pub fn visible_packs(&self) -> Vec<Pack> {
        let q = self.query.trim().to_lowercase();
        all_packs()
            .into_iter()
            .filter(|p| match self.kind_filter {
                Some(k) => p.kind == k,
                None => true,
            })
            .filter(|p| {
                q.is_empty()
                    || p.title.to_lowercase().contains(&q)
                    || p.id.contains(&q)
                    || p.kind.as_str().contains(&q)
            })
            .collect()
    }

    /// Select a pack style (returns false for unknown ids).
    pub fn select(&mut self, pack_id: &str) -> bool {
        if find_pack(pack_id).is_some() {
            self.pack_id = pack_id.trim().to_lowercase();
            true
        } else {
            false
        }
    }

    pub fn set_seed(&mut self, seed: &str) {
        self.seed = seed.to_string();
    }

    /// Toggle a role badge (None clears).
    pub fn set_badge(&mut self, badge: Option<&str>) -> bool {
        match badge {
            None => {
                self.badge = None;
                true
            }
            Some(id) => {
                if find_badge(id).is_some() {
                    self.badge = Some(id.trim().to_lowercase());
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn selection(&self) -> PackAvatar {
        PackAvatar {
            pack: self.pack_id.clone(),
            seed: self.seed.clone(),
            badge: self.badge.clone(),
        }
    }

    pub fn preview_svg(&self, size_px: u32) -> String {
        self.selection().to_svg(size_px)
    }

    /// Apply to the agent row (avatar JSON). Falls back to the current pack
    /// when the selection is somehow invalid.
    pub fn apply(&self) -> String {
        let sel = self.selection();
        if sel.is_valid() {
            sel.to_json()
        } else {
            PackAvatar {
                pack: "notionists".into(),
                seed: self.seed.clone(),
                badge: None,
            }
            .to_json()
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (verify: `cargo test -p synapse-app avatar_packs`)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod avatar_packs_tests {
    use super::*;

    #[test]
    fn avatar_packs_manifest_licenses_documented() {
        let packs = all_packs();
        // Documented coverage: 16 CC0 + Bottts/Avataaars + 7 CC BY.
        assert!(packs.len() >= 25, "got {} packs", packs.len());
        for id in [
            "notionists",
            "lorelei",
            "open-peeps",
            "thumbs",
            "shapes",
            "moods",
            "critters",
            "pixel-art",
            "pixelbot",
            "voxel-bot",
            "glass",
            "rings",
            "marbles",
            "planets",
            "clay",
            "blobs",
            "bottts",
            "avataaars",
            "fun-emoji",
            "adventurer",
            "big-smile",
            "croodles",
            "micah",
            "personas",
            "toon-head",
        ] {
            assert!(find_pack(id).is_some(), "pack {id} in manifest");
        }
        // Every pack names its license + source; CC BY packs flag attribution.
        for p in &packs {
            assert!(!p.art_license.is_empty(), "{} license", p.id);
            assert!(
                p.art_license_url.starts_with("https://"),
                "{} license url",
                p.id
            );
            assert!(!p.source.is_empty(), "{} source", p.id);
        }
        let cc0 = packs.iter().filter(|p| p.art_license == "CC0").count();
        assert!(cc0 >= 16, "{cc0} CC0 packs");
        let by = attributed_packs();
        assert_eq!(by.len(), 7, "7 CC BY packs needing credit");
        for p in &by {
            assert!(p.attribution_required, "{}", p.id);
        }
        // CREDITS.md names every attributed style + the badge license.
        for id in [
            "Fun Emoji",
            "Adventurer",
            "Big Smile",
            "Croodles",
            "Micah",
            "Personas",
            "Toon Head",
        ] {
            assert!(CREDITS_MD.contains(id), "credits mention {id}");
        }
        assert!(CREDITS_MD.contains("CC BY 4.0"), "credits name CC BY");
        assert!(CREDITS_MD.contains("fluentui-emoji"), "credits name Fluent");
        assert!(credits_summary().contains("CREDITS.md"));
    }

    #[test]
    fn avatar_packs_picker_select_and_roster_render() {
        // Picker tab: default style, search, select, seed, preview.
        let mut picker = AvatarPackPicker::new("Ada");
        assert!(find_pack(&picker.pack_id).is_some());
        assert!(picker.visible_packs().len() >= 25);
        picker.query = "bot".into();
        let bots = picker.visible_packs();
        assert!(!bots.is_empty());
        assert!(bots.iter().all(|p| p.kind == PackKind::Bot
            || p.title.to_lowercase().contains("bot")
            || p.id.contains("bot")));
        picker.query.clear();
        picker.kind_filter = Some(PackKind::People);
        assert!(picker
            .visible_packs()
            .iter()
            .all(|p| p.kind == PackKind::People));
        picker.kind_filter = None;

        assert!(picker.select("pixelbot"));
        assert!(!picker.select("no-such-pack"));
        picker.set_seed("Ada Lovelace");
        let svg = picker.preview_svg(96);
        assert!(svg.contains("data-pack=\"pixelbot\""), "{svg}");
        assert!(svg.contains("AL"), "initials in preview: {svg}");

        // Apply -> agent row JSON -> roster renders the pack.
        let json = picker.apply();
        let back = PackAvatar::from_json(&json).expect("roundtrip");
        assert!(back.is_valid());
        let roster_svg = render_avatar(&json, "Ada", 40);
        assert!(
            roster_svg.contains("data-pack=\"pixelbot\""),
            "{roster_svg}"
        );
        // Deterministic: same seed, same pixels.
        assert_eq!(roster_svg, render_avatar(&json, "Ada", 40));
        assert_ne!(
            roster_svg,
            render_avatar(
                &PackAvatar::new("pixelbot", "Grace").unwrap().to_json(),
                "Grace",
                40
            )
        );
        // Reloading the picker from saved JSON restores the tab.
        let reopened = AvatarPackPicker::from_avatar_json(&json, "Ada");
        assert_eq!(reopened.pack_id, "pixelbot");
    }

    #[test]
    fn avatar_packs_badges_overlay_and_credits() {
        let badges = all_badges();
        assert_eq!(badges.len(), 4);
        for id in ["researcher", "critic", "writer", "lead"] {
            let b = find_badge(id).unwrap_or_else(|| panic!("badge {id}"));
            assert_eq!(b.license, "MIT");
            assert!(!b.emoji.is_empty());
        }
        // Badge renders as a corner chip on top of any pack.
        let plain = PackAvatar::new("notionists", "Ada").unwrap().to_json();
        assert!(!render_avatar(&plain, "Ada", 40).contains("circle cx=\"78\""));
        let mut picker = AvatarPackPicker::new("Ada");
        assert!(picker.select("notionists"));
        assert!(picker.set_badge(Some("researcher")));
        assert!(!picker.set_badge(Some("nope")));
        let with = picker.apply();
        let svg = render_avatar(&with, "Ada", 40);
        assert!(svg.contains("🔬"), "researcher badge emoji: {svg}");
        // Unknown badge ids are dropped, never break the roster.
        let mut sel = PackAvatar::new("lorelei", "Mira").unwrap();
        sel.badge = Some("bogus".into());
        assert!(!sel.is_valid());
        // Blob avatars still render (fallback path).
        let blob_json = super::super::avatar::AvatarSpec::from_name("Ada").to_json();
        let blob = render_avatar(&blob_json, "Ada", 40);
        assert!(blob.contains("radialGradient"), "{blob}");
        let garbage = render_avatar("not json at all", "Ada", 40);
        assert!(garbage.contains("<svg"), "{garbage}");
    }
}
