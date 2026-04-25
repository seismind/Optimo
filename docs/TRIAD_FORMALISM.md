# Triad Formalism (State, Snapshot, Observation)

This document formalizes the Optimo triad as a deterministic computation model.

## 1) Sets and Spaces

Let:

- $D$ be the set of OCR documents (possibly multi-page).
- $S$ be the reducer runtime state space (`ReducerState`).
- $M_s$ be snapshot metadata space (runtime-generated):
  - `snapshot_id`
  - `created_at`
- $P$ be the snapshot projection space (`ReducerSnapshot`).
- $M_o$ be observation metadata space (runtime-generated):
  - `observation_id`
  - `created_at`
- $O$ be the semantic observation space (`OcrObservation`).

## 2) Deterministic Core Fold

The reducer fold is a pure function over explicit inputs:

$$
R: S \times D \rightarrow S
$$

and for a sequence $\langle d_1, \dots, d_n \rangle$:

$$
S_n = R(\dots R(R(S_0, d_1), d_2) \dots, d_n)
$$

where $S_0$ is the neutral initial state.

### Invariant A — Reducer Purity

$R$ must not depend on implicit context (clock, randomness, I/O, global mutable state).

## 3) Structural Projection (Snapshot)

Snapshot creation is a projection with explicit runtime metadata injection:

$$
\Pi: S \times M_s \rightarrow P
$$

`ReducerSnapshot` is structural: it captures deterministic state-derived data plus injected runtime metadata.

### Invariant B — Metadata Injection Boundary

All non-deterministic fields in $P$ come from $M_s$, never from reducer internals.

## 4) Semantic Projection (Observation)

Observation generation is split in two parts:

1. deterministic decision shape from state
2. runtime metadata attachment

Formally:

$$
\Omega: S \times M_o \rightarrow O
$$

with status function:

$$
\sigma: S \rightarrow \{\text{Converged}, \text{Ambiguous}, \text{Failed}\}
$$

and observation payload constrained by validation rules.

### Invariant C — Semantic/Structural Separation

- Snapshot ($P$) is structural and replay-oriented.
- Observation ($O$) is semantic and audit/explainability-oriented.

## 5) Persistence Boundary

Persistence is modeled as external boundary morphism:

$$
B: P \cup O \rightarrow \text{Storage}
$$

where $B$ is implemented by `state_bridge.rs`.

### Invariant D — Core/Infrastructure Decoupling

No storage concerns are allowed inside $R$, $\Pi$, or $\Omega$.

## 6) Replay Model (Target)

Replay reconstructs state by applying the fold from a checkpoint:

$$
S_{k+n} = R^n(S_k, \langle d_{k+1}, \dots, d_{k+n} \rangle)
$$

where $S_k$ can be loaded from a persisted snapshot $P_k$.

This enables bounded replay cost and deterministic re-evaluation.

### Current implementation status

`src/replay.rs` provides the initial replay skeleton:

- deterministic event ordering (`timestamp`, then `id`)
- genesis replay from event stream
- checkpoint + tail replay (`latest snapshot` + events after snapshot cutoff)

Snapshot now explicitly separates:

- projection data (`fields`, `confidence`, `iterations`)
- rehydration data (minimal canonical reducer internals for deterministic resume)

## 7) Convergence Criterion (Current Implementation)

Current status classifier uses normalized basis-point scores in state:

- `convergence_score_bps` in $[0, 10\,000]$
- `ambiguity_score_bps = 10\,000 - convergence_score_bps`

Decision thresholds:

- Converged: ambiguity $\le 1000$ and convergence $\ge 9000$
- Ambiguous: convergence $\ge 5000$ and not converged
- Failed: otherwise

These thresholds define the current practical approximation of $\sigma$.

## 8) Contract Summary

The triad contract is:

$$
\langle R, \Pi, \Omega \rangle
$$

with strict boundary guarantees:

1. $R$ pure and deterministic
2. metadata injected externally via $M_s$ and $M_o$
3. persistence delegated to $B$
4. snapshot structural, observation semantic
