use anyhow::{Context, Result};
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::snapshot::ReducerSnapshot;
use crate::state::AppState;

/// Boundary to persistence.
/// Today: JSONL (decision-only).
/// Tomorrow: SQLite (same contract).
#[derive(Debug, Clone)]
pub struct StateBridge {
    snapshots_path: PathBuf,
}

impl StateBridge {
    pub fn new(state: &AppState) -> Self {
        // Keep it in data/ so it survives runs but stays local.
        let snapshots_path = state.data_dir.join("snapshots.jsonl");
        Self { snapshots_path }
    }

    pub fn persist_snapshot(&self, snapshot: &ReducerSnapshot) -> Result<()> {
        let line = serde_json::to_string(snapshot).context("failed to serialize snapshot")?;
        self.append_line(&self.snapshots_path, &line)
    }

    fn append_line(&self, path: &PathBuf, line: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            create_dir_all(parent)
                .with_context(|| format!("failed to create parent dir {:?}", parent))?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("failed to open {:?}", path))?;

        writeln!(file, "{line}").context("failed to write jsonl line")?;
        Ok(())
    }
}
