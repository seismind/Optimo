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
use std::collections::BTreeMap;
use std::path::PathBuf;
use tokio::task::{spawn_blocking, JoinSet};
use uuid::Uuid;

use crate::aggregate_state::ReducerState;
use crate::app_state::AppState;
use crate::fold;
use crate::ocrys;
use crate::ocrys::normalize;
use crate::ocrys::preprocess;
use crate::ocrys::types::OCRDocument;
use crate::persistence::{StateBridge, SqliteStore};
use crate::profile::IngestionProfile;
use crate::roi_analysis;
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
        let (reducer_state, raw_observations, roi_plan) = res.context("task join failed")??;

        eprintln!(
            "[optimo] source={} iterations={} convergence={} ambiguity={} collision_rate={} semantic_conflicts={} (neg={} num={})",
            reducer_state.source,
            reducer_state.iterations,
            reducer_state.convergence_score_bps,
            reducer_state.ambiguity_score_bps,
            reducer_state.collision_rate_bps,
            reducer_state.semantic_conflict_count,
            reducer_state.negation_conflicts,
            reducer_state.numeric_conflicts,
        );

        if let Some(plan) = &roi_plan {
            eprintln!("[optimo] ROI_PLAN regions={} atoms={}", plan.merged_regions.len(), plan.atomic_count);
        }

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

async fn process_one_document(state: &AppState, doc: PathBuf) -> Result<(ReducerState, Vec<RawObservation>, Option<roi_analysis::ROIPlan>)> {
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
///   - Option<ROIPlan> if numeric conflicts detected (PASS 2 execution plan)
fn cpu_map_reduce_ocr(
    doc: &PathBuf,
    run_dir: &PathBuf,
    lang: &str,
    profile: &IngestionProfile,
) -> Result<(ReducerState, Vec<RawObservation>, Option<roi_analysis::ROIPlan>)> {
    let source = doc.to_string_lossy().to_string();
    // document_id must match what ReducerState will derive from the source path.
    let document_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, source.as_bytes());
    let now = Utc::now();

    let variants = vec!["original", "high_contrast", "rotated"];

    // ---- MAP ----
    // Each variant applies a different perceptual transformation before OCR so
    // the reducer receives genuinely different inputs and produces a real
    // convergence score rather than a trivial 10000.
    let pairs: Vec<(OCRDocument, OCRDocument, String)> =
        variants
            .par_iter()
            .map(|variant| {
                // 1. Preprocess: transform the image for this variant.
                let (preprocessed_path, metrics) =
                    preprocess::preprocess_for_variant(doc, variant, run_dir)
                        .with_context(|| format!("preprocessing failed for variant {}", variant))?;

                eprintln!(
                    "[optimo] preprocess variant={} orig={}x{} roi={}x{} threshold={:?} resize={:?}",
                    variant,
                    metrics.original_dimensions.0, metrics.original_dimensions.1,
                    metrics.roi_dimensions.0,      metrics.roi_dimensions.1,
                    metrics.threshold_used,
                    metrics.resize_factor,
                );

                // 2. Run OCR on the preprocessed image (not the original).
                let mut raw_doc = ocrys::run_ocr(&preprocessed_path, run_dir, lang, variant)
                    .with_context(|| format!("OCR failed for variant {}", variant))?;

                // Keep reducer identity tied to the original input document,
                // not to transient run_dir artifacts (preproc_*.png).
                raw_doc.source = source.clone();

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
    let mut reduced_state = fold::reduce_documents(normalized_docs)?;

    // ---- PASS 1 → PASS 2: Generate ROIPlan for numeric conflicts ----
    let roi_plan = if reduced_state.numeric_conflicts > 0 {
        // Extract rehydration state (observations) from reducer.
        let rehydration = reduced_state.rehydration_state();
        // Observe numeric conflict zones.
        let atomics = roi_analysis::observe_numeric_conflicts(&rehydration);
        if !atomics.is_empty() {
            // Merge into execution plan.
            let padding = 2; // lines to add above/below conflict
            let plan = roi_analysis::generate_roi_plan(
                document_id,
                atomics,
                rehydration.source.clone(),
                padding,
            );
            Some(plan)
        } else {
            None
        }
    } else {
        None
    };

    // ---- PASS 3: Execute ROI plan and merge targeted refinement ----
    if let Some(plan) = &roi_plan {
        if !plan.merged_regions.is_empty() {
            let sparse_refinement = execute_roi_refinement(
                doc,
                run_dir,
                lang,
                profile,
                plan,
                &source,
            )?;
            reduced_state.update_from_document(sparse_refinement);
        }
    }

    Ok((reduced_state, raw_observations, roi_plan))
}

/// PASS 3 execution:
/// - run OCR deterministically for each preprocess hint used in the ROI plan
/// - extract only targeted line ranges from those OCR outputs
/// - build a sparse document preserving original line indices
/// - return the sparse document so reducer can merge refined observations
fn execute_roi_refinement(
    doc: &PathBuf,
    run_dir: &PathBuf,
    lang: &str,
    profile: &IngestionProfile,
    plan: &roi_analysis::ROIPlan,
    source: &str,
) -> Result<OCRDocument> {
    // Cache one OCR document per variant used by the plan.
    let mut variant_docs: BTreeMap<String, OCRDocument> = BTreeMap::new();

    for region in &plan.merged_regions {
        let variant = variant_for_hint(&region.preprocess_hint);
        if variant_docs.contains_key(variant) {
            continue;
        }

        let variant_label = format!("{}_roi", variant);
        let (preprocessed_path, _) = preprocess::preprocess_for_variant(doc, variant, run_dir)
            .with_context(|| format!("preprocessing failed for ROI variant {}", variant))?;
        let mut raw_doc = ocrys::run_ocr(&preprocessed_path, run_dir, lang, &variant_label)
            .with_context(|| format!("ROI OCR failed for variant {}", variant))?;
        raw_doc.source = source.to_string();
        let normalized_doc = normalize::normalize_document_with_profile(&raw_doc, profile);
        variant_docs.insert(variant.to_string(), normalized_doc);
    }

    // Sparse page buffers: page_number -> vec[line_text], keeping 1-indexed line positions.
    let mut page_buffers: BTreeMap<usize, Vec<String>> = BTreeMap::new();

    for region in &plan.merged_regions {
        let variant = variant_for_hint(&region.preprocess_hint);
        let Some(doc_for_variant) = variant_docs.get(variant) else {
            continue;
        };

        let page_number = region.page as usize;
        let Some(src_page) = doc_for_variant.pages.iter().find(|p| p.page_number == page_number) else {
            continue;
        };

        let start = region.line_range.0.max(1) as usize;
        let end = (region.line_range.1 as usize).min(src_page.lines.len());
        if start > end {
            continue;
        }

        let buffer = page_buffers.entry(page_number).or_default();
        if buffer.len() < end {
            buffer.resize(end, String::new());
        }

        for idx in start..=end {
            let candidate = src_page.lines[idx - 1].text.clone();
            if !candidate.is_empty() {
                // Keep deterministic first-write semantics per line position.
                if buffer[idx - 1].is_empty() {
                    buffer[idx - 1] = candidate;
                }
            }
        }
    }

    let pages = page_buffers
        .into_iter()
        .map(|(page_number, lines)| crate::ocrys::types::OCRPage {
            page_number,
            lines: lines
                .into_iter()
                .map(|text| crate::ocrys::types::OCRLine { text, confidence: None })
                .collect(),
        })
        .collect();

    Ok(OCRDocument {
        source: source.to_string(),
        pages,
    })
}

fn variant_for_hint(hint: &roi_analysis::PreprocessHint) -> &'static str {
    match hint {
        roi_analysis::PreprocessHint::HighContrast => "high_contrast",
        roi_analysis::PreprocessHint::Original => "original",
        roi_analysis::PreprocessHint::Rotated => "rotated",
    }
}

