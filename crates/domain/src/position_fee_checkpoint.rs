use serde::{Deserialize, Serialize};

/// Fee accounting mode used by strategy/runtime and readiness tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PositionTruthMode {
    /// Existing fee heuristics (Tier1/Tier2 style).
    #[default]
    Heuristic,
    /// Position-truth accounting based on per-position checkpoint timeline (Tier3).
    PositionTruth,
}

/// Source quality marker for values recorded in checkpoint rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointSource {
    /// Value comes directly from on-chain read.
    Onchain,
    /// Value is derived from local pipeline/model.
    Derived,
    /// Value missing at capture time (kept explicit for auditability).
    Missing,
}

/// Minimal checkpoint row for position-level fee timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionFeeCheckpoint {
    pub ts_utc: String,
    pub position: String,
    pub pool: String,
    pub event_type: String,
    pub tick_lower: i32,
    pub tick_upper: i32,
    pub liquidity: String,
    pub fees_owed_a: u64,
    pub fees_owed_b: u64,
    pub collected_a: u64,
    pub collected_b: u64,
    pub source: CheckpointSource,
}
