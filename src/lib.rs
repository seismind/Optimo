/// Optimo: deterministic OCR document reducer.
///
/// This library provides:
/// - OCR variant management (original, high_contrast, rotated)
/// - Deterministic text reduction via clustering
/// - Confidence scoring and convergence detection
/// - Event replay and snapshot persistence

pub mod config;
pub mod event;
pub mod observation;
pub mod ocrys;
pub mod profile;
pub mod fold;
pub mod aggregate_state;
pub mod timequake;
pub mod snapshot;
pub mod app_state;
pub mod persistence;
pub mod pipeline;
pub mod fold_properties;
pub mod fold_adversarial;
pub mod operational_policy;
