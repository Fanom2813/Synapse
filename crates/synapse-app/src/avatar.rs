//! Agent blob avatars: deterministic procedural "dots" (see docs/UI.md §4).
//!
//! Toolkit-free so it is unit testable. The GPUI view layer renders
//! [`AvatarSpec::to_svg`] into an `img` (SVG source) or redraws the same
//! geometry with `canvas` + `PathBuilder`; state motion comes from
//! [`AvatarState::should_animate`] and [`AvatarState::motion`].

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Options (UI.md §4)
// ---------------------------------------------------------------------------

/// Blob shapes: 7 blobs + 4 animal-ish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    #[default]
    Bean,
    Round,
    Pear,
    Cloud,
    Star,
    SoftSquare,
    Drop,
    Cat,
    Bird,
    Bear,
    Fish,
}

impl Shape {
    pub fn all() -> &'static [Shape] {
        &[
            Shape::Bean,
            Shape::Round,
            Shape::Pear,
            Shape::Cloud,
            Shape::Star,
            Shape::SoftSquare,
            Shape::Drop,
            Shape::Cat,
            Shape::Bird,
            Shape::Bear,
            Shape::Fish,
        ]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bean => "bean",
            Self::Round => "round",
            Self::Pear => "pear",
            Self::Cloud => "cloud",
            Self::Star => "star",
            Self::SoftSquare => "soft_square",
            Self::Drop => "drop",
            Self::Cat => "cat",
            Self::Bird => "bird",
            Self::Bear => "bear",
            Self::Fish => "fish",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_lowercase().as_str() {
            "bean" => Self::Bean,
            "round" => Self::Round,
            "pear" => Self::Pear,
            "cloud" => Self::Cloud,
            "star" | "star-ish" | "starish" => Self::Star,
            "soft_square" | "square-soft" | "squaresoft" | "softsquare" => Self::SoftSquare,
            "drop" => Self::Drop,
            "cat" => Self::Cat,
            "bird" => Self::Bird,
            "bear" => Self::Bear,
            "fish" => Self::Fish,
            _ => return None,
        })
    }
}

/// Eye styles; diamond is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Eyes {
    #[default]
    Diamond,
    Dot,
    Beady,
    Sleepy,
    Happy,
    Wide,
}

impl Eyes {
    pub fn all() -> &'static [Eyes] {
        &[
            Self::Diamond,
            Self::Dot,
            Self::Beady,
            Self::Sleepy,
            Self::Happy,
            Self::Wide,
        ]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Diamond => "diamond",
            Self::Dot => "dot",
            Self::Beady => "beady",
            Self::Sleepy => "sleepy",
            Self::Happy => "happy",
            Self::Wide => "wide",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_lowercase().as_str() {
            "diamond" => Self::Diamond,
            "dot" => Self::Dot,
            "beady" => Self::Beady,
            "sleepy" => Self::Sleepy,
            "happy" | "happy_arcs" | "happy-arcs" => Self::Happy,
            "wide" => Self::Wide,
            _ => return None,
        })
    }
}

/// Mouth styles; none is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mouth {
    #[default]
    None,
    Smile,
    O,
}

impl Mouth {
    pub fn all() -> &'static [Mouth] {
        &[Self::None, Self::Smile, Self::O]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Smile => "smile",
            Self::O => "o",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_lowercase().as_str() {
            "none" | "" => Self::None,
            "smile" => Self::Smile,
            "o" => Self::O,
            _ => return None,
        })
    }
}

/// Accessories; at most [`MAX_ACCESSORIES`] per avatar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accessory {
    Glasses,
    Sunglasses,
    BowTie,
    Beret,
    Cap,
    Headphones,
    Crown,
    Wreath,
}

impl Accessory {
    pub fn all() -> &'static [Accessory] {
        &[
            Self::Glasses,
            Self::Sunglasses,
            Self::BowTie,
            Self::Beret,
            Self::Cap,
            Self::Headphones,
            Self::Crown,
            Self::Wreath,
        ]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Glasses => "glasses",
            Self::Sunglasses => "sunglasses",
            Self::BowTie => "bow_tie",
            Self::Beret => "beret",
            Self::Cap => "cap",
            Self::Headphones => "headphones",
            Self::Crown => "crown",
            Self::Wreath => "wreath",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_lowercase().replace('-', "_").as_str() {
            "glasses" => Self::Glasses,
            "sunglasses" => Self::Sunglasses,
            "bow_tie" | "bowtie" => Self::BowTie,
            "beret" => Self::Beret,
            "cap" => Self::Cap,
            "headphones" => Self::Headphones,
            "crown" => Self::Crown,
            "wreath" => Self::Wreath,
            _ => return None,
        })
    }
}

pub const MAX_ACCESSORIES: usize = 2;

/// 12 preset palettes: base + highlight gradient stops (UI.md §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub name: &'static str,
    pub base: &'static str,
    pub highlight: &'static str,
}

pub const PALETTES: [Palette; 12] = [
    Palette {
        name: "leaf",
        base: "#3fa34d",
        highlight: "#9bf29b",
    },
    Palette {
        name: "teal",
        base: "#1d9e8e",
        highlight: "#a7efe5",
    },
    Palette {
        name: "ocean",
        base: "#2f7de1",
        highlight: "#a9ccff",
    },
    Palette {
        name: "grape",
        base: "#7c5cd6",
        highlight: "#d3c2ff",
    },
    Palette {
        name: "berry",
        base: "#d6457c",
        highlight: "#ffc0da",
    },
    Palette {
        name: "sunset",
        base: "#e86a33",
        highlight: "#ffd3a8",
    },
    Palette {
        name: "honey",
        base: "#d9a91e",
        highlight: "#ffec9e",
    },
    Palette {
        name: "ember",
        base: "#d63a2f",
        highlight: "#ffb3a3",
    },
    Palette {
        name: "lime",
        base: "#7cb518",
        highlight: "#ddf39a",
    },
    Palette {
        name: "sky",
        base: "#23b5d3",
        highlight: "#b3ecff",
    },
    Palette {
        name: "indigo",
        base: "#3e4cc9",
        highlight: "#b9c2ff",
    },
    Palette {
        name: "stone",
        base: "#6b7280",
        highlight: "#d7dde5",
    },
];

/// Supported render sizes (UI.md §4); below 24 px only colour + eyes.
pub const AVATAR_SIZES: [u32; 5] = [16, 24, 40, 96, 160];
pub const MIN_FULL_DETAIL_SIZE: u32 = 24;
/// Idle avatars settle to a still pose after 30 s (PERFORMANCE.md).
pub const IDLE_SETTLE_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// Spec
// ---------------------------------------------------------------------------

/// Full avatar look, stored as JSON on the agent row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvatarSpec {
    #[serde(default)]
    pub shape: Shape,
    /// Index into [`PALETTES`]; ignored when `custom` is set.
    #[serde(default)]
    pub palette: usize,
    /// Custom base/highlight hex pair; `None` = use preset.
    #[serde(default)]
    pub custom: Option<(String, String)>,
    #[serde(default)]
    pub eyes: Eyes,
    #[serde(default)]
    pub mouth: Mouth,
    #[serde(default)]
    pub accessories: Vec<Accessory>,
    /// Deterministic seed for "surprise me" / blob wobble.
    #[serde(default)]
    pub seed: u64,
}

impl Default for AvatarSpec {
    fn default() -> Self {
        Self::default_lead()
    }
}

impl AvatarSpec {
    /// Default lead: green bean with two diamond eyes (UI.md §4).
    pub fn default_lead() -> Self {
        Self {
            shape: Shape::Bean,
            palette: 0,
            custom: None,
            eyes: Eyes::Diamond,
            mouth: Mouth::None,
            accessories: Vec::new(),
            seed: 0,
        }
    }

    /// Deterministic look derived from the agent name.
    pub fn from_name(name: &str) -> Self {
        let h = fnv1a(name.trim().to_lowercase().as_bytes());
        let mut rng = XorShift::new(h);
        Self {
            shape: Shape::all()[rng.next_usize(Shape::all().len())],
            palette: rng.next_usize(PALETTES.len()),
            custom: None,
            eyes: Eyes::all()[rng.next_usize(Eyes::all().len())],
            mouth: Mouth::all()[rng.next_usize(Mouth::all().len())],
            accessories: Vec::new(),
            seed: h,
        }
    }

    /// Deterministic shuffle from an explicit seed ("surprise me" core).
    pub fn surprise_me_from(seed: u64) -> Self {
        let mut rng = XorShift::new(seed);
        let mut accessories = Vec::new();
        // 0–2 accessories, no duplicates.
        let n = rng.next_usize(MAX_ACCESSORIES + 1);
        for _ in 0..n {
            let a = Accessory::all()[rng.next_usize(Accessory::all().len())];
            if !accessories.contains(&a) {
                accessories.push(a);
            }
        }
        Self {
            shape: Shape::all()[rng.next_usize(Shape::all().len())],
            palette: rng.next_usize(PALETTES.len()),
            custom: None,
            eyes: Eyes::all()[rng.next_usize(Eyes::all().len())],
            mouth: Mouth::all()[rng.next_usize(Mouth::all().len())],
            accessories,
            seed,
        }
    }

    pub fn colours(&self) -> (String, String) {
        if let Some((b, h)) = &self.custom {
            return (b.clone(), h.clone());
        }
        let p = PALETTES[self.palette.min(PALETTES.len() - 1)];
        (p.base.to_string(), p.highlight.to_string())
    }

    /// Validate + clamp (unknown indices, >2 accessories, bad hex).
    /// Returns human-readable fixes applied.
    pub fn normalize(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        if self.palette >= PALETTES.len() {
            self.palette = 0;
            notes.push("unknown palette: reset to leaf".into());
        }
        if let Some((b, h)) = &self.custom {
            if !is_hex(b) || !is_hex(h) {
                self.custom = None;
                notes.push("bad custom colour: reset to preset".into());
            }
        }
        if self.accessories.len() > MAX_ACCESSORIES {
            self.accessories.truncate(MAX_ACCESSORIES);
            notes.push("at most 2 accessories: extras dropped".into());
        }
        let mut deduped = Vec::new();
        for a in &self.accessories {
            if !deduped.contains(a) {
                deduped.push(*a);
            }
        }
        if deduped.len() != self.accessories.len() {
            notes.push("duplicate accessories dropped".into());
        }
        self.accessories = deduped;
        notes
    }

    pub fn is_valid(&self) -> bool {
        self.palette < PALETTES.len()
            && self.accessories.len() <= MAX_ACCESSORIES
            && match &self.custom {
                Some((b, h)) => is_hex(b) && is_hex(h),
                None => true,
            }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    pub fn from_json(s: &str) -> Self {
        let mut spec: Self = serde_json::from_str(s).unwrap_or_else(|_| Self::default_lead());
        spec.normalize();
        spec
    }

    /// Minimal (colour + eyes) below 24 px, full detail at/above.
    pub fn detail(size_px: u32) -> Detail {
        if size_px < MIN_FULL_DETAIL_SIZE {
            Detail::Minimal
        } else {
            Detail::Full
        }
    }

    /// Deterministic SVG render at `size_px` (square viewBox).
    pub fn to_svg(&self, size_px: u32) -> String {
        let size = size_px.clamp(8, 512);
        let (base, hi) = self.colours();
        let body = body_path(self.shape, self.seed);
        let eyes = eyes_svg(self.eyes, matches!(Self::detail(size), Detail::Minimal));
        let mouth = if matches!(Self::detail(size), Detail::Full) {
            mouth_svg(self.mouth)
        } else {
            String::new()
        };
        let extras = if matches!(Self::detail(size), Detail::Full) {
            accessories_svg(&self.accessories)
        } else {
            String::new()
        };
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\" width=\"{size}\" height=\"{size}\" role=\"img\">\
            <defs><radialGradient id=\"g\" cx=\"35%\" cy=\"30%\" r=\"80%\"><stop offset=\"0%\" stop-color=\"{hi}\"/><stop offset=\"100%\" stop-color=\"{base}\"/></radialGradient></defs>\
            <path d=\"{body}\" fill=\"url(#g)\"/>\
            {eyes}{mouth}{extras}</svg>"
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    Minimal,
    Full,
}

fn is_hex(s: &str) -> bool {
    let b = s.as_bytes();
    (b.len() == 7 || b.len() == 4) && b[0] == b'#' && b[1..].iter().all(|c| c.is_ascii_hexdigit())
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    // Never zero (xorshift needs nonzero state).
    if h == 0 {
        0x9e3779b97f4a7c15
    } else {
        h
    }
}

struct XorShift(u64);

impl XorShift {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x9e3779b97f4a7c15 } else { seed })
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn next_usize(&mut self, n: usize) -> usize {
        (self.next() % (n as u64)) as usize
    }
}

/// Blob body paths on a 0–100 viewBox; `seed` adds a small deterministic
/// wobble (±2 units) so same-shape agents differ slightly.
fn body_path(shape: Shape, seed: u64) -> String {
    let w = ((seed % 5) as i32) - 2;
    let b = |x: i32| x + w;
    match shape {
        Shape::Bean => format!("M{} 12 C 66 6, 90 30, 84 58 C 78 86, 44 96, 24 86 C 6 77, {} 60, 12 38 C 16 22, 30 15, {} 12 Z", b(34), b(8), b(34)),
        Shape::Round => "M50 8 C 74 8, 92 28, 92 52 C 92 76, 73 93, 50 93 C 27 93, 8 76, 8 52 C 8 28, 26 8, 50 8 Z".into(),
        Shape::Pear => "M50 8 C 66 8, 72 24, 68 36 C 80 46, 88 62, 82 76 C 75 90, 62 94, 50 94 C 38 94, 25 90, 18 76 C 12 62, 20 46, 32 36 C 28 24, 34 8, 50 8 Z".into(),
        Shape::Cloud => "M28 88 C 12 88, 6 74, 14 64 C 6 56, 12 42, 24 42 C 28 30, 44 26, 52 34 C 60 24, 78 28, 80 40 C 92 42, 94 58, 86 64 C 94 74, 86 88, 72 88 Z".into(),
        Shape::Star => "M50 6 L60 30 L86 32 L66 50 L73 76 L50 62 L27 76 L34 50 L14 32 L40 30 Z".into(),
        Shape::SoftSquare => "M24 10 H76 C 88 10, 92 16, 92 28 V72 C 92 84, 88 90, 76 90 H24 C 12 90, 8 84, 8 72 V28 C 8 16, 12 10, 24 10 Z".into(),
        Shape::Drop => "M50 4 C 62 22, 90 44, 90 66 C 90 86, 71 94, 50 94 C 29 94, 10 86, 10 66 C 10 44, 38 22, 50 4 Z".into(),
        Shape::Cat => "M18 40 L14 12 L36 26 C 44 22, 56 22, 64 26 L86 12 L82 40 C 90 52, 90 74, 76 84 C 64 92, 36 92, 24 84 C 10 74, 10 52, 18 40 Z".into(),
        Shape::Bird => "M30 88 C 18 76, 14 52, 28 34 C 38 20, 58 14, 72 24 L88 18 L82 32 C 90 44, 86 66, 72 78 C 60 88, 40 92, 30 88 Z".into(),
        Shape::Bear => "M20 30 C 12 30, 8 20, 14 14 C 19 9, 29 12, 30 20 C 36 15, 44 13, 50 13 C 56 13, 64 15, 70 20 C 71 12, 81 9, 86 14 C 92 20, 88 30, 80 30 C 86 42, 88 62, 78 76 C 68 89, 32 89, 22 76 C 12 62, 14 42, 20 30 Z".into(),
        Shape::Fish => "M8 50 L26 36 C 34 26, 52 22, 66 32 L78 26 L76 38 L78 50 L76 62 L78 74 L66 68 C 52 78, 34 74, 26 64 Z".into(),
    }
}

fn eyes_svg(eyes: Eyes, minimal: bool) -> String {
    // Two eyes centred around (38/62, 52). Minimal still shows eyes.
    match eyes {
        Eyes::Diamond => "<g fill=\"#1c232b\"><path d=\"M38 44 L44 52 L38 60 L32 52 Z\"/><path d=\"M62 44 L68 52 L62 60 L56 52 Z\"/></g>".into(),
        Eyes::Dot => "<g fill=\"#1c232b\"><circle cx=\"38\" cy=\"52\" r=\"5\"/><circle cx=\"62\" cy=\"52\" r=\"5\"/></g>".into(),
        Eyes::Beady => "<g fill=\"#1c232b\"><circle cx=\"38\" cy=\"52\" r=\"3\"/><circle cx=\"62\" cy=\"52\" r=\"3\"/></g>".into(),
        Eyes::Sleepy => "<g stroke=\"#1c232b\" stroke-width=\"3\" stroke-linecap=\"round\"><line x1=\"31\" y1=\"52\" x2=\"45\" y2=\"54\"/><line x1=\"55\" y1=\"54\" x2=\"69\" y2=\"52\"/></g>".into(),
        Eyes::Happy => "<g fill=\"none\" stroke=\"#1c232b\" stroke-width=\"3\" stroke-linecap=\"round\"><path d=\"M31 55 A8 8 0 0 1 45 55\"/><path d=\"M55 55 A8 8 0 0 1 69 55\"/></g>".into(),
        Eyes::Wide => {
            let extra = if minimal { String::new() } else { "<circle cx=\"39.5\" cy=\"50.5\" r=\"1.6\" fill=\"#fff\"/><circle cx=\"63.5\" cy=\"50.5\" r=\"1.6\" fill=\"#fff\"/>".to_string() };
            format!("<g><ellipse cx=\"38\" cy=\"52\" rx=\"7\" ry=\"8.5\" fill=\"#fff\" stroke=\"#1c232b\" stroke-width=\"2.5\"/><ellipse cx=\"62\" cy=\"52\" rx=\"7\" ry=\"8.5\" fill=\"#fff\" stroke=\"#1c232b\" stroke-width=\"2.5\"/><circle cx=\"38\" cy=\"53\" r=\"3\" fill=\"#1c232b\"/><circle cx=\"62\" cy=\"53\" r=\"3\" fill=\"#1c232b\"/>{extra}</g>")
        }
    }
}

fn mouth_svg(mouth: Mouth) -> String {
    match mouth {
        Mouth::None => String::new(),
        Mouth::Smile => "<path d=\"M41 68 Q50 75 59 68\" fill=\"none\" stroke=\"#1c232b\" stroke-width=\"3\" stroke-linecap=\"round\"/>".into(),
        Mouth::O => "<ellipse cx=\"50\" cy=\"70\" rx=\"5\" ry=\"6\" fill=\"#1c232b\"/>".into(),
    }
}

fn accessories_svg(list: &[Accessory]) -> String {
    let mut out = String::new();
    for a in list {
        out.push_str(match a {
            Accessory::Glasses => "<g fill=\"none\" stroke=\"#1c232b\" stroke-width=\"2.5\"><circle cx=\"38\" cy=\"52\" r=\"9\"/><circle cx=\"62\" cy=\"52\" r=\"9\"/><line x1=\"47\" y1=\"52\" x2=\"53\" y2=\"52\"/></g>",
            Accessory::Sunglasses => "<g fill=\"#1c232b\"><rect x=\"28\" y=\"44\" width=\"20\" height=\"13\" rx=\"4\"/><rect x=\"52\" y=\"44\" width=\"20\" height=\"13\" rx=\"4\"/><rect x=\"47\" y=\"48\" width=\"6\" height=\"3\"/></g>",
            Accessory::BowTie => "<g fill=\"#d63a2f\"><path d=\"M50 84 L38 78 L38 90 Z\"/><path d=\"M50 84 L62 78 L62 90 Z\"/><circle cx=\"50\" cy=\"84\" r=\"3\" fill=\"#7c1f18\"/></g>",
            Accessory::Beret => "<g><ellipse cx=\"58\" cy=\"18\" rx=\"24\" ry=\"10\" fill=\"#3e4cc9\"/><rect x=\"34\" y=\"22\" width=\"44\" height=\"5\" rx=\"2\" fill=\"#2b3590\"/></g>",
            Accessory::Cap => "<g><path d=\"M28 28 C30 12, 70 12, 72 28 Z\" fill=\"#e86a33\"/><rect x=\"24\" y=\"26\" width=\"56\" height=\"6\" rx=\"3\" fill=\"#b34a1f\"/></g>",
            Accessory::Headphones => "<g fill=\"none\" stroke=\"#1c232b\" stroke-width=\"4\"><path d=\"M26 60 C26 30, 74 30, 74 60\"/><rect x=\"20\" y=\"52\" width=\"10\" height=\"16\" rx=\"4\" fill=\"#1c232b\"/><rect x=\"70\" y=\"52\" width=\"10\" height=\"16\" rx=\"4\" fill=\"#1c232b\"/></g>",
            Accessory::Crown => "<g fill=\"#d9a91e\" stroke=\"#8a6410\" stroke-width=\"1.5\"><path d=\"M36 20 L40 8 L47 17 L50 5 L53 17 L60 8 L64 20 Z\"/></g>",
            Accessory::Wreath => "<g fill=\"none\" stroke=\"#3fa34d\" stroke-width=\"3\"><circle cx=\"32\" cy=\"80\" r=\"5\"/><circle cx=\"68\" cy=\"80\" r=\"5\"/><path d=\"M37 80 Q50 88 63 80\"/></g>",
        });
    }
    out
}

// ---------------------------------------------------------------------------
// States (UI.md §4 table)
// ---------------------------------------------------------------------------

/// Live avatar state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AvatarState {
    #[default]
    Idle,
    Thinking,
    Speaking,
    Working,
    NeedsYou,
    Paused,
    Error,
}

impl AvatarState {
    pub fn all() -> &'static [AvatarState] {
        &[
            Self::Idle,
            Self::Thinking,
            Self::Speaking,
            Self::Working,
            Self::NeedsYou,
            Self::Paused,
            Self::Error,
        ]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Thinking => "thinking",
            Self::Speaking => "speaking",
            Self::Working => "working",
            Self::NeedsYou => "needs_you",
            Self::Paused => "paused",
            Self::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_lowercase().replace('-', "_").as_str() {
            "idle" => Self::Idle,
            "thinking" => Self::Thinking,
            "speaking" | "streaming" => Self::Speaking,
            "working" => Self::Working,
            "needs_you" | "needsyou" | "needs-you" => Self::NeedsYou,
            "paused" => Self::Paused,
            "error" => Self::Error,
            _ => return None,
        })
    }

    /// Short motion description for the view layer (GPUI `with_animation`).
    pub fn motion(self) -> &'static str {
        match self {
            Self::Idle => "float ±2px 4s, blink 3–7s, squash on hover",
            Self::Thinking => "wobble, eyes look up-left/right",
            Self::Speaking => "bounce on delta (debounced), mouth/eye pulse",
            Self::Working => "orbiting dots around blob",
            Self::NeedsYou => "bounce + badge, colour pulse 2s (stops after 10s)",
            Self::Paused => "dimmed, eyes closed, no motion",
            Self::Error => "shake once, red badge",
        }
    }

    /// Whether this state ever animates on its own (paused never does).
    pub fn animates(self) -> bool {
        !matches!(self, Self::Paused)
    }

    /// Gate for the frame budget: no frames when the window is hidden, the
    /// avatar is off-screen, reduce-motion is on (except badge-worthy
    /// needs-you/error shown still), the size is below 24 px, the state is
    /// paused, or an idle avatar has settled after 30 s.
    pub fn should_animate(self, ctx: &AnimateCtx) -> bool {
        if !ctx.window_visible || !ctx.on_screen {
            return false;
        }
        if ctx.reduce_motion {
            return false;
        }
        if !self.animates() {
            return false;
        }
        if ctx.size_px < MIN_FULL_DETAIL_SIZE {
            return false;
        }
        if self == Self::Idle && ctx.idle_secs >= IDLE_SETTLE_SECS {
            return false;
        }
        if self == Self::NeedsYou && ctx.state_secs >= 10 {
            // Pulse stops after 10 s; the badge stays (UI.md §4).
            return false;
        }
        true
    }
}

/// Inputs to [`AvatarState::should_animate`].
#[derive(Debug, Clone, Copy)]
pub struct AnimateCtx {
    pub window_visible: bool,
    pub on_screen: bool,
    pub reduce_motion: bool,
    pub size_px: u32,
    /// Seconds in the current state (idle settle / needs-you pulse cap).
    pub idle_secs: u64,
    pub state_secs: u64,
}

impl Default for AnimateCtx {
    fn default() -> Self {
        Self {
            window_visible: true,
            on_screen: true,
            reduce_motion: false,
            size_px: 40,
            idle_secs: 0,
            state_secs: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Editor (preview + choose + surprise-me, UI.md §9 step 3)
// ---------------------------------------------------------------------------

/// Avatar picker state: live preview plus one option per dimension.
#[derive(Debug, Clone)]
pub struct AvatarEditor {
    pub spec: AvatarSpec,
    pub preview_size: u32,
    surprise_count: u64,
}

impl Default for AvatarEditor {
    fn default() -> Self {
        Self {
            spec: AvatarSpec::default_lead(),
            preview_size: 160,
            surprise_count: 0,
        }
    }
}

impl AvatarEditor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_spec(spec: AvatarSpec) -> Self {
        Self {
            spec,
            preview_size: 160,
            surprise_count: 0,
        }
    }

    pub fn preview_svg(&self) -> String {
        self.spec.to_svg(self.preview_size)
    }

    pub fn set_shape(&mut self, shape: Shape) {
        self.spec.shape = shape;
    }

    pub fn set_palette(&mut self, index: usize) -> bool {
        if index < PALETTES.len() {
            self.spec.palette = index;
            self.spec.custom = None;
            true
        } else {
            false
        }
    }

    pub fn set_custom(&mut self, base: &str, highlight: &str) -> bool {
        if is_hex(base) && is_hex(highlight) {
            self.spec.custom = Some((base.to_string(), highlight.to_string()));
            true
        } else {
            false
        }
    }

    pub fn set_eyes(&mut self, eyes: Eyes) {
        self.spec.eyes = eyes;
    }

    pub fn set_mouth(&mut self, mouth: Mouth) {
        self.spec.mouth = mouth;
    }

    /// Toggle an accessory; at most 2. Returns false when a third distinct
    /// accessory was rejected.
    pub fn toggle_accessory(&mut self, accessory: Accessory) -> bool {
        if let Some(ix) = self.spec.accessories.iter().position(|a| *a == accessory) {
            self.spec.accessories.remove(ix);
            true
        } else if self.spec.accessories.len() < MAX_ACCESSORIES {
            self.spec.accessories.push(accessory);
            true
        } else {
            false
        }
    }

    /// "Surprise me": deterministic shuffle that bounces the preview.
    /// Same seed sequence always yields the same looks.
    pub fn surprise_me(&mut self) -> &AvatarSpec {
        // Chain off the current seed so repeated clicks walk the space.
        let base = self
            .spec
            .seed
            .wrapping_add(0x9e3779b97f4a7c15)
            .wrapping_add(self.surprise_count);
        self.surprise_count += 1;
        self.spec = AvatarSpec::surprise_me_from(base ^ 0x123456789abcdef);
        self.spec.seed = base ^ 0x123456789abcdef;
        &self.spec
    }

    pub fn apply(&self) -> AvatarSpec {
        self.spec.clone()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod avatar_render_tests {
    use super::*;

    #[test]
    fn avatar_render_shapes_palettes_eyes_mouth_accessories() {
        assert_eq!(Shape::all().len(), 11, "7 blobs + 4 animal-ish");
        for name in [
            "bean",
            "round",
            "pear",
            "cloud",
            "star-ish",
            "square-soft",
            "drop",
            "cat",
            "bird",
            "bear",
            "fish",
        ] {
            assert!(Shape::parse(name).is_some(), "shape {name}");
        }
        assert_eq!(Shape::default(), Shape::Bean);
        assert_eq!(PALETTES.len(), 12);
        assert_eq!(Eyes::all().len(), 6);
        assert_eq!(Eyes::default(), Eyes::Diamond);
        assert_eq!(Mouth::all().len(), 3);
        assert_eq!(Mouth::default(), Mouth::None);
        assert_eq!(Accessory::all().len(), 8);
        assert_eq!(MAX_ACCESSORIES, 2);
        assert!(Accessory::parse("bow tie").is_some() || Accessory::parse("bow_tie").is_some());
    }

    #[test]
    fn avatar_render_default_lead_and_name_deterministic() {
        let lead = AvatarSpec::default_lead();
        assert_eq!(lead.shape, Shape::Bean);
        assert_eq!(lead.eyes, Eyes::Diamond);
        assert_eq!(lead.mouth, Mouth::None);
        assert!(lead.accessories.is_empty());
        assert_eq!(lead.colours().0, PALETTES[0].base);

        let a = AvatarSpec::from_name("Ada");
        let b = AvatarSpec::from_name("Ada");
        assert_eq!(a, b, "deterministic from name");
        assert_eq!(AvatarSpec::from_name("ada"), b, "case-insensitive");
        assert_ne!(AvatarSpec::from_name("Ada"), AvatarSpec::from_name("Grace"));

        // Every shape renders a gradient blob with eyes.
        for shape in Shape::all() {
            let mut spec = AvatarSpec::default_lead();
            spec.shape = *shape;
            let svg = spec.to_svg(96);
            assert!(svg.contains("radialGradient"), "{shape:?}");
            assert!(svg.contains("<path"), "{shape:?}");
        }
        // Every eye/mouth/accessory renders.
        for eyes in Eyes::all() {
            let mut spec = AvatarSpec::default_lead();
            spec.eyes = *eyes;
            assert!(!spec.to_svg(40).is_empty());
        }
        for mouth in Mouth::all() {
            let mut spec = AvatarSpec::default_lead();
            spec.mouth = *mouth;
            let svg = spec.to_svg(40);
            if *mouth == Mouth::None {
                assert!(!svg.contains("41 68") && !svg.contains("cy=\"70\""));
            } else {
                assert!(svg.len() > 100);
            }
        }
        // Custom colour overrides the preset.
        let mut spec = AvatarSpec::default_lead();
        spec.custom = Some(("#112233".into(), "#aabbcc".into()));
        let svg = spec.to_svg(40);
        assert!(svg.contains("#112233") && svg.contains("#aabbcc"));
    }

    #[test]
    fn avatar_render_sizes_and_json_roundtrip() {
        assert_eq!(AVATAR_SIZES, [16, 24, 40, 96, 160]);
        assert_eq!(AvatarSpec::detail(16), Detail::Minimal);
        assert_eq!(AvatarSpec::detail(24), Detail::Full);
        // Below 24 px: colour + eyes only (no mouth/accessories).
        let mut spec = AvatarSpec::default_lead();
        spec.mouth = Mouth::Smile;
        spec.accessories = vec![Accessory::Crown];
        let small = spec.to_svg(16);
        assert!(
            !small.contains("#d9a91e"),
            "no crown paint at 16px: {small}"
        );
        let big = spec.to_svg(96);
        assert!(big.contains("#d9a91e"), "crown renders at 96px");

        let spec = AvatarSpec::surprise_me_from(42);
        let back = AvatarSpec::from_json(&spec.to_json());
        assert_eq!(spec, back);
        assert!(AvatarSpec::from_json("garbage").is_valid());
        // Normalization clamps bad input instead of breaking.
        let bad = AvatarSpec::from_json(
            r#"{"shape":"bean","palette":99,"accessories":["crown","cap","beret"]}"#,
        );
        assert!(bad.is_valid());
        assert!(bad.accessories.len() <= 2);
    }
}

#[cfg(test)]
mod avatar_states_tests {
    use super::*;

    fn live(size: u32) -> AnimateCtx {
        AnimateCtx {
            size_px: size,
            ..AnimateCtx::default()
        }
    }

    #[test]
    fn avatar_states_motion_and_gating() {
        // Every documented state has a motion description.
        for state in AvatarState::all() {
            assert!(!state.motion().is_empty(), "{state:?}");
            assert!(AvatarState::parse(state.as_str()) == Some(*state));
        }
        assert_eq!(AvatarState::parse("streaming"), Some(AvatarState::Speaking));

        // Active states animate when visible.
        for state in [
            AvatarState::Thinking,
            AvatarState::Speaking,
            AvatarState::Working,
            AvatarState::NeedsYou,
            AvatarState::Error,
        ] {
            assert!(state.should_animate(&live(40)), "{state:?} animates");
        }
        // Paused never animates (dimmed, eyes closed).
        assert!(!AvatarState::Paused.should_animate(&live(40)));
        // Idle animates, then settles after 30 s to a still pose.
        let mut ctx = live(40);
        assert!(AvatarState::Idle.should_animate(&ctx));
        ctx.idle_secs = IDLE_SETTLE_SECS;
        assert!(
            !AvatarState::Idle.should_animate(&ctx),
            "idle settles after 30s"
        );
        // Animations stop when hidden / off-screen / reduce-motion / tiny.
        for ctx in [
            AnimateCtx {
                window_visible: false,
                ..live(40)
            },
            AnimateCtx {
                on_screen: false,
                ..live(40)
            },
            AnimateCtx {
                reduce_motion: true,
                ..live(40)
            },
            AnimateCtx {
                size_px: 16,
                ..live(40)
            },
        ] {
            assert!(!AvatarState::Thinking.should_animate(&ctx), "{ctx:?}");
            assert!(!AvatarState::Speaking.should_animate(&ctx), "{ctx:?}");
        }
        // Needs-you pulse stops after 10 s (badge stays).
        let mut ctx = live(40);
        ctx.state_secs = 11;
        assert!(!AvatarState::NeedsYou.should_animate(&ctx));
        ctx.state_secs = 2;
        assert!(AvatarState::NeedsYou.should_animate(&ctx));
    }

    #[test]
    fn avatar_states_editor_preview_and_surprise_me() {
        let mut editor = AvatarEditor::new();
        // Live preview renders large in the centre.
        assert_eq!(editor.preview_size, 160);
        assert!(editor.preview_svg().contains("<svg"));

        // Choose every option; preview updates.
        editor.set_shape(Shape::Cat);
        assert!(editor.preview_svg().contains("<svg"));
        assert!(editor.set_palette(3));
        assert!(!editor.set_palette(99));
        assert!(editor.set_custom("#112233", "#aabbcc"));
        assert!(!editor.set_custom("red", "blue"));
        editor.set_eyes(Eyes::Happy);
        editor.set_mouth(Mouth::Smile);
        assert_eq!(editor.apply().eyes, Eyes::Happy);

        // Accessories cap at 2; toggling removes.
        assert!(editor.toggle_accessory(Accessory::Glasses));
        assert!(editor.toggle_accessory(Accessory::Crown));
        assert!(
            !editor.toggle_accessory(Accessory::Cap),
            "third accessory rejected"
        );
        assert_eq!(editor.apply().accessories.len(), 2);
        assert!(editor.toggle_accessory(Accessory::Glasses));
        assert!(editor.toggle_accessory(Accessory::Cap));

        // Surprise-me shuffles with a deterministic sequence.
        let mut a = AvatarEditor::from_spec(AvatarSpec::default_lead());
        let mut b = AvatarEditor::from_spec(AvatarSpec::default_lead());
        let first_a = a.surprise_me().clone();
        let first_b = b.surprise_me().clone();
        assert_eq!(first_a, first_b, "same seed, same surprise");
        assert!(first_a.is_valid());
        assert!(first_a.accessories.len() <= 2);
        let second = a.surprise_me().clone();
        assert!(second.is_valid());
        // Surprise covers the documented option space over many seeds.
        let mut shapes = std::collections::HashSet::new();
        for seed in 0..60u64 {
            shapes.insert(AvatarSpec::surprise_me_from(seed).shape);
        }
        assert!(shapes.len() > 3, "surprise varies shapes");
    }
}
