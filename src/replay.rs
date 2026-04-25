//! Legacy alias for replay/time-travel APIs.
//!
//! Deprecated: use [crate::timequake_core] directly for all new code.
//! This module intentionally contains no logic; it only re-exports the
//! canonical replay engine and types.

#[deprecated(note = "replay is legacy alias; use crate::timequake_core")]
pub use crate::timequake_core::{
    EquivalenceReport,
    ReplayInput,
    ReplayResult,
    TimequakeCore,
};
