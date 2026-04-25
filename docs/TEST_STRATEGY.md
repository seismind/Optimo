# Test Strategy for Optimo Reducer

## Goal

Verify that `ReducerState` exhibits the algebraic properties expected of a deterministic fold:
- **Associativity** (chunk invariance: reduce all ≡ reduce(reduce chunks))
- **Commutativity** (order-independence)
- **Idempotence** (no phantom clusters; scores don't diverge on rerun)
- **Monotonicity** (more input evidence ⇒ score doesn't decrease)
- **Boundedness** (scores always in [0, 10000] basis points)
- **Correctness** (domain-specific invariants)

We test both at the **unit level** (small synthetic cases) and **integration level** (real OCR fixtures).

---

## Property-Based Testing: Reducer Algebra

### 0. Associativity (Chunk Invariance)

**Property**: Reducing documents in chunks produces the same result as reducing all at once.

```
R(all_docs) ≡ R(R(chunk_1) + R(chunk_2) + ...)
```

**Why it matters**: The reducer should decompose. Batching shouldn't affect final state.

**Test approach**:
- Generate N OCR documents.
- Compute state from all N at once.
- Split into chunks, reduce each chunk separately, then fold chunk results.
- Compare final states (via content_hash, not raw JSON).
- Assert both paths yield identical convergence scores and cluster structure.

---

### 1. Commutativity

**Property**: Reducing documents in any order produces the same final state.

```
R([doc1, doc2, doc3]) ≡ R([doc3, doc1, doc2]) (modulo serialization order)
```

**Why it matters**: OCR variants arrive in unpredictable order. If order affects convergence, we have a bug.

**Test approach**:
- Generate 3 random OCR documents (all same source).
- Reduce in multiple permutations.
- Compare `convergence_score_bps`, `ambiguity_score_bps`, resulting pages/clusters.
- Assert all permutations produce identical `ReducerSnapshot` (via content_hash equality, not string comparison).

---

### 2. Idempotence: No Phantom Clusters

**Property**: Feeding the same document twice does not create new clusters.

```
cluster_count(state.update(doc)) ≡ cluster_count(state.update(doc).update(doc))
```

**Why it matters**: Duplicates are allowed (by IngestionProfile), but they shouldn't spawn phantom clusters.

**Test approach**:
- Create one OCR document with 2-3 unique lines.
- Update reducer, record cluster count at each position.
- Update same reducer with the same doc again.
- Assert cluster structure (page/line/count) unchanged.

---

### 2b. Idempotence: Score Stability

**Property**: Scores stabilize on repeated input (they may change on first pass, but not on second).

```
state1 = state.update(doc)
state2 = state1.update(doc)
state2.convergence_score_bps ≡ state1.convergence_score_bps  (not strictly ==, but no divergence)
```

**Why it matters**: Confidence scores should reflect actual agreement, not grow unboundedly.

**Test approach**:
- Create 3 OCR documents with light variations.
- Reduce once, capture scores.
- Feed the same 3 documents again in same order.
- Assert scores either identical or move in a bounded Δ (< 100 bps).

---

### 3. Monotonicity: More Evidence ⇒ Score Stability or Increase

**Property**: Adding more evidence (more OCR variants on the same lines) should not decrease convergence score.

```
state1 = reduce([doc1, doc2])
state2 = reduce([doc1, doc2, doc3])
state2.convergence_score_bps ≥ state1.convergence_score_bps  (or within small tolerance)
```

**Why it matters**: Additional evidence should only strengthen (or maintain) confidence, never weaken it arbitrarily.

**Test approach**:
- Generate 2 OCR documents with good agreement.
- Reduce them, record score A.
- Add a third document with same or similar lines.
- Reduce all 3, record score B.
- Assert B ≥ A - epsilon (small tolerance for floating-point).
- Repeat for multiple random configurations.

---

### 4. Boundedness: Scores in Valid Range

**Property**: Scores are always in basis points [0, 10000].

```
∀ state: 0 ≤ state.convergence_score_bps ≤ 10000
∀ state: 0 ≤ state.ambiguity_score_bps ≤ 10000
```

**Why it matters**: Scores are used for threshold comparisons. Out-of-bounds values break logic.

**Test approach**:
- Fuzz multiple OCR document combinations (various counts, line overlaps).
- After each update, check both scores are in [0, 10000].
- Fail if any score violates bounds.

---

### 5. Convergence Threshold Correctness

**Property**: Convergence state follows the configured profile thresholds.

**Important**: Thresholds are NOT hardcoded. They come from `IngestionProfile` (via config resolver).
Tests must read thresholds from the active profile, not duplicate constants.

**Test approach**:
- Load `IngestionProfile::tesseract()` (or other profile).
- Manually construct states with specific score values.
- Verify state classification matches profile logic.
- Test boundary conditions around profile thresholds.
- Repeat for multiple profiles (tesseract, strict, carbo).

---

### 6. Line Conservation (No Data Loss)

**Property**: Every input line must appear in the output (either as the winner or as an alternative candidate).

```
input_lines ⊆ (output.pages.lines ∪ output.alternatives)
```

**Why it matters**: If lines disappear, we're throwing away OCR evidence silently.

**Test approach**:
- Generate N OCR variants of the same document.
- Track every unique line across variants.
- After reduction, verify every input line is in the cluster groups.
- No line should be dropped.

---

## Comparison Method: Typed Equality, Not String Comparison

**Key principle**: Compare `ReducerSnapshot` structures via **content_hash** and structured field equality, not raw JSON strings.

**Rationale**:
- BTreeMap serialization order is deterministic, but comparing JSON strings is fragile and unreadable.
- `content_hash = Uuid::new_v5(canonical_json)` gives us a deterministic fingerprint.
- Structure-level comparison (scores, cluster count, page count) is more maintainable.

**Pattern**:
```rust
let state_a = reduce(docs_1);
let state_b = reduce(docs_2);
assert_eq!(state_a.content_hash, state_b.content_hash);
assert_eq!(state_a.convergence_score_bps, state_b.convergence_score_bps);
assert_eq!(state_a.cluster_groups.len(), state_b.cluster_groups.len());
```

---

## Domain Fixture Testing

### Real OCR Scenarios

We maintain:
- **`fixtures/domain/table_extraction/`** — multi-row tables with columnar alignment
- **`fixtures/domain/ocr_variants/`** — same PDF ran through 3 OCR strategies (original, high_contrast, rotated)
- **`fixtures/domain/known_mismatches/`** — edge cases where OCR fails in predictable ways

Each fixture includes:
- `raw_ocr_*.txt` — tesseract direct output (per variant)
- `expected.json` — manually validated ground truth (what the reducer should produce)
- `notes.md` — why this case matters (OCR failure mode, business significance)

Example:
```
fixtures/domain/table_invoice_sample/
├── raw_ocr_original.txt
├── raw_ocr_high_contrast.txt
├── raw_ocr_rotated.txt
├── expected.json        (ground truth)
└── notes.md            (invoice table, Carbo AI won't exist yet, Tesseract struggles on tabular)
```

### Business-Relevant Edge Cases

1. **Decimal number ambiguity**: "45,20" (comma in Italian) vs "45.20" (dot in English)
   - Different OCR variants produce both forms.
   - Reducer should normalize and cluster them together.

2. **Currency symbols**: "€1000" vs "1000 EUR" vs "1000€"
   - Multiple semantic representations of the same value.
   - Reducer should recognize semantic equivalence (via NFC + canonical form).

3. **OCR hallucination**: One variant "sees" a line that others don't.
   - Minority candidate (1/3 variants) should not trigger False Positive.
   - Convergence score should reflect low agreement.

4. **Missing lines**: One variant misses a line entirely (e.g., rotated OCR fails).
   - Reducer should still converge if majority agrees on what's present.
   - No "data loss" flag — just lower confidence.

---

## Test Modules

### `src/reducer_algebra.rs`
- New module dedicated to algebraic properties.
- Property tests: commutativity, associativity, idempotence (both flavors), monotonicity, boundedness.
- No domain knowledge; pure reducer properties.
- Runs on synthetic OCR documents.

### `src/reducer_state.rs::tests`
- Mutation tests (ensure metrics update correctly after `update_from_document`).
- Score calculation verification.
- Cluster management correctness.

### `tests/integration/`
- Real OCR fixture → reducer → validation against expected JSON.
- End-to-end pipeline: PNG → tesseract → normalize → reducer → snapshot.
- Acceptance tests with domain-specific assertions.
- Load fixture expected values; compare results via content_hash + structure equality.

### Fixtures
- **`fixtures/domain/table_invoice_sample/`** — Italian invoice table; tests decimal normalization (45,20 → 45.20) and numeric clustering.
- **`fixtures/domain/currency_variants/`** — Same amount in 3 forms (€1000, 1000 EUR, 1000€); tests semantic equivalence.
- **`fixtures/domain/ocr_hallucination/`** — One variant "sees" extra lines; tests majority voting (low minority score).

---

## Thresholds Under Test

**Thresholds are profile-dependent, NOT hardcoded.**

Each `IngestionProfile` may define its own convergence criteria:

| Profile | `convergence_score_bps` threshold | `min_confidence` | Notes |
|---------|----------------------------------|-----------------|-------|
| tesseract | ~9000 (via `reduce_documents` logic) | 0.55 | Permissive; multi-variant OCR |
| strict | ~9500 | 0.95 | Acceptance testing; high bar |
| carbo | (TBD) | 0.80 | Future AI source |

Tests must **read these thresholds from the active profile**, not hardcode them.

---

## Regression Prevention

Once we have solid fixtures and property tests:
1. Every new profile variant must pass **all** property tests.
2. Every new OCR source integration must validate against fixtures.
3. Threshold changes require updating **both** threshold tests + fixture expected values.

---

## Implementation Order

1. **Create `src/reducer_algebra.rs`**: Implement property tests (0–4: associativity through boundedness).
   - Use synthetic OCR documents (randomized lines, controlled variance).
   - Run permutations, chunks, score bounds checks.
   - Compare results via content_hash + structure equality.

2. **Create 3 domain fixtures**:
   - `fixtures/domain/table_invoice_sample/` (decimal normalization)
   - `fixtures/domain/currency_variants/` (semantic equivalence)
   - `fixtures/domain/ocr_hallucination/` (majority voting)
   - Each with `raw_ocr_*.txt`, `expected.json`, `notes.md`.

3. **Create `tests/integration/fixture_validation.rs`**:
   - Load each fixture.
   - Run reducer on raw OCR files.
   - Compare snapshot against expected.
   - Validate convergence_score, cluster count, line preservation.

4. **Extend `src/observation.rs`**:
   - Business-side validation (e.g., "invoice total must be present").
   - Observation status rules tied to profile and convergence state.
