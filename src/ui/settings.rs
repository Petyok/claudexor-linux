//! Settings (modal from the ⚙ menu). Every control saves on change as a
//! single-key partial patch (`POST /v2/settings`; absent = keep, null = clear);
//! text fields save on Enter or focus loss. Tabs mirror the Mac app: Routing,
//! Budget, Harnesses (Doctor + per-harness defaults), Keys & trust, Engine.

use super::theme::{R_SM, SP, T_SMALL, T_TITLE, semibold};
use super::icons;
use super::{View, dim};
use crate::model::{Settings, secret_slot};
use crate::state::{Fetch, State};
use egui::{Frame, Id, Margin, RichText, ScrollArea, TextEdit, Ui};
use serde_json::{Value, json};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Routing,
    Budget,
    Harnesses,
    Keys,
    Appearance,
    Engine,
}

/// Client-only preferences the shell owns (theme 0 system · 1 light · 2 dark).
pub struct Appearance {
    pub theme: u8,
    pub reduce_transparency: bool,
}

/// Draw the modal; returns false once the user closes it.
pub fn show(ctx: &egui::Context, v: &mut View, s: &mut State, tab: &mut Tab, look: &mut Appearance) -> bool {
    let t = v.t;
    let mut open = true;
    let modal = egui::Modal::new(Id::new("settings")).show(ctx, |ui| {
        ui.set_width(660.0_f32.min(ctx.content_rect().width() - 48.0));
        ui.spacing_mut().item_spacing.y = 2.0 * SP;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Settings").family(semibold()).size(T_TITLE).color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new("Done").color(t.on_accent)).fill(t.accent_solid).corner_radius(R_SM)).clicked() {
                    open = false;
                }
            });
        });
        let before = *tab;
        super::segmented(
            ui,
            &t,
            tab,
            &[(Tab::Routing, "Routing"), (Tab::Budget, "Budget"), (Tab::Harnesses, "Harnesses"), (Tab::Keys, "Keys & trust"), (Tab::Appearance, "Appearance"), (Tab::Engine, "Engine")],
        );
        if *tab != before {
            // the modal resizes to the new tab: paint the settled layout too
            ui.ctx().request_repaint();
        }
        let body_h = (ctx.content_rect().height() - 220.0).max(240.0);
        // client-only tabs work without the engine; the rest render once settings arrive
        let st = match &s.settings {
            Some(Fetch::Ready(st)) => Some(st.clone()),
            _ => None,
        };
        ScrollArea::vertical().max_height(body_h).min_scrolled_height(body_h).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0 * SP;
            match (*tab, &st) {
                (Tab::Appearance, _) => appearance(ui, v, look),
                (Tab::Engine, _) => engine(ui, v, s, st.as_ref()),
                (_, None) => loading(ui, v, s),
                (Tab::Routing, Some(st)) => routing(ui, v, s, st),
                (Tab::Budget, Some(st)) => budget(ui, v, s, st),
                (Tab::Harnesses, Some(st)) => harnesses(ui, v, s, st),
                (Tab::Keys, Some(_)) => keys(ui, v, s),
            }
        });
        // "Saved" fades after 2 s; errors stay until the next change
        let fresh = s.settings_note_at.is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(2));
        match &s.settings_note {
            Some(Ok(m)) if fresh => {
                ui.label(RichText::new(format!("{} {m}", icons::CHECK)).size(T_SMALL).color(t.success));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            }
            Some(Err(e)) => {
                ui.add(egui::Label::new(RichText::new(e).size(T_SMALL).color(t.failed)).wrap().selectable(true));
            }
            _ => {
                ui.label(dim(&t, "Changes save as you make them."));
            }
        }
    });
    if modal.should_close() {
        open = false;
    }
    open
}

fn loading(ui: &mut Ui, v: &View, s: &State) {
    let t = v.t;
    match &s.settings {
        Some(Fetch::Failed(e)) => {
            ui.label(RichText::new(format!("Could not read settings: {e}")).color(t.failed));
        }
        _ if s.client.is_none() => {
            ui.label(dim(&t, "The engine is offline; these settings open once it is back."));
        }
        _ => {
            ui.horizontal(|ui| {
                super::spinner(ui, 12.0, v.t.text2);
                ui.label(dim(&t, "Reading settings…"));
            });
        }
    }
}

fn appearance(ui: &mut Ui, v: &View, look: &mut Appearance) {
    let t = v.t;
    super::group(ui, &t, |ui| {
        super::row(ui, &t, "Theme", |ui| super::segmented(ui, &t, &mut look.theme, &[(0u8, "System"), (1, "Light"), (2, "Dark")]));
        super::row(ui, &t, "Reduce transparency", |ui| super::toggle_switch(ui, &t, &mut look.reduce_transparency))
            .on_hover_text("Solid surfaces instead of glass");
        if !v.glass.blur_available() {
            ui.label(RichText::new("Frosted glass is unavailable on this GPU: panels use solid fallbacks.").size(T_SMALL).color(t.text2));
        }
    });
}

fn heading(ui: &mut Ui, v: &View, text: &str, help: &str) {
    ui.add_space(SP / 2.0);
    ui.label(RichText::new(text).strong().color(v.t.text));
    if !help.is_empty() {
        ui.add(egui::Label::new(dim(&v.t, help)).wrap());
    }
}

/// A segmented choice that saves `{key: value}` on change.
fn choice(ui: &mut Ui, v: &View, s: &mut State, key: &str, current: Option<&str>, options: &[(&str, &str)]) {
    let t = v.t;
    let opts: Vec<(Option<&str>, &str)> = options.iter().map(|(k, l)| (Some(*k), *l)).collect();
    let mut cur = current;
    if super::segmented(ui, &t, &mut cur, &opts) {
        if let Some(value) = cur {
            s.save_settings(json!({ key: value }));
        }
    }
}

/// A text field with a draft kept across frames; `commit` runs on Enter / focus loss.
fn text_field(ui: &mut Ui, id: impl std::hash::Hash, stored: String, hint: &str, width: f32, password: bool) -> Option<String> {
    let key = Id::new(("settings-field", id));
    let mut draft: String = ui.ctx().data(|d| d.get_temp(key)).unwrap_or_else(|| stored.clone());
    let r = ui.add(TextEdit::singleline(&mut draft).hint_text(hint).desired_width(width).password(password));
    let commit = r.lost_focus() && draft != stored;
    if r.has_focus() || draft != stored {
        ui.ctx().data_mut(|d| d.insert_temp(key, draft.clone()));
    }
    if commit || (r.lost_focus() && password) {
        ui.ctx().data_mut(|d| d.remove::<String>(key));
    }
    commit.then_some(draft)
}

fn routing(ui: &mut Ui, v: &View, s: &mut State, st: &Settings) {
    let r = &st.routing;
    heading(ui, v, "Routing goal", "Auto paces expiring quota, Quality uses your highest comparable tier, Economy minimizes incremental paid spend.");
    choice(ui, v, s, "routingGoal", r.goal.as_deref(), &[("auto", "Auto"), ("quality", "Quality"), ("economy", "Economy")]);
    heading(ui, v, "Paid fallback", "Whether a run may move to a paid route when subscriptions can't take it.");
    choice(
        ui,
        v,
        s,
        "paidFallback",
        r.paid_fallback.as_deref(),
        &[("never", "Never"), ("when_unavailable", "When unavailable"), ("allowed_within_cap", "Allowed within cap")],
    );
    heading(ui, v, "Primary harness", "Who answers when a turn names none. Engine routing picks by quota and readiness.");
    let ids: Vec<String> =
        s.harnesses.iter().filter(|h| s.harness_usable(h) || r.primary_harness.as_deref() == Some(&h.id)).map(|h| h.id.clone()).collect();
    ui.horizontal_wrapped(|ui| {
        if ui.selectable_label(r.primary_harness.is_none(), "Engine routing").clicked() && r.primary_harness.is_some() {
            s.save_settings(json!({ "primaryHarness": Value::Null }));
        }
        for id in ids {
            let on = r.primary_harness.as_deref() == Some(&id);
            if ui.selectable_label(on, &id).clicked() && !on {
                s.save_settings(json!({ "primaryHarness": id }));
            }
        }
    });
    heading(ui, v, "Auth route", "Subscription sessions first, API keys first, or let the engine choose.");
    choice(ui, v, s, "authPreference", r.auth_preference.as_deref(), &[("auto", "Auto"), ("subscription", "Subscription"), ("api_key", "API key")]);
    heading(ui, v, "Environment", "Mirror your native shell environment into harness processes, or start them clean.");
    choice(ui, v, s, "envInheritance", r.env_inheritance.as_deref(), &[("mirror_native", "Mirror native"), ("clean", "Clean")]);
}

fn budget(ui: &mut Ui, v: &View, s: &mut State, st: &Settings) {
    let t = v.t;
    let pb = st.budget.paid_budget_per_run.clone().unwrap_or(json!({"kind": "unlimited"}));
    let unlimited = pb["kind"] != "finite";
    heading(ui, v, "Paid budget per run", "Unlimited still records exact or estimated spend; it removes only the cap. Zero admits only subscription or proven-free routes.");
    let mut u = unlimited;
    if super::row(ui, &t, "Unlimited", |ui| super::toggle_switch(ui, &t, &mut u)).changed() {
        let patch = if u { json!({"kind": "unlimited"}) } else { json!({"kind": "finite", "maxUsd": 1.0}) };
        s.save_settings(json!({ "paidBudgetPerRun": patch }));
    }
    if !unlimited {
        ui.horizontal(|ui| {
            ui.label("Max USD per run");
            let cur = pb["maxUsd"].as_f64().map(|x| format!("{x}")).unwrap_or_default();
            if let Some(text) = text_field(ui, "maxUsd", cur, "e.g. 2.50", 100.0, false) {
                match text.trim().trim_start_matches('$').parse::<f64>() {
                    Ok(x) if x.is_finite() && x >= 0.0 => s.save_settings(json!({"paidBudgetPerRun": {"kind": "finite", "maxUsd": x}})),
                    _ => s.note_settings(Err(format!("“{text}” is not a dollar amount"))),
                }
            }
        });
    }
    heading(ui, v, "Question timeout", "How long a run waits for your answer to an interactive question. Empty = wait forever.");
    ui.horizontal(|ui| {
        let cur = st.interaction_timeout_ms.map(|ms| format!("{}", ms / 60_000)).unwrap_or_default();
        if let Some(text) = text_field(ui, "timeout", cur, "never", 80.0, false) {
            match text.trim() {
                "" => s.save_settings(json!({ "interactionTimeoutMs": Value::Null })),
                x => match x.parse::<u64>() {
                    Ok(m) if m > 0 => s.save_settings(json!({ "interactionTimeoutMs": m * 60_000 })),
                    _ => s.note_settings(Err(format!("“{x}” is not a number of minutes"))),
                },
            }
        }
        ui.label(dim(&t, "minutes"));
    });
}

fn harnesses(ui: &mut Ui, v: &View, s: &mut State, st: &Settings) {
    let t = v.t;
    ui.horizontal(|ui| {
        ui.label(dim(&t, "Readiness is probed by the engine. Recheck re-runs every probe."));
        if ui.add_enabled(s.client.is_some(), egui::Button::new("Recheck")).clicked() {
            s.recheck_harnesses();
        }
    });
    let list = s.harnesses.clone();
    for h in list {
        let (color, word) = match h.status.as_str() {
            "ok" => (t.success, "Ok"),
            "degraded" => (t.blocked, "Degraded"),
            _ => (t.failed, "Unavailable"),
        };
        Frame::new().fill(t.raised).corner_radius(R_SM).inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                super::chip(ui, &t, h.label(), t.harness(&h.id));
                ui.label(RichText::new(word).size(T_SMALL).color(color));
                if !h.routable_intents.is_empty() {
                    ui.label(dim(&t, format!("routable for {}", h.routable_intents.len())));
                }
            });
            for r in &h.doctor_rows() {
                let (g, c) = match r.status.as_str() {
                    "pass" => (icons::CIRCLE_CHECK, t.success),
                    "fail" => (icons::CIRCLE_X, t.failed),
                    _ => (icons::CIRCLE, t.text3),
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(g).size(T_SMALL).color(c));
                    ui.label(RichText::new(&r.title).size(T_SMALL).color(t.text));
                    if let Some(d) = &r.detail {
                        ui.add(egui::Label::new(dim(&t, d)).wrap());
                    }
                });
            }
            for a in h.auth_sources.iter().filter(|a| a.availability == "available") {
                ui.label(dim(&t, format!("auth: {} · {}", a.source.replace('_', " "), a.detail.as_deref().unwrap_or("available"))));
            }
            for r in &h.reasons {
                ui.add(egui::Label::new(RichText::new(r).size(T_SMALL).color(t.text2)).wrap());
            }
            if h.status == "unavailable" {
                return;
            }
            // per-harness defaults
            let hs = st.harnesses.get(&h.id).cloned().unwrap_or_default();
            ui.horizontal_wrapped(|ui| {
                let mut en = hs.enabled.unwrap_or(true);
                ui.label(RichText::new("Enabled").size(T_SMALL).color(t.text2));
                if super::toggle_switch(ui, &t, &mut en).changed() {
                    s.save_settings(json!({"harnesses": { h.id.clone(): {"enabled": en} }}));
                }
                s.load_models(&h.id);
                let models: Vec<String> = match s.models.get(&h.id) {
                    Some(Ok(l)) => l.models.iter().map(|m| m.id.clone()).collect(),
                    _ => vec![],
                };
                let cur = hs.default_model.clone();
                egui::ComboBox::from_id_salt(("default-model", &h.id))
                    .selected_text(cur.clone().unwrap_or_else(|| "Harness default model".into()))
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(cur.is_none(), "Harness default model").clicked() && cur.is_some() {
                            s.save_settings(json!({"harnesses": { h.id.clone(): {"defaultModel": Value::Null} }}));
                        }
                        for m in &models {
                            if ui.selectable_label(cur.as_deref() == Some(m), m).clicked() && cur.as_deref() != Some(m) {
                                s.save_settings(json!({"harnesses": { h.id.clone(): {"defaultModel": m} }}));
                            }
                        }
                    });
                let levels = h.effort_levels(cur.as_deref());
                if !levels.is_empty() {
                    let e = hs.effort.clone();
                    egui::ComboBox::from_id_salt(("default-effort", &h.id)).selected_text(e.clone().unwrap_or_else(|| "Default effort".into())).show_ui(ui, |ui| {
                        if ui.selectable_label(e.is_none(), "Default effort").clicked() && e.is_some() {
                            s.save_settings(json!({"harnesses": { h.id.clone(): {"effort": Value::Null} }}));
                        }
                        for l in &levels {
                            if ui.selectable_label(e.as_deref() == Some(l), l).clicked() && e.as_deref() != Some(l) {
                                s.save_settings(json!({"harnesses": { h.id.clone(): {"effort": l} }}));
                            }
                        }
                    });
                }
            });
        });
    }
}

fn keys(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    heading(ui, v, "API keys", "Stored by the engine in its owner-only secret file. Keys are write-only here: a stored key is never shown again.");
    let secrets = s.secrets.clone().unwrap_or_default();
    let stored = |slot: &str| secrets.iter().any(|x| x.name == slot && x.present);
    let mut slots: Vec<(&'static str, String)> = s
        .harnesses
        .iter()
        .filter_map(|h| secret_slot(&h.id).filter(|slot| h.status != "unavailable" || stored(slot)).map(|slot| (slot, h.label().to_string())))
        .collect();
    slots.dedup_by_key(|x| x.0);
    for (slot, label) in slots {
        let present = secrets.iter().any(|x| x.name == slot && x.present);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{label} ({slot})")).size(T_SMALL).color(t.text));
            if present {
                ui.label(RichText::new("stored").size(T_SMALL).color(t.success));
                if ui.small_button("Remove").clicked() {
                    s.delete_secret(slot);
                }
            }
            let hint = if present { "replace key…" } else { "paste key, Enter to save" };
            if let Some(val) = text_field(ui, ("secret", slot), String::new(), hint, 220.0, true) {
                if !val.trim().is_empty() {
                    s.set_secret(slot, val.trim().to_string());
                }
            }
        });
    }
    for x in secrets.iter().filter(|x| s.harnesses.iter().all(|h| secret_slot(&h.id) != Some(x.name.as_str()))) {
        ui.label(dim(&t, format!("{} · {}", x.name, if x.present { "stored" } else { "empty" })));
    }

    heading(ui, v, "Full-access grants", "Projects where agents may run without a sandbox.");
    let grants: Vec<_> = s.trust_all.clone().unwrap_or_default().into_iter().filter(|g| g.allow_full_access).collect();
    if grants.is_empty() {
        ui.label(dim(&t, "None. Every project runs sandboxed."));
    }
    for g in grants {
        ui.horizontal(|ui| match &g.repo_root {
            Some(root) => {
                ui.add(egui::Label::new(RichText::new(root).monospace().size(T_SMALL).color(t.text)).truncate());
                if ui.add(egui::Button::new(RichText::new("Revoke").color(t.failed))).clicked() {
                    s.revoke_full_access(root);
                }
            }
            None => {
                ui.label(dim(&t, "A legacy grant without a recorded root: run `claudexor trust --revoke-full-access` in that repo."));
            }
        });
    }
}

fn engine(ui: &mut Ui, v: &View, s: &mut State, st: Option<&Settings>) {
    let t = v.t;
    super::group(ui, &t, |ui| {
        ui.label(RichText::new(format!("Claudexor for Linux {} · MIT", env!("CARGO_PKG_VERSION"))).size(T_SMALL).color(t.text));
        match &s.engine_version {
            Some(ver) => ui.label(dim(&t, format!("Engine {ver} · protocol {}", crate::api::PROTOCOL_MAJOR))),
            None => ui.label(dim(&t, "Engine not connected")),
        };
    });
    heading(ui, v, "Keyboard", "");
    super::group(ui, &t, |ui| {
        for (k, what) in [
            ("Enter", "Send"),
            ("Shift+Enter", "New line"),
            ("Ctrl+Enter", "Send, or stop a running turn"),
            ("Ctrl+N", "New thread (keeps the draft)"),
            ("Ctrl+K", "Search threads"),
            ("Alt+↑ / Alt+↓", "Previous / next thread"),
            ("Ctrl+.", "Toggle the workspace panel"),
        ] {
            super::row(ui, &t, k, |ui| ui.label(RichText::new(what).size(T_SMALL).color(t.text)));
        }
    });
    let Some(c) = st.and_then(|st| st.runtime.as_ref()).and_then(|r| r.concurrency.as_ref()) else { return };
    heading(ui, v, "Concurrency", "Set in the engine's config file; shown here read-only.");
    super::group(ui, &t, |ui| {
        egui::Grid::new("concurrency").num_columns(3).spacing([24.0, 4.0]).show(ui, |ui| {
            ui.label(dim(&t, ""));
            ui.label(dim(&t, "configured"));
            ui.label(dim(&t, "in effect"));
            ui.end_row();
            for (name, a, b) in [
                ("Concurrent runs", c.configured.max_concurrent, c.effective.max_concurrent),
                ("Parallel candidates", c.configured.max_parallel_candidates, c.effective.max_parallel_candidates),
                ("Deep-scan width", c.configured.max_deep_scan_width, c.effective.max_deep_scan_width),
                ("Council members", c.configured.max_council_members, c.effective.max_council_members),
            ] {
                ui.label(RichText::new(name).size(T_SMALL));
                ui.label(RichText::new(a.to_string()).size(T_SMALL));
                ui.label(RichText::new(b.to_string()).size(T_SMALL).color(if a == b { t.text } else { t.blocked }));
                ui.end_row();
            }
        });
        if c.restart_required {
            ui.label(RichText::new("Restart the engine to apply the configured values.").size(T_SMALL).color(t.blocked));
        }
    });
}
