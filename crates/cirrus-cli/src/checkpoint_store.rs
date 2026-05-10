//! Disk-backed [`CheckpointHook`] for crash-recovery audit trails.
//!
//! Every `Msg::Checkpoint` the engine emits gets appended to a JSONL
//! file (default `~/.cirrus/checkpoints.jsonl`). On daemon restart,
//! `manager.rs` logs the most recent record so an operator can answer
//! "where was the engine when the daemon went down?" without a full
//! plan replay.
//!
//! Full crash-recovery (resume the plan from the last checkpoint) is
//! deferred — it requires plan-arg persistence and msg_cache replay
//! which are deeper concerns.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use cirrus_engine::{CheckpointHook, CheckpointSnapshot};
use serde::{Deserialize, Serialize};

/// One JSONL record. Stable shape — extend additively.
#[derive(Debug, Serialize, Deserialize)]
pub struct CheckpointRecord {
    /// Wall-clock UTC nanoseconds since the unix epoch.
    pub timestamp_ns: u64,
    /// Currently-open run uid, if any.
    pub run_uid: Option<String>,
    /// Cirrus version that produced this record (for cross-version
    /// audit). Set to `CARGO_PKG_VERSION` at append time.
    pub cirrus_version: String,
}

/// Append-only checkpoint store. The file is opened lazily on the
/// first append and held for the daemon's lifetime; OS writeback
/// flushes the line buffer.
pub struct JsonlCheckpointStore {
    path: PathBuf,
    /// `None` until the first append succeeds. Behind a mutex so the
    /// `CheckpointHook` (Fn) can mutate it.
    file: StdMutex<Option<std::fs::File>>,
}

impl JsonlCheckpointStore {
    /// Build a store at `path`. The file is not opened yet.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            file: StdMutex::new(None),
        }
    }

    /// Append one record. Errors are logged via `tracing::warn!` and
    /// swallowed so a transient I/O fault doesn't crash the engine.
    pub fn append(&self, record: &CheckpointRecord) {
        let line = match serde_json::to_string(record) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("checkpoint store: serialize failed: {e}");
                return;
            }
        };
        let mut g = self.file.lock().unwrap();
        if g.is_none() {
            // Lazy open. Create parent dir if missing.
            if let Some(parent) = self.path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    tracing::warn!("checkpoint store: mkdir {}: {e}", parent.display());
                    return;
                }
            }
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
            {
                Ok(f) => *g = Some(f),
                Err(e) => {
                    tracing::warn!("checkpoint store: open {}: {e}", self.path.display());
                    return;
                }
            }
        }
        if let Some(f) = g.as_mut() {
            if let Err(e) = writeln!(f, "{line}") {
                tracing::warn!("checkpoint store: write: {e}");
            }
        }
    }

    /// Wrap as a [`CheckpointHook`]. The returned `Arc` can be
    /// passed straight to `RunEngine::set_checkpoint_hook`.
    pub fn into_hook(self: Arc<Self>) -> CheckpointHook {
        Arc::new(move |snap: CheckpointSnapshot| {
            self.append(&CheckpointRecord {
                timestamp_ns: snap.timestamp_ns,
                run_uid: snap.run_uid,
                cirrus_version: env!("CARGO_PKG_VERSION").to_string(),
            });
        })
    }

    /// Return the most recent record from the file (last JSONL line).
    /// `None` if the file is missing or empty. Errors are logged
    /// and surfaced as `None`.
    pub fn latest(path: &Path) -> Option<CheckpointRecord> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => return None,
        };
        let last = text.lines().rev().find(|l| !l.trim().is_empty())?;
        match serde_json::from_str(last) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!("checkpoint store: parse last record: {e}");
                None
            }
        }
    }
}

/// Default path: `$XDG_STATE_HOME/cirrus/checkpoints.jsonl` if set,
/// else `$HOME/.cirrus/checkpoints.jsonl`.
pub fn default_path() -> PathBuf {
    if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
        let mut p = PathBuf::from(state);
        p.push("cirrus");
        p.push("checkpoints.jsonl");
        return p;
    }
    if let Some(home) = std::env::var_os("HOME") {
        let mut p = PathBuf::from(home);
        p.push(".cirrus");
        p.push("checkpoints.jsonl");
        return p;
    }
    PathBuf::from(".cirrus_checkpoints.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_then_latest_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ckpt.jsonl");
        let store = Arc::new(JsonlCheckpointStore::new(&path));
        let hook = store.clone().into_hook();
        hook(CheckpointSnapshot {
            timestamp_ns: 1_000,
            run_uid: Some("r1".into()),
        });
        hook(CheckpointSnapshot {
            timestamp_ns: 2_000,
            run_uid: Some("r2".into()),
        });
        // Force flush by dropping the file handle.
        *store.file.lock().unwrap() = None;
        let last = JsonlCheckpointStore::latest(&path).expect("latest");
        assert_eq!(last.timestamp_ns, 2_000);
        assert_eq!(last.run_uid.as_deref(), Some("r2"));
        assert_eq!(last.cirrus_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn latest_handles_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.jsonl");
        assert!(JsonlCheckpointStore::latest(&path).is_none());
    }
}
