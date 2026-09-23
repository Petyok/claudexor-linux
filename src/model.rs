//! Wire types for the ~12 control-API DTOs the UI reads.
//!
//! Tolerant by design (PLAN §3): no `deny_unknown_fields`, every non-identity
//! field optional or defaulted, list responses decoded per row so one
//! schema-skewed record degrades the display instead of blanking it. Shapes are
//! taken from `packages/schema/generated/*.schema.json` (claudexor v3.13.0).

#![allow(dead_code)] // wire mirror: fields exist for tolerant decoding + fixtures, not all are drawn

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Decode a JSON array row by row, dropping (and counting) rows that fail.
pub fn lossy_vec<T: DeserializeOwned>(rows: Vec<Value>) -> (Vec<T>, usize) {
    let mut ok = Vec::with_capacity(rows.len());
    let mut dropped = 0;
    for row in rows {
        match serde_json::from_value(row) {
            Ok(v) => ok.push(v),
            Err(_) => dropped += 1,
        }
    }
    (ok, dropped)
}

fn lossy<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    let rows: Option<Vec<Value>> = Option::deserialize(d)?;
    Ok(lossy_vec(rows.unwrap_or_default()).0)
}

// ---- discovery + handshake -------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    pub host: String,
    pub port: u16,
    pub token_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Handshake {
    pub protocol_major: i64,
    #[serde(default)]
    pub compatible: bool,
    #[serde(default)]
    pub engine: Option<Engine>,
    /// `normal` | `recovery_only`; absent on pre-#165 daemons (= normal).
    #[serde(default)]
    pub serving_mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Engine {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub sha: Option<String>,
}

/// `ControlProblem`: the typed error body of every route.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub required_actions: Vec<String>,
}

// ---- threads ---------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub repo_root: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub workspace_mode: Option<String>,
    #[serde(default)]
    pub primary_harness: Option<String>,
    /// `active` | `closed`.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub trashed_at: Option<String>,
    #[serde(default)]
    pub head_run_id: Option<String>,
    #[serde(default)]
    pub needs_human: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

impl Thread {
    pub fn display_title(&self) -> &str {
        match self.title.as_deref() {
            Some(t) if !t.trim().is_empty() => t,
            _ => "Untitled thread",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThreadList {
    #[serde(default, deserialize_with = "lossy")]
    pub threads: Vec<Thread>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThreadDetail {
    pub thread: Thread,
    #[serde(default, deserialize_with = "lossy")]
    pub turns: Vec<Turn>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub id: String,
    #[serde(default)]
    pub run_id: Option<String>,
    /// `initial` | `followup` | ...
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub run: Option<TurnRun>,
    #[serde(default)]
    pub enqueue_error: Option<EnqueueError>,
    #[serde(default)]
    pub created_at: String,
    /// This turn answers the open questions of that plan run.
    #[serde(default)]
    pub answers_plan_run_id: Option<String>,
    /// This turn implements that plan run (frozen by hash).
    #[serde(default)]
    pub plan_run_id: Option<String>,
    #[serde(default)]
    pub plan_hash: Option<String>,
    #[serde(default)]
    pub plan_readiness_overridden: bool,
    /// How context reached this turn (native resume, or a replayed packet).
    #[serde(default)]
    pub continuity: Option<Continuity>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Continuity {
    /// native_resume | packet | …
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub packet_turns: u32,
    #[serde(default)]
    pub summarized: bool,
    #[serde(default)]
    pub lane_switched_from: Option<LaneFrom>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneFrom {
    #[serde(default)]
    pub harness: Option<String>,
}

/// The compact run card embedded on a turn (no N+1 detail fetch for the list).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnRun {
    /// queued | running | succeeded | failed | cancelled | interrupted
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub result: Option<RunResult>,
    #[serde(default)]
    pub spend_usd: Option<f64>,
    /// pending | finalizing | ready | diagnostic
    #[serde(default)]
    pub output_ready_state: Option<String>,
    #[serde(default)]
    pub waiting_on_user: Option<bool>,
    #[serde(default)]
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    /// patch | answer | plan | report | none
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub diff_stat: Option<DiffStat>,
    #[serde(default)]
    pub apply_state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiffStat {
    #[serde(default)]
    pub files: u64,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueError {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub code: Option<String>,
    /// None (older daemons) reads as retryable.
    #[serde(default)]
    pub retryable: Option<bool>,
    #[serde(default)]
    pub required_actions: Vec<String>,
}

// ---- requests ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    None,
    Project { root: String, context: &'static str },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateThread {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub scope: Scope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_harness: Option<String>,
    /// in_place (default) | isolated: a persistent thread worktree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnRequest {
    pub prompt: String,
    /// ask | plan | agent — the daemon defaults to agent, so always send it.
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Pin this turn to one account; unknown/disabled ids refuse, never default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_profile_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ResourceRef>,
    /// A Plan follow-up that answers this plan run's open questions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answers_plan_run_id: Option<String>,
    /// An Agent turn that implements this (frozen) plan run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_run_id: Option<String>,
    /// Explicit, recorded override: implement although questions remain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_plan_readiness: Option<bool>,
    /// Requested write scope (Agent only): readonly | workspace_write | full.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access: Option<String>,
    /// Explicit eligible pool (Best-of candidates, Council members).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub harnesses: Vec<String>,
    /// Race width (Best-of) or Council member count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    /// Plain-Agent repair cap; never together with `untilClean`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until_clean: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub council: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deep_scan: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegate: Option<bool>,
    /// `{"kind":"finite","maxUsd":N}`; absent = the settings default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paid_budget: Option<Value>,
    /// Harness-scoped model map; beats the scalar `model`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reviewer_panel: Vec<ReviewerEntry>,
    /// off | auto | cached | live
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser: Option<bool>,
    /// subscription | api_key | auto
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_preference: Option<String>,
}

/// One explicit reviewer: `harness[=model[:effort]]`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewerEntry {
    pub harness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

/// Parse the reviewer-panel editor: comma-separated `harness=model:effort`
/// entries (model and effort optional). Err names the first bad entry.
pub fn parse_reviewer_panel(text: &str) -> Result<Vec<ReviewerEntry>, String> {
    let slug = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.[]".contains(c));
    let mut out = vec![];
    for raw in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (harness, rest) = match raw.split_once('=') {
            Some((h, r)) => (h.trim(), Some(r.trim())),
            None => (raw, None),
        };
        let (model, effort) = match rest {
            Some(r) => match r.split_once(':') {
                Some((m, e)) => (Some(m.trim()).filter(|m| !m.is_empty()), Some(e.trim()).filter(|e| !e.is_empty())),
                None => (Some(r).filter(|m| !m.is_empty()), None),
            },
            None => (None, None),
        };
        if !slug(harness) || model.is_some_and(|m| !slug(m)) || effort.is_some_and(|e| !slug(e)) {
            return Err(format!("“{raw}” is not harness=model:effort"));
        }
        out.push(ReviewerEntry { harness: harness.into(), model: model.map(Into::into), effort: effort.map(Into::into) });
    }
    Ok(out)
}

/// Agent execution strategy (Single is the default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Strategy {
    #[default]
    Single,
    BestOf,
    UntilClean,
    Create,
}

/// Composer "Options": per-turn knobs, sticky across sends. `None` fields mean
/// "engine default" and stay off the wire.
#[derive(Debug, Clone)]
pub struct TurnOpts {
    /// readonly | workspace_write | full; None = the repo's trust default.
    pub access: Option<&'static str>,
    pub strategy: Strategy,
    /// Single-candidate repair cap (1..=8).
    pub attempts: u32,
    /// Explicit Best-of pool; empty = let the engine pick.
    pub pool: Vec<String>,
    /// Per-harness model for this turn (beats the scalar model).
    pub models: BTreeMap<String, String>,
    pub council: bool,
    /// Council members (2..=4).
    pub members: u32,
    pub deep_scan: bool,
    pub delegate: bool,
    pub browser: bool,
    /// Max paid spend in USD, as typed; empty = the settings default.
    pub budget: String,
    /// off | auto | cached | live
    pub web: Option<&'static str>,
    /// auto | subscription | api_key
    pub auth: Option<&'static str>,
    pub review: bool,
    /// `harness[=model[:effort]]`, comma separated.
    pub panel: String,
}

impl Default for TurnOpts {
    fn default() -> Self {
        TurnOpts {
            access: None,
            strategy: Strategy::Single,
            attempts: 3,
            pool: vec![],
            models: BTreeMap::new(),
            council: false,
            members: 2,
            deep_scan: false,
            delegate: false,
            browser: false,
            budget: String::new(),
            web: None,
            auth: None,
            review: false,
            panel: String::new(),
        }
    }
}

impl TurnOpts {
    /// Strategy as it will be sent: Until clean cannot run read-only.
    pub fn strategy_for(&self, access: &str) -> Strategy {
        if access == "readonly" && self.strategy == Strategy::UntilClean { Strategy::Single } else { self.strategy }
    }

    /// Write these options onto `req` (whose `mode` is already set). `access` is
    /// the effective profile (explicit pick, else the repo default). Errors are
    /// user-facing and block Send.
    pub fn apply(&self, req: &mut TurnRequest, access: &str) -> Result<(), String> {
        let budget = self.budget.trim();
        if !budget.is_empty() {
            match budget.trim_start_matches('$').parse::<f64>() {
                Ok(usd) if usd.is_finite() && usd > 0.0 => req.paid_budget = Some(serde_json::json!({"kind": "finite", "maxUsd": usd})),
                _ => return Err(format!("Budget “{budget}” is not a positive dollar amount")),
            }
        }
        req.web = self.web.map(Into::into);
        req.auth_preference = self.auth.map(Into::into);
        req.models = self.models.iter().filter(|(_, m)| !m.trim().is_empty()).map(|(h, m)| (h.clone(), m.trim().into())).collect();
        match req.mode.as_str() {
            "ask" => req.deep_scan = self.deep_scan.then_some(true),
            "plan" if self.council => {
                req.council = Some(true);
                req.n = Some(self.members.clamp(2, 4));
            }
            "agent" => {
                let panel = parse_reviewer_panel(&self.panel)?;
                let writable = access != "readonly";
                req.access = self.access.map(Into::into);
                req.delegate = self.delegate.then_some(true);
                req.browser = self.browser.then_some(true);
                let promised_review = match self.strategy_for(access) {
                    Strategy::Single => {
                        req.attempts = writable.then_some(self.attempts.clamp(1, 8));
                        false
                    }
                    Strategy::BestOf => {
                        req.harnesses = self.pool.clone();
                        req.n = Some(if self.pool.len() == 1 { 1 } else { self.pool.len().max(2) as u32 });
                        true
                    }
                    Strategy::UntilClean => {
                        req.until_clean = Some(true);
                        true
                    }
                    Strategy::Create => {
                        req.create = Some(true);
                        false
                    }
                };
                req.review = Some(promised_review || self.review || !panel.is_empty());
                req.reviewer_panel = panel;
            }
            _ => {}
        }
        Ok(())
    }

    /// How many knobs differ from the defaults, for the Options badge.
    pub fn changed(&self, mode: &str) -> usize {
        let d = TurnOpts::default();
        let common = [self.web.is_some(), self.auth.is_some(), !self.budget.trim().is_empty(), !self.models.is_empty()];
        let per_mode: Vec<bool> = match mode {
            "ask" => vec![self.deep_scan],
            "plan" => vec![self.council],
            "agent" => vec![
                self.access.is_some(),
                self.strategy != d.strategy,
                self.strategy == Strategy::Single && self.attempts != d.attempts,
                self.delegate,
                self.browser,
                self.review,
                !self.panel.trim().is_empty(),
            ],
            _ => vec![],
        };
        common.iter().chain(&per_mode).filter(|b| **b).count()
    }
}

impl TurnRequest {
    pub fn new(prompt: impl Into<String>, mode: &str) -> Self {
        TurnRequest {
            prompt: prompt.into(),
            mode: mode.into(),
            primary_harness: None,
            model: None,
            effort: None,
            credential_profile_id: None,
            attachments: vec![],
            answers_plan_run_id: None,
            plan_run_id: None,
            override_plan_readiness: None,
            access: None,
            harnesses: vec![],
            n: None,
            attempts: None,
            until_clean: None,
            create: None,
            council: None,
            deep_scan: None,
            delegate: None,
            paid_budget: None,
            models: BTreeMap::new(),
            review: None,
            reviewer_panel: vec![],
            web: None,
            browser: None,
            auth_preference: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRef {
    pub resource_id: String,
}

/// `ControlResource` (upload finalize result).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub resource_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadStatus {
    pub upload_id: String,
}

// ---- credential profiles (accounts) ------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profiles {
    #[serde(default, deserialize_with = "lossy")]
    pub profiles: Vec<ProfileRow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProfileRow {
    pub profile: Profile,
    #[serde(default)]
    pub status: Option<ProfileStatus>,
    #[serde(default)]
    pub identity: Option<Identity>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub profile_id: String,
    pub harness_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub credential_kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProfileStatus {
    /// available | unavailable | unknown
    #[serde(default)]
    pub availability: String,
    /// passed | failed | not_run
    #[serde(default)]
    pub verification: String,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Identity {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub plan: Option<String>,
}

impl ProfileRow {
    /// Ready means THIS exact source is available + passed, never aggregate harness health.
    pub fn ready(&self) -> bool {
        self.status.as_ref().is_some_and(|s| s.availability == "available" && s.verification == "passed")
    }
    pub fn label(&self) -> String {
        self.identity
            .as_ref()
            .and_then(|i| i.email.clone())
            .or_else(|| self.profile.display_name.clone())
            .unwrap_or_else(|| self.profile.profile_id.clone())
    }
}

/// `ControlThreadTurnResponse`: 200 = started (runId), 202 = queued (jobId + state).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStarted {
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

impl TurnStarted {
    /// Id the run SSE accepts: the run id, or the job id while still queued.
    pub fn stream_id(&self) -> Option<&str> {
        self.run_id.as_deref().or(self.job_id.as_deref())
    }
}

// ---- runs --------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDetail {
    pub summary: RunSummary,
    #[serde(default)]
    pub primary_output: Option<PrimaryOutput>,
    /// Highest event seq already reflected in this snapshot (SSE resume cursor).
    #[serde(default)]
    pub last_seq: Option<i64>,
    #[serde(default, deserialize_with = "lossy")]
    pub pending_interactions: Vec<Interaction>,
    /// Open questions of a plan run (parsed once by the engine); empty otherwise.
    #[serde(default, deserialize_with = "lossy")]
    pub plan_questions: Vec<PlanQuestion>,
    /// ready | needs_answers | unverified; None for non-plan runs.
    #[serde(default)]
    pub plan_readiness: Option<PlanReadiness>,
    #[serde(default)]
    pub budget: Option<RunBudget>,
    /// Engine-composed one-liner, e.g. "Done · not reviewed".
    #[serde(default)]
    pub outcome_banner: Option<String>,
    #[serde(default, deserialize_with = "lossy")]
    pub candidates: Vec<Candidate>,
    #[serde(default, deserialize_with = "lossy")]
    pub review_findings: Vec<ReviewFinding>,
    #[serde(default, deserialize_with = "lossy")]
    pub children: Vec<RunSummary>,
    #[serde(default)]
    pub plan_progress: Option<PlanProgress>,
    #[serde(default, deserialize_with = "lossy")]
    pub timeline: Vec<TimelineEvent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunBudget {
    #[serde(default)]
    pub spend_usd: Option<f64>,
    /// exact | estimated | unknown
    #[serde(default)]
    pub cash_knowledge: Option<String>,
    /// What the run would have cost at list prices (subscription runs spend $0 cash).
    #[serde(default)]
    pub valuation_usd: Option<f64>,
    #[serde(default)]
    pub valuation_knowledge: Option<String>,
    #[serde(default)]
    pub paid_budget: Option<Value>,
    #[serde(default)]
    pub remaining_usd: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    #[serde(default)]
    pub attempt_id: String,
    #[serde(default)]
    pub harness_id: Option<String>,
    #[serde(default)]
    pub winner: bool,
    #[serde(default)]
    pub gates_passed: Option<u32>,
    #[serde(default)]
    pub gates_total: Option<u32>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub cost_estimated: bool,
    #[serde(default)]
    pub blockers: u32,
    #[serde(default)]
    pub review_verified: Option<bool>,
    #[serde(default)]
    pub errored: bool,
    #[serde(default)]
    pub error_reason: Option<String>,
    #[serde(default)]
    pub diffstat: Option<DiffStat>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewFinding {
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub claim: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlanProgress {
    #[serde(default, deserialize_with = "lossy")]
    pub items: Vec<PlanItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlanItem {
    #[serde(default)]
    pub title: String,
    /// pending | in_progress | completed
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEvent {
    #[serde(default)]
    pub ts: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    /// info | warn | error
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub harness_id: Option<String>,
    #[serde(default)]
    pub error_summary: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlanQuestion {
    pub id: String,
    /// single | multi | text
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default, deserialize_with = "lossy")]
    pub options: Vec<PlanOption>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlanOption {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReadiness {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub question_count: u32,
}

/// Follow-up prompt for plan answers (same encoding as ClaudexorKit's
/// `PlanAnswerComposer`): one line per question, option LABELS, or the
/// user's own words instead of the chips.
pub fn encode_plan_answers(questions: &[PlanQuestion], picked: &dyn Fn(&str) -> Vec<String>, text: &dyn Fn(&str) -> String) -> String {
    let mut lines = vec!["Answers to your plan questions:".to_string()];
    for q in questions {
        let own = text(&q.id).trim().to_string();
        let labels: Vec<String> = q.options.iter().filter(|o| picked(&q.id).contains(&o.id)).map(|o| o.label.clone()).collect();
        let parts = if own.is_empty() { labels } else { vec![own] };
        let answer = if parts.is_empty() { "(no answer)".to_string() } else { parts.join(", ") };
        lines.push(format!("- {} → {answer}", q.prompt));
    }
    lines.join("\n")
}

/// Every question answered: text needs words; single/multi need a pick or own words.
pub fn plan_answers_complete(questions: &[PlanQuestion], picked: &dyn Fn(&str) -> Vec<String>, text: &dyn Fn(&str) -> String) -> bool {
    !questions.is_empty()
        && questions.iter().all(|q| {
            let typed = !text(&q.id).trim().is_empty();
            if q.kind == "text" { typed } else { typed || !picked(&q.id).is_empty() }
        })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub output_ready_state: Option<String>,
    #[serde(default)]
    pub waiting_on_user: Option<bool>,
    #[serde(default)]
    pub failure: Option<RunFailure>,
    /// Requested vs stream-observed model (verified only on observed evidence).
    #[serde(default)]
    pub route: Option<RunRoute>,
    #[serde(default)]
    pub requested_access: Option<String>,
    #[serde(default)]
    pub effective_access: Option<String>,
    #[serde(default)]
    pub auth_route: Option<AuthRoute>,
    #[serde(default)]
    pub web_evidence: Option<WebEvidence>,
    #[serde(default)]
    pub delegation: Option<RunDelegation>,
    /// race | attempts | until_clean | create | council | …; None = single.
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthRoute {
    #[serde(default)]
    pub requested: Option<String>,
    #[serde(default)]
    pub effective: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default, rename = "profileId")]
    pub profile_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebEvidence {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub effective_mode: Option<String>,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunDelegation {
    #[serde(default)]
    pub requested: bool,
    #[serde(default)]
    pub effective: bool,
    #[serde(default)]
    pub used: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRoute {
    #[serde(default)]
    pub harness_id: Option<String>,
    #[serde(default)]
    pub requested_model: Option<String>,
    #[serde(default)]
    pub observed_model: Option<String>,
    #[serde(default)]
    pub verified: bool,
}

/// `summary.failure` (mirrors ClaudexorKit `RunFailureInfo`): surfaces show
/// `safeMessage` + the typed code, never parse the prose for remedies.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFailure {
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub safe_message: String,
    #[serde(default)]
    pub next_actions: Vec<String>,
    #[serde(default)]
    pub resets_at: Option<String>,
    /// The vendor's own typed failure: displayed as a fact, never mapped to a remedy.
    #[serde(default)]
    pub vendor_failure: Option<VendorFailure>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VendorFailure {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrimaryOutput {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Interaction {
    pub interaction_id: String,
    pub run_id: String,
    #[serde(default)]
    pub harness_id: Option<String>,
    #[serde(default, deserialize_with = "lossy")]
    pub questions: Vec<Question>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Question {
    pub id: String,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default, deserialize_with = "lossy")]
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub multi_select: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub question_id: String,
    pub selected_labels: Vec<String>,
    pub free_text: Option<String>,
}

// ---- harnesses, models ---------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct HarnessList {
    #[serde(default, deserialize_with = "lossy")]
    pub harnesses: Vec<Harness>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Harness {
    pub id: String,
    /// ok | degraded | unavailable
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub manifest: Option<Manifest>,
    #[serde(default)]
    pub configured_model: Option<String>,
    #[serde(default)]
    pub reasons: Vec<String>,
    /// Engine-owned login transport (`in_app` = daemon job, `external_terminal` = attach a PTY).
    #[serde(default)]
    pub setup_login: Option<SetupLogin>,
    #[serde(default)]
    pub delegation: Option<Delegation>,
}

/// Engine-owned Delegate capability for one harness.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delegation {
    pub available: bool,
    #[serde(default)]
    pub remediation: Option<String>,
    #[serde(default)]
    pub requires_full_access: bool,
}

/// One repo's trust file: the default access profile and the full-access grant.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustState {
    #[serde(default)]
    pub repo_root: Option<String>,
    #[serde(default)]
    pub allow_full_access: bool,
    /// readonly | workspace_write | full | inherit_native
    #[serde(default)]
    pub access_default: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrustList {
    #[serde(default)]
    pub entries: Vec<TrustState>,
}

impl Harness {
    pub fn label(&self) -> &str {
        self.manifest.as_ref().and_then(|m| m.display_name.as_deref()).unwrap_or(&self.id)
    }

    pub fn browser_tool(&self) -> bool {
        self.manifest.as_ref().and_then(|m| m.capabilities.as_ref()).and_then(|c| c.browser_tool).unwrap_or(false)
    }

    pub fn can_delegate(&self) -> bool {
        self.delegation.as_ref().is_some_and(|d| d.available)
    }

    /// Effort ladder for `model` (per-model list first, then harness-wide).
    pub fn effort_levels(&self, model: Option<&str>) -> Vec<String> {
        let Some(caps) = self.manifest.as_ref().and_then(|m| m.capabilities.as_ref()) else {
            return vec![];
        };
        if let Some(per) = model.and_then(|m| caps.model_effort_levels.get(m)) {
            if !per.levels.is_empty() {
                return per.levels.clone();
            }
        }
        caps.effort_levels.clone()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub capabilities: Option<Capabilities>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub effort_levels: Vec<String>,
    #[serde(default)]
    pub model_effort_levels: BTreeMap<String, ModelEffort>,
    /// The harness can drive a real browser window (the Browser option).
    #[serde(default)]
    pub browser_tool: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModelEffort {
    #[serde(default)]
    pub levels: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelList {
    #[serde(default)]
    pub harness_id: String,
    #[serde(default, deserialize_with = "lossy")]
    pub models: Vec<Model>,
    /// api | manifest | none
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Model {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetupLogin {
    #[serde(default)]
    pub mode: String,
}

// ---- setup (native login) jobs ---------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupJob {
    pub job_id: String,
    #[serde(default)]
    pub harness: String,
    /// queued | running | waiting_for_input | succeeded | failed | cancelled | timed_out | interrupted_unknown | not_supported
    #[serde(default)]
    pub state: String,
    /// preparing | launching | awaiting_user | verifying | cancelling | completed
    #[serde(default)]
    pub phase: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub deadline_at: Option<String>,
    /// The vendor owns the window: Extend is refused (typed 409).
    #[serde(default)]
    pub deadline_fixed: bool,
    #[serde(default)]
    pub outcome: Option<SetupOutcome>,
}

impl SetupJob {
    pub fn terminal(&self) -> bool {
        matches!(self.state.as_str(), "succeeded" | "failed" | "cancelled" | "timed_out" | "interrupted_unknown" | "not_supported")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetupOutcome {
    #[serde(default)]
    pub reason: String,
}

/// `ControlSetupJobSnapshot`: the job plus the TRANSIENT sign-in disclosure
/// (never journaled; present only while the job awaits the user).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupSnapshot {
    pub job: SetupJob,
    #[serde(default)]
    pub device_code: Option<DeviceCode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    /// chatgptDeviceCode | chatgpt | oauth_url | oauth_url_input (the last wants a pasted code back)
    #[serde(default)]
    pub flow: String,
    #[serde(default)]
    pub verification_url: String,
    #[serde(default)]
    pub user_code: String,
}

// ---- quota, pools, projects -------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Quota {
    #[serde(default, deserialize_with = "lossy")]
    pub snapshots: Vec<QuotaSnapshot>,
    #[serde(default, deserialize_with = "lossy")]
    pub absences: Vec<QuotaAbsence>,
    #[serde(default)]
    pub refreshed_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuotaSubject {
    #[serde(default)]
    pub harness: String,
    #[serde(default)]
    pub credential_route: Option<String>,
    #[serde(default)]
    pub plan_label: Option<String>,
    #[serde(default)]
    pub subject_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuotaSnapshot {
    pub subject: QuotaSubject,
    #[serde(default, deserialize_with = "lossy")]
    pub constraints: Vec<QuotaConstraint>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub observed_at: Option<String>,
    /// fresh | stale | unknown
    #[serde(default)]
    pub freshness: Option<String>,
    #[serde(default)]
    pub availability: Option<QuotaAvailability>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuotaConstraint {
    #[serde(default)]
    pub label: String,
    /// Unknown usage stays None — never render it as 0%.
    #[serde(default)]
    pub used_ratio: Option<f64>,
    #[serde(default)]
    pub window_seconds: Option<f64>,
    #[serde(default)]
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuotaAvailability {
    /// available | exhausted | cooldown
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QuotaAbsence {
    pub subject: QuotaSubject,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountPools {
    #[serde(default, deserialize_with = "lossy")]
    pub account_pools: Vec<AccountPool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccountPool {
    pub harness_id: String,
    pub next_up: NextUp,
}

/// `next_up` union: `profile` {profileId} | `api_key_route` | `none` {reason}.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NextUp {
    pub kind: String,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

impl QuotaSubject {
    /// The account this subject belongs to. Under the unified account model a
    /// default-store login (no subject id) IS the `<harness>-default` row, so
    /// both quota subjects describe one account and must not count twice.
    pub fn account_key(&self) -> (String, String, String) {
        let id = self.subject_id.clone().unwrap_or_else(|| format!("{}-default", self.harness));
        (self.harness.clone(), self.credential_route.clone().unwrap_or_default(), id)
    }
}

impl Quota {
    /// One snapshot per account (prefers the row that names its account).
    pub fn accounts(&self) -> Vec<&QuotaSnapshot> {
        let mut out: Vec<&QuotaSnapshot> = Vec::new();
        for s in &self.snapshots {
            match out.iter().position(|o| o.subject.account_key() == s.subject.account_key()) {
                Some(i) if out[i].subject.subject_id.is_none() && s.subject.subject_id.is_some() => out[i] = s,
                Some(_) => {}
                None => out.push(s),
            }
        }
        out
    }

    /// One absence per account (same rule as `accounts`).
    pub fn account_absences(&self) -> Vec<&QuotaAbsence> {
        let mut out: Vec<&QuotaAbsence> = Vec::new();
        for a in &self.absences {
            match out.iter().position(|o| o.subject.account_key() == a.subject.account_key()) {
                Some(i) if out[i].subject.subject_id.is_none() && a.subject.subject_id.is_some() => out[i] = a,
                Some(_) => {}
                None => out.push(a),
            }
        }
        out
    }
}

impl NextUp {
    pub fn describe(&self) -> String {
        match self.kind.as_str() {
            "profile" => self.profile_id.clone().unwrap_or_else(|| "account".into()),
            "api_key_route" => "API key".into(),
            // the server's reason is a sentence; callers show it as hover text
            _ => "not signed in".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectList {
    #[serde(default, deserialize_with = "lossy")]
    pub projects: Vec<Project>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub id: String,
    pub root: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Upstream fixtures wrap the value: `{"schema": "...", "value": {...}}`.
    fn fixture(name: &str) -> Value {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        v.get("value").cloned().unwrap_or(v)
    }

    #[test]
    fn decodes_handshake_fixtures() {
        for f in ["handshake-response.json", "handshake-response-unknown-sha.json", "handshake-response-recovery-only.json"] {
            let h: Handshake = serde_json::from_value(fixture(f)).unwrap();
            assert_eq!(h.protocol_major, 3, "{f}");
        }
        let h: Handshake = serde_json::from_value(fixture("handshake-response-recovery-only.json")).unwrap();
        assert_eq!(h.serving_mode.as_deref(), Some("recovery_only"));
    }

    #[test]
    fn decodes_threads_and_turns() {
        let t: Thread = serde_json::from_value(fixture("thread-minimal.json")).unwrap();
        assert_eq!(t.display_title(), "Untitled thread");
        let t: Thread = serde_json::from_value(fixture("thread-maximal.json")).unwrap();
        assert!(t.needs_human);
        for f in [
            "thread-turn-minimal.json",
            "thread-turn-delegation.json",
            "thread-turn-continuity-packet.json",
            "thread-turn-native-resume.json",
            "thread-turn-plan-implemented.json",
        ] {
            let turn: Turn = serde_json::from_value(fixture(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
            assert!(!turn.id.is_empty());
        }
    }

    #[test]
    fn thread_list_drops_bad_rows_only() {
        let list: ThreadList = serde_json::from_value(serde_json::json!({
            "threads": [fixture("thread-minimal.json"), {"title": "no id"}, fixture("thread-maximal.json")]
        }))
        .unwrap();
        assert_eq!(list.threads.len(), 2);
    }

    #[test]
    fn decodes_quota_fixtures() {
        for f in ["quota-response.json", "control-quota-response.json"] {
            let q: Quota = serde_json::from_value(fixture(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
            assert!(!q.snapshots.is_empty() || !q.absences.is_empty(), "{f}");
        }
        let q: Quota = serde_json::from_value(fixture("quota-response.json")).unwrap();
        assert_eq!(q.snapshots[0].constraints[0].used_ratio, Some(0.42));
        assert_eq!(q.absences[0].reason, "not_logged_in");
    }

    #[test]
    fn decodes_problem_fixtures() {
        let p: Problem = serde_json::from_value(fixture("problem-maximal.json")).unwrap();
        assert_eq!(p.code.as_deref(), Some("trust_full_access_required"));
        assert_eq!(p.required_actions.len(), 1);
    }

    #[test]
    fn scope_serializes_as_tagged_union() {
        let s = serde_json::to_value(Scope::Project { root: "/r".into(), context: "auto" }).unwrap();
        assert_eq!(s, serde_json::json!({"kind": "project", "root": "/r", "context": "auto"}));
        assert_eq!(serde_json::to_value(Scope::None).unwrap(), serde_json::json!({"kind": "none"}));
    }

    #[test]
    fn turn_request_omits_unset_fields() {
        let r = TurnRequest::new("hi", "ask");
        assert_eq!(serde_json::to_value(r).unwrap(), serde_json::json!({"prompt": "hi", "mode": "ask"}));
    }

    /// Every JSON recorded from the live daemon (tests/fixtures/live/*.json)
    /// must decode as the DTO its file name declares.
    #[test]
    fn decodes_live_recordings() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/live");
        let Ok(entries) = std::fs::read_dir(&dir) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".json") {
                continue;
            }
            let v: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).unwrap()).unwrap();
            let r = match name.split('.').next().unwrap_or("") {
                "handshake" => serde_json::from_value::<Handshake>(v).map(drop),
                "threads" => serde_json::from_value::<ThreadList>(v).map(drop),
                "thread" => serde_json::from_value::<ThreadDetail>(v).map(drop),
                "run" => serde_json::from_value::<RunDetail>(v).map(drop),
                "harnesses" => serde_json::from_value::<HarnessList>(v).map(drop),
                "models" => serde_json::from_value::<ModelList>(v).map(drop),
                "quota" => serde_json::from_value::<Quota>(v).map(drop),
                "account-pools" => serde_json::from_value::<AccountPools>(v).map(drop),
                "projects" => serde_json::from_value::<ProjectList>(v).map(drop),
                "turn" => serde_json::from_value::<TurnStarted>(v).map(drop),
                _ => Ok(()),
            };
            r.unwrap_or_else(|err| panic!("{name}: {err}"));
        }
    }
}

#[cfg(test)]
mod account_tests {
    use super::*;

    /// Live daemon quota: one Claude login reported as two subjects (default
    /// store with no id + the `claude-default` row) must count as ONE account.
    #[test]
    fn default_store_and_default_row_are_one_account() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/live/quota.json");
        let q: Quota = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let claude: Vec<_> = q.snapshots.iter().filter(|s| s.subject.harness == "claude").collect();
        assert_eq!(claude.len(), 2, "fixture carries both subjects");
        let accts: Vec<_> = q.accounts().into_iter().filter(|s| s.subject.harness == "claude").collect();
        assert_eq!(accts.len(), 1);
        assert_eq!(accts[0].subject.subject_id.as_deref(), Some("claude-default"), "keeps the named row");
    }

    #[test]
    fn run_route_decodes_observed_model() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/live/run.run-7ef7c842eb39.json");
        let d: RunDetail = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let r = d.summary.route.unwrap();
        assert_eq!(r.observed_model.as_deref(), Some("claude-opus-5"));
        assert!(r.verified);
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    fn qs() -> Vec<PlanQuestion> {
        serde_json::from_value(serde_json::json!([
            {"id": "q1", "kind": "single", "prompt": "Which DB?", "options": [{"id": "a", "label": "Postgres"}, {"id": "b", "label": "SQLite"}]},
            {"id": "q2", "kind": "multi", "prompt": "Targets?", "options": [{"id": "x", "label": "Linux"}, {"id": "y", "label": "macOS"}]},
            {"id": "q3", "kind": "text", "prompt": "Deadline?"}
        ]))
        .unwrap()
    }

    /// Same encoding as ClaudexorKit's PlanAnswerComposer: labels, own words win.
    #[test]
    fn encodes_like_the_mac_app() {
        let q = qs();
        let picked = |id: &str| match id {
            "q1" => vec!["b".to_string()],
            "q2" => vec!["x".to_string(), "y".to_string()],
            _ => vec![],
        };
        let text = |id: &str| if id == "q3" { " Friday ".to_string() } else { String::new() };
        assert!(plan_answers_complete(&q, &picked, &text));
        assert_eq!(
            encode_plan_answers(&q, &picked, &text),
            "Answers to your plan questions:\n- Which DB? → SQLite\n- Targets? → Linux, macOS\n- Deadline? → Friday"
        );
        let own = |id: &str| if id == "q1" { "DuckDB".to_string() } else { text(id) };
        assert!(encode_plan_answers(&q, &picked, &own).contains("- Which DB? → DuckDB"), "own words replace chips");
    }

    #[test]
    fn text_questions_need_words() {
        let q = qs();
        let picked = |id: &str| if id == "q3" { vec![] } else { vec!["a".to_string(), "x".to_string()] };
        assert!(!plan_answers_complete(&q, &picked, &|_: &str| String::new()));
        assert!(!plan_answers_complete(&[], &picked, &|_: &str| String::new()), "no questions is not an answer");
    }

    #[test]
    fn turn_request_serializes_plan_fields() {
        let mut r = TurnRequest::new("Implement this plan.", "agent");
        r.plan_run_id = Some("run-1".into());
        r.override_plan_readiness = Some(true);
        r.attachments = vec![ResourceRef { resource_id: "res-1".into() }];
        assert_eq!(
            serde_json::to_value(r).unwrap(),
            serde_json::json!({"prompt": "Implement this plan.", "mode": "agent", "planRunId": "run-1",
                               "overridePlanReadiness": true, "attachments": [{"resourceId": "res-1"}]})
        );
    }
    #[test]
    fn options_map_to_the_wire() {
        let wire = |mode: &str, o: &TurnOpts, access: &str| {
            let mut r = TurnRequest::new("p", mode);
            o.apply(&mut r, access).map(|()| serde_json::to_value(&r).unwrap())
        };
        // Defaults: Agent Single sends its repair cap and an explicit review opt-out, nothing else.
        let v = wire("agent", &TurnOpts::default(), "workspace_write").unwrap();
        assert_eq!(v["attempts"], 3);
        assert_eq!(v["review"], false);
        assert!(v.get("access").is_none() && v.get("n").is_none() && v.get("paidBudget").is_none());
        // Read-only: no repair loop; a stale Until clean degrades to Single.
        let o = TurnOpts { strategy: Strategy::UntilClean, access: Some("readonly"), ..Default::default() };
        let v = wire("agent", &o, "readonly").unwrap();
        assert!(v.get("untilClean").is_none() && v.get("attempts").is_none());
        assert_eq!(v["access"], "readonly");
        // Best-of: one-harness pool single-routes, larger pools race one per harness.
        let o = TurnOpts { strategy: Strategy::BestOf, pool: vec!["claude".into()], ..Default::default() };
        assert_eq!(wire("agent", &o, "workspace_write").unwrap()["n"], 1);
        let o = TurnOpts { strategy: Strategy::BestOf, pool: vec!["claude".into(), "codex".into(), "agy".into()], ..Default::default() };
        let v = wire("agent", &o, "workspace_write").unwrap();
        assert_eq!((v["n"].as_u64(), v["review"].as_bool(), v.get("attempts")), (Some(3), Some(true), None));
        let o = TurnOpts { strategy: Strategy::BestOf, ..Default::default() };
        assert_eq!(wire("agent", &o, "workspace_write").unwrap()["n"], 2);
        // Until clean never sends attempts.
        let o = TurnOpts { strategy: Strategy::UntilClean, ..Default::default() };
        let v = wire("agent", &o, "workspace_write").unwrap();
        assert_eq!((v["untilClean"].as_bool(), v.get("attempts")), (Some(true), None));
        // A reviewer panel is a review request; a bad one blocks Send.
        let o = TurnOpts { panel: "codex=gpt-5:high".into(), ..Default::default() };
        let v = wire("agent", &o, "workspace_write").unwrap();
        assert_eq!((v["review"].as_bool(), v["reviewerPanel"][0]["effort"].as_str()), (Some(true), Some("high")));
        assert!(wire("agent", &TurnOpts { panel: "bad harness".into(), ..Default::default() }, "workspace_write").is_err());
        // Plan Council clamps members; Ask carries deep scan; agent knobs never leak.
        let o = TurnOpts { council: true, members: 9, deep_scan: true, delegate: true, ..Default::default() };
        let v = wire("plan", &o, "workspace_write").unwrap();
        assert_eq!((v["council"].as_bool(), v["n"].as_u64()), (Some(true), Some(4)));
        assert!(v.get("deepScan").is_none() && v.get("delegate").is_none() && v.get("review").is_none());
        assert_eq!(wire("ask", &o, "readonly").unwrap()["deepScan"], true);
        // Budget: "$2.5" is a finite cap, junk refuses.
        let o = TurnOpts { budget: "$2.5".into(), web: Some("live"), ..Default::default() };
        let v = wire("ask", &o, "readonly").unwrap();
        assert_eq!((v["paidBudget"]["maxUsd"].as_f64(), v["web"].as_str()), (Some(2.5), Some("live")));
        assert!(wire("ask", &TurnOpts { budget: "-1".into(), ..Default::default() }, "readonly").is_err());
    }

    #[test]
    fn run_detail_depth_decodes() {
        let d: RunDetail = serde_json::from_str(include_str!("../tests/fixtures/live/run.run-7ef7c842eb39.json")).unwrap();
        assert_eq!(d.outcome_banner.as_deref(), Some("Done · not reviewed"));
        let b = d.budget.unwrap();
        assert_eq!((b.cash_knowledge.as_deref(), b.valuation_usd), (Some("exact"), Some(0.09783)));
        assert_eq!(d.timeline[0].kind, "run.created");
        let s = d.summary;
        assert_eq!((s.requested_access.as_deref(), s.effective_access.as_deref()), (Some("readonly"), Some("readonly")));
        assert_eq!(s.auth_route.unwrap().effective.as_deref(), Some("local_session"));
        assert_eq!(s.web_evidence.unwrap().effective_mode.as_deref(), Some("auto"));
        assert!(!s.delegation.unwrap().requested);
        let turn = |f: &str| serde_json::from_value::<Turn>(serde_json::from_str::<Value>(f).unwrap()["value"].clone()).unwrap();
        let c = turn(include_str!("../tests/fixtures/thread-turn-continuity-packet.json")).continuity.unwrap();
        assert_eq!((c.kind.as_str(), c.packet_turns, c.lane_switched_from.and_then(|l| l.harness)), ("packet", 3, Some("codex".into())));
    }

    #[test]
    fn harness_capabilities_decode() {
        let list: HarnessList = serde_json::from_str(include_str!("../tests/fixtures/live/harnesses.json")).unwrap();
        let claude = list.harnesses.iter().find(|h| h.id == "claude").expect("claude row survives lossy decode");
        assert!(claude.can_delegate() && claude.browser_tool());
        assert!(!list.harnesses.iter().find(|h| h.id == "codex").unwrap().can_delegate());
    }

}
