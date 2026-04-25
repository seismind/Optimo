use unicode_normalization::UnicodeNormalization;

use super::types::*;
use crate::profile::IngestionProfile;

#[allow(dead_code)]
pub fn normalize_text(raw: &str, source: &str) -> OCRDocument {
    let lines = raw
        .lines()
        .map(|l| OCRLine {
            text: l.trim().to_string(),
            confidence: None,
        })
        .collect();

    OCRDocument {
        source: source.to_string(),
        pages: vec![OCRPage {
            page_number: 1,
            lines,
        }],
    }
}

/// Canonical line normalization — deterministic, loss-free.
///
/// Contract (ordered):
///   1. Unicode NFC        — é = \u00e9, not e + combining accent
///   2. Trim               — strip leading/trailing whitespace
///   3. Decimal comma→dot  — "45,20" → "45.20" (digit,digit only)
///
/// NOT applied here:
///   - lowercase      (preserves semantic case: "TRF" ≠ "trf")
///   - deduplication  (controlled by IngestionProfile / reducer)
///   - field inference (never in this layer)
pub fn normalize_line_canonical(raw: &str) -> String {
    // 1. NFC
    let nfc: String = raw.nfc().collect();
    // 2. Trim
    let trimmed = nfc.trim();
    // 3. Decimal comma → dot (no regex dep: manual scan)
    let chars: Vec<char> = trimmed.chars().collect();
    let mut out = String::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == ',' {
            let prev_digit = i > 0 && chars[i - 1].is_ascii_digit();
            let next_digit = i + 1 < chars.len() && chars[i + 1].is_ascii_digit();
            if prev_digit && next_digit {
                out.push('.');
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Apply canonical normalization to every line in a document.
/// Returns a new OCRDocument; the original is untouched (raw truth preserved by caller).
/// Deprecated in favour of [`normalize_document_with_profile`]; kept for compatibility.
#[allow(dead_code)]
pub fn normalize_document(doc: &OCRDocument) -> OCRDocument {
    OCRDocument {
        source: doc.source.clone(),
        pages: doc
            .pages
            .iter()
            .map(|page| OCRPage {
                page_number: page.page_number,
                lines: page
                    .lines
                    .iter()
                    .map(|l| OCRLine {
                        text: normalize_line_canonical(&l.text),
                        confidence: l.confidence,
                    })
                    .filter(|l| !l.text.is_empty())
                    .collect(),
            })
            .collect(),
    }
}

/// Flatten all lines in a document to a single newline-joined string.
pub fn document_to_text(doc: &OCRDocument) -> String {
    doc.pages
        .iter()
        .flat_map(|p| p.lines.iter().map(|l| l.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Apply profile-driven normalization on top of canonical normalization.
///
/// Steps applied (in order, controlled by each profile flag):
///   1. `unicode_normalize` → NFC  (always included in `normalize_line_canonical`)
///   2. Trim                       (always applied)
///   3. Decimal comma→dot          (always applied — lossless)
///   4. `normalize_whitespace`     → collapse consecutive whitespace to single space
///   5. `normalize_case`           → lowercase (opt-in; disabled for Tesseract/Carbo)
///   6. `min_confidence`           → drop lines below threshold
///
/// Returns a new OCRDocument. The original (raw) document is never mutated.
pub fn normalize_document_with_profile(doc: &OCRDocument, profile: &IngestionProfile) -> OCRDocument {
    OCRDocument {
        source: doc.source.clone(),
        pages: doc
            .pages
            .iter()
            .map(|page| OCRPage {
                page_number: page.page_number,
                lines: page
                    .lines
                    .iter()
                    // confidence filter — drop lines below threshold
                    .filter(|l| {
                        profile.min_confidence <= 0.0
                            || l.confidence
                                .map(|c| c as f64 >= profile.min_confidence)
                                .unwrap_or(true) // no confidence metadata → keep
                    })
                    .map(|l| {
                        // 1-3: canonical (NFC + trim + decimal comma)
                        let mut text = normalize_line_canonical(&l.text);
                        // 4: whitespace collapse
                        if profile.normalize_whitespace {
                            text = collapse_whitespace(&text);
                        }
                        // 5: optional lowercase
                        if profile.normalize_case {
                            text = text.to_lowercase();
                        }
                        OCRLine { text, confidence: l.confidence }
                    })
                    .filter(|l| !l.text.is_empty())
                    .collect(),
            })
            .collect(),
    }
}

/// Collapse consecutive ASCII whitespace characters into a single space.
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_ascii_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::IngestionProfile;

    #[test]
    fn nfc_normalization_applied() {
        // e + combining acute accent → é (NFC)
        let raw = "caf\u{0065}\u{0301}";
        assert_eq!(normalize_line_canonical(raw), "caf\u{00e9}");
    }

    #[test]
    fn decimal_comma_converted_to_dot() {
        assert_eq!(normalize_line_canonical("45,20"), "45.20");
        assert_eq!(normalize_line_canonical("1.234,56"), "1.234.56");
    }

    #[test]
    fn non_decimal_comma_preserved() {
        assert_eq!(normalize_line_canonical("hello, world"), "hello, world");
        assert_eq!(normalize_line_canonical(",20"), ",20");  // no preceding digit
    }

    #[test]
    fn trim_applied() {
        assert_eq!(normalize_line_canonical("  trf 091  "), "trf 091");
    }

    #[test]
    fn case_preserved() {
        assert_eq!(normalize_line_canonical("TRF 0911131517"), "TRF 0911131517");
    }

    #[test]
    fn profile_lowercase_applied_when_enabled() {
        let mut profile = IngestionProfile::tesseract();
        profile.normalize_case = true;
        let doc = make_doc("TRF 0911131517", None);
        let out = normalize_document_with_profile(&doc, &profile);
        assert_eq!(out.pages[0].lines[0].text, "trf 0911131517");
    }

    #[test]
    fn profile_case_preserved_when_disabled() {
        let profile = IngestionProfile::tesseract(); // normalize_case: false
        let doc = make_doc("TRF 0911131517", None);
        let out = normalize_document_with_profile(&doc, &profile);
        assert_eq!(out.pages[0].lines[0].text, "TRF 0911131517");
    }

    #[test]
    fn profile_filters_low_confidence_lines() {
        let profile = IngestionProfile::strict(); // min_confidence: 0.95
        let doc = OCRDocument {
            source: "test".to_string(),
            pages: vec![OCRPage {
                page_number: 1,
                lines: vec![
                    OCRLine { text: "high confidence".to_string(), confidence: Some(0.98) },
                    OCRLine { text: "low confidence".to_string(), confidence: Some(0.40) },
                ],
            }],
        };
        let out = normalize_document_with_profile(&doc, &profile);
        assert_eq!(out.pages[0].lines.len(), 1);
        assert_eq!(out.pages[0].lines[0].text, "high confidence");
    }

    #[test]
    fn profile_collapses_whitespace() {
        let profile = IngestionProfile::tesseract(); // normalize_whitespace: true
        let doc = make_doc("TRF   0911   1517", None);
        let out = normalize_document_with_profile(&doc, &profile);
        assert_eq!(out.pages[0].lines[0].text, "TRF 0911 1517");
    }

    fn make_doc(text: &str, confidence: Option<f32>) -> OCRDocument {
        OCRDocument {
            source: "test".to_string(),
            pages: vec![OCRPage {
                page_number: 1,
                lines: vec![OCRLine { text: text.to_string(), confidence }],
            }],
        }
    }
}
