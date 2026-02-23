use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Final status of the reducer for a given field extraction.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ObservationStatus {
    Converged,
    Ambiguous,
    Failed,
}

/// Severity is only meaningful when status != Converged.
/// Keep it small and stable (for dashboards / alerts).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

/// Minimal, durable record: one row per *decision*, not per OCR attempt.
///
/// This is what is allowed to enter SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrObservation {
    /// Unique id for this observation record.
    pub id: Uuid,

    /// The logical document id (not the filename). Stable across re-runs.
    pub document_id: Uuid,

    /// Optional: page index (1-based is human-friendly).
    pub page: Option<u32>,

    /// The logical target field, e.g. "title_block.project_name".
    pub field: String,

    /// Final chosen value (if any). None means "no reliable value".
    pub value: Option<String>,

    /// Final confidence after reduce (0.0..=1.0). None if not computed.
    pub confidence: Option<f32>,

    /// How many reduce iterations were needed (useful for convergence metrics).
    pub iterations: u32,

    /// Reducer decision outcome.
    pub status: ObservationStatus,

    /// Only set when status != Converged.
    pub severity: Option<Severity>,

    /// Why it failed / stayed ambiguous (short, stable codes).
    /// Examples: "low_confidence", "conflict", "geometry_rejected".
    pub reason_code: Option<String>,

    /// Human-readable note (short). Keep this optional and bounded.
    pub note: Option<String>,

    /// When the observation was recorded.
    pub created_at: DateTime<Utc>,
}

impl OcrObservation {
    /// Constructor that sets ids and timestamp for you.
    pub fn new(
        document_id: Uuid,
        field: impl Into<String>,
        status: ObservationStatus,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            document_id,
            page: None,
            field: field.into(),
            value: None,
            confidence: None,
            iterations: 0,
            status,
            severity: None,
            reason_code: None,
            note: None,
            created_at: Utc::now(),
        }
    }

    /// Enforces invariants before persisting (so SQLite doesn't become a trash bin).
    pub fn validate(&self) -> anyhow::Result<()> {
        // Confidence must be within [0, 1] if present.
        if let Some(c) = self.confidence {
            if !(0.0..=1.0).contains(&c) {
                anyhow::bail!("confidence out of range: {}", c);
            }
        }

        // If converged, we should not store severity/reason unless you really want it.
        if self.status == ObservationStatus::Converged {
            if self.severity.is_some() || self.reason_code.is_some() {
                anyhow::bail!("converged observation must not carry severity/reason");
            }
        } else {
            // If not converged, we *must* have at least a reason_code (cheap taxonomy).
            if self.reason_code.is_none() {
                anyhow::bail!("non-converged observation requires reason_code");
            }
        }

        Ok(())
    }
}