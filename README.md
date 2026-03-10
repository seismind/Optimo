# OPTIMO

Deterministic OCR pipeline in Rust with Tokio + Rayon orchestration and Tesseract execution.

The project explores a strongly decoupled processing architecture where OCR acts only as an input generator for a deterministic pipeline.

---

# Architecture

```mermaid
flowchart TD

A[main.rs<br>Bootstrap Orchestrator]

A --> B[state.rs<br>Application State]
A --> C[task.rs<br>Async Orchestration]
A --> D[reducer.rs<br>Deterministic Reducer]
A --> E[observation.rs<br>Observation Model]
A --> F[state_bridge.rs<br>Persistence Boundary]

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

This allows the core logic to remain deterministic and replayable.

---

# Project Structure

```text
src/

main.rs                # Bootstrap and runtime startup

state.rs               # Application state (paths, dirs, OCR language)

task.rs                # Async orchestration (Tokio + Rayon boundary)

reducer.rs             # Deterministic merge/vote logic

observation.rs         # Observation model and validation rules

state_bridge.rs        # Persistence boundary (JSONL today)

ocrys/
  mod.rs               # OCR facade
  tesseract.rs         # Tesseract CLI integration
  normalize.rs         # (legacy/experimental normalization)
  types.rs             # OCRDocument / OCRPage / OCRLine

scripts/

setup_data.sh          # Prepare data directories and ownership
process_all.sh         # Run all images via Docker image
process_all_local.sh   # Run all images locally via cargo
```

---

# Processing Model

1. `main.rs` loads `AppState` and parses input document paths.

2. `task.rs` schedules one async task per document using `JoinSet`.

3. Each document crosses into CPU workers using `spawn_blocking`.

4. Rayon executes OCR variants in parallel:

   - `original`
   - `high_contrast`
   - `rotated`

5. `reducer.rs` merges variant outputs deterministically using fuzzy clustering:

   - line alignment by position (line index as positional proxy)
   - similarity scoring via `strsim::jaro_winkler`
   - stable winner selection by cluster size
   - deterministic tie-break rules

6. The final observation is appended by `state_bridge.rs` to:

```
data/observations.jsonl
```

---

# Deterministic Reducer Logic

```mermaid
flowchart TD

A[OCR Variant Output<br>original / contrast / rotated]

A --> B[Line Extraction]

B --> C[Positional Alignment<br>group by line index]

C --> D[Fuzzy Similarity Check<br>Jaro-Winkler]

D --> E[Cluster Formation]

E --> F[Cluster Size Ranking]

F --> G[Tie-break Rules<br>deterministic]

G --> H[Winning Text Line]

H --> I[Final Merged OCR Output]

I --> J[Observation Record]

J --> K[State Bridge]

K --> L[data/observations.jsonl]
```

## Reducer Algorithm

The reducer merges OCR variants deterministically.

Processing steps:

1. Extract lines from each OCR variant.
2. Align candidate lines by position (current implementation: line index).
3. Compare textual similarity using Jaro-Winkler.
4. Build clusters of similar lines across OCR variants.
5. Rank clusters by size.
6. Select the winning line using deterministic tie-break rules.
7. Produce a final merged output.
8. Emit an observation record describing the decision.

## Reducer Contract

```text
Input:
  OCRVariant[]

Output:
  DeterministicMergedOCR

Guarantees:
  - deterministic output
  - stable tie-break rules
  - replayable decisions
```

---

# Runtime and Dependencies

Core stack:

- **Language / Runtime**: Rust + Tokio async runtime
- **OCR Engine**: Tesseract CLI
- **Parallel Compute**: Rayon
- **Serialization**: serde + serde_json
- **String Similarity**: strsim (Jaro-Winkler)

---

# Run Locally

Requires a local Tesseract installation.

```
cargo run -- fixtures/sample.png
```

Batch process a folder:

```
./scripts/process_all_local.sh fixtures
```

---

# Run with Docker (Recommended)

Build image:

```
docker build -t optimo:latest .
```

Run one file:

```
mkdir -p data

docker run --rm \
-v "$(pwd)/fixtures:/app/fixtures:ro" \
-v "$(pwd)/data:/app/data" \
optimo:latest /app/fixtures/sample.png
```

Run all images in a folder:

```
./scripts/process_all.sh fixtures
```

---

# Output

```
data/observations.jsonl
```

Append-only decision records (one JSON object per line).

Artifacts generated during runs:

```
data/ocrys/latest/
```

Example record:

```json
{"decision":"ocr_converged","lines":3,"preview":"hello world ocr test 2024 optimo pipeline ","source":"/app/fixtures/sample.png"}
```

---

# Notes

- Default OCR language in `AppState` is currently `ita`.
- `observation.rs` already defines richer typed observations (`OcrObservation`) for the next persistence phase.
- JSONL is the current persistence backend.
- SQLite is planned and can be introduced behind `state_bridge.rs` without changing reducer or orchestration logic.

---

# Architectural Direction

OCR is currently used only as a **pipeline stress-test and input generator**.

The long-term objective is a deterministic document analysis engine where:

- parsing
- validation
- rule evaluation
- structural checks

can run through the same reducer/observation pipeline.

This design allows the system to evolve without modifying the deterministic core.
