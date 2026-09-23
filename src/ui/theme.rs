//! Design tokens: the ONLY place colours, radii, spacing and type sizes live
//! (PLAN §4; semantics from claudexor DESIGN_SYSTEM §2). Views read `Theme`,
//! never raw hex.

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle};
use super::icons;
use std::sync::Arc;

// Radius ladder (DESIGN_SYSTEM §2.4 / DesignTokens): control 8 · card 12 · bubble 16 · hero 22.
pub const R_SM: u8 = 8; // controls, chips, rows, code wells
pub const R_MD: u8 = 12; // cards, sidebar, popovers
pub const R_BUBBLE: u8 = 16; // the user's message bubble
pub const R_LG: u8 = 22; // floating composer
pub const SP: f32 = 4.0; // spacing scale unit: 4, 8, 12, 16, 24, 32
// Type scale: body 14 · callout 13 · caption 12 · caption2 11 · title 18.
pub const T_BODY: f32 = 14.0;
pub const T_CALLOUT: f32 = 13.0;
pub const T_SMALL: f32 = 12.0;
pub const T_CAPTION: f32 = 11.0;
pub const T_TITLE: f32 = 18.0;
pub const SIDEBAR_W: f32 = 260.0;
pub const MEASURE: f32 = 680.0;

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    /// Window backdrop (alpha < 255 lets the compositor's blur show through).
    pub base: Color32,
    pub glow_a: Color32,
    pub glow_b: Color32,
    pub raised: Color32,
    pub raised_hi: Color32,
    pub overlay: Color32,
    /// Solid, max-contrast surface for code, diffs, transcripts.
    pub code: Color32,
    pub separator: Color32,
    pub text: Color32,
    pub text2: Color32,
    pub text3: Color32,
    pub accent: Color32,
    pub accent_solid: Color32,
    pub on_accent: Color32,
    pub user_bubble: Color32,
    /// Glass film tints (alpha = film mix over the blurred backdrop).
    pub chrome_tint: [u8; 4],
    pub card_tint: [u8; 4],
    pub rim_top: Color32,
    pub rim_bottom: Color32,
    pub shadow: Color32,
    pub running: Color32,
    pub success: Color32,
    pub needs_you: Color32,
    pub blocked: Color32,
    pub failed: Color32,
    pub cancelled: Color32,
    pub queued: Color32,
}

const fn c(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}
const fn ca(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    Color32::from_rgba_premultiplied(
        ((r as u16 * a as u16) / 255) as u8,
        ((g as u16 * a as u16) / 255) as u8,
        ((b as u16 * a as u16) / 255) as u8,
        a,
    )
}

/// Signature deep-graphite dark ("command center"), never pure black.
pub const DARK: Theme = Theme {
    dark: true,
    base: ca(0x1A, 0x1B, 0x1E, 0xE0),
    glow_a: ca(0x4F, 0x74, 0xAD, 0x40),
    glow_b: ca(0x6B, 0x5B, 0x9E, 0x22),
    raised: c(0x24, 0x26, 0x2B),
    raised_hi: c(0x2C, 0x2F, 0x35),
    overlay: c(0x22, 0x24, 0x29),
    code: c(0x16, 0x17, 0x1A),
    separator: ca(0xFF, 0xFF, 0xFF, 0x18),
    text: c(0xE8, 0xEA, 0xEE),
    text2: c(0xA9, 0xAF, 0xBA),
    text3: c(0x80, 0x86, 0x91),
    accent: c(0x86, 0xA8, 0xD8),
    accent_solid: c(0x4A, 0x6F, 0xA8),
    on_accent: c(0xFF, 0xFF, 0xFF),
    user_bubble: c(0x38, 0x47, 0x6B),
    chrome_tint: [0x2A, 0x2D, 0x34, 0x9E],
    card_tint: [0x26, 0x28, 0x2E, 0xC4],
    rim_top: ca(0xFF, 0xFF, 0xFF, 0x38),
    rim_bottom: ca(0xFF, 0xFF, 0xFF, 0x0C),
    shadow: ca(0x00, 0x00, 0x00, 0x66),
    running: c(0x5A, 0xA9, 0xFF),
    success: c(0x4C, 0xC3, 0x8A),
    needs_you: c(0x9A, 0xA5, 0xFF),
    blocked: c(0xE5, 0xA9, 0x3B),
    failed: c(0xF0, 0x6A, 0x6A),
    cancelled: c(0x8A, 0x8F, 0x98),
    queued: c(0x80, 0x86, 0x91),
};

/// Warm off-white light theme.
pub const LIGHT: Theme = Theme {
    dark: false,
    base: ca(0xF4, 0xF2, 0xEE, 0xE6),
    glow_a: ca(0x7C, 0xA0, 0xDC, 0x3A),
    glow_b: ca(0xE8, 0xC9, 0xA0, 0x30),
    raised: c(0xFF, 0xFF, 0xFF),
    raised_hi: c(0xFB, 0xFA, 0xF7),
    overlay: c(0xFF, 0xFF, 0xFF),
    code: c(0xF7, 0xF7, 0xF5),
    separator: ca(0x00, 0x00, 0x00, 0x16),
    text: c(0x1B, 0x1D, 0x21),
    text2: c(0x4F, 0x55, 0x5F),
    text3: c(0x72, 0x78, 0x82),
    accent: c(0x3A, 0x63, 0xA0),
    accent_solid: c(0x3A, 0x63, 0xA0),
    on_accent: c(0xFF, 0xFF, 0xFF),
    user_bubble: c(0xCC, 0xDB, 0xFA),
    chrome_tint: [0xFF, 0xFF, 0xFF, 0x9A],
    card_tint: [0xFF, 0xFF, 0xFF, 0xC8],
    rim_top: ca(0xFF, 0xFF, 0xFF, 0xD0),
    rim_bottom: ca(0x00, 0x00, 0x00, 0x10),
    shadow: ca(0x00, 0x00, 0x00, 0x24),
    running: c(0x1F, 0x78, 0xD8),
    success: c(0x1E, 0x8E, 0x5A),
    needs_you: c(0x55, 0x5F, 0xD8),
    blocked: c(0xA8, 0x6A, 0x00),
    failed: c(0xC8, 0x3A, 0x3A),
    cancelled: c(0x6E, 0x73, 0x7C),
    queued: c(0x72, 0x78, 0x82),
};

impl Theme {
    pub fn for_dark(dark: bool) -> Theme {
        if dark { DARK } else { LIGHT }
    }

    /// Status colour + glyph + word for a run lifecycle/terminal state.
    pub fn status(&self, state: &str) -> (Color32, &'static str, &'static str) {
        match state {
            "running" => (self.running, icons::LOADER_CIRCLE, "Running"),
            "queued" => (self.queued, icons::CIRCLE_DASHED, "Queued"),
            "succeeded" | "completed" => (self.success, icons::CIRCLE_CHECK, "Done"),
            "blocked" => (self.blocked, icons::TRIANGLE_ALERT, "Needs decision"),
            "failed" => (self.failed, icons::CIRCLE_X, "Failed"),
            "cancelled" => (self.cancelled, icons::SQUARE, "Cancelled"),
            "interrupted" => (self.blocked, icons::CIRCLE_PAUSE, "Interrupted"),
            "needs_you" => (self.needs_you, icons::MESSAGE_SQUARE, "Needs your answer"),
            "refused" => (self.failed, icons::CIRCLE_X, "Refused"),
            _ => (self.text3, icons::CIRCLE, "Unknown"),
        }
    }

    /// Harness identity hue (only in harness UI, never chrome).
    pub fn harness(&self, id: &str) -> Color32 {
        let (d, l) = match id {
            "codex" => (c(0x3C, 0xC8, 0xB4), c(0x0E, 0x86, 0x78)),
            "claude" => (c(0xE8, 0x93, 0x5A), c(0xB0, 0x58, 0x1E)),
            "cursor" => (c(0xB0, 0x8A, 0xF5), c(0x74, 0x4A, 0xC8)),
            "opencode" => (c(0xA6, 0xD0, 0x4E), c(0x5A, 0x82, 0x10)),
            "agy" => (c(0x5A, 0xAE, 0xF2), c(0x1C, 0x6E, 0xB8)),
            "raw-api" => (c(0xD8, 0x6C, 0xC0), c(0xA0, 0x30, 0x88)),
            _ => (self.text2, self.text2),
        };
        if self.dark { d } else { l }
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let mut v = if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.panel_fill = Color32::TRANSPARENT;
        v.window_fill = self.overlay;
        v.extreme_bg_color = self.code;
        // text fields sit on the raised surface, never egui's black well
        v.text_edit_bg_color = Some(self.raised);
        // a blinking caret repaints twice a second forever: idle must be 0 frames
        v.text_cursor.blink = false;
        v.code_bg_color = self.code;
        v.faint_bg_color = self.raised_hi;
        v.override_text_color = Some(self.text);
        v.hyperlink_color = self.accent;
        v.selection.bg_fill = self.accent_solid.gamma_multiply(0.55);
        v.selection.stroke.color = self.accent;
        v.window_corner_radius = R_MD.into();
        v.menu_corner_radius = R_SM.into();
        v.window_stroke.color = self.separator;
        v.popup_shadow.color = self.shadow;
        v.window_shadow.color = self.shadow;
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = R_SM.into();
        }
        v.widgets.inactive.weak_bg_fill = self.raised_hi;
        v.widgets.inactive.bg_fill = self.raised_hi;
        v.widgets.inactive.bg_stroke.color = self.separator;
        v.widgets.inactive.fg_stroke.color = self.text;
        v.widgets.hovered.weak_bg_fill = self.raised_hi.gamma_multiply(1.15);
        v.widgets.hovered.bg_stroke.color = self.accent.gamma_multiply(0.6);
        v.widgets.active.weak_bg_fill = self.accent_solid.gamma_multiply(0.4);
        v.widgets.noninteractive.bg_stroke.color = self.separator;
        v.widgets.noninteractive.fg_stroke.color = self.text2;
        ctx.set_visuals(v);

        ctx.global_style_mut(|s| {
            // tight defaults; layouts add explicit gaps from the 4 px scale
            s.spacing.item_spacing = egui::vec2(2.0 * SP, SP);
            s.spacing.button_padding = egui::vec2(2.0 * SP, SP);
            s.spacing.interact_size.y = 24.0;
            s.spacing.scroll.floating = true;
            s.spacing.scroll.bar_width = 6.0;
            s.text_styles = [
                (TextStyle::Small, FontId::proportional(T_CAPTION)),
                (TextStyle::Body, FontId::proportional(T_BODY)),
                (TextStyle::Button, FontId::proportional(T_CALLOUT)),
                (TextStyle::Heading, FontId::new(T_TITLE, semibold())),
                (TextStyle::Monospace, FontId::monospace(T_SMALL - 0.5)),
            ]
            .into();
            // Motion: 150–200 ms ease-out on hover/expand/open (PLAN §4).
            s.animation_time = 0.18;
        });
    }
}

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Inter for UI, JetBrains Mono for code — bundled, so every desktop matches.
pub fn install_fonts(ctx: &egui::Context) {
    let mut f = FontDefinitions::default();
    f.font_data.insert("inter".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Inter-Regular.ttf"))));
    f.font_data.insert("inter-sb".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"))));
    f.font_data.insert("jbm".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf"))));
    f.font_data.insert("icons".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Lucide-subset.ttf"))));
    f.families.entry(FontFamily::Proportional).or_default().insert(0, "inter".into());
    // icons FIRST: Inter maps its own alternates into the private-use range, so
    // after Inter it would shadow the icons; the icon font has no Latin glyphs.
    f.families.get_mut(&FontFamily::Proportional).expect("proportional family").insert(0, "icons".into());
    f.families.entry(FontFamily::Monospace).or_default().insert(0, "jbm".into());
    // Fallbacks (egui's default fonts carry emoji + symbols) stay after ours.
    // semibold: icons, Inter SemiBold, then the regular chain (Inter + fallbacks)
    let mut sb = vec!["icons".to_string(), "inter-sb".to_string()];
    sb.extend(f.families[&FontFamily::Proportional].iter().skip(1).cloned());
    f.families.insert(semibold(), sb);
    ctx.set_fonts(f);
}

// ---- time helpers (ISO-8601 → relative words), no chrono dependency ----------

/// Parse `YYYY-MM-DDTHH:MM:SS(.fff)?(Z|±HH:MM)` to unix seconds.
pub fn parse_iso(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    let mut rest = &s[19..];
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest.as_bytes().first() {
        Some(b'+') | Some(b'-') if rest.len() >= 6 => {
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            sign * (rest[1..3].parse::<i64>().ok()? * 3600 + rest[4..6].parse::<i64>().ok()? * 60)
        }
        _ => 0,
    };
    // days_from_civil (Howard Hinnant)
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se - offset)
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn span(secs: i64) -> String {
    let s = secs.abs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h {}m", s / 3600, (s % 3600) / 60),
        _ => format!("{}d", s / 86_400),
    }
}

pub fn ago(iso: &str) -> String {
    match parse_iso(iso) {
        Some(t) if now_unix() - t < 45 => "now".into(),
        Some(t) => format!("{} ago", span(now_unix() - t)),
        None => String::new(),
    }
}

pub fn until(iso: &str) -> String {
    match parse_iso(iso) {
        Some(t) if t > now_unix() => format!("in {}", span(t - now_unix())),
        Some(_) => "now".into(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_parsing() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("2026-07-19T12:00:00.000Z"), Some(1_784_462_400));
        assert_eq!(parse_iso("2026-07-19T14:00:00+02:00"), Some(1_784_462_400));
        assert_eq!(parse_iso("nope"), None);
    }

    #[test]
    fn spans() {
        assert_eq!(span(5), "5s");
        assert_eq!(span(125), "2m");
        assert_eq!(span(3 * 3600 + 120), "3h 2m");
        assert_eq!(span(3 * 86_400), "3d");
    }

    /// Every symbol the UI draws must exist in the bundled fonts (or egui's
    /// fallbacks), otherwise it renders as tofu.
    #[test]
    fn ui_glyphs_are_in_the_bundled_fonts() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let _ = ctx.run_ui(Default::default(), |_| {});
        let missing: Vec<char> =
            crate::ui::GLYPHS.chars().chain(crate::ui::icons::ALL.iter().flat_map(|s| s.chars())).filter(|c| !ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(T_SMALL), *c))).collect();
        assert!(missing.is_empty(), "missing glyphs: {missing:?}");
    }

    /// WCAG AA 4.5:1 for body text on every solid and glass-backed surface.
    #[test]
    fn text_contrast_is_aa() {
        fn lum(c: Color32) -> f64 {
            let f = |v: u8| {
                let v = v as f64 / 255.0;
                if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
        }
        fn ratio(a: Color32, b: Color32) -> f64 {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        }
        for t in [DARK, LIGHT] {
            let film = Color32::from_rgb(t.chrome_tint[0], t.chrome_tint[1], t.chrome_tint[2]);
            for (name, bg) in [("raised", t.raised), ("raised_hi", t.raised_hi), ("code", t.code), ("user", t.user_bubble), ("film", film)]
            {
                assert!(ratio(t.text, bg) >= 4.5, "{} text on {name}: {:.2}", if t.dark { "dark" } else { "light" }, ratio(t.text, bg));
            }
            // secondary text carries receipts, hints and meta lines: AA on every card surface
            for (name, bg) in [("raised", t.raised), ("raised_hi", t.raised_hi), ("code", t.code)] {
                assert!(ratio(t.text2, bg) >= 4.5, "secondary text on {name}: {:.2}", ratio(t.text2, bg));
            }
            // tertiary is for timestamps and truncation notes only: AA-large
            assert!(ratio(t.text3, t.raised_hi) >= 3.0, "tertiary text");
            assert!(ratio(t.on_accent, t.accent_solid) >= 4.5, "send button");
        }
    }
}
