//! Floating composer (Liquid Glass chrome, solid contents — no glass-on-glass):
//! a controls row (mode · project · harness · model · effort) over a growing
//! input with Send, which swaps to Stop while the thread's head turn runs.
//! Enter sends, Shift+Enter inserts a newline.

use super::glass::Kind;
use super::theme::{R_LG, R_SM, SP, T_BODY, T_SMALL};
use super::{View, basename, dim, last_rect, store_rect};
use crate::state::{MODES, State};
use egui::{Align, Frame, Id, Key, Layout, Margin, Modifiers, Rect, RichText, Stroke, TextEdit, Ui, UiBuilder, pos2, vec2};

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

/// One control of the composer's controls row, with its laid-out width.
type Control = (f32, fn(&mut Ui, &View, &mut State));

/// Flow the controls into rows by their known widths. egui grows a parent past
/// its max rect when a row overflows (which pushed Send off the glass), so we
/// never let a row overflow instead of relying on wrapping.
fn controls(ui: &mut Ui, v: &View, s: &mut State, inner_w: f32) {
    let gap = 2.0 * SP;
    let mut items: Vec<Control> =
        vec![(176.0, mode_segment), (if s.selected.is_none() { 164.0 } else { 120.0 }, project_chip), (124.0, harness_pick)];
    if let Some(h) = s.composer.harness.clone() {
        s.load_models(&h);
        let has_accounts = s.profiles.as_ref().is_some_and(|p| p.profiles.iter().any(|r| r.profile.harness_id == h && r.profile.enabled));
        if has_accounts {
            items.push((134.0, account_pin));
        }
        items.push((144.0, model_pick));
        if s.harnesses.iter().any(|x| x.id == h && !x.effort_levels(s.composer.model.as_deref()).is_empty()) {
            items.push((114.0, effort_pick));
        }
    }
    let mut rows: Vec<Vec<Control>> = vec![vec![]];
    let mut used = 0.0;
    for it in items {
        if used > 0.0 && used + gap + it.0 > inner_w {
            rows.push(vec![]);
            used = 0.0;
        }
        used += if used > 0.0 { gap } else { 0.0 } + it.0;
        rows.last_mut().expect("at least one row").push(it);
    }
    for (i, row) in rows.into_iter().enumerate() {
        if i > 0 {
            ui.add_space(SP);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (_, draw) in row {
                draw(ui, v, s);
            }
        });
    }
    if s.effective_root().is_none() && s.composer.mode != 0 {
        s.composer.mode = 0;
    }
}

/// Ask / Plan / Agent (Plan and Agent need a project).
fn mode_segment(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    let has_project = s.effective_root().is_some();
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
                let color = if on {
                    t.on_accent
                } else if enabled {
                    t.text
                } else {
                    t.text3
                };
                let b = egui::Button::new(RichText::new(label).size(T_SMALL).color(color))
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
}

/// The working directory: selectable for a draft thread, bound for an open one.
fn project_chip(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    if s.selected.is_none() {
        let label = s.composer.project.as_deref().map(basename).unwrap_or("No project").to_string();
        drop_up(ui, "project", RichText::new(format!("📁 {label}")).size(T_SMALL), 140.0, true, |ui| {
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
                ui.close();
            }
            ui.ctx().data_mut(|d| d.insert_temp(key, path));
        });
    } else if let Some(root) = s.effective_root() {
        ui.add(egui::Label::new(dim(&t, format!("📁 {}", basename(&root)))).truncate()).on_hover_text(root);
    } else {
        ui.label(dim(&t, "no project · Ask only"));
    }
}

fn harness_pick(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    let hl = s.composer.harness.clone().unwrap_or_else(|| "Auto".into());
    let before = s.composer.harness.clone();
    drop_up(
        ui,
        "harness",
        RichText::new(&hl).size(T_SMALL).color(s.composer.harness.as_deref().map_or(t.text, |h| t.harness(h))),
        100.0,
        false,
        |ui| {
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
        },
    );
    if s.composer.harness != before {
        s.composer.model = None;
        s.composer.effort = None;
        s.composer.account = None;
    }
}

/// Model for the explicit harness — only its truth-source list, no free text.
fn model_pick(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    let Some(h) = s.composer.harness.clone() else { return };
    let models = s.models.get(&h).cloned();
    let ml = s.composer.model.clone().unwrap_or_else(|| "Default model".into());
    drop_up(ui, ("model", &h), RichText::new(&ml).size(T_SMALL), 120.0, false, |ui| {
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
}

fn effort_pick(ui: &mut Ui, _v: &View, s: &mut State) {
    let Some(h) = s.composer.harness.clone() else { return };
    let levels = s.harnesses.iter().find(|x| x.id == h).map(|x| x.effort_levels(s.composer.model.as_deref())).unwrap_or_default();
    let el = s.composer.effort.clone().unwrap_or_else(|| "Default effort".into());
    drop_up(ui, ("effort", &h), RichText::new(&el).size(T_SMALL), 90.0, false, |ui| {
        ui.selectable_value(&mut s.composer.effort, None, "Default effort");
        for l in &levels {
            ui.selectable_value(&mut s.composer.effort, Some(l.clone()), l);
        }
    });
    if s.composer.effort.as_ref().is_some_and(|e| !levels.contains(e)) {
        s.composer.effort = None;
    }
}

fn input(ui: &mut Ui, v: &View, s: &mut State, inner_w: f32) {
    let t = v.t;
    let running = s.head_live_id();
    attachments_row(ui, v, s);
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

/// Pin the turn to one account of the chosen harness ("Automatic" = the
/// quota-aware pool). Only enabled accounts are offered; the engine refuses
/// unknown/disabled ids rather than silently defaulting.
fn account_pin(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    let Some(h) = s.composer.harness.clone() else { return };
    let rows: Vec<(String, String, bool)> = s
        .profiles
        .as_ref()
        .map(|p| {
            p.profiles
                .iter()
                .filter(|r| r.profile.harness_id == h && r.profile.enabled)
                .map(|r| (r.profile.profile_id.clone(), r.label(), r.ready()))
                .collect()
        })
        .unwrap_or_default();
    if rows.is_empty() {
        return;
    }
    let cur = s
        .composer
        .account
        .as_ref()
        .and_then(|id| rows.iter().find(|r| &r.0 == id))
        .map(|r| r.1.clone())
        .unwrap_or_else(|| "Automatic".into());
    drop_up(ui, ("account", &h), RichText::new(cur).size(T_SMALL), 110.0, false, |ui| {
        ui.selectable_value(&mut s.composer.account, None, "Automatic")
            .on_hover_text("Route through the quota-aware pool of enabled accounts");
        for (id, label, ready) in &rows {
            let text = if *ready { RichText::new(label) } else { RichText::new(format!("{label} · not ready")).color(t.text3) };
            ui.selectable_value(&mut s.composer.account, Some(id.clone()), text);
        }
    });
}

/// Attach (file chooser) · Capture (screen region) · removable chips with
/// upload state. Send stays blocked while any upload is in flight.
fn attachments_row(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0 * SP;
        let online = s.client.is_some() && !s.composer.picking;
        if ui
            .add_enabled(online, egui::Button::new(RichText::new("+ Attach").size(T_SMALL)).frame(false))
            .on_hover_text("Attach files")
            .clicked()
        {
            s.pick_files();
        }
        if ui
            .add_enabled(online, egui::Button::new(RichText::new("Capture").size(T_SMALL)).frame(false))
            .on_hover_text("Capture a screen region (grim + slurp)")
            .clicked()
        {
            s.capture_region();
        }
        if s.composer.picking {
            ui.spinner();
        }
        for a in &s.composer.attachments {
            let (c, note) = match &a.state {
                crate::state::AttachState::Uploading => (t.running, "uploading…".to_string()),
                crate::state::AttachState::Ready(_) => (t.text2, format!("{} KB", a.size.div_ceil(1024))),
                crate::state::AttachState::Failed(e) => (t.failed, e.clone()),
            };
            Frame::new()
                .fill(t.raised)
                .corner_radius(10)
                .inner_margin(Margin::symmetric(8, 3))
                .stroke(Stroke::new(1.0_f32, c.gamma_multiply(0.5)))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(RichText::new(&a.name).size(T_SMALL).color(t.text)).on_hover_text(&note);
                        ui.label(RichText::new(&note).size(T_SMALL - 1.0).color(c));
                        if ui
                            .add(egui::Button::new(RichText::new("×").size(T_SMALL)).frame(false))
                            .on_hover_text("Remove attachment")
                            .clicked()
                        {
                            remove = Some(a.local);
                        }
                    });
                });
        }
    });
    if let Some(l) = remove {
        s.remove_attachment(l);
    }
}

/// A combo-style button whose list opens ABOVE it: the composer sits at the
/// bottom of the window, where a ComboBox's downward list was clipped off-screen.
/// `keep_open` keeps the list up while typing into a field inside it.
fn drop_up(ui: &mut Ui, id_salt: impl std::hash::Hash, text: RichText, width: f32, keep_open: bool, add: impl FnOnce(&mut Ui)) {
    let resp = ui.add(egui::Button::new(text).right_text(RichText::new("▲").size(7.0)).min_size(vec2(width, 24.0)));
    let mut popup = egui::Popup::menu(&resp)
        .id(ui.id().with(id_salt))
        .align(egui::RectAlign::TOP_START)
        .align_alternatives(&[egui::RectAlign::TOP_START, egui::RectAlign::TOP_END]);
    if keep_open {
        popup = popup.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    }
    popup.show(|ui| {
        ui.set_min_width(width);
        egui::ScrollArea::vertical().max_height(320.0).show(ui, add);
    });
}
