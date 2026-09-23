//! Floating composer (Liquid Glass chrome, solid contents: no glass-on-glass).
//! One controls row (DESIGN_SYSTEM §5 "Chat composer"): intent · project ·
//! harness+account · access, then attach · capture · ⋯ options on the right;
//! a growing input (capped at 6 lines, scrolls inside) with Send / Stop.
//! Enter sends; Shift/Alt+Enter is a newline; Ctrl+Enter sends, or stops a
//! running turn (plain Enter never stops one).

use super::glass::Kind;
use super::theme::{R_LG, R_SM, SP, T_BODY, T_CAPTION, T_SMALL};
use super::{View, basename, caption, chip_menu, dim, ic, icon_button, icons, last_rect, row, segmented, store_rect, toggle_switch};
use crate::model::{Strategy, TurnOpts};
use crate::state::{MODES, State};
use egui::{Align, Event, Frame, Id, Key, Layout, Margin, Modifiers, Rect, RichText, Stroke, TextEdit, Ui, UiBuilder, pos2, vec2};

const INPUT: &str = "composer-input";

/// Draw the composer anchored to the bottom of `area`; returns its height.
pub fn show(ui: &mut Ui, v: &mut View, s: &mut State, area: Rect) -> f32 {
    let w = (area.width() - 8.0 * SP).min(super::theme::MEASURE + 40.0);
    let id = Id::new("composer");
    let h = last_rect(ui, id).map_or(104.0, |r| r.height());
    let rect = Rect::from_min_size(pos2(area.center().x - w / 2.0, area.max.y - h - 4.0 * SP), vec2(w, h));
    v.glass.surface(ui, rect, R_LG, Kind::Chrome);

    let mut out = rect;
    ui.scope_builder(UiBuilder::new().max_rect(Rect::from_min_size(rect.min, vec2(w, 400.0))), |ui| {
        let r = Frame::new().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(w - 28.0);
            ui.spacing_mut().item_spacing.y = 2.0 * SP;
            controls(ui, v, s, w - 28.0);
            attachments_row(ui, v, s);
            input(ui, v, s, w - 28.0);
            nesting_hint(ui, v, s);
        });
        out = r.response.rect;
    });
    store_rect(ui, id, out);
    // buttons don't keep focus (Mac): after a chip or Send click, typing goes
    // back to the message field, unless another field or a menu has it.
    if ui.memory(|m| m.focused().is_none()) && !egui::Popup::is_any_open(ui.ctx()) && s.client.is_some() {
        ui.memory_mut(|m| m.request_focus(Id::new(INPUT)));
    }
    out.height()
}

/// The controls row, wrapped only when it can't fit (chips never wrap inside).
fn controls(ui: &mut Ui, v: &View, s: &mut State, _inner_w: f32) {
    if let Some(root) = s.effective_root() {
        s.load_trust(&root);
    }
    if s.effective_root().is_none() && s.composer.mode != 0 {
        s.composer.mode = 0;
    }
    // chips wrap in the width left of a fixed icon slot, so nothing overlaps
    let icons_w = 3.0 * 26.0 + 8.0;
    ui.horizontal(|ui| {
        let left_w = (ui.available_width() - icons_w).max(120.0);
        ui.allocate_ui_with_layout(vec2(left_w, 24.0), Layout::left_to_right(Align::Center).with_main_wrap(true), |ui| {
            ui.spacing_mut().item_spacing = vec2(2.0 * SP, SP);
            mode_segment(ui, v, s);
            project_chip(ui, v, s);
            route_chip(ui, v, s);
            if s.current_mode() == "agent" && s.effective_root().is_some() {
                access_chip(ui, v, s);
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            options_pick(ui, v, s);
            let online = s.client.is_some() && !s.composer.picking;
            ui.add_enabled_ui(online, |ui| {
                if icon_button(ui, &v.t, icons::CAMERA, "Capture a screen region (grim + slurp)").clicked() {
                    s.capture_region();
                }
                if icon_button(ui, &v.t, icons::PAPERCLIP, "Attach files").clicked() {
                    s.pick_files();
                }
            });
        });
    });
}

/// Ask / Plan / Agent (Plan and Agent need a project).
fn mode_segment(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    let has_project = s.effective_root().is_some();
    let mut mode = s.composer.mode;
    ui.add_enabled_ui(true, |ui| {
        let labels = [(0usize, "Ask"), (1, "Plan"), (2, "Agent")];
        let allowed: Vec<(usize, &str)> = if has_project { labels.to_vec() } else { labels[..1].to_vec() };
        if segmented(ui, &t, &mut mode, &allowed) {
            s.composer.mode = mode;
        }
    })
    .response
    .on_hover_text(if has_project { "Ask: read-only answer · Plan: a report with open questions · Agent: writes code" } else { "Pick a project to use Plan and Agent" });
    let _ = MODES;
}

/// The working directory: selectable for a draft thread, bound for an open one.
fn project_chip(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    if s.selected.is_none() {
        let label = s.composer.project.as_deref().map(basename).unwrap_or("No project").to_string();
        chip_menu(ui, &t, "project", &format!("{} {label}", icons::FOLDER), t.text, true, |ui| {
            ui.selectable_value(&mut s.composer.project, None, "No project (Ask only)");
            for p in &s.projects {
                ui.selectable_value(&mut s.composer.project, Some(p.root.clone()), &p.root);
            }
            ui.add_space(SP);
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
        ui.add(egui::Label::new(RichText::new(format!("{} {}", icons::FOLDER, basename(&root))).size(T_SMALL).color(t.text2)).truncate()).on_hover_text(root);
    } else {
        ui.label(dim(&t, "No project · Ask only"));
    }
}

/// Harness and account in ONE capsule (Mac HarnessAccountChip): "claude · work".
fn route_chip(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    let h = s.composer.harness.clone();
    let accounts: Vec<(String, String, bool)> = match (&h, &s.profiles) {
        (Some(h), Some(p)) => p.profiles.iter().filter(|r| &r.profile.harness_id == h && r.profile.enabled).map(|r| (r.profile.profile_id.clone(), r.label(), r.ready())).collect(),
        _ => vec![],
    };
    let account = s.composer.account.as_ref().and_then(|id| accounts.iter().find(|r| &r.0 == id)).map(|r| r.1.clone());
    let label = match (&h, &account) {
        (None, _) => "Auto".to_string(),
        (Some(h), Some(a)) => format!("{h} · {a}"),
        (Some(h), None) => h.clone(),
    };
    let color = h.as_deref().map_or(t.text, |x| t.harness(x));
    let (before_h, before_a) = (s.composer.harness.clone(), s.composer.account.clone());
    chip_menu(ui, &t, "route", &label, color, false, |ui| {
        caption(ui, &t, "Harness");
        ui.selectable_value(&mut s.composer.harness, None, "Auto").on_hover_text("Let the engine route by quota and readiness");
        for x in &s.harnesses {
            let ok = x.status != "unavailable";
            let text = RichText::new(x.label()).color(if ok { t.harness(&x.id) } else { t.text3 });
            let resp = ui.add_enabled(ok, egui::Button::selectable(s.composer.harness.as_deref() == Some(&x.id), text));
            let resp = if x.reasons.is_empty() { resp } else { resp.on_hover_text(x.reasons.join("\n")) };
            if resp.on_disabled_hover_text(format!("{}: unavailable", x.id)).clicked() {
                s.composer.harness = Some(x.id.clone());
            }
        }
        if !accounts.is_empty() {
            caption(ui, &t, "Account");
            ui.selectable_value(&mut s.composer.account, None, "Automatic").on_hover_text("Route through the quota-aware pool of enabled accounts");
            for (id, label, ready) in &accounts {
                let text = if *ready { RichText::new(label) } else { RichText::new(format!("{label} · not ready")).color(t.text3) };
                ui.selectable_value(&mut s.composer.account, Some(id.clone()), text);
            }
        }
    });
    if s.composer.harness != before_h {
        s.composer.model = None;
        s.composer.effort = None;
        s.composer.account = None;
        s.patch_thread(serde_json::json!({ "primaryHarness": s.composer.harness, "credentialProfileId": null }));
    } else if s.composer.account != before_a {
        s.patch_thread(serde_json::json!({ "primaryHarness": s.composer.harness, "credentialProfileId": s.composer.account }));
    }
    if let Some(h) = &h {
        s.load_models(h);
    }
}

/// Access, visible on the row for Agent turns (amber when Full).
fn access_chip(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    let access = s.effective_access();
    let label = match access {
        "readonly" => "Read-only",
        "full" => "Full access",
        _ => "Workspace write",
    };
    let color = if access == "full" { t.blocked } else { t.text };
    let root = s.effective_root().unwrap_or_default();
    let trust = s.repo_trust().cloned();
    let default_label = match trust.as_ref().map(|t| t.access_default.as_str()) {
        Some("readonly") => "Repo default (read-only)",
        Some("full") => "Repo default (full)",
        _ => "Repo default (workspace write)",
    };
    let before = s.composer.opts.access;
    chip_menu(ui, &t, "access", &format!("{} {label}", icons::SHIELD), color, true, |ui| {
        for (k, l) in [(None, default_label), (Some("readonly"), "Read-only"), (Some("workspace_write"), "Workspace write"), (Some("full"), "Full access")] {
            ui.selectable_value(&mut s.composer.opts.access, k, l);
        }
        if s.composer.opts.access == Some("full") && !trust.as_ref().is_some_and(|t| t.allow_full_access) {
            ui.add_space(SP);
            ui.add(egui::Label::new(RichText::new("Full access runs unsandboxed. It needs a recorded grant for this repo.").size(T_SMALL).color(t.blocked)).wrap());
            // two deliberate steps: one stray click must never grant it
            let key = Id::new(("full-access-ack", &root));
            let mut ack: bool = ui.ctx().data(|d| d.get_temp(key)).unwrap_or(false);
            ui.horizontal(|ui| {
                toggle_switch(ui, &t, &mut ack);
                ui.add(egui::Label::new(RichText::new(format!("I want agents to run unsandboxed in {}", basename(&root))).size(T_SMALL)).wrap());
            });
            ui.ctx().data_mut(|d| d.insert_temp(key, ack));
            if ui.add_enabled(ack, egui::Button::new(RichText::new("Grant full access").size(T_SMALL))).clicked() {
                s.grant_full_access(&root);
                ui.ctx().data_mut(|d| d.remove::<bool>(key));
            }
        }
    });
    if s.composer.opts.access != before {
        s.patch_thread(serde_json::json!({ "access": s.composer.opts.access }));
    }
}

/// Take an Enter press with EXACTLY these modifiers (egui's `consume_key`
/// ignores extra Shift/Alt, which made Shift+Enter send).
fn take_enter(ui: &Ui, want: Modifiers) -> bool {
    ui.input_mut(|i| {
        let mut hit = false;
        i.events.retain(|e| match e {
            Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if modifiers.matches_exact(want) => {
                hit = true;
                false
            }
            _ => true,
        });
        hit
    })
}

fn input(ui: &mut Ui, v: &View, s: &mut State, inner_w: f32) {
    let t = v.t;
    let running = s.head_live_id();
    ui.horizontal(|ui| {
        let edit_id = Id::new(INPUT);
        let focused = ui.memory(|m| m.has_focus(edit_id));
        let (mut send_key, mut stop_key) = (false, false);
        if focused {
            if running.is_some() {
                stop_key = take_enter(ui, Modifiers::COMMAND);
                // plain Enter neither sends nor stops while a turn runs
                let _ = take_enter(ui, Modifiers::NONE);
            } else {
                send_key = take_enter(ui, Modifiers::NONE) | take_enter(ui, Modifiers::COMMAND);
            }
        }
        let btn_w = 76.0;
        let line_h = ui.fonts_mut(|f| f.row_height(&egui::FontId::proportional(T_BODY)));
        Frame::new()
            .fill(t.raised)
            .corner_radius(R_SM + 4)
            .inner_margin(Margin::symmetric(12, 8))
            .stroke(Stroke::new(1.5_f32, if focused { t.accent.gamma_multiply(0.6) } else { t.separator }))
            .show(ui, |ui| {
                let hint = if s.client.is_none() {
                    "Engine offline"
                } else if s.selected.is_none() {
                    "Ask, plan or describe a change…"
                } else {
                    "Continue this conversation…"
                };
                // grows to 6 lines, then scrolls inside instead of covering the chat
                egui::ScrollArea::vertical()
                    .id_salt("composer-scroll")
                    .max_height(6.0 * line_h + 2.0)
                    .min_scrolled_height(line_h) // egui's 64 px default made one line look like four
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                    ui.add(
                        TextEdit::multiline(&mut s.composer.text)
                            .id(edit_id)
                            .hint_text(RichText::new(hint).color(t.text3))
                            .desired_rows(1)
                            .desired_width(inner_w - btn_w - 2.0 * SP - 26.0)
                            .frame(Frame::NONE)
                            .font(egui::FontId::proportional(T_BODY)),
                    );
                });
            });
        ui.with_layout(Layout::bottom_up(Align::Max), |ui| {
            if let Some(id) = &running {
                let b = egui::Button::new(RichText::new(format!("{} Stop", icons::SQUARE)).size(T_SMALL).color(t.text))
                    .fill(t.raised)
                    .stroke(Stroke::new(1.0_f32, t.separator))
                    .corner_radius(17)
                    .min_size(vec2(btn_w, 34.0));
                if ui.add_enabled(s.client.is_some(), b).on_hover_text(format!("Stop the running turn (Ctrl+Enter) · {id}")).clicked() || stop_key {
                    s.cancel_head();
                }
            } else {
                let ready = s.can_send();
                let label = if s.composer.sending { "Sending…" } else { "Send" };
                let b = egui::Button::new(RichText::new(label).size(T_SMALL).color(t.on_accent))
                    .fill(if ready { t.accent_solid } else { t.accent_solid.gamma_multiply(0.35) })
                    .corner_radius(17)
                    .min_size(vec2(btn_w, 34.0));
                let opt_err = s.options_error();
                let why = if s.client.is_none() {
                    "Engine offline"
                } else if s.composer.text.trim().is_empty() {
                    "Type a message first"
                } else if let Some(e) = &opt_err {
                    e
                } else {
                    "Sending…"
                };
                if ui.add_enabled(ready, b).on_hover_text("Send (Enter)").on_disabled_hover_text(why).clicked() || (send_key && ready) {
                    s.submit();
                }
            }
        });
    });
}

/// Overlapping registered roots keep separate thread/artifact/trust identities.
fn nesting_hint(ui: &mut Ui, v: &View, s: &State) {
    let t = v.t;
    if let Some(p) = s.effective_root().and_then(|r| s.projects.iter().find(|p| p.root == r)) {
        for n in &p.nesting {
            let verb = if n.relation == "inside" { "Nested inside" } else { "Contains" };
            ui.label(RichText::new(format!("{verb} {}", basename(&n.root))).size(T_CAPTION).color(t.text2))
                .on_hover_text(format!("{verb} the registered project at {}. Overlapping roots have separate thread, artifact and trust identities.", n.root));
        }
    }
}

/// "⋯" Options: every per-turn knob the current mode takes, as label-column
/// rows. An accent badge counts the knobs changed from the defaults; it turns
/// red while an option blocks Send.
fn options_pick(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    let mode = if s.effective_root().is_none() { "ask" } else { s.current_mode() };
    let draft_isolated = s.selected.is_none() && s.composer.isolated;
    let n = s.composer.opts.changed(mode) + usize::from(draft_isolated) + usize::from(s.composer.model.is_some()) + usize::from(s.composer.effort.is_some());
    let bad = s.options_error().is_some();
    let resp = icon_button(ui, &t, icons::ELLIPSIS, "Options");
    if n > 0 || bad {
        let c = resp.rect.right_top() + vec2(-5.0, 5.0);
        ui.painter().circle_filled(c, 6.0, if bad { t.failed } else { t.accent_solid });
        ui.painter().text(c, egui::Align2::CENTER_CENTER, if bad { "!".to_string() } else { n.to_string() }, egui::FontId::proportional(9.0), t.on_accent);
    }
    egui::Popup::menu(&resp)
        .id(Id::new("options-popup"))
        .align(egui::RectAlign::TOP_END)
        .align_alternatives(&[egui::RectAlign::TOP_END, egui::RectAlign::TOP_START])
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(380.0);
            let max_h = (resp.rect.top() - 24.0).clamp(320.0, 560.0);
            egui::ScrollArea::vertical().max_height(max_h).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = SP;
                model_rows(ui, v, s);
                match mode {
                    "agent" => agent_options(ui, v, s),
                    "plan" => {
                        caption(ui, &t, "Plan strategy");
                        let o = &mut s.composer.opts;
                        row(ui, &t, "Council", |ui| toggle_switch(ui, &t, &mut o.council))
                            .on_hover_text("Several harnesses draft in parallel; the primary merges");
                        if o.council {
                            row(ui, &t, "Members", |ui| stepper(ui, &t, &mut o.members, 2, 4));
                        }
                    }
                    _ => {
                        caption(ui, &t, "Ask");
                        row(ui, &t, "Deep scan", |ui| toggle_switch(ui, &t, &mut s.composer.opts.deep_scan)).on_hover_text("Read the whole project first");
                    }
                }
                caption(ui, &t, "Routing");
                row(ui, &t, "Web", |ui| choice(ui, &t, &mut s.composer.opts.web, &[(None, "Default"), (Some("off"), "Off"), (Some("auto"), "Auto"), (Some("cached"), "Cached"), (Some("live"), "Live")]));
                row(ui, &t, "Auth route", |ui| choice(ui, &t, &mut s.composer.opts.auth, &[(None, "Default"), (Some("auto"), "Auto"), (Some("subscription"), "Subscription"), (Some("api_key"), "API key")]));
                row(ui, &t, "Budget", |ui| {
                    ui.label(RichText::new("$").size(T_SMALL).color(t.text2));
                    ui.add(TextEdit::singleline(&mut s.composer.opts.budget).hint_text("settings default").desired_width(110.0));
                });
                if s.selected.is_none() && s.composer.project.is_some() {
                    caption(ui, &t, "Workspace");
                    row(ui, &t, "Isolated", |ui| toggle_switch(ui, &t, &mut s.composer.isolated)).on_hover_text("An own git worktree for this thread; Apply thread merges it");
                }
                if let Some(e) = s.options_error() {
                    ui.add(egui::Label::new(RichText::new(e).size(T_SMALL).color(t.failed)).wrap());
                }
                if n > 0 && ui.button(RichText::new("Reset to defaults").size(T_SMALL)).clicked() {
                    s.composer.opts = TurnOpts::default();
                    s.composer.isolated = false;
                    s.composer.model = None;
                    s.composer.effort = None;
                }
            });
        });
}

/// Model and effort for the chosen harness (its truth-source lists only).
fn model_rows(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    let Some(h) = s.composer.harness.clone() else { return };
    caption(ui, &t, "Model");
    let models: Vec<(String, String)> = match s.models.get(&h) {
        Some(Ok(l)) => l.models.iter().map(|m| (m.id.clone(), m.label.clone().unwrap_or_else(|| m.id.clone()))).collect(),
        _ => vec![],
    };
    row(ui, &t, "Model", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut s.composer.model, None, RichText::new("Default").size(T_SMALL));
            for (id, label) in &models {
                ui.selectable_value(&mut s.composer.model, Some(id.clone()), RichText::new(label).size(T_SMALL));
            }
        });
    });
    let levels = s.harnesses.iter().find(|x| x.id == h).map(|x| x.effort_levels(s.composer.model.as_deref())).unwrap_or_default();
    if s.composer.effort.as_ref().is_some_and(|e| !levels.contains(e)) {
        s.composer.effort = None;
    }
    if !levels.is_empty() {
        row(ui, &t, "Effort", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut s.composer.effort, None, RichText::new("Default").size(T_SMALL));
                for l in &levels {
                    ui.selectable_value(&mut s.composer.effort, Some(l.clone()), RichText::new(l).size(T_SMALL));
                }
            });
        });
    }
}

fn agent_options(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    let access = s.effective_access();
    caption(ui, &t, "Agent strategy");
    let mut strategies = vec![(Strategy::Single, "Single"), (Strategy::BestOf, "Best-of"), (Strategy::UntilClean, "Until clean"), (Strategy::Create, "Create")];
    if access == "readonly" {
        strategies.retain(|x| x.0 != Strategy::UntilClean);
    }
    let mut strat = s.composer.opts.strategy_for(access);
    if segmented(ui, &t, &mut strat, &strategies) {
        s.composer.opts.strategy = strat;
    }
    let blurb = match strat {
        Strategy::Single => "One candidate; review is optional.",
        Strategy::BestOf => "One candidate per pooled harness in isolated envelopes, cross-reviewed; the best wins.",
        Strategy::UntilClean => "One envelope repaired until gates and review are clean.",
        Strategy::Create => "Scaffold a new repo or component.",
    };
    ui.add(egui::Label::new(dim(&t, blurb)).wrap());
    match strat {
        Strategy::Single if access != "readonly" => {
            row(ui, &t, "Max attempts", |ui| stepper(ui, &t, &mut s.composer.opts.attempts, 1, 8));
        }
        Strategy::BestOf => {
            ui.label(dim(&t, "Pool (none = the engine picks two)"));
            let ids: Vec<(String, bool)> = s.harnesses.iter().map(|h| (h.id.clone(), h.status != "unavailable")).collect();
            for (id, ok) in ids {
                let mut on = s.composer.opts.pool.contains(&id);
                let changed = ui.add_enabled_ui(ok, |ui| row(ui, &t, &id, |ui| toggle_switch(ui, &t, &mut on).changed())).inner;
                if changed {
                    if on {
                        s.composer.opts.pool.push(id.clone());
                    } else {
                        s.composer.opts.pool.retain(|x| x != &id);
                        s.composer.opts.models.remove(&id);
                    }
                }
                if on {
                    s.load_models(&id);
                    pool_model(ui, v, s, &id);
                }
            }
        }
        _ => {}
    }

    let primary = s.composer.harness.clone();
    let offered = |f: fn(&crate::model::Harness) -> bool| match &primary {
        Some(p) => s.harnesses.iter().any(|h| &h.id == p && f(h)),
        None => s.harnesses.iter().any(f),
    };
    let (can_delegate, can_browse) = (offered(|h| h.can_delegate()), offered(|h| h.browser_tool()));
    let o = &mut s.composer.opts;
    if !can_delegate {
        o.delegate = false;
    }
    ui.add_enabled_ui(can_delegate, |ui| row(ui, &t, "Delegate", |ui| toggle_switch(ui, &t, &mut o.delegate)))
        .response
        .on_hover_text(if can_delegate { "Let the agent spawn bounded sub-runs" } else { "The chosen harness cannot receive the delegation belt" });
    if can_browse {
        row(ui, &t, "Browser", |ui| toggle_switch(ui, &t, &mut o.browser)).on_hover_text("The agent drives a real browser window");
    } else {
        o.browser = false;
    }

    caption(ui, &t, "Review");
    let promised = matches!(strat, Strategy::BestOf | Strategy::UntilClean);
    let mut on = promised || o.review || !o.panel.trim().is_empty();
    let changed = ui.add_enabled_ui(!promised, |ui| row(ui, &t, "Review changes", |ui| toggle_switch(ui, &t, &mut on).changed())).inner;
    if changed {
        o.review = on;
    }
    row(ui, &t, "Reviewers", |ui| {
        ui.add(TextEdit::singleline(&mut o.panel).hint_text("codex=gpt-5:high, claude").desired_width(220.0))
            .on_hover_text("Explicit reviewer panel: harness[=model[:effort]], comma separated")
    });
    if o.strategy == Strategy::Create {
        row(ui, &t, "Test command", |ui| {
            ui.add(TextEdit::singleline(&mut o.test_command).hint_text("e.g. npm test").desired_width(220.0))
                .on_hover_text("A deterministic gate run after the candidate. Typed argv: quotes group words; no shell, pipes or variables.")
        });
    }
    row(ui, &t, "Protected paths", |ui| {
        ui.add(TextEdit::multiline(&mut o.approvals).hint_text("glob[:reason], one per line").desired_rows(1).desired_width(220.0))
            .on_hover_text("Paths this turn may change although they are protected (auto-protected gate/test paths only)")
    });
}

/// Model choices for one pooled harness (its truth-source list only).
fn pool_model(ui: &mut Ui, v: &View, s: &mut State, harness: &str) {
    let Some(Ok(list)) = s.models.get(harness) else { return };
    let ids: Vec<(String, String)> = list.models.iter().map(|m| (m.id.clone(), m.label.clone().unwrap_or_else(|| m.id.clone()))).collect();
    if ids.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.add_space(128.0);
        let cur = s.composer.opts.models.get(harness).cloned();
        if ui.selectable_label(cur.is_none(), RichText::new("default").size(T_SMALL)).clicked() {
            s.composer.opts.models.remove(harness);
        }
        for (id, label) in ids {
            if ui.selectable_label(cur.as_deref() == Some(&id), RichText::new(label).size(T_SMALL).color(v.t.text2)).clicked() {
                s.composer.opts.models.insert(harness.to_string(), id);
            }
        }
    });
}

/// A segmented choice over optional wire values.
fn choice(ui: &mut Ui, t: &super::theme::Theme, value: &mut Option<&'static str>, options: &[(Option<&'static str>, &str)]) {
    segmented(ui, t, value, options);
}

/// − n + stepper within bounds.
fn stepper(ui: &mut Ui, t: &super::theme::Theme, value: &mut u32, lo: u32, hi: u32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if ui.add_enabled(*value > lo, egui::Button::new(RichText::new("−").size(T_SMALL)).min_size(vec2(24.0, 22.0))).clicked() {
            *value -= 1;
        }
        ui.add_sized(vec2(28.0, 22.0), egui::Label::new(RichText::new(value.to_string()).size(T_SMALL).color(t.text)));
        if ui.add_enabled(*value < hi, egui::Button::new(RichText::new("+").size(T_SMALL)).min_size(vec2(24.0, 22.0))).clicked() {
            *value += 1;
        }
    });
}

/// Attachment chips with upload state; only shown when there is something to
/// show. Send stays blocked while any upload is in flight.
fn attachments_row(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    if s.composer.attachments.is_empty() && !s.composer.picking {
        return;
    }
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0 * SP;
        if s.composer.picking {
            super::spinner(ui, 12.0, v.t.text2);
        }
        for a in &s.composer.attachments {
            let (c, note) = match &a.state {
                crate::state::AttachState::Uploading => (t.running, "uploading…".to_string()),
                crate::state::AttachState::Ready(_) => (t.text2, format!("{} KB", a.size.div_ceil(1024))),
                crate::state::AttachState::Failed(e) => (t.failed, e.clone()),
            };
            Frame::new().fill(t.raised).corner_radius(R_SM).inner_margin(Margin::symmetric(8, 3)).stroke(Stroke::new(1.0_f32, c.gamma_multiply(0.5))).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(ic(icons::FILE, 12.0, t.text2));
                    ui.label(RichText::new(&a.name).size(T_SMALL).color(t.text)).on_hover_text(&note);
                    ui.label(RichText::new(&note).size(T_CAPTION).color(c));
                    if icon_button(ui, &t, icons::X, "Remove attachment").clicked() {
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
