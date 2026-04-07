//! Alert types and structures.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

/// Alert severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertLevel {
    /// Informational alert.
    Info,
    /// Warning alert.
    Warning,
    /// Critical alert requiring immediate attention.
    Critical,
}

impl AlertLevel {
    /// Returns the emoji for this alert level.
    #[must_use]
    pub fn emoji(&self) -> &'static str {
        match self {
            Self::Info => "ℹ️",
            Self::Warning => "⚠️",
            Self::Critical => "🚨",
        }
    }

    /// Returns the color code for this alert level.
    #[must_use]
    pub fn color(&self) -> &'static str {
        match self {
            Self::Info => "blue",
            Self::Warning => "yellow",
            Self::Critical => "red",
        }
    }
}

/// Type of alert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertType {
    /// Position exited its price range.
    RangeExit,
    /// Position re-entered its price range.
    RangeEntry,
    /// Impermanent loss exceeded threshold.
    ILThreshold,
    /// PnL target reached.
    PnLTarget,
    /// Fees earned milestone.
    FeesMilestone,
    /// Position needs rebalancing.
    RebalanceNeeded,
    /// System error occurred.
    SystemError,
    /// Connection issue.
    ConnectionIssue,
    /// Custom alert.
    Custom(String),
    /// Rebalance closed old position but failed to open the new one (funds in wallet).
    RebalanceIncomplete,
}

impl AlertType {
    /// Returns a human-readable name for this alert type.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::RangeExit => "Range Exit",
            Self::RangeEntry => "Range Entry",
            Self::ILThreshold => "IL Threshold",
            Self::PnLTarget => "PnL Target",
            Self::FeesMilestone => "Fees Milestone",
            Self::RebalanceNeeded => "Rebalance Needed",
            Self::SystemError => "System Error",
            Self::ConnectionIssue => "Connection Issue",
            Self::Custom(name) => name,
            Self::RebalanceIncomplete => "Rebalance Incomplete",
        }
    }
}

/// An alert instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    /// Unique alert ID.
    pub id: String,
    /// Alert level.
    pub level: AlertLevel,
    /// Alert type.
    pub alert_type: AlertType,
    /// Position address (if applicable).
    pub position: Option<String>,
    /// Pool address (if applicable).
    pub pool: Option<String>,
    /// Alert message.
    pub message: String,
    /// Additional data.
    pub data: Option<AlertData>,
    /// Timestamp.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Whether the alert has been acknowledged.
    pub acknowledged: bool,
}

impl Alert {
    /// Creates a new alert.
    pub fn new(level: AlertLevel, alert_type: AlertType, message: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            level,
            alert_type,
            position: None,
            pool: None,
            message: message.into(),
            data: None,
            timestamp: chrono::Utc::now(),
            acknowledged: false,
        }
    }

    /// Sets the position for this alert.
    #[must_use]
    pub fn with_position(mut self, position: &Pubkey) -> Self {
        self.position = Some(position.to_string());
        self
    }

    /// Sets the pool for this alert.
    #[must_use]
    pub fn with_pool(mut self, pool: &Pubkey) -> Self {
        self.pool = Some(pool.to_string());
        self
    }

    /// Sets additional data for this alert.
    #[must_use]
    pub fn with_data(mut self, data: AlertData) -> Self {
        self.data = Some(data);
        self
    }

    /// Acknowledges this alert.
    pub fn acknowledge(&mut self) {
        self.acknowledged = true;
    }

    /// Formats the alert for display.
    #[must_use]
    pub fn format(&self) -> String {
        format!(
            "{} [{}] {}: {}",
            self.level.emoji(),
            self.timestamp.format("%Y-%m-%d %H:%M:%S"),
            self.alert_type.name(),
            self.message
        )
    }
}

/// Additional data for alerts.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AlertData {
    /// Current price.
    pub current_price: Option<Decimal>,
    /// Range lower bound.
    pub range_lower: Option<Decimal>,
    /// Range upper bound.
    pub range_upper: Option<Decimal>,
    /// IL percentage.
    pub il_pct: Option<Decimal>,
    /// PnL value.
    pub pnl: Option<Decimal>,
    /// Fees earned.
    pub fees: Option<Decimal>,
    /// Custom key-value pairs.
    pub custom: Option<std::collections::HashMap<String, String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alert_creation() {
        let alert = Alert::new(
            AlertLevel::Warning,
            AlertType::RangeExit,
            "Position exited range",
        );

        assert_eq!(alert.level, AlertLevel::Warning);
        assert!(!alert.acknowledged);
    }

    #[test]
    fn test_alert_format() {
        let alert = Alert::new(
            AlertLevel::Critical,
            AlertType::ILThreshold,
            "IL exceeded 10%",
        );

        let formatted = alert.format();
        assert!(formatted.contains("🚨"));
        assert!(formatted.contains("IL Threshold"));
    }
}
