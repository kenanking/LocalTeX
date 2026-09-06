use gpui::{Rgba, rgb, rgba};

pub const BG: u32 = 0xf6f5f6;
pub const BG_RAISED: u32 = 0xffffff;
pub const BG_SUNKEN: u32 = 0xefefef;
pub const TEXT: u32 = 0x1a1a1a;
/// Preview document ink (body, captions, formulas). Chrome stays `TEXT`/`MUTED`.
pub const INK: u32 = 0x000000;
pub const MUTED: u32 = 0x6b6b6b;
pub const ACCENT: u32 = 0x2563eb;
pub const DANGER: u32 = 0xdc2626;
pub const OK: u32 = 0x16a34a;
pub const WARN: u32 = 0xd97706;
pub const BORDER: u32 = 0xe6e6e6;
pub const PAPER: u32 = 0xfbfaf7;
pub const PAPER_DOT: u32 = 0xc8c4bc;
pub const PAPER_RULE: u32 = 0xc9d3e4;
pub const ACCENT_SOFT_FILL: u32 = 0xeff6ff;
pub const ACCENT_BORDER: u32 = 0xbfdbfe;
pub const ON_ACCENT: u32 = 0xffffff;
/// Off-state track for toggle switches.
pub const TRACK_OFF: u32 = 0xd5d5da;
/// Neutral segment in composition bars (the part that is not ours).
pub const SEG_NEUTRAL: u32 = 0x94a3b8;

pub fn accent_soft() -> Rgba {
    rgb(ACCENT_SOFT_FILL)
}

pub fn danger_soft() -> Rgba {
    rgb(0xfef2f2)
}

pub fn row_hover() -> Rgba {
    let mut c = rgb(0x000000);
    c.a = 0.04;
    c
}

pub fn scrollbar_thumb() -> Rgba {
    rgb(0x9ca3af)
}

pub fn scrollbar_thumb_subtle() -> Rgba {
    rgb(0xd4d4d8)
}

/// Translucent gallery scrim (~0.38). gpui 0.2 has no backdrop-filter.
pub fn overlay_scrim() -> Rgba {
    rgba(0x16161c61)
}

/// Light frost veil for file intake. gpui 0.2 has no backdrop-filter.
pub fn intake_scrim() -> Rgba {
    rgba(0xf6f5f6c9)
}

pub fn hud_pill() -> Rgba {
    rgba(0x1212169e)
}

pub fn hud_pill_hover() -> Rgba {
    rgba(0x121216d1)
}

pub fn nav_disc() -> Rgba {
    rgba(0xffffffe6)
}

pub fn film_selected_ring() -> Rgba {
    rgba(0xffffffe6)
}

pub fn film_scroll_thumb() -> Rgba {
    rgba(0xffffff66)
}

pub fn status_color(kind: StatusKind) -> Rgba {
    match kind {
        StatusKind::Idle => rgb(0x9ca3af),
        StatusKind::Ready => rgb(OK),
        StatusKind::Busy => rgb(WARN),
        StatusKind::Error => rgb(DANGER),
    }
}

#[derive(Clone, Copy)]
pub enum StatusKind {
    Idle,
    Ready,
    Busy,
    Error,
}

#[cfg(target_os = "windows")]
pub const SOURCE_FONT: &str = "Consolas";
#[cfg(not(target_os = "windows"))]
pub const SOURCE_FONT: &str = "monospace";
