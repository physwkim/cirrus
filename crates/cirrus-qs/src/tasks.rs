//! Async task tracker — backs the bluesky-queueserver `task_status` /
//! `task_result` RPCs and the cirrus-specific `lua_eval` RPC.
//!
//! Each `lua_eval` (and any future async-shaped RPC) registers a task
//! with [`TaskTracker::start`]; the spawned worker calls
//! [`TaskTracker::complete`] or [`TaskTracker::fail`] when done.
//! Clients poll [`TaskTracker::status`] / [`TaskTracker::result`] until
//! the task transitions out of `Running`.
//!
//! ## Retention
//!
//! Completed tasks live in the tracker for `MAX_TASKS` entries (FIFO
//! evict). This is a small bound (256) — enough for an interactive
//! REPL session, not for long-running production logging. For audit
//! trails, log task completions through `tracing` instead.

use std::collections::{HashMap, VecDeque};
use std::sync::RwLock;
use std::time::Instant;

/// Outcome of a Lua eval (or any async task).
#[derive(Clone, Debug)]
pub struct EvalResult {
    /// Captured stdout (lines printed via Lua's `print`).
    pub stdout: String,
    /// Stringified return value of the last Lua expression. `None`
    /// when the input was a statement (no return value).
    pub return_value: Option<String>,
    /// Error message if eval failed; `None` on success.
    pub error: Option<String>,
}

impl EvalResult {
    /// True if the eval finished without raising.
    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }
}

/// One tracked task.
#[derive(Debug)]
struct TaskEntry {
    state: TaskState,
    #[allow(dead_code)]
    started_at: Instant,
}

#[derive(Debug)]
enum TaskState {
    Running,
    Completed(EvalResult),
}

/// In-memory task tracker. `Arc`-clone to share across async tasks.
pub struct TaskTracker {
    inner: RwLock<Inner>,
}

struct Inner {
    /// uid → entry.
    entries: HashMap<String, TaskEntry>,
    /// FIFO order of uids; evict oldest when over MAX_TASKS.
    order: VecDeque<String>,
}

const MAX_TASKS: usize = 256;

impl TaskTracker {
    /// Build an empty tracker.
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Inner {
                entries: HashMap::new(),
                order: VecDeque::new(),
            }),
        }
    }

    /// Register a new task as `Running`. Caller is responsible for
    /// generating a unique `uid` (typically a UUID).
    pub fn start(&self, uid: &str) {
        let mut g = self.inner.write().unwrap();
        // Evict oldest if at capacity (don't let a runaway client
        // exhaust memory by spamming task creates).
        if g.entries.len() >= MAX_TASKS {
            if let Some(old) = g.order.pop_front() {
                g.entries.remove(&old);
            }
        }
        g.entries.insert(
            uid.to_string(),
            TaskEntry {
                state: TaskState::Running,
                started_at: Instant::now(),
            },
        );
        g.order.push_back(uid.to_string());
    }

    /// Mark `uid` as finished with the supplied result. No-op if the
    /// uid is unknown (caller raced eviction).
    pub fn complete(&self, uid: &str, result: EvalResult) {
        let mut g = self.inner.write().unwrap();
        if let Some(e) = g.entries.get_mut(uid) {
            e.state = TaskState::Completed(result);
        }
    }

    /// Status string for the bluesky `task_status` RPC. Returns
    /// `None` if the uid isn't tracked (caller should fall back to
    /// the legacy "always completed" stub for unknown uids).
    pub fn status(&self, uid: &str) -> Option<&'static str> {
        let g = self.inner.read().unwrap();
        g.entries.get(uid).map(|e| match &e.state {
            TaskState::Running => "running",
            TaskState::Completed(r) if r.is_success() => "completed",
            TaskState::Completed(_) => "failed",
        })
    }

    /// Full result for `task_result`. `None` for unknown / still-running
    /// tasks — caller distinguishes by also checking `status`.
    pub fn result(&self, uid: &str) -> Option<EvalResult> {
        let g = self.inner.read().unwrap();
        match g.entries.get(uid)? {
            TaskEntry {
                state: TaskState::Completed(r),
                ..
            } => Some(r.clone()),
            _ => None,
        }
    }
}

impl Default for TaskTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_result(out: &str) -> EvalResult {
        EvalResult {
            stdout: out.to_string(),
            return_value: None,
            error: None,
        }
    }

    #[test]
    fn unknown_uid_status_is_none() {
        let t = TaskTracker::new();
        assert_eq!(t.status("nope"), None);
        assert!(t.result("nope").is_none());
    }

    #[test]
    fn start_then_complete_transitions_state() {
        let t = TaskTracker::new();
        t.start("u1");
        assert_eq!(t.status("u1"), Some("running"));
        assert!(t.result("u1").is_none());
        t.complete("u1", ok_result("hi"));
        assert_eq!(t.status("u1"), Some("completed"));
        let r = t.result("u1").unwrap();
        assert_eq!(r.stdout, "hi");
        assert!(r.is_success());
    }

    #[test]
    fn failed_eval_reports_failed_status() {
        let t = TaskTracker::new();
        t.start("u2");
        t.complete(
            "u2",
            EvalResult {
                stdout: String::new(),
                return_value: None,
                error: Some("boom".into()),
            },
        );
        assert_eq!(t.status("u2"), Some("failed"));
        assert_eq!(t.result("u2").unwrap().error.as_deref(), Some("boom"));
    }

    #[test]
    fn fifo_evicts_oldest_at_capacity() {
        let t = TaskTracker::new();
        for i in 0..MAX_TASKS + 5 {
            t.start(&format!("u{i}"));
            t.complete(&format!("u{i}"), ok_result(""));
        }
        // First 5 evicted.
        for i in 0..5 {
            assert_eq!(t.status(&format!("u{i}")), None, "u{i} should be evicted");
        }
        // Last MAX_TASKS still present.
        for i in 5..MAX_TASKS + 5 {
            assert_eq!(t.status(&format!("u{i}")), Some("completed"));
        }
    }
}
