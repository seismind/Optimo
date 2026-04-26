# ARCHITECTURE

## Core Architectural Invariants

This document defines the non-negotiable invariants of the Optimo core.

These rules exist to preserve determinism, replayability, observability, and long-term evolvability.

---

## 1. The reducer must remain pure

The reducer is the deterministic computation core of the system.

It must:

- accept explicit input state and input data
- return deterministic output
- perform no I/O
- read no global mutable state
- generate no timestamps, UUIDs, randomness, or side effects

### Allowed

```text
(State, Input) -> ReducerResult
```

### Forbidden

```text
(State, Input) -> ReducerResult + side effects
```

or any internal use of:

- current time
- random generators
- filesystem writes
- network access
- persistence calls

### Why

If the reducer stops being pure, replayability and exact reproducibility are lost.

---

## 2. Non-deterministic metadata must be injected from the runtime

Any metadata that depends on time, identity generation, or execution context must be provided by the outer runtime layer.

This includes:

- timestamps
- event ids
- correlation ids
- execution ids
- clock-derived metadata
- random values

### Correct model

```text
Runtime -> injects metadata -> Event Builder / Bridge
Reducer -> computes deterministic result
```

### Incorrect model

```text
Reducer -> generates timestamp / UUID / random metadata
```

### Why

Replayable systems require the same inputs to produce the same outputs bit-for-bit.

---

## 3. Persistence must stay outside the core

The core computation layer must never know how state or events are stored.

Persistence belongs to the boundary layer.

Current boundary:

```text
persistence.rs
```

Current storage targets:

- observations.jsonl
- snapshots.jsonl
- events.jsonl

Future storage may include:

- SQLite
- PostgreSQL
- object storage

### Rule

The reducer and observation model must not depend on storage format, file paths, or database engines.

### Why

This keeps the core stable while persistence evolves independently.

---

## 4. Events are derived from results, not emitted as side effects of computation

The reducer may indicate that an event should exist, but the reducer itself must not perform event emission or persistence.

### Correct flow

```text
Input
  -> Reducer
  -> ReducerResult
  -> Runtime decides event construction
  -> Bridge persists
```

### Example

The reducer may return:

```text
ReducerResult {
  state,
  observation,
  event_signal
}
```

The runtime layer then decides whether to construct and persist an `EventPayload`.

### Why

This preserves separation between deterministic computation and side effects.

---

## 5. Observability is a first-class output of the system

Observations are not debug leftovers.
They are part of the system contract.

An observation should explain:

- what decision was made
- why it was made
- whether the result converged, was ambiguous, or failed
- what information is relevant for audit and diagnosis

### Rule

Observation records must remain structured, explicit, and stable enough to support:

- audit
- debugging
- replay analysis
- future validation layers

### Why

Without structured observation, deterministic computation becomes opaque.
With it, the system becomes inspectable and explainable.

---

## Architectural Summary

Optimo is based on a strict separation of concerns:

```text
Input generation
  -> deterministic computation
  -> observation
  -> event construction
  -> persistence boundary
  -> storage
```

In practical terms:

- OCR is an input generator
- the reducer is the deterministic core
- observation explains decisions
- events are derived from reducer results
- state_bridge isolates persistence
- storage remains replaceable

---

## Formal Triad Model

The computation contract is formalized as a triad:

- deterministic fold: `ReducerState`
- structural projection: `ReducerSnapshot`
- semantic projection: `OcrObservation`

See the mathematical formalization in:

- `docs/TRIAD_FORMALISM.md`

---

## Design Goal

The long-term goal is not simply to process OCR.

The long-term goal is to preserve a computation model where:

- the core is deterministic
- replay is exact
- side effects are isolated
- decisions are observable
- persistence is replaceable

This is the foundation for a scalable document intelligence engine.
