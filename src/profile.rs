/// Runtime-selectable ingestion profile.
///
/// Design: enum + config struct (data-driven policy, no dynamic dispatch).
///
/// # Selection order (highest precedence first)
///   1. CLI flag   `--profile <name>`
///   2. Env var    `OPTIMO_PROFILE=<name>`
///   3. Default    `tesseract`
///
/// # Adding a new profile
///   1. Add a variant to `ProfileKind`.
///   2. Add a factory method on `IngestionProfile`.
///   3. Handle the new name in `ProfileKind::from_str`.
///
/// See docs/DECISIONS.md ADR-0010.
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ProfileKind — the discriminant used in logs, snapshots, observations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProfileKind {
    /// Standard Tesseract OCR pipeline. Permissive, all normalizations on.
    #[default]
    Tesseract,
    /// Future Carbo AI source. Strict dedup, high confidence bar.
    Carbo,
    /// Legacy batch import (CSV / scanned archives). Permissive, lowercase off.
    LegacyImport,
    /// Strict validation mode. Used for acceptance tests and QA.
    Strict,
}

impl ProfileKind {
    /// Parse a profile name from a CLI arg or env value.
    /// Case-insensitive. Returns `None` for unknown names.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "tesseract" => Some(Self::Tesseract),
            "carbo" => Some(Self::Carbo),
            "legacy" | "legacy_import" => Some(Self::LegacyImport),
            "strict" => Some(Self::Strict),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tesseract => "tesseract",
            Self::Carbo => "carbo",
            Self::LegacyImport => "legacy_import",
            Self::Strict => "strict",
        }
    }
}

impl std::fmt::Display for ProfileKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// IngestionProfile — the full policy config
// ---------------------------------------------------------------------------

/// Data-driven ingestion policy.
///
/// Every field controls a specific behaviour in the normalization and
/// reduction pipeline. No `if source == "tesseract"` anywhere — only
/// `if profile.allow_duplicate_positions` etc.
///
/// Serializable: persisted in `RawObservation.profile_used` so every
/// observation carries the exact policy that produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IngestionProfile {
    /// Which named profile this config corresponds to (for logs / audit).
    pub kind: ProfileKind,

    /// Allow two lines at the same (page, line) position.
    /// Tesseract: true (multi-variant OCR intentionally overlaps).
    /// Carbo / Strict: false.
    pub allow_duplicate_positions: bool,

    /// Collapse multiple consecutive whitespace characters to a single space.
    pub normalize_whitespace: bool,

    /// Lowercase the entire text.
    /// WARNING: disables case-sensitive tokens like "TRF", "EUR", "NL".
    /// Disable for document sources where case carries semantic meaning.
    pub normalize_case: bool,

    /// Apply Unicode NFC normalization.
    /// Almost always true; disable only for byte-exact legacy sources.
    pub unicode_normalize: bool,

    /// Minimum OCR line confidence threshold [0.0, 1.0].
    /// Lines below this threshold are dropped during normalization.
    /// Set to 0.0 to disable filtering.
    pub min_confidence: f64,
}

impl Default for IngestionProfile {
    fn default() -> Self {
        Self::tesseract()
    }
}

impl IngestionProfile {
    // ------------------------------------------------------------------
    // Factory defaults — the named profiles
    // ------------------------------------------------------------------

    /// Standard Tesseract OCR pipeline.
    ///
    /// - Permissive duplicate positions (3 variants intentionally overlap)
    /// - All normalizations enabled
    /// - Moderate confidence bar
    pub fn tesseract() -> Self {
        Self {
            kind: ProfileKind::Tesseract,
            allow_duplicate_positions: true,
            normalize_whitespace: true,
            normalize_case: false, // "TRF", "EUR" etc are meaningful
            unicode_normalize: true,
            min_confidence: 0.55,
        }
    }

    /// Carbo AI source (planned).
    ///
    /// - Strict: no duplicate positions
    /// - High confidence bar
    /// - Case preserved (AI output is already canonical)
    pub fn carbo() -> Self {
        Self {
            kind: ProfileKind::Carbo,
            allow_duplicate_positions: false,
            normalize_whitespace: true,
            normalize_case: false,
            unicode_normalize: true,
            min_confidence: 0.80,
        }
    }

    /// Legacy batch import.
    ///
    /// - Permissive (archives may have noisy positions)
    /// - No case normalization (preserve original casing)
    /// - Low confidence bar (no confidence metadata in old archives)
    pub fn legacy_import() -> Self {
        Self {
            kind: ProfileKind::LegacyImport,
            allow_duplicate_positions: true,
            normalize_whitespace: true,
            normalize_case: false,
            unicode_normalize: true,
            min_confidence: 0.0,
        }
    }

    /// Strict validation mode.
    ///
    /// - No duplicates, high confidence, all normalizations.
    /// - Used for acceptance tests and QA runs.
    pub fn strict() -> Self {
        Self {
            kind: ProfileKind::Strict,
            allow_duplicate_positions: false,
            normalize_whitespace: true,
            normalize_case: false,
            unicode_normalize: true,
            min_confidence: 0.95,
        }
    }

    // ------------------------------------------------------------------
    // Runtime selection
    // ------------------------------------------------------------------

    /// Resolve profile from CLI arg or env var.
    ///
    /// **Deprecated**: prefer `config::ResolvedConfig::resolve()` which also
    /// handles config file and tracks the source of every value.
    /// Kept for call-sites that don't need full resolution.
    #[allow(dead_code)]
    pub fn from_cli_or_env(cli_override: Option<&str>) -> Self {
        let name = cli_override
            .map(|s| s.to_string())
            .or_else(|| std::env::var("OPTIMO_PROFILE").ok());

        match name.as_deref() {
            Some(s) => match ProfileKind::from_str(s) {
                Some(kind) => Self::for_kind(kind),
                None => {
                    eprintln!(
                        "warning: unknown profile {:?}, falling back to 'tesseract'",
                        s
                    );
                    Self::tesseract()
                }
            },
            None => Self::tesseract(),
        }
    }

    /// Build default config for a given `ProfileKind`.
    pub fn for_kind(kind: ProfileKind) -> Self {
        match kind {
            ProfileKind::Tesseract => Self::tesseract(),
            ProfileKind::Carbo => Self::carbo(),
            ProfileKind::LegacyImport => Self::legacy_import(),
            ProfileKind::Strict => Self::strict(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_roundtrip() {
        for (input, expected) in [
            ("tesseract", ProfileKind::Tesseract),
            ("Tesseract", ProfileKind::Tesseract),
            ("carbo", ProfileKind::Carbo),
            ("strict", ProfileKind::Strict),
            ("legacy", ProfileKind::LegacyImport),
            ("legacy_import", ProfileKind::LegacyImport),
        ] {
            assert_eq!(
                ProfileKind::from_str(input),
                Some(expected),
                "failed for input {:?}",
                input
            );
        }
    }

    #[test]
    fn unknown_profile_returns_none() {
        assert_eq!(ProfileKind::from_str("unknown_xyz"), None);
    }

    #[test]
    fn strict_disallows_duplicates() {
        assert!(!IngestionProfile::strict().allow_duplicate_positions);
    }

    #[test]
    fn tesseract_allows_duplicates() {
        assert!(IngestionProfile::tesseract().allow_duplicate_positions);
    }

    #[test]
    fn for_kind_roundtrip() {
        for kind in [
            ProfileKind::Tesseract,
            ProfileKind::Carbo,
            ProfileKind::LegacyImport,
            ProfileKind::Strict,
        ] {
            assert_eq!(IngestionProfile::for_kind(kind).kind, kind);
        }
    }

    #[test]
    fn default_is_tesseract() {
        assert_eq!(IngestionProfile::default().kind, ProfileKind::Tesseract);
    }

    #[test]
    fn serialization_roundtrip() {
        let p = IngestionProfile::carbo();
        let json = serde_json::to_string(&p).expect("serialize");
        let back: IngestionProfile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.kind, ProfileKind::Carbo);
        assert!(!back.allow_duplicate_positions);
    }
}
