//! Thread list: a floating frosted panel (L1) with a Liquid Glass header (L2),
//! thread rows with a needs-you marker, and the accounts trigger at the bottom.

use super::glass::Kind;
use super::theme::{R_MD, R_SM, SP, T_BODY, T_SMALL, semibold};
use super::{View, basename, dim};
use crate::state::{Conn, State};
use egui::{
    Align, FontId, Id, Key, KeyboardShortcut, Layout, Modifiers, Rect, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui, UiBuilder, pos2,
    vec2,
};

pub struct Out {
    pub toggle_accounts: bool,
    pub accounts_anchor: Rect,
}

pub fn show(ui: &mut Ui, v: &mut View, s: &mut State, rect: Rect) -> Out {
    let t = v.t;
    v.glass.surface(ui, rect, R_MD, Kind::Card);
    let inner = rect.shrink(2.0 * SP);

    // header (chrome)
    let header = Rect::from_min_size(inner.min, vec2(inner.width(), 44.0));
    v.glass.surface(ui, header, R_SM + 4, Kind::Chrome);
    let mut new_clicked = false;
    ui.scope_builder(UiBuilder::new().max_rect(header.shrink2(vec2(3.0 * SP, 0.0))), |ui| {
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.label(RichText::new("Claudexor").family(semibold()).size(T_BODY + 1.0).color(t.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let b = egui::Button::new(RichText::new("+ New").color(t.on_accent).size(T_SMALL)).fill(t.accent_solid).corner_radius(10);
                if ui
                    .add_enabled(s.client.is_some(), b)
                    .on_hover_text("New thread (Ctrl+N)")
                    .on_disabled_hover_text("Engine offline")
                    .clicked()
                {
                    new_clicked = true;
                }
            });
        });
    });
    if new_clicked || ui.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::N))) {
        s.new_thread();
    }

    // accounts trigger (bottom)
    let foot = Rect::from_min_max(pos2(inner.min.x, inner.max.y - 40.0), inner.max);
    let list = Rect::from_min_max(pos2(inner.min.x, header.max.y + 2.0 * SP), pos2(inner.max.x, foot.min.y - SP));

    ui.scope_builder(UiBuilder::new().max_rect(list), |ui| {
        let offline_copy = !s.threads_loaded && !s.threads.is_empty();
        if offline_copy {
            ui.label(dim(&t, "Offline copy · read-only until the engine is back"));
        }
        if !s.threads_loaded && !offline_copy {
            ui.add_space(3.0 * SP);
            match &s.conn {
                Conn::Online { .. } => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(dim(&t, "Loading threads…"));
                    });
                }
                _ => {
                    ui.label(dim(&t, "Threads appear when the engine is online."));
                }
            }
            return;
        }
        // search + trash toggle
        let search_id = ui.id().with("thread-search");
        let mut query: String = ui.ctx().data(|d| d.get_temp(search_id)).unwrap_or_default();
        ui.horizontal(|ui| {
            let trash_label = if s.show_trash { "← Threads" } else { "Trash" };
            let w = ui.available_width() - 64.0;
            let r = ui.add(TextEdit::singleline(&mut query).hint_text("Search threads  (Ctrl+K)").desired_width(w));
            if ui.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::K))) {
                r.request_focus();
            }
            if ui.add(egui::Button::new(RichText::new(trash_label).size(T_SMALL - 1.0)).frame(false)).clicked() {
                s.show_trash = !s.show_trash;
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(search_id, query.clone()));
        ui.add_space(SP);

        let q = query.trim().to_lowercase();
        let visible: Vec<usize> = s
            .threads
            .iter()
            .enumerate()
            .filter(|(_, th)| th.trashed_at.is_some() == s.show_trash)
            .filter(|(_, th)| {
                q.is_empty()
                    || th.display_title().to_lowercase().contains(&q)
                    || th.repo_root.as_deref().is_some_and(|r| r.to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect();
        if visible.is_empty() {
            ui.add_space(2.0 * SP);
            let msg = match (s.show_trash, q.is_empty()) {
                (true, true) => "Trash is empty.",
                (false, true) => "No threads yet. Type below to start one.",
                _ => "No thread matches.",
            };
            ui.label(dim(&t, msg));
        }
        let mut pick = None;
        let mut act: Option<(String, Act)> = None;
        ScrollArea::vertical().id_salt("threads").auto_shrink([false; 2]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = SP;
            if s.selected.is_none() && !s.show_trash {
                row(ui, v, "New thread", "draft", true, false, false);
            }
            for &i in &visible {
                let th = &s.threads[i];
                let selected = s.selected.as_deref() == Some(&th.id);
                let mut sub = th.repo_root.as_deref().map(basename).unwrap_or("no project").to_string();
                let when = super::theme::ago(&th.updated_at);
                if !when.is_empty() {
                    sub = format!("{sub} · {when}");
                }
                let closed = th.state.as_deref() == Some("closed");
                if closed {
                    sub = format!("archived · {sub}");
                }
                let resp = row(ui, v, th.display_title(), &sub, selected, th.needs_human, closed);
                if resp.clicked() && !s.show_trash {
                    pick = Some(th.id.clone());
                }
                // offline rows are a cached copy: no actions that would silently no-op
                let resp = if s.client.is_some() { resp } else { resp.on_hover_text("Offline copy") };
                resp.context_menu(|ui| {
                    if s.client.is_none() {
                        ui.label(dim(&t, "Engine offline"));
                        return;
                    }
                    if s.show_trash {
                        if ui.button("Restore").clicked() {
                            act = Some((th.id.clone(), Act::Restore));
                            ui.close();
                        }
                        if ui.button(RichText::new("Delete forever…").color(t.failed)).clicked() {
                            act = Some((th.id.clone(), Act::ConfirmPurge(th.display_title().to_string())));
                            ui.close();
                        }
                    } else {
                        if ui.button("Rename…").clicked() {
                            act = Some((th.id.clone(), Act::Rename(th.display_title().to_string())));
                            ui.close();
                        }
                        let label = if closed { "Reopen" } else { "Archive" };
                        if ui.button(label).clicked() {
                            act = Some((th.id.clone(), Act::State(if closed { "active" } else { "closed" })));
                            ui.close();
                        }
                        if ui.button("Move to Trash").clicked() {
                            act = Some((th.id.clone(), Act::Trash));
                            ui.close();
                        }
                    }
                });
            }
        });
        if let Some(id) = pick {
            s.select(Some(id));
        }
        match act {
            Some((id, Act::Rename(title))) => {
                ui.ctx().data_mut(|d| d.insert_temp(Id::new("rename-dialog"), (id, title)));
            }
            Some((id, Act::ConfirmPurge(title))) => {
                ui.ctx().data_mut(|d| d.insert_temp(Id::new("purge-dialog"), (id, title)));
            }
            Some((id, Act::State(st))) => s.set_thread_state(&id, st),
            Some((id, Act::Trash)) => s.thread_action(&id, "trash"),
            Some((id, Act::Restore)) => s.thread_action(&id, "restore"),
            None => {}
        }
    });
    dialogs(ui, v, s);

    // accounts trigger: worst-readiness dot + label + worst quota %
    let resp = ui.interact(foot, ui.id().with("acct"), Sense::click());
    let painter = ui.painter();
    if resp.hovered() {
        painter.rect_filled(foot, R_SM, t.raised_hi.gamma_multiply(0.6));
    }
    let (worst, label) = quota_summary(s);
    let dot = match worst {
        Some(r) if r >= 0.9 => t.failed,
        Some(r) if r >= 0.7 => t.blocked,
        Some(_) => t.success,
        None => t.text3,
    };
    painter.circle_filled(pos2(foot.min.x + 14.0, foot.center().y), 4.0, dot);
    painter.text(pos2(foot.min.x + 26.0, foot.center().y), egui::Align2::LEFT_CENTER, label, FontId::proportional(T_SMALL), t.text2);
    painter.text(pos2(foot.max.x - 10.0, foot.center().y), egui::Align2::RIGHT_CENTER, "⌃", FontId::proportional(T_SMALL), t.text3);
    Out { toggle_accounts: resp.on_hover_text("Accounts & quota").clicked(), accounts_anchor: foot }
}

fn row(ui: &mut Ui, v: &View, title: &str, sub: &str, selected: bool, needs_you: bool, closed: bool) -> egui::Response {
    let t = v.t;
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 48.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, R_SM, t.accent_solid.gamma_multiply(if t.dark { 0.38 } else { 0.16 }));
        p.rect_stroke(rect, R_SM, Stroke::new(1.0_f32, t.accent.gamma_multiply(0.45)), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        p.rect_filled(rect, R_SM, t.raised_hi.gamma_multiply(0.55));
    }
    let x = rect.min.x + 3.0 * SP;
    let max_w = rect.width() - 6.0 * SP - if needs_you { 14.0 } else { 0.0 };
    let title_col = if closed { t.text3 } else { t.text };
    let tg = p.layout(title.to_string(), FontId::new(T_SMALL + 1.0, semibold()), title_col, max_w);
    let tg = truncate_one_line(ui, tg, title, FontId::new(T_SMALL + 1.0, semibold()), title_col, max_w);
    p.galley(pos2(x, rect.min.y + 7.0), tg, title_col);
    let sg = truncate_one_line(
        ui,
        p.layout_no_wrap(sub.to_string(), FontId::proportional(T_SMALL - 1.0), t.text3),
        sub,
        FontId::proportional(T_SMALL - 1.0),
        t.text3,
        max_w,
    );
    p.galley(pos2(x, rect.min.y + 27.0), sg, t.text3);
    if needs_you {
        p.circle_filled(pos2(rect.max.x - 12.0, rect.center().y), 4.5, t.needs_you);
    }
    let resp = if needs_you { resp.on_hover_text("Needs you") } else { resp };
    resp.on_hover_text(title)
}

/// Single line with an ellipsis, never wrapping into the next row.
fn truncate_one_line(
    ui: &Ui,
    g: std::sync::Arc<egui::Galley>,
    text: &str,
    font: FontId,
    color: egui::Color32,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    if g.size().x <= max_w && g.rows.len() <= 1 {
        return g;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
    ui.painter().layout_job(job)
}

/// Worst known quota usage across fresh snapshots, and the trigger label.
fn quota_summary(s: &State) -> (Option<f64>, String) {
    let Some(q) = &s.quota else {
        return (None, if s.client.is_some() { "Accounts".into() } else { "Accounts · offline".into() });
    };
    let accounts = q.accounts();
    let worst = accounts
        .iter()
        .flat_map(|sn| sn.constraints.iter().filter_map(|c| c.used_ratio))
        .fold(None, |m: Option<f64>, r| Some(m.map_or(r, |m| m.max(r))));
    let n = accounts.len();
    let label = match (n, worst) {
        (0, _) => "Accounts · no quota data".into(),
        (1, Some(w)) => format!("{} · {:.0}%", accounts[0].subject.harness, w * 100.0),
        (n, Some(w)) => format!("{n} accounts · worst {:.0}%", w * 100.0),
        (n, None) => format!("{n} accounts · usage unknown"),
    };
    (worst, label)
}

enum Act {
    Rename(String),
    State(&'static str),
    Trash,
    Restore,
    ConfirmPurge(String),
}

/// Rename and delete-forever dialogs (state kept in egui temp memory).
fn dialogs(ui: &mut Ui, v: &View, s: &mut State) {
    let t = v.t;
    let ctx = ui.ctx().clone();
    let rename_id = Id::new("rename-dialog");
    if let Some((id, mut title)) = ctx.data(|d| d.get_temp::<(String, String)>(rename_id)) {
        let mut done: Option<bool> = None;
        let r = egui::Modal::new(rename_id).show(&ctx, |ui| {
            ui.set_width(360.0);
            ui.label(RichText::new("Rename thread").family(semibold()).size(T_BODY + 1.0));
            let e = ui.add(TextEdit::singleline(&mut title).desired_width(f32::INFINITY));
            if !e.has_focus() && !e.lost_focus() {
                e.request_focus();
            }
            if e.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                done = Some(true);
            }
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("Rename").color(t.on_accent)).fill(t.accent_solid)).clicked() {
                    done = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    done = Some(false);
                }
            });
        });
        if r.should_close() {
            done = done.or(Some(false));
        }
        match done {
            Some(ok) => {
                ctx.data_mut(|d| d.remove::<(String, String)>(rename_id));
                if ok {
                    s.rename_thread(&id, &title);
                }
            }
            None => {
                ctx.data_mut(|d| d.insert_temp(rename_id, (id, title)));
            }
        }
    }
    let purge_id = Id::new("purge-dialog");
    if let Some((id, title)) = ctx.data(|d| d.get_temp::<(String, String)>(purge_id)) {
        let mut done: Option<bool> = None;
        let r = egui::Modal::new(purge_id).show(&ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new("Delete forever?").family(semibold()).size(T_BODY + 1.0));
            ui.label(RichText::new(format!("“{title}” and its turns are purged permanently. This cannot be undone.")).color(t.text2));
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("Delete forever").color(t.on_accent)).fill(t.failed)).clicked() {
                    done = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    done = Some(false);
                }
            });
        });
        if r.should_close() {
            done = done.or(Some(false));
        }
        if let Some(ok) = done {
            ctx.data_mut(|d| d.remove::<(String, String)>(purge_id));
            if ok {
                s.thread_action(&id, "purge");
            }
        }
    }
}
