// SPDX-License-Identifier: MIT
//
// Turns a stream of Claude Code hook payloads into a single aggregate state for
// the crab to animate.
//
// Events arrive as one JSON file per hook invocation in `inbox_dir`, written by
// the shell snippet that crab_hooks.py registers. Files are consumed in
// filename order (nanosecond timestamps) and unlinked.
//
// This module deliberately knows nothing about Wayland or rendering, so the
// whole state machine is unit-testable by feeding it recorded payloads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

/// At most this many files are drained per poll, oldest first.
pub const MAX_FILES_PER_POLL: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    Idle,
    Working,
    WaitingInput,
}

/// A one-shot thing that happened while applying events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reaction {
    /// A session reached Stop: play the celebration once.
    Finished,
    /// A tool reported failure: play the stumble once.
    Errored,
}

#[derive(Debug, Clone, Default)]
struct Session {
    state: State,
    tool: String,
    last_seen_ms: i64,
}

pub struct SessionTracker {
    inbox_dir: PathBuf,
    sessions: HashMap<String, Session>,
    /// session id -> last update, for tool priority.
    order: HashMap<String, i64>,

    aggregate: State,
    current_tool: String,
    stale_timeout_ms: i64,
    inbox_max_bytes: i64,
    inbox_max_age_ms: i64,
    sequence: i64,
    last_count: usize,
    /// Set by anything that changes the aggregate; drained by the caller.
    dirty: bool,
}

impl SessionTracker {
    pub fn new(inbox_dir: PathBuf) -> Self {
        Self {
            inbox_dir,
            sessions: HashMap::new(),
            order: HashMap::new(),
            aggregate: State::Idle,
            current_tool: String::new(),
            stale_timeout_ms: 10 * 60 * 1000,
            inbox_max_bytes: 32 * 1024 * 1024,
            inbox_max_age_ms: 60 * 60 * 1000,
            sequence: 0,
            last_count: 0,
            dirty: false,
        }
    }

    pub fn aggregate_state(&self) -> State {
        self.aggregate
    }

    pub fn current_tool(&self) -> &str {
        &self.current_tool
    }

    /// True (once) if the aggregate changed since the last call.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Sessions silent for longer than this are retired. Default 10 minutes.
    pub fn set_stale_timeout_ms(&mut self, ms: i64) {
        self.stale_timeout_ms = ms;
    }

    pub fn set_inbox_budget(&mut self, max_bytes: i64, max_age_ms: i64) {
        self.inbox_max_bytes = max_bytes;
        self.inbox_max_age_ms = max_age_ms;
    }

    /// Create the inbox and drain whatever is already in it.
    ///
    /// Prune before the first drain: after a spell with the crab not running
    /// there may be a large backlog, all of it too old to mean anything.
    pub fn start(&mut self) -> Vec<Reaction> {
        if let Err(err) = std::fs::create_dir_all(&self.inbox_dir) {
            log::warn!("cannot create {} - {err}", self.inbox_dir.display());
        }
        let now = now_ms();
        self.prune_inbox(now);
        self.poll()
    }

    /// Drain the inbox now. Safe to call at any time.
    pub fn poll(&mut self) -> Vec<Reaction> {
        let mut names = match std::fs::read_dir(&self.inbox_dir) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .map(|e| e.file_name())
                .filter(|n| Path::new(n).extension().is_some_and(|x| x == "json"))
                .collect::<Vec<_>>(),
            // A missing inbox is normal before the first hook fires.
            Err(_) => return Vec::new(),
        };
        if names.is_empty() {
            return Vec::new();
        }
        // Filenames are nanosecond timestamps, so a name sort is a time sort.
        names.sort();

        if names.len() > MAX_FILES_PER_POLL {
            log::warn!(
                "inbox has {} files; draining oldest {MAX_FILES_PER_POLL} this tick",
                names.len()
            );
            names.truncate(MAX_FILES_PER_POLL);
        }

        let now = now_ms();
        let mut reactions = Vec::new();
        for name in names {
            let path = self.inbox_dir.join(&name);

            let raw = match std::fs::read(&path) {
                Ok(raw) => raw,
                Err(err) => {
                    log::warn!("cannot read {} ({err}) - discarding", path.display());
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
            };
            let _ = std::fs::remove_file(&path);

            if let Ok(Value::Object(obj)) = serde_json::from_slice::<Value>(&raw) {
                reactions.extend(self.handle_event(&obj, now));
                continue;
            }

            // Most likely the hook capped an oversized payload mid-string. The
            // fields the crab needs all precede tool_input, so they survive.
            if let Some(salvaged) = salvage(&raw) {
                reactions.extend(self.handle_event(&salvaged, now));
                continue;
            }

            // A malformed payload is never fatal; the crab keeps walking.
            log::warn!("discarding unparseable event {}", name.to_string_lossy());
        }
        reactions
    }

    /// Apply one hook payload. `now_ms` is injected so tests are deterministic.
    pub fn handle_event(&mut self, payload: &Map<String, Value>, now_ms: i64) -> Vec<Reaction> {
        let id = payload.get("session_id").and_then(Value::as_str).unwrap_or_default();
        let event = payload.get("hook_event_name").and_then(Value::as_str).unwrap_or_default();
        if id.is_empty() || event.is_empty() {
            log::warn!("event missing session_id or hook_event_name; ignoring");
            return Vec::new();
        }

        if event == "SessionEnd" {
            if self.sessions.remove(id).is_some() {
                self.order.remove(id);
                self.recompute();
            }
            return Vec::new();
        }

        // An unknown session is registered implicitly, so starting the crab
        // mid-session still produces correct state.
        self.sequence += 1;
        let sequence = self.sequence;
        let session = self.sessions.entry(id.to_string()).or_default();
        session.last_seen_ms = now_ms;
        self.order.insert(id.to_string(), sequence);

        let mut emit_finished = false;
        let mut emit_errored = false;

        match event {
            "SessionStart" => {
                session.state = State::Idle;
                session.tool.clear();
            }
            "UserPromptSubmit" => {
                session.state = State::Working;
                session.tool.clear();
            }
            "PreToolUse" => {
                session.state = State::Working;
                session.tool =
                    payload.get("tool_name").and_then(Value::as_str).unwrap_or_default().to_string();
            }
            "PostToolUse" => {
                session.state = State::Working;
                session.tool.clear();
                emit_errored = response_indicates_error(payload.get("tool_response"));
            }
            "Notification" => {
                session.state = State::WaitingInput;
                session.tool.clear();
            }
            "Stop" => {
                session.state = State::Idle;
                session.tool.clear();
                emit_finished = true;
            }
            // SubagentStop, PreCompact and anything added later: refresh
            // liveness only. Reacting to unknown events would make the crab
            // twitchy.
            _ => return Vec::new(),
        }

        self.recompute();

        let mut reactions = Vec::new();
        if emit_errored {
            reactions.push(Reaction::Errored);
        }
        if emit_finished {
            reactions.push(Reaction::Finished);
        }
        reactions
    }

    /// Retire sessions with no event since `now_ms - stale_timeout_ms`.
    pub fn sweep_stale(&mut self, now_ms: i64) {
        let cutoff = now_ms - self.stale_timeout_ms;
        let stale: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.last_seen_ms < cutoff)
            .map(|(id, _)| id.clone())
            .collect();
        if stale.is_empty() {
            return;
        }
        for id in stale {
            // A SIGKILLed session never sends Stop or SessionEnd. Without this
            // the crab would walk forever.
            log::debug!("retiring stale session {id}");
            self.sessions.remove(&id);
            self.order.remove(&id);
        }
        self.recompute();
    }

    /// Enforce the inbox budget: drop events older than `inbox_max_age_ms`,
    /// then drop the oldest remaining until the directory is under
    /// `inbox_max_bytes`.
    ///
    /// The hooks keep writing whether or not the crab is running, so without
    /// this the inbox grows without limit across a logged-out weekend. Stale
    /// events are worthless anyway -- a session state from an hour ago says
    /// nothing about now.
    pub fn prune_inbox(&mut self, now_ms: i64) {
        // Include .tmp files: a hook killed mid-write leaves one behind forever.
        let mut entries: Vec<(PathBuf, i64, i64)> = match std::fs::read_dir(&self.inbox_dir) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter(|e| {
                    let name = e.file_name();
                    let ext = Path::new(&name).extension();
                    ext.is_some_and(|x| x == "json" || x == "tmp")
                })
                .filter_map(|e| {
                    let meta = e.metadata().ok()?;
                    let mtime = meta
                        .modified()
                        .ok()?
                        .duration_since(UNIX_EPOCH)
                        .ok()
                        .map_or(0, |d| d.as_millis() as i64);
                    Some((e.path(), meta.len() as i64, mtime))
                })
                .collect(),
            Err(_) => return,
        };
        if entries.is_empty() {
            return;
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let mut removed_aged = 0usize;
        let mut total = 0i64;
        let mut kept = Vec::with_capacity(entries.len());

        for (path, size, mtime) in entries {
            if now_ms - mtime > self.inbox_max_age_ms {
                if std::fs::remove_file(&path).is_ok() {
                    removed_aged += 1;
                }
                continue;
            }
            total += size;
            kept.push((path, size));
        }

        // Oldest first, until the directory is back inside its byte budget.
        let mut removed_for_size = 0usize;
        for (path, size) in &kept {
            if total <= self.inbox_max_bytes {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                total -= size;
                removed_for_size += 1;
            }
        }

        // Never drop events silently: a gap in the crab's behaviour should
        // always have a line explaining it.
        if removed_aged > 0 || removed_for_size > 0 {
            log::warn!(
                "pruned inbox: {removed_aged} stale, {removed_for_size} over budget; \
                 {total} bytes remain"
            );
        }
    }

    fn recompute(&mut self) {
        let mut aggregate = State::Idle;
        let mut tool = String::new();
        let mut newest_working = -1i64;

        for (id, session) in &self.sessions {
            if session.state == State::WaitingInput {
                aggregate = State::WaitingInput;
            } else if session.state == State::Working && aggregate != State::WaitingInput {
                aggregate = State::Working;
            }
            if session.state == State::Working {
                let seq = self.order.get(id).copied().unwrap_or(0);
                if seq > newest_working {
                    newest_working = seq;
                    tool = session.tool.clone();
                }
            }
        }

        if aggregate != State::Working {
            tool.clear();
        }

        let count = self.sessions.len();
        if aggregate != self.aggregate || tool != self.current_tool || count != self.last_count {
            self.aggregate = aggregate;
            self.current_tool = tool;
            self.last_count = count;
            self.dirty = true;
        }
    }
}

fn response_indicates_error(response: Option<&Value>) -> bool {
    match response {
        Some(Value::Object(obj)) => {
            if let Some(success) = obj.get("success") {
                // Qt's toBool(true): a non-bool `success` reads as true.
                return !success.as_bool().unwrap_or(true);
            }
            obj.contains_key("error")
        }
        Some(Value::String(text)) => text.to_lowercase().starts_with("error"),
        _ => false,
    }
}

/// Recover the fields the crab needs from a payload the hook truncated.
/// Returns None if `raw` does not yield a usable event.
pub fn salvage(raw: &[u8]) -> Option<Map<String, Value>> {
    let text = String::from_utf8_lossy(raw);

    let id = scan_field(&text, "session_id")?;
    let event = scan_field(&text, "hook_event_name")?;
    if id.is_empty() || event.is_empty() {
        return None;
    }

    let mut obj = Map::new();
    obj.insert("session_id".to_string(), Value::String(id));
    obj.insert("hook_event_name".to_string(), Value::String(event));
    if let Some(tool) = scan_field(&text, "tool_name")
        && !tool.is_empty()
    {
        obj.insert("tool_name".to_string(), Value::String(tool));
    }
    // tool_response sits after tool_input and so is the field most likely to
    // have been cut; a truncated payload simply loses the error blip.
    Some(obj)
}

/// Find `"key" : "value"` and return the value.
///
/// First match wins on purpose: these keys appear near the head of the payload,
/// so an occurrence of the same text inside a later tool_input cannot shadow
/// the real one.
///
/// ponytail: a hand-rolled scan rather than a regex dependency. It matches the
/// original `"key"\s*:\s*"([^"]*)"` exactly, escapes included -- which is to
/// say neither handles `\"` inside the value. Swap in `regex` if the salvage
/// path ever needs real string parsing.
fn scan_field(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(&needle) {
        let after = from + rel + needle.len();
        let rest = &text[after..];
        let trimmed = rest.trim_start();
        if let Some(rest) = trimmed.strip_prefix(':') {
            let value = rest.trim_start();
            if let Some(value) = value.strip_prefix('"')
                && let Some(end) = value.find('"')
            {
                return Some(value[..end].to_string());
            }
        }
        from = after;
    }
    None
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, name: &str) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("session_id".into(), Value::String(id.into()));
        m.insert("hook_event_name".into(), Value::String(name.into()));
        m
    }

    fn tool_event(id: &str, name: &str, tool: &str) -> Map<String, Value> {
        let mut m = event(id, name);
        m.insert("tool_name".into(), Value::String(tool.into()));
        m
    }

    fn tracker() -> SessionTracker {
        SessionTracker::new(PathBuf::from("/nonexistent-inbox"))
    }

    #[test]
    fn starts_idle() {
        let t = tracker();
        assert_eq!(t.aggregate_state(), State::Idle);
        assert_eq!(t.sessions.len(), 0);
        assert_eq!(t.current_tool(), "");
    }

    #[test]
    fn prompt_starts_working() {
        let mut t = tracker();
        t.handle_event(&event("a", "UserPromptSubmit"), 1000);
        assert_eq!(t.aggregate_state(), State::Working);
        assert_eq!(t.sessions.len(), 1);
    }

    #[test]
    fn pre_tool_use_records_the_tool() {
        let mut t = tracker();
        t.handle_event(&tool_event("a", "PreToolUse", "Bash"), 1000);
        assert_eq!(t.aggregate_state(), State::Working);
        assert_eq!(t.current_tool(), "Bash");
    }

    #[test]
    fn post_tool_use_clears_the_tool() {
        let mut t = tracker();
        t.handle_event(&tool_event("a", "PreToolUse", "Bash"), 1000);
        t.handle_event(&event("a", "PostToolUse"), 1001);
        assert_eq!(t.aggregate_state(), State::Working);
        assert_eq!(t.current_tool(), "");
    }

    #[test]
    fn stop_returns_to_idle_and_celebrates() {
        let mut t = tracker();
        t.handle_event(&event("a", "UserPromptSubmit"), 1000);
        let r = t.handle_event(&event("a", "Stop"), 1001);
        assert_eq!(t.aggregate_state(), State::Idle);
        assert_eq!(r, vec![Reaction::Finished]);
    }

    #[test]
    fn notification_wins_over_working() {
        let mut t = tracker();
        t.handle_event(&event("a", "UserPromptSubmit"), 1000);
        t.handle_event(&event("b", "Notification"), 1001);
        assert_eq!(t.aggregate_state(), State::WaitingInput);
        // Waiting is not working, so no tool is surfaced.
        assert_eq!(t.current_tool(), "");
    }

    #[test]
    fn newest_working_session_supplies_the_tool() {
        let mut t = tracker();
        t.handle_event(&tool_event("a", "PreToolUse", "Read"), 1000);
        t.handle_event(&tool_event("b", "PreToolUse", "Bash"), 1001);
        assert_eq!(t.current_tool(), "Bash");
        // An older session re-reporting takes the slot back.
        t.handle_event(&tool_event("a", "PreToolUse", "Grep"), 1002);
        assert_eq!(t.current_tool(), "Grep");
    }

    #[test]
    fn session_end_removes_the_session() {
        let mut t = tracker();
        t.handle_event(&event("a", "UserPromptSubmit"), 1000);
        t.handle_event(&event("a", "SessionEnd"), 1001);
        assert_eq!(t.sessions.len(), 0);
        assert_eq!(t.aggregate_state(), State::Idle);
    }

    #[test]
    fn unknown_events_only_refresh_liveness() {
        let mut t = tracker();
        t.handle_event(&event("a", "UserPromptSubmit"), 1000);
        t.handle_event(&event("a", "PreCompact"), 2000);
        assert_eq!(t.aggregate_state(), State::Working);
        // Liveness moved forward, so the sweep must not retire it.
        t.set_stale_timeout_ms(1500);
        t.sweep_stale(3000);
        assert_eq!(t.sessions.len(), 1);
    }

    #[test]
    fn events_missing_required_fields_are_ignored() {
        let mut t = tracker();
        let mut m = Map::new();
        m.insert("hook_event_name".into(), Value::String("Stop".into()));
        t.handle_event(&m, 1000);
        assert_eq!(t.sessions.len(), 0);
    }

    #[test]
    fn stale_sessions_are_retired() {
        let mut t = tracker();
        t.set_stale_timeout_ms(1000);
        t.handle_event(&event("a", "UserPromptSubmit"), 1000);
        t.sweep_stale(1500);
        assert_eq!(t.sessions.len(), 1);
        t.sweep_stale(2500);
        assert_eq!(t.sessions.len(), 0);
        assert_eq!(t.aggregate_state(), State::Idle);
    }

    #[test]
    fn error_detection_covers_every_shape() {
        let cases: [(&str, bool); 7] = [
            (r#"{"success": false}"#, true),
            (r#"{"success": true}"#, false),
            (r#"{"error": "boom"}"#, true),
            (r#"{"ok": 1}"#, false),
            (r#""Error: no such file""#, true),
            (r#""error: lowercase too""#, true),
            (r#""all good""#, false),
        ];
        for (json, expected) in cases {
            let v: Value = serde_json::from_str(json).unwrap();
            assert_eq!(response_indicates_error(Some(&v)), expected, "for {json}");
        }
        assert!(!response_indicates_error(None));
    }

    #[test]
    fn post_tool_use_error_emits_errored() {
        let mut t = tracker();
        let mut m = event("a", "PostToolUse");
        m.insert("tool_response".into(), serde_json::json!({"success": false}));
        assert_eq!(t.handle_event(&m, 1000), vec![Reaction::Errored]);
    }

    #[test]
    fn salvage_recovers_a_truncated_payload() {
        let raw = br#"{"session_id":"abc","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"very long comm"#;
        let obj = salvage(raw).expect("should salvage");
        assert_eq!(obj["session_id"], "abc");
        assert_eq!(obj["hook_event_name"], "PreToolUse");
        assert_eq!(obj["tool_name"], "Bash");
    }

    #[test]
    fn salvage_needs_both_required_fields() {
        assert!(salvage(br#"{"tool_name":"Bash"}"#).is_none());
        assert!(salvage(br#"{"session_id":"abc"}"#).is_none());
        assert!(salvage(b"not json at all").is_none());
    }

    #[test]
    fn salvage_takes_the_first_match() {
        // A later tool_input echoing the key must not shadow the real one.
        let raw = br#"{"session_id":"real","hook_event_name":"Stop","tool_input":{"text":"\"session_id\":\"fake\""}}"#;
        let obj = salvage(raw).unwrap();
        assert_eq!(obj["session_id"], "real");
    }

    #[test]
    fn salvage_tolerates_whitespace() {
        let raw = br#"{ "session_id"  :  "abc" , "hook_event_name" : "Stop" }"#;
        let obj = salvage(raw).unwrap();
        assert_eq!(obj["session_id"], "abc");
        assert_eq!(obj["hook_event_name"], "Stop");
    }

    #[test]
    fn dirty_flag_reports_changes_once() {
        let mut t = tracker();
        t.handle_event(&event("a", "UserPromptSubmit"), 1000);
        assert!(t.take_dirty());
        assert!(!t.take_dirty());
        // A repeat of the same state is not a change.
        t.handle_event(&event("a", "UserPromptSubmit"), 1001);
        assert!(!t.take_dirty());
    }
}
