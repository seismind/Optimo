# title_block_001 — Fixture Notes

## Purpose
Canonical OCR robustness benchmark.  
Uses a clean, printed Italian construction drawing title block (`cartiglio.png`, 969×477)
to establish the real perception entropy floor before escalating to noisy real-world scans.

## First real run — 2026-05-07

### Metrics (frozen)
| Metric                 | Observed | Bound           |
|------------------------|----------|-----------------|
| `convergence_score_bps`| 6666     | ≥ 6000 (floor)  |
| `ambiguity_score_bps`  | 2856     | ≤ 4000 (ceiling)|
| `collision_rate_bps`   | 2631     | ≤ 3500 (ceiling)|
| `semantic_conflicts`   | 0        | = 0             |
| iterations             | 3        | —               |

**convergence=6666 is NOT a failure.**  
It is the first real measurement of perception entropy under 3 genuinely different
perceptual transforms. It is the canonical floor for this fixture.

### Variants and their perceptual transforms
| Variant         | Transform applied                                | Dimensions     |
|-----------------|--------------------------------------------------|----------------|
| `original`      | Grayscale only                                   | 969×477        |
| `high_contrast` | Otsu binarization (threshold=187)                | 969×477        |
| `rotated`       | 60% downsample (Nearest) + upsample (Lanczos3)   | 581×286        |

### Observed OCR divergence

**Stable across all 3 variants:**
- `05/06/2020` (date)
- `DESCRIZIONE :`, `NOME FILE :`, `MATERIALE :`, `NUM. DISEGNO` (structural labels)
- `FOGLIO 1 DI 1` (sheet ID — near-stable; `rotated` produces `FOGLIO 1 DI i`)

**Divergence caused by rotated/downsampled variant:**
- `MASSA :` disappears entirely — field too small after 60% downsample
- `FINITURA SUP :` disappears entirely — same cause
- `SCALE : 1 : 2` → `de:` (original/high_contrast) → `1 dor)` (rotated) — small text degrades under both contrast change and resize
- Final digit confusion: `FOGLIO 1 DI 1` → `FOGLIO 1 DI i` — OCR misreads `1` as `i` at reduced resolution

**Known structural gaps (no variant detects):**
- `LAVORAZIONI :` — salmon/orange background; low contrast survives Otsu binarization but text not picked up by Tesseract
- `REV` cell — not extracted by any variant

### Root-cause analysis
The 60% downsample variant (`rotated`) is the primary divergence driver.  
It collapses fine-pitch fields (scale, massa, finitura) below the minimum spatial frequency Tesseract requires for reliable character segmentation.  
`high_contrast` and `original` agree on the main labels but both lose `SCALE` and `LAVORAZIONI` due to typographic weight and background color.

### Entropy interpretation
- `collision_rate=2631` means ~26% of votes merged into an existing cluster.  
  This is expected: all 3 variants agree on dates, labels, and the sheet number, so those form dense clusters. The remaining 74% are singleton or split clusters from the degraded fields above.
- `ambiguity=2856` reflects genuine uncertainty — the reducer correctly refuses to pick a winner for corrupted fields rather than hallucinating one.

## Images
| File              | Description                                      |
|-------------------|--------------------------------------------------|
| `cartiglio.png`   | Full-resolution title block (root `fixtures/`)   |

## Preprocessed output (generated — `data/ocrys/latest/`)
| File                       | Variant          |
|----------------------------|------------------|
| `preproc_original.png`     | `original`       |
| `preproc_high_contrast.png`| `high_contrast`  |
| `preproc_rotated.png`      | `rotated`        |

## Escalation path
1. title_block_001 — clean printed block ← **you are here** (converged, fixture frozen)
2. title_block_002 — same block, low-resolution scan
3. title_block_003 — block with stamp/overprint noise
4. full_drawing_001 — full A1 drawing, single page

## Status
- [x] `cartiglio.png` fixture established
- [x] `expected.json` populated with real observed metrics
- [x] Automated semantic stability test written (`fold_ocr_stability.rs`)
- [ ] Replay equivalence verified end-to-end via `--replay` flag
- [ ] `title_block_002` created (next escalation step)
