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
use serde_json;

use crate::ocrys;
use crate::state::AppState;
use crate::state_bridge::StateBridge;
use crate::ocrys::types::OCRDocument;
use crate::reducer;

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
        let observation = res.context("task join failed")??;
        bridge.record_jsonl(&observation)?;
    }

    Ok(())
}

async fn process_one_document(state: &AppState, doc: PathBuf) -> Result<String> {
    // Extract only what the CPU worker needs
    let lang = state.ocr_lang.clone();
    let run_dir = state.ocr_run_dir("latest");

    tokio::fs::create_dir_all(&run_dir).await?;

    // CPU-bound section (Tokio → Rayon boundary)
    let reduced_doc = tokio::task::spawn_blocking(move || {
        cpu_map_reduce_ocr(&doc, &run_dir, &lang)
    })
    .await?
    ?;

    // ---- FINAL REDUCE: decide what survives ----
    let line_count: usize = reduced_doc
        .pages
        .iter()
        .map(|p| p.lines.len())
        .sum();

    let decision = if line_count == 0 {
        "empty"
    } else {
        "ocr_converged"
    };

    let preview = preview_document(&reduced_doc, 200);

    Ok(serde_json::to_string(&serde_json::json!({
        "source": reduced_doc.source,
        "decision": decision,
        "lines": line_count,
        "preview": preview
    }))?)
}

/// MAP + REDUCE (CPU-bound)
fn cpu_map_reduce_ocr(
    doc: &PathBuf,
    run_dir: &PathBuf,
    lang: &str,
) -> Result<OCRDocument> {
    // ---- MAP ----
    let variants = vec![
        "original",
        "high_contrast",
        "rotated",
    ];

    let docs: Vec<OCRDocument> = variants
        .par_iter()
        .map(|variant| {
            ocrys::run_ocr(doc, run_dir, lang, variant)
                .with_context(|| format!("OCR failed for variant {}", variant))
        })
        .collect::<Result<Vec<_>>>()?;

    // ---- REDUCE (delegated to reducer module) ----
    reducer::reduce_documents(docs)
}

/// Small preview helper
fn preview_document(doc: &OCRDocument, max: usize) -> String {
    let mut s = String::new();

    for page in &doc.pages {
        for line in &page.lines {
            s.push_str(&line.text);
            s.push(' ');
            if s.len() >= max {
                s.truncate(max);
                s.push('…');
                return sanitize(&s);
            }
        }
    }

    sanitize(&s)
}

fn sanitize(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
        .replace('\r', " ")
}
