# Fixture: OCR Hallucination

## Purpose

Tests that the reducer correctly rejects lines that appear in only one OCR variant (minority vote).

## Scenario

Three OCR variants of the same document:

| Variant         | Lines | Hallucination |
|-----------------|-------|---------------|
| original        | 5     | none          |
| high_contrast   | 5     | none          |
| rotated         | 6     | `GHOST LINE artifact` (extra) |

## Expected Reducer Behaviour

- `original` and `high_contrast` agree on 5 lines → majority (2/3)
- `GHOST LINE artifact` appears only in `rotated` → minority (1/3) → rejected from canonical output
- Convergence score should stay **≥ 8500 bps** because the two agreeing variants are an exact match

## What This Validates

- Property 2a (no phantom clusters): spurious minority lines must not create stable clusters in the output
- Property 2b (score stability): the presence of a noisy variant should not drag the score below the majority agreement threshold
- Profile confidence filtering operates **after** majority voting; this fixture isolates the voting layer

## Profile Used

`tesseract` (default) — confidence threshold 0.0 (no filtering applied), so the hallucination must be rejected purely by vote count, not filtered by confidence.
