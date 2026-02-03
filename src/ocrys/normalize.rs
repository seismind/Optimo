use super::types::*;

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
