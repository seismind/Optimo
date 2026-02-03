use anyhow::{Context, Result};
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::state::AppState;

/// Boundary to persistence.
/// Today: JSONL (decision-only).
/// Tomorrow: SQLite (same contract).
#[derive(Debug, Clone)]
pub struct StateBridge {
    path: PathBuf,
}

impl StateBridge {
    pub fn new(state: &AppState) -> Self {
        // Keep it in data/ so it survives runs but stays local.
        let path = state.data_dir.join("observations.jsonl");
        Self { path }
    }

    pub fn record_jsonl(&self, line: &str) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            create_dir_all(parent)
                .with_context(|| format!("failed to create parent dir {:?}", parent))?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("failed to open {:?}", self.path))?;

        writeln!(file, "{line}").context("failed to write observation line")?;
        Ok(())
    }
}
