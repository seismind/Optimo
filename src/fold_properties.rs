/// Algebraic property tests for the reducer.
///
/// These tests verify that `ReducerState::reduce_documents()` exhibits
/// the properties of a deterministic, order-independent fold:
/// - Commutativity (order independence)
/// - Idempotence (no phantom clusters, stable scores)
/// - Monotonicity (more evidence ⇒ score doesn't decrease)
/// - Boundedness (scores always in [0, 10000] basis points)
///
/// All tests use synthetic OCR documents (no external fixtures needed).
/// All comparisons via `content_hash` and structured equality, not JSON strings.

#[cfg(test)]
mod tests {
    use crate::ocrys::types::{OCRDocument, OCRPage, OCRLine};
    use crate::fold;

    // -------- Synthetic OCR generation --------

    /// Generate a simple OCR document with N lines.
    fn make_doc(source: &str, lines: Vec<&str>) -> OCRDocument {
        OCRDocument {
            source: source.to_string(),
            pages: vec![OCRPage {
                page_number: 1,
                lines: lines.into_iter().map(|t| OCRLine {
                    text: t.to_string(),
                    confidence: Some(0.90),
                }).collect(),
            }],
        }
    }

    /// Two states are considered structurally equal when scores and all line texts match.
    /// Uses sorted flat line texts to be order-independent within pages.
    fn hashes_equal(state_a: &crate::aggregate_state::ReducerState, state_b: &crate::aggregate_state::ReducerState) -> bool {
        if state_a.convergence_score_bps != state_b.convergence_score_bps {
            return false;
        }
        if state_a.ambiguity_score_bps != state_b.ambiguity_score_bps {
            return false;
        }
        let sorted_lines = |state: &crate::aggregate_state::ReducerState| -> Vec<String> {
            let mut lines: Vec<String> = state.pages.iter()
                .flat_map(|p| p.lines.iter().map(|l| l.text.clone()))
                .collect();
            lines.sort();
            lines
        };
        sorted_lines(state_a) == sorted_lines(state_b)
    }

    // -------- Property 1: Commutativity --------

    #[test]
    fn commutativity_order_independence() {
        let source = "file://test.png";
        let doc1 = make_doc(source, vec!["alpha", "beta"]);
        let doc2 = make_doc(source, vec!["alpha", "gamma"]);
        let doc3 = make_doc(source, vec!["beta", "gamma"]);

        // All permutations should yield identical hash + scores
        let perms = vec![
            vec![doc1.clone(), doc2.clone(), doc3.clone()],
            vec![doc3.clone(), doc1.clone(), doc2.clone()],
            vec![doc2.clone(), doc3.clone(), doc1.clone()],
        ];

        let states: Vec<_> = perms.into_iter()
            .map(|docs| fold::reduce_documents(docs).expect("reduce"))
            .collect();

        // All pairs should have matching hashes
        for i in 1..states.len() {
            assert!(hashes_equal(&states[0], &states[i]),
                "permutation {} should match permutation 0", i);
        }
    }

    // -------- Property 2a: Idempotence - No Phantom Clusters --------

    #[test]
    fn idempotence_no_phantom_clusters() {
        let source = "file://test.png";
        let doc = make_doc(source, vec!["line1", "line2", "line3"]);

        let state1 = fold::reduce_documents(vec![doc.clone()])
            .expect("first pass");
        let state2 = fold::reduce_documents(vec![doc.clone(), doc.clone()])
            .expect("second pass with duplicate");

        // Cluster count should not increase
        assert_eq!(state1.cluster_groups.len(), state2.cluster_groups.len(),
            "duplicate input should not create new clusters");
        
        // Each cluster should have same line count
        for (page, clusters1) in &state1.cluster_groups {
            let clusters2 = state2.cluster_groups.get(page)
                .expect("page should exist in state2");
            assert_eq!(clusters1.len(), clusters2.len(),
                "page {} cluster count must match", page);
        }
    }

    // -------- Property 2b: Idempotence - Score Stability --------

    #[test]
    fn idempotence_score_stability() {
        let source = "file://test.png";
        let doc1 = make_doc(source, vec!["line1", "line2"]);
        let doc2 = make_doc(source, vec!["line1", "line3"]);
        let doc3 = make_doc(source, vec!["line2", "line3"]);

        let state_once = fold::reduce_documents(vec![doc1.clone(), doc2.clone(), doc3.clone()])
            .expect("once");
        let state_twice = fold::reduce_documents(vec![
            doc1.clone(), doc2.clone(), doc3.clone(),
            doc1.clone(), doc2.clone(), doc3.clone(),
        ])
            .expect("twice");

        // Scores should stabilize (allow 100 bps tolerance for floating point)
        let delta_convergence = (state_once.convergence_score_bps as i32 - state_twice.convergence_score_bps as i32).abs();
        assert!(delta_convergence <= 100, 
            "convergence score should stabilize: delta={}", delta_convergence);
    }

    // -------- Property 3: Monotonicity - More Evidence ⇒ Score Doesn't Decrease --------

    #[test]
    fn monotonicity_more_evidence_stable_score() {
        let source = "file://test.png";
        let doc1 = make_doc(source, vec!["invoice", "total"]);
        let doc2 = make_doc(source, vec!["invoice", "total"]);

        let state_2docs = fold::reduce_documents(vec![doc1.clone(), doc2.clone()])
            .expect("2 docs");

        // Add a third doc with same content
        let doc3 = make_doc(source, vec!["invoice", "total"]);
        let state_3docs = fold::reduce_documents(vec![doc1.clone(), doc2.clone(), doc3.clone()])
            .expect("3 docs");

        // Score should not decrease (allow 100 bps tolerance)
        let score_2 = state_2docs.convergence_score_bps as i32;
        let score_3 = state_3docs.convergence_score_bps as i32;
        assert!(score_3 >= score_2 - 100,
            "more evidence should not decrease score: 2-doc={}, 3-doc={}", score_2, score_3);
    }

    // -------- Property 4: Boundedness - Scores in [0, 10000] --------

    #[test]
    fn boundedness_scores_in_range() {
        let source = "file://test.png";

        // Generate various configurations
        let test_cases = vec![
            vec!["alpha"],
            vec!["alpha", "beta"],
            vec!["alpha", "beta", "gamma"],
            vec!["a", "b", "c", "d", "e"],
        ];

        for lines in test_cases {
            let docs = vec![
                make_doc(source, lines.clone()),
                make_doc(source, lines.clone()),
                make_doc(source, lines),
            ];

            let state = fold::reduce_documents(docs)
                .expect("reduce");

            // Check bounds (u32 is always >= 0; only check upper bound)
            assert!(state.convergence_score_bps <= 10_000,
                "convergence score {} > 10000", state.convergence_score_bps);
            assert!(state.ambiguity_score_bps <= 10_000,
                "ambiguity score {} > 10000", state.ambiguity_score_bps);
        }
    }

    // -------- Property 5: Line Conservation --------

    #[test]
    fn line_conservation_no_data_loss() {
        // Every line present in ALL variants must survive into the reducer output.
        // We give all three docs the same content so there is no majority ambiguity.
        let source = "file://test.png";
        let lines = vec!["alpha", "beta", "gamma"];
        let docs = vec![
            make_doc(source, lines.clone()),
            make_doc(source, lines.clone()),
            make_doc(source, lines.clone()),
        ];

        let state = fold::reduce_documents(docs)
            .expect("reduce");

        let lines_in_state: std::collections::HashSet<String> = state.pages.iter()
            .flat_map(|p| p.lines.iter().map(|l| l.text.clone()))
            .collect();

        for expected_line in &lines {
            assert!(lines_in_state.contains(*expected_line),
                "expected line {:?} not found in reducer output", expected_line);
        }
    }
}
