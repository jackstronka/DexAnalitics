//! Dip propose-only decision envelope (Faza C).
//!
//! Persists into the shared audit file `data/agent/agent_decisions.jsonl` as a free-form
//! `decision` object with `kind = "dip_propose"`. Does **not** use or extend
//! `clmm_lp_domain::agent_decision::AgentDecision` (apply-optimize only).

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Stable discriminator in the JSONL `decision` object.
pub const DIP_PROPOSE_KIND: &str = "dip_propose";

/// Default `source` field on the audit row.
pub const DIP_PROPOSE_SOURCE: &str = "dip_agent";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DipProposeAction {
    BuyDip,
    Sell,
    Hold,
    CancelOrder,
    AdjustLp,
    Skip,
}

impl DipProposeAction {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "buy_dip" | "buydip" => Ok(Self::BuyDip),
            "sell" => Ok(Self::Sell),
            "hold" => Ok(Self::Hold),
            "cancel_order" | "cancel" => Ok(Self::CancelOrder),
            "adjust_lp" | "adjustlp" => Ok(Self::AdjustLp),
            "skip" => Ok(Self::Skip),
            other => bail!("unknown dip propose action: {other}"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::BuyDip => "buy_dip",
            Self::Sell => "sell",
            Self::Hold => "hold",
            Self::CancelOrder => "cancel_order",
            Self::AdjustLp => "adjust_lp",
            Self::Skip => "skip",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DipProposeMode {
    ProposeOnly,
    Paper,
    Live,
}

impl DipProposeMode {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "propose-only" | "propose_only" | "proposeonly" => Ok(Self::ProposeOnly),
            "paper" => Ok(Self::Paper),
            "live" => Ok(Self::Live),
            other => bail!("unknown dip propose mode: {other}"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProposeOnly => "propose-only",
            Self::Paper => "paper",
            Self::Live => "live",
        }
    }
}

/// Body stored under `decision` in `agent_decisions.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DipProposeDecision {
    /// Always [`DIP_PROPOSE_KIND`] — separates from optimize `AgentDecision`.
    pub kind: String,
    pub action: DipProposeAction,
    pub mode: DipProposeMode,
    pub requires_user_confirm: bool,
    pub rationale: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sol_usd: Option<f64>,
    /// Placeholder for future Trigger create/cancel bodies (Faza E); never broadcast here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_orders: Option<serde_json::Value>,
}

impl DipProposeDecision {
    pub fn new(
        action: DipProposeAction,
        mode: DipProposeMode,
        rationale: impl Into<String>,
        refs: Vec<String>,
    ) -> Result<Self> {
        let rationale = rationale.into().trim().to_string();
        if rationale.is_empty() {
            bail!("rationale cannot be empty");
        }
        let requires_user_confirm = !matches!(mode, DipProposeMode::Live);
        let out = Self {
            kind: DIP_PROPOSE_KIND.to_string(),
            action,
            mode,
            requires_user_confirm,
            rationale,
            refs,
            zone: None,
            sol_usd: None,
            proposed_orders: None,
        };
        out.validate()?;
        Ok(out)
    }

    pub fn validate(&self) -> Result<()> {
        if self.kind != DIP_PROPOSE_KIND {
            bail!(
                "decision.kind must be `{DIP_PROPOSE_KIND}`, got {}",
                self.kind
            );
        }
        if self.rationale.trim().is_empty() {
            bail!("rationale cannot be empty");
        }
        if matches!(self.mode, DipProposeMode::ProposeOnly) && !self.requires_user_confirm {
            bail!("propose-only mode requires requires_user_confirm=true");
        }
        Ok(())
    }
}

/// One JSONL audit row (same shape as API `AgentDecisionRow`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DipAuditDecisionRow {
    pub ts_utc: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub decision: DipProposeDecision,
}

pub fn default_agent_decisions_path() -> PathBuf {
    PathBuf::from("data")
        .join("agent")
        .join("agent_decisions.jsonl")
}

/// Append a validated dip propose row to the shared agent decisions JSONL.
pub fn append_dip_propose(
    path: &Path,
    mut row: DipAuditDecisionRow,
) -> Result<DipAuditDecisionRow> {
    row.decision.validate()?;
    if row.source.trim().is_empty() {
        row.source = DIP_PROPOSE_SOURCE.to_string();
    }
    if row.ts_utc.trim().is_empty() {
        row.ts_utc = Utc::now().to_rfc3339();
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create_dir_all {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    let line = serde_json::to_string(&row).context("serialize dip audit row")?;
    writeln!(file, "{line}").with_context(|| format!("write {}", path.display()))?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn builds_propose_only_with_confirm() {
        let d = DipProposeDecision::new(
            DipProposeAction::Hold,
            DipProposeMode::ProposeOnly,
            "zone A — wait",
            vec!["config/dip/context/news-digest.md".into()],
        )
        .unwrap();
        assert_eq!(d.kind, DIP_PROPOSE_KIND);
        assert!(d.requires_user_confirm);
        assert_eq!(d.action.as_str(), "hold");
    }

    #[test]
    fn rejects_empty_rationale() {
        let err = DipProposeDecision::new(
            DipProposeAction::BuyDip,
            DipProposeMode::ProposeOnly,
            "   ",
            vec![],
        )
        .unwrap_err();
        assert!(err.to_string().contains("rationale"));
    }

    #[test]
    fn rejects_wrong_kind() {
        let mut d = DipProposeDecision::new(
            DipProposeAction::Skip,
            DipProposeMode::ProposeOnly,
            "noop",
            vec![],
        )
        .unwrap();
        d.kind = "agent_decision".into();
        assert!(d.validate().is_err());
    }

    #[test]
    fn appends_jsonl_hermetic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent_decisions.jsonl");
        let decision = DipProposeDecision::new(
            DipProposeAction::BuyDip,
            DipProposeMode::ProposeOnly,
            "zone B ladder propose",
            vec!["config/dip/zones.json".into()],
        )
        .unwrap();
        let row = DipAuditDecisionRow {
            ts_utc: "2026-11-09T12:00:00Z".into(),
            source: DIP_PROPOSE_SOURCE.into(),
            strategy_id: None,
            position_id: None,
            chain_id: None,
            session_id: Some("dip-test".into()),
            decision,
        };
        append_dip_propose(&path, row.clone()).unwrap();
        append_dip_propose(&path, row).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<_> = text.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("dip_propose"));
        assert!(lines[0].contains("buy_dip"));
    }

    #[test]
    fn parse_action_aliases() {
        assert_eq!(
            DipProposeAction::parse("BUY_DIP").unwrap(),
            DipProposeAction::BuyDip
        );
        assert_eq!(
            DipProposeAction::parse("cancel").unwrap(),
            DipProposeAction::CancelOrder
        );
    }
}
