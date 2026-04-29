use anyhow::Result;
use uuid::Uuid;

use crate::aggregate_state::ReducerState;
use crate::event::{Event, EventPayload, EventSource};
use crate::ocrys::types::{OCRDocument, OCRLine, OCRPage};
use crate::snapshot::{ReducerSnapshot, SnapshotMetadata};

/// Input for deterministic replay.
#[derive(Debug, Clone)]
pub struct ReplayInput {
    pub checkpoint: Option<ReducerSnapshot>,
    pub events: Vec<Event>,
}

/// Deterministic replay output.
#[derive(Debug, Clone)]
pub struct ReplayResult {
    pub state: ReducerState,
    pub applied_ocr_events: usize,
    pub skipped_events: usize,
}

/// Equivalence report between full-history replay and checkpoint+tail replay.
#[derive(Debug, Clone)]
pub struct EquivalenceReport {
    pub equivalent: bool,
    pub full_replay_hash: Uuid,
    pub checkpoint_replay_hash: Uuid,
    pub full_replay: ReplayResult,
    pub checkpoint_replay: ReplayResult,
}

/// Temporal core for deterministic state reconstruction.
///
/// Scope:
/// - replay from genesis/full history
/// - replay from snapshot + tail events
/// - deterministic equivalence checks
///
/// Out of scope:
/// - OCR execution
/// - persistence
/// - business policy
#[derive(Debug, Default, Clone)]
pub struct TimequakeCore;

impl TimequakeCore {
    pub fn new() -> Self {
        Self
    }

    pub fn replay(&self, mut input: ReplayInput) -> Result<ReplayResult> {
        input
            .events
            .sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then_with(|| a.id.cmp(&b.id)));

        let mut state = match input.checkpoint {
            Some(snapshot) => ReducerState::from_snapshot_projection(&snapshot)?,
            None => ReducerState::new(),
        };

        let mut applied_ocr_events = 0_usize;
        let mut skipped_events = 0_usize;

        for event in input.events {
            if let Some(doc) = event_to_document(event) {
                state.update_from_document(doc);
                applied_ocr_events = applied_ocr_events.saturating_add(1);
            } else {
                skipped_events = skipped_events.saturating_add(1);
            }
        }

        Ok(ReplayResult {
            state,
            applied_ocr_events,
            skipped_events,
        })
    }

    pub fn replay_genesis(&self, events: Vec<Event>) -> Result<ReplayResult> {
        self.replay(ReplayInput {
            checkpoint: None,
            events,
        })
    }

    pub fn replay_from_checkpoint(
        &self,
        checkpoint: ReducerSnapshot,
        tail_events: Vec<Event>,
    ) -> Result<ReplayResult> {
        self.replay(ReplayInput {
            checkpoint: Some(checkpoint),
            events: tail_events,
        })
    }

    /// Build a checkpoint from full history at `checkpoint_cut`, then verify
    /// deterministic equivalence between:
    /// - full replay from genesis
    /// - replay from checkpoint + tail
    pub fn verify_equivalence_with_cut(
        &self,
        full_history: Vec<Event>,
        checkpoint_cut: usize,
        metadata: SnapshotMetadata,
    ) -> Result<EquivalenceReport> {
        if checkpoint_cut > full_history.len() {
            anyhow::bail!(
                "checkpoint_cut out of bounds: cut={}, events={}",
                checkpoint_cut,
                full_history.len()
            );
        }

        let full_replay = self.replay_genesis(full_history.clone())?;

        let pre_checkpoint = self.replay_genesis(full_history[..checkpoint_cut].to_vec())?;
        let checkpoint = pre_checkpoint.state.snapshot_with_metadata(metadata);
        let checkpoint_replay = self.replay_from_checkpoint(
            checkpoint,
            full_history[checkpoint_cut..].to_vec(),
        )?;

        let full_state_json = serde_json::to_string(&full_replay.state)?;
        let checkpoint_state_json = serde_json::to_string(&checkpoint_replay.state)?;

        let full_replay_hash = Uuid::new_v5(&Uuid::NAMESPACE_OID, full_state_json.as_bytes());
        let checkpoint_replay_hash =
            Uuid::new_v5(&Uuid::NAMESPACE_OID, checkpoint_state_json.as_bytes());

        Ok(EquivalenceReport {
            equivalent: full_state_json == checkpoint_state_json,
            full_replay_hash,
            checkpoint_replay_hash,
            full_replay,
            checkpoint_replay,
        })
    }
}

fn event_to_document(event: Event) -> Option<OCRDocument> {
    let (page, line_index) = match &event.source {
        EventSource::OcrVariant {
            page, line_index, ..
        } => (*page, *line_index),
        EventSource::Reducer => return None,
    };

    match event.payload {
        EventPayload::OcrLine(line) => {
            let mut lines = Vec::new();
            if line_index > 0 {
                lines.resize(
                    line_index,
                    OCRLine {
                        text: String::new(),
                        confidence: None,
                    },
                );
            }
            lines.push(line);

            Some(OCRDocument {
                source: format!("event://{}", event.id),
                pages: vec![OCRPage {
                    page_number: page,
                    lines,
                }],
            })
        }
        EventPayload::Observation(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use uuid::Uuid;

    use crate::event::{Event, EventPayload, EventSource};
    use crate::ocrys::types::OCRLine;

    use super::{SnapshotMetadata, TimequakeCore};

    #[test]
    fn verifies_equivalence_full_vs_checkpoint_tail() {
        let events = vec![
            mk_ocr_event(1, 1, 0, "invoice 2026 alpha", 0.93),
            mk_ocr_event(2, 1, 0, "invoice 2026 alfa", 0.88),
            mk_ocr_event(3, 1, 1, "total 1000 eur", 0.96),
            mk_ocr_event(4, 1, 1, "total 1000 euro", 0.92),
            mk_ocr_event(5, 1, 2, "status approved", 0.97),
        ];

        let core = TimequakeCore::new();
        let report = core
            .verify_equivalence_with_cut(
                events,
                3,
                SnapshotMetadata {
                    snapshot_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, b"checkpoint-3"),
                    created_at: ts(3),
                },
            )
            .expect("equivalence check should succeed");

        assert!(report.equivalent, "full replay and checkpoint replay must match");
        assert_eq!(
            report.full_replay_hash,
            report.checkpoint_replay_hash,
            "state hashes must be identical"
        );
    }

    #[test]
    fn checkpoint_cut_out_of_bounds_fails() {
        let core = TimequakeCore::new();
        let result = core.verify_equivalence_with_cut(
            vec![mk_ocr_event(1, 1, 0, "only one", 0.99)],
            2,
            SnapshotMetadata {
                snapshot_id: Uuid::new_v4(),
                created_at: ts(1),
            },
        );

        assert!(result.is_err(), "invalid cut should fail");
    }

    #[test]
    fn replay_preserves_live_metrics() {
        let events = vec![
            mk_ocr_event(1, 1, 0, "A", 0.90),
            mk_ocr_event(2, 1, 0, "A", 0.85),
            mk_ocr_event(3, 1, 0, "B", 0.80),
            mk_ocr_event(4, 1, 1, "TOTAL 100", 0.95),
        ];

        let core = TimequakeCore::new();
        let replay = core
            .replay_genesis(events.clone())
            .expect("genesis replay should succeed");

        let mut runtime_state = crate::aggregate_state::ReducerState::new();
        for event in events {
            if let Some(doc) = super::event_to_document(event) {
                runtime_state.update_from_document(doc);
            }
        }

        assert_eq!(
            replay.state.convergence_score_bps,
            runtime_state.convergence_score_bps,
            "replay must preserve convergence metrics"
        );
        assert_eq!(
            replay.state.ambiguity_score_bps,
            runtime_state.ambiguity_score_bps,
            "replay must preserve ambiguity metrics"
        );
        assert_eq!(
            replay.state.fields,
            runtime_state.fields,
            "replay and runtime should converge to same projection"
        );
    }

    #[test]
    fn snapshot_midstream_then_tail_equals_genesis() {
        let events = vec![
            mk_ocr_event(1, 1, 0, "invoice 2026 alpha", 0.93),
            mk_ocr_event(2, 1, 0, "invoice 2026 alfa", 0.88),
            mk_ocr_event(3, 1, 1, "total 1000 eur", 0.96),
            mk_ocr_event(4, 1, 1, "total 1000 euro", 0.92),
            mk_ocr_event(5, 1, 2, "status approved", 0.97),
            mk_ocr_event(6, 1, 2, "status approv3d", 0.62),
        ];

        let core = TimequakeCore::new();
        let genesis = core
            .replay_genesis(events.clone())
            .expect("full replay should succeed");

        let cut = events.len() / 2;
        let head = core
            .replay_genesis(events[..cut].to_vec())
            .expect("head replay should succeed");
        let snapshot = head.state.snapshot_with_metadata(SnapshotMetadata {
            snapshot_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, b"midstream-cut"),
            created_at: ts(cut as i64),
        });

        let from_checkpoint = core
            .replay_from_checkpoint(snapshot, events[cut..].to_vec())
            .expect("checkpoint+tail replay should succeed");

        let left = serde_json::to_string(&genesis.state).expect("serialize genesis state");
        let right =
            serde_json::to_string(&from_checkpoint.state).expect("serialize checkpoint state");
        assert_eq!(
            left, right,
            "midstream snapshot + tail must match full replay"
        );
    }

    #[test]
    fn multi_checkpoint_paths_equal_genesis() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Deterministic seed for reproducibility
        let seed = 0xDEADBEEF_u64;
        let mut hasher = DefaultHasher::new();
        seed.hash(&mut hasher);
        let mut rng_state = hasher.finish();

        // Pseudo-random generator: simple linear congruential
        let mut next_random = || -> u32 {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (rng_state >> 32) as u32
        };

        // Generate 1000 deterministic events
        let mut events = Vec::new();
        for i in 0..1000 {
            let page = (next_random() % 5) as usize + 1;
            let line_index = (next_random() % 20) as usize;
            let confidence = (next_random() % 100) as f32 / 100.0;

            let text_choices = vec![
                "INVOICE 2026",
                "TOTAL 1000",
                "APPROVED",
                "PENDING",
                "AMOUNT DUE",
                "DATE",
                "VENDOR",
                "TERMS",
            ];
            let text = text_choices[(next_random() as usize) % text_choices.len()];

            events.push(mk_ocr_event(
                i as u64,
                page,
                line_index,
                text,
                confidence,
            ));
        }

        let core = TimequakeCore::new();

        // Genesis: full replay from start
        let genesis = core
            .replay_genesis(events.clone())
            .expect("genesis replay should succeed");

        // Checkpoint every 100 events
        let checkpoint_interval = 100;
        let num_checkpoints = (events.len() + checkpoint_interval - 1) / checkpoint_interval;

        for checkpoint_idx in 0..num_checkpoints {
            let cut = (checkpoint_idx + 1) * checkpoint_interval;
            if cut > events.len() {
                continue;
            }

            let head = core
                .replay_genesis(events[..cut].to_vec())
                .expect("head replay should succeed");

            let snapshot = head.state.snapshot_with_metadata(SnapshotMetadata {
                snapshot_id: Uuid::new_v5(
                    &Uuid::NAMESPACE_OID,
                    format!("checkpoint-{}", checkpoint_idx).as_bytes(),
                ),
                created_at: ts(cut as i64),
            });

            let from_checkpoint = core
                .replay_from_checkpoint(snapshot.clone(), events[cut..].to_vec())
                .expect("checkpoint+tail replay should succeed");

            // Assert all critical fields match genesis
            assert_eq!(
                from_checkpoint.state.fields, genesis.state.fields,
                "checkpoint {} fields must match genesis",
                checkpoint_idx
            );

            assert_eq!(
                from_checkpoint.state.cluster_groups, genesis.state.cluster_groups,
                "checkpoint {} cluster_groups must match genesis",
                checkpoint_idx
            );

            assert_eq!(
                from_checkpoint.state.convergence_score_bps, genesis.state.convergence_score_bps,
                "checkpoint {} convergence_score_bps must match genesis",
                checkpoint_idx
            );

            assert_eq!(
                from_checkpoint.state.ambiguity_score_bps, genesis.state.ambiguity_score_bps,
                "checkpoint {} ambiguity_score_bps must match genesis",
                checkpoint_idx
            );

            // Verify by comparing serialized state
            let genesis_serialized = serde_json::to_string(&genesis.state)
                .expect("serialize genesis state");
            let checkpoint_serialized = serde_json::to_string(&from_checkpoint.state)
                .expect("serialize checkpoint state");
            assert_eq!(
                checkpoint_serialized, genesis_serialized,
                "checkpoint {} state must match genesis when serialized",
                checkpoint_idx
            );
        }
    }

    fn mk_ocr_event(timestamp: u64, page: usize, line_index: usize, text: &str, confidence: f32) -> Event {
        Event::with_metadata(
            Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                format!("{}:{}:{}:{}", timestamp, page, line_index, text).as_bytes(),
            ),
            timestamp,
            EventSource::OcrVariant {
                variant: "timequake-test".to_string(),
                page,
                line_index,
            },
            EventPayload::OcrLine(OCRLine {
                text: text.to_string(),
                confidence: Some(confidence),
            }),
            confidence,
        )
    }

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("valid test timestamp")
    }
}
