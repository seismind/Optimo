use std::process::Command;
use std::path::{Path, PathBuf};
use std::fs;

use anyhow::{Result, Context};

use crate::ocrys::types::{OCRDocument, OCRPage, OCRLine};

pub fn run_tesseract(
    input: &Path,
    run_dir: &Path,
    lang: &str,
    variant: &str,
) -> Result<OCRDocument> {
    // Output base path WITHOUT extension (tesseract adds .txt)
    let output_base: PathBuf = run_dir.join(format!("ocr_{}", variant));

    let status = Command::new("tesseract")
        .arg(input)
        .arg(&output_base)
        .arg("-l")
        .arg(lang)
        .status()
        .context("failed to start tesseract")?;

    if !status.success() {
        anyhow::bail!("tesseract failed with status {:?}", status);
    }

    // Read generated text file
    let txt_path = output_base.with_extension("txt");
    let raw_text = fs::read_to_string(&txt_path)
        .with_context(|| format!("failed to read {:?}", txt_path))?;

    Ok(text_to_document(&raw_text, input))
}

/// Very first normalization step:
/// - split lines
/// - no geometry
/// - no confidence yet
fn text_to_document(raw: &str, source: &Path) -> OCRDocument {
    let lines = raw
        .lines()
        .map(|l| OCRLine {
            text: l.trim().to_string(),
            confidence: None,
        })
        .filter(|l| !l.text.is_empty())
        .collect();

    OCRDocument {
        source: source.to_string_lossy().to_string(),
        pages: vec![OCRPage {
            page_number: 1,
            lines,
        }],
    }
}
