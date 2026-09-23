pub mod accounts;
pub mod composer;
pub mod glass;
pub mod icons;
pub mod refract;
pub mod settings;
pub mod sidebar;
pub mod stats;
pub mod theme;
pub mod thread;

use egui::{Color32, Id, Rect, Response, RichText, Sense, Stroke, Ui, vec2};
use glass::Glass;
use theme::{R_SM, SP, T_CAPTION, T_SMALL, Theme};

/// Every non-ASCII symbol the views draw (checked against the fonts in a test).
#[cfg(test)]
pub const GLYPHS: &str = "●○✔◆✖■◇⊘·◑▼▶▲↻×⎘↵⌃•…—−⚙📁←↑↓→▣★›≈";

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

/// Secondary text (receipts, hints, meta). `text2` keeps it AA on every card;
/// `faint()` (tertiary) is only for timestamps and truncation notes.
pub fn dim(t: &Theme, s: impl Into<String>) -> RichText {
    RichText::new(s).color(t.text2).size(T_SMALL)
}

pub fn faint(t: &Theme, s: impl Into<String>) -> RichText {
    RichText::new(s).color(t.text3).size(T_CAPTION)
}

/// An icon glyph at a size, in a colour.
pub fn ic(icon: &str, size: f32, color: Color32) -> RichText {
    RichText::new(icon).size(size).color(color)
}

/// Frameless icon button with hover fill (toolbar, rows, banners).
pub fn icon_button(ui: &mut Ui, t: &Theme, icon: &str, tip: &str) -> Response {
    let size = vec2(26.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered() && ui.is_enabled();
    if hovered {
        ui.painter().rect_filled(rect, R_SM, t.text.gamma_multiply(0.08));
    }
    let color = if !ui.is_enabled() { t.text3 } else if hovered { t.text } else { t.text2 };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(15.0), color);
    // screen readers (and tests) get the tooltip as the button's name
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tip));
    resp.on_hover_text(tip)
}

/// The chip-style menu button (composer, pickers): a 24 px capsule sized to
/// its text with a chevron, whose list opens above (the composer sits at the
/// bottom, where a downward list was clipped). `keep_open` for editable lists.
pub fn chip_menu(ui: &mut Ui, t: &Theme, id_salt: impl std::hash::Hash, text: &str, color: Color32, keep_open: bool, add: impl FnOnce(&mut Ui)) -> Response {
    let font = egui::FontId::proportional(T_SMALL);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let size = vec2(galley.size().x + 2.0 * SP + 18.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, ui.is_enabled(), text));
    let popup_id = ui.id().with(id_salt);
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let fill = if resp.hovered() || open { t.raised_hi } else { t.raised.gamma_multiply(0.6) };
    ui.painter().rect_filled(rect, R_SM, fill);
    ui.painter().rect_stroke(rect, R_SM, Stroke::new(1.0_f32, t.separator), egui::StrokeKind::Inside);
    ui.painter().galley(egui::pos2(rect.min.x + 2.0 * SP, rect.center().y - galley.size().y / 2.0), galley, color);
    ui.painter().text(egui::pos2(rect.max.x - 10.0, rect.center().y), egui::Align2::CENTER_CENTER, icons::CHEVRON_DOWN, egui::FontId::proportional(11.0), t.text2);
    let mut popup = egui::Popup::menu(&resp)
        .id(popup_id)
        .align(egui::RectAlign::TOP_START)
        .align_alternatives(&[egui::RectAlign::TOP_START, egui::RectAlign::TOP_END]);
    if keep_open {
        popup = popup.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    }
    let max_h = if keep_open { (rect.top() - 24.0).clamp(320.0, 520.0) } else { 320.0 };
    popup.show(|ui| {
        ui.set_min_width(rect.width().max(160.0));
        egui::ScrollArea::vertical().max_height(max_h).show(ui, add);
    });
    resp
}

/// Segmented control: one rounded well, the selected segment accent-filled.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, t: &Theme, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    egui::Frame::new().fill(t.raised).corner_radius(R_SM).inner_margin(egui::Margin::same(2)).stroke(Stroke::new(1.0_f32, t.separator)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (k, label) in options {
                let on = *value == *k;
                let font = egui::FontId::proportional(T_SMALL);
                let galley = ui.painter().layout_no_wrap(label.to_string(), font, if on { t.on_accent } else { t.text });
                let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 3.0 * SP, 22.0), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, *label));
                if on {
                    ui.painter().rect_filled(rect, R_SM - 2, t.accent_solid);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, R_SM - 2, t.text.gamma_multiply(0.07));
                }
                ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.text);
                if resp.clicked() && !on {
                    *value = *k;
                    changed = true;
                }
            }
        });
    });
    changed
}

pub fn toggle_switch(ui: &mut Ui, t: &Theme, on: &mut bool) -> Response {
    let size = vec2(30.0, 17.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() && ui.is_enabled() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, ""));
    let k = ui.ctx().animate_bool_responsive(resp.id, *on);
    let enabled = ui.is_enabled();
    let track = if *on { t.accent_solid } else { t.text3.gamma_multiply(0.45) };
    let track = if enabled { track } else { track.gamma_multiply(0.5) };
    ui.painter().rect_filled(rect, 9, track);
    let x = egui::lerp(rect.left() + 8.5..=rect.right() - 8.5, k);
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), 6.5, if enabled { Color32::WHITE } else { t.text3 });
    resp
}

/// A quiet spinner: an arc that turns at 10 fps. egui's own spinner repaints
/// every frame while visible, which kept the GPU busy for the whole run.
pub fn spinner(ui: &mut Ui, size: f32, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let time = ui.input(|i| i.time);
    let start = (time * 4.0) as f32;
    let r = size * 0.38;
    let points: Vec<egui::Pos2> = (0..=18)
        .map(|i| {
            let a = start + i as f32 / 18.0 * std::f32::consts::PI * 1.4;
            rect.center() + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    ui.painter().add(egui::Shape::line(points, Stroke::new(1.6_f32, color)));
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    resp
}

/// Settings/popover group: flat raised panel, hairline, 12 px padding, no shadow.
pub fn group<R>(ui: &mut Ui, t: &Theme, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(t.raised)
        .corner_radius(R_SM)
        .stroke(Stroke::new(1.0_f32, t.separator))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A section caption above a group (11 pt semibold, secondary).
pub fn caption(ui: &mut Ui, t: &Theme, text: &str) {
    ui.add_space(SP);
    ui.label(RichText::new(text).size(T_CAPTION).family(theme::semibold()).color(t.text2));
}

/// A label-column row: fixed-width label, then the control.
pub fn row<R>(ui: &mut Ui, t: &Theme, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        // left-aligned label column (add_sized would centre it)
        ui.allocate_ui_with_layout(vec2(120.0, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(120.0);
            ui.add(egui::Label::new(RichText::new(label).size(T_SMALL).color(t.text2)).truncate());
        });
        add(ui)
    })
    .inner
}

/// A full-width disclosure header: chevron (animated) + label; returns the new open state.
pub fn disclosure(ui: &mut Ui, t: &Theme, id: egui::Id, open: bool, label: RichText) -> bool {
    let resp = ui.horizontal(|ui| {
        let k = ui.ctx().animate_bool_responsive(id, open);
        let chevron = if k > 0.5 { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT };
        ui.label(ic(chevron, 12.0, t.text2));
        ui.label(label);
    });
    let r = ui.interact(resp.response.rect, id.with("hit"), Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(resp.response.rect.expand(2.0), R_SM, t.text.gamma_multiply(0.05));
    }
    if r.clicked() { !open } else { open }
}

pub fn basename(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}
