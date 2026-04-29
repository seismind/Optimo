use crate::observation::{ObservationStatus, Severity};
use crate::profile::{IngestionProfile, ProfileKind};

#[allow(dead_code)]
fn escalate(severity: Severity) -> Severity {
    match severity {
        Severity::Low => Severity::Medium,
        Severity::Medium => Severity::High,
        Severity::High => Severity::Critical,
        Severity::Critical => Severity::Critical,
    }
}

/// Maps semantic reducer outcome to operational response priority.
///
/// Contract:
/// - `status` is semantic truth.
/// - `severity` is context-aware operational urgency.
#[allow(dead_code)]
pub fn map_severity(
    status: ObservationStatus,
    document_type: &str,
    source: &str,
    confidence: Option<f32>,
    profile: &IngestionProfile,
) -> Option<Severity> {
    if status == ObservationStatus::Converged {
        return None;
    }

    let mut severity = match status {
        ObservationStatus::Converged => Severity::Low,
        ObservationStatus::Ambiguous => Severity::Medium,
        ObservationStatus::Failed => Severity::High,
    };

    let doc = document_type.to_ascii_lowercase();
    if doc.contains("invoice")
        || doc.contains("payment")
        || doc.contains("iban")
        || doc.contains("f24")
    {
        severity = escalate(severity);
    }

    if source.contains("fixtures/") || source.contains("/fixtures/") || source.contains("test") {
        severity = match severity {
            Severity::Critical => Severity::High,
            Severity::High => Severity::Medium,
            other => other,
        };
    }

    if let Some(c) = confidence {
        if c < 0.15 {
            severity = Severity::Critical;
        } else if c < 0.35 {
            severity = escalate(severity);
        }
    }

    if profile.kind == ProfileKind::Strict && status == ObservationStatus::Failed {
        severity = Severity::Critical;
    }

    Some(severity)
}

#[cfg(test)]
mod tests {
    use super::map_severity;
    use crate::observation::{ObservationStatus, Severity};
    use crate::profile::IngestionProfile;

    #[test]
    fn converged_has_no_operational_severity() {
        let profile = IngestionProfile::tesseract();
        let out = map_severity(
            ObservationStatus::Converged,
            "invoice",
            "fixtures/sample.png",
            Some(0.99),
            &profile,
        );
        assert_eq!(out, None);
    }

    #[test]
    fn strict_failed_is_critical() {
        let profile = IngestionProfile::strict();
        let out = map_severity(
            ObservationStatus::Failed,
            "generic",
            "data/input.png",
            Some(0.42),
            &profile,
        );
        assert_eq!(out, Some(Severity::Critical));
    }

    #[test]
    fn low_confidence_ambiguous_escalates() {
        let profile = IngestionProfile::tesseract();
        let out = map_severity(
            ObservationStatus::Ambiguous,
            "invoice",
            "data/invoice_1.png",
            Some(0.20),
            &profile,
        );
        assert_eq!(out, Some(Severity::Critical));
    }
}