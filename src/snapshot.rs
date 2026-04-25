use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;
use chrono::{DateTime, Utc};

pub use crate::profile::IngestionProfile;


/// Raw observation: what actually arrived from a single OCR source before any
/// semantic transformation.
///
/// Two-layer contract:
///   raw_text        — direct tesseract output (trim-only). The audit trail.
///   normalized_text — canonical projection fed to the reducer.
///
/// These two MUST both be persisted. Never discard raw_text.
/// See ARCHITECTURE.md § Raw vs Canonical.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawObservation {
    /// Unique ID for this observation (Uuid::now_v7 — injected by runtime).
    pub observation_id: Uuid,
    /// Derived deterministically from the source file path.
    pub document_id: Uuid,
    /// Source file path (same key used for document_id derivation).
    pub source: String,
    /// OCR variant that produced this observation: "original" | "high_contrast" | "rotated".
    pub variant: String,
    /// Wall-clock time at observation capture (injected by runtime, never by reducer).
    pub created_at: DateTime<Utc>,
    /// Direct OCR output: trim-only. Preserved forever as audit trail.
    pub raw_text: String,
    /// Canonical projection: NFC + decimal comma→dot. Fed to the reducer.
    pub normalized_text: String,
    /// Which ingestion policy was active when this observation was captured.
    pub profile_used: IngestionProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub snapshot_id: Uuid,
    pub created_at: DateTime<Utc>,
}

/// A single extracted text line.
/// Typed and SQLite-row-ready: one row per (snapshot_id, page, line).
/// Replaces the old BTreeMap<String, String> `page_N_line_M` pattern.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct SnapshotLine {
    pub page: u32,
    pub line: u32,
    pub text: String,
}

/// Minimal canonical reducer internals for deterministic fold resume.
/// Separate from projection fields (SnapshotLine) intentionally:
/// what you store for audit != what you need to resume the fold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReducerRehydrationState {
    pub source: String,
    pub cluster_groups: BTreeMap<usize, BTreeMap<usize, Vec<Vec<String>>>>,
}

/// V1 snapshot schema.
///
/// Schema versioning contract:
///   schema_version = 1  →  this struct, SnapshotLine-based
///   schema_version = 2  →  future migration required
///
/// content_hash = Uuid::new_v5(NAMESPACE_OID, canonical_json(document_id, sorted lines, iterations))
/// Enables corruption detection without full rehydration.
/// See ADR-0009 for confidence formula.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReducerSnapshot {
    pub snapshot_id: Uuid,
    pub document_id: Uuid,
    pub created_at: DateTime<Utc>,

    /// Extracted text lines — typed, ordered, SQLite-row-ready.
    pub lines: Vec<SnapshotLine>,

    /// Deterministic fingerprint: hash(document_id, sorted lines, iterations).
    /// Stable across re-runs given identical input.
    pub content_hash: Uuid,

    /// Convergence confidence in [0.0, 1.0].
    /// Formula: mean(winner_cluster_size / total_candidates) over all line positions.
    /// See ADR-0009.
    pub confidence: f32,

    pub iterations: u32,

    /// Minimal reducer internals for deterministic fold resume.
    #[serde(default)]
    pub rehydration: Option<ReducerRehydrationState>,

    pub schema_version: u32,
}

/// Computes a deterministic content fingerprint over the projection payload.
///
/// Canonicalization rules:
///   - lines are sorted by (page, line) before hashing — order of insertion is irrelevant
///   - document_id and iterations are included verbatim
///   - runtime metadata (snapshot_id, created_at) are intentionally excluded
///
/// Contract:
///   same content + different metadata → same hash
///   different line order + same content → same hash
///   any text change in any line      → different hash
///   duplicate lines are significant  → [A,B] != [A,B,B]
///   compute_content_hash is cardinality-sensitive:
///   duplicate lines are preserved and affect the hash by design.
pub fn compute_content_hash(document_id: Uuid, lines: &[SnapshotLine], iterations: u32) -> Uuid {
    let mut sorted = lines.to_vec();
    sorted.sort();
    let canonical = serde_json::json!({
        "document_id": document_id.to_string(),
        "iterations": iterations,
        "lines": sorted,
    });
    Uuid::new_v5(&Uuid::NAMESPACE_OID, canonical.to_string().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(page: u32, line: u32, text: &str) -> SnapshotLine {
        SnapshotLine { page, line, text: text.to_string() }
    }

    fn doc_id() -> Uuid {
        Uuid::new_v5(&Uuid::NAMESPACE_OID, b"test-document")
    }

    #[test]
    fn same_content_different_metadata_produces_same_hash() {
        let lines = vec![
            line(1, 1, "invoice alfa"),
            line(1, 2, "total 1000 eur"),
        ];

        let hash_a = compute_content_hash(doc_id(), &lines, 3);
        // Different snapshot_id and created_at are not inputs — hash must be identical.
        let hash_b = compute_content_hash(doc_id(), &lines, 3);

        assert_eq!(hash_a, hash_b,
            "same content + different metadata must produce same hash");
    }

    #[test]
    fn different_line_order_produces_same_hash() {
        let ordered = vec![
            line(1, 1, "invoice alfa"),
            line(1, 2, "total 1000 eur"),
            line(1, 3, "status approved"),
        ];
        let shuffled = vec![
            line(1, 3, "status approved"),
            line(1, 1, "invoice alfa"),
            line(1, 2, "total 1000 eur"),
        ];

        let hash_ordered  = compute_content_hash(doc_id(), &ordered, 3);
        let hash_shuffled = compute_content_hash(doc_id(), &shuffled, 3);

        assert_eq!(hash_ordered, hash_shuffled,
            "lines in different order must canonicalize to the same hash");
    }

    #[test]
    fn single_line_change_produces_different_hash() {
        let original = vec![
            line(1, 1, "invoice alfa"),
            line(1, 2, "total 1000 eur"),
        ];
        let mutated = vec![
            line(1, 1, "invoice alfa"),
            line(1, 2, "total 999 eur"),  // one word changed
        ];

        let hash_original = compute_content_hash(doc_id(), &original, 3);
        let hash_mutated  = compute_content_hash(doc_id(), &mutated, 3);

        assert_ne!(hash_original, hash_mutated,
            "a single changed line must produce a different hash");
    }

    #[test]
    fn duplicate_line_changes_hash() {
        let base = vec![
            line(1, 1, "invoice alfa"),
            line(1, 2, "total 1000 eur"),
        ];
        let duplicated = vec![
            line(1, 1, "invoice alfa"),
            line(1, 2, "total 1000 eur"),
            line(1, 2, "total 1000 eur"),
        ];

        let hash_base = compute_content_hash(doc_id(), &base, 3);
        let hash_duplicated = compute_content_hash(doc_id(), &duplicated, 3);

        assert_ne!(
            hash_base,
            hash_duplicated,
            "duplicates are semantic input and must affect content_hash"
        );
    }
}
