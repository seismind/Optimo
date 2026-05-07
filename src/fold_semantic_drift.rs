/// Semantic-drift stress tests under OCR-like noise.
///
/// These tests focus on meaning preservation for critical business fields
/// (amounts, IDs, negations, line alignment), not just string similarity.

#[cfg(test)]
mod tests {
    use crate::fold;
    use crate::ocrys::types::{OCRDocument, OCRLine, OCRPage};

    fn doc_with_lines(source: &str, lines: Vec<&str>, confidence: f32) -> OCRDocument {
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

    fn page_line_text(state: &crate::aggregate_state::AggregateState, page: usize, line: usize) -> String {
        state
            .pages
            .iter()
            .find(|p| p.page_number == page)
            .and_then(|p| p.lines.get(line - 1))
            .map(|l| l.text.clone())
            .unwrap_or_default()
    }

    #[test]
    fn semantic_total_amount_stable_under_ocr_noise() {
        let docs = vec![
            doc_with_lines("file://tot_1.png", vec!["Totale EUR 1.250,00"], 0.95),
            doc_with_lines("file://tot_2.png", vec!["Totale EUR 1.250,00"], 0.92),
            doc_with_lines("file://tot_3.png", vec!["Totale EUR l.250,00"], 0.75),
            doc_with_lines("file://tot_4.png", vec!["Totale EUR 1,250.00"], 0.80),
            doc_with_lines("file://tot_5.png", vec!["Totale EUR 1 250,00"], 0.70),
        ];

        let state = fold::reduce_documents(docs).expect("reduce");
        let winner = page_line_text(&state, 1, 1);

        assert!(winner.contains("1.250.00") || winner.contains("1.250,00") || winner.contains("1 250,00"));
        assert!(state.convergence_score_bps >= 6400,
            "amount line should stay stable under mild OCR drift: {}", state.convergence_score_bps);
    }

    #[test]
    fn semantic_negation_flip_raises_ambiguity() {
        let docs = vec![
            doc_with_lines("file://neg_1.png", vec!["Stato: pagato"], 0.95),
            doc_with_lines("file://neg_2.png", vec!["Stato: pagato"], 0.90),
            doc_with_lines("file://neg_3.png", vec!["Stato: non pagato"], 0.92),
            doc_with_lines("file://neg_4.png", vec!["Stato: non pagato"], 0.88),
        ];

        let state = fold::reduce_documents(docs).expect("reduce");
        assert!(state.ambiguity_score_bps >= 4500,
            "polarity flip must surface as ambiguity: {}", state.ambiguity_score_bps);
    }

    #[test]
    fn semantic_identifier_one_char_drift_not_silently_converged() {
        let docs = vec![
            doc_with_lines("file://id_1.png", vec!["PIVA: 12345678901"], 0.94),
            doc_with_lines("file://id_2.png", vec!["PIVA: 12345678901"], 0.93),
            doc_with_lines("file://id_3.png", vec!["PIVA: 12345678907"], 0.92),
            doc_with_lines("file://id_4.png", vec!["PIVA: 12345678907"], 0.91),
        ];

        let state = fold::reduce_documents(docs).expect("reduce");
        assert!(state.ambiguity_score_bps >= 4500,
            "ID one-char drift should not look fully converged: {}", state.ambiguity_score_bps);
    }

    #[test]
    fn semantic_table_row_shift_lowers_convergence() {
        let doc_a = doc_with_lines("file://row_1.png", vec!["Voce A 100", "Voce B 200"], 0.95);
        let doc_b = doc_with_lines("file://row_2.png", vec!["Voce A 100", "Voce B 200"], 0.94);
        // OCR row shift in second variant
        let doc_c = doc_with_lines("file://row_3.png", vec!["Voce B 200", "Voce A 100"], 0.92);

        let state = fold::reduce_documents(vec![doc_a, doc_b, doc_c]).expect("reduce");
        assert!(state.convergence_score_bps <= 8000,
            "row shift must penalize convergence: {}", state.convergence_score_bps);
        assert!(state.ambiguity_score_bps >= 2000,
            "row shift should raise ambiguity: {}", state.ambiguity_score_bps);
    }

    #[test]
    fn semantic_drift_with_clean_majority_keeps_expected_winner() {
        let docs = vec![
            doc_with_lines("file://clean_1.png", vec!["CIG: Z123ABC456"], 0.95),
            doc_with_lines("file://clean_2.png", vec!["CIG: Z123ABC456"], 0.94),
            doc_with_lines("file://clean_3.png", vec!["CIG: Z123ABC456"], 0.93),
            doc_with_lines("file://noise_1.png", vec!["CIG: ZI23ABC456"], 0.80),
            doc_with_lines("file://noise_2.png", vec!["CIG: Z123A8C456"], 0.78),
        ];

        let state = fold::reduce_documents(docs).expect("reduce");
        let winner = page_line_text(&state, 1, 1);
        assert!(winner.contains("Z123ABC456"), "clean majority should win: {winner}");
    }

    #[test]
    fn semantic_replay_stability_under_fixed_noise_set() {
        let docs_a = vec![
            doc_with_lines("file://rep_1.png", vec!["IBAN: IT60X0542811101000000123456"], 0.95),
            doc_with_lines("file://rep_2.png", vec!["IBAN: IT60X0542811101000000123456"], 0.93),
            doc_with_lines("file://rep_3.png", vec!["IBAN: IT60X0542811101000000I23456"], 0.82),
            doc_with_lines("file://rep_4.png", vec!["IBAN: IT60X054281110100000012345G"], 0.80),
        ];

        let docs_b = vec![
            doc_with_lines("file://rep_3.png", vec!["IBAN: IT60X0542811101000000I23456"], 0.82),
            doc_with_lines("file://rep_1.png", vec!["IBAN: IT60X0542811101000000123456"], 0.95),
            doc_with_lines("file://rep_4.png", vec!["IBAN: IT60X054281110100000012345G"], 0.80),
            doc_with_lines("file://rep_2.png", vec!["IBAN: IT60X0542811101000000123456"], 0.93),
        ];

        let state_a = fold::reduce_documents(docs_a).expect("reduce a");
        let state_b = fold::reduce_documents(docs_b).expect("reduce b");

        assert_eq!(state_a.fields, state_b.fields, "semantic winner must be replay-stable");
        assert_eq!(state_a.convergence_score_bps, state_b.convergence_score_bps);
        assert_eq!(state_a.ambiguity_score_bps, state_b.ambiguity_score_bps);
    }

    /// Semantic conflict counters must be non-zero when the guardrail fires.
    ///
    /// - `negation_conflicts` > 0 when a near-identical pair has a polarity flip.
    /// - `numeric_conflicts`  > 0 when near-identical identifiers differ only in digits.
    /// - Both must appear in `semantic_conflict_count`.
    #[test]
    fn semantic_observability_counters_populated_on_veto() {
        use crate::fold::reduce_documents_with_profile;
        use crate::profile::{IngestionProfile, ProfileKind};

        // --- Negation veto -----------------------------------------------
        // "pagato" / "non pagato" have jaro_winkler ~0.70. We use a permissive
        // profile (threshold 0.65) so the pair passes the distance gate but is
        // stopped by the semantic negation guardrail, incrementing negation_conflicts.
        let permissive = IngestionProfile {
            kind: ProfileKind::Tesseract,
            allow_duplicate_positions: true,
            normalize_whitespace: true,
            normalize_case: false,
            unicode_normalize: true,
            min_confidence: 0.0,
            similarity_threshold: 0.65,
        };
        let neg_state = reduce_documents_with_profile(
            vec![
                doc_with_lines("file://a.png", vec!["pagato"], 0.95),
                doc_with_lines("file://b.png", vec!["non pagato"], 0.95),
            ],
            &permissive,
        )
        .expect("negation reduce failed");
        assert!(
            neg_state.negation_conflicts > 0,
            "expected negation_conflicts > 0, got {}",
            neg_state.negation_conflicts
        );
        assert_eq!(
            neg_state.semantic_conflict_count,
            neg_state.negation_conflicts + neg_state.numeric_conflicts,
        );

        // --- Numeric veto ------------------------------------------------
            // "invoice2026" vs "invoice2027" — jaro_winkler ~0.96, numeric suffix
            // differs → numeric guardrail fires even at default threshold 0.90.
            let default_profile = IngestionProfile::tesseract();
            let num_state = reduce_documents_with_profile(
            vec![
                doc_with_lines("file://c.png", vec!["invoice2026"], 0.95),
                doc_with_lines("file://d.png", vec!["invoice2027"], 0.95),
            ],
                &default_profile,
        )
        .expect("numeric reduce failed");
        assert!(
            num_state.numeric_conflicts > 0,
            "expected numeric_conflicts > 0, got {}",
            num_state.numeric_conflicts
        );
        assert_eq!(
            num_state.semantic_conflict_count,
            num_state.negation_conflicts + num_state.numeric_conflicts,
        );
    }
}
