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
state_bridge.rs
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

## Future ADR Candidates

Potential upcoming decisions:

- introduction of SQLite storage backend
- distributed reducer execution model
- deterministic ID strategy across services
- observation schema versioning
- DSL-based document parsing layer
