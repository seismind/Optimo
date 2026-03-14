// Tokio + Rayon "perfect" boundary:
//
// - Tokio orchestrates *documents* concurrently (async / IO-friendly).
// - For each document we hop into a CPU worker using spawn_blocking.
// - Inside spawn_blocking we use Rayon to parallelize OCR "variants" (Map).
// - Then we normalize/merge (Reduce) deterministically.
// - Finally we decide what deserves to be persisted (StateBridge = final reducer).

use anyhow::{Context, Result};
use rayon::prelude::*;
use std::path::PathBuf;
use tokio::task::JoinSet;

use crate::ocrys;
use crate::state::AppState;
use crate::state_bridge::StateBridge;
use crate::reducer;
use crate::reducer_state::ReducerState;

/// Entry point from main.rs
pub async fn process_documents(state: &AppState, docs: Vec<PathBuf>) -> Result<()> {
    let bridge = StateBridge::new(state);

    let mut set = JoinSet::new();

    for doc in docs {
        let state = state.clone();
        set.spawn(async move {
            process_one_document(&state, doc).await
        });
    }

    while let Some(res) = set.join_next().await {
        let reducer_state = res.context("task join failed")??;
        let snapshot = reducer_state.snapshot();
        bridge.persist_snapshot(&snapshot)?;
    }

    Ok(())
}

async fn process_one_document(state: &AppState, doc: PathBuf) -> Result<ReducerState> {
    // Extract only what the CPU worker needs
    let lang = state.ocr_lang.clone();
    let run_dir = state.ocr_run_dir("latest");

    tokio::fs::create_dir_all(&run_dir).await?;

    // CPU-bound section (Tokio → Rayon boundary)
    let reduced_state = tokio::task::spawn_blocking(move || {
        cpu_map_reduce_ocr(&doc, &run_dir, &lang)
    })
    .await?
    ?;

    Ok(reduced_state)
}

/// MAP + REDUCE (CPU-bound)
fn cpu_map_reduce_ocr(
    doc: &PathBuf,
    run_dir: &PathBuf,
    lang: &str,
) -> Result<ReducerState> {
    // ---- MAP ----
    let variants = vec![
        "original",
        "high_contrast",
        "rotated",
    ];

    let docs: Vec<crate::ocrys::types::OCRDocument> = variants
        .par_iter()
        .map(|variant| {
            ocrys::run_ocr(doc, run_dir, lang, variant)
                .with_context(|| format!("OCR failed for variant {}", variant))
        })
        .collect::<Result<Vec<_>>>()?;

    // ---- REDUCE (delegated to reducer module) ----
    reducer::reduce_documents(docs)
}

