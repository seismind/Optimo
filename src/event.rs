use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::observation::OcrObservation;
use crate::ocrys::types::OCRLine;

/// Current event schema version.
/// Increment when the Event structure changes in a breaking way.
pub const SCHEMA_VERSION: u32 = 1;

/// A single immutable fact emitted by the OCR pipeline.
/// Events are the raw input to the reducer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id:             Uuid,
    pub schema_version: u32,
    pub timestamp:      u64,
    pub source:         EventSource,
    pub payload:        EventPayload,
    pub confidence:     f32,
}

/// Where the event originated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventSource {
    OcrVariant {
        variant: String,
        page: usize,
        line_index: usize,
    },
    Reducer,
}

/// What the event carries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventPayload {
    OcrLine(OCRLine),
    Observation(OcrObservation),
}

impl Event {
    pub fn with_metadata(
        id: Uuid,
        timestamp: u64,
        source: EventSource,
        payload: EventPayload,
        confidence: f32,
    ) -> Self {
        Event {
            id,
            schema_version: SCHEMA_VERSION,
            timestamp,
            source,
            payload,
            confidence,
        }
    }

    #[allow(dead_code)]
    pub fn from_ocr_line(
        line: OCRLine,
        variant: &str,
        page: usize,
        line_index: usize,
    ) -> Self {
        let confidence = line.confidence.unwrap_or(0.0);
        Event::with_metadata(
            Uuid::now_v7(),
            now_secs(),
            EventSource::OcrVariant {
                variant: variant.to_string(),
                page,
                line_index,
            },
            EventPayload::OcrLine(line),
            confidence,
        )
    }

    #[allow(dead_code)]
    pub fn from_observation(observation: OcrObservation, source: EventSource) -> Self {
        let confidence = observation.confidence.unwrap_or(0.0);
        Event::with_metadata(
            Uuid::now_v7(),
            now_secs(),
            source,
            EventPayload::Observation(observation),
            confidence,
        )
    }

    #[allow(dead_code)]
    pub fn from_observation_with_metadata(
        observation: OcrObservation,
        source: EventSource,
        id: Uuid,
        timestamp: u64,
    ) -> Self {
        let confidence = observation.confidence.unwrap_or(0.0);
        Event::with_metadata(
            id,
            timestamp,
            source,
            EventPayload::Observation(observation),
            confidence,
        )
    }
}

#[allow(dead_code)]
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
