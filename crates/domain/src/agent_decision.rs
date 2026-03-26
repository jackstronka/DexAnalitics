//! Structured output from an external AI decision layer (approve/reject + optional `OptimizeResultFile`).

use crate::optimize_result::OptimizeResultFile;
use serde::{Deserialize, Serialize};

/// Agent decision document (e.g. LLM) consumed by the API `apply-optimize-result` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDecision {
    /// Schema version; bump when fields change.
    pub schema_version: u32,
    /// When `true`, `optimize_result` must be present and is applied to the executor.
    pub approved: bool,
    /// Optional human-readable rationale (audit / logs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Full grid result to apply when `approved` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimize_result: Option<OptimizeResultFile>,
}

impl AgentDecision {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_fields() {
        let j = r#"{"schema_version":1,"approved":false,"typo_field":true}"#;
        assert!(serde_json::from_str::<AgentDecision>(j).is_err());
    }
}
