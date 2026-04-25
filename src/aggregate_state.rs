use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

use crate::observation::ObservationStatus;
use crate::ocrys::types::{OCRDocument, OCRLine, OCRPage};
use crate::profile::IngestionProfile;
use crate::snapshot::{ReducerRehydrationState, ReducerSnapshot, SnapshotLine, SnapshotMetadata, compute_content_hash};

const SIM_THRESHOLD: f64 = 0.90;
const SCORE_SCALE: u32 = 10_000;

/// Deterministic reducer state.
///
/// All fields are snapshot-safe and serialization-safe:
/// - `BTreeMap` only
/// - normalized integer scores (basis points)
/// - stable ordering for pages, lines and clusters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReducerState {
    pub document_id: Uuid,
    pub source: String,
    pub fields: BTreeMap<String, String>,
    pub pages: Vec<OCRPage>,
    pub convergence_score_bps: u32,
    pub iterations: u32,
    pub ambiguity_score_bps: u32,
    pub cluster_groups: BTreeMap<usize, BTreeMap<usize, Vec<Vec<String>>>>,
}

impl ReducerState {
    pub fn new() -> Self {
        Self {
            document_id: Uuid::nil(),
            source: String::new(),
            fields: BTreeMap::new(),
            pages: Vec::new(),
            convergence_score_bps: 0,
            iterations: 0,
            ambiguity_score_bps: 0,
            cluster_groups: BTreeMap::new(),
        }
    }

    pub fn update_from_document(&mut self, doc: OCRDocument) {
        if self.source.is_empty() {
            self.source = doc.source.clone();
            self.document_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, doc.source.as_bytes());
        }

        self.iterations = self.iterations.saturating_add(1);

        for page in doc.pages {
            let page_number = page.page_number;
            let line_map = self.cluster_groups.entry(page_number).or_default();

            for (line_index, line) in page.lines.into_iter().enumerate() {
                let text = normalize(&line.text);
                if text.is_empty() {
                    continue;
                }

                let clusters = line_map.entry(line_index).or_default();
                insert_candidate(clusters, text);
                normalize_cluster_order(clusters);
            }
        }

        self.rebuild_pages();
        self.recompute_metrics();
    }

    pub fn compute_convergence(&self) -> ObservationStatus {
        if self.pages.is_empty() {
            return ObservationStatus::Failed;
        }

        if self.ambiguity_score_bps <= 1_000 && self.convergence_score_bps >= 9_000 {
            ObservationStatus::Converged
        } else if self.convergence_score_bps >= 5_000 {
            ObservationStatus::Ambiguous
        } else {
            ObservationStatus::Failed
        }
    }

    pub fn snapshot_with_metadata(&self, metadata: SnapshotMetadata) -> ReducerSnapshot {
        let lines = self.as_snapshot_lines();
        let content_hash = compute_content_hash(self.document_id, &lines, self.iterations);
        ReducerSnapshot {
            snapshot_id: metadata.snapshot_id,
            document_id: self.document_id,
            created_at: metadata.created_at,
            lines,
            content_hash,
            confidence: self.global_confidence(),
            iterations: self.iterations,
            rehydration: Some(ReducerRehydrationState {
                source: self.source.clone(),
                cluster_groups: self.cluster_groups.clone(),
            }),
            schema_version: 1,
        }
    }

    /// Converts current pages to typed snapshot lines (SQLite-row-ready).
    fn as_snapshot_lines(&self) -> Vec<SnapshotLine> {
        let mut out = Vec::new();
        for (page_idx, page) in self.pages.iter().enumerate() {
            for (line_idx, line) in page.lines.iter().enumerate() {
                out.push(SnapshotLine {
                    page: (page_idx + 1) as u32,
                    line: (line_idx + 1) as u32,
                    text: line.text.clone(),
                });
            }
        }
        out
    }

    pub fn from_snapshot_projection(snapshot: &ReducerSnapshot) -> anyhow::Result<Self> {
        Self::from_snapshot_projection_with_profile(snapshot, &IngestionProfile::default())
    }

    pub fn from_snapshot_projection_with_profile(
        snapshot: &ReducerSnapshot,
        profile: &IngestionProfile,
    ) -> anyhow::Result<Self> {
        if snapshot.schema_version != 1 {
            anyhow::bail!("unsupported snapshot schema version: {}", snapshot.schema_version);
        }

        if !profile.allow_duplicate_positions {
            let mut seen = BTreeSet::new();
            for line in &snapshot.lines {
                let pos = (line.page, line.line);
                if !seen.insert(pos) {
                    anyhow::bail!(
                        "duplicate snapshot line position forbidden by ingestion profile: page={}, line={}",
                        line.page,
                        line.line
                    );
                }
            }
        }

        let rehydration = snapshot
            .rehydration
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("snapshot missing rehydration payload"))?;

        if rehydration.source.trim().is_empty() {
            anyhow::bail!("snapshot rehydration source is empty");
        }

        let expected_document_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, rehydration.source.as_bytes());
        if snapshot.document_id != expected_document_id {
            anyhow::bail!(
                "snapshot document_id/source mismatch: expected={}, got={}",
                expected_document_id,
                snapshot.document_id
            );
        }

        let mut cluster_groups = rehydration.cluster_groups.clone();
        for line_map in cluster_groups.values_mut() {
            for clusters in line_map.values_mut() {
                normalize_cluster_order(clusters);
            }
        }

        let mut state = Self {
            document_id: snapshot.document_id,
            source: rehydration.source.clone(),
            fields: BTreeMap::new(),
            pages: Vec::new(),
            convergence_score_bps: 0,
            iterations: snapshot.iterations,
            ambiguity_score_bps: SCORE_SCALE,
            cluster_groups,
        };

        state.rebuild_pages();
        state.recompute_metrics();

        let expected_confidence = state.global_confidence();
        if (expected_confidence - snapshot.confidence).abs() > 0.0001 {
            anyhow::bail!(
                "snapshot confidence mismatch: expected={:.4}, got={:.4}",
                expected_confidence,
                snapshot.confidence
            );
        }

        let expected_lines = state.as_snapshot_lines();
        let expected_hash = compute_content_hash(state.document_id, &expected_lines, state.iterations);
        if snapshot.content_hash != Uuid::nil() && snapshot.content_hash != expected_hash {
            anyhow::bail!(
                "snapshot content_hash mismatch: expected={}, got={}",
                expected_hash,
                snapshot.content_hash
            );
        }

        Ok(state)
    }

    pub fn global_confidence(&self) -> f32 {
        (self.convergence_score_bps as f32) / 10_000.0
    }

    fn rebuild_pages(&mut self) {
        let mut pages = Vec::new();

        for (page_number, line_map) in &self.cluster_groups {
            let mut lines = Vec::new();

            for clusters in line_map.values() {
                let winner = pick_winner_cluster(clusters);
                lines.push(OCRLine {
                    text: winner,
                    confidence: None,
                });
            }

            pages.push(OCRPage {
                page_number: *page_number,
                lines,
            });
        }

        self.pages = pages;

        self.fields.clear();
        for (page_idx, page) in self.pages.iter().enumerate() {
            for (line_idx, line) in page.lines.iter().enumerate() {
                let key = format!("page_{}_line_{}", page_idx + 1, line_idx + 1);
                self.fields.insert(key, line.text.clone());
            }
        }
    }

    fn recompute_metrics(&mut self) {
        let mut total_positions = 0_u32;
        let mut convergence_sum = 0_u32;

        for line_map in self.cluster_groups.values() {
            for clusters in line_map.values() {
                let total_candidates: usize = clusters.iter().map(|c| c.len()).sum();
                if total_candidates == 0 {
                    continue;
                }

                total_positions = total_positions.saturating_add(1);

                let winner_size = clusters.iter().map(|c| c.len()).max().unwrap_or(0) as u32;
                let total_candidates = total_candidates as u32;
                let line_convergence = (winner_size * SCORE_SCALE) / total_candidates.max(1);
                convergence_sum = convergence_sum.saturating_add(line_convergence);
            }
        }

        if total_positions == 0 {
            self.convergence_score_bps = 0;
            self.ambiguity_score_bps = SCORE_SCALE;
            return;
        }

        self.convergence_score_bps = convergence_sum / total_positions;
        self.ambiguity_score_bps = SCORE_SCALE.saturating_sub(self.convergence_score_bps);
    }
}

fn normalize(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut prev_space = false;

    for ch in lower.chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
            prev_space = false;
        } else if ch.is_whitespace() && !prev_space {
            out.push(' ');
            prev_space = true;
        }
    }

    out.trim().to_string()
}

fn insert_candidate(clusters: &mut Vec<Vec<String>>, candidate: String) {
    for cluster in clusters.iter_mut() {
        if let Some(rep) = cluster.first() {
            if strsim::jaro_winkler(rep, &candidate) >= SIM_THRESHOLD {
                cluster.push(candidate);
                return;
            }
        }
    }

    clusters.push(vec![candidate]);
}

fn normalize_cluster_order(clusters: &mut Vec<Vec<String>>) {
    for cluster in clusters.iter_mut() {
        cluster.sort();
    }

    clusters.sort_by(|a, b| {
        b.len()
            .cmp(&a.len())
            .then_with(|| best_rep(a).cmp(&best_rep(b)))
    });
}

fn pick_winner_cluster(clusters: &[Vec<String>]) -> String {
    clusters
        .iter()
        .max_by(|a, b| {
            a.len()
                .cmp(&b.len())
                .then_with(|| best_rep(a).len().cmp(&best_rep(b).len()))
                .then_with(|| best_rep(b).cmp(&best_rep(a)))
        })
        .map(|cluster| best_rep(cluster).to_string())
        .unwrap_or_default()
}

fn best_rep(cluster: &[String]) -> &str {
    cluster
        .iter()
        .max_by(|a, b| a.len().cmp(&b.len()).then_with(|| b.cmp(a)))
        .map(|s| s.as_str())
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use uuid::Uuid;

    use super::ReducerState;
    use crate::profile::IngestionProfile;
    use crate::snapshot::{ReducerRehydrationState, ReducerSnapshot, SnapshotLine};

    #[test]
    fn snapshot_must_fail_before_hydration_when_rehydration_missing() {
        let bad_snapshot = ReducerSnapshot {
            snapshot_id: Uuid::new_v4(),
            document_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, b"test-doc"),
            created_at: ts(10),
            lines: vec![],
            content_hash: Uuid::nil(),
            confidence: 0.85,
            iterations: 5,
            rehydration: None,
            schema_version: 1,
        };

        let result = ReducerState::from_snapshot_projection(&bad_snapshot);
        assert!(
            result.is_err(),
            "snapshot without rehydration must fail before hydration"
        );

        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(
            err_msg.contains("rehydration"),
            "error must mention rehydration: {}",
            err_msg
        );
    }

    #[test]
    fn snapshot_must_fail_on_document_id_source_mismatch() {
        let rehydration = ReducerRehydrationState {
            source: "image://original.png".to_string(),
            cluster_groups: Default::default(),
        };

        let wrong_document_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, b"wrong-id");

        let bad_snapshot = ReducerSnapshot {
            snapshot_id: Uuid::new_v4(),
            document_id: wrong_document_id,
            created_at: ts(10),
            lines: vec![],
            content_hash: Uuid::nil(),
            confidence: 0.0,
            iterations: 0,
            rehydration: Some(rehydration),
            schema_version: 1,
        };

        let result = ReducerState::from_snapshot_projection(&bad_snapshot);
        assert!(
            result.is_err(),
            "snapshot with mismatched document_id/source must fail"
        );

        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(
            err_msg.to_lowercase().contains("mismatch"),
            "error must mention mismatch: {}",
            err_msg
        );
    }

    #[test]
    fn snapshot_must_fail_on_empty_source() {
        let rehydration = ReducerRehydrationState {
            source: "".to_string(),
            cluster_groups: Default::default(),
        };

        let bad_snapshot = ReducerSnapshot {
            snapshot_id: Uuid::new_v4(),
            document_id: Uuid::nil(),
            created_at: ts(10),
            lines: vec![],
            content_hash: Uuid::nil(),
            confidence: 0.0,
            iterations: 0,
            rehydration: Some(rehydration),
            schema_version: 1,
        };

        let result = ReducerState::from_snapshot_projection(&bad_snapshot);
        assert!(result.is_err(), "snapshot with empty source must fail");

        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(
            err_msg.to_lowercase().contains("empty"),
            "error must mention empty source: {}",
            err_msg
        );
    }

    #[test]
    fn snapshot_must_fail_on_confidence_mismatch_after_recompute() {
        let rehydration = ReducerRehydrationState {
            source: "image://original.png".to_string(),
            cluster_groups: Default::default(),
        };

        let correct_document_id =
            Uuid::new_v5(&Uuid::NAMESPACE_URL, "image://original.png".as_bytes());

        let bad_snapshot = ReducerSnapshot {
            snapshot_id: Uuid::new_v4(),
            document_id: correct_document_id,
            created_at: ts(10),
            lines: vec![],
            content_hash: Uuid::nil(),
            confidence: 0.99,
            iterations: 0,
            rehydration: Some(rehydration),
            schema_version: 1,
        };

        let result = ReducerState::from_snapshot_projection(&bad_snapshot);
        assert!(
            result.is_err(),
            "snapshot with inconsistent confidence must fail after recompute"
        );

        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(
            err_msg.to_lowercase().contains("confidence"),
            "error must mention confidence mismatch: {}",
            err_msg
        );
    }

    #[test]
    fn ingestion_profile_allows_duplicate_positions_when_enabled() {
        let source = "image://profile-allow.png";
        let document_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, source.as_bytes());
        let snapshot = ReducerSnapshot {
            snapshot_id: Uuid::new_v4(),
            document_id,
            created_at: ts(10),
            lines: vec![
                SnapshotLine {
                    page: 1,
                    line: 1,
                    text: "a".to_string(),
                },
                SnapshotLine {
                    page: 1,
                    line: 1,
                    text: "a-duplicate".to_string(),
                },
            ],
            content_hash: Uuid::nil(),
            confidence: 0.0,
            iterations: 0,
            rehydration: Some(ReducerRehydrationState {
                source: source.to_string(),
                cluster_groups: Default::default(),
            }),
            schema_version: 1,
        };

        let profile = IngestionProfile::tesseract();
        let result = ReducerState::from_snapshot_projection_with_profile(&snapshot, &profile);
        assert!(result.is_ok(), "duplicates should be allowed by profile");
    }

    #[test]
    fn ingestion_profile_rejects_duplicate_positions_when_disabled() {
        let source = "image://profile-reject.png";
        let document_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, source.as_bytes());
        let snapshot = ReducerSnapshot {
            snapshot_id: Uuid::new_v4(),
            document_id,
            created_at: ts(10),
            lines: vec![
                SnapshotLine {
                    page: 1,
                    line: 1,
                    text: "a".to_string(),
                },
                SnapshotLine {
                    page: 1,
                    line: 1,
                    text: "a-duplicate".to_string(),
                },
            ],
            content_hash: Uuid::nil(),
            confidence: 0.0,
            iterations: 0,
            rehydration: Some(ReducerRehydrationState {
                source: source.to_string(),
                cluster_groups: Default::default(),
            }),
            schema_version: 1,
        };

        let profile = IngestionProfile::strict();
        let result = ReducerState::from_snapshot_projection_with_profile(&snapshot, &profile);
        assert!(result.is_err(), "duplicates should be rejected by profile");
    }

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("valid test timestamp")
    }
}
