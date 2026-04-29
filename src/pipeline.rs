// Tokio + Rayon "perfect" boundary:
//
// - Tokio orchestrates *documents* concurrently (async / IO-friendly).
// - For each document we hop into a CPU worker using spawn_blocking.
// - Inside spawn_blocking we use Rayon to parallelize OCR "variants" (Map).
// - Then we normalize/merge (Reduce) deterministically.
// - Finally we decide what deserves to be persisted (StateBridge = final reducer).
//
// Two-layer persistence contract:
//   1. RawObservation  — direct tesseract output (trim-only). Persisted first. Never mutated.
//   2. ReducerSnapshot — canonical projection after normalization + reduction.

use anyhow::{Context, Result};
use chrono::Utc;
use rayon::prelude::*;
use std::path::PathBuf;
use tokio::task::{spawn_blocking, JoinSet};
use uuid::Uuid;

use crate::aggregate_state::ReducerState;
use crate::app_state::AppState;
use crate::fold;
use crate::ocrys;
use crate::ocrys::normalize;
use crate::ocrys::types::OCRDocument;
use crate::persistence::{StateBridge, SqliteStore};
use crate::profile::IngestionProfile;
use crate::snapshot::{RawObservation, SnapshotMetadata};

/// Entry point from main.rs
pub async fn process_documents(state: &AppState, docs: Vec<PathBuf>) -> Result<()> {
    let bridge = StateBridge::new(state);
    let sqlite = SqliteStore::new(state);

    let mut set = JoinSet::new();

    for doc in docs {
        let state = state.clone();
        set.spawn(async move {
            process_one_document(&state, doc).await
        });
    }

    while let Some(res) = set.join_next().await {
        let (reducer_state, raw_observations) = res.context("task join failed")??;

        // Persist raw observations FIRST — audit trail before any derived data.
        for obs in &raw_observations {
            bridge.persist_raw_observation(obs)?;
            sqlite.persist_raw_observation(obs)?;
        }

        let metadata = SnapshotMetadata {
            snapshot_id: Uuid::now_v7(),
            created_at: Utc::now(),
        };
        let snapshot = reducer_state.snapshot_with_metadata(metadata);
        bridge.persist_snapshot(&snapshot)?;
        sqlite.persist_snapshot(&snapshot)?;
    }

    Ok(())
}

async fn process_one_document(state: &AppState, doc: PathBuf) -> Result<(ReducerState, Vec<RawObservation>)> {
    // Extract only what the CPU worker needs
    let lang = state.ocr_lang.clone();
    let run_dir = state.ocr_run_dir("latest");
    let profile = state.ingestion_profile.clone();

    tokio::fs::create_dir_all(&run_dir).await?;

    // CPU-bound section (Tokio → Rayon boundary)
    let result = spawn_blocking(move || {
        cpu_map_reduce_ocr(&doc, &run_dir, &lang, &profile)
    })
    .await??;

    Ok(result)
}

/// MAP + REDUCE (CPU-bound)
///
/// Returns:
///   - ReducerState from normalized (canonical) documents
///   - Vec<RawObservation> with raw_text + normalized_text per variant
fn cpu_map_reduce_ocr(
    doc: &PathBuf,
    run_dir: &PathBuf,
    lang: &str,
    profile: &IngestionProfile,
) -> Result<(ReducerState, Vec<RawObservation>)> {
    let source = doc.to_string_lossy().to_string();
    // document_id must match what ReducerState will derive from the source path.
    let document_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, source.as_bytes());
    let now = Utc::now();

    let variants = vec!["original", "high_contrast", "rotated"];

    // ---- MAP ----
    // Produce (raw_doc, normalized_doc) in parallel per variant.
    let pairs: Vec<(OCRDocument, OCRDocument, String)> =
        variants
            .par_iter()
            .map(|variant| {
                let raw_doc = ocrys::run_ocr(doc, run_dir, lang, variant)
                    .with_context(|| format!("OCR failed for variant {}", variant))?;
                let normalized_doc = normalize::normalize_document_with_profile(&raw_doc, profile);
                Ok((raw_doc, normalized_doc, variant.to_string()))
            })
            .collect::<Result<Vec<_>>>()?;

    // ---- CAPTURE RAW OBSERVATIONS ----
    // Built before reduction — preserves raw truth regardless of reducer outcome.
    let raw_observations: Vec<RawObservation> = pairs
        .iter()
        .map(|(raw_doc, norm_doc, variant)| RawObservation {
            observation_id: Uuid::now_v7(), // runtime-injected monotonic ID
            document_id,
            source: source.clone(),
            variant: variant.clone(),
            created_at: now,
            raw_text: normalize::document_to_text(raw_doc),
            normalized_text: normalize::document_to_text(norm_doc),
            profile_used: profile.clone(),
        })
        .collect();

    // ---- REDUCE (delegated to reducer module) ----
    // Only the normalized documents flow into the reducer.
    let normalized_docs: Vec<_> = pairs.into_iter().map(|(_, norm, _)| norm).collect();
    let reduced_state = fold::reduce_documents(normalized_docs)?;

    Ok((reduced_state, raw_observations))
}

