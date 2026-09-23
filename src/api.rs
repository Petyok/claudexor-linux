//! Daemon client: discovery, token, handshake, typed calls (ureq, blocking).
//!
//! Every call here blocks; callers run them on worker threads (state.rs), never
//! on the UI thread. Wire rules from claudexor ARCHITECTURE §7 / INTEGRATIONS:
//! bearer token on every route but /healthz, `X-Claudexor-Protocol-Major: 3`
//! on every product route (missing → typed 426), `Idempotency-Key` on every
//! POST (turn create and retry REQUIRE it).

use crate::model::*;
use crate::sse;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::ffi::CString;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::os::fd::FromRawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const PROTOCOL_MAJOR: i64 = 3;
const CLIENT_NAME: &str = "claudexor-linux";

// ---- errors ------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ApiError {
    /// Socket-level failure: daemon down, connection refused/reset.
    Transport(String),
    /// Non-2xx with the typed `ControlProblem` body when the daemon sent one.
    Http {
        status: u16,
        problem: Option<Problem>,
        body: String,
    },
    Decode(String),
}

impl ApiError {
    pub fn is_transport(&self) -> bool {
        matches!(self, ApiError::Transport(_))
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Transport(m) => write!(f, "engine unreachable: {m}"),
            ApiError::Decode(m) => write!(f, "unexpected response: {m}"),
            ApiError::Http { status, problem: Some(p), body } => {
                let msg = p.message.as_deref().or(p.error.as_deref()).unwrap_or(body);
                match &p.code {
                    Some(code) => write!(f, "{msg} ({code}, HTTP {status})"),
                    None => write!(f, "{msg} (HTTP {status})"),
                }
            }
            ApiError::Http { status, body, .. } => {
                let body: String = body.chars().take(200).collect();
                write!(f, "HTTP {status}: {body}")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum ConnectError {
    /// No discovery file: the daemon has never started or is stopped.
    NotRunning,
    /// Discovery/token file failed the owner-only checks; refuse to use it.
    Insecure(String),
    Invalid(String),
    Unreachable(String),
    /// Reached the daemon but it speaks another protocol major.
    Incompatible(String),
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectError::NotRunning => write!(f, "The Claudexor engine is not running. Start it with `claudexor daemon start`."),
            ConnectError::Insecure(m) => write!(f, "Refusing insecure engine metadata: {m}"),
            ConnectError::Invalid(m) => write!(f, "Engine discovery is invalid: {m}"),
            ConnectError::Unreachable(m) => write!(f, "Engine unreachable: {m}"),
            ConnectError::Incompatible(m) => write!(f, "Engine is incompatible with this app: {m}"),
        }
    }
}

// ---- discovery ------------------------------------------------------------------

/// `$CLAUDEXOR_CONFIG_DIR/daemon/control-api.json`, else `~/.claudexor/v3/daemon/control-api.json`.
pub fn discovery_path() -> PathBuf {
    match std::env::var("CLAUDEXOR_CONFIG_DIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir).join("daemon/control-api.json"),
        _ => dirs::home_dir().unwrap_or_default().join(".claudexor/v3/daemon/control-api.json"),
    }
}

/// Read a daemon-owned private file the way ClaudexorKit's `SecureLocalFile`
/// does: the parent must be a 0700 directory we own, the leaf a 0600 regular
/// file we own with one link, opened no-follow through the parent's fd so a
/// swapped symlink can never redirect the read. Ok(None) = does not exist.
pub fn read_private(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let dir = path.parent().ok_or("path has no parent")?;
    let leaf = path.file_name().and_then(|l| l.to_str()).ok_or("bad file name")?;
    if leaf.is_empty() || leaf == "." || leaf == ".." {
        return Err(format!("bad file name {leaf:?}"));
    }
    let cdir = CString::new(dir.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
    let cleaf = CString::new(leaf).map_err(|e| e.to_string())?;
    let euid = unsafe { libc::geteuid() };
    // SAFETY: plain POSIX calls on owned C strings; every fd is closed via File.
    unsafe {
        let dfd = libc::open(cdir.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
        if dfd < 0 {
            return if *libc::__errno_location() == libc::ENOENT { Ok(None) } else { Err(format!("cannot open {}", dir.display())) };
        }
        let dfile = File::from_raw_fd(dfd);
        let mut st: libc::stat = std::mem::zeroed();
        if libc::fstat(dfd, &mut st) != 0 || st.st_mode & libc::S_IFMT != libc::S_IFDIR || st.st_uid != euid || st.st_mode & 0o777 != 0o700
        {
            return Err(format!("{} is not an owner-only (0700) directory", dir.display()));
        }
        let fd = libc::openat(dfd, cleaf.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC);
        drop(dfile);
        if fd < 0 {
            return if *libc::__errno_location() == libc::ENOENT { Ok(None) } else { Err(format!("cannot open {}", path.display())) };
        }
        let mut file = File::from_raw_fd(fd);
        if libc::fstat(fd, &mut st) != 0
            || st.st_mode & libc::S_IFMT != libc::S_IFREG
            || st.st_uid != euid
            || st.st_nlink != 1
            || st.st_mode & 0o777 != 0o600
        {
            return Err(format!("{} is not an owner-only (0600) regular file", path.display()));
        }
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        Ok(Some(buf))
    }
}

pub fn discover() -> Result<(Discovery, String), ConnectError> {
    let path = discovery_path();
    let raw = read_private(&path).map_err(ConnectError::Insecure)?.ok_or(ConnectError::NotRunning)?;
    let d: Discovery = serde_json::from_slice(&raw).map_err(|e| ConnectError::Invalid(e.to_string()))?;
    // Loopback only: the token must never leave the machine.
    if !matches!(d.host.as_str(), "127.0.0.1" | "::1" | "localhost") {
        return Err(ConnectError::Invalid(format!("non-loopback host {:?}", d.host)));
    }
    let token = read_private(Path::new(&d.token_path))
        .map_err(ConnectError::Insecure)?
        .ok_or_else(|| ConnectError::Invalid("token file is missing".into()))?;
    let token = String::from_utf8(token).map_err(|_| ConnectError::Invalid("token is not UTF-8".into()))?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(ConnectError::Invalid("token file is empty".into()));
    }
    Ok((d, token))
}

// ---- client ------------------------------------------------------------------------

#[derive(Clone)]
pub struct Client {
    base: String,
    bearer: String,
    agent: ureq::Agent,
    stream_agent: ureq::Agent,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // never print the bearer token
        f.debug_struct("Client").field("base", &self.base).finish_non_exhaustive()
    }
}

/// Percent-encode one path segment (ids are server-minted, but never trust that).
pub fn seg(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 128-bit random hex for `Idempotency-Key`.
fn idempotency_key() -> String {
    let mut b = [0u8; 16];
    if File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_err() {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        b[..16].copy_from_slice(&t.as_nanos().to_le_bytes());
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

type Resp = Result<ureq::http::Response<ureq::Body>, ureq::Error>;

impl Client {
    pub fn new(d: &Discovery, token: &str) -> Self {
        let host = if d.host.contains(':') { format!("[{}]", d.host) } else { d.host.clone() };
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(3)))
            .timeout_global(Some(Duration::from_secs(90)))
            .build()
            .new_agent();
        let stream_agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(3)))
            .timeout_global(None)
            .build()
            .new_agent();
        Client { base: format!("http://{host}:{}", d.port), bearer: format!("Bearer {token}"), agent, stream_agent }
    }

    /// Discover, read the token, and negotiate the protocol major.
    pub fn connect() -> Result<(Client, Handshake), ConnectError> {
        let (d, token) = discover()?;
        let client = Client::new(&d, &token);
        let h = client.handshake().map_err(|e| match e {
            ApiError::Transport(m) => ConnectError::Unreachable(m),
            ApiError::Http { status: 426, .. } => ConnectError::Incompatible(e.to_string()),
            other => ConnectError::Unreachable(other.to_string()),
        })?;
        if h.protocol_major != PROTOCOL_MAJOR || !h.compatible {
            return Err(ConnectError::Incompatible(format!(
                "engine speaks protocol {} (this app speaks {PROTOCOL_MAJOR})",
                h.protocol_major
            )));
        }
        Ok((client, h))
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v2/{}", self.base, path.trim_start_matches('/'))
    }

    fn finish<T: DeserializeOwned>(resp: Resp) -> Result<(u16, T), ApiError> {
        let mut resp = resp.map_err(|e| ApiError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().map_err(|e| ApiError::Transport(e.to_string()))?;
        if !(200..300).contains(&status) {
            let problem = serde_json::from_str::<Problem>(&body).ok();
            return Err(ApiError::Http { status, problem, body });
        }
        let v = serde_json::from_str(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        Ok((status, v))
    }

    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let r = self.agent.get(self.url(path)).header("Authorization", &self.bearer).header("X-Claudexor-Protocol-Major", "3").call();
        Self::finish(r).map(|(_, v)| v)
    }

    pub fn post<T: DeserializeOwned>(&self, path: &str, body: impl Serialize) -> Result<(u16, T), ApiError> {
        let r = self
            .agent
            .post(self.url(path))
            .header("Authorization", &self.bearer)
            .header("X-Claudexor-Protocol-Major", "3")
            .header("Idempotency-Key", idempotency_key())
            .send_json(body);
        Self::finish(r)
    }

    fn post_empty<T: DeserializeOwned>(&self, path: &str) -> Result<(u16, T), ApiError> {
        let r = self
            .agent
            .post(self.url(path))
            .header("Authorization", &self.bearer)
            .header("X-Claudexor-Protocol-Major", "3")
            .header("Idempotency-Key", idempotency_key())
            .send_empty();
        Self::finish(r)
    }

    pub fn handshake(&self) -> Result<Handshake, ApiError> {
        self.post("handshake", json!({"protocolMajor": PROTOCOL_MAJOR, "client": CLIENT_NAME})).map(|(_, h)| h)
    }

    pub fn threads(&self) -> Result<ThreadList, ApiError> {
        self.get("threads")
    }

    pub fn thread(&self, id: &str) -> Result<ThreadDetail, ApiError> {
        self.get(&format!("threads/{}", seg(id)))
    }

    pub fn create_thread(&self, req: &CreateThread) -> Result<Thread, ApiError> {
        self.post("threads", req).map(|(_, t)| t)
    }

    pub fn send_turn(&self, thread_id: &str, req: &TurnRequest) -> Result<TurnStarted, ApiError> {
        self.post(&format!("threads/{}/turns", seg(thread_id)), req).map(|(_, t)| t)
    }

    /// Exact Retry of a refused turn (same turn, recorded params, fresh preflight).
    pub fn retry_turn(&self, thread_id: &str, turn_id: &str) -> Result<TurnStarted, ApiError> {
        self.post_empty(&format!("threads/{}/turns/{}/retry", seg(thread_id), seg(turn_id))).map(|(_, t)| t)
    }

    pub fn cancel(&self, run_id: &str) -> Result<(), ApiError> {
        self.post::<Value>(
            &format!("runs/{}/control", seg(run_id)),
            json!({"control": {"kind": "cancel", "reason_code": "user_cancelled"}}),
        )
        .map(drop)
    }

    pub fn run(&self, run_id: &str) -> Result<RunDetail, ApiError> {
        self.get(&format!("runs/{}", seg(run_id)))
    }

    /// Raw artifact text (fallback when `primaryOutput.text` is absent).
    pub fn artifact_text(&self, run_id: &str, path: &str) -> Result<String, ApiError> {
        let path: Vec<String> = path.split('/').map(seg).collect();
        let r = self
            .agent
            .get(self.url(&format!("runs/{}/artifacts/{}", seg(run_id), path.join("/"))))
            .header("Authorization", &self.bearer)
            .header("X-Claudexor-Protocol-Major", "3")
            .call();
        let mut resp = r.map_err(|e| ApiError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().map_err(|e| ApiError::Decode(e.to_string()))?;
        if status != 200 {
            return Err(ApiError::Http { status, problem: serde_json::from_str(&body).ok(), body });
        }
        Ok(body)
    }

    pub fn answer(&self, run_id: &str, interaction_id: &str, answers: &[Answer]) -> Result<(), ApiError> {
        self.post::<Value>(&format!("runs/{}/interactions/{}/answer", seg(run_id), seg(interaction_id)), json!({ "answers": answers }))
            .map(drop)
    }

    pub fn harnesses(&self) -> Result<HarnessList, ApiError> {
        self.get("harnesses")
    }

    pub fn models(&self, harness: &str) -> Result<ModelList, ApiError> {
        self.get(&format!("harnesses/{}/models", seg(harness)))
    }

    pub fn quota(&self) -> Result<Quota, ApiError> {
        self.get("quota")
    }

    pub fn refresh_quota(&self) -> Result<Quota, ApiError> {
        self.post("quota", json!({})).map(|(_, q)| q)
    }

    pub fn account_pools(&self) -> Result<AccountPools, ApiError> {
        self.get("account-pools")
    }

    /// Start (or rejoin: a duplicate create returns the active job) a native
    /// login — for the harness's bootstrap account, or one exact profile.
    pub fn start_login(&self, harness: &str, profile: Option<&str>) -> Result<SetupJob, ApiError> {
        let mut body = json!({"harness": harness, "action": "login", "authRequest": "subscription"});
        if let Some(p) = profile {
            body["profileId"] = json!(p);
        }
        self.post("setup/jobs", body).map(|(_, j)| j)
    }

    // ---- threads ---------------------------------------------------------------------

    fn patch<T: DeserializeOwned>(&self, path: &str, body: impl Serialize) -> Result<T, ApiError> {
        let r = self
            .agent
            .patch(self.url(path))
            .header("Authorization", &self.bearer)
            .header("X-Claudexor-Protocol-Major", "3")
            .send_json(body);
        Self::finish(r).map(|(_, v)| v)
    }

    /// Rename (`{"title"}`) or archive/reopen (`{"state": "closed"|"active"}`).
    pub fn update_thread(&self, id: &str, body: Value) -> Result<Thread, ApiError> {
        self.patch(&format!("threads/{}", seg(id)), body)
    }

    /// `trash` | `restore` | `purge` (purge only from trash; permanent).
    pub fn thread_action(&self, id: &str, action: &str) -> Result<Thread, ApiError> {
        self.post_empty(&format!("threads/{}/{}", seg(id), seg(action))).map(|(_, t)| t)
    }

    // ---- accounts (credential profiles) ------------------------------------------------

    pub fn profiles(&self) -> Result<Profiles, ApiError> {
        self.get("credential-profiles")
    }

    pub fn create_profile(&self, harness: &str, profile_id: &str, display_name: Option<&str>) -> Result<Value, ApiError> {
        let mut body = json!({"harnessId": harness, "profileId": profile_id});
        if let Some(n) = display_name {
            body["displayName"] = json!(n);
        }
        self.post("credential-profiles", body).map(|(_, v)| v)
    }

    pub fn set_profile_enabled(&self, harness: &str, profile_id: &str, enabled: bool) -> Result<Value, ApiError> {
        self.patch(&format!("credential-profiles/{}/{}", seg(harness), seg(profile_id)), json!({ "enabled": enabled }))
    }

    pub fn delete_profile(&self, harness: &str, profile_id: &str) -> Result<Value, ApiError> {
        let r = self
            .agent
            .delete(self.url(&format!("credential-profiles/{}/{}", seg(harness), seg(profile_id))))
            .header("Authorization", &self.bearer)
            .header("X-Claudexor-Protocol-Major", "3")
            .header("Idempotency-Key", idempotency_key())
            .call();
        Self::finish(r).map(|(_, v)| v)
    }

    // ---- attachments -----------------------------------------------------------------

    /// The v2 resource pipeline: create → one PUT of the complete bytes →
    /// finalize. Returns the immutable resource a turn may reference.
    pub fn upload(&self, name: &str, kind: &str, mime: &str, bytes: &[u8]) -> Result<Resource, ApiError> {
        let (_, up): (u16, UploadStatus) =
            self.post("uploads", json!({"kind": kind, "mime": mime, "name": name, "sizeBytes": bytes.len()}))?;
        let r = self
            .agent
            .put(self.url(&format!("uploads/{}/bytes", seg(&up.upload_id))))
            .header("Authorization", &self.bearer)
            .header("X-Claudexor-Protocol-Major", "3")
            .header("Content-Type", "application/octet-stream")
            .send(bytes);
        Self::finish::<Value>(r)?;
        // ponytail: no client-side sha256 (would add a dependency); the daemon hashes
        // and revalidates the bytes at finalize and again at enqueue.
        self.post(&format!("uploads/{}/finalize", seg(&up.upload_id)), json!({})).map(|(_, r)| r)
    }

    /// Job + transient sign-in disclosure (URL / one-time code).
    pub fn login_snapshot(&self, job_id: &str) -> Result<SetupSnapshot, ApiError> {
        self.get(&format!("setup/jobs/{}/snapshot", seg(job_id)))
    }

    /// Deliver the pasted sign-in code. Transient on the daemon side; never logged here.
    pub fn login_input(&self, job_id: &str, value: &str) -> Result<SetupJob, ApiError> {
        self.post(&format!("setup/jobs/{}/input", seg(job_id)), json!({ "value": value })).map(|(_, j)| j)
    }

    /// +15 min on an engine-owned login deadline (refused when `deadlineFixed`).
    pub fn extend_login(&self, job_id: &str) -> Result<SetupJob, ApiError> {
        self.post_empty(&format!("setup/jobs/{}/extend", seg(job_id))).map(|(_, j)| j)
    }

    pub fn cancel_login(&self, job_id: &str) -> Result<SetupJob, ApiError> {
        self.post_empty(&format!("setup/jobs/{}/cancel", seg(job_id))).map(|(_, j)| j)
    }

    /// Trust state of one repo; None when it has no trust file yet.
    pub fn trust(&self, root: &str) -> Result<Option<TrustState>, ApiError> {
        let list: TrustList = self.get(&format!("trust?repoRoot={}", seg(root)))?;
        Ok(list.entries.into_iter().find(|e| e.repo_root.as_deref() == Some(root)))
    }

    /// Grant unsandboxed full access for one repo (recorded in its trust file).
    pub fn grant_full_access(&self, root: &str) -> Result<TrustState, ApiError> {
        self.post("trust", serde_json::json!({"repoRoot": root, "allowFullAccess": true})).map(|(_, t)| t)
    }

    pub fn projects(&self) -> Result<ProjectList, ApiError> {
        self.get("projects")
    }

    /// Open an SSE stream (`runs/:id/events` or `global/events`), resuming after
    /// `last_event_id`. The reader blocks; run it on its own thread.
    pub fn open_stream(&self, path: &str, last_event_id: Option<&str>) -> Result<impl Read + use<>, ApiError> {
        let mut req = self
            .stream_agent
            .get(self.url(path))
            .header("Authorization", &self.bearer)
            .header("X-Claudexor-Protocol-Major", "3")
            .header("Accept", "text/event-stream");
        if let Some(id) = last_event_id {
            req = req.header("Last-Event-ID", id);
        }
        let mut resp = req.call().map_err(|e| ApiError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        if status != 200 {
            let body = resp.body_mut().read_to_string().unwrap_or_default();
            return Err(ApiError::Http { status, problem: serde_json::from_str(&body).ok(), body });
        }
        let ct = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("");
        if !ct.to_ascii_lowercase().starts_with("text/event-stream") {
            return Err(ApiError::Decode(format!("events response is {ct:?}, not text/event-stream")));
        }
        Ok(resp.into_body().into_reader())
    }

    /// Read one stream until it ends; frames go to `on_frame` (false = stop).
    pub fn stream(&self, path: &str, parser: &mut sse::Parser, on_frame: impl FnMut(sse::Frame) -> bool) -> Result<sse::End, ApiError> {
        let last = parser.last_id().map(str::to_owned);
        let reader = self.open_stream(path, last.as_deref())?;
        sse::pump(reader, parser, on_frame).map_err(|e| ApiError::Transport(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cxl-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn private_read_accepts_owner_only_and_rejects_loose_modes() {
        let d = tmpdir("priv");
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
        let f = d.join("token");
        std::fs::write(&f, "secret\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_private(&f).unwrap().as_deref(), Some(&b"secret\n"[..]));

        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_private(&f).is_err(), "world-readable file must be refused");
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();

        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(read_private(&f).is_err(), "loose parent dir must be refused");
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();

        let link = d.join("link");
        std::os::unix::fs::symlink(&f, &link).unwrap();
        assert!(read_private(&link).is_err(), "symlink leaf must be refused");

        assert_eq!(read_private(&d.join("missing")).unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Phase 3 exit criterion at the transport level: a stream cut mid-run
    /// resumes with `Last-Event-ID` and yields every seq exactly once.
    #[test]
    fn sse_resume_after_drop_has_no_gaps_or_duplicates() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut seen_headers = vec![];
            for (i, conn) in listener.incoming().take(2).enumerate() {
                let mut conn = conn.unwrap();
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    head.push_str(&line);
                }
                seen_headers.push(head.to_ascii_lowercase());
                let ev = |seq: u32| format!("id: {seq}\nevent: harness.event\ndata: {{\"seq\":{seq},\"type\":\"harness.event\"}}\n\n");
                let body = if i == 0 {
                    // replay 1..3, then drop the connection without `end`
                    format!(": connected\n\n{}{}{}", ev(1), ev(2), ev(3))
                } else {
                    format!(": connected\n\n{}event: end\ndata: {{}}\n\n", ev(4))
                };
                write!(conn, "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}").unwrap();
            }
            seen_headers
        });
        let d = Discovery { host: "127.0.0.1".into(), port, token_path: String::new() };
        let c = Client::new(&d, "tok");
        let mut parser = sse::Parser::default();
        let mut seqs = vec![];
        let first = c.stream("runs/r1/events", &mut parser, |f| {
            seqs.push(f.id.unwrap());
            true
        });
        assert_eq!(first.unwrap(), sse::End::Lost);
        let second = c.stream("runs/r1/events", &mut parser, |f| {
            seqs.push(f.id.unwrap());
            true
        });
        assert_eq!(second.unwrap(), sse::End::Terminal);
        assert_eq!(seqs, ["1", "2", "3", "4"]);
        let headers = server.join().unwrap();
        assert!(!headers[0].contains("last-event-id"), "first open is a full replay");
        assert!(headers[1].contains("last-event-id: 3"), "resume sends the cursor: {}", headers[1]);
        assert!(headers[1].contains("authorization: bearer tok"));
        assert!(headers[1].contains("x-claudexor-protocol-major: 3"));
    }

    #[test]
    fn segment_encoding() {
        assert_eq!(seg("th-1_a.b~"), "th-1_a.b~");
        assert_eq!(seg("a/b c"), "a%2Fb%20c");
    }

    #[test]
    fn idempotency_keys_are_unique_hex() {
        let a = idempotency_key();
        assert_eq!(a.len(), 32);
        assert_ne!(a, idempotency_key());
    }
}
