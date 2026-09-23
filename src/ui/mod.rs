pub mod accounts;
pub mod composer;
pub mod glass;
pub mod refract;
pub mod sidebar;
pub mod stats;
pub mod theme;
pub mod thread;

use egui::{Color32, Id, Rect, Response, RichText, Sense, Stroke, Ui, vec2};
use glass::Glass;
use theme::{R_SM, SP, T_SMALL, Theme};

/// Every non-ASCII symbol the views draw (checked against the fonts in a test).
#[cfg(test)]
pub const GLYPHS: &str = "●○✔◆✖■◇⊘·◑▼▶▲↻×⎘↵⌃•…—−⚙📁←↑↓→";

/// Everything a view needs besides the app state.
pub struct View<'a> {
    pub glass: &'a Glass,
    pub md: &'a mut egui_commonmark::CommonMarkCache,
    pub t: Theme,
}

/// Frost contract #3 (backdrop-blur-egui): a surface's rect is only known after
/// its content lays out, but the frost must be enqueued first — so frost LAST
/// frame's rect and store this frame's for the next one.
pub fn last_rect(ui: &Ui, id: Id) -> Option<Rect> {
    ui.ctx().data(|d| d.get_temp::<Rect>(id))
}

pub fn store_rect(ui: &Ui, id: Id, r: Rect) {
    let prev = last_rect(ui, id);
    ui.ctx().data_mut(|d| d.insert_temp(id, r));
    // A size change means last frame's frost was one frame stale: paint once more.
    if prev.is_some_and(|p| (p.size() - r.size()).length() > 0.5 || (p.min - r.min).length() > 0.5) {
        ui.ctx().request_repaint();
    }
}

/// A small fixed-height capsule (chips never wrap: DESIGN_SYSTEM §1.1 rule 5).
pub fn chip(ui: &mut Ui, t: &Theme, text: &str, color: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(T_SMALL - 1.0), color);
    let size = vec2(galley.size().x + 3.0 * SP, 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, R_SM, color.gamma_multiply(0.14));
    ui.painter().rect_stroke(rect, R_SM, Stroke::new(1.0_f32, color.gamma_multiply(0.35)), egui::StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    let _ = t;
    resp
}

pub fn dim(t: &Theme, s: impl Into<String>) -> RichText {
    RichText::new(s).color(t.text3).size(T_SMALL)
}

pub fn basename(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}
