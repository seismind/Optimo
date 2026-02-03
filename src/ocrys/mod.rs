use std::path::Path;
use anyhow::Result;
use tempfile::tempdir;
use std::fs;

pub mod tesseract;
pub mod normalize;
pub mod types;

pub use types::*;

pub fn run_ocr(input: &Path) -> Result<OCRDocument> {
    let dir = tempdir()?;
    let out_base = dir.path().join("out");

    let txt_path = out_base.with_extension("txt");
    let raw = fs::read_to_string(&txt_path)?;

    Ok(normalize::normalize_text(&raw, &input.display().to_string()))
}
