//! Byte-level Server-Sent Events parser (WHATWG EventSource field rules).
//!
//! Works on raw bytes so blank delimiter lines are never lost, trims a trailing
//! `\r` (CRLF servers), joins multi-line `data:`, ignores `:` comments /
//! heartbeats, and remembers the last `id:` for `Last-Event-ID` resume.
//! Mirrors ClaudexorKit's `SSEParser` so both clients read the daemon identically.

use std::io::Read;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub id: Option<String>,
    pub event: String,
    pub data: String,
}

#[derive(Default)]
pub struct Parser {
    buf: Vec<u8>,
    data_lines: Vec<String>,
    event: Option<String>,
    frame_id: Option<String>,
    last_id: Option<String>,
}

impl Parser {
    /// A parser that resumes after `id` (sent as `Last-Event-ID` on open).
    pub fn resume(id: Option<String>) -> Self {
        Parser { last_id: id, ..Self::default() }
    }

    /// Last `id:` seen on the stream, for `Last-Event-ID` on reconnect.
    pub fn last_id(&self) -> Option<&str> {
        self.last_id.as_deref()
    }

    /// Feed a chunk; returns every frame the chunk completed.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Frame> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        let mut start = 0;
        while let Some(pos) = self.buf[start..].iter().position(|&b| b == b'\n') {
            let mut end = start + pos;
            if end > start && self.buf[end - 1] == b'\r' {
                end -= 1;
            }
            let line = String::from_utf8_lossy(&self.buf[start..end]).into_owned();
            start += pos + 1;
            if let Some(f) = self.line(&line) {
                out.push(f);
            }
        }
        self.buf.drain(..start);
        out
    }

    fn line(&mut self, line: &str) -> Option<Frame> {
        if line.is_empty() {
            // Blank line dispatches; an empty data buffer resets without dispatching.
            let event = self.event.take().unwrap_or_else(|| "message".into());
            let id = self.frame_id.take().or_else(|| self.last_id.clone());
            if self.data_lines.is_empty() {
                return None;
            }
            let data = std::mem::take(&mut self.data_lines).join("\n");
            return Some(Frame { id, event, data });
        }
        if line.starts_with(':') {
            return None; // comment / heartbeat
        }
        let (field, value) = match line.find(':') {
            Some(i) => {
                let v = &line[i + 1..];
                (&line[..i], v.strip_prefix(' ').unwrap_or(v))
            }
            None => (line, ""),
        };
        match field {
            "id" if !value.contains('\0') => {
                self.frame_id = Some(value.to_string());
                self.last_id = Some(value.to_string());
            }
            "event" => self.event = Some(value.to_string()),
            "data" => self.data_lines.push(value.to_string()),
            _ => {} // unknown fields (and `retry`) are ignored
        }
        None
    }
}

/// How a stream read ended.
#[derive(Debug, PartialEq, Eq)]
pub enum End {
    /// The server sent the terminal `end` event.
    Terminal,
    /// The consumer asked to stop.
    Stopped,
    /// EOF without a terminal event (daemon restart, network loss): reconnect.
    Lost,
}

/// Pump a byte stream through the parser, handing each frame to `on_frame`.
/// `on_frame` returns false to stop early. I/O errors surface as `Err`.
pub fn pump(mut reader: impl Read, parser: &mut Parser, mut on_frame: impl FnMut(Frame) -> bool) -> std::io::Result<End> {
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            return Ok(End::Lost);
        }
        for frame in parser.feed(&chunk[..n]) {
            if frame.event == "end" {
                return Ok(End::Terminal);
            }
            if !on_frame(frame) {
                return Ok(End::Stopped);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(p: &mut Parser, s: &str) -> Vec<Frame> {
        p.feed(s.as_bytes())
    }

    #[test]
    fn basic_frame_with_id_and_event() {
        let mut p = Parser::default();
        let f = all(&mut p, "id: 7\nevent: harness.event\ndata: {\"a\":1}\n\n");
        assert_eq!(f, vec![Frame { id: Some("7".into()), event: "harness.event".into(), data: "{\"a\":1}".into() }]);
        assert_eq!(p.last_id(), Some("7"));
    }

    #[test]
    fn crlf_lines() {
        let mut p = Parser::default();
        let f = all(&mut p, "id: 1\r\nevent: x\r\ndata: hi\r\n\r\n");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].data, "hi");
        assert_eq!(f[0].event, "x");
    }

    #[test]
    fn multi_line_data_joins_with_newline() {
        let mut p = Parser::default();
        let f = all(&mut p, "data: one\ndata: two\ndata:three\n\n");
        assert_eq!(f[0].data, "one\ntwo\nthree");
        assert_eq!(f[0].event, "message");
    }

    #[test]
    fn comments_and_heartbeats_are_ignored() {
        let mut p = Parser::default();
        assert!(all(&mut p, ": heartbeat\n\n:\n\n").is_empty());
        let f = all(&mut p, ": ping\ndata: x\n\n");
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn frames_split_across_chunks() {
        let mut p = Parser::default();
        assert!(p.feed(b"id: 4\nda").is_empty());
        assert!(p.feed(b"ta: part").is_empty());
        let f = p.feed(b"ial\n\n");
        assert_eq!(f[0].data, "partial");
        assert_eq!(f[0].id.as_deref(), Some("4"));
    }

    #[test]
    fn id_persists_for_resume_but_resets_per_frame_event() {
        let mut p = Parser::default();
        let f = all(&mut p, "id: 9\nevent: a\ndata: 1\n\ndata: 2\n\n");
        assert_eq!(f[1].event, "message", "event name resets after dispatch");
        assert_eq!(f[1].id.as_deref(), Some("9"), "last id carries forward");
        assert_eq!(p.last_id(), Some("9"));
    }

    #[test]
    fn empty_data_does_not_dispatch() {
        let mut p = Parser::default();
        assert!(all(&mut p, "event: noop\n\n").is_empty());
        // the stale event name must not leak into the next frame
        assert_eq!(all(&mut p, "data: y\n\n")[0].event, "message");
    }

    #[test]
    fn nul_in_id_is_ignored() {
        let mut p = Parser::default();
        let f = all(&mut p, "id: 3\ndata: a\n\nid: bad\0id\ndata: b\n\n");
        assert_eq!(f[1].id.as_deref(), Some("3"));
    }

    #[test]
    fn single_leading_space_only_is_stripped() {
        let mut p = Parser::default();
        assert_eq!(all(&mut p, "data:  two spaces\n\n")[0].data, " two spaces");
    }

    #[test]
    fn pump_stops_on_end_and_reports_lost_on_eof() {
        let mut p = Parser::default();
        let mut seen = vec![];
        let body = "id: 1\nevent: a\ndata: {}\n\nevent: end\ndata: {}\n\nid: 2\nevent: a\ndata: {}\n\n";
        let r = pump(body.as_bytes(), &mut p, |f| {
            seen.push(f);
            true
        })
        .unwrap();
        assert_eq!(r, End::Terminal);
        assert_eq!(seen.len(), 1);

        let mut p = Parser::default();
        let r = pump("id: 5\nevent: a\ndata: {}\n\n".as_bytes(), &mut p, |_| true).unwrap();
        assert_eq!(r, End::Lost);
        assert_eq!(p.last_id(), Some("5"), "resume cursor survives a lost stream");
    }
}
