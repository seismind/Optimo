use serde::{Serialize, Deserialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReducerSnapshot {
    pub snapshot_id: Uuid,
    pub document_id: Uuid,
    pub created_at: DateTime<Utc>,

    // Stato deterministico ridotto
    pub fields: BTreeMap<String, String>,

    // Metriche convergenza
    pub confidence: f32,
    pub iterations: u32,

    // Versioning schema
    pub schema_version: u32,
}
