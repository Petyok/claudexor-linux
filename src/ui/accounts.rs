//! Accounts & quota popover: quota is read-only + Refresh; Log in runs the
//! engine's in-app native login. Every quota figure is a projection
//! of `/v2/quota` + `/v2/account-pools`: unknown usage stays "unknown" (never
//! 0%), stale data is labelled stale, typed absences say why they are absent.

use super::glass::Kind;
use super::theme::{R_MD, R_SM, SP, T_BODY, T_SMALL, Theme, semibold, until};
use super::{View, dim, last_rect, store_rect};
use crate::model::{QuotaConstraint, QuotaSubject};
use crate::state::State;
use egui::{Area, Frame, Id, Margin, Order, Pos2, RichText, ScrollArea, Sense, Stroke, Ui, vec2};
use std::collections::BTreeSet;

/// Returns false when the popover should close (click outside / Esc).
pub fn show(ctx: &egui::Context, v: &mut View, s: &mut State, anchor_bottom_left: Pos2) -> bool {
    let t = v.t;
    let id = Id::new("accounts-pop");
    let mut keep = true;
    let area = Area::new(id).order(Order::Foreground).fixed_pos(anchor_bottom_left).pivot(egui::Align2::LEFT_BOTTOM).show(ctx, |ui| {
        if let Some(r) = last_rect(ui, id) {
            v.glass.surface(ui, r, R_MD, Kind::Popover);
        }
        let resp = Frame::new().inner_margin(Margin::same(14)).show(ui, |ui| {
            ui.set_width(420.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Accounts & quota").family(semibold()).size(T_BODY + 1.0).color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if s.quota_loading {
                        super::spinner(ui, 14.0, egui::Color32::GRAY);
                    } else if ui
                        .add_enabled(s.client.is_some(), egui::Button::new(RichText::new(format!("{} Refresh", super::icons::REFRESH_CW)).size(T_SMALL)))
                        .on_hover_text("Ask every vendor for a fresh reading")
                        .clicked()
                    {
                        s.refresh_quota(true);
                    }
                });
            });
            if let Some(q) = &s.quota {
                if let Some(at) = &q.refreshed_at {
                    ui.label(dim(&t, format!("refreshed {}", super::theme::ago(at))));
                }
            }
            if let Some(e) = &s.quota_error {
                ui.label(RichText::new(e).color(t.failed).size(T_SMALL));
            }
            if s.login.is_some() {
                ui.add_space(SP);
                login_panel(ui, &t, s);
            }
            ui.add_space(SP);
            Frame::new().fill(t.overlay).corner_radius(R_SM).inner_margin(Margin::same(10)).stroke(Stroke::new(1.0_f32, t.separator)).show(
                ui,
                |ui| {
                    ui.set_width(ui.available_width());
                    // a real minimum: without one egui kept the first frame's ~130 px and clipped the rows
                    let room = (anchor_bottom_left.y - 180.0).clamp(160.0, 460.0);
                    // bar pinned visible: an auto bar flip-flopped with the rows' wrap width (repaint loop)
                    ScrollArea::vertical()
                        .max_height(room)
                        .min_scrolled_height(room)
                        // fixed height: shrink-to-content let the viewport and the bar flip each frame
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                        .show(ui, |ui| body(ui, &t, s));
                },
            );
        });
        store_rect(ui, id, resp.response.rect);
        resp.response
    });
    let clicked_outside =
        ctx.input(|i| i.pointer.any_click()) && !area.response.rect.contains(ctx.input(|i| i.pointer.interact_pos().unwrap_or_default()));
    if clicked_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        keep = false;
    }
    let _ = area.response.interact(Sense::hover());
    keep
}

/// What the rows asked for; applied after drawing (rows only borrow State).
enum AcctAct {
    Login(String, Option<String>),
    Enable(String, String, bool),
    Remove(String, String),
    Add(String, String),
    CancelLogin(String),
}

fn body(ui: &mut Ui, t: &Theme, s: &mut State) {
    let mut act = None;
    body_rows(ui, t, s, &mut act);
    add_account_row(ui, t, s, &mut act);
    match act {
        Some(AcctAct::Login(h, p)) => s.start_login_for(&h, p),
        Some(AcctAct::Enable(h, p, on)) => s.set_profile_enabled(&h, &p, on),
        Some(AcctAct::Remove(h, p)) => s.delete_profile(&h, &p),
        Some(AcctAct::CancelLogin(job)) => s.cancel_login_job(&job),
        Some(AcctAct::Add(h, name)) => s.add_account(&h, &name),
        None => {}
    }
}

fn in_app_login(s: &State, harness: &str) -> bool {
    s.harnesses.iter().find(|x| x.id == harness).and_then(|x| x.setup_login.as_ref()).is_some_and(|l| l.mode == "in_app")
}

/// One section per harness: its accounts (credential profiles) with readiness,
/// Enabled toggle, Log in / Remove, and the account's own quota windows.
fn body_rows(ui: &mut Ui, t: &Theme, s: &State, act: &mut Option<AcctAct>) {
    let profiles: Vec<&crate::model::ProfileRow> = s.profiles.as_ref().map(|p| p.profiles.iter().collect()).unwrap_or_default();
    let quota = s.quota.as_ref();
    let mut harnesses: BTreeSet<String> = profiles.iter().map(|r| r.profile.harness_id.clone()).collect();
    if let Some(q) = quota {
        harnesses.extend(q.snapshots.iter().map(|x| x.subject.harness.clone()));
        harnesses.extend(q.absences.iter().map(|x| x.subject.harness.clone()));
    }
    harnesses.extend(s.pools.iter().map(|p| p.harness_id.clone()));
    if harnesses.is_empty() || (s.profiles.is_none() && s.client.is_some()) {
        ui.horizontal(|ui| {
            if s.client.is_some() {
                super::spinner(ui, 12.0, t.text2);
            }
            ui.label(dim(t, if s.client.is_some() { "Loading accounts…" } else { "Engine offline" }));
        });
        return;
    }
    let busy = s.login.as_ref().is_some_and(|l| l.active());
    for h in harnesses {
        let rows: Vec<_> = profiles.iter().filter(|r| r.profile.harness_id == h).collect();
        let installed = s.harnesses.iter().find(|x| x.id == h).is_some_and(|x| x.setup_login.is_some() || x.status == "ok");
        ui.horizontal(|ui| {
            ui.label(RichText::new(&h).color(t.harness(&h)).family(semibold()).size(T_BODY));
            if let Some(p) = s.pools.iter().find(|p| p.harness_id == h) {
                let r = ui.label(dim(t, format!("next up: {}", p.next_up.describe())));
                if let Some(why) = &p.next_up.reason {
                    r.on_hover_text(why);
                }
            }
            // A harness with no account row yet: its bootstrap login.
            if rows.is_empty() && in_app_login(s, &h) {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let b =
                        egui::Button::new(RichText::new("Log in").size(T_SMALL).color(t.on_accent)).fill(t.accent_solid).corner_radius(8);
                    if ui.add_enabled(!busy && s.client.is_some(), b).on_disabled_hover_text("A sign-in is already in progress").clicked() {
                        *act = Some(AcctAct::Login(h.clone(), None));
                    }
                });
            }
        });
        if !installed && rows.is_empty() {
            if let Some(hs) = s.harnesses.iter().find(|x| x.id == h) {
                if let Some(r) = hs.reasons.first() {
                    ui.add(egui::Label::new(dim(t, r)).wrap());
                }
            }
        }
        for r in &rows {
            account_row(ui, t, s, r, busy, act);
        }
        // Quota subjects that belong to no listed account (e.g. before the registry loads).
        if let Some(q) = quota {
            let listed: Vec<&str> = rows.iter().map(|r| r.profile.profile_id.as_str()).collect();
            for sn in q.accounts().into_iter().filter(|x| x.subject.harness == h && !listed.contains(&x.subject.account_key().2.as_str())) {
                ui.label(RichText::new(subject_label(&sn.subject)).color(t.text2).size(T_SMALL + 1.0));
                for c in &sn.constraints {
                    constraint(ui, t, c);
                }
            }
        }
        ui.add_space(2.0 * SP);
    }
}

fn account_row(ui: &mut Ui, t: &Theme, s: &State, r: &crate::model::ProfileRow, busy: bool, act: &mut Option<AcctAct>) {
    let (h, id) = (r.profile.harness_id.clone(), r.profile.profile_id.clone());
    let ready = r.ready();
    ui.horizontal(|ui| {
        let dot = if !r.profile.enabled {
            t.text3
        } else if ready {
            t.success
        } else {
            t.blocked
        };
        let (rect, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, dot);
        ui.add(egui::Label::new(RichText::new(r.label()).color(t.text).size(T_SMALL + 1.0)).truncate()).on_hover_text(format!(
            "{id}{}",
            r.status.as_ref().and_then(|x| x.detail.as_deref()).map(|d| format!("\n{d}")).unwrap_or_default()
        ));
        if let Some(plan) = r.identity.as_ref().and_then(|i| i.plan.as_deref()) {
            ui.label(dim(t, plan.replace('_', " ")));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Remove: two clicks (arm, confirm) — deletes the binding + Claudexor-owned state.
            let arm = Id::new(("remove-account", &id));
            let armed: bool = ui.ctx().data(|d| d.get_temp(arm)).unwrap_or(false);
            let label = if armed { RichText::new("Confirm remove").color(t.failed) } else { RichText::new("Remove").color(t.text3) };
            if ui.add(egui::Button::new(label.size(T_SMALL - 1.0)).frame(false)).on_hover_text("Delete this account binding").clicked() {
                if armed {
                    ui.ctx().data_mut(|d| d.remove::<bool>(arm));
                    *act = Some(AcctAct::Remove(h.clone(), id.clone()));
                } else {
                    ui.ctx().data_mut(|d| d.insert_temp(arm, true));
                }
            }
            // a login already in flight (possibly started by the CLI): continue it or cancel it
            if let Some(job) = s.pending_login(&h, &id) {
                if ui.add_enabled(s.client.is_some(), egui::Button::new(RichText::new("Cancel").size(T_SMALL))).on_hover_text(format!("Cancel sign-in {}", job.job_id)).clicked() {
                    *act = Some(AcctAct::CancelLogin(job.job_id.clone()));
                }
                let b = egui::Button::new(RichText::new("Continue sign-in").size(T_SMALL).color(t.on_accent)).fill(t.accent_solid).corner_radius(8);
                if ui.add_enabled(!busy && s.client.is_some(), b).on_hover_text("Show the sign-in link for this pending login").clicked() {
                    *act = Some(AcctAct::Login(h.clone(), Some(id.clone())));
                }
            } else if !ready && in_app_login(s, &h) {
                // offered for switched-off accounts too: signing in and routing are separate
                let b = egui::Button::new(RichText::new("Log in").size(T_SMALL).color(t.on_accent)).fill(t.accent_solid).corner_radius(8);
                if ui.add_enabled(!busy && s.client.is_some(), b).on_disabled_hover_text("A sign-in is already in progress").clicked() {
                    *act = Some(AcctAct::Login(h.clone(), Some(id.clone())));
                }
            }
            let mut on = r.profile.enabled;
            if super::toggle_switch(ui, t, &mut on).on_hover_text("Enabled: include this account in the routing pool").changed() {
                *act = Some(AcctAct::Enable(h.clone(), id.clone(), on));
            }
        });
    });
    if let Some(q) = &s.quota {
        for sn in q.accounts().into_iter().filter(|x| x.subject.harness == h && x.subject.account_key().2 == id) {
            for c in &sn.constraints {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    constraint(ui, t, c);
                });
            }
            if let Some(a) = &sn.availability {
                if a.state != "available" {
                    ui.label(
                        RichText::new(format!("   {} {}", a.state, a.resets_at.as_deref().map(until).unwrap_or_default()))
                            .color(t.failed)
                            .size(T_SMALL - 1.0),
                    );
                }
            }
        }
        for a in q.account_absences().into_iter().filter(|x| x.subject.harness == h && x.subject.account_key().2 == id) {
            ui.label(RichText::new(format!("   {}", a.reason.replace('_', " "))).color(t.blocked).size(T_SMALL - 1.0));
        }
    }
}

/// "Add account": a named account row for a harness, then its login at once.
fn add_account_row(ui: &mut Ui, t: &Theme, s: &State, act: &mut Option<AcctAct>) {
    let hs: Vec<String> =
        s.harnesses.iter().filter(|x| x.setup_login.as_ref().is_some_and(|l| l.mode == "in_app")).map(|x| x.id.clone()).collect();
    if hs.is_empty() || s.client.is_none() {
        return;
    }
    let key = Id::new("add-account");
    let (mut harness, mut name): (String, String) = ui.ctx().data(|d| d.get_temp(key)).unwrap_or_else(|| (hs[0].clone(), String::new()));
    ui.separator();
    ui.label(RichText::new("Add another account").color(t.text2).size(T_SMALL));
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("add-account-harness").selected_text(&harness).width(80.0).show_ui(ui, |ui| {
            for h in &hs {
                ui.selectable_value(&mut harness, h.clone(), h);
            }
        });
        ui.add(egui::TextEdit::singleline(&mut name).hint_text("name, e.g. work").desired_width(120.0));
        let busy = s.login.as_ref().is_some_and(|l| l.active());
        let b = egui::Button::new(RichText::new("Add & log in").size(T_SMALL).color(t.on_accent)).fill(t.accent_solid).corner_radius(8);
        if ui
            .add_enabled(!busy && !name.trim().is_empty(), b)
            .on_disabled_hover_text("Name the account (and finish any sign-in in progress)")
            .clicked()
        {
            *act = Some(AcctAct::Add(harness.clone(), name.clone()));
            name.clear();
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(key, (harness, name)));
}

/// The in-app sign-in: link (+ code) from the job snapshot, a paste field for
/// `oauth_url_input` flows, live phase, and a typed terminal outcome.
fn login_panel(ui: &mut Ui, t: &Theme, s: &mut State) {
    let mut submit = false;
    let mut cancel = false;
    let mut close = false;
    let mut extend = false;
    let mut retry = false;
    let mut open: Option<String> = None;
    let Some(l) = s.login.as_mut() else { return };
    Frame::new().fill(t.raised_hi).corner_radius(R_SM).inner_margin(Margin::same(10)).stroke(Stroke::new(1.5_f32, t.accent)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Sign in to {}", l.harness)).family(semibold()).size(T_BODY).color(t.harness(&l.harness)));
                if let Some(j) = &l.job {
                    ui.label(dim(t, format!("{} · {}", j.phase.replace('_', " "), j.state.replace('_', " "))));
                }
            });
            let job = l.job.clone();
            match &job {
                None if l.error.is_none() => {
                    ui.horizontal(|ui| {
                        super::spinner(ui, 14.0, egui::Color32::GRAY);
                        ui.label(dim(t, "Starting the login…"));
                    });
                }
                Some(j) if j.terminal() => {
                    let ok = j.state == "succeeded";
                    let (c, g) = if ok { (t.success, super::icons::CIRCLE_CHECK) } else { (t.failed, super::icons::CIRCLE_X) };
                    let why = j.outcome.as_ref().map(|o| o.reason.replace('_', " ")).unwrap_or_else(|| j.state.replace('_', " "));
                    ui.label(RichText::new(if ok { format!("{g} Signed in") } else { format!("{g} {why}") }).color(c).size(T_SMALL + 1.0));
                    if !j.message.is_empty() {
                        ui.add(egui::Label::new(dim(t, &j.message)).wrap().selectable(true));
                    }
                }
                Some(j) => {
                    if !j.message.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&j.message).color(t.text2).size(T_SMALL)).wrap().selectable(true));
                    }
                    match &l.disclosure {
                        Some(d) if !d.verification_url.is_empty() => {
                            ui.label(RichText::new("1. Open the sign-in page").color(t.text).size(T_SMALL + 1.0));
                            ui.horizontal(|ui| {
                                let open_btn = egui::Button::new(RichText::new("Open in browser").size(T_SMALL).color(t.on_accent))
                                    .fill(t.accent_solid)
                                    .corner_radius(8);
                                if ui.add(open_btn).clicked() {
                                    open = Some(d.verification_url.clone());
                                }
                                if ui.button(RichText::new("Copy link").size(T_SMALL)).clicked() {
                                    ui.ctx().copy_text(d.verification_url.clone());
                                }
                            });
                            ui.add(
                                egui::Label::new(RichText::new(&d.verification_url).monospace().size(T_SMALL - 1.0).color(t.text3))
                                    .truncate()
                                    .selectable(true),
                            )
                            .on_hover_text(&d.verification_url);
                            if !d.user_code.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.label(dim(t, "Code to enter:"));
                                    ui.label(RichText::new(&d.user_code).monospace().size(T_BODY).color(t.text));
                                    if ui.small_button("Copy").clicked() {
                                        ui.ctx().copy_text(d.user_code.clone());
                                    }
                                });
                            }
                            if d.flow == "oauth_url_input" {
                                if l.code_sent {
                                    ui.horizontal(|ui| {
                                        super::spinner(ui, 14.0, egui::Color32::GRAY);
                                        ui.label(dim(t, "Code sent, verifying…"));
                                    });
                                } else {
                                    ui.label(RichText::new("2. Paste the code the page shows you").color(t.text).size(T_SMALL + 1.0));
                                    ui.horizontal(|ui| {
                                        let r = ui.add(
                                            egui::TextEdit::singleline(&mut l.code).hint_text("authorization code").desired_width(230.0),
                                        );
                                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                        if ui.add_enabled(!l.code.trim().is_empty(), egui::Button::new("Submit")).clicked() || enter {
                                            submit = true;
                                        }
                                    });
                                }
                            }
                        }
                        _ => {
                            ui.horizontal(|ui| {
                                super::spinner(ui, 14.0, egui::Color32::GRAY);
                                ui.label(dim(t, "Waiting for the sign-in link (the vendor CLI takes ~20 s to start)…"));
                            });
                        }
                    }
                    if let Some(at) = &j.deadline_at {
                        ui.horizontal(|ui| {
                            ui.label(dim(t, format!("expires {}", until(at))));
                            if !j.deadline_fixed
                                && ui.small_button("Extend 15 min").on_hover_text("More time to finish signing in").clicked()
                            {
                                extend = true;
                            }
                        });
                    }
                }
                None => {}
            }
            if let Some(e) = &l.error {
                ui.add(egui::Label::new(RichText::new(e).color(t.failed).size(T_SMALL)).wrap().selectable(true));
            }
            ui.horizontal(|ui| {
                if l.active() && l.job.is_some() {
                    if ui.button(RichText::new("Cancel sign-in").size(T_SMALL)).clicked() {
                        cancel = true;
                    }
                } else {
                    let failed = l.error.is_some() || l.job.as_ref().is_some_and(|j| j.terminal() && j.state != "succeeded");
                    if failed && ui.button(RichText::new("Try again").size(T_SMALL)).clicked() {
                        retry = true;
                    }
                    if ui.button(RichText::new("Close").size(T_SMALL)).clicked() {
                        close = true;
                    }
                }
            });
        },
    );
    if let Some(url) = open {
        s.open_url(&url);
    }
    if extend {
        s.extend_login();
    }
    if retry {
        let h = s.login.as_ref().map(|l| l.harness.clone());
        if let Some(h) = h {
            s.start_login(&h);
        }
    }
    if submit {
        s.submit_login_code();
    }
    if cancel {
        s.cancel_login();
    }
    if close {
        s.login = None;
    }
}

fn subject_label(s: &QuotaSubject) -> String {
    let mut parts = vec![];
    if let Some(id) = &s.subject_id {
        parts.push(id.clone());
    }
    if let Some(p) = &s.plan_label {
        parts.push(p.clone());
    }
    if let Some(r) = &s.credential_route {
        parts.push(r.replace('_', " "));
    }
    if parts.is_empty() { "default".into() } else { parts.join(" · ") }
}

fn constraint(ui: &mut Ui, t: &Theme, c: &QuotaConstraint) {
    ui.horizontal(|ui| {
        ui.add_sized(vec2(90.0, 16.0), egui::Label::new(RichText::new(&c.label).color(t.text2).size(T_SMALL)).truncate());
        let (rect, _) = ui.allocate_exact_size(vec2(150.0, 8.0), Sense::hover());
        ui.painter().rect_filled(rect, 4.0, t.raised_hi);
        match c.used_ratio {
            Some(r) => {
                let r = r.clamp(0.0, 1.0) as f32;
                let col = if r >= 0.9 {
                    t.failed
                } else if r >= 0.7 {
                    t.blocked
                } else {
                    t.accent
                };
                let mut fill = rect;
                fill.set_width(rect.width() * r);
                ui.painter().rect_filled(fill, 4.0, col);
                ui.label(RichText::new(format!("{:.0}%", r * 100.0)).color(t.text).size(T_SMALL).monospace());
            }
            None => {
                ui.label(dim(t, "unknown"));
            }
        }
        if let Some(at) = &c.resets_at {
            ui.label(dim(t, format!("resets {}", until(at))));
        }
    });
}
