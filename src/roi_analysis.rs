/// Deterministic ROI refinement for targeted semantic conflict reduction.
///
/// Purpose:
///   Extract regions of semantic instability from full-board metrics,
///   generate deterministic ROI refinement plans, then merge refined
///   observations without breaking replay invariance.
///
/// Design contract:
///   ROI generation is FULLY deterministic.
///   No timing, async ordering, or randomness.
///   Replay must regenerate identical ROI plans.
///
/// Semantic rule:
///   Objective is to reduce perceptual noise conflicts,
///   NOT to fabricate convergence. Reducer preserves semantic honesty.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::snapshot::ReducerRehydrationState;

/// Reason why a region was selected for refinement.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
pub enum ROIReason {
    /// Numeric tokens present but different (e.g., "2026" vs "2027" in IDs).
    NumericConflict,
    /// High ambiguity (runner-up vote weight near winner).
    Ambiguity,
    /// Dense cluster with many near-identical variants.
    DenseCluster,
}

/// A region of interest for targeted refinement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROIRegion {
    /// Page number (1-indexed).
    pub page: u32,
    /// Line range [start, end] (1-indexed, inclusive).
    pub line_range: (u32, u32),
    /// Why this region was selected.
    pub reason: ROIReason,
    /// Confidence that refinement will help [0.0, 1.0].
    pub confidence: f32,
    /// Padding in lines to add above/below the conflict zone.
    pub padding: u32,
    /// Suggested preprocessing variant for this region.
    pub preprocess_hint: String,
}

/// A deterministic plan for targeted ROI refinement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROIPlan {
    /// Document this plan addresses.
    pub document_id: Uuid,
    /// Regions to refine, sorted by (page, line_range) for determinism.
    pub regions: Vec<ROIRegion>,
    /// Minimum threshold to include a line in the plan.
    pub conflict_threshold: f32,
    /// Source state that generated this plan.
    pub rehydration_hash: String,
}

/// Analyzes full-board reduction state and extracts numeric conflict zones.
///
/// Returns a list of (page, line_index, conflict_type) tuples, deterministically sorted.
pub fn analyze_numeric_conflicts(
    rehydration: &ReducerRehydrationState,
) -> Vec<(u32, u32, ROIReason)> {
    let mut conflicts = Vec::new();

    for (page_usize, line_map) in &rehydration.cluster_groups {
        let page = (*page_usize) as u32;

        for (line_idx_usize, clusters) in line_map {
            if clusters.is_empty() {
                continue;
            }

            // Detect numeric conflicts:
            // If a cluster has 2+ candidates that differ only in numeric tokens,
            // it indicates a numeric conflict zone.
            let mut has_numeric_conflict = false;

            for cluster in clusters.iter() {
                if cluster.len() >= 2 {
                    // Check if candidates in this cluster differ only in numeric tokens.
                    let mut numeric_diff_count = 0;
                    for i in 0..cluster.len() - 1 {
                        let a = &cluster[i];
                        let b = &cluster[i + 1];
                        if differs_only_in_numeric(a, b) {
                            numeric_diff_count += 1;
                        }
                    }
                    if numeric_diff_count > 0 {
                        has_numeric_conflict = true;
                        break;
                    }
                }
            }

            if has_numeric_conflict {
                conflicts.push((page, (*line_idx_usize + 1) as u32, ROIReason::NumericConflict));
            }
        }
    }

    // Deterministic sort by (page, line).
    conflicts.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| (a.2 as u8).cmp(&(b.2 as u8)))
    });

    conflicts
}

/// Returns true if two strings are identical except for numeric tokens.
fn differs_only_in_numeric(a: &str, b: &str) -> bool {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();

    let a_alpha: String = a_lower.chars().filter(|c| !c.is_ascii_digit()).collect();
    let b_alpha: String = b_lower.chars().filter(|c| !c.is_ascii_digit()).collect();

    if a_alpha != b_alpha {
        return false; // Different non-numeric parts.
    }

    let a_nums: Vec<&str> = a_lower.split(|c: char| !c.is_ascii_digit()).collect();
    let b_nums: Vec<&str> = b_lower.split(|c: char| !c.is_ascii_digit()).collect();

    // Same numeric structure but different values.
    a_nums != b_nums
}

/// Generates a deterministic ROI refinement plan from conflict analysis.
///
/// Inputs are fully deterministic:
/// - sorted conflict list
/// - fixed thresholds
/// - stable coordinate system
///
/// Output ROI plan is deterministically ordered and reproducible.
pub fn generate_roi_plan(
    document_id: Uuid,
    conflicts: Vec<(u32, u32, ROIReason)>,
    rehydration_hash: String,
    padding: u32,
    conflict_threshold: f32,
) -> ROIPlan {
    let mut regions: Vec<ROIRegion> = Vec::new();

    for (page, line, reason) in conflicts {
        // Merge adjacent conflict lines into single region.
        if let Some(last_region) = regions.last_mut() {
            if last_region.page == page && line <= last_region.line_range.1 + 1 {
                // Extend the existing region.
                last_region.line_range.1 = line;
                continue;
            }
        }

        regions.push(ROIRegion {
            page,
            line_range: (line.saturating_sub(padding), line + padding),
            reason,
            confidence: 0.75,
            padding,
            preprocess_hint: "high_contrast".to_string(),
        });
    }

    // Final deterministic sort.
    regions.sort_by(|a, b| {
        a.page
            .cmp(&b.page)
            .then_with(|| a.line_range.0.cmp(&b.line_range.0))
    });

    ROIPlan {
        document_id,
        regions,
        conflict_threshold,
        rehydration_hash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn numeric_conflict_detection_isolates_id_drift() {
        let mut cluster_groups: BTreeMap<usize, BTreeMap<usize, Vec<Vec<String>>>> =
            BTreeMap::new();

        // Numeric conflict: invoice IDs differing only in year.
        cluster_groups
            .entry(1)
            .or_default()
            .insert(0, vec![vec!["invoice 2026".to_string(), "invoice 2027".to_string()]]);

        let rehydration = ReducerRehydrationState {
            source: "test.png".to_string(),
            cluster_groups,
        };

        let conflicts = analyze_numeric_conflicts(&rehydration);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].0, 1); // page
        assert_eq!(conflicts[0].1, 1); // line
        assert_eq!(conflicts[0].2, ROIReason::NumericConflict);
    }

    #[test]
    fn roi_plan_generation_is_deterministic() {
        let doc_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, b"test-doc");
        let conflicts = vec![
            (1, 5, ROIReason::NumericConflict),
            (1, 15, ROIReason::NumericConflict),
            (2, 3, ROIReason::NumericConflict),
        ];

        let plan1 = generate_roi_plan(
            doc_id,
            conflicts.clone(),
            "hash1".to_string(),
            2,
            0.75,
        );

        let plan2 = generate_roi_plan(
            doc_id,
            conflicts,
            "hash1".to_string(),
            2,
            0.75,
        );

        assert_eq!(plan1.regions.len(), plan2.regions.len());
        for (r1, r2) in plan1.regions.iter().zip(plan2.regions.iter()) {
            assert_eq!(r1.page, r2.page);
            assert_eq!(r1.line_range, r2.line_range);
        }
    }

    #[test]
    fn adjacent_conflicts_merge_into_single_region() {
        let doc_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, b"test-doc");
        let conflicts = vec![
            (1, 10, ROIReason::NumericConflict),
            (1, 11, ROIReason::NumericConflict),
            (1, 12, ROIReason::NumericConflict),
        ];

        let plan = generate_roi_plan(
            doc_id,
            conflicts,
            "hash1".to_string(),
            1, // padding = 1
            0.75,
        );

        // Adjacent lines within padding distance should merge into a single region.
        // With padding=1: first conflict at 10 → (9,11), then 11 and 12 are
        // within "last_region + 1" so they merge, extending to 12.
        assert_eq!(plan.regions.len(), 1);
        assert_eq!(plan.regions[0].line_range, (9, 12));
    }

    #[test]
    fn differs_only_in_numeric_detects_id_variants() {
        assert!(differs_only_in_numeric("invoice2026", "invoice2027"));
        assert!(differs_only_in_numeric("ITEM-001", "ITEM-002"));
        assert!(!differs_only_in_numeric("invoice2026", "invoice2026x"));
        assert!(!differs_only_in_numeric("pagato", "non pagato"));
    }
}
