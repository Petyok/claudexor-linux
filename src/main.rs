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
use egui::{Align2, Color32, FontId, Id, Rect, RichText, Sense, Stroke, UiBuilder, pos2, vec2};
use state::{Conn, State};
use ui::glass::{Glass, Kind};
use ui::theme::{self, R_SM, SIDEBAR_W, SP, T_SMALL, Theme};
use ui::{View, last_rect, store_rect};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ThemePref {
    System,
    Light,
    Dark,
}

struct Prefs {
    theme: ThemePref,
    reduce_transparency: bool,
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
        let _ = std::fs::write(p, format!("theme={theme}\nreduce_transparency={}\n", self.reduce_transparency as u8));
    }
}

struct App {
    state: State,
    glass: Glass,
    md: egui_commonmark::CommonMarkCache,
    drafts: ui::thread::Drafts,
    prefs: Prefs,
    applied_dark: Option<bool>,
    accounts_open: bool,
    accounts_opened_at: u64,
    accounts_anchor: egui::Pos2,
    stats: Option<ui::stats::FrameStats>,
    last_frame: std::time::Instant,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
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
        let state = State::new(cc.egui_ctx.clone());
        App {
            state,
            glass: Glass { frost, refract, reduce_transparency: prefs.reduce_transparency, theme: theme::DARK },
            md: Default::default(),
            drafts: Default::default(),
            prefs,
            applied_dark: None,
            accounts_open: false,
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
            ThemePref::System => ctx.system_theme().is_none_or(|t| t == egui::Theme::Dark),
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
        self.state.pump();
        // Desktop notifications only when the user is elsewhere (window unfocused).
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        for n in self.state.notices.drain(..) {
            if !focused {
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("notify-send")
                        .args(["--app-name=Claudexor", "--icon=claudexor-linux", &n.title, &n.body])
                        .status();
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
        let side = Rect::from_min_size(full.min + vec2(g, g), vec2(SIDEBAR_W, full.height() - 2.0 * g));
        let main = Rect::from_min_max(pos2(side.max.x + g, full.min.y), full.max);

        let mut view = View { glass: &self.glass, md: &mut self.md, t };

        // Conversation first: the composer's glass then frosts/refracts it.
        let composer_h = last_rect(ui, Id::new("composer")).map_or(118.0, |r| r.height());
        ui::thread::show(ui, &mut view, &mut self.state, &mut self.drafts, main, 64.0, composer_h + 8.0 * SP);
        ui::composer::show(ui, &mut view, &mut self.state, main);
        let side_out = ui::sidebar::show(ui, &mut view, &mut self.state, side);
        if side_out.toggle_accounts {
            self.accounts_open = !self.accounts_open;
            self.accounts_opened_at = ctx.cumulative_frame_nr();
            self.accounts_anchor = side_out.accounts_anchor.left_top() + vec2(0.0, -SP);
            if self.accounts_open {
                self.state.refresh_quota(false);
            }
        }

        // status pill (top-right chrome) + settings menu
        let pill_id = Id::new("pill");
        let pill_w = last_rect(ui, pill_id).map_or(220.0, |r| r.width());
        let pill = Rect::from_min_size(pos2(main.max.x - pill_w - 4.0 * SP, main.min.y + g), vec2(pill_w, 34.0));
        self.glass.surface(ui, pill, 17, Kind::Chrome);
        let (dot, word) = match &self.state.conn {
            Conn::Online { engine } => (t.success, format!("Engine {engine}")),
            Conn::Connecting => (t.queued, "Connecting…".into()),
            Conn::Recovering => (t.blocked, "Recovering…".into()),
            Conn::Offline(_) => (t.failed, "Offline".into()),
            Conn::Incompatible(_) => (t.failed, "Incompatible".into()),
        };
        let pool = self
            .state
            .pools
            .iter()
            .find(|p| Some(&p.harness_id) == self.state.composer.harness.as_ref())
            .or_else(|| self.state.pools.iter().find(|p| p.next_up.kind != "none"))
            .or_else(|| self.state.pools.first());
        let label = match pool {
            Some(p) if matches!(self.state.conn, Conn::Online { .. }) => format!("{word}  ·  {} {}", p.harness_id, p.next_up.describe()),
            _ => word,
        };
        let max_w = (main.width() * 0.5).max(160.0);
        let p = ui.painter();
        let mut job = egui::text::LayoutJob::simple_singleline(label, FontId::proportional(T_SMALL), t.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_w - 58.0);
        let galley = p.layout_job(job);
        let want_w = galley.size().x + 58.0;
        p.circle_filled(pos2(pill.min.x + 16.0, pill.center().y), 4.0, dot);
        p.galley(pos2(pill.min.x + 28.0, pill.center().y - galley.size().y / 2.0), galley, t.text);
        p.text(pos2(pill.max.x - 14.0, pill.center().y), Align2::RIGHT_CENTER, "⚙", FontId::proportional(T_SMALL + 1.0), t.text2);
        store_rect(ui, pill_id, Rect::from_min_size(pill.min, vec2(want_w, 34.0)));
        let pill_resp = ui.interact(pill, pill_id.with("click"), Sense::click());
        let tip = match &self.state.conn {
            Conn::Offline(why) | Conn::Incompatible(why) => why.clone(),
            _ => match pool.and_then(|p| p.next_up.reason.clone()) {
                Some(r) => format!("Engine status · settings\n\nnext up: {r}"),
                None => "Engine status · settings".into(),
            },
        };
        let pill_resp = pill_resp.on_hover_text(tip);
        egui::Popup::menu(&pill_resp).show(|ui| {
            ui.label(RichText::new("Appearance").size(T_SMALL).color(t.text3));
            let mut changed = false;
            for (p, name) in [(ThemePref::System, "System"), (ThemePref::Light, "Light"), (ThemePref::Dark, "Dark")] {
                changed |= ui.radio_value(&mut self.prefs.theme, p, name).changed();
            }
            ui.separator();
            changed |= ui.checkbox(&mut self.prefs.reduce_transparency, "Reduce transparency").changed();
            if changed {
                self.prefs.save();
                self.applied_dark = None;
            }
            ui.separator();
            if ui.button("Accounts & quota").clicked() {
                self.accounts_open = true;
                self.accounts_opened_at = ctx.cumulative_frame_nr();
                self.accounts_anchor = side_out.accounts_anchor.left_top() + vec2(0.0, -SP);
                self.state.refresh_quota(false);
            }
            ui.label(RichText::new(format!("Claudexor for Linux {} · MIT", env!("CARGO_PKG_VERSION"))).size(T_SMALL).color(t.text3));
            if let Some(v) = &self.state.engine_version {
                ui.label(RichText::new(format!("engine {v} · protocol {}", api::PROTOCOL_MAJOR)).size(T_SMALL).color(t.text3));
            }
            ui.label(RichText::new("Ctrl+N new · Ctrl+K search · Alt+↑/↓ threads").size(T_SMALL).color(t.text3));
            if !self.glass.blur_available() {
                ui.label(RichText::new("glass: solid fallback").size(T_SMALL).color(t.text3));
            }
        });

        // error banner (dismissable, selectable)
        if let Some(err) = self.state.error.clone() {
            let w = (main.width() - 16.0 * SP).min(640.0);
            let r = Rect::from_min_size(pos2(main.center().x - w / 2.0, main.min.y + g + 42.0), vec2(w, 40.0));
            ui.painter().rect_filled(
                r,
                R_SM,
                t.failed.gamma_multiply(if t.dark { 0.28 } else { 0.14 }).to_opaque().lerp_to_gamma(t.overlay, 0.35),
            );
            ui.painter().rect_stroke(r, R_SM, Stroke::new(1.0_f32, t.failed.gamma_multiply(0.6)), egui::StrokeKind::Inside);
            ui.scope_builder(UiBuilder::new().max_rect(r.shrink2(vec2(12.0, 6.0))), |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add(egui::Label::new(RichText::new(&err).color(t.text).size(T_SMALL)).truncate().selectable(true))
                        .on_hover_text(&err);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("×").frame(false)).on_hover_text("Dismiss").clicked() {
                            self.state.error = None;
                        }
                        if ui.add(egui::Button::new("⎘").frame(false)).on_hover_text("Copy message").clicked() {
                            ui.ctx().copy_text(err.clone());
                        }
                    });
                });
            });
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
            .with_inner_size([1180.0, 780.0])
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
