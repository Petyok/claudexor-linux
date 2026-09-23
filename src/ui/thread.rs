//! Conversation: past turns + live run timeline (PLAN §4 "Timeline").
//!
//! Each turn reads top-down: the user's bubble, then the assistant card —
//! final answer (markdown on a solid inset), ONE receipt row (status · harness
//! · state · elapsed · tools · chevron) that toggles the activity transcript,
//! question cards when the run waits on you, and the refused/failed card when
//! honest. Dense text (answer, activity, code) always sits on solids.

use super::theme::{MEASURE, R_BUBBLE, R_MD, R_SM, SP, T_BODY, T_CALLOUT, T_CAPTION, T_SMALL, T_TITLE, Theme, semibold, span};
use super::{View, chip, dim, faint, ic, icons};
use crate::model::{Answer, Interaction, RunDetail, Turn};
use crate::state::{AnswerState, Conn, Fetch, Preview, State, failure_line};
use crate::transcript::{Block, ToolStatus};
use egui::{Align, Color32, Frame, Id, Layout, Margin, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui, UiBuilder};
use std::collections::HashMap;

/// UI-local drafts for interactive answers: (interaction, question) → (picked, free text).
#[derive(Default)]
pub struct Drafts {
    pub answers: HashMap<(String, String), (Vec<String>, String)>,
    /// Plan answers: (plan run, question) → (picked option ids, own words).
    pub plan: HashMap<(String, String), (Vec<String>, String)>,
    /// Plan runs whose answers were just sent (until the follow-up turn shows up).
    pub plan_sent: std::collections::HashSet<String>,
    /// Decision-bar text per run (accepted risk or rerun feedback).
    pub decision: HashMap<String, String>,
    /// Runs whose "Override needs-human" was clicked once (awaiting the confirm).
    pub override_armed: std::collections::HashSet<String>,
}

const MAX_ROWS: usize = 80;

pub fn show(ui: &mut Ui, v: &mut View, s: &mut State, drafts: &mut Drafts, rect: Rect, top_pad: f32, bottom_pad: f32) {
    let t = v.t;
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        let col_w = (rect.width() - 8.0 * SP).min(MEASURE).max(280.0);
        // follow the bottom only while a run streams; expanding an old turn stays put
        let live = s.head_live_id().is_some();
        ScrollArea::vertical().id_salt(("conv", s.selected.clone())).stick_to_bottom(live).auto_shrink([false; 2]).show(ui, |ui| {
            let pad = ((ui.available_width() - col_w) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.vertical(|ui| {
                    ui.set_width(col_w);
                    ui.add_space(top_pad);
                    body(ui, v, s, drafts, &t);
                    ui.add_space(bottom_pad);
                    // a just-sent turn lands in view even when scrolled up
                    if std::mem::take(&mut s.scroll_to_end) {
                        ui.scroll_to_cursor(Some(Align::BOTTOM));
                    }
                });
            });
        });
    });
}

fn body(ui: &mut Ui, v: &mut View, s: &mut State, drafts: &mut Drafts, t: &Theme) {
    match (&s.conn, s.selected.is_some()) {
        (Conn::Offline(why), _) if s.detail.is_none() => {
            let why = why.clone();
            empty(ui, v, icons::WIFI_OFF, "Engine offline", &why, true);
            return engine_button(ui, v, s);
        }
        (Conn::Incompatible(why), _) => return empty(ui, v, icons::TRIANGLE_ALERT, "Incompatible engine", why, true),
        (Conn::Connecting, _) if s.detail.is_none() => {
            return empty(ui, v, icons::LOADER_CIRCLE, "Connecting…", "Looking for the local Claudexor engine.", false);
        }
        (Conn::Recovering, _) if s.detail.is_none() => {
            return empty(
                ui,
                v,
                icons::HISTORY,
                "Engine is recovering",
                "The daemon is serving its journal-recovery plane only. It opens normally once recovery completes.",
                false,
            );
        }
        (Conn::Online { .. }, false) if !s.harnesses.is_empty() && s.harnesses.iter().all(|h| h.routable_intents.is_empty()) => {
            empty(
                ui,
                v,
                icons::KEY_ROUND,
                "Set up a harness first",
                "No harness is signed in and ready to take a turn yet. Sign in to Claude, Codex, Cursor or another installed harness, and the composer comes alive.",
                false,
            );
            ui.vertical_centered(|ui| {
                ui.add_space(2.0 * SP);
                if ui.add(accent_button("Set up an account")).clicked() {
                    s.want_accounts = true;
                }
            });
            return;
        }
        (_, false) => {
            let hint = match s.composer.project.as_deref() {
                Some(root) => format!("New thread in {root}. Ask, plan, or let an agent work."),
                None => "No project selected: this thread will be Ask-only. Pick a project in the composer for Plan and Agent.".into(),
            };
            return empty(ui, v, icons::SPARKLES, "What should we work on?", &hint, false);
        }
        _ => {}
    }
    let Some(detail) = s.detail.clone() else {
        ui.add_space(8.0 * SP);
        ui.horizontal(|ui| {
            super::spinner(ui, 14.0, egui::Color32::GRAY);
            ui.label(dim(t, "Loading conversation…"));
        });
        return;
    };
    // no in-content H1 (DESIGN_SYSTEM §5.1): the title goes to the window bar
    let title = format!("{} · Claudexor", detail.thread.display_title());
    if ui.ctx().data(|d| d.get_temp::<String>(Id::new("win-title"))).as_deref() != Some(title.as_str()) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
        ui.ctx().data_mut(|d| d.insert_temp(Id::new("win-title"), title));
    }
    ui.add_space(2.0 * SP);
    if detail.turns.is_empty() {
        ui.label(dim(t, "No turns yet."));
    }
    let harness = detail.thread.primary_harness.clone();
    let n = detail.turns.len();
    for (i, turn) in detail.turns.iter().enumerate() {
        turn_card(ui, v, s, drafts, turn, &detail.turns[i + 1..], harness.as_deref(), i + 1 == n);
        ui.add_space(4.0 * SP);
    }
}

fn empty(ui: &mut Ui, v: &View, icon: &str, title: &str, text: &str, command_hint: bool) {
    let t = v.t;
    ui.add_space(ui.available_height().max(0.0) * 0.18 + 72.0);
    ui.vertical_centered(|ui| {
        ui.label(ic(icon, 32.0, t.text3));
        ui.add_space(2.0 * SP);
        ui.label(RichText::new(title).family(semibold()).size(T_TITLE + 2.0).color(t.text));
        ui.add_space(SP);
        ui.add(egui::Label::new(RichText::new(text).color(t.text2).size(T_BODY)).wrap());
        if command_hint {
            ui.add_space(3.0 * SP);
            code_line(ui, v, "claudexor daemon start");
            ui.add_space(SP);
            ui.label(dim(&t, "This window reconnects on its own."));
        }
    });
}

fn code_line(ui: &mut Ui, v: &View, text: &str) {
    Frame::new().fill(v.t.code).corner_radius(R_SM).inner_margin(Margin::symmetric(12, 6)).stroke(Stroke::new(1.0_f32, v.t.separator)).show(
        ui,
        |ui| {
            ui.add(egui::Label::new(RichText::new(text).monospace().color(v.t.text)).selectable(true));
        },
    );
}

/// Effective state word for a turn (live stream wins over the embedded card).
fn turn_state(s: &State, turn: &Turn) -> String {
    if turn.enqueue_error.is_some() && turn.run.is_none() {
        return "refused".into();
    }
    let live = s.live_of(turn);
    if live.is_some_and(|l| l.waiting)
        || turn.run.as_ref().and_then(|r| r.waiting_on_user) == Some(true) && live.is_none_or(|l| l.terminal.is_none())
    {
        return "needs_you".into();
    }
    // the embedded card is authoritative once it reports terminal
    if let Some(r) = &turn.run {
        if !matches!(r.state.as_str(), "queued" | "running" | "") {
            return r.state.clone();
        }
    }
    match live.and_then(|l| l.terminal.as_deref()) {
        Some("run.completed") => "succeeded".into(),
        Some("run.blocked") => "blocked".into(),
        Some("run.failed") => "failed".into(),
        Some(_) => "succeeded".into(),
        None => live.map(|l| l.phase.clone()).or_else(|| turn.run.as_ref().map(|r| r.state.clone())).unwrap_or_else(|| "queued".into()),
    }
}

fn turn_card(
    ui: &mut Ui,
    v: &mut View,
    s: &mut State,
    drafts: &mut Drafts,
    turn: &Turn,
    later: &[Turn],
    harness: Option<&str>,
    is_head: bool,
) {
    let t = v.t;
    // 1. user bubble, right-aligned, tinted by hue (not only alignment)
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        let max = ui.available_width() * 0.82;
        Frame::new().fill(t.user_bubble).corner_radius(R_BUBBLE).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_max_width(max);
            // text reads left-to-right inside a right-aligned bubble
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.add(egui::Label::new(RichText::new(&turn.prompt).color(t.text).size(T_BODY)).wrap().selectable(true));
            });
        });
    });
    ui.add_space(2.0 * SP);

    let state = turn_state(s, turn);
    let active = matches!(state.as_str(), "queued" | "running" | "needs_you");
    let terminal = !active && state != "refused";
    if terminal {
        if let Some(rid) = &turn.run_id {
            s.ensure_answer(rid);
        }
    }

    // 2. assistant card: background reserved now, sized after layout (same frame)
    let slot = v.glass.card_slot(ui);
    let resp = Frame::new().inner_margin(Margin::same(14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = SP;
        // refused turn: typed problem + retry
        if let Some(err) = &turn.enqueue_error {
            if turn.run.is_none() {
                refused(ui, v, s, drafts, turn, err);
                return;
            }
        }
        // final answer
        if let Some(rid) = &turn.run_id {
            // no answer file: the typed final message of the stream is the answer
            let final_text = s.live_of(turn).and_then(|l| l.transcript.final_text.clone());
            match s.answers.get(rid).cloned() {
                Some(AnswerState::Ready(text)) => answer_bubble(ui, v, &text, &turn.id),
                Some(AnswerState::None) if terminal => match final_text {
                    Some(text) => answer_bubble(ui, v, &text, &turn.id),
                    None => s.ensure_replay(rid),
                },
                Some(AnswerState::Loading) if terminal && final_text.is_some() => {
                    answer_bubble(ui, v, final_text.as_deref().unwrap_or_default(), &turn.id);
                }
                Some(AnswerState::Loading) if terminal => {
                    ui.horizontal(|ui| {
                        super::spinner(ui, 12.0, v.t.text2);
                        ui.label(dim(&t, "Loading answer…"));
                    });
                }
                Some(AnswerState::Failed(e)) => {
                    ui.label(RichText::new(format!("Could not load the answer: {e}")).color(t.failed).size(T_SMALL));
                }
                _ => {}
            }
            let failure = s.details.get(rid).and_then(|d| d.summary.failure.clone());
            if let (Some(f), true) = (failure, matches!(state.as_str(), "failed" | "cancelled" | "interrupted")) {
                failure_card(ui, &t, &f);
            }
        }
        // 3. receipt row
        let expanded_key = turn.id.clone();
        let expanded = if active { !s.collapsed.contains(&expanded_key) } else { s.expanded.contains(&expanded_key) };
        match receipt(ui, v, s, turn, &state, harness, expanded) {
            ReceiptClick::Toggle => {
                if active {
                    if !s.collapsed.remove(&expanded_key) {
                        s.collapsed.insert(expanded_key.clone());
                    }
                } else if !s.expanded.remove(&expanded_key) {
                    s.expanded.insert(expanded_key.clone());
                }
            }
            ReceiptClick::Workspace => {
                s.ws_open = true;
                s.ws_run = turn.run_id.clone();
            }
            ReceiptClick::None => {}
        }
        if let Some(l) = s.live_of(turn) {
            for n in &l.notes {
                ui.horizontal(|ui| {
                    ui.label(ic(icons::ARROW_RIGHT_LEFT, 12.0, t.needs_you));
                    ui.add(egui::Label::new(RichText::new(n).size(T_SMALL).color(t.text2)).wrap());
                });
            }
        }
        // 3b. what the card keeps inline (DESIGN_SYSTEM §5 turn card): one line
        // of applied changes, sub-runs, the decision bar. Detail lives in the workspace.
        if let Some(d) = turn.run_id.as_ref().and_then(|r| s.details.get(r)).cloned() {
            // only when files actually changed (an Agent run can end with an empty patch)
            let changed = d.summary.result.as_ref().or(turn.run.as_ref().and_then(|r| r.result.as_ref())).and_then(|r| r.diff_stat.as_ref()).is_some_and(|x| x.files > 0);
            if changed && (d.has_patch() || d.summary.result.as_ref().is_some_and(|r| r.revertable)) {
                changes_line(ui, v, s, &d, turn);
            }
            for c in &d.children {
                sub_run_row(ui, &t, c);
            }
            if d.needs_decision() {
                let rid = d.summary.run_id.clone().or_else(|| turn.run_id.clone()).unwrap_or_default();
                let online = s.client.is_some() && !s.run_busy.contains(&rid);
                decision_bar(ui, v, s, drafts, &d, &rid, online);
            }
        }
        // 4. activity transcript
        if expanded {
            if !active {
                if let Some(rid) = &turn.run_id {
                    s.ensure_replay(rid);
                }
            }
            activity(ui, v, s, turn);
        }
        // 5. pending questions
        if state == "needs_you" {
            if let Some(rid) = &turn.run_id {
                if let Some(pending) = s.details.get(rid).map(|d| d.pending_interactions.clone()) {
                    for it in pending {
                        question_card(ui, v, s, drafts, &it);
                    }
                }
            }
        }
        // 6. plan lifecycle (questions, Implement) and plan provenance receipts
        plan_section(ui, v, s, drafts, turn, later, &state);
        let _ = is_head;
    });
    v.glass.fill_card(ui, slot, resp.response.rect, R_MD);
}

fn failure_card(ui: &mut Ui, t: &Theme, f: &crate::model::RunFailure) {
    Frame::new().fill(t.failed.gamma_multiply(0.12)).corner_radius(R_SM).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(ic(icons::CIRCLE_X, 14.0, t.failed));
            if let Some(line) = failure_line(f) {
                ui.add(egui::Label::new(RichText::new(line).color(t.text).size(T_CALLOUT)).wrap().selectable(true));
            }
        });
        for a in &f.next_actions {
            ui.label(RichText::new(format!("• {a}")).color(t.text2).size(T_SMALL));
        }
        if let Some(at) = &f.resets_at {
            ui.label(dim(t, format!("Window reopens {}", super::theme::until(at))));
        }
        if let Some(v) = &f.vendor_failure {
            let parts: Vec<&str> = [v.code.as_deref(), v.message.as_deref()].into_iter().flatten().collect();
            if !parts.is_empty() {
                ui.label(faint(t, format!("vendor: {}", parts.join(" · "))));
            }
        }
    });
}

/// One line for a run's changes: state + diffstat, Apply when the gate allows,
/// and a link into the workspace for the diff, revert and evidence.
fn changes_line(ui: &mut Ui, v: &View, s: &mut State, d: &RunDetail, turn: &Turn) {
    let t = v.t;
    let Some(rid) = d.summary.run_id.clone().or_else(|| turn.run_id.clone()) else { return };
    let result = d.summary.result.clone().or_else(|| turn.run.as_ref().and_then(|r| r.result.clone()));
    let apply_state = result.as_ref().and_then(|r| r.apply_state.clone()).unwrap_or_else(|| "not_applied".into());
    let eligible = d.apply_eligibility.as_ref().is_some_and(|e| e.eligible) && apply_state == "not_applied";
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(ic(icons::GIT_BRANCH, 13.0, t.text2));
        let word = match apply_state.as_str() {
            "applied" => "Changes applied",
            "applied_review_blocked" => "Applied, review blocked",
            "reverted" => "Changes reverted",
            "discarded" => "Changes discarded",
            _ => "Changes ready",
        };
        ui.label(RichText::new(word).size(T_SMALL).color(t.text));
        if let Some(x) = result.as_ref().and_then(|r| r.diff_stat.as_ref()).filter(|x| x.files > 0) {
            ui.label(RichText::new(format!("{} file{}", x.files, if x.files == 1 { "" } else { "s" })).size(T_SMALL).color(t.text2));
            ui.label(RichText::new(format!("+{}", x.additions)).size(T_SMALL).color(t.success));
            ui.label(RichText::new(format!("−{}", x.deletions)).size(T_SMALL).color(t.failed));
        }
        let busy = s.run_busy.contains(&rid);
        if eligible {
            if ui.add_enabled(s.client.is_some() && !busy, accent_button("Apply patch")).clicked() {
                s.apply_run(&rid, "apply");
            }
            if ui.add_enabled(s.client.is_some() && !busy, egui::Button::new(RichText::new("As branch").size(T_SMALL))).clicked() {
                s.apply_run(&rid, "branch");
            }
        }
        if busy {
            super::spinner(ui, 12.0, v.t.text2);
        }
        if ui.add(egui::Button::new(RichText::new("Review").size(T_SMALL).color(t.accent)).frame(false)).on_hover_text("Diff, revert and evidence in the workspace").clicked() {
            s.ws_open = true;
            s.ws_run = Some(rid.clone());
            s.ws_tab = 0;
        }
    });
    match s.run_note.get(&rid) {
        Some(Ok(line)) => {
            ui.label(RichText::new(line).size(T_SMALL).color(t.success));
        }
        Some(Err(e)) => {
            ui.add(egui::Label::new(RichText::new(e).size(T_SMALL).color(t.failed)).wrap().selectable(true));
        }
        None => {}
    }
}

/// A delegated child run (Mac `DelegatedRunRow`): status, id, phase word.
fn sub_run_row(ui: &mut Ui, t: &Theme, c: &crate::model::RunSummary) {
    let (color, _, word) = t.status(&c.state);
    Frame::new().fill(t.raised.gamma_multiply(0.6)).corner_radius(R_SM).inner_margin(Margin::symmetric(10, 5)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            if matches!(c.state.as_str(), "running" | "queued") {
                super::spinner(ui, 12.0, color);
            } else {
                ui.label(ic(icons::CIRCLE_DOT, 12.0, color));
            }
            ui.label(RichText::new("Delegated").size(T_CAPTION).color(t.accent));
            ui.label(RichText::new(c.run_id.as_deref().unwrap_or("sub-run")).monospace().size(T_CAPTION).color(t.text));
            ui.label(RichText::new(word).size(T_SMALL).color(t.text2));
        });
    });
}

/// Solid accent capsule for a card's primary action.
fn accent_button(label: &str) -> egui::Button<'_> {
    // accent_solid is the same in both themes' contrast checks (white on it ≥ 4.5:1)
    egui::Button::new(RichText::new(label).size(T_SMALL).color(egui::Color32::WHITE)).fill(super::theme::DARK.accent_solid).corner_radius(R_SM)
}

/// The answer: markdown on a solid inset with a flush accent edge. Long ones
/// clamp to 260 px with "Show more" (Mac: > 1200 chars or 14 lines).
fn answer_bubble(ui: &mut Ui, v: &mut View, text: &str, turn_id: &str) {
    let t = v.t;
    let key = Id::new(("answer-open", turn_id));
    let long = text.chars().count() > 1200 || text.lines().count() > 14;
    let open: bool = ui.ctx().data(|d| d.get_temp(key)).unwrap_or(false);
    let r = Frame::new().fill(t.raised_hi).corner_radius(R_SM).inner_margin(Margin { left: 16, right: 14, top: 12, bottom: 12 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if long && !open {
            ScrollArea::vertical().id_salt(key.with("clamp")).max_height(260.0)
                .min_scrolled_height(260.0)
                .scroll_source(egui::scroll_area::ScrollSource::NONE)
                // a clamp, not a scroller: a bar here flip-flopped with the text wrap and repainted forever
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                egui_commonmark::CommonMarkViewer::new().show(ui, v.md, text);
            });
        } else {
            egui_commonmark::CommonMarkViewer::new().show(ui, v.md, text);
        }
    });
    // flush, full-height accent edge (the answer is the loudest element in the feed)
    let rect = r.response.rect;
    let edge = Rect::from_min_max(rect.min, egui::pos2(rect.min.x + 3.0, rect.max.y));
    ui.painter().rect_filled(edge, egui::CornerRadius { nw: R_SM, sw: R_SM, ne: 0, se: 0 }, t.accent.gamma_multiply(0.45));
    if long && ui.add(egui::Button::new(RichText::new(if open { "Show less" } else { "Show more" }).size(T_SMALL).color(t.accent)).frame(false)).clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(key, !open));
    }
}

enum ReceiptClick {
    None,
    Toggle,
    Workspace,
}

/// ONE receipt row (Mac TurnReceiptBar): status, harness, word, attention on
/// the left; elapsed · cash · counts, workspace and chevron on the right.
fn receipt(ui: &mut Ui, v: &View, s: &State, turn: &Turn, state: &str, harness: Option<&str>, expanded: bool) -> ReceiptClick {
    let t = v.t;
    let live = s.live_of(turn);
    let mut click = ReceiptClick::None;
    let inner = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (color, _, word) = t.status(state);
        match state {
            "running" | "queued" => {
                super::spinner(ui, 12.0, color);
            }
            _ => {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.0, color);
            }
        }
        // the route that actually ran beats the live stream's harness, which beats the thread default
        let route = turn.run_id.as_ref().and_then(|r| s.details.get(r)).and_then(|d| d.summary.route.as_ref());
        let h = route.and_then(|r| r.harness_id.clone()).or_else(|| live.and_then(|l| l.transcript.harness.clone())).or(harness.map(str::to_owned));
        if let Some(h) = &h {
            chip(ui, &t, h, t.harness(h));
        }
        let word = if state == "running" { "Working…" } else { word };
        let loud = matches!(state, "failed" | "blocked" | "needs_you" | "refused" | "interrupted");
        ui.label(RichText::new(word).size(T_SMALL).color(if loud { color } else { t.text }));
        if let Some(r) = route {
            match (r.observed_model.as_deref(), r.requested_model.as_deref()) {
                (Some(obs), Some(req)) if obs != req => {
                    ui.label(RichText::new(format!("{obs} (asked {req})")).color(t.blocked).size(T_SMALL))
                        .on_hover_text("The vendor answered with a different model than requested");
                }
                (Some(obs), _) => {
                    ui.label(dim(&t, obs)).on_hover_text(if r.verified { "Model observed in the vendor stream" } else { "Model reported, not verified" });
                }
                _ => {}
            }
        }
        // collapsed live card: what it's doing now
        if !expanded {
            if let Some(a) = live.filter(|l| l.terminal.is_none()).and_then(|l| l.transcript.last_activity()) {
                ui.add(egui::Label::new(RichText::new(format!("· {a}")).size(T_SMALL).color(t.text2)).truncate());
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let chevron = if expanded { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT };
            if super::icon_button(ui, &t, chevron, if expanded { "Hide activity" } else { "Show activity" }).clicked() {
                click = ReceiptClick::Toggle;
            }
            if turn.run_id.is_some() && super::icon_button(ui, &t, icons::PANEL_RIGHT, "Open this run in the workspace").clicked() {
                click = ReceiptClick::Workspace;
            }
            let mut facts: Vec<String> = vec![];
            if let Some(l) = live {
                let n = l.transcript.tool_count();
                if n > 0 {
                    facts.push(format!("{n} tool{}", if n == 1 { "" } else { "s" }));
                }
            }
            if let Some(ds) = turn.run.as_ref().and_then(|r| r.result.as_ref()).and_then(|r| r.diff_stat.as_ref()).filter(|d| d.files > 0) {
                facts.push(format!("{} file{}", ds.files, if ds.files == 1 { "" } else { "s" }));
            }
            if let Some(usd) = turn.run.as_ref().and_then(|r| r.spend_usd).filter(|x| *x > 0.0) {
                facts.push(format!("${usd:.2}"));
            }
            // elapsed from the turn's start (not from when this window attached)
            let start = super::theme::parse_iso(&turn.created_at);
            let end = turn.run.as_ref().and_then(|r| r.finished_at.as_deref()).and_then(super::theme::parse_iso);
            let secs = match (live.filter(|l| l.terminal.is_none() && state != "queued"), start, end) {
                (Some(_), Some(a), _) => {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                    Some(super::theme::now_unix() - a)
                }
                (_, Some(a), Some(b)) => Some(b - a),
                _ => None,
            };
            if let Some(x) = secs.filter(|x| *x >= 0) {
                facts.push(span(x));
            }
            facts.reverse();
            ui.label(RichText::new(facts.join(" · ")).size(T_SMALL).color(t.text2));
        });
    });
    let r = ui.interact(inner.response.rect, ui.id().with(("receipt", &turn.id)), Sense::click());
    if r.clicked() && matches!(click, ReceiptClick::None) {
        click = ReceiptClick::Toggle;
    }
    click
}

fn activity(ui: &mut Ui, v: &mut View, s: &State, turn: &Turn) {
    let t = v.t;
    let Some(live) = s.live_of(turn) else {
        ui.label(dim(&t, "Loading activity…"));
        return;
    };
    let tr = &live.transcript;
    if tr.blocks.is_empty() {
        let msg = if live.streaming() { "Waiting for activity…" } else { "No activity recorded for this turn." };
        ui.label(dim(&t, msg));
        return;
    }
    let done = live.terminal.is_some();
    // flat rows under the receipt (Mac TranscriptView): an 8 px indent, no slab
    ui.horizontal(|ui| {
        ui.add_space(2.0 * SP);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if tr.agents > 0 {
                let open = tr.agents - tr.agents_done.min(tr.agents);
                ui.horizontal(|ui| {
                    let (icon, c) = if open > 0 && !done { (icons::LOADER_CIRCLE, t.running) } else { (icons::CIRCLE_CHECK, t.success) };
                    ui.label(ic(icon, 13.0, c));
                    let n = tr.agents;
                    let finished = if done { n } else { tr.agents_done.min(n) };
                    ui.label(RichText::new(format!("{n} subagent{} · {finished} finished", if n == 1 { "" } else { "s" })).size(T_SMALL).color(t.text));
                })
                .response
                .on_hover_text("Background subagents this agent launched. The engine doesn't yet say which one finished, only how many.");
            }
            let rows = fold_rows(&tr.blocks);
            let skip = rows.len().saturating_sub(MAX_ROWS);
            if tr.trimmed + skip > 0 {
                ui.label(faint(&t, format!("{} earlier items not shown", tr.trimmed + skip)));
            }
            for row in rows.into_iter().skip(skip) {
                match row {
                    Row::Thinking(secs) => {
                        ui.horizontal(|ui| {
                            ui.label(ic(icons::BRAIN, 13.0, t.text2));
                            let label = match secs {
                                Some(x) if x > 0 => format!("Thinking · {}", span(x)),
                                _ => "Thinking".into(),
                            };
                            ui.label(RichText::new(label).size(T_SMALL).color(t.text2));
                        });
                    }
                    Row::Message(text) => {
                        // mid-run narration: dimmed markdown, one step under body
                        let head: String = text.chars().take(4000).collect();
                        ui.scope(|ui| {
                            ui.multiply_opacity(0.72);
                            ui.style_mut().override_text_style = Some(egui::TextStyle::Button);
                            egui_commonmark::CommonMarkViewer::new().show(ui, v.md, &head);
                        });
                    }
                    Row::Tool(tool) => tool_row(ui, &t, tool, done),
                    Row::Group(tool, n) => {
                        ui.horizontal(|ui| {
                            ui.label(ic(icons::CIRCLE_CHECK, 13.0, t.success));
                            ui.label(ic(kind_icon(tool), 13.0, t.text2));
                            ui.label(RichText::new(tool_title(tool)).size(T_SMALL).color(t.text));
                            ui.label(RichText::new(format!("· {n} calls")).size(T_SMALL).color(t.text2));
                        });
                    }
                }
            }
            if tr.truncated_chars > 0 {
                ui.label(faint(&t, format!("{} characters clipped from overlong items", tr.truncated_chars)));
            }
        });
    });
}

enum Row<'a> {
    Thinking(Option<i64>),
    Message(&'a str),
    Tool(&'a crate::transcript::Tool),
    Group(&'a crate::transcript::Tool, usize),
}

/// Runs of more than three consecutive same-name OK tools collapse into one row.
fn fold_rows(blocks: &[Block]) -> Vec<Row<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < blocks.len() {
        match &blocks[i] {
            Block::Thinking { secs, .. } => out.push(Row::Thinking(*secs)),
            Block::Message { text, .. } => out.push(Row::Message(text)),
            Block::Tool { tool, .. } => {
                let mut j = i;
                while j + 1 < blocks.len()
                    && matches!(&blocks[j + 1], Block::Tool { tool: n, .. } if n.name == tool.name && n.status == ToolStatus::Ok && n.note.is_none())
                {
                    j += 1;
                }
                let run = j - i + 1;
                if run > 3 && tool.status == ToolStatus::Ok && tool.note.is_none() {
                    out.push(Row::Group(tool, run));
                    i = j + 1;
                    continue;
                }
                out.push(Row::Tool(tool));
            }
        }
        i += 1;
    }
    out
}

fn kind_icon(tool: &crate::transcript::Tool) -> &'static str {
    if matches!(tool.name.as_str(), "Agent" | "Task") {
        return icons::BOT;
    }
    match tool.kind.as_deref() {
        Some("command" | "shell" | "exec") => icons::TERMINAL,
        Some("file" | "read" | "edit" | "write") => icons::FILE_TEXT,
        Some("search" | "grep" | "glob") => icons::SEARCH,
        Some("web" | "fetch" | "browser") => icons::GLOBE,
        Some("mcp") => icons::PUZZLE,
        _ => icons::WRENCH,
    }
}

/// The row title: a command's program, otherwise the tool name.
fn tool_title(tool: &crate::transcript::Tool) -> String {
    match (tool.kind.as_deref(), tool.target.as_deref()) {
        (Some("command" | "shell" | "exec"), Some(target)) => {
            let cmd = humanize_target(&tool.name, target);
            cmd.split_whitespace().next().map(|w| super::basename(w).to_string()).unwrap_or_else(|| tool.name.clone())
        }
        _ => tool.name.clone(),
    }
}

/// Adapters send `"{name}: {value}"`; drop that prefix, the leading `VAR=…`
/// and `cd …` setup segments of a command, and spell $HOME as `~`.
pub fn humanize_target(name: &str, target: &str) -> String {
    let mut t = target.lines().next().unwrap_or("").trim();
    if let Some(rest) = t.strip_prefix(name).and_then(|r| r.strip_prefix(':')) {
        t = rest.trim_start();
    }
    let mut parts: Vec<&str> = t.split(" && ").flat_map(|x| x.split("; ")).map(str::trim).collect();
    while parts.len() > 1 && parts[0].split_whitespace().next().is_some_and(|w| w == "cd" || (w.contains('=') && !w.starts_with('-'))) {
        parts.remove(0);
    }
    let mut out = parts.join(" && ");
    if let Some(home) = dirs::home_dir() {
        out = out.replace(&*home.to_string_lossy(), "~");
    }
    out
}

fn tool_row(ui: &mut Ui, t: &Theme, tool: &crate::transcript::Tool, run_done: bool) {
    let (status, color) = match tool.status {
        ToolStatus::Running => (icons::LOADER_CIRCLE, t.running),
        // a background subagent's "ok" only means it started
        ToolStatus::Ok if tool.background && !run_done => (icons::LOADER_CIRCLE, t.running),
        ToolStatus::Ok => (icons::CIRCLE_CHECK, t.success),
        ToolStatus::Error => (icons::CIRCLE_X, t.failed),
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(ic(status, 13.0, color));
        ui.label(ic(kind_icon(tool), 13.0, t.text2));
        if let Some(note) = &tool.note {
            ui.label(RichText::new(format!("Agent · {note}")).size(T_SMALL).color(t.text));
            if let Some(sub) = &tool.sub {
                ui.label(RichText::new(sub).size(T_CAPTION).color(t.text2));
            }
            return;
        }
        ui.label(RichText::new(tool_title(tool)).size(T_SMALL).color(t.text));
        if let Some(target) = &tool.target {
            let human = humanize_target(&tool.name, target);
            let file = matches!(tool.kind.as_deref(), Some("file" | "read" | "edit" | "write"));
            let resp = if file {
                // basename in text colour, parent dimmed
                let (parent, base) = human.rsplit_once('/').unwrap_or(("", &human));
                ui.label(RichText::new(base).monospace().size(T_CAPTION).color(t.text));
                ui.add(egui::Label::new(RichText::new(parent).monospace().size(T_CAPTION).color(t.text3)).truncate())
            } else {
                ui.add(egui::Label::new(RichText::new(human.as_str()).monospace().size(T_CAPTION).color(t.text2)).truncate())
            };
            resp.on_hover_text(target);
        }
        if let (ToolStatus::Error, Some(code)) = (tool.status, tool.exit_code) {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(format!("exit {code}")).size(T_CAPTION).color(t.failed));
            });
        }
    });
    if tool.status == ToolStatus::Error {
        if let Some(why) = tool.detail.clone().filter(|d| !d.is_empty()) {
            ui.horizontal(|ui| {
                ui.add_space(38.0);
                ui.add(egui::Label::new(RichText::new(why).color(t.failed.gamma_multiply(0.9)).size(T_CAPTION)).wrap().selectable(true));
            });
        }
    }
}

fn refused(ui: &mut Ui, v: &View, s: &mut State, drafts: &mut Drafts, turn: &Turn, err: &crate::model::EnqueueError) {
    let t = v.t;
    Frame::new()
        .fill(t.failed.gamma_multiply(0.10))
        .corner_radius(R_SM)
        .inner_margin(Margin::symmetric(12, 10))
        .stroke(Stroke::new(1.0_f32, t.failed.gamma_multiply(0.35)))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(ic(icons::CIRCLE_X, 14.0, t.failed));
                ui.label(RichText::new("Turn refused").color(t.failed).family(semibold()).size(T_CALLOUT));
                if let Some(code) = &err.code {
                    ui.label(dim(&t, code));
                }
            });
            ui.add(egui::Label::new(RichText::new(&err.message).color(t.text).size(T_SMALL + 1.0)).wrap().selectable(true));
            for a in &err.required_actions {
                ui.label(RichText::new(format!("• {a}")).color(t.text2).size(T_SMALL));
            }
            let full_access = err.code.as_deref() == Some("trust_full_access_required");
            let root = s.detail.as_ref().and_then(|d| d.thread.repo_root.clone());
            if let (true, Some(root)) = (full_access, root) {
                // two steps: the grant lets agents run unsandboxed in this repo
                ui.add_space(SP);
                let key = format!("full-access:{}", turn.id);
                let online = s.client.is_some() && !s.composer.sending;
                ui.horizontal_wrapped(|ui| {
                    if drafts.override_armed.contains(&key) {
                        ui.label(RichText::new(format!("Agents will run unsandboxed in {}.", super::basename(&root))).size(T_SMALL).color(t.needs_you));
                        if ui.add_enabled(online, egui::Button::new(RichText::new("Grant & Retry").color(t.on_accent)).fill(t.failed)).clicked() {
                            s.grant_full_access_and_retry(&root, &turn.id);
                            drafts.override_armed.remove(&key);
                        }
                        if ui.button("Cancel").clicked() {
                            drafts.override_armed.remove(&key);
                        }
                    } else if ui.add_enabled(online, egui::Button::new("Allow full access & Retry…")).clicked() {
                        drafts.override_armed.insert(key);
                    }
                });
            } else if err.retryable != Some(false) {
                ui.add_space(SP);
                let b = egui::Button::new(RichText::new("Retry").color(t.on_accent)).fill(t.accent_solid).corner_radius(10);
                if ui
                    .add_enabled(s.client.is_some() && !s.composer.sending, b)
                    .on_disabled_hover_text("Engine offline or a request is in flight")
                    .clicked()
                {
                    s.retry(&turn.id);
                }
            } else {
                ui.label(dim(&t, "Not retryable: send a new message instead."));
            }
        });
}

fn question_card(ui: &mut Ui, v: &View, s: &mut State, drafts: &mut Drafts, it: &Interaction) {
    let t = v.t;
    ui.add_space(SP);
    Frame::new()
        .fill(t.raised_hi)
        .corner_radius(R_SM)
        .inner_margin(Margin::symmetric(12, 10))
        .stroke(Stroke::new(1.5_f32, t.needs_you))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(ic(icons::MESSAGE_SQUARE, 14.0, t.needs_you));
            ui.label(RichText::new("The agent is asking").color(t.needs_you).family(semibold()).size(T_CALLOUT));
            for q in &it.questions {
                let key = (it.interaction_id.clone(), q.id.clone());
                let entry = drafts.answers.entry(key).or_default();
                if let Some(h) = &q.header {
                    ui.label(dim(&t, h));
                }
                ui.add(egui::Label::new(RichText::new(&q.question).color(t.text).size(T_BODY)).wrap().selectable(true));
                ui.horizontal_wrapped(|ui| {
                    for o in &q.options {
                        let on = entry.0.contains(&o.label);
                        let r = ui.add(option_chip(&v.t, on, &o.label));
                        let r = match &o.description {
                            Some(d) => r.on_hover_text(d),
                            None => r,
                        };
                        if r.clicked() {
                            if on {
                                entry.0.retain(|x| x != &o.label);
                            } else if q.multi_select {
                                entry.0.push(o.label.clone());
                            } else {
                                entry.0 = vec![o.label.clone()];
                            }
                        }
                    }
                });
                ui.add(egui::TextEdit::singleline(&mut entry.1).hint_text("Or answer in your own words").desired_width(f32::INFINITY));
                ui.add_space(SP);
            }
            let ready = it.questions.iter().all(|q| {
                drafts.answers.get(&(it.interaction_id.clone(), q.id.clone())).is_some_and(|(p, f)| !p.is_empty() || !f.trim().is_empty())
            });
            let b = egui::Button::new(RichText::new("Send answer").color(t.on_accent)).fill(t.accent_solid).corner_radius(10);
            if ui.add_enabled(ready && s.client.is_some(), b).on_disabled_hover_text("Answer every question first").clicked() {
                let answers = it
                    .questions
                    .iter()
                    .map(|q| {
                        let (picked, free) = drafts.answers.remove(&(it.interaction_id.clone(), q.id.clone())).unwrap_or_default();
                        Answer {
                            question_id: q.id.clone(),
                            selected_labels: picked,
                            free_text: Some(free).filter(|f| !f.trim().is_empty()),
                        }
                    })
                    .collect();
                s.answer(&it.run_id, &it.interaction_id, answers);
            }
        });
    let _ = Color32::TRANSPARENT;
}

fn short(id: &str) -> &str {
    &id[id.len().saturating_sub(6)..]
}

/// Plan lifecycle on a plan turn: server-derived readiness, the open-question
/// card (answers go back as a follow-up plan turn tied by `answersPlanRunId`),
/// and Implement (an agent turn with `planRunId`; the engine freezes the plan).
/// Also the provenance receipts on the turns that answered / implemented one.
fn plan_section(ui: &mut Ui, v: &View, s: &mut State, drafts: &mut Drafts, turn: &Turn, later: &[Turn], state: &str) {
    let t = v.t;
    if let Some(p) = &turn.plan_run_id {
        let mut line = format!("Implements plan …{}", short(p));
        if let Some(h) = &turn.plan_hash {
            line.push_str(&format!(" · sha256 {}", &h[..h.len().min(12)]));
        }
        ui.horizontal(|ui| {
            ui.label(dim(&t, line));
            if turn.plan_readiness_overridden {
                ui.label(RichText::new("· implemented over open questions").color(t.blocked).size(T_SMALL));
            }
        });
    }
    if let Some(p) = &turn.answers_plan_run_id {
        ui.label(dim(&t, format!("Answers plan …{}", short(p))));
    }
    let Some(rid) = turn.run_id.clone() else { return };
    let Some(detail) = s.details.get(&rid) else { return };
    let Some(readiness) = detail.plan_readiness.clone() else { return };
    if !matches!(state, "succeeded" | "blocked") {
        return;
    }
    let questions = detail.plan_questions.clone();
    let answered_later = later.iter().any(|x| x.answers_plan_run_id.as_deref() == Some(rid.as_str()));
    let implemented_later = later.iter().any(|x| x.plan_run_id.as_deref() == Some(rid.as_str()));
    let sent = drafts.plan_sent.contains(&rid);

    ui.add_space(SP);
    ui.horizontal(|ui| {
        let (c, label) = match readiness.state.as_str() {
            "ready" => (t.success, "Plan ready".to_string()),
            "needs_answers" => {
                let n = readiness.question_count.max(questions.len() as u32);
                (t.needs_you, format!("{n} open question{}", if n == 1 { "" } else { "s" }))
            }
            _ => (t.blocked, "Plan unverified".to_string()),
        };
        chip(ui, &t, &label, c).on_hover_text(match readiness.state.as_str() {
            "unverified" => "The planner did not produce a parseable open-questions block",
            _ => "Readiness is derived by the engine from the plan's question block",
        });
        if implemented_later {
            ui.label(dim(&t, "implemented below"));
        } else if answered_later {
            ui.label(dim(&t, "answered below"));
        } else if sent {
            ui.label(dim(&t, "answers sent…"));
        }
    });

    let open = readiness.state == "needs_answers" && !questions.is_empty() && !answered_later && !sent && !implemented_later;
    if open {
        plan_questions_card(ui, v, s, drafts, &rid, &questions);
    }
    if implemented_later || answered_later {
        return;
    }
    // Implement: straight when ready; otherwise an explicit, recorded override.
    let busy = s.head_live_id().is_some() || s.composer.sending || s.client.is_none();
    ui.add_space(SP);
    ui.horizontal(|ui| {
        if readiness.state == "ready" {
            let b = egui::Button::new(RichText::new("Implement plan").color(t.on_accent)).fill(t.accent_solid).corner_radius(10);
            if ui
                .add_enabled(!busy, b)
                .on_hover_text("Run an Agent turn against this frozen plan")
                .on_disabled_hover_text("A turn is running or the engine is offline")
                .clicked()
            {
                s.implement_plan(&rid, false);
            }
        } else {
            let arm_id = Id::new(("implement-anyway", &rid));
            let armed: bool = ui.ctx().data(|d| d.get_temp(arm_id)).unwrap_or(false);
            let label = if armed { "Click again: implement over open questions" } else { "Implement anyway" };
            let b = egui::Button::new(RichText::new(label).color(t.on_accent)).fill(t.blocked).corner_radius(10);
            if ui
                .add_enabled(!busy, b)
                .on_hover_text("Recorded on the turn as a readiness override")
                .on_disabled_hover_text("A turn is running or the engine is offline")
                .clicked()
            {
                if armed {
                    ui.ctx().data_mut(|d| d.remove::<bool>(arm_id));
                    s.implement_plan(&rid, true);
                } else {
                    ui.ctx().data_mut(|d| d.insert_temp(arm_id, true));
                }
            }
        }
    });
}

fn plan_questions_card(ui: &mut Ui, v: &View, s: &mut State, drafts: &mut Drafts, rid: &str, questions: &[crate::model::PlanQuestion]) {
    let t = v.t;
    ui.add_space(SP);
    Frame::new()
        .fill(t.raised_hi)
        .corner_radius(R_SM)
        .inner_margin(Margin::symmetric(12, 10))
        .stroke(Stroke::new(1.5_f32, t.needs_you))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Answer the plan's open questions").color(t.needs_you).family(semibold()).size(T_SMALL + 1.0));
            ui.label(dim(&t, "Options are suggestions — your own words always count as a full answer."));
            for q in questions {
                ui.add_space(SP);
                let key = (rid.to_string(), q.id.clone());
                let entry = drafts.plan.entry(key).or_default();
                ui.add(egui::Label::new(RichText::new(&q.prompt).color(t.text).size(T_BODY)).wrap().selectable(true));
                if !q.options.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        for o in &q.options {
                            let on = entry.0.contains(&o.id);
                            if ui.add(option_chip(&t, on, &o.label)).clicked() {
                                if q.kind == "multi" {
                                    if on {
                                        entry.0.retain(|x| x != &o.id);
                                    } else {
                                        entry.0.push(o.id.clone());
                                    }
                                } else {
                                    entry.0 = if on { vec![] } else { vec![o.id.clone()] };
                                }
                                if !entry.0.is_empty() {
                                    entry.1.clear(); // own words replace chips, never both
                                }
                            }
                        }
                    });
                }
                let hint = if q.kind == "text" { "Your answer" } else { "Or answer in your own words" };
                if ui.add(egui::TextEdit::multiline(&mut entry.1).hint_text(hint).desired_rows(1).desired_width(f32::INFINITY)).changed()
                    && !entry.1.trim().is_empty()
                {
                    entry.0.clear();
                }
            }
            let picked = |qid: &str| drafts.plan.get(&(rid.to_string(), qid.to_string())).map(|e| e.0.clone()).unwrap_or_default();
            let text = |qid: &str| drafts.plan.get(&(rid.to_string(), qid.to_string())).map(|e| e.1.clone()).unwrap_or_default();
            let complete = crate::model::plan_answers_complete(questions, &picked, &text);
            let prompt = crate::model::encode_plan_answers(questions, &picked, &text);
            ui.add_space(SP);
            let busy = s.head_live_id().is_some() || s.composer.sending || s.client.is_none();
            let b = egui::Button::new(RichText::new("Send answers").color(t.on_accent)).fill(t.accent_solid).corner_radius(10);
            if ui
                .add_enabled(complete && !busy, b)
                .on_disabled_hover_text(if busy { "A turn is running or the engine is offline" } else { "Answer every question first" })
                .clicked()
            {
                s.answer_plan(rid, prompt);
                drafts.plan.retain(|(r, _), _| r != rid);
                drafts.plan_sent.insert(rid.to_string());
            }
        });
}

fn engine_button(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    ui.add_space(3.0 * SP);
    ui.vertical_centered(|ui| {
        if s.engine_starting {
            ui.horizontal(|ui| {
                super::spinner(ui, 14.0, egui::Color32::GRAY);
                ui.label(dim(&t, "Starting the engine…"));
            });
        } else {
            if ui.add(accent_button("Start engine")).on_hover_text("Runs `claudexor daemon start`").clicked() {
                s.start_engine();
            }
        }
        if let Some(e) = &s.engine_start_error {
            ui.add(egui::Label::new(RichText::new(e).color(t.failed).size(T_SMALL)).wrap().selectable(true));
        }
    });
}

/// An answer option that reads as a clickable chip whether picked or not.
fn option_chip<'a>(t: &Theme, on: bool, label: &'a str) -> egui::Button<'a> {
    let text = RichText::new(label).size(T_SMALL + 1.0).color(if on { t.on_accent } else { t.text });
    egui::Button::new(text)
        .selected(on)
        .fill(if on { t.accent_solid } else { t.raised })
        .stroke(Stroke::new(1.0_f32, if on { t.accent } else { t.separator }))
        .corner_radius(10)
}

/// Phases the run has been through, derived from its timeline event types.
fn phases(d: &RunDetail) -> Vec<(&'static str, bool)> {
    const STEPS: [(&str, &[&str]); 5] = [
        ("Queued", &["run.created", "task."]),
        ("Routed", &["route."]),
        ("Running", &["harness.", "attempt.", "tool."]),
        ("Gates", &["gate."]),
        ("Review", &["review."]),
    ];
    let seen = |pre: &[&str]| d.timeline.iter().any(|e| pre.iter().any(|p| e.kind.starts_with(p)));
    let mut out: Vec<(&str, bool)> = STEPS.iter().filter(|(_, p)| seen(p)).map(|(n, _)| (*n, true)).collect();
    let end = d.timeline.iter().rev().find_map(|e| match e.kind.as_str() {
        "run.completed" => Some(("Done", true)),
        "run.failed" => Some(("Failed", false)),
        "run.blocked" => Some(("Blocked", false)),
        _ => None,
    });
    out.extend(end);
    out
}

/// Everything the run detail knows beyond the answer: phases, route facts,
/// budget, plan checklist, candidates, review findings, children, warnings.
fn run_details(ui: &mut Ui, v: &View, d: &RunDetail, turn: &Turn) {
    let t = v.t;
    let line = |ui: &mut Ui, k: &str, val: String| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(k).size(T_SMALL).color(t.text3));
            ui.add(egui::Label::new(RichText::new(val).size(T_SMALL).color(t.text2)).wrap().selectable(true));
        });
    };
    Frame::new().fill(t.raised).corner_radius(R_SM).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 3.0;
        let ph = phases(d);
        if !ph.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (i, (name, ok)) in ph.iter().enumerate() {
                    if i > 0 {
                        ui.label(RichText::new("›").size(T_SMALL).color(t.text3));
                    }
                    ui.label(RichText::new(*name).size(T_SMALL).color(if *ok { t.text } else { t.failed }));
                }
            });
        }
        let s = &d.summary;
        match (s.requested_access.as_deref(), s.effective_access.as_deref()) {
            (Some(r), Some(e)) if r != e => line(ui, "access", format!("{e} (asked {r})")),
            (_, Some(e)) => line(ui, "access", e.to_string()),
            _ => {}
        }
        if let Some(a) = &s.auth_route {
            let mut v = a.effective.clone().unwrap_or_else(|| "?".into());
            if let Some(r) = &a.reason {
                v.push_str(&format!(" · {r}"));
            }
            if let Some(p) = &a.profile_id {
                v.push_str(&format!(" · {p}"));
            }
            line(ui, "auth", v);
        }
        if let Some(w) = &s.web_evidence {
            let mode = w.effective_mode.as_deref().or(w.mode.as_deref()).unwrap_or("?");
            let st = w.status.as_deref().unwrap_or("none");
            line(ui, "web", format!("{mode} · {st}{}", if w.required { " · required" } else { "" }));
        }
        if let Some(dl) = s.delegation.as_ref().filter(|x| x.requested) {
            let st = if dl.used { "used" } else if dl.effective { "available, unused" } else { "unavailable" };
            line(ui, "delegate", format!("{st}{}", dl.reason.as_ref().map(|r| format!(" · {r}")).unwrap_or_default()));
        }
        if let Some(st) = &s.strategy {
            line(ui, "strategy", st.clone());
        }
        if let Some(c) = &turn.continuity {
            let mut v = match c.kind.as_str() {
                "native_resume" => "native session resume".to_string(),
                "packet" => format!("context packet of {} turn{}", c.packet_turns, if c.packet_turns == 1 { "" } else { "s" }),
                k => k.to_string(),
            };
            if c.summarized {
                v.push_str(" · summarized");
            }
            if let Some(h) = c.lane_switched_from.as_ref().and_then(|l| l.harness.as_deref()) {
                v.push_str(&format!(" · switched from {h}"));
            }
            line(ui, "context", v);
        }
        if let Some(b) = &d.budget {
            let mut v = match b.spend_usd {
                Some(x) => format!("${x:.2} cash ({})", b.cash_knowledge.as_deref().unwrap_or("unknown")),
                None => "cash unknown".into(),
            };
            if let Some(x) = b.valuation_usd.filter(|x| *x > 0.0) {
                v.push_str(&format!(" · ≈${x:.2} at list price"));
            }
            if let Some(cap) = b.paid_budget.as_ref().and_then(|p| p.get("maxUsd")).and_then(serde_json::Value::as_f64) {
                v.push_str(&format!(" · cap ${cap:.2}"));
            }
            if let Some(r) = b.remaining_usd {
                v.push_str(&format!(" · ${r:.2} left"));
            }
            line(ui, "budget", v);
        }
        if let Some(pp) = d.plan_progress.as_ref().filter(|p| !p.items.is_empty()) {
            ui.add_space(SP / 2.0);
            let done = pp.items.iter().filter(|i| i.status == "completed").count();
            ui.label(RichText::new(format!("Plan {done}/{}", pp.items.len())).size(T_SMALL).strong().color(t.text2));
            for it in &pp.items {
                let (g, c) = match it.status.as_str() {
                    "completed" => ("✓", t.success),
                    "in_progress" => ("●", t.running),
                    _ => ("○", t.text3),
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(g).size(T_SMALL).color(c));
                    ui.label(RichText::new(&it.title).size(T_SMALL).color(t.text));
                });
            }
        }
        if !d.candidates.is_empty() {
            ui.add_space(SP / 2.0);
            ui.label(RichText::new("Candidates").size(T_SMALL).strong().color(t.text2));
            for c in &d.candidates {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0 * SP;
                    ui.label(RichText::new(if c.winner { "★" } else { "·" }).size(T_SMALL).color(t.accent));
                    let h = c.harness_id.as_deref().unwrap_or("?");
                    chip(ui, &t, h, t.harness(h));
                    ui.label(dim(&t, &c.attempt_id));
                    if let (Some(p), Some(n)) = (c.gates_passed, c.gates_total) {
                        ui.label(dim(&t, format!("gates {p}/{n}")));
                    }
                    if let Some(x) = c.cost_usd {
                        ui.label(dim(&t, format!("{}${x:.2}", if c.cost_estimated { "≈" } else { "" })));
                    }
                    if c.blockers > 0 {
                        ui.label(RichText::new(format!("{} blocker{}", c.blockers, if c.blockers == 1 { "" } else { "s" })).size(T_SMALL).color(t.blocked));
                    }
                    if c.review_verified == Some(true) {
                        ui.label(RichText::new("reviewed").size(T_SMALL).color(t.success));
                    }
                    if let Some(ds) = c.diffstat.as_ref().filter(|d| d.files > 0) {
                        ui.label(dim(&t, format!("{} files +{} −{}", ds.files, ds.additions, ds.deletions)));
                    }
                    if c.errored {
                        ui.label(RichText::new(c.error_reason.as_deref().unwrap_or("errored")).size(T_SMALL).color(t.failed));
                    }
                });
            }
        }
        if !d.review_findings.is_empty() {
            ui.add_space(SP / 2.0);
            ui.label(RichText::new("Review findings").size(T_SMALL).strong().color(t.text2));
            for f in &d.review_findings {
                let c = match f.severity.as_str() {
                    "BLOCK" | "FIX_FIRST" => t.failed,
                    "WARN" | "NEEDS_HUMAN" => t.needs_you,
                    _ => t.text3,
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&f.severity).size(T_SMALL).strong().color(c));
                    ui.label(dim(&t, format!("{} · {}", f.category, f.status)));
                    ui.add(egui::Label::new(RichText::new(&f.claim).size(T_SMALL).color(t.text)).wrap().selectable(true));
                });
            }
        }
        if !d.children.is_empty() {
            ui.add_space(SP / 2.0);
            ui.label(RichText::new("Sub-runs").size(T_SMALL).strong().color(t.text2));
            for c in &d.children {
                let (color, glyph, word) = t.status(&c.state);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(glyph).size(T_SMALL).color(color));
                    ui.label(dim(&t, format!("{} · {word}", c.run_id.as_deref().unwrap_or("?"))));
                });
            }
        }
        let warns: Vec<_> = d.timeline.iter().filter(|e| matches!(e.severity.as_str(), "warn" | "warning" | "error")).collect();
        if !warns.is_empty() {
            ui.add_space(SP / 2.0);
            for e in warns.iter().rev().take(8).rev() {
                let c = if e.severity == "error" { t.failed } else { t.needs_you };
                let text = match &e.error_summary {
                    Some(x) => format!("{} · {x}", e.title),
                    None => e.title.clone(),
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("!").size(T_SMALL).strong().color(c));
                    ui.add(egui::Label::new(RichText::new(text).size(T_SMALL).color(t.text2)).wrap().selectable(true));
                });
            }
        }
    });
}

/// Isolated thread: deliver the worktree's cumulative diff into the project.
fn apply_thread_bar(ui: &mut Ui, v: &View, s: &mut State, thread_id: &str) {
    let t = v.t;
    ui.horizontal_wrapped(|ui| {
        match s.thread_apply.get(thread_id) {
            Some(Fetch::Ready(line)) => {
                ui.label(RichText::new(format!("{} {line}", icons::CHECK)).size(T_SMALL).color(t.success));
            }
            Some(Fetch::Loading) => {
                super::spinner(ui, 14.0, egui::Color32::GRAY);
                ui.label(dim(&t, "Applying the thread…"));
            }
            other => {
                if let Some(Fetch::Failed(e)) = other {
                    ui.label(RichText::new(e.clone()).size(T_SMALL).color(t.failed));
                }
                let online = s.client.is_some();
                if ui.add_enabled(online, egui::Button::new(RichText::new("Apply thread").size(T_SMALL).color(t.on_accent)).fill(t.accent_solid))
                    .on_hover_text("Merge this thread's isolated worktree changes into the project")
                    .clicked()
                {
                    s.apply_thread(thread_id, "apply");
                }
                if ui.add_enabled(online, egui::Button::new(RichText::new("As branch").size(T_SMALL))).on_hover_text("Deliver the changes as a new git branch").clicked() {
                    s.apply_thread(thread_id, "branch");
                }
            }
        }
    });
}

/// Changes of one run: diff, Apply / Revert, and the decision bar when blocked.
fn workspace(ui: &mut Ui, v: &View, s: &mut State, drafts: &mut Drafts, d: &RunDetail, turn: &Turn) {
    let t = v.t;
    let Some(rid) = d.summary.run_id.clone().or_else(|| turn.run_id.clone()) else { return };
    let result = d.summary.result.clone().or_else(|| turn.run.as_ref().and_then(|r| r.result.clone()));
    let apply_state = result.as_ref().and_then(|r| r.apply_state.clone()).unwrap_or_else(|| "not_applied".into());
    let key = format!("changes:{}", turn.id);
    let open = s.expanded.contains(&key);
    ui.add_space(SP / 2.0);
    ui.horizontal_wrapped(|ui| {
        let stat = result.as_ref().and_then(|r| r.diff_stat.as_ref()).map(|x| format!(" · {} files +{} −{}", x.files, x.additions, x.deletions)).unwrap_or_default();
        let word = match apply_state.as_str() {
            "applied" => " · applied",
            "applied_review_blocked" => " · applied, review blocked",
            "reverted" => " · reverted",
            "discarded" => " · discarded",
            _ => "",
        };
        let now = super::disclosure(ui, &t, Id::new(&key), open, RichText::new(format!("Changes{stat}{word}")).size(T_SMALL).color(t.text));
        if now != open {
            if now {
                s.expanded.insert(key.clone());
            } else {
                s.expanded.remove(&key);
            }
        }
    });
    if open {
        s.load_diff(&rid);
        match s.diffs.get(&rid) {
            Some(Fetch::Ready(text)) if text.trim().is_empty() => {
                ui.label(dim(&t, "No patch file for this run."));
            }
            Some(Fetch::Ready(text)) => diff_view(ui, &t, text),
            Some(Fetch::Failed(e)) => {
                ui.label(RichText::new(format!("Could not load the patch: {e}")).size(T_SMALL).color(t.failed));
            }
            _ => {
                ui.horizontal(|ui| {
                    super::spinner(ui, 14.0, egui::Color32::GRAY);
                    ui.label(dim(&t, "Loading patch…"));
                });
            }
        }
    }

    let busy = s.run_busy.contains(&rid);
    let online = s.client.is_some() && !busy;
    let eligible = d.apply_eligibility.as_ref().is_some_and(|e| e.eligible);
    let revertable = result.as_ref().is_some_and(|r| r.revertable) && !matches!(apply_state.as_str(), "reverted" | "discarded");
    ui.horizontal_wrapped(|ui| {
        if eligible && apply_state == "not_applied" {
            if ui.add_enabled(online, egui::Button::new(RichText::new("Apply patch").size(T_SMALL).color(t.on_accent)).fill(t.accent_solid)).clicked() {
                s.apply_run(&rid, "apply");
            }
            if ui.add_enabled(online, egui::Button::new(RichText::new("Apply as branch").size(T_SMALL))).clicked() {
                s.apply_run(&rid, "branch");
            }
        } else if let Some(why) = d.apply_eligibility.as_ref().filter(|e| !e.eligible && apply_state == "not_applied").and_then(|e| e.reason.clone()) {
            ui.label(dim(&t, format!("Apply unavailable: {why}")));
        }
        if revertable
            && ui
                .add_enabled(online, egui::Button::new(RichText::new("Revert").size(T_SMALL)))
                .on_hover_text("Restore the project to this run's pre-turn state (the engine refuses if you've edited since)")
                .clicked()
        {
            s.decide(&rid, serde_json::json!({"action": "revert_run"}));
        }
        if busy {
            super::spinner(ui, 14.0, egui::Color32::GRAY);
        }
    });
    match s.run_note.get(&rid) {
        Some(Ok(line)) => {
            ui.label(RichText::new(format!("{} {line}", icons::CHECK)).size(T_SMALL).color(t.success));
        }
        Some(Err(e)) => {
            ui.add(egui::Label::new(RichText::new(e).size(T_SMALL).color(t.failed)).wrap().selectable(true));
        }
        None => {}
    }
    if d.needs_decision() {
        decision_bar(ui, v, s, drafts, d, &rid, online);
    }
}

/// A blocked run needs a human: accept the risk, rerun with feedback, or override.
fn decision_bar(ui: &mut Ui, v: &View, s: &mut State, drafts: &mut Drafts, d: &RunDetail, rid: &str, online: bool) {
    let t = v.t;
    Frame::new().fill(t.needs_you.gamma_multiply(0.10)).corner_radius(R_SM).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Needs your decision").size(T_SMALL).strong().color(t.needs_you));
        for a in &d.required_actions {
            if !a.detail.is_empty() {
                ui.label(dim(&t, format!("• {}", a.detail)));
            }
        }
        let text = drafts.decision.entry(rid.to_string()).or_default();
        ui.add(TextEdit::multiline(text).hint_text("The risk you accept, or feedback for a rerun").desired_rows(2).desired_width(f32::INFINITY));
        let has_text = !text.trim().is_empty();
        let text = text.trim().to_string();
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(online && has_text, egui::Button::new(RichText::new("Accept risk & unblock").size(T_SMALL))).on_disabled_hover_text("Describe the risk you accept first").clicked() {
                s.decide(rid, serde_json::json!({"action": "accept_risk", "acceptedRisks": [text.clone()]}));
                drafts.decision.remove(rid);
            }
            if ui.add_enabled(online && has_text, egui::Button::new(RichText::new("Rerun with feedback").size(T_SMALL))).on_disabled_hover_text("Write the feedback first").clicked() {
                s.decide(rid, serde_json::json!({"action": "rerun_with_feedback", "feedback": text.clone()}));
                drafts.decision.remove(rid);
            }
            if drafts.override_armed.contains(rid) {
                if ui.add_enabled(online, egui::Button::new(RichText::new("Confirm override").size(T_SMALL).color(t.on_accent)).fill(t.failed)).clicked() {
                    s.decide(rid, serde_json::json!({"action": "override_needs_human"}));
                    drafts.override_armed.remove(rid);
                }
                if ui.button(RichText::new("Cancel").size(T_SMALL)).clicked() {
                    drafts.override_armed.remove(rid);
                }
            } else if ui
                .add_enabled(online, egui::Button::new(RichText::new("Override needs-human…").size(T_SMALL).color(t.failed)))
                .on_hover_text("Records an auditable override bound to the current patch. Apply becomes available; a changed patch invalidates it.")
                .clicked()
            {
                drafts.override_armed.insert(rid.to_string());
            }
        });
    });
}

/// One file of a unified diff: its path, +/− counts and gutter-numbered lines.
pub struct DiffFile {
    pub path: String,
    pub adds: usize,
    pub dels: usize,
    /// (old line, new line, text); hunk headers carry neither number.
    pub lines: Vec<(Option<u32>, Option<u32>, String)>,
}

/// Split a unified diff into files and number each line from its hunk header.
pub fn parse_diff(text: &str) -> Vec<DiffFile> {
    let mut files: Vec<DiffFile> = vec![];
    let (mut old, mut new) = (0u32, 0u32);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = rest.rsplit(" b/").next().unwrap_or(rest).to_string();
            files.push(DiffFile { path, adds: 0, dels: 0, lines: vec![] });
            continue;
        }
        let Some(f) = files.last_mut() else { continue };
        if line.starts_with("index ") || line.starts_with("--- ") || line.starts_with("+++ ") || line.starts_with("new file") || line.starts_with("deleted file") {
            continue;
        }
        if let Some(h) = line.strip_prefix("@@ ") {
            // @@ -a,b +c,d @@
            let mut it = h.split_whitespace();
            let num = |s: Option<&str>| s.and_then(|x| x[1..].split(',').next()).and_then(|x| x.parse::<u32>().ok()).unwrap_or(1);
            old = num(it.next());
            new = num(it.next());
            f.lines.push((None, None, line.to_string()));
        } else if line.starts_with('+') {
            f.adds += 1;
            f.lines.push((None, Some(new), line.to_string()));
            new += 1;
        } else if line.starts_with('-') {
            f.dels += 1;
            f.lines.push((Some(old), None, line.to_string()));
            old += 1;
        } else {
            f.lines.push((Some(old), Some(new), line.to_string()));
            old += 1;
            new += 1;
        }
    }
    files
}

/// Unified diff on a solid inset: a collapsible section per file with +/−
/// counts and old/new gutters. Long files show 400 lines, then "Show all".
fn diff_view(ui: &mut Ui, t: &Theme, text: &str) {
    const CAP: usize = 400;
    let files = parse_diff(text);
    if files.is_empty() {
        ui.label(dim(t, "The patch has no file changes."));
        return;
    }
    let many = files.len() > 6;
    for (i, f) in files.iter().enumerate() {
        let id = ui.id().with(("diff-file", i, &f.path));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, !many)
            .show_header(ui, |ui| {
                ui.label(RichText::new(&f.path).monospace().size(T_SMALL).color(t.text));
                ui.label(RichText::new(format!("+{}", f.adds)).size(T_SMALL).color(t.success));
                ui.label(RichText::new(format!("−{}", f.dels)).size(T_SMALL).color(t.failed));
            })
            .body(|ui| {
                let all_key = id.with("all");
                let show_all: bool = ui.ctx().data(|d| d.get_temp(all_key)).unwrap_or(false);
                let n = if show_all { f.lines.len() } else { f.lines.len().min(CAP) };
                // one painted row per line at the mono text height (a widget row is 24 px)
                let font = egui::FontId::monospace(T_CAPTION + 0.5);
                let line_h = ui.fonts_mut(|x| x.row_height(&font)) + 1.0;
                Frame::new().fill(t.code).corner_radius(R_SM).inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let want = (n as f32 * line_h + 2.0).min(480.0);
                    ScrollArea::both().id_salt(id.with("scroll")).max_height(480.0).min_scrolled_height(want).auto_shrink([false, true]).show_rows(ui, line_h, n, |ui, range| {
                        let gutter_w = 64.0;
                        for (o, nw, line) in &f.lines[range] {
                            let color = if line.starts_with('+') {
                                t.success
                            } else if line.starts_with('-') {
                                t.failed
                            } else if line.starts_with("@@") {
                                t.accent
                            } else {
                                t.text2
                            };
                            let text_w = ui.fonts_mut(|x| x.layout_no_wrap(line.clone(), font.clone(), color).size().x);
                            let (rect, _) = ui.allocate_exact_size(egui::vec2((gutter_w + text_w + 8.0).max(ui.available_width()), line_h), Sense::hover());
                            if line.starts_with('+') || (line.starts_with('-') && !line.starts_with("---")) {
                                ui.painter().rect_filled(rect, 0, color.gamma_multiply(0.08));
                            }
                            let gut = |x: &Option<u32>| x.map_or("    ".to_string(), |v| format!("{v:>4}"));
                            let p = ui.painter();
                            p.text(rect.left_center(), egui::Align2::LEFT_CENTER, format!("{} {}", gut(o), gut(nw)), font.clone(), t.text3);
                            p.text(rect.left_center() + egui::vec2(gutter_w, 0.0), egui::Align2::LEFT_CENTER, line, font.clone(), color);
                        }
                    });
                });
                if f.lines.len() > n && ui.button(RichText::new(format!("Show all {} lines", f.lines.len())).size(T_SMALL)).clicked() {
                    ui.ctx().data_mut(|d| d.insert_temp(all_key, true));
                }
            });
    }
}

/// Files the run wrote into the project's outputs; Open saves a private copy and hands it to the desktop.
fn outputs(ui: &mut Ui, v: &View, s: &mut State, turn: &Turn) {
    let t = v.t;
    let Some(rid) = turn.run_id.clone() else { return };
    let key = format!("outputs:{}", turn.id);
    let open = s.expanded.contains(&key);
    if !open {
        if let Some(Fetch::Ready(list)) = s.files.get(&rid) {
            if list.is_empty() {
                return; // known empty: no row at all
            }
        }
    }
    let now = super::disclosure(ui, &t, Id::new(&key), open, RichText::new("Outputs").size(T_SMALL).color(t.text));
    if now != open {
        if now {
            s.expanded.insert(key.clone());
        } else {
            s.expanded.remove(&key);
        }
    }
    if !open {
        return;
    }
    s.load_files(&rid);
    let list = match s.files.get(&rid) {
        Some(Fetch::Ready(list)) => list.clone(),
        Some(Fetch::Failed(e)) => {
            ui.label(RichText::new(format!("Could not list outputs: {e}")).size(T_SMALL).color(t.failed));
            return;
        }
        _ => {
            super::spinner(ui, 14.0, egui::Color32::GRAY);
            return;
        }
    };
    if list.is_empty() {
        ui.label(dim(&t, "No produced files."));
    }
    for a in list {
        let kind = crate::state::output_kind(&a.path, a.mime.as_deref());
        ui.horizontal(|ui| {
            ui.label(RichText::new(if kind == Some("image") { "▣" } else { "◇" }).size(T_SMALL).color(t.text3));
            ui.add(egui::Label::new(RichText::new(&a.path).size(T_SMALL).monospace().color(t.text)).truncate());
            if let Some(b) = a.bytes {
                ui.label(dim(&t, format!("{} KB", b.div_ceil(1024))));
            }
            if kind.is_some() && ui.add_enabled(s.client.is_some(), egui::Button::new(RichText::new("View").size(T_SMALL))).clicked() {
                s.viewing = Some((rid.clone(), a.path.clone()));
            }
            if ui.add_enabled(s.client.is_some(), egui::Button::new(RichText::new("Open").size(T_SMALL))).on_hover_text("Open in the default app").clicked() {
                s.open_output(&rid, &a.path);
            }
        });
        // images get an inline thumbnail (click to enlarge)
        if kind == Some("image") {
            s.load_preview(&rid, &a.path, "image");
            match s.previews.get(&State::preview_key(&rid, &a.path)) {
                Some(Fetch::Ready(Preview::Image(bytes))) => {
                    let img = egui::Image::from_bytes(format!("bytes://out/{rid}/{}", a.path), egui::load::Bytes::Shared(bytes.clone()))
                        .max_height(140.0)
                        .max_width(ui.available_width())
                        .corner_radius(R_SM)
                        .sense(Sense::click());
                    if ui.add(img).on_hover_text("Click to enlarge").clicked() {
                        s.viewing = Some((rid.clone(), a.path.clone()));
                    }
                }
                Some(Fetch::Failed(e)) => {
                    ui.label(RichText::new(format!("preview: {e}")).size(T_SMALL).color(t.text3));
                }
                _ => {
                    super::spinner(ui, 14.0, egui::Color32::GRAY);
                }
            }
        }
    }
}

/// The output viewer: an image at full size or a text file (markdown rendered).
pub fn viewer(ctx: &egui::Context, v: &mut View, s: &mut State) {
    let Some((rid, path)) = s.viewing.clone() else { return };
    let t = v.t;
    let kind = crate::state::output_kind(&path, None).unwrap_or("text");
    s.load_preview(&rid, &path, kind);
    let preview = match s.previews.get(&State::preview_key(&rid, &path)) {
        Some(Fetch::Ready(p)) => Some(p.clone()),
        Some(Fetch::Failed(e)) => {
            let e = e.clone();
            Some(Preview::Text(format!("Could not load {path}: {e}")))
        }
        _ => None,
    };
    let screen = ctx.content_rect();
    let mut close = false;
    let m = egui::Modal::new(Id::new("output-viewer")).show(ctx, |ui| {
        ui.set_width((screen.width() - 96.0).min(980.0));
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(&path).monospace().color(t.text)).truncate());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    close = true;
                }
                if ui.button("Open").on_hover_text("Open in the default app").clicked() {
                    s.open_output(&rid, &path);
                }
            });
        });
        ui.separator();
        let h = screen.height() - 160.0;
        ScrollArea::both().max_height(h).min_scrolled_height(h.min(420.0)).auto_shrink([false, true]).show(ui, |ui| match preview {
            Some(Preview::Image(bytes)) => {
                ui.add(egui::Image::from_bytes(format!("bytes://out/{rid}/{path}"), egui::load::Bytes::Shared(bytes)).max_width(ui.available_width()));
            }
            Some(Preview::Text(text)) if path.ends_with(".md") => {
                egui_commonmark::CommonMarkViewer::new().show(ui, v.md, &text);
            }
            Some(Preview::Text(text)) => {
                Frame::new().fill(t.code).corner_radius(R_SM).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(text).monospace().size(T_SMALL).color(t.text)).selectable(true).extend());
                });
            }
            None => {
                super::spinner(ui, 14.0, egui::Color32::GRAY);
            }
        });
    });
    if close || m.should_close() {
        s.viewing = None;
    }
}

#[cfg(test)]
mod tests {
    use super::parse_diff;

    #[test]
    fn diff_parses_files_counts_and_gutters() {
        let text = "diff --git a/notes.txt b/notes.txt\nindex 08fe272..fa8765f 100644\n--- a/notes.txt\n+++ b/notes.txt\n@@ -1 +1,2 @@\n first line\n+hello\ndiff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -10,2 +10,1 @@\n-gone\n kept\n";
        let f = parse_diff(text);
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].path.as_str(), f[0].adds, f[0].dels), ("notes.txt", 1, 0));
        assert_eq!(f[0].lines[1], (Some(1), Some(1), " first line".to_string()));
        assert_eq!(f[0].lines[2], (None, Some(2), "+hello".to_string()));
        assert_eq!(f[1].lines[1], (Some(10), None, "-gone".to_string()));
        assert_eq!(f[1].lines[2], (Some(11), Some(10), " kept".to_string()));
    }
}

/// The workspace side panel (DESIGN_SYSTEM §4: a trailing panel, never a tab
/// replacing the chat): Changes / Outputs / Evidence across the thread's runs,
/// optionally scoped to one run by the filter chip.
pub fn workspace_side(ui: &mut Ui, v: &mut View, s: &mut State, drafts: &mut Drafts, rect: Rect) {
    let t = v.t;
    let Some(detail) = s.detail.clone() else { return };
    let slot = v.glass.card_slot(ui);
    ui.scope_builder(UiBuilder::new().max_rect(rect.shrink(12.0)), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0 * SP;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Workspace").family(semibold()).size(T_CALLOUT).color(t.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if super::icon_button(ui, &t, icons::X, "Close the workspace (Ctrl+.)").clicked() {
                    s.ws_open = false;
                }
            });
        });
        if detail.thread.workspace_mode.as_deref() == Some("isolated") && detail.turns.iter().any(|x| x.run.as_ref().is_some_and(|r| r.mode.as_deref() == Some("agent"))) {
            apply_thread_bar(ui, v, s, &detail.thread.id);
        }
        let runs: Vec<(usize, &Turn, String)> = detail.turns.iter().enumerate().filter_map(|(i, x)| x.run_id.clone().map(|r| (i + 1, x, r))).collect();
        if s.ws_run.as_ref().is_some_and(|r| !runs.iter().any(|x| &x.2 == r)) {
            s.ws_run = None;
        }
        ui.horizontal(|ui| {
            let mut tab = s.ws_tab;
            if super::segmented(ui, &t, &mut tab, &[(0u8, "Changes"), (1, "Outputs"), (2, "Evidence")]) {
                s.ws_tab = tab;
            }
            let label = match &s.ws_run {
                Some(r) => runs.iter().find(|x| &x.2 == r).map(|x| format!("Turn {}", x.0)).unwrap_or_else(|| "Run".into()),
                None => "All runs".into(),
            };
            super::chip_menu(ui, &t, "ws-run", &label, t.text, false, |ui| {
                if ui.selectable_label(s.ws_run.is_none(), "All runs").clicked() {
                    s.ws_run = None;
                }
                for (n, turn, rid) in &runs {
                    let title: String = turn.prompt.lines().next().unwrap_or("").chars().take(40).collect();
                    if ui.selectable_label(s.ws_run.as_deref() == Some(rid), format!("Turn {n} · {title}")).clicked() {
                        s.ws_run = Some(rid.clone());
                    }
                }
            });
        });
        ScrollArea::vertical().id_salt("ws-scroll").auto_shrink([false; 2]).show(ui, |ui| {
            let mut shown = 0;
            for (n, turn, rid) in runs {
                if s.ws_run.as_ref().is_some_and(|r| r != &rid) {
                    continue;
                }
                if !s.details.contains_key(&rid) {
                    s.load_run(&rid, false);
                }
                let Some(d) = s.details.get(&rid).cloned() else { continue };
                let relevant = match s.ws_tab {
                    0 => d.has_patch() || d.summary.result.as_ref().is_some_and(|r| r.revertable),
                    1 => turn.run.as_ref().is_some_and(|r| r.mode.as_deref() == Some("agent")),
                    _ => true,
                };
                if !relevant {
                    continue;
                }
                shown += 1;
                super::group(ui, &t, |ui| {
                    let title: String = turn.prompt.lines().next().unwrap_or("").chars().take(90).collect();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Turn {n}")).size(T_SMALL).family(semibold()).color(t.text2));
                        ui.add(egui::Label::new(RichText::new(title).size(T_SMALL).color(t.text)).truncate());
                    });
                    match s.ws_tab {
                        0 => {
                            s.expanded.insert(format!("changes:{}", turn.id));
                            workspace(ui, v, s, drafts, &d, turn);
                        }
                        1 => {
                            s.expanded.insert(format!("outputs:{}", turn.id));
                            outputs(ui, v, s, turn);
                        }
                        _ => run_details(ui, v, &d, turn),
                    }
                });
            }
            if shown == 0 {
                ui.add_space(4.0 * SP);
                ui.vertical_centered(|ui| {
                    ui.label(ic(icons::INBOX, 28.0, t.text3));
                    ui.label(dim(&t, match s.ws_tab {
                        0 => "No run in this thread changed files.",
                        1 => "No Agent run in this thread produced files.",
                        _ => "Run details load when a run finishes.",
                    }));
                });
            }
        });
    });
    v.glass.fill_card(ui, slot, rect, R_MD);
}
