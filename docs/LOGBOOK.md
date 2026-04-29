# LOGBOOK

Technical notes, architecture snapshots, and decisions accumulated during development.
This is the living record of what Optimo was, is, and is becoming.

---

## Architecture Snapshot (Apr 2026)

```mermaid
flowchart TD

A[main.rs<br>Bootstrap Orchestrator]

A --> B[app_state.rs<br>Application State]
A --> C[pipeline.rs<br>Async Orchestration]
A --> D[fold.rs<br>Deterministic Reducer]
A --> E[observation.rs<br>Observation Model]
A --> F[persistence.rs<br>Persistence Boundary]

C --> G[OCR Pipeline]
G --> H[Tokio spawn_blocking + Rayon par_iter]
H --> D

D --> F
F --> I[data/observations.jsonl]
```

The architecture separates:

- orchestration
- deterministic logic
- observation
- persistence

---

## Non-Negotiable Invariants

1. **Reducer purity** — the reducer must remain pure, deterministic, and free of side effects.
2. **External metadata injection** — timestamps, ids, and other non-deterministic metadata must come from the runtime layer.
3. **Persistence boundary isolation** — storage concerns must stay outside the core.
4. **Derived event model** — events must be derived from reducer results, not emitted as side effects.
5. **First-class observability** — observations are part of the system contract.

---

## Module Map (Apr 2026)

```text
src/

main.rs                # Bootstrap and runtime startup
app_state.rs           # Application state (paths, dirs, OCR language)
pipeline.rs            # Async orchestration (Tokio + Rayon boundary)
fold.rs                # Deterministic weighted positional reducer
observation.rs         # Observation model and validation rules
persistence.rs         # Persistence boundary (JSONL + SQLite)
timequake.rs           # Temporal replay core (deterministic, no I/O)
aggregate_state.rs     # Fold-derived deterministic state
snapshot.rs            # Structural projection + rehydration payload
profile.rs             # Ingestion profile (enum + config)
config.rs              # Config resolution (CLI > ENV > FILE > DEFAULT)

ocrys/
  mod.rs               # OCR facade
  tesseract.rs         # Tesseract CLI integration
  normalize.rs         # Canonical line normalization
  types.rs             # OCRDocument / OCRPage / OCRLine

scripts/

setup_data.sh          # Prepare data directories
process_all.sh         # Run all images via Docker
process_all_local.sh   # Run all images locally via cargo
```

---

## Processing Model

1. `main.rs` loads `AppState` and parses input document paths.
2. `pipeline.rs` schedules one async task per document using `JoinSet`.
3. Each document crosses into CPU workers using `spawn_blocking`.
4. Rayon executes OCR variants in parallel: `original`, `high_contrast`, `rotated`.
5. `fold.rs` merges variant outputs using inline weighted positional voting:
   - group evidence by logical position (page, line)
   - normalize text (NFC, trim, whitespace collapse, decimal comma harmonization)
   - cluster similar candidates via Jaro-Winkler
   - accumulate confidence weights incrementally
   - recompute winner and convergence/ambiguity scores after each vote
6. The final observation is appended by `persistence.rs`.

---

## Reducer Flow

```mermaid
flowchart TD

A[OCR Variant Output<br>original / contrast / rotated]

A --> B[Line Extraction]
B --> C[Positional Alignment<br>page × line index]
C --> D[Normalize<br>NFC · trim · decimal · whitespace]
D --> E[Cluster Matching<br>Jaro-Winkler ≥ 0.90]
E --> F[Inline Vote<br>accumulate confidence weight]
F --> G[Live Winner + Metrics<br>convergence · ambiguity bps]
G --> H[AggregateState]
H --> I[Observation]
I --> J[persistence.rs]
J --> K[data/observations.jsonl]
```

### Reducer Contract

```text
Input:   Vec<OCRDocument>
Output:  AggregateState

Guarantees:
  - deterministic: same input → same output
  - order-independent: stable under permutation
  - replayable: no I/O, no timestamps, no randomness
  - convergence viva: metrics updated per incoming line
```

---

## Replay Engine (Apr 2026)

### Implemented

- Deterministic replay from genesis (events ordered by timestamp + id)
- Checkpoint + tail replay (latest snapshot + events after cutoff)
- Rigorous snapshot hydration:
  - validates schema_version, document_id/source coherence, confidence match
  - fails explicitly before any reducer contamination
  - separates projection (reporting) from rehydration (fold resume)
- Equivalence test: genesis and checkpoint+tail replay produce identical final state ✓
- Failure mode tests: 5 tests guarantee no panic, no zombie state on corruption

### Test Suite

```bash
cargo test timequake::tests
```

All 5 tests pass ✓

### Next Steps (Architected)

1. Schema Evolution — versioned migrations for snapshot format
2. Integrity Hash Chain — snapshot_hash + tail_chain_hash for audit
3. Observation Replay — emit_observation in replay flow with deterministic metadata

---

## Run Notes

### Local

```bash
cargo run -- fixtures/sample.png
cargo run -- --replay
cargo run -- --replay <document_uuid>
./scripts/process_all_local.sh fixtures
```

### Docker

```bash
docker build -t optimo:latest .

docker run --rm \
  -v "$(pwd)/fixtures:/app/fixtures:ro" \
  -v "$(pwd)/data:/app/data" \
  optimo:latest /app/fixtures/sample.png

./scripts/process_all.sh fixtures
```

### Output

```
data/observations.jsonl     # append-only decision records
data/ocrys/latest/          # OCR artifacts per run
```

---

## Stack Notes

- Default OCR language: `ita`
- Persistence: JSONL (primary) + SQLite (parallel, queryable)
- `observation.rs` defines typed `OcrObservation` for structured audit
- `timequake.rs` is the canonical replay engine; no business logic, no I/O
- `profile.rs` drives normalization policy per ingestion source
- Config precedence: CLI > ENV > FILE (`optimo.yml`) > DEFAULT

---

## Architectural Direction

OCR is currently used only as a pipeline stress-test and input generator.

Long-term objective: a deterministic document analysis engine where parsing, validation, rule evaluation, and structural checks run through the same reducer/observation pipeline — without modifying the deterministic core.
