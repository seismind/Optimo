use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::fs::{create_dir_all, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use uuid::Uuid;

use crate::event::Event;
use crate::snapshot::{ReducerSnapshot, RawObservation};
use crate::app_state::AppState;

/// Boundary to persistence.
/// Today: JSONL (decision-only).
/// Tomorrow: SQLite (same contract).
#[derive(Debug, Clone)]
pub struct StateBridge {
    snapshots_path: PathBuf,
    events_path: PathBuf,
    raw_observations_path: PathBuf,
}

impl StateBridge {
    pub fn new(state: &AppState) -> Self {
        // Keep it in data/ so it survives runs but stays local.
        let snapshots_path = state.data_dir.join("snapshots.jsonl");
        let events_path = state.data_dir.join("events.jsonl");
        let raw_observations_path = state.data_dir.join("raw_observations.jsonl");
        Self {
            snapshots_path,
            events_path,
            raw_observations_path,
        }
    }

    pub fn persist_snapshot(&self, snapshot: &ReducerSnapshot) -> Result<()> {
        let line = serde_json::to_string(snapshot).context("failed to serialize snapshot")?;
        self.append_line(&self.snapshots_path, &line)
    }

    /// Persist a raw observation to `raw_observations.jsonl`.
    /// MUST be called before any normalization/reduction step consumes the data.
    pub fn persist_raw_observation(&self, obs: &RawObservation) -> Result<()> {
        let line = serde_json::to_string(obs).context("failed to serialize raw observation")?;
        self.append_line(&self.raw_observations_path, &line)
    }

    pub fn load_snapshots(&self) -> Result<Vec<ReducerSnapshot>> {
        self.read_jsonl(&self.snapshots_path)
    }

    pub fn load_latest_snapshot(
        &self,
        document_id: Option<Uuid>,
    ) -> Result<Option<ReducerSnapshot>> {
        let snapshots = self.load_snapshots()?;
        let filtered = snapshots.into_iter().filter(|s| match document_id {
            Some(doc_id) => s.document_id == doc_id,
            None => true,
        });

        Ok(filtered.max_by(|a, b| a.created_at.cmp(&b.created_at)))
    }

    pub fn load_events(&self) -> Result<Vec<Event>> {
        self.read_jsonl(&self.events_path)
    }

    pub fn load_events_after_snapshot(&self, snapshot: &ReducerSnapshot) -> Result<Vec<Event>> {
        let cutoff = snapshot.created_at.timestamp();
        let cutoff = if cutoff < 0 { 0 } else { cutoff as u64 };

        let events = self.load_events()?;
        Ok(events
            .into_iter()
            .filter(|event| event.timestamp > cutoff)
            .collect())
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

    fn read_jsonl<T>(&self, path: &PathBuf) -> Result<Vec<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = std::fs::File::open(path)
            .with_context(|| format!("failed to open {:?}", path))?;
        let reader = BufReader::new(file);

        let mut out = Vec::new();
        for line in reader.lines() {
            let line = line.context("failed to read jsonl line")?;
            if line.trim().is_empty() {
                continue;
            }

            let value = serde_json::from_str::<T>(&line)
                .with_context(|| format!("failed to parse jsonl line from {:?}", path))?;
            out.push(value);
        }

        Ok(out)
    }
}

/// SQLite persistence layer for snapshots and snapshot lines.
///
/// Schema:
///   document_snapshots  — one row per snapshot (header + metrics)
///   snapshot_lines      — one row per (snapshot_id, page, line) — SQLite-queryable projection
///
/// Same contract as StateBridge: the reducer never touches this.
pub struct SqliteStore {
    db_path: PathBuf,
}

impl SqliteStore {
    pub fn new(state: &AppState) -> Self {
        Self {
            db_path: state.db_path.clone(),
        }
    }

    pub fn persist_snapshot(&self, snapshot: &ReducerSnapshot) -> Result<()> {
        let conn = self.open()?;
        self.ensure_schema(&conn)?;

        conn.execute(
            "INSERT OR REPLACE INTO document_snapshots
                (snapshot_id, document_id, created_at, content_hash, confidence, iterations, schema_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                snapshot.snapshot_id.to_string(),
                snapshot.document_id.to_string(),
                snapshot.created_at.to_rfc3339(),
                snapshot.content_hash.to_string(),
                snapshot.confidence as f64,
                snapshot.iterations,
                snapshot.schema_version,
            ],
        )
        .context("failed to insert document_snapshot row")?;

        for sl in &snapshot.lines {
            conn.execute(
                "INSERT OR REPLACE INTO snapshot_lines (snapshot_id, page, line, text)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    snapshot.snapshot_id.to_string(),
                    sl.page,
                    sl.line,
                    sl.text,
                ],
            )
            .context("failed to insert snapshot_line row")?;
        }

        Ok(())
    }

    /// Persist a raw observation to the `raw_observations` table.
    /// Called BEFORE the reducer sees the normalized data.
    pub fn persist_raw_observation(&self, obs: &RawObservation) -> Result<()> {
        let conn = self.open()?;
        self.ensure_schema(&conn)?;

        conn.execute(
            "INSERT OR IGNORE INTO raw_observations
                (observation_id, document_id, source, variant, created_at, raw_text, normalized_text, allow_duplicates)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                obs.observation_id.to_string(),
                obs.document_id.to_string(),
                obs.source,
                obs.variant,
                obs.created_at.to_rfc3339(),
                obs.raw_text,
                obs.normalized_text,
                obs.profile_used.allow_duplicate_positions as i32,
            ],
        )
        .context("failed to insert raw_observation row")?;

        Ok(())
    }

    #[allow(dead_code)]
    pub fn load_latest_snapshot(&self, document_id: Option<Uuid>) -> Result<Option<ReducerSnapshot>> {
        if !self.db_path.exists() {
            return Ok(None);
        }

        let conn = self.open()?;
        self.ensure_schema(&conn)?;

        let row = match document_id {
            Some(doc_id) => conn.query_row(
                "SELECT snapshot_id, document_id, created_at, content_hash, confidence, iterations, schema_version
                 FROM document_snapshots WHERE document_id = ?1
                 ORDER BY created_at DESC LIMIT 1",
                params![doc_id.to_string()],
                |r| Ok((
                    r.get::<_, String>(0)?, r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?, r.get::<_, String>(3)?,
                    r.get::<_, f64>(4)?,    r.get::<_, u32>(5)?,
                    r.get::<_, u32>(6)?,
                )),
            ),
            None => conn.query_row(
                "SELECT snapshot_id, document_id, created_at, content_hash, confidence, iterations, schema_version
                 FROM document_snapshots ORDER BY created_at DESC LIMIT 1",
                [],
                |r| Ok((
                    r.get::<_, String>(0)?, r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?, r.get::<_, String>(3)?,
                    r.get::<_, f64>(4)?,    r.get::<_, u32>(5)?,
                    r.get::<_, u32>(6)?,
                )),
            ),
        };

        let (snap_id, doc_id, created_at, content_hash, confidence, iterations, schema_version) =
            match row {
                Ok(r) => r,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
                Err(e) => return Err(anyhow::anyhow!(e).context("failed to query latest snapshot")),
            };

        let snapshot_id: Uuid = snap_id.parse().context("invalid snapshot_id uuid")?;

        let mut stmt = conn
            .prepare("SELECT page, line, text FROM snapshot_lines WHERE snapshot_id = ?1 ORDER BY page, line")
            .context("failed to prepare snapshot_lines query")?;

        let lines = stmt
            .query_map(params![snapshot_id.to_string()], |r| {
                Ok(crate::snapshot::SnapshotLine {
                    page: r.get(0)?,
                    line: r.get(1)?,
                    text: r.get(2)?,
                })
            })
            .context("failed to query snapshot_lines")?
            .collect::<Result<Vec<_>, _>>()
            .context("failed to collect snapshot_lines")?;

        Ok(Some(ReducerSnapshot {
            snapshot_id,
            document_id: doc_id.parse().context("invalid document_id uuid")?,
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                .context("invalid created_at")?
                .with_timezone(&chrono::Utc),
            content_hash: content_hash.parse().context("invalid content_hash uuid")?,
            confidence: confidence as f32,
            iterations,
            schema_version,
            lines,
            rehydration: None,
        }))
    }

    fn open(&self) -> Result<Connection> {
        if let Some(parent) = self.db_path.parent() {
            create_dir_all(parent)
                .with_context(|| format!("failed to create db dir {:?}", parent))?;
        }
        Connection::open(&self.db_path)
            .with_context(|| format!("failed to open SQLite db at {:?}", self.db_path))
    }

    fn ensure_schema(&self, conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS document_snapshots (
                snapshot_id    TEXT PRIMARY KEY,
                document_id    TEXT NOT NULL,
                created_at     TEXT NOT NULL,
                content_hash   TEXT NOT NULL,
                confidence     REAL NOT NULL,
                iterations     INTEGER NOT NULL,
                schema_version INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ds_document_id ON document_snapshots(document_id);
            CREATE INDEX IF NOT EXISTS idx_ds_created_at  ON document_snapshots(created_at);

            CREATE TABLE IF NOT EXISTS snapshot_lines (
                snapshot_id TEXT    NOT NULL,
                page        INTEGER NOT NULL,
                line        INTEGER NOT NULL,
                text        TEXT    NOT NULL,
                PRIMARY KEY (snapshot_id, page, line),
                FOREIGN KEY (snapshot_id) REFERENCES document_snapshots(snapshot_id)
            );

            CREATE TABLE IF NOT EXISTS raw_observations (
                observation_id   TEXT PRIMARY KEY,
                document_id      TEXT NOT NULL,
                source           TEXT NOT NULL,
                variant          TEXT NOT NULL,
                created_at       TEXT NOT NULL,
                raw_text         TEXT NOT NULL,
                normalized_text  TEXT NOT NULL,
                allow_duplicates INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ro_document_id ON raw_observations(document_id);
            CREATE INDEX IF NOT EXISTS idx_ro_created_at  ON raw_observations(created_at);",
        )
        .context("failed to apply SQLite schema")
    }
}
