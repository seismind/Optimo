use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::observation::ObservationStatus;
use crate::ocrys::types::{OCRDocument, OCRLine, OCRPage};
use crate::snapshot::ReducerSnapshot;

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

    pub fn snapshot(&self) -> ReducerSnapshot {
        ReducerSnapshot {
            snapshot_id: uuid::Uuid::new_v4(),
            document_id: self.document_id,
            created_at: chrono::Utc::now(),
            fields: self.fields.clone(),
            confidence: self.global_confidence(),
            iterations: self.iterations,
            schema_version: 1,
        }
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
