use crate::ocrys::types::OCRDocument;
use crate::reducer_state::ReducerState;
use crate::snapshot::ReducerSnapshot;
use crate::observation::{OcrObservation, ObservationStatus, Severity};
use uuid::Uuid;

// TODO: snapshot persistence
// TODO: event replay integration
// TODO: distributed reducer partitioning

/// Deterministic reducer.
///
/// Contract:
///   Reducer: Vec<OCRDocument> -> ReducerState
pub fn reduce_documents(docs: Vec<OCRDocument>) -> anyhow::Result<ReducerState> {
    if docs.is_empty() {
        return Err(anyhow::anyhow!("no OCR documents to reduce"));
    }

    let mut state = ReducerState::new();
    for doc in docs {
        state.update_from_document(doc);
    }

    Ok(state)
}

#[allow(dead_code)]
pub fn snapshot_documents(docs: Vec<OCRDocument>) -> anyhow::Result<ReducerSnapshot> {
    let state = reduce_documents(docs)?;
    Ok(state.snapshot())
}

pub fn emit_observation(state: &ReducerState) -> anyhow::Result<Option<OcrObservation>> {
    let status = state.compute_convergence();
    if status == ObservationStatus::Converged {
        return Ok(None);
    }

    let mut observation = OcrObservation::new(
        Uuid::new_v5(&Uuid::NAMESPACE_URL, state.source.as_bytes()),
        "reducer.document",
        status,
    );

    observation.value = Some(state.source.clone());
    observation.confidence = Some(state.global_confidence());
    observation.iterations = state.iterations;

    match status {
        ObservationStatus::Ambiguous => {
            observation.severity = Some(Severity::Medium);
            observation.reason_code = Some("ambiguity_high".to_string());
            observation.note = Some(format!(
                "ambiguity_score_bps={} convergence_score_bps={}",
                state.ambiguity_score_bps,
                state.convergence_score_bps
            ));
        }
        ObservationStatus::Failed => {
            observation.severity = Some(Severity::High);
            observation.reason_code = Some("reducer_failed".to_string());
            observation.note = Some(format!(
                "pages={} iterations={}",
                state.pages.len(),
                state.iterations
            ));
        }
        ObservationStatus::Converged => {}
    }

    observation.validate()?;
    Ok(Some(observation))
}
