//! Async event bus primitives for API/execution communication.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::RwLock as StdRwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{RwLock, broadcast};
use tokio::time::{Duration, sleep};
use tracing::warn;

pub const EVENT_POSITION_UPDATED: &str = "position.updated";
pub const EVENT_ALERT_RAISED: &str = "alert.raised";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: String,
    pub event_type: String,
    pub event_version: u16,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub source: String,
    pub correlation_id: String,
    pub payload: Value,
}

impl EventEnvelope {
    pub fn new(event_type: impl Into<String>, source: impl Into<String>, payload: Value) -> Self {
        let correlation_id = uuid::Uuid::new_v4().to_string();
        Self {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_type: event_type.into(),
            event_version: 1,
            occurred_at: chrono::Utc::now(),
            source: source.into(),
            correlation_id,
            payload,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EventBusStats {
    pub published: u64,
    pub retries: u64,
    pub duplicates: u64,
    pub failed: u64,
    pub dlq_size: usize,
}

#[async_trait]
pub trait EventBus: Send + Sync {
    async fn publish(&self, event: EventEnvelope) -> anyhow::Result<()>;
    fn subscribe(&self, event_type: &str) -> broadcast::Receiver<EventEnvelope>;
    async fn dlq(&self) -> Vec<EventEnvelope>;
    async fn push_dlq(&self, event: EventEnvelope);
    fn stats(&self) -> EventBusStats;
}

#[derive(Debug)]
pub struct InProcessEventBus {
    channels: StdRwLock<HashMap<String, broadcast::Sender<EventEnvelope>>>,
    dedup_seen: RwLock<HashMap<String, chrono::DateTime<chrono::Utc>>>,
    dedup_ttl_secs: i64,
    dlq: RwLock<VecDeque<EventEnvelope>>,
    published: AtomicU64,
    retries: AtomicU64,
    duplicates: AtomicU64,
    failed: AtomicU64,
}

impl InProcessEventBus {
    pub fn new() -> Self {
        Self {
            channels: StdRwLock::new(HashMap::new()),
            dedup_seen: RwLock::new(HashMap::new()),
            dedup_ttl_secs: 3600,
            dlq: RwLock::new(VecDeque::new()),
            published: AtomicU64::new(0),
            retries: AtomicU64::new(0),
            duplicates: AtomicU64::new(0),
            failed: AtomicU64::new(0),
        }
    }

    async fn sender_for(&self, event_type: &str) -> broadcast::Sender<EventEnvelope> {
        if let Some(tx) = self
            .channels
            .read()
            .expect("channels lock poisoned")
            .get(event_type)
            .cloned()
        {
            return tx;
        }
        let mut guard = self.channels.write().expect("channels lock poisoned");
        guard
            .entry(event_type.to_string())
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(2048);
                tx
            })
            .clone()
    }

    async fn cleanup_seen(&self, now: chrono::DateTime<chrono::Utc>) {
        let threshold = now - chrono::Duration::seconds(self.dedup_ttl_secs);
        let mut seen = self.dedup_seen.write().await;
        seen.retain(|_, ts| *ts > threshold);
    }

    fn dedup_key(event: &EventEnvelope) -> String {
        format!("{}:{}", event.event_type, event.event_id)
    }

}

#[async_trait]
impl EventBus for InProcessEventBus {
    async fn publish(&self, event: EventEnvelope) -> anyhow::Result<()> {
        self.cleanup_seen(chrono::Utc::now()).await;
        let key = Self::dedup_key(&event);
        {
            let mut seen = self.dedup_seen.write().await;
            if seen.contains_key(&key) {
                self.duplicates.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            seen.insert(key, chrono::Utc::now());
        }
        let tx = self.sender_for(&event.event_type).await;
        // broadcast send can fail only when there are no receivers; we still count as published.
        let _ = tx.send(event);
        self.published.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn subscribe(&self, event_type: &str) -> broadcast::Receiver<EventEnvelope> {
        // sync fallback receiver; actual sender is initialized lazily in async publish path
        // and here for deterministic subscription behavior.
        if let Some(tx) = self
            .channels
            .read()
            .expect("channels lock poisoned")
            .get(event_type)
            .cloned()
        {
            return tx.subscribe();
        }
        let mut guard = self.channels.write().expect("channels lock poisoned");
        guard
            .entry(event_type.to_string())
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(2048);
                tx
            })
            .subscribe()
    }

    async fn dlq(&self) -> Vec<EventEnvelope> {
        self.dlq.read().await.iter().cloned().collect()
    }

    async fn push_dlq(&self, event: EventEnvelope) {
        let mut guard = self.dlq.write().await;
        guard.push_back(event);
        while guard.len() > 10_000 {
            let _ = guard.pop_front();
        }
    }

    fn stats(&self) -> EventBusStats {
        EventBusStats {
            published: self.published.load(Ordering::Relaxed),
            retries: self.retries.load(Ordering::Relaxed),
            duplicates: self.duplicates.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            dlq_size: self.dlq.try_read().map(|g| g.len()).unwrap_or(0),
        }
    }
}

#[derive(Debug)]
pub struct BrokerEventBus {
    backend: String,
    shadow_mode: bool,
    inner: Arc<InProcessEventBus>,
}

impl BrokerEventBus {
    pub fn new(backend: impl Into<String>, shadow_mode: bool) -> Self {
        Self {
            backend: backend.into(),
            shadow_mode,
            inner: Arc::new(InProcessEventBus::new()),
        }
    }
}

#[async_trait]
impl EventBus for BrokerEventBus {
    async fn publish(&self, event: EventEnvelope) -> anyhow::Result<()> {
        // Adapter scaffold: in shadow mode we keep app behavior and mirror to local bus.
        // Real broker client can replace this path without API surface changes.
        if self.shadow_mode {
            warn!(backend = %self.backend, event_type = %event.event_type, "event bus shadow mode (broker adapter scaffold)");
            return self.inner.publish(event).await;
        }
        // For now, keep functional behavior without external dependency.
        self.inner.publish(event).await
    }

    fn subscribe(&self, event_type: &str) -> broadcast::Receiver<EventEnvelope> {
        self.inner.subscribe(event_type)
    }

    async fn dlq(&self) -> Vec<EventEnvelope> {
        self.inner.dlq().await
    }

    async fn push_dlq(&self, event: EventEnvelope) {
        self.inner.push_dlq(event).await;
    }

    fn stats(&self) -> EventBusStats {
        self.inner.stats()
    }
}

pub async fn publish_with_retry(
    bus: &dyn EventBus,
    event: EventEnvelope,
    max_retries: u8,
) -> anyhow::Result<()> {
    let mut attempt = 0u8;
    loop {
        match bus.publish(event.clone()).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                attempt = attempt.saturating_add(1);
                if attempt > max_retries {
                    bus.push_dlq(event).await;
                    return Err(e);
                }
                let backoff_ms = 50u64 * (1u64 << (attempt.saturating_sub(1)));
                sleep(Duration::from_millis(backoff_ms.min(2000))).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn inprocess_publish_and_subscribe() {
        let bus = InProcessEventBus::new();
        let mut rx = bus.subscribe(EVENT_ALERT_RAISED);
        bus.publish(EventEnvelope::new(
            EVENT_ALERT_RAISED,
            "test",
            serde_json::json!({"x":1}),
        ))
        .await
        .unwrap();
        let msg = rx.recv().await.unwrap();
        assert_eq!(msg.event_type, EVENT_ALERT_RAISED);
    }

    #[tokio::test]
    async fn inprocess_deduplicates_event_id_per_type() {
        let bus = InProcessEventBus::new();
        let mut event = EventEnvelope::new(EVENT_ALERT_RAISED, "test", serde_json::json!({}));
        event.event_id = "same".to_string();
        bus.publish(event.clone()).await.unwrap();
        bus.publish(event).await.unwrap();
        let s = bus.stats();
        assert_eq!(s.published, 1);
        assert_eq!(s.duplicates, 1);
    }
}

