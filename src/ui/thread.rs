//! Conversation: past turns + live run timeline (PLAN §4 "Timeline").
//!
//! Each turn reads top-down: the user's bubble, then the assistant card —
//! final answer (markdown on a solid inset), ONE receipt row (status · harness
//! · state · elapsed · tools · chevron) that toggles the activity transcript,
//! question cards when the run waits on you, and the refused/failed card when
//! honest. Dense text (answer, activity, code) always sits on solids.

use super::glass::Kind;
use super::theme::{MEASURE, R_MD, R_SM, SP, T_BODY, T_SMALL, T_TITLE, Theme, semibold, span};
use super::{View, chip, dim, last_rect, store_rect};
use crate::model::{Answer, Interaction, Turn};
use crate::state::{AnswerState, Conn, State, failure_line};
use crate::transcript::{Block, ToolStatus};
use egui::{Align, Color32, Frame, Id, Layout, Margin, Rect, RichText, ScrollArea, Sense, Stroke, Ui, UiBuilder};
use std::collections::HashMap;

/// UI-local drafts for interactive answers: (interaction, question) → (picked, free text).
#[derive(Default)]
pub struct Drafts {
    pub answers: HashMap<(String, String), (Vec<String>, String)>,
    /// Plan answers: (plan run, question) → (picked option ids, own words).
    pub plan: HashMap<(String, String), (Vec<String>, String)>,
    /// Plan runs whose answers were just sent (until the follow-up turn shows up).
    pub plan_sent: std::collections::HashSet<String>,
}

const MAX_ROWS: usize = 80;

pub fn show(ui: &mut Ui, v: &mut View, s: &mut State, drafts: &mut Drafts, rect: Rect, top_pad: f32, bottom_pad: f32) {
    let t = v.t;
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        let col_w = (rect.width() - 8.0 * SP).min(MEASURE).max(280.0);
        ScrollArea::vertical().id_salt(("conv", s.selected.clone())).stick_to_bottom(true).auto_shrink([false; 2]).show(ui, |ui| {
            let pad = ((ui.available_width() - col_w) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.vertical(|ui| {
                    ui.set_width(col_w);
                    ui.add_space(top_pad);
                    body(ui, v, s, drafts, &t);
                    ui.add_space(bottom_pad);
                });
            });
        });
    });
}

fn body(ui: &mut Ui, v: &mut View, s: &mut State, drafts: &mut Drafts, t: &Theme) {
    match (&s.conn, s.selected.is_some()) {
        (Conn::Offline(why), _) if s.detail.is_none() => {
            let why = why.clone();
            empty(ui, v, "Engine offline", &why, true);
            return engine_button(ui, v, s);
        }
        (Conn::Incompatible(why), _) => return empty(ui, v, "Incompatible engine", why, true),
        (Conn::Connecting, _) if s.detail.is_none() => {
            return empty(ui, v, "Connecting…", "Looking for the local Claudexor engine.", false);
        }
        (Conn::Recovering, _) if s.detail.is_none() => {
            return empty(
                ui,
                v,
                "Engine is recovering",
                "The daemon is serving its journal-recovery plane only. It opens normally once recovery completes.",
                false,
            );
        }
        (_, false) => {
            let hint = match s.composer.project.as_deref() {
                Some(root) => format!("New thread in {root}. Ask, plan, or let an agent work."),
                None => "No project selected: this thread will be Ask-only. Pick a project in the composer for Plan and Agent.".into(),
            };
            return empty(ui, v, "What should we work on?", &hint, false);
        }
        _ => {}
    }
    let Some(detail) = s.detail.clone() else {
        ui.add_space(8.0 * SP);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(dim(t, "Loading conversation…"));
        });
        return;
    };
    // title
    ui.label(RichText::new(detail.thread.display_title()).family(semibold()).size(T_TITLE).color(t.text));
    let mut meta = vec![detail.thread.repo_root.clone().unwrap_or_else(|| "no project (Ask only)".into())];
    if let Some(h) = &detail.thread.primary_harness {
        meta.push(h.clone());
    }
    if let Some(m) = &detail.thread.workspace_mode {
        meta.push(m.replace('_', "-"));
    }
    ui.label(dim(t, meta.join(" · ")));
    ui.add_space(4.0 * SP);
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

fn empty(ui: &mut Ui, v: &View, title: &str, text: &str, command_hint: bool) {
    let t = v.t;
    ui.add_space(ui.available_height().max(0.0) * 0.18 + 80.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).family(semibold()).size(T_TITLE + 6.0).color(t.text));
        ui.add_space(2.0 * SP);
        ui.label(RichText::new(text).color(t.text2).size(T_BODY));
        if command_hint {
            ui.add_space(3.0 * SP);
            code_line(ui, v, "claudexor daemon start");
            ui.add_space(SP);
            ui.label(dim(&t, "This window reconnects on its own."));
        }
    });
}

fn code_line(ui: &mut Ui, v: &View, text: &str) {
    Frame::new().fill(v.t.code).corner_radius(10).inner_margin(Margin::symmetric(12, 6)).stroke(Stroke::new(1.0_f32, v.t.separator)).show(
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
        Frame::new().fill(t.user_bubble).corner_radius(R_MD).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
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

    // 2. assistant card (frosted; last frame's rect, contract #3)
    let card_id = Id::new(("card", &turn.id));
    if let Some(r) = last_rect(ui, card_id) {
        v.glass.surface(ui, r, R_MD, Kind::Card);
    }
    let resp = Frame::new().inner_margin(Margin::same(14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        // refused turn: typed problem + retry
        if let Some(err) = &turn.enqueue_error {
            if turn.run.is_none() {
                refused(ui, v, s, turn, err);
                return;
            }
        }
        // final answer
        if let Some(rid) = &turn.run_id {
            match s.answers.get(rid).cloned() {
                Some(AnswerState::Ready(text)) => answer_bubble(ui, v, &text),
                Some(AnswerState::Loading) if terminal => {
                    ui.horizontal(|ui| {
                        ui.spinner();
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
                ui.add_space(SP);
                Frame::new().fill(t.failed.gamma_multiply(0.12)).corner_radius(R_SM).inner_margin(Margin::symmetric(12, 8)).show(
                    ui,
                    |ui| {
                        ui.set_width(ui.available_width());
                        if let Some(line) = failure_line(&f) {
                            ui.add(egui::Label::new(RichText::new(line).color(t.text).size(T_SMALL + 1.0)).wrap().selectable(true));
                        }
                        for a in &f.next_actions {
                            ui.label(RichText::new(format!("• {a}")).color(t.text2).size(T_SMALL));
                        }
                        if let Some(at) = &f.resets_at {
                            ui.label(dim(&t, format!("window reopens {}", super::theme::until(at))));
                        }
                        if let Some(v) = &f.vendor_failure {
                            let parts: Vec<&str> = [v.code.as_deref(), v.message.as_deref()].into_iter().flatten().collect();
                            if !parts.is_empty() {
                                ui.label(dim(&t, format!("vendor: {}", parts.join(" · "))));
                            }
                        }
                        if let Some(phase) = &f.phase {
                            ui.label(dim(&t, format!("phase: {phase}")));
                        }
                    },
                );
            }
        }
        // 3. receipt row
        ui.add_space(SP);
        let expanded_key = turn.id.clone();
        let expanded = if active { !s.collapsed.contains(&expanded_key) } else { s.expanded.contains(&expanded_key) };
        if receipt(ui, v, s, turn, &state, harness, expanded).clicked() {
            if active {
                if !s.collapsed.remove(&expanded_key) {
                    s.collapsed.insert(expanded_key.clone());
                }
            } else if !s.expanded.remove(&expanded_key) {
                s.expanded.insert(expanded_key.clone());
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
    store_rect(ui, card_id, resp.response.rect);
}

fn answer_bubble(ui: &mut Ui, v: &mut View, text: &str) {
    let t = v.t;
    let r = Frame::new().fill(t.raised_hi).corner_radius(R_SM).inner_margin(Margin { left: 16, right: 14, top: 12, bottom: 12 }).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            egui_commonmark::CommonMarkViewer::new().show(ui, v.md, text);
        },
    );
    // 2 pt accent leading edge (the answer is the loudest element in the feed),
    // painted after layout so it spans the real height, inside the left padding.
    let rect = r.response.rect;
    let x = rect.min.x + 6.0;
    ui.painter().line_segment([egui::pos2(x, rect.min.y + 10.0), egui::pos2(x, rect.max.y - 10.0)], Stroke::new(2.0_f32, t.accent));
}

fn receipt(ui: &mut Ui, v: &View, s: &State, turn: &Turn, state: &str, harness: Option<&str>, expanded: bool) -> egui::Response {
    let t = v.t;
    let live = s.live_of(turn);
    let inner = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0 * SP;
        let (color, glyph, word) = t.status(state);
        let running = state == "running";
        let pulse = if running {
            // low-frequency pulse only while running; static otherwise (no idle animation)
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
            0.55 + 0.45 * ((ui.input(|i| i.time) * 2.4).sin() as f32 * 0.5 + 0.5)
        } else {
            1.0
        };
        ui.label(RichText::new(glyph).color(color.gamma_multiply(pulse)).size(T_SMALL + 1.0));
        // The route that actually ran (harness + stream-observed model) beats the thread default.
        let route = turn.run_id.as_ref().and_then(|r| s.details.get(r)).and_then(|d| d.summary.route.as_ref());
        let h = route.and_then(|r| r.harness_id.as_deref()).or(harness).unwrap_or("auto");
        chip(ui, &t, h, t.harness(h));
        ui.label(RichText::new(word).color(t.text2).size(T_SMALL));
        if let Some(r) = route {
            match (r.observed_model.as_deref(), r.requested_model.as_deref()) {
                (Some(obs), Some(req)) if obs != req => {
                    ui.label(RichText::new(format!("{obs} (asked {req})")).color(t.blocked).size(T_SMALL))
                        .on_hover_text("The vendor answered with a different model than requested");
                }
                (Some(obs), _) => {
                    let tip = if r.verified { "Model observed in the vendor stream" } else { "Model reported, not verified" };
                    ui.label(dim(&t, obs)).on_hover_text(tip);
                }
                (None, Some(req)) => {
                    ui.label(dim(&t, format!("{req} (requested)"))).on_hover_text("No model observed in the stream");
                }
                _ => {}
            }
        }
        if let Some(run) = &turn.run {
            if let Some(mode) = &run.mode {
                ui.label(dim(&t, mode));
            }
        }
        // elapsed
        let secs = match live {
            Some(l) if l.terminal.is_none() && state != "queued" => Some(l.started.elapsed().as_secs() as i64),
            _ => {
                let start = super::theme::parse_iso(&turn.created_at);
                let end = turn.run.as_ref().and_then(|r| r.finished_at.as_deref()).and_then(super::theme::parse_iso);
                start.zip(end).map(|(a, b)| b - a)
            }
        };
        if let Some(sec) = secs.filter(|x| *x >= 0) {
            ui.label(dim(&t, span(sec)));
            if live.is_some_and(|l| l.terminal.is_none()) {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
            }
        }
        if let Some(usd) = turn.run.as_ref().and_then(|r| r.spend_usd) {
            ui.label(dim(&t, format!("${usd:.2}")));
        }
        if let Some(l) = live {
            let n = l.transcript.tool_count();
            if n > 0 {
                ui.label(dim(&t, format!("{n} tool{}", if n == 1 { "" } else { "s" })));
            }
        }
        if let Some(ds) = turn.run.as_ref().and_then(|r| r.result.as_ref()).and_then(|r| r.diff_stat.as_ref()) {
            if ds.files > 0 {
                ui.label(RichText::new(format!("{} files +{} −{}", ds.files, ds.additions, ds.deletions)).color(t.text3).size(T_SMALL));
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(dim(&t, if expanded { "▼ activity" } else { "▶ activity" }));
        });
    });
    ui.interact(inner.response.rect, ui.id().with(("receipt", &turn.id)), Sense::click()).on_hover_text(if expanded {
        "Hide activity"
    } else {
        "Show activity"
    })
}

fn activity(ui: &mut Ui, v: &View, s: &State, turn: &Turn) {
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
    Frame::new().fill(t.code).corner_radius(R_SM).inner_margin(Margin::symmetric(12, 10)).stroke(Stroke::new(1.0_f32, t.separator)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            let rows = fold_rows(&tr.blocks);
            let skip = rows.len().saturating_sub(MAX_ROWS);
            if tr.trimmed + skip > 0 {
                ui.label(dim(&t, format!("{} earlier items not shown", tr.trimmed + skip)));
            }
            for row in rows.into_iter().skip(skip) {
                match row {
                    Row::Thinking(text) => {
                        let tail: String = text.chars().rev().take(280).collect::<Vec<_>>().into_iter().rev().collect();
                        ui.label(RichText::new("◑ Thinking").color(t.text3).size(T_SMALL));
                        ui.add(egui::Label::new(RichText::new(tail.trim()).color(t.text3).italics().size(T_SMALL - 1.0)).wrap());
                    }
                    Row::Message(text) => {
                        let head: String = text.chars().take(4000).collect();
                        ui.add(egui::Label::new(RichText::new(head).color(t.text2).size(T_SMALL)).wrap().selectable(true));
                    }
                    Row::Tool(tool) => tool_row(ui, &t, tool),
                    Row::Group(name, n) => {
                        ui.label(RichText::new(format!("✔ {name} · {n} calls")).color(t.text3).size(T_SMALL));
                    }
                }
            }
            if tr.truncated_chars > 0 {
                ui.label(dim(&t, format!("{} characters clipped from overlong items", tr.truncated_chars)));
            }
        },
    );
}

enum Row<'a> {
    Thinking(&'a str),
    Message(&'a str),
    Tool(&'a crate::transcript::Tool),
    Group(&'a str, usize),
}

/// Runs of more than three consecutive same-name OK tools collapse into one row.
fn fold_rows(blocks: &[Block]) -> Vec<Row<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < blocks.len() {
        match &blocks[i] {
            Block::Thinking { text, .. } => out.push(Row::Thinking(text)),
            Block::Message { text, .. } => out.push(Row::Message(text)),
            Block::Tool { tool, .. } => {
                let mut j = i;
                while j + 1 < blocks.len()
                    && matches!(&blocks[j + 1], Block::Tool { tool: n, .. } if n.name == tool.name && n.status == ToolStatus::Ok)
                {
                    j += 1;
                }
                let run = j - i + 1;
                if run > 3 && tool.status == ToolStatus::Ok {
                    out.push(Row::Group(&tool.name, run));
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

fn tool_row(ui: &mut Ui, t: &Theme, tool: &crate::transcript::Tool) {
    let (glyph, color) = match tool.status {
        ToolStatus::Running => ("○", t.running),
        ToolStatus::Ok => ("✔", t.success),
        ToolStatus::Error => ("✖", t.failed),
    };
    // a command shows its binary's basename; the full target is the subtitle
    let title = match (tool.kind.as_deref(), tool.target.as_deref()) {
        (Some("command") | Some("shell") | Some("exec"), Some(cmd)) => {
            cmd.split_whitespace().next().map(super::basename).unwrap_or(&tool.name).to_string()
        }
        _ => tool.name.clone(),
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new(glyph).color(color).size(T_SMALL));
        ui.label(RichText::new(title).color(t.text).size(T_SMALL));
        if let Some(target) = &tool.target {
            let one: String = target.lines().next().unwrap_or("").chars().take(160).collect();
            ui.add(egui::Label::new(RichText::new(one).monospace().color(t.text3).size(T_SMALL - 1.0)).truncate()).on_hover_text(target);
        }
    });
    if tool.status == ToolStatus::Error {
        let why = tool.detail.clone().or_else(|| tool.exit_code.map(|c| format!("exit {c}"))).unwrap_or_default();
        if !why.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                ui.add(
                    egui::Label::new(RichText::new(why).color(t.failed.gamma_multiply(0.9)).size(T_SMALL - 1.0)).wrap().selectable(true),
                );
            });
        }
    }
}

fn refused(ui: &mut Ui, v: &View, s: &mut State, turn: &Turn, err: &crate::model::EnqueueError) {
    let t = v.t;
    Frame::new()
        .fill(t.failed.gamma_multiply(0.10))
        .corner_radius(R_SM)
        .inner_margin(Margin::symmetric(12, 10))
        .stroke(Stroke::new(1.0_f32, t.failed.gamma_multiply(0.35)))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("⊘ Turn refused").color(t.failed).family(semibold()).size(T_SMALL + 1.0));
                if let Some(code) = &err.code {
                    ui.label(dim(&t, code));
                }
            });
            ui.add(egui::Label::new(RichText::new(&err.message).color(t.text).size(T_SMALL + 1.0)).wrap().selectable(true));
            for a in &err.required_actions {
                ui.label(RichText::new(format!("• {a}")).color(t.text2).size(T_SMALL));
            }
            if err.retryable != Some(false) {
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
            ui.label(RichText::new("? The agent is asking").color(t.needs_you).family(semibold()).size(T_SMALL + 1.0));
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
                ui.spinner();
                ui.label(dim(&t, "Starting the engine…"));
            });
        } else {
            let b = egui::Button::new(RichText::new("Start engine").color(t.on_accent)).fill(t.accent_solid).corner_radius(10);
            if ui.add(b).on_hover_text("Runs `claudexor daemon start`").clicked() {
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
