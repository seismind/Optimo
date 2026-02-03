# Optimo

Optimo is a Rust backend scaffold built on top of Axum.

It is designed with an **infrastructure-first** approach, where clarity,
explicit wiring and long-term maintainability come before convenience.

## Overview

Optimo provides:
- a minimal and explicit `main` entry point
- a clear separation between orchestration and application logic
- modular routing
- async-first execution
- fast failure over partial recovery

There is no hidden behavior and no implicit magic.

## Architecture Principles

- The `main` function orchestrates, then steps aside
- Configuration is loaded before the application starts
- State is built explicitly and passed where needed
- Infrastructure concerns are isolated from domain logic

## Current Features

- Axum HTTP server
- Health check endpoint
- Modular route organization
- Tokio-based async runtime
- Tracing-ready bootstrap

## Non-Goals

Optimo is **not yet**:
- a full-featured framework
- a tutorial project
- an opinionated domain application

The long-term goal is to evolve Optimo into a structured backend framework,
once its architectural principles and boundaries are fully validated.

## Status

Early stage.
The focus is on structure, not surface area.

## OCR Processing Model

Optimo uses a deterministic, industrial OCR pipeline designed for reliability,
not heuristics or machine learning.

### Parallel Map (Rayon)

OCR is CPU-bound. For this reason, Optimo deliberately exits the async runtime
(Tokio) and uses Rayon to distribute OCR variants across all available CPU cores.

Each document is processed using multiple OCR variants (contrast, rotation, etc.)
in parallel.

### Deterministic Reduce

OCR results are merged using a deterministic reduce step based on:

- textual similarity (token overlap)
- confidence comparison
- explicit disagreement preservation

No probabilistic or learning-based behavior is involved.

### StateBridge: Selective Memory

Only meaningful outcomes (conflicts, low confidence, convergence failures)
are persisted.

The database is treated as a memory of *events worth remembering*,
not a raw data dump.

---

Built with Rust, Axum and discipline.
