//! Pure fold of run SSE events into transcript blocks (no UI, no I/O).
//!
//! A port of ClaudexorKit's `TranscriptReducer` so the Linux and macOS clients
//! show the same activity for the same stream:
//! - idempotent by monotonic `seq` (a replay after reconnect never duplicates);
//! - only `harness.event` contributes; the harness payload `type` drives it;
//! - consecutive `thinking` merges into one segment;
//! - message `delta`s grow one streaming block, a complete message replaces it
//!   by id, a typed `final` seals the stream and removes its narration twin
//!   (the final text IS the answer bubble, never an Activity row);
//! - `tool_result` updates its `tool_call` by `use_id` (fallback: last open tool
//!   of the same name), or stands alone after a mid-tool reconnect;
//! - hard caps on block count, per-block chars and tool fields, always disclosed.

use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Ok,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub name: String,
    pub kind: Option<String>,
    pub target: Option<String>,
    pub status: ToolStatus,
    pub detail: Option<String>,
    pub exit_code: Option<i64>,
    /// Agent/Task tool: the subagent's description, and "type · model".
    pub note: Option<String>,
    pub sub: Option<String>,
    /// A background launch: its result only means "started", never "done".
    pub background: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Thinking { id: String, text: String, secs: Option<i64> },
    Message { id: String, text: String },
    Tool { id: String, tool: Tool },
}

impl Block {
    pub fn id(&self) -> &str {
        match self {
            Block::Thinking { id, .. } | Block::Message { id, .. } | Block::Tool { id, .. } => id,
        }
    }
    fn chars(&self) -> usize {
        match self {
            Block::Thinking { text, .. } | Block::Message { text, .. } => text.chars().count(),
            Block::Tool { tool, .. } => {
                tool.name.chars().count()
                    + tool.kind.as_ref().map_or(0, |s| s.chars().count())
                    + tool.target.as_ref().map_or(0, |s| s.chars().count())
                    + tool.detail.as_ref().map_or(0, |s| s.chars().count())
            }
        }
    }
}

pub struct Transcript {
    pub blocks: Vec<Block>,
    /// Oldest whole blocks evicted to stay under the caps ("N earlier…").
    pub trimmed: usize,
    /// Characters cut from overlong single blocks — disclosed, never silent.
    pub truncated_chars: usize,
    /// Highest seq folded; the SSE resume cursor.
    pub last_seq: i64,
    /// The harness streaming this run (every harness event carries it).
    pub harness: Option<String>,
    /// Background subagents launched, and how many have finished: each
    /// finish resumes the parent session with a fresh `started` event.
    pub agents: usize,
    pub agents_done: usize,
    /// The typed final message: the answer when the run wrote no answer file.
    pub final_text: Option<String>,
    thinking_from: Option<i64>,
    text_chars: usize,
    tool_by_use_id: HashMap<String, usize>,
    open_tool_by_name: HashMap<String, usize>,
    streaming_msg: Option<String>,
    pending_flush: Option<String>,
    msg_candidate: Option<String>,
    finalized: bool,
    cap: usize,
    block_cap: usize,
    total_budget: usize,
    tool_field_cap: usize,
}

impl Default for Transcript {
    fn default() -> Self {
        Self::with_caps(200, 100_000, 600_000, 2_048)
    }
}

fn clip(text: &str, cap: usize, keep_tail: bool) -> String {
    let n = text.chars().count();
    if n <= cap {
        return text.to_string();
    }
    if keep_tail { text.chars().skip(n - cap).collect() } else { text.chars().take(cap).collect() }
}

fn s(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_owned)
}

impl Transcript {
    pub fn with_caps(cap: usize, block_cap: usize, total_budget: usize, tool_field_cap: usize) -> Self {
        Transcript {
            blocks: vec![],
            trimmed: 0,
            truncated_chars: 0,
            last_seq: 0,
            harness: None,
            agents: 0,
            agents_done: 0,
            final_text: None,
            thinking_from: None,
            text_chars: 0,
            tool_by_use_id: HashMap::new(),
            open_tool_by_name: HashMap::new(),
            streaming_msg: None,
            pending_flush: None,
            msg_candidate: None,
            finalized: false,
            cap,
            block_cap,
            total_budget,
            tool_field_cap,
        }
    }

    pub fn tool_count(&self) -> usize {
        self.blocks.iter().filter(|b| matches!(b, Block::Tool { .. })).count()
    }

    fn bound(&mut self, text: &str, cap: usize, keep_tail: bool) -> String {
        let c = clip(text, cap, keep_tail);
        self.truncated_chars += text.chars().count() - c.chars().count();
        c
    }

    /// Fold one event. Returns true when the visible transcript changed.
    pub fn apply(&mut self, seq: i64, event: &Value) -> bool {
        if seq <= self.last_seq {
            return false; // already folded (replay)
        }
        self.last_seq = seq;
        if event.get("type").and_then(Value::as_str) != Some("harness.event") {
            return false;
        }
        let Some(p) = event.get("payload") else { return false };
        let key = seq.to_string();
        if self.harness.is_none() {
            self.harness = s(p.get("harness_id"));
        }
        let ts = p.get("ts").and_then(Value::as_str).and_then(crate::ui::theme::parse_iso);
        match p.get("type").and_then(Value::as_str) {
            Some("started") => {
                if self.agents > self.agents_done {
                    self.agents_done += 1;
                    return true;
                }
                false
            }
            Some("thinking") => {
                let Some(text) = p.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()) else {
                    return false;
                };
                if let Some(Block::Thinking { text: prev, .. }) = self.blocks.last() {
                    let merged = format!("{prev}\n{text}");
                    let prev_n = prev.chars().count();
                    let bounded = self.bound(&merged, self.block_cap, true);
                    self.text_chars = self.text_chars + bounded.chars().count() - prev_n;
                    let secs = self.thinking_from.zip(ts).map(|(a, b)| (b - a).max(0));
                    if let Some(Block::Thinking { text: t, secs: sc, .. }) = self.blocks.last_mut() {
                        *t = bounded;
                        if secs.is_some() {
                            *sc = secs;
                        }
                    }
                    self.enforce_budget();
                } else {
                    self.thinking_from = ts;
                    let t = self.bound(text, self.block_cap, true);
                    self.append(Block::Thinking { id: format!("th-{key}"), text: t, secs: Some(0).filter(|_| ts.is_some()) });
                }
                true
            }
            Some("message") => {
                if p.get("final").and_then(Value::as_bool) == Some(true) {
                    self.streaming_msg = None;
                    self.pending_flush = None;
                    self.finalized = true;
                    if let Some(final_text) = p.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                        self.final_text = Some(clip(final_text, self.block_cap, false));
                        let cand = self.msg_candidate.clone();
                        self.remove_twin(cand.as_deref(), final_text);
                    }
                    self.msg_candidate = None;
                    return true;
                }
                let Some(text) = p.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()) else {
                    return false;
                };
                // adapters nest the flag (`payload: {delta: true}`); older shapes had it flat
                let delta = p.get("delta").or_else(|| p.get("payload").and_then(|x| x.get("delta"))).and_then(Value::as_bool) == Some(true);
                if delta {
                    if self.finalized {
                        return false;
                    }
                    let continues = match (&self.streaming_msg, self.blocks.last()) {
                        (Some(sid), Some(Block::Message { id, .. })) => sid == id,
                        _ => false,
                    };
                    if continues {
                        let prev = match self.blocks.last() {
                            Some(Block::Message { text, .. }) => text.clone(),
                            _ => String::new(),
                        };
                        let bounded = self.bound(&(prev.clone() + text), self.block_cap, false);
                        self.text_chars = self.text_chars + bounded.chars().count() - prev.chars().count();
                        if let Some(Block::Message { text: t, .. }) = self.blocks.last_mut() {
                            *t = bounded;
                        }
                        self.enforce_budget();
                    } else {
                        let id = format!("msg-{key}");
                        let t = self.bound(text, self.block_cap, false);
                        self.append(Block::Message { id: id.clone(), text: t });
                        self.streaming_msg = Some(id);
                    }
                    self.pending_flush = self.streaming_msg.clone();
                    self.msg_candidate = self.streaming_msg.clone();
                    return true;
                }
                // A complete message replaces its delta-built block by id, even
                // after an intervening tool row orphaned it from the tail.
                if let Some(pid) = self.pending_flush.clone() {
                    if let Some(i) = self.blocks.iter().rposition(|b| b.id() == pid) {
                        if let Block::Message { text: prev, .. } = &self.blocks[i] {
                            let prev_n = prev.chars().count();
                            let bounded = self.bound(text, self.block_cap, false);
                            self.text_chars = self.text_chars + bounded.chars().count() - prev_n;
                            self.blocks[i] = Block::Message { id: pid.clone(), text: bounded };
                            self.pending_flush = None;
                            self.streaming_msg = None;
                            self.msg_candidate = Some(pid);
                            self.enforce_budget();
                            return true;
                        }
                    }
                }
                let id = format!("msg-{key}");
                let t = self.bound(text, self.block_cap, false);
                self.append(Block::Message { id: id.clone(), text: t });
                self.msg_candidate = Some(id);
                true
            }
            Some("tool_call") => {
                let tool = p.get("tool");
                let f = |k: &str| s(tool.and_then(|t| t.get(k)));
                let block = self.bounded_tool(
                    f("name").unwrap_or_else(|| "tool".into()),
                    f("kind"),
                    f("target"),
                    ToolStatus::Running,
                    f("content_summary"),
                    tool.and_then(|t| t.get("exit_code")).and_then(Value::as_f64).map(|x| x as i64),
                );
                let mut block = block;
                if matches!(block.name.as_str(), "Agent" | "Task") {
                    let input = p.get("payload").and_then(|x| x.get("input"));
                    let g = |k: &str| s(input.and_then(|i| i.get(k)));
                    block.note = g("description");
                    let sub: Vec<String> = [g("subagent_type"), g("model")].into_iter().flatten().collect();
                    block.sub = Some(sub.join(" · ")).filter(|x| !x.is_empty());
                }
                let name = block.name.clone();
                self.append(Block::Tool { id: format!("tool-{key}"), tool: block });
                let idx = self.blocks.len() - 1;
                if let Some(u) = f("use_id") {
                    self.tool_by_use_id.insert(u, idx);
                }
                self.open_tool_by_name.insert(name, idx);
                true
            }
            Some("tool_result") => {
                let tool = p.get("tool");
                let f = |k: &str| s(tool.and_then(|t| t.get(k)));
                let use_id = f("use_id");
                let name = f("name");
                let idx = use_id
                    .as_ref()
                    .and_then(|u| self.tool_by_use_id.get(u))
                    .or_else(|| name.as_ref().and_then(|n| self.open_tool_by_name.get(n)))
                    .copied();
                let status = if f("status").as_deref() == Some("error") { ToolStatus::Error } else { ToolStatus::Ok };
                let detail = f("error_summary").or_else(|| f("content_summary"));
                let exit = tool.and_then(|t| t.get("exit_code")).and_then(Value::as_f64).map(|x| x as i64);
                if let Some(i) = idx.filter(|&i| i < self.blocks.len()) {
                    if let Block::Tool { .. } = &self.blocks[i] {
                        let bounded = detail.map(|d| self.bound(&d, self.tool_field_cap, false));
                        let launched = detail_launch(tool);
                        if launched {
                            self.agents += 1;
                        }
                        if let Block::Tool { tool: b, .. } = &mut self.blocks[i] {
                            b.status = status;
                            b.background = launched;
                            if let Some(d) = bounded {
                                let old = b.detail.as_ref().map_or(0, |x| x.chars().count());
                                self.text_chars = self.text_chars + d.chars().count() - old;
                                b.detail = Some(d);
                            }
                            if exit.is_some() {
                                b.exit_code = exit;
                            }
                        }
                        if let Some(u) = &use_id {
                            self.tool_by_use_id.remove(u);
                        }
                        if let Some(n) = &name {
                            self.open_tool_by_name.remove(n);
                        }
                        self.enforce_budget();
                        return true;
                    }
                }
                let kind = f("kind");
                let target = f("target");
                let block = self.bounded_tool(name.unwrap_or_else(|| "tool".into()), kind, target, status, detail, exit);
                self.append(Block::Tool { id: format!("tool-{key}"), tool: block });
                true
            }
            _ => false,
        }
    }

    /// The newest thing the run is doing, for a collapsed live card.
    pub fn last_activity(&self) -> Option<String> {
        self.blocks.iter().rev().find_map(|b| match b {
            Block::Tool { tool, .. } => Some(tool.note.clone().unwrap_or_else(|| tool.name.clone())),
            Block::Thinking { .. } => Some("Thinking".into()),
            Block::Message { .. } => None,
        })
    }

    fn bounded_tool(
        &mut self,
        name: String,
        kind: Option<String>,
        target: Option<String>,
        status: ToolStatus,
        detail: Option<String>,
        exit_code: Option<i64>,
    ) -> Tool {
        let c = self.tool_field_cap;
        Tool {
            name: self.bound(&name, c, false),
            kind: kind.map(|k| self.bound(&k, c, false)),
            target: target.map(|t| self.bound(&t, c, false)),
            status,
            detail: detail.map(|d| self.bound(&d, c, false)),
            exit_code,
            note: None,
            sub: None,
            background: false,
        }
    }

    fn append(&mut self, block: Block) {
        if !matches!(block, Block::Message { .. }) {
            self.streaming_msg = None;
        }
        self.text_chars += block.chars();
        self.blocks.push(block);
        self.enforce_budget();
    }

    fn enforce_budget(&mut self) {
        let mut drop = 0;
        let mut remaining = self.text_chars;
        while self.blocks.len() - drop > self.cap || (remaining > self.total_budget && self.blocks.len() - drop > 1) {
            remaining -= self.blocks[drop].chars();
            drop += 1;
        }
        if drop == 0 {
            return;
        }
        self.blocks.drain(..drop);
        self.text_chars = remaining;
        self.trimmed += drop;
        let shift = |m: &mut HashMap<String, usize>| {
            m.retain(|_, i| *i >= drop);
            m.values_mut().for_each(|i| *i -= drop);
        };
        shift(&mut self.tool_by_use_id);
        shift(&mut self.open_tool_by_name);
    }

    /// Drop the narration twin of a typed final — only when its text IS the
    /// (head-bounded) final text, so distinct narration survives.
    fn remove_twin(&mut self, id: Option<&str>, final_text: &str) {
        let Some(id) = id else { return };
        let Some(i) = self.blocks.iter().rposition(|b| b.id() == id) else { return };
        let Block::Message { text, .. } = &self.blocks[i] else { return };
        if *text != clip(final_text, self.block_cap, false) {
            return;
        }
        self.text_chars -= text.chars().count();
        self.blocks.remove(i);
        let shift = |m: &mut HashMap<String, usize>| {
            m.values_mut().for_each(|x| {
                if *x > i {
                    *x -= 1
                }
            })
        };
        shift(&mut self.tool_by_use_id);
        shift(&mut self.open_tool_by_name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn h(payload: Value) -> Value {
        json!({"type": "harness.event", "payload": payload})
    }

    #[test]
    fn replay_is_idempotent_by_seq() {
        let mut t = Transcript::default();
        assert!(t.apply(1, &h(json!({"type": "message", "text": "hi"}))));
        assert!(!t.apply(1, &h(json!({"type": "message", "text": "hi"}))));
        assert!(!t.apply(0, &h(json!({"type": "message", "text": "old"}))));
        assert_eq!(t.blocks.len(), 1);
        assert_eq!(t.last_seq, 1);
    }

    #[test]
    fn non_harness_events_advance_cursor_only() {
        let mut t = Transcript::default();
        assert!(!t.apply(5, &json!({"type": "run.created", "payload": {}})));
        assert_eq!(t.last_seq, 5);
        assert!(t.blocks.is_empty());
    }

    #[test]
    fn thinking_merges_until_another_block() {
        let mut t = Transcript::default();
        t.apply(1, &h(json!({"type": "thinking", "text": "a"})));
        t.apply(2, &h(json!({"type": "thinking", "text": "b"})));
        t.apply(3, &h(json!({"type": "tool_call", "tool": {"name": "Read"}})));
        t.apply(4, &h(json!({"type": "thinking", "text": "c"})));
        assert_eq!(t.blocks.len(), 3);
        assert_eq!(t.blocks[0], Block::Thinking { id: "th-1".into(), text: "a\nb".into(), secs: None });
    }

    #[test]
    fn nested_delta_flag_streams_one_block() {
        // the live claude adapter shape: the flag sits in the harness event's own payload
        let mut t = Transcript::default();
        for (i, chunk) in ["I", "'", "m comparing the Lin", "ux eg"].iter().enumerate() {
            t.apply(i as i64 + 1, &h(json!({"type": "message", "text": chunk, "payload": {"delta": true}})));
        }
        t.apply(9, &h(json!({"type": "message", "text": "I'm comparing the Linux eg"})));
        let msgs: Vec<_> = t.blocks.iter().filter_map(|b| match b { Block::Message { text, .. } => Some(text.as_str()), _ => None }).collect();
        assert_eq!(msgs, vec!["I'm comparing the Linux eg"]);
    }

    #[test]
    fn deltas_stream_then_complete_message_replaces_by_id() {
        let mut t = Transcript::default();
        t.apply(1, &h(json!({"type": "message", "text": "Hel", "delta": true})));
        t.apply(2, &h(json!({"type": "message", "text": "lo", "delta": true})));
        assert_eq!(t.blocks[0], Block::Message { id: "msg-1".into(), text: "Hello".into() });
        t.apply(3, &h(json!({"type": "tool_call", "tool": {"name": "Bash", "use_id": "u1"}})));
        t.apply(4, &h(json!({"type": "message", "text": "Hello world"})));
        assert_eq!(t.blocks.len(), 2, "complete flush reconciles, no duplicate paragraph");
        assert_eq!(t.blocks[0], Block::Message { id: "msg-1".into(), text: "Hello world".into() });
    }

    #[test]
    fn final_seals_and_removes_only_its_exact_twin() {
        let mut t = Transcript::default();
        t.apply(1, &h(json!({"type": "message", "text": "Looking…"})));
        t.apply(2, &h(json!({"type": "message", "text": "The answer"})));
        t.apply(3, &h(json!({"type": "message", "text": "The answer", "final": true})));
        assert_eq!(t.blocks, vec![Block::Message { id: "msg-1".into(), text: "Looking…".into() }]);
        // sealed: late deltas are dropped
        assert!(!t.apply(4, &h(json!({"type": "message", "text": "late", "delta": true}))));
    }

    #[test]
    fn tool_result_pairs_by_use_id_then_name_then_standalone() {
        let mut t = Transcript::default();
        t.apply(1, &h(json!({"type": "tool_call", "tool": {"name": "Bash", "use_id": "a", "target": "ls"}})));
        t.apply(2, &h(json!({"type": "tool_call", "tool": {"name": "Read"}})));
        t.apply(3, &h(json!({"type": "tool_result", "tool": {"name": "Bash", "use_id": "a", "status": "error", "error_summary": "boom", "exit_code": 2}})));
        t.apply(4, &h(json!({"type": "tool_result", "tool": {"name": "Read", "content_summary": "12 lines"}})));
        t.apply(5, &h(json!({"type": "tool_result", "tool": {"name": "Grep", "use_id": "zz"}})));
        assert_eq!(t.blocks.len(), 3);
        let Block::Tool { tool: bash, .. } = &t.blocks[0] else { panic!() };
        assert_eq!((bash.status, bash.detail.as_deref(), bash.exit_code), (ToolStatus::Error, Some("boom"), Some(2)));
        let Block::Tool { tool: read, .. } = &t.blocks[1] else { panic!() };
        assert_eq!(read.status, ToolStatus::Ok);
        let Block::Tool { tool: grep, .. } = &t.blocks[2] else { panic!() };
        assert_eq!(grep.name, "Grep", "orphan result stands alone");
    }

    #[test]
    fn caps_evict_oldest_and_keep_indices_valid() {
        let mut t = Transcript::with_caps(3, 1000, 10_000, 50);
        t.apply(1, &h(json!({"type": "tool_call", "tool": {"name": "Bash", "use_id": "keep"}})));
        for i in 2..=5 {
            t.apply(i, &h(json!({"type": "tool_call", "tool": {"name": format!("T{i}")}})));
        }
        assert_eq!(t.blocks.len(), 3);
        assert_eq!(t.trimmed, 2);
        // the evicted call's result must not corrupt a surviving row
        t.apply(6, &h(json!({"type": "tool_result", "tool": {"name": "T5", "status": "ok"}})));
        let Block::Tool { tool, .. } = &t.blocks[2] else { panic!() };
        assert_eq!((tool.name.as_str(), tool.status), ("T5", ToolStatus::Ok));
    }

    #[test]
    fn overlong_fields_are_clipped_and_disclosed() {
        let mut t = Transcript::with_caps(10, 5, 1000, 4);
        t.apply(1, &h(json!({"type": "message", "text": "abcdefgh"})));
        t.apply(2, &h(json!({"type": "tool_call", "tool": {"name": "LongName"}})));
        assert_eq!(t.blocks[0], Block::Message { id: "msg-1".into(), text: "abcde".into() });
        let Block::Tool { tool, .. } = &t.blocks[1] else { panic!() };
        assert_eq!(tool.name, "Long");
        assert_eq!(t.truncated_chars, 3 + 4);
    }
}

#[cfg(test)]
mod replay {
    /// `CXL_REPLAY=<events.jsonl> cargo test replay_log -- --nocapture`: fold a real run log.
    #[test]
    fn replay_log() {
        let Ok(path) = std::env::var("CXL_REPLAY") else { return };
        let mut t = super::Transcript::default();
        for line in std::fs::read_to_string(path).unwrap().lines() {
            let ev: serde_json::Value = serde_json::from_str(line).unwrap();
            t.apply(ev["seq"].as_i64().unwrap_or(0), &ev);
        }
        for b in &t.blocks {
            if let super::Block::Message { text, .. } = b {
                println!("MSG {:?}", text.chars().take(70).collect::<String>());
            }
        }
        println!("blocks {}", t.blocks.len());
    }
}

/// An Agent/Task result that only reports a background launch.
fn detail_launch(tool: Option<&Value>) -> bool {
    tool.and_then(|t| t.get("content_summary")).and_then(Value::as_str).is_some_and(|x| x.starts_with("Async agent launched"))
}
