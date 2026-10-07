//! Verified source coordinates shared by collectors and learning policy.
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cursor {
    pub session: String,
    pub offset: u64,
}

/// A projected record recovered from the selected append-only native source.
/// Offsets are comparable only within that source's session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalRecord {
    pub reference: String,
    pub session: String,
    pub offset: u64,
    pub record: Value,
}
