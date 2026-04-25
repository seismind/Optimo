# fixture: currency_variants

## Business Significance

Currency amounts can appear in multiple semantic forms:
- **€1000.00** — symbol prefix (common in EU)
- **1000 EUR** — code suffix (international)
- **1000€** — symbol suffix (alternative EU)

Despite surface differences, they represent the same value: 1000 EUR.

## Expected Behavior

The reducer should:
1. Normalize via NFC (no combining characters)
2. Recognize that "1000" is the core numeric value
3. Not cluster €1000, 1000 EUR, 1000 USD as identical (USD is different currency)
4. Achieve moderate convergence (6000+ bps) due to variant symbol placement

## Failure Modes

- ❌ All three treated as identical: semantic equivalence is too aggressive (1000 USD ≠ 1000 EUR)
- ❌ All three treated as different: no recognition of symbol exchange
- ❌ Low convergence (< 5000): suggests OCR variants too dissimilar (check fixture quality)

## Implementation Notes

- 3 variants intentionally shuffle symbol placement
- Numeric value (1000) is preserved across all three
- USD vs EUR distinction is important (not equivalent)
- Tests should verify clustering of EUR variants while rejecting USD
