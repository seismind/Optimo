# Architectural Decisions

This document records key architectural decisions taken in the Optimo project.

The goal is to preserve reasoning context and prevent architectural drift over time.

Each decision includes:

- context
- decision
- consequences

This is a lightweight ADR log.

---

## ADR-0001 — Deterministic Reducer Core

### Context

Optimo processes multiple OCR variants and must produce stable, reproducible results.
Future goals include replay, auditability, and distributed processing.

### Decision

The reducer is implemented as a pure deterministic function:

```text
(State, Input) -> ReducerResult
```

The reducer must:

- perform no I/O
- generate no timestamps or random identifiers
- depend only on explicit inputs

### Consequences

- exact replay is possible
- testability is significantly improved
- architectural discipline is required to prevent side effects from leaking into the core

---

## ADR-0002 — Externalized Persistence via State Bridge

### Context

The system must evolve from JSONL logging to more advanced storage systems (e.g., SQLite).

### Decision

All persistence operations are routed through a dedicated boundary:

```text
persistence.rs
```

The reducer and observation layers must remain storage-agnostic.

### Consequences

- storage can evolve without modifying core logic
- clear separation between computation and infrastructure
- additional mapping layer complexity

---

## ADR-0003 — Observation as First-Class Output

### Context

Deterministic computation alone is insufficient for audit and diagnostics.
System decisions must be explainable.

### Decision

Every reducer execution produces a structured observation record.

Observations describe:

- convergence status
- ambiguity
- failure conditions
- relevant metadata for analysis

### Consequences

- improved debuggability
- audit-ready execution traces
- increased data volume in logs

---

## ADR-0004 — Event Emission Derived from Reducer Results

### Context

Certain reducer outcomes (e.g., ambiguous or failed convergence) must trigger higher-level system reactions.

### Decision

The reducer signals event necessity via its result.
Event construction and persistence are handled by the runtime layer.

### Consequences

- deterministic core remains pure
- event-driven extensions become possible
- additional coordination logic required in orchestration layer

---

## ADR-0005 — Snapshot Logging for Replay Scalability

### Context

Full event replay may become expensive as the system scales.

### Decision

Periodic state snapshots are persisted independently of observations.

Current implementation:

```text
data/snapshots.jsonl
```

### Consequences

- faster replay initialization
- storage overhead increases
- snapshot strategy may need refinement over time

---

## ADR-0006 — OCR as Pipeline Stress-Test, Not System Identity

### Context

The current implementation uses OCR to generate input data.
However, the long-term system goal is broader deterministic document intelligence.

### Decision

OCR is treated as an input generator rather than the defining capability of the system.

### Consequences

- architecture remains generalizable
- future input sources can be integrated without core redesign
- system messaging must avoid being perceived as “just OCR”

---

## ADR-0007 — Mathematical Formalization of the Core Triad

### Context

The architecture had clear invariants, but lacked a shared formal model for:

- deterministic state evolution
- structural checkpointing
- semantic decision observation

Without this model, replay and convergence proofs risk becoming implementation-specific.

### Decision

Formalize the triad as:

- $R$: deterministic fold over state and documents
- $\Pi$: snapshot projection with runtime metadata injection
- $\Omega$: observation projection with runtime metadata injection

and maintain persistence as external boundary $B$.

Reference document:

```text
docs/TRIAD_FORMALISM.md
```

### Consequences

- roadmap items (replay engine, cognitive observation layer, convergence theorem) now have a stable formal base
- implementation reviews can be validated against explicit morphisms and invariants
- boundary violations (metadata generation in core) become straightforward to detect

---

## ADR-0008 — Deterministic Replay with Rigorous Snapshot Hydration

### Context

Full replay capability requires checkpoint-based initialization to scale beyond pure genesis replay.
This demands a clear contract for snapshot rehydration that:

- never leaves state in an inconsistent or "zombie" condition
- fails fast and explicitly before any core contamination
- distinguishes between projection (what to show) and rehydration (what's needed for fold resume)

### Decision

1. Snapshot now explicitly carries two independent payloads:
   - projection: fields, confidence, iterations (for audit/reporting)
   - rehydration: source + cluster_groups (minimal canonical state for deterministic fold restart)

2. From-snapshot hydration is rigorous:
   - validates schema_version early
   - enforces document_id/source coherence
   - recomputes metrics and validates confidence match
   - fails loudly if rehydration payload is missing or invalid

3. Replay engine ensures:
   - deterministic event ordering (timestamp, then id)
   - genesis replay (events only) path
   - checkpoint + tail replay (latest snapshot + events after cutoff)

### Consequences

- snapshot corruption cannot silently contaminate the fold
- genesis and checkpoint+tail replay produce identical final state (tested)
- schema evolution is now feasible via versioned migrations
- integrity chain (snapshot_hash + tail_chain_hash) is architected for future audit

---

## ADR-0009 — Confidence Formula and Snapshot Schema v1

### Context

The reducer produces a `confidence` score, but its derivation was implicit and undocumented.
Additionally, snapshot fields used UI-like string keys (`page_1_line_1`) and a loose `BTreeMap`
that was not queryable in SQLite.

### Decision

**Confidence formula — Weighted Cluster Plurality Ratio:**

For each line position `(page, line_index)` across all OCR variants:

```
line_convergence(pos) = max_cluster_size(pos) / total_candidates(pos)
```

where `max_cluster_size` is the count of candidates in the winning (plurality) cluster,
and `total_candidates` is the total count of candidates across all clusters at that position.

Global confidence:

```
confidence = mean(line_convergence) over all positions
           = (Σ line_convergence) / N_positions
```

Reported in basis points internally (`convergence_score_bps` ∈ [0, 10000]).
Exported as `f32` in [0.0, 1.0].

This is a **plurality agreement ratio**: it measures what fraction of OCR candidates
agree on the winning answer, on average across all extracted line positions.

It is NOT entropy, NOT a weighted average of per-word Tesseract confidence scores,
and NOT a geometric mean. Those are potential future refinements (see below).

**Snapshot schema v1 — Typed projection:**

- `fields: BTreeMap<String, String>` removed from `ReducerSnapshot`
- replaced by `lines: Vec<SnapshotLine>` where each element is `{ page, line, text }`
- `content_hash: Uuid` added: deterministic fingerprint over `(document_id, sorted lines, iterations)`
- `compute_content_hash` is cardinality-sensitive: duplicate lines are preserved and affect the hash by design
- SQLite schema introduced: `document_snapshots` + `snapshot_lines` tables
- `SqliteStore` in `persistence.rs` persists both tables on every run alongside JSONL

### Consequences

- confidence formula is now auditable and testable against the implementation
- snapshot lines are directly queryable in SQLite (`SELECT * FROM snapshot_lines WHERE page = 1`)
- content_hash enables corruption detection before rehydration
- JSONL (human-readable) and SQLite (queryable) are both written on every run
- future confidence improvements (entropy, Tesseract scores, weighted purity) can be grafted in without changing the schema

---

## Future ADR Candidates

Potential upcoming decisions:

- distributed reducer execution model
- deterministic ID strategy across services
- observation schema versioning
- confidence v2: incorporate per-line Tesseract confidence scores via weighted cluster purity
- DSL-based document parsing layer
