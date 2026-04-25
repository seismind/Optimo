# Invoice Table Sample

## Business Context

This is a realistic invoice document with an itemized table. It tests the following OCR challenges:

1. **Decimal notation variance**: Italian format uses comma (45,50), English uses dot (45.50).
   - **Normalization required**: The reducer must recognize these as the same value.
   - After canonicalization (NFC + decimal comma→dot), both become "45.50".

2. **Table alignment**: Multi-row itemized table with columnar structure.
   - OCR variants may drift on column boundaries.
   - Reducer should still recognize rows as distinct lines.

3. **Text abbreviation**: "Support & Maintenance" vs "Support and Maint."
   - Rotated OCR may truncate or abbreviate due to skew.
   - Not identical, but semantically related (same line item).
   - Lower confidence on this specific line.

4. **Header variation**: "TOTAL DUE" vs "TOTAL"
   - Different variants capture different header text.
   - Rotated variant simplifies.

## Expected Reducer Behavior

- **Convergence**: High agreement on core invoice fields (number, date, amounts).
- **Ambiguity**: Medium ambiguity due to formatting differences.
- **Status**: `Ambiguous` — safe to process but flag for manual review of line items.

## Regression Scenarios

If this fixture starts failing:
- **Score drops**: Tesseract behavior changed or pre-processing shifted.
- **Clusters merge**: Divergent lines merged incorrectly (overly aggressive normalization).
- **Clusters explode**: Identical lines split into separate clusters (insufficient normalization).

## Related Topics

- See `docs/DECISIONS.md` ADR-0009 for canonicalization rules.
- See `src/profile.rs` for normalization flags per OCR source.
