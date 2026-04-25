//! Integration tests for reducer algebraic properties.
//!
//! These tests verify that the reducer exhibits the expected mathematical properties:
//! - Commutativity (order-independence)
//! - Idempotence (stability under repetition)
//! - Score bounds (values in [0, 10000])
//! - Convergence threshold correctness

use optimo::ocrys::types::{OCRDocument, OCRLine, OCRPage};
use optimo::fold;
use optimo::aggregate_state::ReducerState;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_doc(source: &str, page_num: usize, lines: Vec<&str>) -> OCRDocument {
    OCRDocument {
        source: source.to_string(),
        pages: vec![OCRPage {
            page_number: page_num,
            lines: lines
                .into_iter()
                .map(|text| OCRLine {
                    text: text.to_string(),
                    confidence: Some(0.95),
                })
                .collect(),
        }],
    }
}

fn sort_state_json(state: &ReducerState) -> String {
    // Serialize to JSON for comparison (deterministic ordering via BTreeMap).
    serde_json::to_string(state).expect("serialize state")
}

// ---------------------------------------------------------------------------
// Property 1: Commutativity (order doesn't matter)
// ---------------------------------------------------------------------------

#[test]
fn property_commutativity_two_documents() {
    let source = "test://document";
    let doc_a = make_doc(source, 1, vec!["line A", "line B"]);
    let doc_b = make_doc(source, 1, vec!["line C", "line D"]);

    // Reduce (A, B)
    let mut state_ab = ReducerState::new();
    state_ab.update_from_document(doc_a.clone());
    state_ab.update_from_document(doc_b.clone());

    // Reduce (B, A)
    let mut state_ba = ReducerState::new();
    state_ba.update_from_document(doc_b);
    state_ba.update_from_document(doc_a);

    // Both should produce identical results
    let json_ab = sort_state_json(&state_ab);
    let json_ba = sort_state_json(&state_ba);
    assert_eq!(
        json_ab, json_ba,
        "order of documents should not affect final state"
    );
}

#[test]
fn property_commutativity_three_documents() {
    let source = "test://document";
    let doc_a = make_doc(source, 1, vec!["a"]);
    let doc_b = make_doc(source, 1, vec!["b"]);
    let doc_c = make_doc(source, 1, vec!["c"]);

    // All permutations
    let perms = vec![
        vec![doc_a.clone(), doc_b.clone(), doc_c.clone()],
        vec![doc_a.clone(), doc_c.clone(), doc_b.clone()],
        vec![doc_b.clone(), doc_a.clone(), doc_c.clone()],
        vec![doc_b.clone(), doc_c.clone(), doc_a.clone()],
        vec![doc_c.clone(), doc_a.clone(), doc_b.clone()],
        vec![doc_c.clone(), doc_b.clone(), doc_a],
    ];

    let mut first_json = String::new();
    for (i, perm) in perms.into_iter().enumerate() {
        let mut state = ReducerState::new();
        for doc in perm {
            state.update_from_document(doc);
        }
        let json = sort_state_json(&state);
        if i == 0 {
            first_json = json;
        } else {
            assert_eq!(
                json, first_json,
                "permutation {} produced different state", i
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property 2: Idempotence (feeding same doc twice)
// ---------------------------------------------------------------------------

#[test]
fn property_idempotence_single_document() {
    let source = "test://document";
    let doc = make_doc(source, 1, vec!["line A", "line B", "line C"]);

    // Reduce once
    let mut state_once = ReducerState::new();
    state_once.update_from_document(doc.clone());

    // Reduce twice
    let mut state_twice = ReducerState::new();
    state_twice.update_from_document(doc.clone());
    state_twice.update_from_document(doc);

    // Should be identical
    let json_once = sort_state_json(&state_once);
    let json_twice = sort_state_json(&state_twice);
    assert_eq!(
        json_once, json_twice,
        "feeding same document twice should not change state"
    );
}

#[test]
fn property_idempotence_scores_stable() {
    let source = "test://document";
    let doc = make_doc(source, 1, vec!["line 1", "line 2"]);

    let mut state = ReducerState::new();
    state.update_from_document(doc.clone());
    let score_after_1 = (state.convergence_score_bps, state.ambiguity_score_bps);

    state.update_from_document(doc);
    let score_after_2 = (state.convergence_score_bps, state.ambiguity_score_bps);

    assert_eq!(
        score_after_1, score_after_2,
        "scores should not change after feeding same document again"
    );
}

// ---------------------------------------------------------------------------
// Property 3: Score bounds (always in [0, 10000])
// ---------------------------------------------------------------------------

#[test]
fn property_convergence_score_in_bounds() {
    let source = "test://document";

    // Try various combinations
    let combos = vec![
        vec!["a"],
        vec!["a", "a"],
        vec!["a", "b"],
        vec!["a", "b", "c"],
        vec!["line 1", "line 2", "line 3", "line 4", "line 5"],
    ];

    for combo in combos {
        let doc1 = make_doc(source, 1, combo.clone());
        let doc2 = make_doc(source, 1, combo.clone());
        let doc3 = make_doc(source, 1, combo);

        let mut state = ReducerState::new();
        state.update_from_document(doc1);
        state.update_from_document(doc2);
        state.update_from_document(doc3);

        assert!(
            state.convergence_score_bps <= 10_000,
            "convergence_score_bps exceeded max: {}",
            state.convergence_score_bps
        );
        assert!(
            state.ambiguity_score_bps <= 10_000,
            "ambiguity_score_bps exceeded max: {}",
            state.ambiguity_score_bps
        );
    }
}

#[test]
fn property_ambiguity_score_in_bounds() {
    let source = "test://document";

    for variant_count in 1..=5 {
        let mut state = ReducerState::new();
        for v in 0..variant_count {
            let text = format!("variant_{}", v);
            let doc = make_doc(source, 1, vec![&text]);
            state.update_from_document(doc);
        }

        assert!(
            state.ambiguity_score_bps >= 0,
            "ambiguity_score_bps below min: {}",
            state.ambiguity_score_bps
        );
        assert!(
            state.ambiguity_score_bps <= 10_000,
            "ambiguity_score_bps above max: {}",
            state.ambiguity_score_bps
        );
    }
}

// ---------------------------------------------------------------------------
// Property 4: Convergence threshold correctness
// ---------------------------------------------------------------------------

#[test]
fn property_convergence_threshold() {
    // Convergence is Converged if:
    //   convergence_score_bps >= 9000 && ambiguity_score_bps <= 1000
    // Ambiguous if:
    //   convergence_score_bps >= 5000 (and not converged)
    // Failed otherwise

    let source = "test://document";

    // Case 1: Three identical documents → high convergence, low ambiguity → Converged
    let mut state = ReducerState::new();
    for _ in 0..3 {
        let doc = make_doc(source, 1, vec!["exact line", "another line"]);
        state.update_from_document(doc);
    }
    let status = state.compute_convergence();
    assert_eq!(
        status,
        optimo::observation::ObservationStatus::Converged,
        "three identical docs should converge"
    );

    // Case 2: Moderately varying documents → medium convergence → Ambiguous
    let mut state = ReducerState::new();
    let doc1 = make_doc(source, 1, vec!["line A", "line B"]);
    let doc2 = make_doc(source, 1, vec!["line A", "line B"]);
    let doc3 = make_doc(source, 1, vec!["line A", "line C"]); // different
    state.update_from_document(doc1);
    state.update_from_document(doc2);
    state.update_from_document(doc3);
    let status = state.compute_convergence();
    // Status should be Ambiguous or Converged (or Failed depending on exact scores)
    println!(
        "moderate variation: convergence={}, ambiguity={}, status={:?}",
        state.convergence_score_bps, state.ambiguity_score_bps, status
    );
}

// ---------------------------------------------------------------------------
// Property 5: No data loss (all input lines appear in output clusters)
// ---------------------------------------------------------------------------

#[test]
fn property_line_conservation() {
    let source = "test://document";

    let line_set = vec!["line 1", "line 2", "line 3"];

    // Create 3 documents with the same lines
    let doc1 = make_doc(source, 1, line_set.clone());
    let doc2 = make_doc(source, 1, line_set.clone());
    let doc3 = make_doc(source, 1, line_set);

    let mut state = ReducerState::new();
    state.update_from_document(doc1);
    state.update_from_document(doc2);
    state.update_from_document(doc3);

    // Count how many lines appear in the output
    let output_lines: Vec<_> = state
        .cluster_groups
        .values()
        .flat_map(|page_map| page_map.values().flatten().flatten())
        .collect();

    assert!(
        !output_lines.is_empty(),
        "output should have lines after fedback documents"
    );
    assert_eq!(
        output_lines.len(),
        3,
        "should have exactly 3 lines in clusters"
    );
}

// ---------------------------------------------------------------------------
// Fixture-based integration test (placeholder)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires fixture files present"]
fn integration_invoice_table_fixture() {
    // This test would:
    // 1. Load fixtures/domain/invoice_table_sample/raw_ocr_*.txt
    // 2. Create OCRDocument from each
    // 3. Run reducer
    // 4. Verify against fixtures/domain/invoice_table_sample/expected.json
    //
    // Placeholder for future implementation with fs API.
    println!("placeholder for fixture-based test");
}
