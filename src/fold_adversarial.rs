/// Adversarial tests for the fold reducer.
///
/// Priority order:
///   1-3  Critical arithmetic safety (NaN, Inf, zero/max weight)
///   4-6  Degenerate reducer states (whitespace, single doc, mass conflict)
///   7-9  Normalization (NFC/NFD, zero-width, decimal comma)
///  10-11 Clustering boundaries (at threshold, below threshold)
///     12 Security realism (Cyrillic homoglyph isolation)

#[cfg(test)]
mod tests {
    use crate::ocrys::types::{OCRDocument, OCRPage, OCRLine};
    use crate::fold;

    // ---- helpers ----

    fn doc(source: &str, lines: Vec<(&str, Option<f32>)>) -> OCRDocument {
        OCRDocument {
            source: source.to_string(),
            pages: vec![OCRPage {
                page_number: 1,
                lines: lines.into_iter().map(|(t, c)| OCRLine {
                    text: t.to_string(),
                    confidence: c,
                }).collect(),
            }],
        }
    }

    fn uniform(source: &str, lines: Vec<&str>) -> OCRDocument {
        doc(source, lines.into_iter().map(|t| (t, Some(0.90))).collect())
    }

    // ======================================================================
    // 1-3  Critical arithmetic safety
    // ======================================================================

    /// NaN confidence must be sanitized to 0, never propagate NaN into scores.
    #[test]
    fn arithmetic_nan_confidence_sanitized() {
        let docs = vec![
            doc("file://a.png", vec![("invoice", Some(f32::NAN))]),
            doc("file://a.png", vec![("invoice", Some(f32::NAN))]),
            doc("file://a.png", vec![("invoice", Some(0.9))]),
        ];
        let state = fold::reduce_documents(docs).expect("NaN must not cause error");
        assert!(state.convergence_score_bps <= 10_000,
            "convergence out of range after NaN: {}", state.convergence_score_bps);
        assert!(state.ambiguity_score_bps <= 10_000,
            "ambiguity out of range after NaN: {}", state.ambiguity_score_bps);
        assert!(!state.convergence_score_bps.leading_zeros().eq(&32), // not NaN (u32 can't be NaN but ensure it's not garbage)
            "NaN must not corrupt scores");
    }

    /// Infinity confidence must be clamped to 1.0, never propagate Inf.
    #[test]
    fn arithmetic_inf_confidence_sanitized() {
        let docs = vec![
            doc("file://a.png", vec![("invoice", Some(f32::INFINITY))]),
            doc("file://a.png", vec![("invoice", Some(f32::NEG_INFINITY))]),
            doc("file://a.png", vec![("invoice", Some(0.9))]),
        ];
        let state = fold::reduce_documents(docs).expect("Inf must not cause error");
        assert!(state.convergence_score_bps <= 10_000,
            "convergence out of range after Inf: {}", state.convergence_score_bps);
        assert!(state.ambiguity_score_bps <= 10_000,
            "ambiguity out of range after Inf: {}", state.ambiguity_score_bps);
    }

    /// A candidate with max weight must beat a candidate with zero weight.
    /// Division-by-zero guard: all-zero weights must not panic or produce garbage.
    #[test]
    fn arithmetic_max_weight_beats_zero_weight() {
        // Two candidates at same position: "winner" has weight 1.0, "loser" has 0.0
        let docs = vec![
            doc("file://a.png", vec![("winner", None)]),       // None → weight 1.0
            doc("file://a.png", vec![("winner", None)]),       // weight 1.0
            doc("file://a.png", vec![("loser",  Some(0.0))]),  // weight 0.0
        ];
        let state = fold::reduce_documents(docs).expect("reduce");
        let page = state.pages.iter().find(|p| p.page_number == 1).expect("page 1");
        assert_eq!(page.lines[0].text, "winner",
            "max weight must beat zero weight candidate");

        // All-zero weights: must not panic, scores must stay in range
        let zero_docs = vec![
            doc("file://b.png", vec![("a", Some(0.0))]),
            doc("file://b.png", vec![("b", Some(0.0))]),
        ];
        let zero_state = fold::reduce_documents(zero_docs).expect("all-zero must not panic");
        assert!(zero_state.convergence_score_bps <= 10_000);
        assert!(zero_state.ambiguity_score_bps <= 10_000);
    }

    // ======================================================================
    // 4-6  Degenerate reducer states
    // ======================================================================

    /// Document containing only whitespace/empty lines must return Err.
    /// The reducer has nothing to work with: returning Ok would be a silent lie.
    #[test]
    fn degenerate_whitespace_only_document_is_error() {
        let docs = vec![
            doc("file://a.png", vec![
                ("   ", Some(0.9)),
                ("\t\n", Some(0.9)),
                ("", Some(0.9)),
            ]),
        ];
        assert!(
            fold::reduce_documents(docs).is_err(),
            "whitespace-only document must return Err, not Ok"
        );
    }

    /// Single document, single line → convergence must be 10000, ambiguity 0.
    /// One witness, no alternatives: certainty is absolute by definition.
    #[test]
    fn degenerate_single_doc_single_line_full_convergence() {
        let docs = vec![uniform("file://a.png", vec!["invoice total 45.20"])];
        let state = fold::reduce_documents(docs).expect("reduce");
        assert_eq!(state.convergence_score_bps, 10_000,
            "single-line single-doc must give convergence=10000");
        assert_eq!(state.ambiguity_score_bps, 0,
            "single-line single-doc must give ambiguity=0");
    }

    /// 100 documents each with a unique token at the same position.
    /// Every token is its own cluster → convergence near zero, ambiguity high.
    #[test]
    fn degenerate_hundred_conflicting_docs_low_convergence() {
        // "token_000"…"token_099" share a long prefix: jaro_winkler chains them
        // all into one cluster (high convergence = wrong fixture).
        // Use LCG multiplication to scatter 100 values across u64 uniformly,
        // producing visually distinct 16-char hex strings with low pairwise similarity.
        let tokens: Vec<String> = (1u64..=100)
            .map(|i| format!("{:016x}", i.wrapping_mul(6364136223846793005_u64)))
            .collect();
        let docs: Vec<OCRDocument> = tokens.iter()
            .map(|t| uniform("file://a.png", vec![t.as_str()]))
            .collect();

        let state = fold::reduce_documents(docs).expect("reduce");
        assert!(state.convergence_score_bps < 2_000,
            "100 unique tokens must give convergence < 2000, got {}",
            state.convergence_score_bps);
        assert!(state.ambiguity_score_bps > 0,
            "ambiguity must be > 0 with 100 unique tokens");
    }

    // ======================================================================
    // 7-9  Normalization
    // ======================================================================

    /// NFC and NFD representations of the same glyph must cluster together.
    /// "café" (NFC U+00E9) vs "cafe" + U+0301 combining accent (NFD).
    #[test]
    fn normalize_nfc_nfd_cluster_together() {
        let nfc = "caf\u{00E9}";        // é precomposto
        let nfd = "cafe\u{0301}";        // e + combining acute accent

        let docs = vec![
            uniform("file://a.png", vec![nfc]),
            uniform("file://a.png", vec![nfd]),
            uniform("file://a.png", vec![nfd]),
        ];
        let state = fold::reduce_documents(docs).expect("reduce");
        let pos = state.cluster_groups.get(&1).expect("page 1").get(&0).expect("pos 0");
        assert_eq!(pos.len(), 1,
            "NFC and NFD must collapse to one cluster, got: {:?}", pos);
    }

    /// Zero-width space (U+200B) embedded in a token must be stripped.
    /// The polluted and clean versions of the same word must cluster together,
    /// and the winner must not contain the invisible character.
    #[test]
    fn normalize_zero_width_space_stripped() {
        let clean    = "invoice";
        let polluted = "inv\u{200B}oice"; // zero-width space mid-word

        let docs = vec![
            uniform("file://a.png", vec![clean]),
            uniform("file://a.png", vec![polluted]),
            uniform("file://a.png", vec![polluted]),
        ];
        let state = fold::reduce_documents(docs).expect("reduce");
        let winner = &state.pages.iter()
            .find(|p| p.page_number == 1).expect("page 1")
            .lines[0].text;
        assert!(!winner.contains('\u{200B}'),
            "winner must not contain zero-width space, got: {:?}", winner);
    }

    /// "45,20" (decimal comma) and "45.20" (decimal point) must be the same token
    /// after harmonization and must land in a single cluster.
    #[test]
    fn normalize_decimal_comma_and_dot_cluster_together() {
        let docs = vec![
            uniform("file://a.png", vec!["45,20"]),
            uniform("file://a.png", vec!["45.20"]),
            uniform("file://a.png", vec!["45.20"]),
        ];
        let state = fold::reduce_documents(docs).expect("reduce");
        let pos = state.cluster_groups.get(&1).expect("page 1").get(&0).expect("pos 0");
        assert_eq!(pos.len(), 1,
            "decimal comma and decimal dot must cluster, got: {:?}", pos);
    }

    // ======================================================================
    // 10-11 Clustering boundaries
    // ======================================================================

    /// Strings with jaro_winkler similarity >= SIM_THRESHOLD (0.90) must cluster.
    /// We assert the similarity at test time to make the fixture self-documenting.
    #[test]
    fn clustering_at_threshold_merges() {
        // "invoice2026" vs "invoice2027": differ only in last digit, high similarity
        let s1 = "invoice2026";
        let s2 = "invoice2027";
        let sim = strsim::jaro_winkler(s1, s2);
        assert!(sim >= 0.90,
            "test fixture: jaro_winkler({s1:?},{s2:?})={sim:.4} must be >= 0.90");

        let docs = vec![
            uniform("file://a.png", vec![s1]),
            uniform("file://a.png", vec![s1]),
            uniform("file://a.png", vec![s2]),
        ];
        let state = fold::reduce_documents(docs).expect("reduce");
        let pos = state.cluster_groups.get(&1).expect("page 1").get(&0).expect("pos 0");
        assert_eq!(pos.len(), 1,
            "strings above threshold must merge into one cluster: {:?}", pos);
    }

    /// Strings with jaro_winkler similarity < SIM_THRESHOLD must NOT cluster.
    #[test]
    fn clustering_below_threshold_stays_separate() {
        let s1 = "invoice";
        let s2 = "payment";
        let sim = strsim::jaro_winkler(s1, s2);
        assert!(sim < 0.90,
            "test fixture: jaro_winkler({s1:?},{s2:?})={sim:.4} must be < 0.90");

        let docs = vec![
            uniform("file://a.png", vec![s1]),
            uniform("file://a.png", vec![s2]),
        ];
        let state = fold::reduce_documents(docs).expect("reduce");
        let pos = state.cluster_groups.get(&1).expect("page 1").get(&0).expect("pos 0");
        assert!(pos.len() >= 2,
            "strings below threshold must stay in separate clusters: {:?}", pos);
    }

    // ======================================================================
    // 12  Security realism
    // ======================================================================

    /// Cyrillic homoglyphs must NOT silently cluster with their Latin lookalikes.
    ///
    /// A document containing "іnvoice" (і = U+0456 Cyrillic) must NOT be treated
    /// as equivalent to "invoice" (all Latin). Silent merging would allow an
    /// adversarially crafted document to inflate the score of a fraudulent token.
    #[test]
    fn security_cyrillic_homoglyph_isolated() {
        let latin    = "invoice";
        let homoglyph = "іnvoice"; // і = U+0456 CYRILLIC SMALL LETTER BYELORUSSIAN-UKRAINIAN I

        let docs = vec![
            uniform("file://a.png", vec![latin]),
            uniform("file://a.png", vec![homoglyph]),
        ];
        let state = fold::reduce_documents(docs).expect("reduce");
        let pos = state.cluster_groups.get(&1).expect("page 1").get(&0).expect("pos 0");
        assert!(pos.len() >= 2,
            "Cyrillic homoglyph must NOT merge with Latin lookalike: {:?}", pos);
    }
}
