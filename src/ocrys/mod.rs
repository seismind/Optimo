use std::path::Path;
use anyhow::Result;

pub mod tesseract;
pub mod normalize;
pub mod types;
pub mod preprocess;

pub use types::*;

/// Convenience wrapper that delegates to `tesseract::run_tesseract`.
///
/// Signature mirrors the underlying implementation so callers can provide
/// the `run_dir`, language and a `variant` label to separate artifacts.
pub fn run_ocr(input: &Path, run_dir: &Path, lang: &str, variant: &str) -> Result<OCRDocument> {
    tesseract::run_tesseract(input, run_dir, lang, variant)
}
