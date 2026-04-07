//! Alert notification channels.

use super::Alert;
use async_trait::async_trait;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use tracing::{error, info};

/// Trait for alert notification channels.
#[async_trait]
pub trait Notifier: Send + Sync {
    /// Sends an alert through this channel.
    async fn notify(&self, alert: &Alert) -> anyhow::Result<()>;

    /// Returns the name of this notifier.
    fn name(&self) -> &str;
}

/// Console notifier - prints alerts to stdout.
pub struct ConsoleNotifier;

#[async_trait]
impl Notifier for ConsoleNotifier {
    async fn notify(&self, alert: &Alert) -> anyhow::Result<()> {
        println!("{}", alert.format());
        Ok(())
    }

    fn name(&self) -> &str {
        "console"
    }
}

/// File notifier - writes alerts to a log file.
pub struct FileNotifier {
    /// Path to the log file.
    path: PathBuf,
}

impl FileNotifier {
    /// Creates a new file notifier.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl Notifier for FileNotifier {
    async fn notify(&self, alert: &Alert) -> anyhow::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        writeln!(file, "{}", serde_json::to_string(alert)?)?;

        Ok(())
    }

    fn name(&self) -> &str {
        "file"
    }
}

/// Webhook notifier - sends alerts to an HTTP endpoint.
pub struct WebhookNotifier {
    /// Webhook URL.
    url: String,
    /// HTTP client.
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl WebhookNotifier {
    /// Creates a new webhook notifier.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl Notifier for WebhookNotifier {
    async fn notify(&self, alert: &Alert) -> anyhow::Result<()> {
        let payload = serde_json::json!({
            "text": alert.format(),
            "alert": alert,
        });

        self.client
            .post(&self.url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("webhook POST: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("webhook response: {e}"))?;

        info!(
            url = %self.url,
            alert_id = %alert.id,
            "Webhook notification sent"
        );

        Ok(())
    }

    fn name(&self) -> &str {
        "webhook"
    }
}

/// Multi-channel notifier that sends to multiple channels.
pub struct MultiNotifier {
    /// List of notifiers.
    notifiers: Vec<Box<dyn Notifier>>,
}

impl MultiNotifier {
    /// Creates a new multi-notifier.
    #[must_use]
    pub fn new() -> Self {
        Self {
            notifiers: Vec::new(),
        }
    }

    /// Adds a notifier.
    pub fn add<N: Notifier + 'static>(&mut self, notifier: N) {
        self.notifiers.push(Box::new(notifier));
    }

    /// Sends an alert to all channels.
    pub async fn notify_all(&self, alert: &Alert) {
        for notifier in &self.notifiers {
            if let Err(e) = notifier.notify(alert).await {
                error!(
                    notifier = notifier.name(),
                    error = %e,
                    "Failed to send notification"
                );
            }
        }
    }
}

impl Default for MultiNotifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alerts::{AlertLevel, AlertType};

    #[tokio::test]
    async fn test_console_notifier() {
        let notifier = ConsoleNotifier;
        let alert = Alert::new(AlertLevel::Info, AlertType::RangeEntry, "Test alert");

        let result = notifier.notify(&alert).await;
        assert!(result.is_ok());
    }
}
