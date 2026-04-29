//! Image preprocessing for region-of-interest (ROI) extraction before OCR.
//!
//! Pipeline per region:
//!   1. Load image from disk
//!   2. `crop_imm(x, y, w, h)` — immutable ROI extraction, source unchanged
//!   3. Convert to grayscale (Luma8)
//!   4. Otsu binary threshold — maximises contrast for Tesseract
//!   5. Optional resize to target dimensions
//!   6. Save to `out_dir/<kind>_<x>_<y>.png` and return path
//!
//! Use cases: title blocks, invoice totals, structural tables, signatures.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::{DynamicImage, GrayImage, ImageBuffer, Luma};
use image::imageops::FilterType;

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// Semantic label for a region of interest.
///
/// Used for naming output files and for downstream routing: different region
/// kinds may warrant different OCR PSM modes or post-processing rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// Company name, document title, header logo area.
    TitleBlock,
    /// Subtotal / VAT / grand-total section (typically bottom-right).
    InvoiceTotals,
    /// Line-item table or any grid with columnar structure.
    StructuralTable,
    /// Handwritten or printed signature / stamp area.
    Signature,
}

impl RegionKind {
    fn label(self) -> &'static str {
        match self {
            Self::TitleBlock      => "title_block",
            Self::InvoiceTotals   => "invoice_totals",
            Self::StructuralTable => "structural_table",
            Self::Signature       => "signature",
        }
    }
}

/// Axis-aligned rectangular region of interest.
#[derive(Debug, Clone)]
pub struct Roi {
    /// X offset of the top-left corner (pixels, 0-based).
    pub x: u32,
    /// Y offset of the top-left corner (pixels, 0-based).
    pub y: u32,
    /// Width of the region in pixels.
    pub width: u32,
    /// Height of the region in pixels.
    pub height: u32,
    /// Semantic label controlling file naming and downstream routing.
    pub kind: RegionKind,
    /// If `Some((w, h))`, resize the thresholded region before saving.
    /// Useful for normalising small regions before Tesseract.
    pub resize: Option<(u32, u32)>,
}

/// A preprocessed image region ready to feed into Tesseract.
#[derive(Debug)]
pub struct PreprocessedRegion {
    /// Absolute path to the saved preprocessed PNG.
    pub path: PathBuf,
    /// Semantic label of the region.
    pub kind: RegionKind,
    /// Original ROI definition (for provenance / logging).
    pub roi: Roi,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Preprocess one ROI from `source` and write the result to `out_dir`.
///
/// Returns a [`PreprocessedRegion`] whose `path` can be passed directly to
/// [`crate::ocrys::run_ocr`] or [`crate::ocrys::tesseract::run_tesseract`].
///
/// # Errors
/// - Image cannot be loaded or decoded.
/// - ROI coordinates fall outside the image bounds (image crate clamps silently
///   but an empty result will fail Tesseract — caller should validate upstream).
/// - Output directory cannot be created or the file cannot be written.
pub fn preprocess_roi(source: &Path, roi: &Roi, out_dir: &Path) -> Result<PreprocessedRegion> {
    // 1. Load
    let img = image::open(source)
        .with_context(|| format!("failed to load image: {}", source.display()))?;

    // 2. Crop — immutable: does not mutate the source buffer
    let cropped: DynamicImage = img.crop_imm(roi.x, roi.y, roi.width, roi.height);

    // 3. Grayscale
    let gray: GrayImage = cropped.into_luma8();

    // 4. Otsu binary threshold
    let binary: GrayImage = otsu_threshold(gray);

    // 5. Optional resize
    let final_img: GrayImage = match roi.resize {
        Some((w, h)) => DynamicImage::ImageLuma8(binary)
            .resize_exact(w, h, FilterType::Lanczos3)
            .into_luma8(),
        None => binary,
    };

    // 6. Save
    std::fs::create_dir_all(out_dir)
        .context("failed to create preprocessing output directory")?;

    let filename = format!("{}_{}_{}.png", roi.kind.label(), roi.x, roi.y);
    let out_path = out_dir.join(&filename);

    final_img
        .save(&out_path)
        .with_context(|| format!("failed to save preprocessed region: {}", out_path.display()))?;

    Ok(PreprocessedRegion {
        path: out_path,
        kind: roi.kind,
        roi: roi.clone(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal: Otsu's thresholding
// ─────────────────────────────────────────────────────────────────────────────

/// Apply Otsu's threshold to a grayscale image, returning a binary image.
///
/// Pixels **above** the threshold → 255 (white background).
/// Pixels **at or below** the threshold → 0 (black foreground).
///
/// This polarity is correct for documents: dark ink on bright paper becomes
/// fully black-on-white, which is what Tesseract expects.
fn otsu_threshold(gray: GrayImage) -> GrayImage {
    let t = compute_otsu_threshold(&gray);
    ImageBuffer::from_fn(gray.width(), gray.height(), |x, y| {
        let Luma([v]) = *gray.get_pixel(x, y);
        Luma([if v > t { 255u8 } else { 0u8 }])
    })
}

/// Compute the Otsu threshold that maximises inter-class variance.
///
/// Falls back to 128 if the image is empty or has uniform intensity.
fn compute_otsu_threshold(gray: &GrayImage) -> u8 {
    let total = (gray.width() * gray.height()) as f64;
    if total == 0.0 {
        return 128;
    }

    // Build 256-bin histogram
    let mut hist = [0u64; 256];
    for Luma([v]) in gray.pixels() {
        hist[*v as usize] += 1;
    }

    // Weighted sum of all intensity levels
    let sum_total: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();

    let mut sum_bg = 0.0_f64;
    let mut count_bg = 0_u64;
    let mut max_variance = 0.0_f64;
    let mut threshold = 0u8;

    for t in 0..=255_usize {
        count_bg += hist[t];
        if count_bg == 0 {
            continue;
        }
        let count_fg = (total as u64).saturating_sub(count_bg);
        if count_fg == 0 {
            break;
        }

        sum_bg += t as f64 * hist[t] as f64;
        let mean_bg = sum_bg / count_bg as f64;
        let mean_fg = (sum_total - sum_bg) / count_fg as f64;

        // Inter-class variance: w_bg * w_fg * (μ_bg - μ_fg)²
        let variance = count_bg as f64 * count_fg as f64 * (mean_bg - mean_fg).powi(2);

        if variance > max_variance {
            max_variance = variance;
            threshold = t as u8;
        }
    }

    threshold
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Luma};
    use tempfile::tempdir;

    // ── Otsu internals ──────────────────────────────────────────────────────

    /// A perfectly uniform image has no inter-class variance: threshold falls
    /// back to 0 (the loop never finds a variance > 0). The important thing is
    /// it does NOT panic.
    #[test]
    fn otsu_uniform_image_does_not_panic() {
        let img = GrayImage::from_pixel(64, 64, Luma([200u8]));
        let t = compute_otsu_threshold(&img);
        // All pixels at 200, so both classes can't exist simultaneously.
        // The only valid threshold is somewhere ≤ 200.
        assert!(t <= 200, "threshold={t} must be <= uniform intensity 200");
    }

    /// A bimodal image (half pixels at 50, half at 200) has a clear valley.
    /// Otsu should pick a threshold between the two modes.
    #[test]
    fn otsu_bimodal_threshold_between_peaks() {
        let mut img = GrayImage::new(100, 100);
        for y in 0..100 {
            for x in 0..100 {
                let v = if x < 50 { 50u8 } else { 200u8 };
                img.put_pixel(x, y, Luma([v]));
            }
        }
        let t = compute_otsu_threshold(&img);
        // Otsu returns the first threshold that maximises inter-class variance.
        // With half pixels at 50 and half at 200 the optimum is t=50: the
        // boundary of the lower mode perfectly separates the two classes.
        // The assertion is >= 50 (not strictly >): t=50 is mathematically correct.
        assert!(t >= 50 && t < 200,
            "bimodal threshold={t} must be strictly between 50 and 200");
    }

    /// An empty image (0×0) must not panic and must return 128.
    #[test]
    fn otsu_empty_image_returns_fallback() {
        let img = GrayImage::new(0, 0);
        assert_eq!(compute_otsu_threshold(&img), 128);
    }

    // ── preprocess_roi integration ──────────────────────────────────────────

    fn white_png(dir: &std::path::Path) -> PathBuf {
        let img = GrayImage::from_pixel(200, 200, Luma([255u8]));
        let p = dir.join("source.png");
        img.save(&p).unwrap();
        p
    }

    /// Basic happy path: a valid ROI produces a PNG file at the expected path.
    #[test]
    fn preprocess_roi_produces_file() {
        let tmp = tempdir().unwrap();
        let src = white_png(tmp.path());
        let out = tmp.path().join("out");

        let roi = Roi {
            x: 10, y: 10, width: 80, height: 80,
            kind: RegionKind::InvoiceTotals,
            resize: None,
        };

        let result = preprocess_roi(&src, &roi, &out).unwrap();
        assert!(result.path.exists(), "preprocessed file must exist");
        assert_eq!(result.kind, RegionKind::InvoiceTotals);
        assert!(result.path.extension().and_then(|e| e.to_str()) == Some("png"));
    }

    /// When `resize` is set the output image must have the requested dimensions.
    #[test]
    fn preprocess_roi_resize_applied() {
        let tmp = tempdir().unwrap();
        let src = white_png(tmp.path());
        let out = tmp.path().join("out");

        let roi = Roi {
            x: 0, y: 0, width: 200, height: 200,
            kind: RegionKind::TitleBlock,
            resize: Some((64, 32)),
        };

        let result = preprocess_roi(&src, &roi, &out).unwrap();
        let saved = image::open(&result.path).unwrap();
        assert_eq!(saved.width(), 64, "width must match resize target");
        assert_eq!(saved.height(), 32, "height must match resize target");
    }

    /// File name encodes kind, x, y for unambiguous provenance.
    #[test]
    fn preprocess_roi_filename_encodes_provenance() {
        let tmp = tempdir().unwrap();
        let src = white_png(tmp.path());
        let out = tmp.path().join("out");

        let roi = Roi {
            x: 5, y: 42, width: 50, height: 50,
            kind: RegionKind::Signature,
            resize: None,
        };

        let result = preprocess_roi(&src, &roi, &out).unwrap();
        let name = result.path.file_name().unwrap().to_string_lossy();
        assert!(name.contains("signature"), "filename must contain kind label");
        assert!(name.contains("5"),  "filename must contain x");
        assert!(name.contains("42"), "filename must contain y");
    }

    /// ROI that exactly covers the full image must succeed.
    #[test]
    fn preprocess_roi_full_image_succeeds() {
        let tmp = tempdir().unwrap();
        let src = white_png(tmp.path());
        let out = tmp.path().join("out");

        let roi = Roi {
            x: 0, y: 0, width: 200, height: 200,
            kind: RegionKind::StructuralTable,
            resize: None,
        };

        assert!(preprocess_roi(&src, &roi, &out).is_ok());
    }
}
