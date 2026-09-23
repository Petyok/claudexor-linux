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
}

impl Harness {
    pub fn label(&self) -> &str {
        self.manifest.as_ref().and_then(|m| m.display_name.as_deref()).unwrap_or(&self.id)
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
        let r = TurnRequest { prompt: "hi".into(), mode: "ask".into(), primary_harness: None, model: None, effort: None };
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
