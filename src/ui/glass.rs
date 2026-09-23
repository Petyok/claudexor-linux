//! Glass surfaces (PLAN §5), three layers each with a fallback:
//! - L0: the window is transparent; `backdrop()` paints a translucent base so
//!   a blurring compositor (e.g. Hyprland `decoration:blur`) shows
//!   the desktop through it. Without compositor blur it still reads as a
//!   graphite/off-white gradient.
//! - L1: `backdrop-blur-egui` frosts the live framebuffer behind popovers
//!   (grab-pass on glow; cards are film-only). Missing/failed renderer → solid surfaces.
//! - L2: `refract` (below) bends the backdrop at the rim of chrome surfaces.
//!
//! Hard rule: never glass behind code, diffs or dense text — callers put those
//! on `Theme::code` solids. "Reduce transparency" turns every surface solid.

use super::refract::Refractor;
use super::theme::Theme;
use backdrop_blur_egui::{BlurRadius, CornerRadius as BlurCorner, GrabPassRenderer, Presence, RepaintPolicy, Surface, Tint};
use egui::epaint::{Mesh, Shadow};
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Stroke, Ui, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Liquid Glass chrome: composer, status pill, sidebar header (L2 where available).
    Chrome,
    /// Content card / sidebar panel: a translucent film. Behind it in OUR
    /// framebuffer is only the static backdrop gradient, so a grab-pass blur
    /// changes nothing visible yet cost ~0.7 ms per 100k px on the iGPU
    /// (measured with CXL_FRAMESTATS). The film lets L0 (the compositor's
    /// real desktop blur) show through instead.
    Card,
    /// Popover floating over live content: real L1 frost.
    Popover,
}

pub struct Glass {
    pub frost: Option<GrabPassRenderer>,
    pub refract: Option<Refractor>,
    pub reduce_transparency: bool,
    pub theme: Theme,
}

impl Glass {
    pub fn blur_available(&self) -> bool {
        self.frost.is_some() && !self.reduce_transparency
    }

    /// L0 window backdrop: translucent base + two static radial glows.
    pub fn backdrop(&self, painter: &Painter, rect: Rect) {
        let t = &self.theme;
        let base = if self.reduce_transparency { t.base.to_opaque() } else { t.base };
        painter.rect_filled(rect, 0.0, base);
        if self.reduce_transparency {
            return;
        }
        let r = rect.width().max(rect.height());
        radial(painter, rect.left_top() + vec2(rect.width() * 0.18, rect.height() * 0.05), r * 0.55, t.glow_a);
        radial(painter, rect.right_bottom() - vec2(rect.width() * 0.12, rect.height() * 0.1), r * 0.45, t.glow_b);
    }

    /// Paint a glass surface's BACKGROUND. Call before painting its content
    /// (the frost grabs whatever is already in the framebuffer at `rect`).
    pub fn surface(&self, ui: &Ui, rect: Rect, radius: u8, kind: Kind) {
        let t = &self.theme;
        let painter = ui.painter();
        let (blur, film) = match kind {
            Kind::Chrome => (22.0, t.chrome_tint),
            Kind::Card | Kind::Popover => (18.0, t.card_tint),
        };
        let shadow = Shadow {
            offset: [0, if kind == Kind::Chrome { 8 } else { 4 }],
            blur: if kind == Kind::Chrome { 28 } else { 16 },
            spread: 0,
            color: t.shadow,
        };
        painter.add(shadow.as_shape(rect, radius));

        match (&self.frost, &self.refract, self.reduce_transparency) {
            // L2: the refraction shader is the whole chrome material (its own light
            // blur, tint, Fresnel rim, specular). Frosting on top would blur the bend away.
            (_, Some(refract), false) if kind == Kind::Chrome => refract.paint(ui, rect, radius as f32, t.dark),
            (_, _, false) if kind == Kind::Card => {
                let [r, g, b, a] = film;
                painter.rect_filled(rect, radius, Color32::from_rgba_unmultiplied(r, g, b, a));
            }
            (Some(frost), _, false) => {
                frost.frost(
                    ui,
                    Surface {
                        rect,
                        blur_radius: BlurRadius::new(blur),
                        tint: Tint::from_srgb_unmultiplied(film),
                        corner_radius: BlurCorner::new(radius as f32),
                        presence: Presence::FULL,
                        repaint: RepaintPolicy::Static,
                    },
                );
            }
            (None, _, false) => {
                // No blur backend: translucent film only (still lets L0 show through).
                let [r, g, b, a] = film;
                painter.rect_filled(rect, radius, Color32::from_rgba_unmultiplied(r, g, b, a.saturating_add(40)));
            }
            (_, _, true) => {
                painter.rect_filled(rect, radius, if kind == Kind::Chrome { t.overlay } else { t.raised });
            }
        }
        self.rim(painter, rect, radius);
    }

    /// Top-lit hairline (light falls from above) + a thin specular line.
    fn rim(&self, painter: &Painter, rect: Rect, radius: u8) {
        let t = &self.theme;
        let cr = CornerRadius::same(radius);
        painter.rect_stroke(rect, cr, Stroke::new(1.0_f32, t.rim_bottom), egui::StrokeKind::Inside);
        let top = Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + rect.height().min(radius as f32 * 2.0 + 8.0)));
        painter.with_clip_rect(top.intersect(painter.clip_rect())).rect_stroke(
            rect,
            cr,
            Stroke::new(1.0_f32, t.rim_top),
            egui::StrokeKind::Inside,
        );
        if self.reduce_transparency {
            return;
        }
        // Specular: brightest at the centre, fading to nothing before the corners.
        let y = rect.min.y + 1.5;
        let (x0, x1) = (rect.min.x + radius as f32, rect.max.x - radius as f32);
        if x1 - x0 > 8.0 {
            let mid = (x0 + x1) * 0.5;
            let mut m = Mesh::default();
            let hi = t.rim_top.gamma_multiply(if t.dark { 1.6 } else { 1.0 });
            for (x, col) in [(x0, Color32::TRANSPARENT), (mid, hi), (x1, Color32::TRANSPARENT)] {
                m.colored_vertex(pos2(x, y - 0.6), col);
                m.colored_vertex(pos2(x, y + 0.6), col);
            }
            for i in 0..2u32 {
                let b = i * 2;
                m.add_triangle(b, b + 1, b + 2);
                m.add_triangle(b + 1, b + 3, b + 2);
            }
            painter.add(m);
        }
    }
}

/// Radial gradient as a triangle fan: `color` at the centre → transparent rim.
fn radial(painter: &Painter, center: Pos2, radius: f32, color: Color32) {
    const N: u32 = 64;
    let mut m = Mesh::default();
    m.colored_vertex(center, color);
    // two rings give a softer, closer-to-gaussian falloff than one linear ramp
    let mid = color.gamma_multiply(0.35);
    for ring in [(0.5, mid), (1.0, Color32::TRANSPARENT)] {
        for i in 0..N {
            let a = i as f32 / N as f32 * std::f32::consts::TAU;
            m.colored_vertex(center + vec2(a.cos(), a.sin()) * radius * ring.0, ring.1);
        }
    }
    for i in 0..N {
        let (a, b) = (1 + i, 1 + (i + 1) % N);
        m.add_triangle(0, a, b);
        let (c, d) = (a + N, b + N);
        m.add_triangle(a, c, d);
        m.add_triangle(a, d, b);
    }
    painter.add(m);
}
