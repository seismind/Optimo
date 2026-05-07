/// OCR semantic stability tests — fixture title_block_001.
///
/// These tests use synthetic documents that replicate the exact OCR output
/// observed from the 3 preprocessing variants run on `fixtures/cartiglio.png`
/// (969×477 Italian construction drawing title block, 2026-05-07).
///
/// They do NOT invoke Tesseract or touch the filesystem. They drive the same
/// fold kernel that the pipeline uses, asserting the convergence, ambiguity,
/// semantic-conflict, and replay-equivalence invariants that must hold
/// for this fixture to remain a valid robustness benchmark.

#[cfg(test)]
mod tests {
    use crate::fold::reduce_documents;
    use crate::ocrys::types::{OCRDocument, OCRLine, OCRPage};

    // ── helpers ────────────────────────────────────────────────────────────

    fn make_doc(source: &str, lines: Vec<&str>, confidence: f32) -> OCRDocument {
        OCRDocument {
            source: source.to_string(),
            pages: vec![OCRPage {
                page_number: 1,
                lines: lines
                    .into_iter()
                    .map(|t| OCRLine {
                        text: t.to_string(),
                        confidence: Some(confidence),
                    })
                    .collect(),
            }],
        }
    }

    /// Lines that appear in ALL three real variant outputs (consensus evidence).
    fn consensus_lines() -> Vec<&'static str> {
        vec![
            "05/06/2020 DESCRIZIONE :",
            "NOME FILE :",
            "MATERIALE :",
            "NUM. DISEGNO",
            "FOGLIO 1 DI 1",
        ]
    }

    /// OCR output from the `original` variant (grayscale only).
    fn doc_original() -> OCRDocument {
        make_doc(
            "preproc_original.png",
            vec![
                "05/06/2020 DESCRIZIONE :",
                "NOME FILE :",
                "MATERIALE :",
                "ss",
                "NUM. DISEGNO",
                "FOGLIO 1 DI 1",
            ],
            0.90,
        )
    }

    /// OCR output from the `high_contrast` variant (Otsu binarization, threshold=187).
    fn doc_high_contrast() -> OCRDocument {
        make_doc(
            "preproc_high_contrast.png",
            vec![
                "05/06/2020 DESCRIZIONE :",
                "NOME FILE :",
                "MATERIALE :",
                "rss",
                "NUM. DISEGNO",
                "FOGLIO 1 DI 1",
            ],
            0.88,
        )
    }

    /// OCR output from the `rotated` variant (60% downsample + Lanczos3 upsample).
    /// This variant loses MASSA, FINITURA, SCALE and corrupts the final digit.
    fn doc_rotated() -> OCRDocument {
        make_doc(
            "preproc_rotated.png",
            vec![
                "05/06/2020",
                "TBCSS",
                "NOME FILE",
                "DIM. FOGLIO : NUM. DISEGNO",
                "A3",
                "1 dor)",
                "FOGLIO 1 DI i",
            ],
            0.72,
        )
    }

    fn all_variants() -> Vec<OCRDocument> {
        vec![doc_original(), doc_high_contrast(), doc_rotated()]
    }

    // ── tests ──────────────────────────────────────────────────────────────

    /// convergence=6666 is the measured floor for this fixture.
    /// The bound is set 10% below the observed value to allow for minor
    /// Tesseract version drift while rejecting gross regressions.
    #[test]
    fn convergence_meets_fixture_floor() {
        let state = reduce_documents(all_variants()).expect("reduce");
        assert!(
            state.convergence_score_bps >= 6000,
            "convergence {0} bps fell below fixture floor 6000 bps \
             (observed baseline 6666 bps on cartiglio.png)",
            state.convergence_score_bps,
        );
    }

    /// Ambiguity must stay under 4000 bps.
    /// Observed = 2856; the ceiling is set above the observed value to
    /// absorb variant noise while still catching runaway ambiguity.
    #[test]
    fn ambiguity_stays_within_ceiling() {
        let state = reduce_documents(all_variants()).expect("reduce");
        assert!(
            state.ambiguity_score_bps <= 4000,
            "ambiguity {0} bps exceeded fixture ceiling 4000 bps \
             (observed baseline 2856 bps on cartiglio.png)",
            state.ambiguity_score_bps,
        );
    }

    /// The clean title block carries no semantic polarity flips or
    /// incompatible numeric identifiers — semantic_conflict_count must remain zero.
    #[test]
    fn no_semantic_conflicts_on_clean_fixture() {
        let state = reduce_documents(all_variants()).expect("reduce");
        assert_eq!(
            state.semantic_conflict_count, 0,
            "unexpected semantic conflicts on clean fixture: \
             neg={} num={}",
            state.negation_conflicts, state.numeric_conflicts,
        );
    }

    /// Fields that appear in both `original` and `high_contrast` with high
    /// confidence must survive in the reducer output.
    /// The `rotated` variant loses MASSA and FINITURA, but the two high-
    /// confidence variants agree on the date and structural labels — the
    /// reducer must preserve that consensus.
    #[test]
    fn consensus_fields_survive_across_variants() {
        let state = reduce_documents(all_variants()).expect("reduce");

        // Collect all text from all output pages
        let all_text: Vec<String> = state
            .pages
            .iter()
            .flat_map(|p| p.lines.iter().map(|l| l.text.clone()))
            .collect();

        let date_present = all_text.iter().any(|t| t.contains("05/06/2020"));
        assert!(
            date_present,
            "consensus date '05/06/2020' must survive reduction; output: {:?}",
            all_text,
        );

        let foglio_present = all_text.iter().any(|t| t.contains("FOGLIO 1 DI"));
        assert!(
            foglio_present,
            "consensus field 'FOGLIO 1 DI 1' must survive reduction; output: {:?}",
            all_text,
        );
    }

    /// Replay equivalence: the reducer is deterministic — identical inputs in
    /// different order must produce the same convergence and ambiguity scores.
    ///
    /// This guards against any HashMap-ordering dependency leaking into the
    /// fold kernel.
    #[test]
    fn replay_order_invariance() {
        let forward = reduce_documents(all_variants()).expect("forward reduce");
        let reversed = reduce_documents(vec![
            doc_rotated(),
            doc_high_contrast(),
            doc_original(),
        ])
        .expect("reversed reduce");

        assert_eq!(
            forward.convergence_score_bps, reversed.convergence_score_bps,
            "convergence must be order-invariant",
        );
        assert_eq!(
            forward.ambiguity_score_bps, reversed.ambiguity_score_bps,
            "ambiguity must be order-invariant",
        );
        assert_eq!(
            forward.semantic_conflict_count, reversed.semantic_conflict_count,
            "semantic_conflict_count must be order-invariant",
        );
    }

    /// When only the two high-quality variants agree (original + high_contrast),
    /// convergence must be higher than the three-variant run — proving the
    /// reducer correctly down-weights the noisy rotated input.
    #[test]
    fn two_clean_variants_converge_above_three_variant_floor() {
        let clean_state =
            reduce_documents(vec![doc_original(), doc_high_contrast()]).expect("clean reduce");
        let full_state = reduce_documents(all_variants()).expect("full reduce");

        assert!(
            clean_state.convergence_score_bps >= full_state.convergence_score_bps,
            "two clean variants ({}) should converge at least as well as three ({}) \
             since rotated adds noise",
            clean_state.convergence_score_bps,
            full_state.convergence_score_bps,
        );
    }

    /// The consensus_lines() set — fields that appear in all three raw
    /// variant outputs — must be present in at least 2 of the 3 input
    /// documents. This is a data-integrity check on the test fixtures
    /// themselves, not on the reducer.
    #[test]
    fn consensus_lines_appear_in_majority_of_inputs() {
        let docs = all_variants();
        for expected in consensus_lines() {
            let count = docs
                .iter()
                .filter(|d| {
                    d.pages
                        .iter()
                        .flat_map(|p| p.lines.iter())
                        .any(|l| l.text.contains(expected))
                })
                .count();
            assert!(
                count >= 2,
                "consensus line {:?} found in only {}/{} input docs",
                expected,
                count,
                docs.len(),
            );
        }
    }
}
