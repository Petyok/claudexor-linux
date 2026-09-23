//! Floating composer (Liquid Glass chrome, solid contents — no glass-on-glass):
//! a controls row (mode · project · harness · model · effort) over a growing
//! input with Send, which swaps to Stop while the thread's head turn runs.
//! Enter sends, Shift+Enter inserts a newline.

use super::glass::Kind;
use super::theme::{R_LG, R_SM, SP, T_BODY, T_SMALL};
use super::{View, basename, dim, last_rect, store_rect};
use crate::state::{MODES, State};
use egui::{Align, ComboBox, Frame, Id, Key, Layout, Margin, Modifiers, Rect, RichText, Stroke, TextEdit, Ui, UiBuilder, pos2, vec2};

/// Draw the composer anchored to the bottom of `area`; returns its height.
pub fn show(ui: &mut Ui, v: &mut View, s: &mut State, area: Rect) -> f32 {
    let t = v.t;
    let w = (area.width() - 8.0 * SP).min(super::theme::MEASURE + 40.0);
    let id = Id::new("composer");
    let h = last_rect(ui, id).map_or(118.0, |r| r.height());
    let rect = Rect::from_min_size(pos2(area.center().x - w / 2.0, area.max.y - h - 4.0 * SP), vec2(w, h));
    v.glass.surface(ui, rect, R_LG, Kind::Chrome);

    let mut out = rect;
    ui.scope_builder(UiBuilder::new().max_rect(Rect::from_min_size(rect.min, vec2(w, 400.0))), |ui| {
        let r = Frame::new().inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(w - 32.0);
            controls(ui, v, s, w - 32.0);
            ui.add_space(2.0 * SP);
            input(ui, v, s, w - 32.0);
        });
        out = r.response.rect;
    });
    store_rect(ui, id, out);
    let _ = t;
    out.height()
}

fn controls(ui: &mut Ui, v: &View, s: &mut State, inner_w: f32) {
    // One row when wide, two when narrow: egui grows a parent past its max
    // rect when a row overflows, which would push Send off the glass.
    let wide = inner_w >= 640.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0 * SP;
        mode_and_project(ui, v, s);
        if wide {
            routing(ui, v, s);
        }
    });
    if !wide {
        ui.add_space(SP);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0 * SP;
            routing(ui, v, s);
        });
    }
}

fn mode_and_project(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    {
        let has_project = s.effective_root().is_some();
        // mode segment: Ask / Plan / Agent (Plan/Agent need a project)
        Frame::new().fill(t.raised).corner_radius(R_SM).inner_margin(Margin::same(2)).stroke(Stroke::new(1.0_f32, t.separator)).show(
            ui,
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, m) in MODES.iter().enumerate() {
                    let label = match *m {
                        "ask" => "Ask",
                        "plan" => "Plan",
                        _ => "Agent",
                    };
                    let on = s.composer.mode == i;
                    let enabled = has_project || *m == "ask";
                    let text = RichText::new(label).size(T_SMALL).color(if on {
                        t.on_accent
                    } else if enabled {
                        t.text
                    } else {
                        t.text3
                    });
                    let b = egui::Button::new(text)
                        .fill(if on { t.accent_solid } else { egui::Color32::TRANSPARENT })
                        .corner_radius(R_SM - 3)
                        .min_size(vec2(52.0, 24.0));
                    let resp = ui.add_enabled(enabled, b);
                    let resp = match *m {
                        "ask" => resp.on_hover_text("Read-only answer"),
                        "plan" => resp.on_hover_text("Read-only planning report with open questions"),
                        _ => resp.on_hover_text("Writes code in the project"),
                    };
                    if resp.on_disabled_hover_text("Pick a project to use Plan and Agent").clicked() {
                        s.composer.mode = i;
                    }
                }
            },
        );
        if !has_project && s.composer.mode != 0 {
            s.composer.mode = 0;
        }

        // project: selectable for a draft thread, bound for an existing one
        if s.selected.is_none() {
            let cur = s.composer.project.clone();
            let label = cur.as_deref().map(basename).unwrap_or("No project");
            ComboBox::from_id_salt("project").selected_text(RichText::new(format!("📁 {label}")).size(T_SMALL)).width(140.0).show_ui(
                ui,
                |ui| {
                    ui.selectable_value(&mut s.composer.project, None, "No project (Ask only)");
                    for p in &s.projects {
                        ui.selectable_value(&mut s.composer.project, Some(p.root.clone()), &p.root);
                    }
                    ui.separator();
                    let key = Id::new("project-path");
                    let mut path: String = ui.ctx().data(|d| d.get_temp(key)).unwrap_or_default();
                    let r = ui.add(TextEdit::singleline(&mut path).hint_text("/absolute/path/to/project").desired_width(260.0));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && path.starts_with('/') {
                        s.composer.project = Some(path.trim_end_matches('/').to_string());
                        path.clear();
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(key, path));
                },
            );
        } else if let Some(root) = s.effective_root() {
            ui.label(dim(&t, format!("📁 {}", basename(&root)))).on_hover_text(root);
        } else {
            ui.label(dim(&t, "no project · Ask only"));
        }
    }
}

fn routing(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    {
        let hl = s.composer.harness.clone().unwrap_or_else(|| "Auto".into());
        let before = s.composer.harness.clone();
        ComboBox::from_id_salt("harness")
            .selected_text(RichText::new(&hl).size(T_SMALL).color(s.composer.harness.as_deref().map_or(t.text, |h| t.harness(h))))
            .width(100.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut s.composer.harness, None, "Auto").on_hover_text("Let the engine route by quota and readiness");
                for h in &s.harnesses {
                    let ok = h.status != "unavailable";
                    let text = RichText::new(h.label()).color(if ok { t.harness(&h.id) } else { t.text3 });
                    let resp = ui.add_enabled(ok, egui::Button::selectable(s.composer.harness.as_deref() == Some(&h.id), text));
                    let resp = if h.reasons.is_empty() { resp } else { resp.on_hover_text(h.reasons.join("\n")) };
                    if resp.on_disabled_hover_text(format!("{}: unavailable", h.id)).clicked() {
                        s.composer.harness = Some(h.id.clone());
                    }
                }
            });
        if s.composer.harness != before {
            s.composer.model = None;
            s.composer.effort = None;
        }

        // model + effort for an explicit harness (no free-text model ids)
        if let Some(h) = s.composer.harness.clone() {
            s.load_models(&h);
            let models = s.models.get(&h).cloned();
            let ml = s.composer.model.clone().unwrap_or_else(|| "Default model".into());
            ComboBox::from_id_salt(("model", &h)).selected_text(RichText::new(&ml).size(T_SMALL)).width(140.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut s.composer.model, None, "Default model");
                match &models {
                    Some(Ok(list)) if !list.models.is_empty() => {
                        for m in &list.models {
                            ui.selectable_value(&mut s.composer.model, Some(m.id.clone()), m.label.as_deref().unwrap_or(&m.id));
                        }
                    }
                    Some(Ok(_)) => {
                        ui.label(dim(&t, "Harness default only"));
                    }
                    Some(Err(e)) => {
                        ui.label(dim(&t, e));
                    }
                    None => {}
                }
            });
            let levels = s.harnesses.iter().find(|x| x.id == h).map(|x| x.effort_levels(s.composer.model.as_deref())).unwrap_or_default();
            if !levels.is_empty() {
                let el = s.composer.effort.clone().unwrap_or_else(|| "Default effort".into());
                ComboBox::from_id_salt(("effort", &h)).selected_text(RichText::new(&el).size(T_SMALL)).width(110.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.composer.effort, None, "Default effort");
                    for l in &levels {
                        ui.selectable_value(&mut s.composer.effort, Some(l.clone()), l);
                    }
                });
                if s.composer.effort.as_ref().is_some_and(|e| !levels.contains(e)) {
                    s.composer.effort = None;
                }
            }
        }
    }
}

fn input(ui: &mut Ui, v: &View, s: &mut State, inner_w: f32) {
    let t = v.t;
    let running = s.head_live_id();
    ui.horizontal(|ui| {
        let edit_id = Id::new("composer-input");
        let focused = ui.memory(|m| m.has_focus(edit_id));
        // Enter sends; Shift+Enter falls through to the editor as a newline.
        let send_key = focused && running.is_none() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        let btn_w = 84.0;
        let lines = s.composer.text.lines().count().clamp(1, 6);
        Frame::new()
            .fill(t.raised)
            .corner_radius(R_SM + 2)
            .inner_margin(Margin::symmetric(12, 8))
            .stroke(Stroke::new(1.0_f32, if focused { t.accent.gamma_multiply(0.8) } else { t.separator }))
            .show(ui, |ui| {
                let hint = if s.client.is_none() {
                    "Engine offline"
                } else if s.selected.is_none() {
                    "Message… (the first message starts a thread)"
                } else {
                    "Reply…"
                };
                ui.add(
                    TextEdit::multiline(&mut s.composer.text)
                        .id(edit_id)
                        .hint_text(RichText::new(hint).color(t.text3))
                        .desired_rows(lines)
                        .desired_width(inner_w - btn_w - 2.0 * SP - 26.0)
                        .frame(Frame::NONE)
                        .font(egui::FontId::proportional(T_BODY)),
                );
            });
        ui.with_layout(Layout::bottom_up(Align::Max), |ui| {
            if let Some(id) = &running {
                let b = egui::Button::new(RichText::new("■ Stop").color(t.on_accent))
                    .fill(t.failed)
                    .corner_radius(R_SM)
                    .min_size(vec2(btn_w, 34.0));
                if ui.add_enabled(s.client.is_some(), b).on_hover_text(format!("Cancel the running turn ({id})")).clicked() {
                    s.cancel_head();
                }
            } else {
                let label = if s.composer.sending { "Sending…" } else { "Send ↵" };
                let b = egui::Button::new(RichText::new(label).color(t.on_accent))
                    .fill(t.accent_solid)
                    .corner_radius(R_SM)
                    .min_size(vec2(btn_w, 34.0));
                let why = if s.client.is_none() {
                    "Engine offline"
                } else if s.composer.text.trim().is_empty() {
                    "Type a message first"
                } else {
                    "Sending…"
                };
                if ui.add_enabled(s.can_send(), b).on_disabled_hover_text(why).clicked() || (send_key && s.can_send()) {
                    s.submit();
                }
            }
        });
        if send_key && !s.can_send() {
            // keep focus; nothing to send
        }
    });
    if s.effective_root().is_none() {
        ui.label(dim(&t, "Pick a project to use Plan and Agent."));
    }
}
