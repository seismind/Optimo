# OPTIMO

Experimental deterministic document pipeline in Rust.

Built to explore repeatable workflows where noisy inputs (OCR today, structured docs tomorrow) are processed through a deterministic core.

## Current Focus

- reducer determinism
- replayability
- pipeline stability
- adversarial tests

## Stack

Rust • Tokio • Rayon • Tesseract • JSONL

## Why

Many document processes depend on unclear transformations and hard-to-audit decisions.

Optimo explores a simpler model:

Input → Normalize → Reduce → Observe → Persist

## Status

Active prototype under test.

See [docs/](docs/) for architecture and decisions.
Full technical notes and module history: [docs/LOGBOOK.md](docs/LOGBOOK.md).
