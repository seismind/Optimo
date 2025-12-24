# Optimo — Manifesto

Optimo exists to resist entropy.

Most backends rot not because of scale,
but because of unclear responsibility.

## First Principle: Orchestration over Action

The `main` function does not work.
It prepares the system, then disappears.

If `main` contains logic, the architecture is already compromised.

## Second Principle: Explicit State

There is no hidden state.
No global mutation.
No invisible dependency.

If something is needed, it is passed.
If it is passed, it is owned or borrowed consciously.

## Third Principle: Fast Failure

A backend that limps is more dangerous than one that crashes.

Configuration errors are fatal.
Observability must exist before functionality.
Partial startup is not allowed.

## Fourth Principle: Infrastructure First

Routes, state, configuration and observability
exist before features.

Features are temporary.
Structure is permanent.

## Fifth Principle: No Decorative Code

Every line must justify its existence.

If a module exists:
- it has a single responsibility
- it has a clear boundary
- it can be removed without collapsing the system

## Final Note

Optimo is not built to impress.
It is built to endure.
It may become a framework.
But only after it has proven it can stay simple