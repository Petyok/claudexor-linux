//! Claudexor for Linux — a native egui client for the Claudexor daemon.
//!
//! `claudexor-linux`                → the app
//! `claudexor-linux --send PROMPT`  → create an Ask thread, send one turn, follow it
//! `claudexor-linux --upload FILE`  → push one file through the attachment pipeline
//! `claudexor-linux --record DIR`   → dump live read-only API responses + one
//!                                    run SSE log as fixtures (never the token)

mod api;
mod model;
mod sse;
mod state;
mod transcript;
mod ui;

use eframe::egui;
use egui::{Color32, FontId, Id, Rect, RichText, Sense, Stroke, UiBuilder, pos2, vec2};
use state::{Conn, State};
use ui::glass::Glass;
use ui::theme::{self, R_SM, SP, T_SMALL, Theme};
use ui::{View, last_rect};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ThemePref {
    System,
    Light,
    Dark,
}

struct Prefs {
    theme: ThemePref,
    reduce_transparency: bool,
    /// Restored between launches (the compositor owns the position on Wayland).
    window: Option<(f32, f32)>,
    sidebar_w: f32,
    last_project: Option<String>,
}

impl Prefs {
    fn path() -> Option<std::path::PathBuf> {
        dirs::config_dir().map(|d| d.join("claudexor-linux/prefs"))
    }
    fn load() -> Prefs {
        let raw = Self::path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        let get = |k: &str| raw.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix('=').map(str::trim).map(str::to_owned));
        Prefs {
            theme: match get("theme").as_deref() {
                Some("light") => ThemePref::Light,
                Some("dark") => ThemePref::Dark,
                _ => ThemePref::System,
            },
            reduce_transparency: get("reduce_transparency").as_deref() == Some("1")
                || std::env::var_os("CLAUDEXOR_REDUCE_TRANSPARENCY").is_some(),
            window: get("window").and_then(|w| {
                let (a, b) = w.split_once('x')?;
                Some((a.parse::<f32>().ok()?.clamp(640.0, 4000.0), b.parse::<f32>().ok()?.clamp(480.0, 3000.0)))
            }),
            sidebar_w: get("sidebar").and_then(|w| w.parse::<f32>().ok()).unwrap_or(ui::theme::SIDEBAR_W).clamp(200.0, 420.0),
            last_project: get("project").filter(|p| p.starts_with('/')),
        }
    }
    fn save(&self) {
        let Some(p) = Self::path() else { return };
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
        let theme = match self.theme {
            ThemePref::System => "system",
            ThemePref::Light => "light",
            ThemePref::Dark => "dark",
        };
        let mut out = format!("theme={theme}\nreduce_transparency={}\nsidebar={:.0}\n", self.reduce_transparency as u8, self.sidebar_w);
        if let Some((w, h)) = self.window {
            out.push_str(&format!("window={w:.0}x{h:.0}\n"));
        }
        if let Some(p) = &self.last_project {
            out.push_str(&format!("project={p}\n"));
        }
        let _ = std::fs::write(p, out);
    }
}

struct App {
    /// Desktop colour scheme from the portal: 0 unknown · 1 dark · 2 light.
    desktop_dark: std::sync::Arc<std::sync::atomic::AtomicU8>,
    /// Follow-up frames still to paint after input or state changes.
    settle: u8,
    state: State,
    glass: Glass,
    md: egui_commonmark::CommonMarkCache,
    drafts: ui::thread::Drafts,
    prefs: Prefs,
    applied_dark: Option<bool>,
    accounts_open: bool,
    accounts_opened_at: u64,
    accounts_anchor: egui::Pos2,
    settings_open: bool,
    settings_tab: ui::settings::Tab,
    stats: Option<ui::stats::FrameStats>,
    last_frame: std::time::Instant,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        // decodes run-output previews handed over as bytes (PNG/JPEG only)
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let prefs = Prefs::load();
        let (frost, refract) = match cc.gl.as_ref() {
            Some(gl) => {
                if ui::stats::FrameStats::enabled() {
                    ui::stats::FrameStats::log_renderer(gl);
                }
                let frost = (std::env::var_os("CXL_NO_FROST").is_none())
                    .then(|| backdrop_blur_egui::GrabPassRenderer::new(gl))
                    .transpose()
                    .map_err(|e| eprintln!("glass: frosted panels unavailable ({e:?}); using solid surfaces"))
                    .ok()
                    .flatten();
                let refract = (std::env::var_os("CXL_NO_REFRACT").is_none())
                    .then(|| ui::refract::Refractor::new(gl))
                    .transpose()
                    .map_err(|e| eprintln!("glass: refraction shader unavailable ({e}); chrome uses frost"))
                    .ok()
                    .flatten();
                (frost, refract)
            }
            None => (None, None),
        };
        let mut state = State::new(cc.egui_ctx.clone());
        state.composer.project = prefs.last_project.clone();
        let desktop_dark = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(desktop_scheme()));
        watch_desktop_scheme(desktop_dark.clone(), cc.egui_ctx.clone());
        App {
            desktop_dark,
            settle: 0,
            state,
            glass: Glass { frost, refract, reduce_transparency: prefs.reduce_transparency, theme: theme::DARK },
            md: Default::default(),
            drafts: Default::default(),
            prefs,
            applied_dark: None,
            accounts_open: false,
            settings_open: false,
            settings_tab: Default::default(),
            accounts_opened_at: 0,
            accounts_anchor: egui::Pos2::ZERO,
            stats: ui::stats::FrameStats::enabled().then(Default::default),
            last_frame: std::time::Instant::now(),
        }
    }

    fn sync_theme(&mut self, ctx: &egui::Context) {
        let dark = match self.prefs.theme {
            ThemePref::Dark => true,
            ThemePref::Light => false,
            // winit gives no desktop theme on X11 and only the app's own on
            // Wayland: ask the desktop (portal / gsettings), then winit, then dark
            ThemePref::System => match self.desktop_dark.load(std::sync::atomic::Ordering::Relaxed) {
                1 => true,
                2 => false,
                _ => ctx.system_theme().is_none_or(|t| t == egui::Theme::Dark),
            },
        };
        self.glass.reduce_transparency = self.prefs.reduce_transparency;
        if self.applied_dark != Some(dark) {
            let t = Theme::for_dark(dark);
            t.apply(ctx);
            self.glass.theme = t;
            self.applied_dark = Some(dark);
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0] // transparent: L0 lets the compositor blur show through
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // 60 fps cap. vsync is off (it froze the UI while hidden), so without this
        // every pointer event from a 500-1000 Hz mouse painted a full glass frame and
        // ran the iGPU hot. Idle stays at 0 frames: this only paces requested ones.
        let min_frame = std::time::Duration::from_micros(16_667);
        let since = self.last_frame.elapsed();
        if since < min_frame && self.stats.is_none() {
            std::thread::sleep(min_frame - since);
        }
        self.last_frame = std::time::Instant::now();
        let ctx = ui.ctx().clone();
        if let Some(st) = &self.stats {
            st.frame(&ctx, frame.info().cpu_usage.map(|s| s * 1000.0), ui.max_rect());
        }
        if std::env::var_os("CXL_REPAINT_DEBUG").is_some() {
            let causes = ctx.repaint_causes();
            if !causes.is_empty() {
                eprintln!("repaint #{}: {causes:?}", ctx.cumulative_frame_nr());
                let evs: Vec<String> = ctx.input(|i| i.raw.events.iter().map(|e| format!("{e:?}").chars().take(90).collect()).collect());
                if !evs.is_empty() {
                    eprintln!("  events: {evs:?}");
                }
            }
        }
        // Widgets react to a click after they've been drawn, and popovers/panels
        // size themselves over a couple of frames, so after input or an engine
        // message paint a short settle burst. Idle still ends at 0 frames.
        let changed = self.state.pump();
        if changed || ctx.input(|i| !i.events.is_empty()) {
            self.settle = 3;
        }
        if self.settle > 0 {
            self.settle -= 1;
            ctx.request_repaint();
        }
        // Desktop notifications only when the user is elsewhere (window unfocused).
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        for n in self.state.notices.drain(..) {
            if !focused {
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    // --action waits for the click; "default" = the notification body
                    let out = std::process::Command::new("notify-send")
                        .args(["--app-name=Claudexor", "--icon=claudexor-linux", "--action=default=Open", &n.title, &n.body])
                        .output();
                    if out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "default") {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                });
            }
        }
        // Alt+Up / Alt+Down: previous / next thread in the sidebar order.
        let step = ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowUp) {
                -1
            } else if i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowDown) {
                1
            } else {
                0
            }
        });
        if step != 0 {
            let live: Vec<String> = self.state.threads.iter().filter(|t| t.trashed_at.is_none()).map(|t| t.id.clone()).collect();
            if !live.is_empty() {
                let cur = self.state.selected.as_ref().and_then(|s| live.iter().position(|x| x == s));
                let next = match cur {
                    Some(i) => (i as i64 + step).clamp(0, live.len() as i64 - 1) as usize,
                    None => 0,
                };
                self.state.select(Some(live[next].clone()));
            }
        }
        self.sync_theme(&ctx);
        let t = self.glass.theme;
        let full = ui.max_rect();
        self.glass.backdrop(ui.painter(), full);

        let g = 3.0 * SP;
        let side = Rect::from_min_size(full.min + vec2(g, g), vec2(self.prefs.sidebar_w, full.height() - 2.0 * g));
        // drag the sidebar's right edge to resize it (saved on exit)
        let handle = Rect::from_min_max(pos2(side.max.x - 2.0, side.min.y + 40.0), pos2(side.max.x + 6.0, side.max.y - 40.0));
        let drag = ui.interact(handle, Id::new("sidebar-resize"), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if drag.dragged() {
            self.prefs.sidebar_w = (self.prefs.sidebar_w + drag.drag_delta().x).clamp(200.0, 420.0);
        }
        // remember the window size for the next launch
        if let Some(r) = ctx.input(|i| i.viewport().inner_rect) {
            self.prefs.window = Some((r.width(), r.height()));
        }
        let main = Rect::from_min_max(pos2(side.max.x + g, full.min.y), full.max);

        let mut view = View { glass: &self.glass, md: &mut self.md, t };

        // Ctrl+. toggles the workspace side panel (a trailing panel, never a tab)
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Period)) && self.state.selected.is_some() {
            self.state.ws_open = !self.state.ws_open;
        }
        let ws = self.state.ws_open && self.state.detail.is_some();
        let (conv, side_ws) = if ws {
            let w = (main.width() * 0.42).clamp(340.0, 440.0);
            (
                Rect::from_min_max(main.min, pos2(main.max.x - w - g, main.max.y)),
                Some(Rect::from_min_max(pos2(main.max.x - w - g, main.min.y + g + 44.0), pos2(main.max.x - g, main.max.y - g))),
            )
        } else {
            (main, None)
        };
        // Conversation first: the composer's glass then frosts/refracts it.
        let composer_h = last_rect(ui, Id::new("composer")).map_or(118.0, |r| r.height());
        ui::thread::show(ui, &mut view, &mut self.state, &mut self.drafts, conv, 56.0, composer_h + 8.0 * SP);
        ui::composer::show(ui, &mut view, &mut self.state, conv);
        if let Some(r) = side_ws {
            ui::thread::workspace_side(ui, &mut view, &mut self.state, &mut self.drafts, r);
        }
        let side_out = ui::sidebar::show(ui, &mut view, &mut self.state, side);
        if side_out.toggle_accounts {
            self.accounts_open = !self.accounts_open;
            self.accounts_opened_at = ctx.cumulative_frame_nr();
            self.accounts_anchor = side_out.accounts_anchor.left_top() + vec2(0.0, -SP);
            if self.accounts_open {
                self.state.refresh_quota(false);
            }
        }

        // One minimal toolbar (DESIGN_SYSTEM §5): an icon cluster, no engine
        // capsule. Connection state appears only when it isn't simply online.
        let bar = Rect::from_min_size(pos2(main.max.x - 3.0 * 30.0 - 3.0 * SP, main.min.y + g), vec2(3.0 * 30.0, 30.0));
        ui.scope_builder(UiBuilder::new().max_rect(bar), |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if ui::icon_button(ui, &t, ui::icons::SETTINGS, "Settings").clicked() {
                    self.settings_open = true;
                    self.state.load_settings();
                }
                if self.state.selected.is_some() {
                    let tip = if self.state.ws_open { "Hide the workspace (Ctrl+.)" } else { "Workspace: changes, outputs, evidence (Ctrl+.)" };
                    if ui::icon_button(ui, &t, ui::icons::PANEL_RIGHT, tip).clicked() {
                        self.state.ws_open = !self.state.ws_open;
                    }
                }
                let (icon, next, tip) = match self.prefs.theme {
                    ThemePref::System => (ui::icons::SUN_MOON, ThemePref::Light, "Appearance: system (click for light)"),
                    ThemePref::Light => (ui::icons::SUN, ThemePref::Dark, "Appearance: light (click for dark)"),
                    ThemePref::Dark => (ui::icons::MOON, ThemePref::System, "Appearance: dark (click for system)"),
                };
                if ui::icon_button(ui, &t, icon, tip).clicked() {
                    self.prefs.theme = next;
                    self.prefs.save();
                    self.applied_dark = None;
                }
            });
        });
        let conn_line = match &self.state.conn {
            Conn::Online { .. } => None,
            Conn::Connecting => Some((t.queued, "Connecting to the engine…".to_string())),
            Conn::Recovering => Some((t.blocked, "The engine is recovering its journal".to_string())),
            Conn::Offline(why) => Some((t.failed, format!("Engine offline · {why}"))),
            Conn::Incompatible(why) => Some((t.failed, format!("Incompatible engine · {why}"))),
        };
        if let Some((c, text)) = conn_line {
            let w = (main.width() * 0.5).min(420.0);
            let r = Rect::from_min_size(pos2(bar.min.x - w - 2.0 * SP, bar.min.y + 3.0), vec2(w, 24.0));
            ui.scope_builder(UiBuilder::new().max_rect(r), |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(text).size(T_SMALL).color(t.text2)).truncate());
                    let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, c);
                });
            });
        }

        // error banner: wraps up to three lines, selectable, copy + dismiss
        if let Some(err) = self.state.error.clone() {
            let w = (main.width() - 16.0 * SP).min(640.0);
            let top = main.min.y + g + 40.0;
            let area = Rect::from_min_size(pos2(main.center().x - w / 2.0, top), vec2(w, 90.0));
            let bid = Id::new("error-banner");
            let h = ui.ctx().data(|d| d.get_temp::<f32>(bid)).unwrap_or(40.0);
            let r = Rect::from_min_size(area.min, vec2(w, h));
            ui.painter().rect_filled(r, R_SM, t.failed.gamma_multiply(if t.dark { 0.28 } else { 0.14 }).to_opaque().lerp_to_gamma(t.overlay, 0.35));
            ui.painter().rect_stroke(r, R_SM, Stroke::new(1.0_f32, t.failed.gamma_multiply(0.6)), egui::StrokeKind::Inside);
            let used = ui
                .scope_builder(UiBuilder::new().max_rect(area.shrink2(vec2(12.0, 8.0))), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(ui::icons::TRIANGLE_ALERT).size(14.0).color(t.failed));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            if ui::icon_button(ui, &t, ui::icons::X, "Dismiss").clicked() {
                                self.state.error = None;
                            }
                            if ui::icon_button(ui, &t, ui::icons::COPY, "Copy message").clicked() {
                                ui.ctx().copy_text(err.clone());
                            }
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                                // wrap inside the banner: its width minus padding, the alert icon and the two buttons
                                let text_w = (w - 24.0 - 22.0 - 2.0 * 30.0).max(120.0);
                                ui.set_max_width(text_w);
                                let mut job = egui::text::LayoutJob::single_section(err.clone(), egui::TextFormat::simple(FontId::proportional(T_SMALL), t.text));
                                job.wrap = egui::text::TextWrapping { max_rows: 3, max_width: text_w, ..Default::default() };
                                ui.add(egui::Label::new(job).selectable(true)).on_hover_text(&err);
                            });
                        });
                    });
                })
                .response
                .rect;
            ui.ctx().data_mut(|d| d.insert_temp(bid, used.height() + 16.0));
        }

        // onboarding's "Set up an account" opens the accounts popover
        if std::mem::take(&mut self.state.want_accounts) {
            self.accounts_open = true;
            self.accounts_opened_at = ctx.cumulative_frame_nr();
            self.accounts_anchor = side_out.accounts_anchor.left_top() + vec2(0.0, -SP);
            self.state.refresh_quota(false);
        }
        {
            let mut view = View { glass: &self.glass, md: &mut self.md, t };
            ui::thread::viewer(&ctx, &mut view, &mut self.state);
        }
        if self.settings_open {
            let mut view = View { glass: &self.glass, md: &mut self.md, t };
            let mut look = ui::settings::Appearance {
                theme: match self.prefs.theme {
                    ThemePref::System => 0,
                    ThemePref::Light => 1,
                    ThemePref::Dark => 2,
                },
                reduce_transparency: self.prefs.reduce_transparency,
            };
            self.settings_open = ui::settings::show(&ctx, &mut view, &mut self.state, &mut self.settings_tab, &mut look);
            let theme = match look.theme {
                1 => ThemePref::Light,
                2 => ThemePref::Dark,
                _ => ThemePref::System,
            };
            if theme != self.prefs.theme || look.reduce_transparency != self.prefs.reduce_transparency {
                self.prefs.theme = theme;
                self.prefs.reduce_transparency = look.reduce_transparency;
                self.prefs.save();
                self.applied_dark = None;
            }
        }
        if self.accounts_open {
            let mut view = View { glass: &self.glass, md: &mut self.md, t };
            let keep = ui::accounts::show(&ctx, &mut view, &mut self.state, self.accounts_anchor);
            if !keep && ctx.cumulative_frame_nr() > self.accounts_opened_at {
                self.accounts_open = false;
            }
        }
        // Hyperlinks (markdown answers): open through our worker-thread xdg-open
        // instead of eframe's handler, which runs on the UI thread.
        let urls: Vec<String> = ctx.output_mut(|o| {
            let mut urls = vec![];
            o.commands.retain(|c| match c {
                egui::OutputCommand::OpenUrl(u) => {
                    urls.push(u.url.clone());
                    false
                }
                _ => true,
            });
            urls
        });
        for u in urls {
            self.state.open_url(&u);
        }
        let _ = Color32::TRANSPARENT;
    }

    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        self.prefs.last_project = self.state.composer.project.clone().or(self.prefs.last_project.clone());
        self.prefs.save();
        if let Some(gl) = gl {
            if let Some(f) = &self.glass.frost {
                f.destroy(gl);
            }
            if let Some(r) = &self.glass.refract {
                r.destroy(gl);
            }
        }
    }
}

/// `--record DIR`: dump every read-only endpoint the UI uses + one full run SSE.
fn record(dir: &str) -> Result<(), String> {
    use serde_json::Value;
    let dir = std::path::Path::new(dir);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (c, h) = api::Client::connect().map_err(|e| e.to_string())?;
    println!("connected: protocol {} serving {:?}", h.protocol_major, h.serving_mode);
    let save = |name: &str, v: &Value| {
        let p = dir.join(name);
        let mut v = v.clone();
        scrub_value(&mut v);
        std::fs::write(&p, scrub(&serde_json::to_string_pretty(&v).unwrap()))
            .map(|_| println!("  wrote {}", p.display()))
            .map_err(|e| e.to_string())
    };
    let (_, hs): (u16, Value) = c
        .post("handshake", serde_json::json!({"protocolMajor": api::PROTOCOL_MAJOR, "client": "claudexor-linux"}))
        .map_err(|e| e.to_string())?;
    save("handshake.json", &hs)?;
    for (name, path) in [
        ("threads", "threads"),
        ("harnesses", "harnesses"),
        ("quota", "quota"),
        ("account-pools", "account-pools"),
        ("projects", "projects"),
        ("credential-profiles", "credential-profiles"),
    ] {
        match c.get::<Value>(path) {
            Ok(v) => save(&format!("{name}.json"), &v)?,
            Err(e) => println!("  {path}: {e}"),
        }
    }
    if let Ok(hl) = c.harnesses() {
        for h in hl.harnesses.iter().filter(|h| h.status != "unavailable") {
            match c.get::<Value>(&format!("harnesses/{}/models", api::seg(&h.id))) {
                Ok(v) => save(&format!("models.{}.json", h.id), &v)?,
                Err(e) => println!("  models {}: {e}", h.id),
            }
        }
    }
    let threads = c.threads().map_err(|e| e.to_string())?;
    let mut run_for_sse = None;
    for th in threads.threads.iter().take(5) {
        let v: Value = c.get(&format!("threads/{}", api::seg(&th.id))).map_err(|e| e.to_string())?;
        save(&format!("thread.{}.json", th.id), &v)?;
        let d: model::ThreadDetail = serde_json::from_value(v).map_err(|e| e.to_string())?;
        for turn in d.turns.iter().rev().take(2) {
            if let Some(rid) = &turn.run_id {
                let v: Value = c.get(&format!("runs/{}", api::seg(rid))).map_err(|e| e.to_string())?;
                save(&format!("run.{rid}.json"), &v)?;
                if run_for_sse.is_none() && turn.run.as_ref().is_some_and(|r| !matches!(r.state.as_str(), "queued" | "running")) {
                    run_for_sse = Some(rid.clone());
                }
            }
        }
    }
    if let Some(rid) = run_for_sse {
        let mut reader = c.open_stream(&format!("runs/{}/events", api::seg(&rid)), None).map_err(|e| e.to_string())?;
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut raw).map_err(|e| e.to_string())?;
        let p = dir.join(format!("run-events.{rid}.sse"));
        std::fs::write(&p, scrub(&String::from_utf8_lossy(&raw))).map_err(|e| e.to_string())?;
        println!("  wrote {} ({} bytes)", p.display(), raw.len());
    } else {
        println!("  no finished run to record SSE from (start one turn, then re-run --record)");
    }
    Ok(())
}

/// `--send PROMPT`: the GUI's write path, headless — create a no-project Ask
/// thread, send one turn, and follow its run SSE to the end (or the refusal).
fn send(prompt: &str) -> Result<(), String> {
    let (c, _) = api::Client::connect().map_err(|e| e.to_string())?;
    let title: String = prompt.chars().take(60).collect();
    let th = c
        .create_thread(&model::CreateThread {
            title: Some(title),
            scope: model::Scope::None,
            mode: Some("ask".into()),
            primary_harness: None,
            workspace: None,
            credential_profile_id: None,
            access: None,
        })
        .map_err(|e| e.to_string())?;
    println!("thread {}", th.id);
    let req = model::TurnRequest::new(prompt, "ask");
    let started = c.send_turn(&th.id, &req).map_err(|e| e.to_string())?;
    println!("turn {:?} run {:?} job {:?} state {:?}", started.turn_id, started.run_id, started.job_id, started.state);
    let Some(sid) = started.stream_id() else { return Err("no run or job id to follow".into()) };
    let mut parser = sse::Parser::default();
    let mut tr = transcript::Transcript::default();
    let end = c
        .stream(&format!("runs/{}/events", api::seg(sid)), &mut parser, |f| {
            let seq = f.id.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&f.data) {
                tr.apply(seq, &v);
            }
            println!("  #{seq} {}", f.event);
            true
        })
        .map_err(|e| e.to_string())?;
    println!("stream end: {end:?}; transcript blocks: {}", tr.blocks.len());
    let d = c.thread(&th.id).map_err(|e| e.to_string())?;
    for t in &d.turns {
        println!(
            "turn {} run {:?} state {:?} refused {:?}",
            t.id,
            t.run_id,
            t.run.as_ref().map(|r| &r.state),
            t.enqueue_error.as_ref().map(|e| &e.message)
        );
        if let Some(rid) = &t.run_id {
            if let Ok(rd) = c.run(rid) {
                println!(
                    "  outputReady {:?} failure {:?}",
                    rd.summary.output_ready_state,
                    rd.summary.failure.as_ref().and_then(state::failure_line)
                );
            }
        }
    }
    Ok(())
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--upload") {
        // headless check of the attachment pipeline: create -> PUT bytes -> finalize
        let path = args.get(i + 1).cloned().unwrap_or_default();
        let r = api::Client::connect().map_err(|e| e.to_string()).and_then(|(c, _)| {
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (kind, mime) = state::mime_for(&name);
            c.upload(&name, kind, mime, &bytes).map_err(|e| e.to_string())
        });
        match r {
            Ok(res) => println!("uploaded {} ({} bytes) -> resource {}", res.name, res.size_bytes, res.resource_id),
            Err(e) => {
                eprintln!("upload failed: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--send") {
        if let Err(e) = send(args.get(i + 1).map(String::as_str).unwrap_or("Say hello in one word.")) {
            eprintln!("send failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--record") {
        let dir = args.get(i + 1).map(String::as_str).unwrap_or("tests/fixtures/live");
        if let Err(e) = record(dir) {
            eprintln!("record failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Claudexor")
            .with_app_id("claudexor-linux")
            .with_inner_size(Prefs::load().window.map_or([1180.0, 780.0], |(w, h)| [w, h]))
            .with_min_inner_size([760.0, 480.0])
            .with_transparent(true),
        renderer: eframe::Renderer::Glow,
        // vsync off: with swap interval 1, Mesa blocks eglSwapBuffers on a Wayland frame
        // callback, which the compositor never sends while the window is hidden (other
        // workspace). Any repaint then froze the UI thread → "Application Not Responding".
        // We only paint on demand (input, SSE, ≤10 Hz pulse), so tearing-free pacing is moot.
        vsync: false,
        ..Default::default()
    };
    eframe::run_native("Claudexor", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

/// Fixtures are committed publicly: strip the recording machine's home path.
fn scrub(text: &str) -> String {
    match dirs::home_dir() {
        Some(h) => text.replace(&*h.to_string_lossy(), "/home/user"),
        None => text.to_string(),
    }
}

/// ...and the account identity: email and plan tier (setup details, not wire shape).
fn scrub_value(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if (k == "plan_label" || k == "plan") && x.is_string() {
                    *x = "plan".into();
                } else if k == "email" && x.is_string() {
                    *x = "user@example.com".into();
                } else {
                    scrub_value(x);
                }
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(scrub_value),
        _ => {}
    }
}

#[cfg(test)]
mod scrub_tests {
    #[test]
    fn scrubs_home_and_plan() {
        let home = dirs::home_dir().unwrap().to_string_lossy().into_owned();
        assert_eq!(super::scrub(&format!("{home}/.claudexor/x")), "/home/user/.claudexor/x");
        let mut v = serde_json::json!({"a": [{"plan_label": "team", "x": 1}], "plan_label": null});
        super::scrub_value(&mut v);
        assert_eq!(v, serde_json::json!({"a": [{"plan_label": "plan", "x": 1}], "plan_label": null}));
    }
}

/// The desktop's colour scheme via the XDG portal (1 dark, 2 light), falling
/// back to GNOME's gsettings; 0 when neither says.
fn desktop_scheme() -> u8 {
    let run = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd)
            .args(args)
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let portal = run(
        "gdbus",
        &[
            "call", "--session", "--dest", "org.freedesktop.portal.Desktop", "--object-path", "/org/freedesktop/portal/desktop",
            "--method", "org.freedesktop.portal.Settings.Read", "org.freedesktop.appearance", "color-scheme",
        ],
    );
    match portal.as_deref().map(|o| o.contains("uint32 1")).zip(portal.as_deref().map(|o| o.contains("uint32 2"))) {
        Some((true, _)) => return 1,
        Some((_, true)) => return 2,
        _ => {}
    }
    match run("gsettings", &["get", "org.gnome.desktop.interface", "color-scheme"]).as_deref().map(str::trim) {
        Some("'prefer-dark'") => 1,
        Some("'prefer-light'") => 2,
        // 'default' means no preference: leave it to winit, then dark
        _ => 0,
    }
}

/// Follow desktop scheme switches: a cheap poll every 20 s on a thread
/// (repaints only when it changes, so idle stays idle).
fn watch_desktop_scheme(cell: std::sync::Arc<std::sync::atomic::AtomicU8>, ctx: egui::Context) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(20));
        let now = desktop_scheme();
        if cell.swap(now, std::sync::atomic::Ordering::Relaxed) != now {
            ctx.request_repaint();
        }
    });
}
