/// Deterministic ROI refinement for targeted semantic conflict reduction.
///
/// Three-layer architecture:
///
///   PASS 1 — full-board OCR → semantic conflicts → Vec<AtomicROI>
///   PASS 2 — merge_rois(&[AtomicROI]) → Vec<MergedROI>  (pure function)
///   PASS 3 — execute MergedROI → targeted OCR → reducer merge
///
/// Separation of concerns:
///   AtomicROI  = unit of semantic observation (1 conflict → 1 AtomicROI).
///                Immutable. Does not depend on ordering.
///   MergedROI  = unit of execution.
///                Derived only from AtomicROIs, never adds semantic information.
///                Groups adjacent AtomicROIs for efficient OCR execution.
///
/// merge_rois is a pure function:
///   fn merge_rois(rois: &[AtomicROI]) -> Vec<MergedROI>
///   - no global state
///   - no random ordering
///   - no side effects
///   - same input → always same output
///
/// The merge is NOT the semantic source.
/// AtomicROIs are the observations.
/// MergedROIs are the execution plan.
///
/// Design contract:
///   All derivation depends ONLY on persisted metrics.
///   Replay regenerates identical AtomicROI and MergedROI sequences.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::snapshot::ReducerRehydrationState;

// ── Semantic observation types ────────────────────────────────────────────────

/// Reason why a position was flagged as a semantic conflict zone.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
pub enum ConflictReason {
    /// Numeric tokens present but different (e.g., "2026" vs "2027" in IDs).
    NumericConflict,
    /// High ambiguity: runner-up cluster weight close to winner.
    Ambiguity,
    /// Dense cluster with many near-identical variants at one position.
    DenseCluster,
}

impl ConflictReason {
    /// Semantic severity: higher = more important for refinement.
    ///
    /// Severity scale:
    ///   2 — NumericConflict (IDs diverge; high structural impact)
    ///   1 — Ambiguity (perceptual noise; moderate impact)
    ///   0 — DenseCluster (observation variance; low impact)
    pub fn severity(&self) -> u8 {
        match self {
            ConflictReason::NumericConflict => 2,
            ConflictReason::Ambiguity       => 1,
            ConflictReason::DenseCluster    => 0,
        }
    }

    /// Priority for merging regions: higher = more semantically important.
    /// When multiple AtomicROIs merge into one MergedROI, the reason with
    /// highest priority becomes the dominant_reason in the merged unit.
    pub fn priority(&self) -> u8 {
        self.severity()
    }
}

/// Unit of semantic observation: one conflict at one (page, line) position.
///
/// AtomicROI is immutable.
/// It represents WHAT the semantic reducer observed, not WHAT to execute.
/// The merge layer must not change its semantic content.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
pub struct AtomicROI {
    /// Page number (1-indexed).
    pub page: u32,
    /// Line number (1-indexed) where the conflict was observed.
    pub line: u32,
    /// Semantic reason for flagging this position.
    pub reason: ConflictReason,
}

// ── Execution plan types ──────────────────────────────────────────────────────

/// Suggested preprocessing variant for a region targeted by OCR refinement.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub enum PreprocessHint {
    HighContrast,
    Original,
    Rotated,
}

/// Unit of execution: one contiguous region to re-process with targeted OCR.
///
/// MergedROI groups adjacent AtomicROIs for execution efficiency.
/// It carries NO semantic information beyond what the constituent AtomicROIs hold.
/// The dominant reason is preserved for observability; it does not override semantics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedROI {
    /// Page number (1-indexed).
    pub page: u32,
    /// Inclusive line range [start, end] (1-indexed).
    pub line_range: (u32, u32),
    /// Dominant reason across constituent AtomicROIs.
    pub dominant_reason: ConflictReason,
    /// Number of AtomicROIs merged into this region.
    pub atomic_count: u32,
    /// Suggested preprocessing variant for targeted OCR.
    pub preprocess_hint: PreprocessHint,
}

/// Execution plan for a single document: ordered list of MergedROIs + metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROIPlan {
    /// Document this plan addresses.
    pub document_id: Uuid,
    /// Execution regions, sorted by (page, line_range.0) for determinism.
    pub merged_regions: Vec<MergedROI>,
    /// Total number of AtomicROIs that generated this plan.
    pub atomic_count: u32,
    /// Fingerprint of the rehydration state that generated the AtomicROIs.
    pub rehydration_hash: String,
}

// ── PASS 1: observation → AtomicROIs ─────────────────────────────────────────

/// Extract AtomicROIs from a full-board rehydration state.
///
/// One AtomicROI per (page, line) position with a detected numeric conflict.
/// Output is deterministically sorted by (page, line, reason).
pub fn observe_numeric_conflicts(rehydration: &ReducerRehydrationState) -> Vec<AtomicROI> {
    let mut observations: Vec<AtomicROI> = Vec::new();

    for (page_usize, line_map) in &rehydration.cluster_groups {
        let page = (*page_usize) as u32;

        for (line_idx_usize, clusters) in line_map {
            if clusters.is_empty() {
                continue;
            }
            for cluster in clusters.iter() {
                if cluster.len() >= 2 {
                    for i in 0..cluster.len() - 1 {
                        if differs_only_in_numeric(&cluster[i], &cluster[i + 1]) {
                            observations.push(AtomicROI {
                                page,
                                line: (*line_idx_usize + 1) as u32,
                                reason: ConflictReason::NumericConflict,
                            });
                            break; // one AtomicROI per position is enough
                        }
                    }
                }
            }
        }
    }

    // Deterministic sort: (page, line, reason).
    // BTreeMap iteration is already ordered so this is stable,
    // but we sort explicitly to guarantee determinism even if
    // the collection type changes in future.
    observations.sort();
    observations.dedup(); // one observation per (page, line, reason)
    observations
}

// ── PASS 2: AtomicROIs → MergedROIs (pure function) ──────────────────────────

/// Merge a sorted slice of AtomicROIs into execution regions.
///
/// Pure function:
///   - no global state
///   - no side effects
///   - no random ordering
///   - same input always produces the same output
///
/// `padding` adds lines above and below the first conflict line in each region.
/// Adjacent or overlapping AtomicROIs on the same page are merged into one MergedROI.
///
/// When multiple AtomicROIs merge into a single region, the `dominant_reason`
/// is the one with highest priority (semantic severity). This ensures that
/// if a region contains both NumericConflict and Ambiguity, NumericConflict
/// becomes the dominant reason for targeted OCR strategy selection.
pub fn merge_rois(rois: &[AtomicROI], padding: u32) -> Vec<MergedROI> {
    // Input must already be sorted (observe_* functions guarantee this).
    // We re-sort here defensively so merge_rois stays pure regardless of caller.
    let mut sorted: Vec<&AtomicROI> = rois.iter().collect();
    sorted.sort_by(|a, b| a.cmp(b));

    let mut merged: Vec<MergedROI> = Vec::new();

    for roi in sorted {
        let roi_start = roi.line.saturating_sub(padding);
        let roi_end = roi.line + padding;

        // Extend existing region if it's on the same page and overlaps/touches.
        if let Some(last) = merged.last_mut() {
            if last.page == roi.page && roi_start <= last.line_range.1 + 1 {
                last.line_range.1 = last.line_range.1.max(roi_end);
                last.atomic_count += 1;
                // dominant_reason: use the reason with highest priority (semantic weight).
                if roi.reason.priority() > last.dominant_reason.priority() {
                    last.dominant_reason = roi.reason;
                }
                continue;
            }
        }

        merged.push(MergedROI {
            page: roi.page,
            line_range: (roi_start, roi_end),
            dominant_reason: roi.reason,
            atomic_count: 1,
            preprocess_hint: hint_for_reason(roi.reason),
        });
    }

    merged
}

/// Generate a complete ROI plan for a document from its AtomicROIs.
pub fn generate_roi_plan(
    document_id: Uuid,
    atomics: Vec<AtomicROI>,
    rehydration_hash: String,
    padding: u32,
) -> ROIPlan {
    let atomic_count = atomics.len() as u32;
    let merged_regions = merge_rois(&atomics, padding);
    ROIPlan {
        document_id,
        merged_regions,
        atomic_count,
        rehydration_hash,
    }
}

/// Compute a stable hash of the ROIPlan for replay validation.
/// Two plans are identical if their hash matches.
pub fn plan_hash(plan: &ROIPlan) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    plan.document_id.hash(&mut hasher);
    plan.atomic_count.hash(&mut hasher);
    plan.rehydration_hash.hash(&mut hasher);
    for region in &plan.merged_regions {
        region.page.hash(&mut hasher);
        region.line_range.0.hash(&mut hasher);
        region.line_range.1.hash(&mut hasher);
        (region.dominant_reason as u8).hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns the suggested preprocessing hint for a given conflict reason.
fn hint_for_reason(reason: ConflictReason) -> PreprocessHint {
    match reason {
        ConflictReason::NumericConflict => PreprocessHint::HighContrast,
        ConflictReason::Ambiguity       => PreprocessHint::HighContrast,
        ConflictReason::DenseCluster    => PreprocessHint::Original,
    }
}

/// Returns true if two strings are identical except for their numeric tokens.
pub(crate) fn differs_only_in_numeric(a: &str, b: &str) -> bool {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();

    let a_alpha: String = a_lower.chars().filter(|c| !c.is_ascii_digit()).collect();
    let b_alpha: String = b_lower.chars().filter(|c| !c.is_ascii_digit()).collect();

    if a_alpha != b_alpha {
        return false;
    }

    let a_nums: Vec<&str> = a_lower.split(|c: char| !c.is_ascii_digit()).collect();
    let b_nums: Vec<&str> = b_lower.split(|c: char| !c.is_ascii_digit()).collect();

    a_nums != b_nums
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    // ── helpers ───────────────────────────────────────────────────────────

    fn rehydration_with_conflict(page: usize, line_idx: usize, a: &str, b: &str) -> ReducerRehydrationState {
        let mut cluster_groups: BTreeMap<usize, BTreeMap<usize, Vec<Vec<String>>>> = BTreeMap::new();
        cluster_groups
            .entry(page)
            .or_default()
            .insert(line_idx, vec![vec![a.to_string(), b.to_string()]]);
        ReducerRehydrationState {
            source: "test.png".to_string(),
            cluster_groups,
        }
    }

    // ── PASS 1: AtomicROI observation tests ───────────────────────────────

    #[test]
    fn observe_numeric_conflicts_isolates_id_drift() {
        let rehydration = rehydration_with_conflict(1, 0, "invoice 2026", "invoice 2027");
        let atomics = observe_numeric_conflicts(&rehydration);
        assert_eq!(atomics.len(), 1);
        assert_eq!(atomics[0].page, 1);
        assert_eq!(atomics[0].line, 1); // line_idx=0 → line=1
        assert_eq!(atomics[0].reason, ConflictReason::NumericConflict);
    }

    #[test]
    fn observe_numeric_conflicts_ignores_non_numeric_differences() {
        let rehydration = rehydration_with_conflict(1, 0, "pagato", "non pagato");
        let atomics = observe_numeric_conflicts(&rehydration);
        assert_eq!(atomics.len(), 0, "non-numeric diff must not produce an AtomicROI");
    }

    #[test]
    fn observe_numeric_conflicts_deduplicates_same_position() {
        // Two clusters on the same (page=1, line=1) both with numeric conflict.
        let mut cluster_groups: BTreeMap<usize, BTreeMap<usize, Vec<Vec<String>>>> = BTreeMap::new();
        cluster_groups
            .entry(1)
            .or_default()
            .insert(0, vec![
                vec!["ITEM-001".to_string(), "ITEM-002".to_string()],
                vec!["REV-1".to_string(),  "REV-2".to_string()],
            ]);
        let rehydration = ReducerRehydrationState { source: "test.png".to_string(), cluster_groups };
        let atomics = observe_numeric_conflicts(&rehydration);
        // After dedup: only one AtomicROI per (page, line, reason).
        assert_eq!(atomics.len(), 1);
    }

    #[test]
    fn observe_output_is_sorted_by_page_then_line() {
        let mut cluster_groups: BTreeMap<usize, BTreeMap<usize, Vec<Vec<String>>>> = BTreeMap::new();
        cluster_groups.entry(2).or_default()
            .insert(4, vec![vec!["A1".to_string(), "A2".to_string()]]);
        cluster_groups.entry(1).or_default()
            .insert(1, vec![vec!["B1".to_string(), "B2".to_string()]]);
        let rehydration = ReducerRehydrationState { source: "test.png".to_string(), cluster_groups };
        let atomics = observe_numeric_conflicts(&rehydration);
        assert_eq!(atomics.len(), 2);
        assert!(atomics[0] < atomics[1], "must be sorted: {:?} >= {:?}", atomics[0], atomics[1]);
        assert_eq!(atomics[0].page, 1);
        assert_eq!(atomics[1].page, 2);
    }

    // ── PASS 2: merge_rois pure-function tests ────────────────────────────

    #[test]
    fn merge_rois_is_pure_same_input_same_output() {
        let atomics = vec![
            AtomicROI { page: 1, line: 5,  reason: ConflictReason::NumericConflict },
            AtomicROI { page: 1, line: 15, reason: ConflictReason::NumericConflict },
            AtomicROI { page: 2, line: 3,  reason: ConflictReason::NumericConflict },
        ];
        let merged1 = merge_rois(&atomics, 2);
        let merged2 = merge_rois(&atomics, 2);
        assert_eq!(merged1.len(), merged2.len());
        for (m1, m2) in merged1.iter().zip(merged2.iter()) {
            assert_eq!(m1.page, m2.page);
            assert_eq!(m1.line_range, m2.line_range);
            assert_eq!(m1.atomic_count, m2.atomic_count);
        }
    }

    #[test]
    fn merge_rois_is_input_order_invariant() {
        let forward = vec![
            AtomicROI { page: 1, line: 5,  reason: ConflictReason::NumericConflict },
            AtomicROI { page: 1, line: 15, reason: ConflictReason::NumericConflict },
            AtomicROI { page: 2, line: 3,  reason: ConflictReason::NumericConflict },
        ];
        // Reversed order input.
        let reversed = vec![
            AtomicROI { page: 2, line: 3,  reason: ConflictReason::NumericConflict },
            AtomicROI { page: 1, line: 15, reason: ConflictReason::NumericConflict },
            AtomicROI { page: 1, line: 5,  reason: ConflictReason::NumericConflict },
        ];
        let merged_fwd = merge_rois(&forward,  2);
        let merged_rev = merge_rois(&reversed, 2);
        assert_eq!(merged_fwd.len(), merged_rev.len());
        for (mf, mr) in merged_fwd.iter().zip(merged_rev.iter()) {
            assert_eq!(mf.page, mr.page);
            assert_eq!(mf.line_range, mr.line_range);
        }
    }

    #[test]
    fn merge_rois_adjacent_conflicts_merge_into_one_region() {
        let atomics = vec![
            AtomicROI { page: 1, line: 10, reason: ConflictReason::NumericConflict },
            AtomicROI { page: 1, line: 11, reason: ConflictReason::NumericConflict },
            AtomicROI { page: 1, line: 12, reason: ConflictReason::NumericConflict },
        ];
        let merged = merge_rois(&atomics, 1);
        assert_eq!(merged.len(), 1, "adjacent lines must merge: {:?}", merged);
        assert_eq!(merged[0].atomic_count, 3);
        assert_eq!(merged[0].line_range.0, 9);  // 10 - padding(1)
        assert_eq!(merged[0].line_range.1, 13); // 12 + padding(1)
    }

    #[test]
    fn merge_rois_non_adjacent_conflicts_produce_separate_regions() {
        let atomics = vec![
            AtomicROI { page: 1, line: 5,  reason: ConflictReason::NumericConflict },
            AtomicROI { page: 1, line: 20, reason: ConflictReason::NumericConflict },
        ];
        let merged = merge_rois(&atomics, 2);
        assert_eq!(merged.len(), 2, "distant conflicts must not merge: {:?}", merged);
        assert_eq!(merged[0].line_range, (3, 7));  // 5±2
        assert_eq!(merged[1].line_range, (18, 22)); // 20±2
    }

    #[test]
    fn merge_rois_different_pages_never_merge() {
        let atomics = vec![
            AtomicROI { page: 1, line: 10, reason: ConflictReason::NumericConflict },
            AtomicROI { page: 2, line: 10, reason: ConflictReason::NumericConflict },
        ];
        let merged = merge_rois(&atomics, 5);
        assert_eq!(merged.len(), 2, "different pages must never merge: {:?}", merged);
        assert_eq!(merged[0].page, 1);
        assert_eq!(merged[1].page, 2);
    }

    // ── numeric helpers ───────────────────────────────────────────────────

    #[test]
    fn differs_only_in_numeric_detects_id_variants() {
        assert!(differs_only_in_numeric("invoice2026", "invoice2027"));
        assert!(differs_only_in_numeric("ITEM-001", "ITEM-002"));
        assert!(!differs_only_in_numeric("invoice2026", "invoice2026x"));
        assert!(!differs_only_in_numeric("pagato", "non pagato"));
    }
}
