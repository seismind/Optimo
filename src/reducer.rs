use strsim::jaro_winkler;
use crate::ocrys::types::{OCRDocument, OCRLine, OCRPage};
use std::collections::BTreeMap;

const SIM_THRESHOLD: f64 = 0.90;

/// Reduce multiple OCRDocuments (variants) into a single deterministic OCRDocument.
pub fn reduce_documents(mut docs: Vec<OCRDocument>) -> anyhow::Result<OCRDocument> {
    if docs.is_empty() {
        return Err(anyhow::anyhow!("no OCR documents to reduce"));
    }

    // Extract source from first doc (all variants share the same source)
    let source = docs.remove(0).source;

    // Group pages by page_number across variants.
    let mut by_page: BTreeMap<usize, Vec<OCRPage>> = BTreeMap::new();

    for d in docs {
        for p in d.pages {
            by_page.entry(p.page_number).or_default().push(p);
        }
    }

    let mut out_pages: Vec<OCRPage> = Vec::new();
    for (page_number, variants_pages) in by_page {
        let reduced_lines = align_and_vote_lines(&variants_pages);
        out_pages.push(OCRPage {
            page_number,
            lines: reduced_lines,
        });
    }

    Ok(OCRDocument {
        source,
        pages: out_pages,
    })
}

/// Align lines by index ("position") and vote using fuzzy matching.
fn align_and_vote_lines(pages: &[OCRPage]) -> Vec<OCRLine> {
    let max_len = pages
        .iter()
        .map(|p| p.lines.len())
        .max()
        .unwrap_or(0);

    let mut out: Vec<OCRLine> = Vec::new();

    for i in 0..max_len {
        // Collect candidates at position i from each variant.
        let mut candidates: Vec<String> = Vec::new();
        for p in pages {
            if let Some(line) = p.lines.get(i) {
                let t = normalize(&line.text);
                if !t.is_empty() {
                    candidates.push(t);
                }
            }
        }

        if candidates.is_empty() {
            continue;
        }

        // Cluster candidates by fuzzy similarity.
        let clusters = cluster_by_similarity(&candidates, SIM_THRESHOLD);

        // Pick winning cluster: most members, tie-break by longest representative, then lexicographic.
        let winner = pick_winner_cluster(&clusters);

        // Convert back to an OCRLine. (Confidence can be computed later; keep None for now.)
        out.push(OCRLine {
            text: winner,
            confidence: None,
        });
    }

    out
}

/// Simple normalization for matching.
fn normalize(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut cleaned = String::with_capacity(lower.len());
    let mut prev_space = false;

    for ch in lower.chars() {
        let is_ok = ch.is_alphanumeric() || ch.is_whitespace();
        if !is_ok {
            continue;
        }

        if ch.is_whitespace() {
            if !prev_space {
                cleaned.push(' ');
                prev_space = true;
            }
        } else {
            cleaned.push(ch);
            prev_space = false;
        }
    }

    cleaned.trim().to_string()
}

fn cluster_by_similarity(cands: &[String], threshold: f64) -> Vec<Vec<String>> {
    let mut clusters: Vec<Vec<String>> = Vec::new();

    'outer: for c in cands.iter().cloned() {
        for cluster in clusters.iter_mut() {
            // compare against cluster representative (first element)
            let rep = &cluster[0];
            if jaro_winkler(rep, &c) >= threshold {
                cluster.push(c);
                continue 'outer;
            }
        }
        clusters.push(vec![c]);
    }

    clusters
}

fn pick_winner_cluster(clusters: &[Vec<String>]) -> String {
    // Choose best cluster by size, then best representative.
    let mut best_cluster: Option<&Vec<String>> = None;

    for c in clusters {
        best_cluster = match best_cluster {
            None => Some(c),
            Some(b) => {
                if c.len() > b.len() {
                    Some(c)
                } else if c.len() == b.len() {
                    // tie-break: best representative in cluster
                    let cb = best_rep(c);
                    let bb = best_rep(b);
                    if cb.len() > bb.len() {
                        Some(c)
                    } else if cb.len() == bb.len() && cb < bb {
                        Some(c)
                    } else {
                        Some(b)
                    }
                } else {
                    Some(b)
                }
            }
        };
    }

    best_rep(best_cluster.unwrap()).to_string()
}

fn best_rep(cluster: &Vec<String>) -> &str {
    // Representative = longest string, tie-break lexicographic
    cluster
        .iter()
        .max_by(|a, b| {
            a.len()
                .cmp(&b.len())
                .then_with(|| b.cmp(a)) // invert to make lexicographic ascending win after max_by
        })
        .map(|s| s.as_str())
        .unwrap_or("")
}
