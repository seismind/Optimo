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
use crate::ocrys::types::{OCRDocument, OCRLine};

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

    Ok(format!(
        r#"{{"source":"{}","decision":"{}","lines":{},"preview":"{}"}}"#,
        reduced_doc.source,
        decision,
        line_count,
        preview
    ))
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
            ocrys::tesseract::run_tesseract(doc, run_dir, lang, variant)
                .with_context(|| format!("OCR failed for variant {}", variant))
        })
        .collect::<Result<Vec<_>>>()?;

    // ---- REDUCE ----
    reduce_documents(docs)
}

/// Deterministic reducer over multiple OCRDocuments
fn reduce_documents(docs: Vec<OCRDocument>) -> Result<OCRDocument> {
    let mut base = docs
        .into_iter()
        .next()
        .context("no OCR documents to reduce")?;

    for page in &mut base.pages {
        page.lines = reduce_lines(std::mem::take(&mut page.lines));
    }

    Ok(base)
}
use std::collections::HashSet;

fn normalize_for_compare(s: &str) -> String {
    s.to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != ' ', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn similarity(a: &str, b: &str) -> f32 {
    let a_tokens: HashSet<_> = a.split_whitespace().collect();
    let b_tokens: HashSet<_> = b.split_whitespace().collect();

    let intersection = a_tokens.intersection(&b_tokens).count() as f32;
    let union = a_tokens.union(&b_tokens).count() as f32;

    if union == 0.0 { 0.0 } else { intersection / union }
}

/// Reduce OCRLine candidates into a single coherent set
fn reduce_lines(lines: Vec<OCRLine>) -> Vec<OCRLine> {
    let mut reduced: Vec<OCRLine> = Vec::new();

    'outer: for line in lines {
        let norm = normalize_for_compare(&line.text);

        for existing in &mut reduced {
            let existing_norm = normalize_for_compare(&existing.text);
            let sim = similarity(&norm, &existing_norm);

            if sim >= 0.7 {
                let new_conf = line.confidence.unwrap_or(0.0);
                let old_conf = existing.confidence.unwrap_or(0.0);

                if new_conf > old_conf {
                    *existing = line;
                }
                continue 'outer;
            }
        }

        reduced.push(line);
    }

    reduced
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


