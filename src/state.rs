//! App state + the message reducer that folds worker results into it.
//!
//! Threading model (PLAN §3): every daemon call runs on a plain std thread and
//! reports back through one mpsc channel, then pokes `ctx.request_repaint()`.
//! The UI thread only drains messages and draws — it never blocks. With no
//! messages and no animation egui paints nothing, so an idle window costs 0%.

use crate::api::{ApiError, Client, ConnectError};
use crate::model::*;
use crate::sse;
use crate::transcript::Transcript;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

pub const MODES: [&str; 3] = ["ask", "plan", "agent"];
const ANSWER_PATHS: [&str; 4] = ["final/answer.md", "final/report.md", "final/plan.md", "final/explore.md"];

#[derive(Debug, Clone, PartialEq)]
pub enum Conn {
    Connecting,
    Online {
        engine: String,
    },
    /// Daemon up but serving only the journal-recovery plane (typed 503s).
    Recovering,
    Offline(String),
    Incompatible(String),
}

#[derive(Debug, Clone)]
pub struct PendingSend {
    pub req: TurnRequest,
}

#[derive(Debug)]
pub enum StreamEnd {
    /// Server sent `end`: the run is terminal and fully replayed.
    Terminal,
    /// Stream dropped (daemon restart / network); resume on reconnect.
    Lost,
    /// Stream refused (unknown run, bad cursor).
    Refused(String),
}

pub enum Msg {
    Online(Client, Handshake),
    Recovering,
    Offline(String),
    Incompatible(String),
    ThreadsStale,
    Threads(Result<ThreadList, ApiError>),
    Thread(String, Result<ThreadDetail, ApiError>),
    Created(Result<Thread, ApiError>, PendingSend),
    TurnSent(String, Result<TurnStarted, ApiError>, Option<String>),
    Event(String, i64, Value),
    StreamEnd(String, StreamEnd),
    Run(String, Result<RunDetail, ApiError>),
    Answer(String, Result<Option<String>, ApiError>),
    Harnesses(Result<HarnessList, ApiError>),
    Models(String, Result<ModelList, ApiError>),
    Quota(Result<Quota, ApiError>),
    Pools(Result<AccountPools, ApiError>),
    Projects(Result<ProjectList, ApiError>),
    Acked(&'static str, Result<(), ApiError>),
    Rewatch(String),
    ThreadChanged(&'static str, Result<Thread, ApiError>),
    Profiles(Result<Profiles, ApiError>),
    ProfileChanged(&'static str, Result<(), ApiError>, Option<(String, String)>),
    Picked(Result<Vec<std::path::PathBuf>, String>),
    Uploaded(u64, Result<Resource, String>),
    EngineStarting,
    EngineStart(Result<(), String>),
    LoginJob(Result<SetupJob, ApiError>),
    LoginSnap(Result<SetupSnapshot, ApiError>),
    LoginInput(Result<SetupJob, ApiError>),
    Trust(String, Result<Option<TrustState>, ApiError>),
}

/// Live state of one run stream (keyed by the id the stream was opened with:
/// the run id, or the job id while a turn is still queued).
pub struct Live {
    pub transcript: Transcript,
    /// Lifecycle word from events: queued → running → terminal type.
    pub phase: String,
    /// `run.completed` | `run.failed` | `run.blocked` once seen.
    pub terminal: Option<String>,
    pub waiting: bool,
    pub started: Instant,
    pub finished: Option<Instant>,
    stop: Option<Arc<AtomicBool>>,
    /// Stream was lost (engine restart): restart it on reconnect.
    lost: bool,
}

impl Live {
    fn new() -> Self {
        Live {
            transcript: Transcript::default(),
            phase: "queued".into(),
            terminal: None,
            waiting: false,
            started: Instant::now(),
            finished: None,
            stop: None,
            lost: false,
        }
    }
    pub fn streaming(&self) -> bool {
        self.stop.is_some()
    }
}

#[derive(Debug, Clone)]
pub enum AnswerState {
    Loading,
    Ready(String),
    /// Terminal with no answer-like output (patch-only, diagnostic, no-change).
    None,
    Failed(String),
}

#[derive(Default)]
pub struct Composer {
    pub text: String,
    pub mode: usize, // index into MODES
    pub harness: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Project root for a draft (new) thread; bound thread roots win.
    pub project: Option<String>,
    pub sending: bool,
    /// Pin the turn to one account (credential profile); None = automatic routing.
    pub account: Option<String>,
    pub attachments: Vec<Attachment>,
    /// In-flight native file picker / screen capture.
    pub picking: bool,
    /// Draft thread only: keep turns in a persistent thread worktree.
    pub isolated: bool,
    /// The "Options" knobs (strategy, access, review, budget, …).
    pub opts: TurnOpts,
}

pub struct Attachment {
    pub local: u64,
    pub name: String,
    pub size: u64,
    pub state: AttachState,
}

pub enum AttachState {
    Uploading,
    Ready(String),
    Failed(String),
}

/// Largest file we read into memory for one attachment.
const MAX_ATTACHMENT: u64 = 25 * 1024 * 1024;

/// A desktop notification the app wants to show (main decides, by window focus).
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// Single-flight + one trailing request, so ping storms cost one fetch.
#[derive(Default)]
struct Flight {
    busy: bool,
    again: bool,
}

impl Flight {
    fn start(&mut self) -> bool {
        if self.busy {
            self.again = true;
            return false;
        }
        self.busy = true;
        true
    }
    fn finish(&mut self) -> bool {
        self.busy = false;
        std::mem::take(&mut self.again)
    }
}

pub struct State {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    pub client: Option<Client>,
    pub conn: Conn,
    stale: Arc<AtomicBool>,

    pub threads: Vec<Thread>,
    pub threads_loaded: bool,
    pub selected: Option<String>,
    pub detail: Option<ThreadDetail>,
    pub detail_loading: bool,
    threads_flight: Flight,
    detail_flight: Flight,
    last_poll: Option<Instant>,

    pub runs: HashMap<String, Live>,
    /// turn id → stream id, for turns whose run id is not yet on the detail.
    pub turn_stream: HashMap<String, String>,
    pub details: HashMap<String, RunDetail>,
    /// Last live-driven detail refresh per run (throttle).
    detail_at: HashMap<String, Instant>,
    pub answers: HashMap<String, AnswerState>,
    run_flight: HashSet<String>,
    /// Turns whose activity the user expanded (terminal turns collapse by default).
    pub expanded: HashSet<String>,
    pub collapsed: HashSet<String>,

    pub harnesses: Vec<Harness>,
    pub models: HashMap<String, Result<ModelList, String>>,
    /// Repo trust by root: Ok(None) = no trust file (engine defaults).
    pub trust: HashMap<String, Result<Option<TrustState>, String>>,
    pub quota: Option<Quota>,
    pub quota_error: Option<String>,
    pub quota_loading: bool,
    pub pools: Vec<AccountPool>,
    pub projects: Vec<Project>,

    pub composer: Composer,
    pub error: Option<String>,
    pub engine_version: Option<String>,
    /// The one in-app native login in flight (the daemon allows one per target).
    pub login: Option<Login>,
    pub profiles: Option<Profiles>,
    /// Sidebar shows the trash instead of live threads.
    pub show_trash: bool,
    /// Turn-finished / needs-you events; main shows them when the window is unfocused.
    pub notices: Vec<Notice>,
    /// `claudexor daemon start` in flight (auto on first launch, or the offline button).
    pub engine_starting: bool,
    pub engine_start_error: Option<String>,
    next_local: u64,
}

/// In-app native login: the daemon runs the vendor CLI and publishes a
/// transient sign-in URL (+ code) on the job snapshot; for `oauth_url_input`
/// the user pastes the vendor's code back, which goes to `/input` only.
pub struct Login {
    pub harness: String,
    pub job: Option<SetupJob>,
    pub disclosure: Option<DeviceCode>,
    pub error: Option<String>,
    /// The pasted one-time code. Held only until sent; never logged.
    pub code: String,
    pub code_sent: bool,
    polling: bool,
    last_poll: Option<Instant>,
    /// One automatic Extend per job: a rejoined job (e.g. started earlier by
    /// `claudexor auth login`) can have only minutes left on its deadline.
    auto_extended: bool,
}

/// Seconds until an ISO deadline (negative when past); None if unparsable.
pub fn secs_left(deadline: Option<&str>) -> Option<i64> {
    deadline.and_then(crate::ui::theme::parse_iso).map(|t| t - crate::ui::theme::now_unix())
}

impl Login {
    pub fn active(&self) -> bool {
        self.job.as_ref().is_none_or(|j| !j.terminal())
    }
}

impl State {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        let stale = Arc::new(AtomicBool::new(false));
        let s = State {
            tx,
            rx,
            ctx,
            client: None,
            conn: Conn::Connecting,
            stale,
            threads: vec![],
            threads_loaded: false,
            selected: None,
            detail: None,
            detail_loading: false,
            threads_flight: Flight::default(),
            detail_flight: Flight::default(),
            last_poll: None,
            runs: HashMap::new(),
            turn_stream: HashMap::new(),
            details: HashMap::new(),
            answers: HashMap::new(),
            run_flight: HashSet::new(),
            expanded: HashSet::new(),
            collapsed: HashSet::new(),
            harnesses: vec![],
            models: HashMap::new(),
            trust: HashMap::new(),
            detail_at: HashMap::new(),
            quota: None,
            quota_error: None,
            quota_loading: false,
            pools: vec![],
            projects: vec![],
            composer: Composer { mode: 2, ..Default::default() },
            error: None,
            engine_version: None,
            login: None,
            profiles: None,
            show_trash: false,
            notices: vec![],
            engine_starting: false,
            engine_start_error: None,
            next_local: 1,
        };
        s.spawn_supervisor();
        s
    }

    // ---- plumbing ------------------------------------------------------------

    fn send(tx: &Sender<Msg>, ctx: &egui::Context, m: Msg) {
        let _ = tx.send(m);
        ctx.request_repaint();
    }

    /// Run `f` against the current client on a worker thread.
    fn spawn(&self, f: impl FnOnce(&Client) -> Msg + Send + 'static) {
        let Some(client) = self.client.clone() else { return };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || Self::send(&tx, &ctx, f(&client)));
    }

    /// Connection supervisor: discover → handshake → hold the global journal
    /// stream open. The stream doubles as the liveness probe: when the daemon
    /// dies its socket closes, we report Offline and retry with backoff.
    fn spawn_supervisor(&self) {
        let (tx, ctx, stale) = (self.tx.clone(), self.ctx.clone(), self.stale.clone());
        std::thread::spawn(move || {
            let mut backoff = Duration::from_millis(500);
            let mut cursor: Option<String> = None;
            let mut auto_started = false;
            loop {
                match Client::connect() {
                    Ok((client, h)) if h.serving_mode.as_deref() == Some("recovery_only") => {
                        let _ = client;
                        Self::send(&tx, &ctx, Msg::Recovering);
                        std::thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                    Ok((client, h)) => {
                        backoff = Duration::from_millis(500);
                        Self::send(&tx, &ctx, Msg::Online(client.clone(), h));
                        let mut parser = sse::Parser::resume(cursor.take());
                        let r = client.stream("global/events", &mut parser, |f| {
                            if f.event == "thread.head.updated" && !stale.swap(true, Ordering::AcqRel) {
                                Self::send(&tx, &ctx, Msg::ThreadsStale);
                            }
                            true
                        });
                        cursor = parser.last_id().map(str::to_owned);
                        match r {
                            // Stale/foreign cursor: drop it and resnapshot from scratch.
                            Err(ApiError::Http { status, .. }) if (400..500).contains(&status) => cursor = None,
                            // No journal stream on this engine: fall back to a slow liveness loop.
                            Err(ApiError::Http { .. }) => std::thread::sleep(Duration::from_secs(5)),
                            _ => {}
                        }
                        // Loop: reconnect; a dead daemon fails `connect` below.
                    }
                    Err(ConnectError::NotRunning) if !auto_started && std::env::var_os("CXL_NO_AUTOSTART").is_none() => {
                        // Like the macOS app launching its bundled daemon: start the
                        // engine once per app launch, then retry at once.
                        auto_started = true;
                        Self::send(&tx, &ctx, Msg::EngineStarting);
                        let r = start_engine_blocking();
                        Self::send(&tx, &ctx, Msg::EngineStart(r.map(|_| ())));
                    }
                    Err(e) => {
                        let m = match &e {
                            ConnectError::Incompatible(_) => Msg::Incompatible(e.to_string()),
                            _ => Msg::Offline(e.to_string()),
                        };
                        Self::send(&tx, &ctx, m);
                        std::thread::sleep(backoff);
                        backoff = (backoff * 2).min(Duration::from_secs(3));
                    }
                }
            }
        });
    }

    // ---- refreshes ---------------------------------------------------------------

    pub fn refresh_threads(&mut self) {
        if self.client.is_none() || !self.threads_flight.start() {
            return;
        }
        self.stale.store(false, Ordering::Release);
        self.spawn(|c| Msg::Threads(c.threads()));
    }

    pub fn refresh_detail(&mut self) {
        let Some(id) = self.selected.clone() else { return };
        if self.client.is_none() || !self.detail_flight.start() {
            return;
        }
        self.detail_loading = true;
        self.spawn(move |c| Msg::Thread(id.clone(), c.thread(&id)));
    }

    pub fn refresh_quota(&mut self, live: bool) {
        if self.client.is_none() || self.quota_loading {
            return;
        }
        self.quota_loading = true;
        self.spawn(move |c| Msg::Quota(if live { c.refresh_quota() } else { c.quota() }));
        self.spawn(|c| Msg::Pools(c.account_pools()));
        self.spawn(|c| Msg::Profiles(c.profiles()));
    }

    pub fn load_models(&mut self, harness: &str) {
        if self.models.contains_key(harness) {
            return;
        }
        self.models.insert(harness.to_string(), Err("loading…".into()));
        let h = harness.to_string();
        self.spawn(move |c| Msg::Models(h.clone(), c.models(&h)));
    }

    pub fn load_trust(&mut self, root: &str) {
        if self.trust.contains_key(root) || self.client.is_none() {
            return;
        }
        self.trust.insert(root.to_string(), Err("loading…".into()));
        let r = root.to_string();
        self.spawn(move |c| Msg::Trust(r.clone(), c.trust(&r)));
    }

    pub fn grant_full_access(&mut self, root: &str) {
        let r = root.to_string();
        self.spawn(move |c| Msg::Trust(r.clone(), c.grant_full_access(&r).map(Some)));
    }

    /// The repo's trust state, once loaded.
    pub fn repo_trust(&self) -> Option<&TrustState> {
        self.trust.get(&self.effective_root()?)?.as_ref().ok()?.as_ref()
    }

    /// Access an Agent turn runs with: the explicit pick, else the repo default.
    pub fn effective_access(&self) -> &'static str {
        if let Some(a) = self.composer.opts.access {
            return a;
        }
        match self.repo_trust().map(|t| t.access_default.as_str()) {
            Some("readonly") => "readonly",
            Some("full") => "full",
            _ => "workspace_write",
        }
    }

    /// Why the Options block Send (bad panel/budget, ungranted full access).
    pub fn options_error(&self) -> Option<String> {
        let mode = if self.effective_root().is_none() { "ask" } else { self.current_mode() };
        let mut probe = TurnRequest::new("", mode);
        if let Err(e) = self.composer.opts.apply(&mut probe, self.effective_access()) {
            return Some(e);
        }
        let ungranted = mode == "agent" && self.composer.opts.access == Some("full") && !self.repo_trust().is_some_and(|t| t.allow_full_access);
        ungranted.then(|| "Full access needs a grant for this repo (Options)".into())
    }

    pub fn select(&mut self, id: Option<String>) {
        if self.selected == id {
            return;
        }
        self.selected = id;
        self.detail = None;
        self.detail_flight = Flight::default();
        self.refresh_detail();
    }

    // ---- run streams ---------------------------------------------------------------

    /// Open (or resume) the SSE stream for `stream_id` unless one is running.
    pub fn watch(&mut self, stream_id: &str) {
        let Some(client) = self.client.clone() else { return };
        let live = self.runs.entry(stream_id.to_string()).or_insert_with(Live::new);
        if live.stop.is_some() {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        live.stop = Some(stop.clone());
        live.lost = false;
        let resume = (live.transcript.last_seq > 0).then(|| live.transcript.last_seq.to_string());
        let (tx, ctx, id) = (self.tx.clone(), self.ctx.clone(), stream_id.to_string());
        std::thread::spawn(move || {
            let mut parser = sse::Parser::resume(resume);
            let path = format!("runs/{}/events", crate::api::seg(&id));
            let r = client.stream(&path, &mut parser, |f| {
                if let (Some(seq), Ok(v)) = (f.id.as_deref().and_then(|s| s.parse::<i64>().ok()), serde_json::from_str::<Value>(&f.data)) {
                    Self::send(&tx, &ctx, Msg::Event(id.clone(), seq, v));
                }
                !stop.load(Ordering::Acquire)
            });
            let end = match r {
                Ok(sse::End::Terminal) => StreamEnd::Terminal,
                Ok(sse::End::Stopped) => return,
                Ok(sse::End::Lost) => StreamEnd::Lost,
                Err(e @ ApiError::Http { .. }) => StreamEnd::Refused(e.to_string()),
                Err(_) => StreamEnd::Lost,
            };
            Self::send(&tx, &ctx, Msg::StreamEnd(id, end));
        });
    }

    /// Stream id a turn's live box lives under.
    pub fn stream_id_of(&self, turn: &Turn) -> Option<String> {
        turn.run_id.clone().or_else(|| self.turn_stream.get(&turn.id).cloned())
    }

    pub fn live_of(&self, turn: &Turn) -> Option<&Live> {
        let by_run = turn.run_id.as_ref().and_then(|r| self.runs.get(r));
        by_run.or_else(|| self.turn_stream.get(&turn.id).and_then(|j| self.runs.get(j)))
    }

    /// Fetch run detail (+ answer text when terminal), single-flight per run.
    pub fn load_run(&mut self, run_id: &str, with_answer: bool) {
        if !self.run_flight.insert(run_id.to_string()) {
            return;
        }
        if with_answer {
            self.answers.entry(run_id.to_string()).or_insert(AnswerState::Loading);
        }
        let id = run_id.to_string();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.spawn(move |c| {
            let detail = c.run(&id);
            if with_answer {
                let ans = match &detail {
                    Ok(d) => answer_text(c, &id, d),
                    Err(e) => Err(e.clone()),
                };
                Self::send(&tx, &ctx, Msg::Answer(id.clone(), ans));
            }
            Msg::Run(id, detail)
        });
    }

    // ---- actions ---------------------------------------------------------------------

    pub fn current_mode(&self) -> &'static str {
        MODES[self.composer.mode.min(2)]
    }

    /// Project root the next turn runs in (bound thread root, else the draft's).
    pub fn effective_root(&self) -> Option<String> {
        match &self.detail {
            Some(d) if self.selected.is_some() => d.thread.repo_root.clone(),
            _ => self.composer.project.clone(),
        }
    }

    pub fn can_send(&self) -> bool {
        // An open thread must be loaded first: its bound root decides the mode.
        let thread_ready = self.selected.is_none() || self.detail.is_some();
        self.client.is_some()
            && thread_ready
            && !self.composer.sending
            && !self.composer.text.trim().is_empty()
            && self.head_live_id().is_none()
            // an attachment the model never saw must never look delivered
            && !self.composer.attachments.iter().any(|a| matches!(a.state, AttachState::Uploading))
            && self.options_error().is_none()
    }

    pub fn submit(&mut self) {
        if !self.can_send() {
            return;
        }
        // A no-project thread is Ask-only (DESIGN_SYSTEM §5 composer rule).
        let mode = if self.effective_root().is_none() { "ask" } else { self.current_mode() };
        let mut req = TurnRequest::new(self.composer.text.trim(), mode);
        req.primary_harness = self.composer.harness.clone();
        req.model = self.composer.harness.as_ref().and(self.composer.model.clone());
        req.effort = self.composer.harness.as_ref().and(self.composer.effort.clone());
        req.credential_profile_id = self.composer.harness.as_ref().and(self.composer.account.clone());
        req.attachments = self
            .composer
            .attachments
            .iter()
            .filter_map(|a| match &a.state {
                AttachState::Ready(id) => Some(ResourceRef { resource_id: id.clone() }),
                _ => None,
            })
            .collect();
        if let Err(e) = self.composer.opts.apply(&mut req, self.effective_access()) {
            self.error = Some(e);
            return;
        }
        self.send_request(req);
    }

    /// Send a turn on the selected thread, or create the thread first (draft).
    fn send_request(&mut self, req: TurnRequest) {
        let mode = req.mode.clone();
        self.composer.sending = true;
        self.error = None;
        match self.selected.clone() {
            Some(tid) => self.spawn(move |c| Msg::TurnSent(tid.clone(), c.send_turn(&tid, &req), Some(req.prompt))),
            None => {
                let title: String = req.prompt.lines().next().unwrap_or("").chars().take(80).collect();
                let create = CreateThread {
                    title: Some(title),
                    scope: match &self.composer.project {
                        Some(root) => Scope::Project { root: root.clone(), context: "auto" },
                        None => Scope::None,
                    },
                    mode: Some(mode.into()),
                    primary_harness: self.composer.harness.clone(),
                    workspace: self.composer.isolated.then(|| "isolated".to_string()),
                };
                self.spawn(move |c| Msg::Created(c.create_thread(&create), PendingSend { req }));
            }
        }
    }

    /// The selected thread's live head (queued/running), as a stream id.
    pub fn head_live_id(&self) -> Option<String> {
        let turn = self.detail.as_ref()?.turns.last()?;
        let live = self.live_of(turn);
        let active = match (&turn.run, live) {
            (_, Some(l)) if l.terminal.is_some() => false,
            (Some(r), _) => matches!(r.state.as_str(), "queued" | "running"),
            (None, Some(_)) => true,
            (None, None) => false,
        };
        active.then(|| self.stream_id_of(turn)).flatten()
    }

    pub fn cancel_head(&mut self) {
        if let Some(id) = self.head_live_id() {
            self.spawn(move |c| Msg::Acked("Stop", c.cancel(&id)));
        }
    }

    pub fn retry(&mut self, turn_id: &str) {
        let Some(tid) = self.selected.clone() else { return };
        let turn_id = turn_id.to_string();
        self.composer.sending = true;
        self.spawn(move |c| Msg::TurnSent(tid.clone(), c.retry_turn(&tid, &turn_id), None));
    }

    pub fn answer(&mut self, run_id: &str, interaction_id: &str, answers: Vec<Answer>) {
        let (r, i) = (run_id.to_string(), interaction_id.to_string());
        self.spawn(move |c| Msg::Acked("Answer", c.answer(&r, &i, &answers)));
    }

    pub fn new_thread(&mut self) {
        self.select(None);
        self.composer.text.clear();
    }

    // ---- thread management -----------------------------------------------------------

    pub fn rename_thread(&mut self, id: &str, title: &str) {
        let (id, title) = (id.to_string(), title.trim().to_string());
        if title.is_empty() {
            return;
        }
        self.spawn(move |c| Msg::ThreadChanged("Rename", c.update_thread(&id, serde_json::json!({ "title": title }))));
    }

    /// Archive (`closed`) or reopen (`active`).
    pub fn set_thread_state(&mut self, id: &str, state: &str) {
        let (id, state) = (id.to_string(), state.to_string());
        self.spawn(move |c| Msg::ThreadChanged("Archive", c.update_thread(&id, serde_json::json!({ "state": state }))));
    }

    /// `trash` | `restore` | `purge` (purge is permanent; UI confirms first).
    pub fn thread_action(&mut self, id: &str, action: &'static str) {
        let id = id.to_string();
        if matches!(action, "trash" | "purge") && self.selected.as_deref() == Some(id.as_str()) {
            self.select(None);
        }
        self.spawn(move |c| Msg::ThreadChanged(action, c.thread_action(&id, action)));
    }

    // ---- plan lifecycle -----------------------------------------------------------------

    /// Answers go back as an ordinary follow-up PLAN turn, tied to the plan by
    /// `answersPlanRunId` (the same conversation continues toward `ready`).
    pub fn answer_plan(&mut self, plan_run_id: &str, prompt: String) {
        let mut req = TurnRequest::new(prompt, "plan");
        req.answers_plan_run_id = Some(plan_run_id.to_string());
        req.primary_harness = self.composer.harness.clone();
        self.send_request(req);
    }

    /// Implement freezes the plan server-side (hash on the turn). `force` is the
    /// explicit, recorded override for a plan that still has open questions.
    pub fn implement_plan(&mut self, plan_run_id: &str, force: bool) {
        let mut req = TurnRequest::new("Implement this plan.", "agent");
        req.plan_run_id = Some(plan_run_id.to_string());
        req.override_plan_readiness = force.then_some(true);
        req.primary_harness = self.composer.harness.clone();
        self.send_request(req);
    }

    // ---- accounts --------------------------------------------------------------------

    pub fn refresh_profiles(&mut self) {
        self.spawn(|c| Msg::Profiles(c.profiles()));
    }

    pub fn set_profile_enabled(&mut self, harness: &str, profile: &str, enabled: bool) {
        let (h, p) = (harness.to_string(), profile.to_string());
        self.spawn(move |c| Msg::ProfileChanged("Update account", c.set_profile_enabled(&h, &p, enabled).map(drop), None));
    }

    pub fn delete_profile(&mut self, harness: &str, profile: &str) {
        let (h, p) = (harness.to_string(), profile.to_string());
        if self.composer.account.as_deref() == Some(profile) {
            self.composer.account = None;
        }
        self.spawn(move |c| Msg::ProfileChanged("Remove account", c.delete_profile(&h, &p).map(drop), None));
    }

    /// Add an account row, then start its login in the same action.
    pub fn add_account(&mut self, harness: &str, name: &str) {
        let slug: String = name
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let profile = if slug.is_empty() { format!("{harness}-{}", self.next_local) } else { format!("{harness}-{slug}") };
        self.next_local += 1;
        let (h, p, display) = (harness.to_string(), profile, name.trim().to_string());
        self.spawn(move |c| {
            let r = c.create_profile(&h, &p, Some(display.as_str()).filter(|d| !d.is_empty())).map(drop);
            Msg::ProfileChanged("Add account", r, Some((h, p)))
        });
    }

    // ---- attachments -----------------------------------------------------------------

    /// Native file chooser (zenity/kdialog), off the UI thread.
    pub fn pick_files(&mut self) {
        if self.composer.picking {
            return;
        }
        self.composer.picking = true;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || Self::send(&tx, &ctx, Msg::Picked(run_file_picker())));
    }

    /// Region screenshot with grim + slurp (Wayland), off the UI thread.
    pub fn capture_region(&mut self) {
        if self.composer.picking {
            return;
        }
        self.composer.picking = true;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || Self::send(&tx, &ctx, Msg::Picked(run_capture())));
    }

    pub fn attach_paths(&mut self, paths: Vec<std::path::PathBuf>) {
        for path in paths {
            let local = self.next_local;
            self.next_local += 1;
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let state = if size > MAX_ATTACHMENT {
                AttachState::Failed(format!("larger than {} MB", MAX_ATTACHMENT / 1024 / 1024))
            } else {
                AttachState::Uploading
            };
            let uploading = matches!(state, AttachState::Uploading);
            self.composer.attachments.push(Attachment { local, name: name.clone(), size, state });
            if uploading {
                self.spawn(move |c| {
                    let r = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| {
                        let (kind, mime) = mime_for(&name);
                        c.upload(&name, kind, mime, &bytes).map_err(|e| e.to_string())
                    });
                    Msg::Uploaded(local, r)
                });
            }
        }
    }

    pub fn remove_attachment(&mut self, local: u64) {
        self.composer.attachments.retain(|a| a.local != local);
    }

    // ---- engine ----------------------------------------------------------------------

    /// The offline screen's "Start engine" (the supervisor reconnects on its own).
    pub fn start_engine(&mut self) {
        if self.engine_starting {
            return;
        }
        self.engine_starting = true;
        self.engine_start_error = None;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || Self::send(&tx, &ctx, Msg::EngineStart(start_engine_blocking().map(drop))));
    }

    // ---- native login -----------------------------------------------------------------

    pub fn start_login(&mut self, harness: &str) {
        self.start_login_for(harness, None);
    }

    /// Login for one exact account (profile-targeted setup job), or the bootstrap one.
    pub fn start_login_for(&mut self, harness: &str, profile: Option<String>) {
        self.login = Some(Login {
            harness: harness.to_string(),
            job: None,
            disclosure: None,
            error: None,
            code: String::new(),
            code_sent: false,
            polling: false,
            last_poll: None,
            auto_extended: false,
        });
        let h = harness.to_string();
        self.spawn(move |c| Msg::LoginJob(c.start_login(&h, profile.as_deref())));
    }

    pub fn submit_login_code(&mut self) {
        let Some(l) = self.login.as_mut() else { return };
        let (Some(job), code) = (l.job.as_ref(), l.code.trim().to_string()) else { return };
        if code.is_empty() {
            return;
        }
        let id = job.job_id.clone();
        l.code.clear(); // do not keep the secret around after handing it over
        l.code_sent = true;
        l.error = None;
        self.spawn(move |c| Msg::LoginInput(c.login_input(&id, &code)));
    }

    pub fn cancel_login(&mut self) {
        let Some(id) = self.login.as_ref().and_then(|l| l.job.as_ref()).filter(|j| !j.terminal()).map(|j| j.job_id.clone()) else {
            self.login = None;
            return;
        };
        self.spawn(move |c| Msg::LoginJob(c.cancel_login(&id)));
    }

    pub fn extend_login(&mut self) {
        let Some(id) =
            self.login.as_ref().and_then(|l| l.job.as_ref()).filter(|j| !j.terminal() && !j.deadline_fixed).map(|j| j.job_id.clone())
        else {
            return;
        };
        self.spawn(move |c| Msg::LoginJob(c.extend_login(&id)));
    }

    /// Open a URL without ever blocking the UI thread on a browser launch.
    pub fn open_url(&self, url: &str) {
        let url = url.to_string();
        std::thread::spawn(move || {
            let _ = std::process::Command::new("xdg-open")
                .arg(url)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        });
    }

    /// Poll the job snapshot once a second while a login is in flight.
    fn poll_login(&mut self) {
        // A rejoined job may be minutes from its deadline: extend it once, up front.
        let low = self.login.as_ref().is_some_and(|l| {
            !l.auto_extended
                && l.job
                    .as_ref()
                    .is_some_and(|j| !j.terminal() && !j.deadline_fixed && secs_left(j.deadline_at.as_deref()).is_some_and(|s| s < 600))
        });
        if low {
            if let Some(l) = self.login.as_mut() {
                l.auto_extended = true;
            }
            self.extend_login();
        }
        let Some(l) = self.login.as_mut() else { return };
        let Some(id) = l.job.as_ref().filter(|j| !j.terminal()).map(|j| j.job_id.clone()) else { return };
        if !l.polling && l.last_poll.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
            l.polling = true;
            l.last_poll = Some(Instant::now());
            self.spawn(move |c| Msg::LoginSnap(c.login_snapshot(&id)));
        }
        self.ctx.request_repaint_after(Duration::from_secs(1));
    }

    fn login_settled(&mut self) {
        let done = self.login.as_ref().and_then(|l| l.job.as_ref()).is_some_and(|j| j.terminal());
        if !done {
            return;
        }
        if let Some(l) = self.login.as_mut() {
            l.disclosure = None; // the sidecar is gone once terminal; never show a stale link
        }
        // readiness, models and quota all change after a login
        self.harnesses.clear();
        self.models.clear();
        self.spawn(|c| Msg::Harnesses(c.harnesses()));
        self.refresh_quota(false);
    }

    // ---- reducer ------------------------------------------------------------------------

    /// Drain worker messages; called once per frame before drawing.
    pub fn pump(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            self.apply(m);
        }
        self.poll_login();
        // Fallback poll only while a turn in the open thread is pending with no
        // stream to tell us about it (e.g. queued from the CLI). Idle = no timer.
        if self.needs_poll() {
            let due = self.last_poll.is_none_or(|t| t.elapsed() >= Duration::from_secs(2));
            if due {
                self.last_poll = Some(Instant::now());
                self.refresh_detail();
            }
            self.ctx.request_repaint_after(Duration::from_secs(2));
        }
    }

    fn needs_poll(&self) -> bool {
        let Some(d) = &self.detail else { return false };
        d.turns.iter().any(|t| {
            let pending = t.run.as_ref().is_none_or(|r| matches!(r.state.as_str(), "queued" | "running"));
            pending && t.enqueue_error.is_none() && self.live_of(t).is_none_or(|l| !l.streaming() && l.terminal.is_none())
        })
    }

    fn apply(&mut self, m: Msg) {
        match m {
            Msg::Online(client, h) => {
                let engine = h.engine.as_ref().and_then(|e| e.version.clone()).unwrap_or_else(|| "unknown".into());
                self.engine_version = Some(engine.clone());
                self.conn = Conn::Online { engine };
                self.client = Some(client);
                self.threads_flight = Flight::default();
                self.detail_flight = Flight::default();
                self.run_flight.clear();
                self.refresh_threads();
                self.refresh_detail();
                if self.harnesses.is_empty() {
                    self.spawn(|c| Msg::Harnesses(c.harnesses()));
                    self.spawn(|c| Msg::Projects(c.projects()));
                }
                self.refresh_quota(false);
                // Resume every stream the restart cut, from its last applied seq.
                let lost: Vec<String> = self.runs.iter().filter(|(_, l)| l.lost && l.terminal.is_none()).map(|(k, _)| k.clone()).collect();
                for id in lost {
                    self.watch(&id);
                }
            }
            Msg::Recovering => {
                self.conn = Conn::Recovering;
            }
            Msg::Offline(why) => {
                self.conn = Conn::Offline(why);
                self.client = None;
                self.composer.sending = false;
            }
            Msg::Incompatible(why) => {
                self.conn = Conn::Incompatible(why);
                self.client = None;
            }
            Msg::ThreadsStale => {
                self.refresh_threads();
                self.refresh_detail();
            }
            Msg::Threads(r) => {
                let again = self.threads_flight.finish();
                match r {
                    Ok(list) => {
                        // keep trashed rows too: the sidebar's Trash view lists them
                        let mut threads: Vec<Thread> = list.threads;
                        threads.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
                        self.threads = threads;
                        self.threads_loaded = true;
                    }
                    Err(e) => self.note(&e),
                }
                if again {
                    self.refresh_threads();
                }
            }
            Msg::Thread(id, r) => {
                let again = self.detail_flight.finish();
                if self.selected.as_deref() == Some(id.as_str()) {
                    self.detail_loading = false;
                    match r {
                        Ok(d) => self.adopt_detail(d),
                        Err(e) => self.note(&e),
                    }
                }
                if again {
                    self.refresh_detail();
                }
            }
            Msg::Created(r, pending) => match r {
                Ok(thread) => {
                    let tid = thread.id.clone();
                    self.threads.insert(0, thread);
                    self.selected = Some(tid.clone());
                    self.detail = None;
                    self.detail_flight = Flight::default();
                    let req = pending.req;
                    self.spawn(move |c| Msg::TurnSent(tid.clone(), c.send_turn(&tid, &req), Some(req.prompt)));
                }
                Err(e) => {
                    self.composer.sending = false;
                    self.note(&e);
                }
            },
            Msg::TurnSent(tid, r, prompt) => {
                self.composer.sending = false;
                match r {
                    Ok(started) => {
                        if prompt.is_some() && self.composer.text.trim() == prompt.as_deref().unwrap_or("").trim() {
                            self.composer.text.clear();
                            self.composer.attachments.clear();
                        }
                        if let Some(sid) = started.stream_id().map(str::to_owned) {
                            if let Some(turn) = &started.turn_id {
                                self.turn_stream.insert(turn.clone(), sid.clone());
                            }
                            self.watch(&sid);
                        }
                        if self.selected.as_deref() == Some(tid.as_str()) {
                            self.refresh_detail();
                        }
                        self.refresh_threads();
                    }
                    Err(e) => {
                        self.note(&e);
                        if self.selected.as_deref() == Some(tid.as_str()) {
                            self.refresh_detail();
                        }
                    }
                }
            }
            Msg::Event(sid, seq, ev) => {
                let kind = ev.get("type").and_then(Value::as_str).unwrap_or("").to_string();
                let run_id = ev.get("run_id").and_then(Value::as_str).map(str::to_owned);
                let Some(live) = self.runs.get_mut(&sid) else { return };
                if seq <= live.transcript.last_seq {
                    return; // replayed after resume
                }
                live.transcript.apply(seq, &ev);
                match kind.as_str() {
                    "run.created" | "harness.started" => live.phase = "running".into(),
                    "interaction.requested" => live.waiting = true,
                    "interaction.answered" | "interaction.timeout" | "interaction.answer_discarded" => live.waiting = false,
                    "run.completed" | "run.failed" | "run.blocked" => {
                        live.terminal = Some(kind.clone());
                        live.phase = kind.trim_start_matches("run.").into();
                        live.waiting = false;
                        live.finished = Some(Instant::now());
                    }
                    _ => {}
                }
                let waiting = live.waiting;
                // Desktop notice for LIVE transitions only: replaying an old run's log
                // (expanding its activity) must not re-announce it.
                let fresh = ev
                    .get("ts")
                    .and_then(Value::as_str)
                    .and_then(crate::ui::theme::parse_iso)
                    .is_some_and(|t| crate::ui::theme::now_unix() - t < 60);
                let word = match kind.as_str() {
                    "run.completed" => Some("Done"),
                    "run.blocked" => Some("Needs a decision"),
                    "run.failed" => Some("Failed"),
                    "interaction.requested" => Some("Needs your answer"),
                    _ => None,
                };
                if let (true, Some(word)) = (fresh, word) {
                    let tid = ev.get("thread_id").and_then(Value::as_str);
                    let title = tid.and_then(|t| self.threads.iter().find(|x| x.id == t)).map(|t| t.display_title().to_string());
                    self.notices
                        .push(Notice {
                            title: format!("Claudexor · {word}"), body: title.unwrap_or_else(|| "A run changed state".into())
                        });
                }
                if let Some(rid) = run_id {
                    // A queued turn streamed by job id: alias the run id to the same box
                    // is unnecessary — lookups go through turn_stream — but interactions
                    // and answers need the real run id.
                    if kind == "interaction.requested" && waiting {
                        self.run_flight.remove(&rid);
                        self.load_run(&rid, false);
                    }
                    // Keep an open details section current, at most every 2 s.
                    let deep = ["gate.", "review.", "plan.progress", "route.", "attempt."].iter().any(|p| kind.starts_with(p));
                    let due = self.detail_at.get(&rid).is_none_or(|t| t.elapsed() >= Duration::from_secs(2));
                    if deep && due && self.details.contains_key(&rid) {
                        self.detail_at.insert(rid.clone(), Instant::now());
                        self.load_run(&rid, false);
                    }
                    // Terminal: `output.ready` provably preceded it, so the answer is fetchable.
                    // A replay of an already-answered run must not refetch (or flicker) it.
                    let terminal = matches!(kind.as_str(), "run.completed" | "run.failed" | "run.blocked");
                    if terminal && !matches!(self.answers.get(&rid), Some(AnswerState::Ready(_))) {
                        self.run_flight.remove(&rid);
                        self.answers.remove(&rid);
                        self.load_run(&rid, true);
                        self.refresh_detail();
                    }
                }
            }
            Msg::StreamEnd(sid, end) => {
                if let Some(live) = self.runs.get_mut(&sid) {
                    live.stop = None;
                    match end {
                        StreamEnd::Terminal => {
                            if live.terminal.is_none() {
                                live.terminal = Some("run.ended".into());
                                live.phase = "ended".into();
                            }
                        }
                        StreamEnd::Lost => {
                            live.lost = true;
                            // Cut after the supervisor already reported Online again
                            // (or a transient drop): retry once a second while online.
                            if self.client.is_some() && live.terminal.is_none() {
                                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                                std::thread::spawn(move || {
                                    std::thread::sleep(Duration::from_secs(1));
                                    Self::send(&tx, &ctx, Msg::Rewatch(sid));
                                });
                            }
                        }
                        StreamEnd::Refused(why) => {
                            live.lost = false;
                            if live.terminal.is_none() {
                                live.phase = format!("unavailable: {why}");
                            }
                        }
                    }
                }
            }
            Msg::Run(id, r) => {
                self.run_flight.remove(&id);
                match r {
                    Ok(d) => {
                        self.details.insert(id, d);
                    }
                    Err(e) => self.note(&e),
                }
            }
            Msg::Answer(id, r) => {
                let s = match r {
                    Ok(Some(t)) => AnswerState::Ready(t),
                    Ok(None) => AnswerState::None,
                    Err(e) => AnswerState::Failed(e.to_string()),
                };
                self.answers.insert(id, s);
            }
            Msg::Harnesses(r) => match r {
                Ok(h) => {
                    self.harnesses = h.harnesses;
                    self.harnesses.sort_by_key(|h| (h.status != "ok", h.id.clone()));
                }
                Err(e) => self.note(&e),
            },
            Msg::Trust(root, r) => {
                match r {
                    // a failed grant keeps the state we already had
                    Err(e) if matches!(self.trust.get(&root), Some(Ok(_))) => self.error = Some(format!("Trust: {e}")),
                    r => {
                        self.trust.insert(root, r.map_err(|e| e.to_string()));
                    }
                }
            }
            Msg::Models(h, r) => {
                self.models.insert(h, r.map_err(|e| e.to_string()));
            }
            Msg::Quota(r) => {
                self.quota_loading = false;
                match r {
                    Ok(q) => {
                        self.quota = Some(q);
                        self.quota_error = None;
                    }
                    Err(e) => self.quota_error = Some(e.to_string()),
                }
            }
            Msg::Pools(r) => {
                if let Ok(p) = r {
                    self.pools = p.account_pools;
                }
            }
            Msg::Projects(r) => {
                if let Ok(p) = r {
                    self.projects = p.projects;
                    if self.composer.project.is_none() {
                        self.composer.project = self.projects.first().map(|p| p.root.clone());
                    }
                }
            }
            Msg::ThreadChanged(what, r) => {
                if let Err(e) = r {
                    self.error = Some(format!("{what} failed: {e}"));
                }
                self.refresh_threads();
                self.refresh_detail();
            }
            Msg::Profiles(r) => match r {
                Ok(p) => self.profiles = Some(p),
                Err(e) => self.note(&e),
            },
            Msg::ProfileChanged(what, r, login_after) => {
                match r {
                    Ok(()) => {
                        if let Some((h, p)) = login_after {
                            self.start_login_for(&h, Some(p));
                        }
                    }
                    Err(e) => self.error = Some(format!("{what} failed: {e}")),
                }
                self.refresh_profiles();
                self.refresh_quota(false);
            }
            Msg::Picked(r) => {
                self.composer.picking = false;
                match r {
                    Ok(paths) => self.attach_paths(paths),
                    Err(e) => self.error = Some(e),
                }
            }
            Msg::Uploaded(local, r) => {
                if let Some(a) = self.composer.attachments.iter_mut().find(|a| a.local == local) {
                    a.state = match r {
                        Ok(res) => AttachState::Ready(res.resource_id),
                        Err(e) => AttachState::Failed(e),
                    };
                }
            }
            Msg::EngineStarting => {
                self.engine_starting = true;
                self.engine_start_error = None;
            }
            Msg::EngineStart(r) => {
                self.engine_starting = false;
                if let Err(e) = r {
                    self.engine_start_error = Some(e);
                }
            }
            Msg::LoginJob(r) => {
                if let Some(l) = self.login.as_mut() {
                    match r {
                        Ok(job) => {
                            l.job = Some(job);
                            l.last_poll = None; // snapshot right away for the link
                        }
                        Err(e) => l.error = Some(e.to_string()),
                    }
                }
                self.login_settled();
            }
            Msg::LoginSnap(r) => {
                if let Some(l) = self.login.as_mut() {
                    l.polling = false;
                    match r {
                        Ok(snap) => {
                            l.disclosure = snap.device_code;
                            l.job = Some(snap.job);
                            l.error = None;
                        }
                        Err(e) => l.error = Some(e.to_string()),
                    }
                }
                self.login_settled();
            }
            Msg::LoginInput(r) => {
                if let Some(l) = self.login.as_mut() {
                    match r {
                        Ok(job) => l.job = Some(job),
                        Err(e) => {
                            l.code_sent = false;
                            l.error = Some(format!("code not accepted: {e}"));
                        }
                    }
                    l.last_poll = None;
                }
                self.login_settled();
            }
            Msg::Rewatch(sid) => {
                if self.client.is_some() && self.runs.get(&sid).is_some_and(|l| l.lost && !l.streaming()) {
                    self.watch(&sid);
                }
            }
            Msg::Acked(what, r) => {
                if let Err(e) = r {
                    self.error = Some(format!("{what} failed: {e}"));
                }
                self.refresh_detail();
            }
        }
    }

    /// Take a fresh thread detail: start streams for live turns and remember
    /// which terminal turns still need their answer.
    fn adopt_detail(&mut self, d: ThreadDetail) {
        for turn in &d.turns {
            let Some(run) = &turn.run else { continue };
            let active = matches!(run.state.as_str(), "queued" | "running");
            if let Some(sid) = turn.run_id.clone().or_else(|| self.turn_stream.get(&turn.id).cloned()) {
                let known = self.runs.get(&sid).is_some_and(|l| l.terminal.is_some());
                if active && !known {
                    self.watch(&sid);
                }
            }
            if run.waiting_on_user == Some(true) {
                if let Some(rid) = &turn.run_id {
                    if !self.details.get(rid).is_some_and(|x| !x.pending_interactions.is_empty()) {
                        self.load_run(rid, false);
                    }
                }
            }
        }
        self.detail = Some(d);
    }

    /// Ask for the answer of a terminal run once (called while drawing).
    pub fn ensure_answer(&mut self, run_id: &str) {
        if !self.answers.contains_key(run_id) {
            self.load_run(run_id, true);
        }
    }

    /// Replay a finished run's full event log into its activity box.
    pub fn ensure_replay(&mut self, run_id: &str) {
        let needs = self.runs.get(run_id).is_none_or(|l| l.transcript.last_seq == 0 && !l.streaming());
        if needs {
            self.watch(run_id);
        }
    }

    fn note(&mut self, e: &ApiError) {
        if e.is_transport() {
            return; // the supervisor owns offline presentation
        }
        self.error = Some(e.to_string());
    }
}

/// The run's user-facing answer: bounded inline `primaryOutput` first, else the
/// first answer-like artifact. Patch/diagnostic outputs are not answers.
fn answer_text(c: &Client, run_id: &str, d: &RunDetail) -> Result<Option<String>, ApiError> {
    let diagnostic = d.summary.output_ready_state.as_deref() == Some("diagnostic");
    if let Some(p) = &d.primary_output {
        if let Some(t) = p.text.as_deref().filter(|t| !t.trim().is_empty()) {
            if p.kind == "patch" || p.kind == "diagnostic" || diagnostic {
                return Ok(None);
            }
            let mut text = t.to_string();
            if p.truncated {
                text.push_str(&format!("\n\n_Inline preview bounded; open `{}` for the full output._", p.path));
            }
            return Ok(Some(text));
        }
    }
    if diagnostic {
        return Ok(None);
    }
    for path in ANSWER_PATHS {
        match c.artifact_text(run_id, path) {
            Ok(t) if !t.trim().is_empty() => return Ok(Some(t)),
            Ok(_) => {}
            Err(ApiError::Http { status: 404 | 410, .. }) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// Human line for a run failure (`summary.failure`).
pub fn failure_line(f: &RunFailure) -> Option<String> {
    let code = f.code.as_deref().or(f.category.as_deref());
    match (f.safe_message.trim(), code) {
        ("", None) => None,
        ("", Some(c)) => Some(c.to_string()),
        (m, Some(c)) => Some(format!("{m} ({c})")),
        (m, None) => Some(m.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flight_is_single_with_one_trailing() {
        let mut f = Flight::default();
        assert!(f.start());
        assert!(!f.start());
        assert!(!f.start());
        assert!(f.finish(), "one trailing request after a storm");
        assert!(f.start());
        assert!(!f.finish());
    }

    #[test]
    fn failure_line_uses_safe_message_and_code() {
        let f: RunFailure = serde_json::from_value(serde_json::json!({
            "phase": "routing", "category": "harness_unavailable", "code": null,
            "safeMessage": "no doctor-OK default route", "nextActions": []
        }))
        .unwrap();
        assert_eq!(failure_line(&f).as_deref(), Some("no doctor-OK default route (harness_unavailable)"));
        let f: RunFailure = serde_json::from_value(serde_json::json!({"category": "budget", "code": "hard_cap"})).unwrap();
        assert_eq!(failure_line(&f).as_deref(), Some("hard_cap"));
    }
}

/// The `claudexor` CLI: PATH first, then the usual npm/nvm locations (a
/// launcher-started app does not inherit nvm's shell PATH).
pub fn find_claudexor() -> Option<std::path::PathBuf> {
    let on_path =
        std::env::var_os("PATH").into_iter().flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).map(|d| d.join("claudexor"));
    let home = dirs::home_dir().unwrap_or_default();
    let mut extra =
        vec![home.join(".claudexor/node/bin/claudexor"), home.join(".local/bin/claudexor"), home.join(".npm-global/bin/claudexor")];
    if let Ok(rd) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        let mut vs: Vec<_> = rd.flatten().map(|e| e.path().join("bin/claudexor")).collect();
        vs.sort();
        extra.extend(vs.into_iter().rev()); // newest node first
    }
    on_path.chain(extra).find(|p| p.is_file())
}

/// `claudexor daemon start` (returns once the daemon reports ready or fails).
pub fn start_engine_blocking() -> Result<String, String> {
    let bin = find_claudexor().ok_or("claudexor CLI not found (install it with `npm install -g claudexor`)")?;
    let mut cmd = std::process::Command::new(&bin);
    cmd.args(["daemon", "start"]).stdin(std::process::Stdio::null());
    // nvm installs need their node next to the script's `#!/usr/bin/env node`.
    if let Some(dir) = bin.parent() {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![dir.to_path_buf()];
        paths.extend(std::env::split_paths(&path));
        if let Ok(p) = std::env::join_paths(paths) {
            cmd.env("PATH", p);
        }
    }
    let out = cmd.output().map_err(|e| format!("could not run {}: {e}", bin.display()))?;
    let text = String::from_utf8_lossy(if out.status.success() { &out.stdout } else { &out.stderr }).trim().to_string();
    if out.status.success() { Ok(text) } else { Err(if text.is_empty() { format!("daemon start exited {}", out.status) } else { text }) }
}

/// zenity (GTK) or kdialog file chooser; Ok(empty) when the user cancels.
fn run_file_picker() -> Result<Vec<std::path::PathBuf>, String> {
    use std::process::Command;
    let out = Command::new("zenity")
        .args(["--file-selection", "--multiple", "--separator=\n", "--title=Attach files"])
        .output()
        .or_else(|_| Command::new("kdialog").args(["--getopenfilename", ".", "--multiple", "--separate-output"]).output())
        .map_err(|_| "no file chooser found (install zenity or kdialog)".to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.trim().is_empty()).map(std::path::PathBuf::from).collect())
}

/// `slurp` picks a region, `grim` captures it into a private temp file.
/// A cancelled selection yields no attachment — never a blank image.
fn run_capture() -> Result<Vec<std::path::PathBuf>, String> {
    use std::process::Command;
    let region = Command::new("slurp").output().map_err(|_| "screen capture needs grim + slurp".to_string())?;
    if !region.status.success() {
        return Ok(vec![]); // Esc in slurp
    }
    let geometry = String::from_utf8_lossy(&region.stdout).trim().to_string();
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let path = dir.join(format!("claudexor-capture-{stamp}.png"));
    let st = Command::new("grim").args(["-g", &geometry]).arg(&path).status().map_err(|e| e.to_string())?;
    if !st.success() || !path.is_file() {
        return Err("grim could not capture the region".into());
    }
    Ok(vec![path])
}

/// Upload kind + MIME from the file name (the daemon revalidates the bytes).
pub fn mime_for(name: &str) -> (&'static str, &'static str) {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => ("image", "image/png"),
        "jpg" | "jpeg" => ("image", "image/jpeg"),
        "gif" => ("image", "image/gif"),
        "webp" => ("image", "image/webp"),
        "pdf" => ("file", "application/pdf"),
        "json" => ("file", "application/json"),
        "md" | "markdown" => ("file", "text/markdown"),
        "txt" | "log" | "rs" | "ts" | "js" | "py" | "go" | "java" | "c" | "h" | "cpp" | "toml" | "yaml" | "yml" | "sh" | "swift" | "kt"
        | "rb" | "css" | "html" | "sql" | "csv" | "xml" => ("file", "text/plain"),
        _ => ("file", "application/octet-stream"),
    }
}

#[cfg(test)]
mod attach_tests {
    #[test]
    fn mime_detection() {
        assert_eq!(super::mime_for("Shot.PNG"), ("image", "image/png"));
        assert_eq!(super::mime_for("main.rs"), ("file", "text/plain"));
        assert_eq!(super::mime_for("blob"), ("file", "application/octet-stream"));
    }
}
