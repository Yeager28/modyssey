//! Design tokens — PLAN.md §6.1 (Modrinth-inspired, dark-first).
//!
//! Single source of truth for colors, radii, spacing and type sizes. All
//! screens and components style from here; no literal colors in screens.

/// App canvas / nav rail background.
pub const BG_APP: u32 = 0x16181d;
/// Content-region background.
pub const BG_SURFACE: u32 = 0x1a1d22;
/// Cards, list rows, secondary buttons.
pub const BG_CARD: u32 = 0x212429;
/// Card hover state.
pub const BG_CARD_HOVER: u32 = 0x292c32;
/// Inputs, search, pressed-in wells.
pub const BG_INSET: u32 = 0x0f1115;
/// Primary text.
pub const TEXT_PRIMARY: u32 = 0xe8ebee;
/// Secondary/meta text.
pub const TEXT_SECONDARY: u32 = 0x9aa1a9;
/// Brand accent (Modrinth-derived green; PLAN.md §3.4 — color token only,
/// never their assets or wordmarks).
pub const ACCENT: u32 = 0x1bd96a;
/// Text on accent surfaces.
pub const ACCENT_FG: u32 = 0x0d1117;
/// Danger (hash mismatches, failures).
pub const DANGER: u32 = 0xf0564f;
/// Warning (preflight warnings).
pub const WARNING: u32 = 0xffb340;
/// Hairline separators / card borders: white at ~6% alpha (`rgba()` bytes
/// are R-G-B-A; 0x0F alpha ≈ 5.9%).
pub const HAIRLINE: u32 = 0xFFFFFF0F;

/// Type scale (§6.1): display / h1 / h2 / body / label / caption / mono.
pub mod text {
    /// Screen titles — 24/700.
    pub const DISPLAY: f32 = 24.0;
    /// Card headers, wizard steps — 20/700.
    pub const H1: f32 = 20.0;
    /// Section headings, card titles — 16/600.
    pub const H2: f32 = 16.0;
    /// Body copy — 14.
    pub const BODY: f32 = 14.0;
    /// Buttons, table headers — 14/600.
    pub const LABEL: f32 = 14.0;
    /// Meta lines, timestamps — 12.
    pub const CAPTION: f32 = 12.0;
    /// Paths, hashes, log lines — 13.
    pub const MONO: f32 = 13.0;
}

/// The system UI font family (DirectWrite resolves it; §6.1 allows Inter or
/// a metric-compatible grotesque — Segoe UI is the Windows-native pick and
/// needs no bundling, per the GPUI spike report finding #5).
pub const FONT_FAMILY: &str = "Segoe UI";

/// Monospace family for paths/hashes/log lines.
pub const FONT_MONO: &str = "Consolas";
