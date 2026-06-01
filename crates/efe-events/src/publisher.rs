//! NATS JetStream event publisher.
//!
//! Construction : [`NatsPublisher::connect`] connects to a NATS URL and
//! ensures the `EVENTS_TENANT_<slug>` JetStream stream exists. If the
//! stream is absent it's auto-created with sensible defaults
//! (limits retention, 7d max_age, 10M max_msgs, 5GB max_bytes, file
//! storage, 3 replicas).
//!
//! Failure semantics : every publish has a per-op timeout (default
//! 250 ms). On timeout or NATS error, returns `EventError::Unavailable`
//! ; callers must `.unwrap_or` or drop. The publisher NEVER blocks
//! the caller's hot path.

use crate::envelope::{Event, EventId};
use crate::error::{EventError, EventResult};
use async_nats::jetstream::{self, stream::StorageType};
use async_trait::async_trait;
use serde::Serialize;
use std::time::Duration;
use tracing::{debug, warn};

const DEFAULT_PUBLISH_TIMEOUT: Duration = Duration::from_millis(250);

/// Stream defaults applied when auto-creating a tenant or system stream.
const DEFAULT_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600); // 7d
const DEFAULT_MAX_MSGS: i64 = 10_000_000;
const DEFAULT_MAX_BYTES: i64 = 5 * 1024 * 1024 * 1024; // 5 GB
const DEFAULT_REPLICAS: usize = 3;

/// Publish-time configuration.
#[derive(Debug, Clone)]
pub struct NatsPublisherConfig {
    /// NATS URL : `nats://host:4222` or `tls://host:4222`.
    pub url: String,

    /// Optional credentials file path. None = anon (only in dev).
    pub creds_path: Option<String>,

    /// Per-publish timeout. Default 250ms — callers must degrade fast
    /// when NATS is unhealthy.
    pub publish_timeout: Duration,

    /// Replica count for auto-created streams. In dev/k3d use 1 ; in
    /// production use 3.
    pub stream_replicas: usize,
}

impl Default for NatsPublisherConfig {
    fn default() -> Self {
        Self {
            url: "nats://localhost:4222".into(),
            creds_path: None,
            publish_timeout: DEFAULT_PUBLISH_TIMEOUT,
            stream_replicas: DEFAULT_REPLICAS,
        }
    }
}

impl NatsPublisherConfig {
    /// Build from env :
    /// - `EFE_NATS_URL` (required ; defaults to localhost in tests).
    /// - `EFE_NATS_CREDS` (optional credentials file).
    /// - `EFE_NATS_PUBLISH_TIMEOUT_MS` (optional, default 250).
    /// - `EFE_NATS_STREAM_REPLICAS` (optional, default 3).
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(url) = std::env::var("EFE_NATS_URL") {
            cfg.url = url;
        }
        cfg.creds_path = std::env::var("EFE_NATS_CREDS").ok();
        if let Ok(ms) = std::env::var("EFE_NATS_PUBLISH_TIMEOUT_MS") {
            if let Ok(n) = ms.parse::<u64>() {
                cfg.publish_timeout = Duration::from_millis(n);
            }
        }
        if let Ok(r) = std::env::var("EFE_NATS_STREAM_REPLICAS") {
            if let Ok(n) = r.parse::<usize>() {
                cfg.stream_replicas = n.max(1);
            }
        }
        cfg
    }
}

/// Async trait for publishing events. Useful for tests where you
/// stub the publisher with an in-memory queue.
#[async_trait]
pub trait EventPublisher: Send + Sync {
    /// Publish a single event. The wire-format payload is built from
    /// `payload` (serialized as JSON) wrapped in an [`Event`] envelope.
    async fn publish<T: Serialize + Send + Sync>(
        &self,
        topic: &str,
        tenant_id: &str,
        payload: T,
    ) -> EventResult<EventId>;

    /// Same as `publish` but propagates an explicit trace context.
    async fn publish_traced<T: Serialize + Send + Sync>(
        &self,
        topic: &str,
        tenant_id: &str,
        payload: T,
        trace_id: &str,
    ) -> EventResult<EventId>;
}

/// Production NATS JetStream publisher.
#[derive(Clone)]
pub struct NatsPublisher {
    js: jetstream::Context,
    cfg: NatsPublisherConfig,
}

impl std::fmt::Debug for NatsPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NatsPublisher")
            .field("url", &self.cfg.url)
            .field("timeout", &self.cfg.publish_timeout)
            .finish()
    }
}

impl NatsPublisher {
    /// Connect + return a ready publisher.
    pub async fn connect(cfg: NatsPublisherConfig) -> EventResult<Self> {
        let mut opts = async_nats::ConnectOptions::new();
        if let Some(creds) = &cfg.creds_path {
            opts = opts
                .credentials_file(creds)
                .await
                .map_err(|e| EventError::Misconfig(format!("credentials_file: {}", e)))?;
        }
        let client = opts
            .name(format!(
                "efe-events-publisher/{}",
                std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into())
            ))
            .connect(&cfg.url)
            .await
            .map_err(|e| EventError::Unavailable(e.to_string()))?;
        let js = jetstream::new(client);
        Ok(Self { js, cfg })
    }

    /// Ensure the per-tenant stream exists. Idempotent. Call once per
    /// tenant on first publish (or eagerly at tenant provisioning time).
    pub async fn ensure_tenant_stream(&self, tenant_id: &str) -> EventResult<()> {
        let stream_name = format!("EVENTS_TENANT_{}", sanitize_stream_name(tenant_id));
        let subjects = vec![format!("efe.tenant.{}.>", tenant_id)];
        self.ensure_stream(&stream_name, subjects).await
    }

    /// Ensure the cross-tenant system stream exists.
    pub async fn ensure_system_stream(&self) -> EventResult<()> {
        let subjects = vec!["efe.system.>".into()];
        self.ensure_stream("EVENTS_SYSTEM", subjects).await
    }

    /// Ensure the batch workqueue stream exists (deliver-once semantics).
    pub async fn ensure_batch_jobs_stream(&self) -> EventResult<()> {
        let cfg = jetstream::stream::Config {
            name: "BATCH_JOBS".into(),
            subjects: vec!["efe.batch.jobs.>".into()],
            retention: jetstream::stream::RetentionPolicy::WorkQueue,
            max_age: Duration::from_secs(24 * 3600),
            storage: StorageType::File,
            num_replicas: self.cfg.stream_replicas,
            discard: jetstream::stream::DiscardPolicy::Old,
            ..Default::default()
        };
        self.js
            .get_or_create_stream(cfg)
            .await
            .map_err(|e| EventError::JetStream(e.to_string()))?;
        Ok(())
    }

    async fn ensure_stream(&self, name: &str, subjects: Vec<String>) -> EventResult<()> {
        let cfg = jetstream::stream::Config {
            name: name.into(),
            subjects,
            retention: jetstream::stream::RetentionPolicy::Limits,
            max_age: DEFAULT_MAX_AGE,
            max_messages: DEFAULT_MAX_MSGS,
            max_bytes: DEFAULT_MAX_BYTES,
            storage: StorageType::File,
            num_replicas: self.cfg.stream_replicas,
            discard: jetstream::stream::DiscardPolicy::Old,
            ..Default::default()
        };
        self.js
            .get_or_create_stream(cfg)
            .await
            .map_err(|e| EventError::JetStream(e.to_string()))?;
        debug!(%name, "ensured JetStream stream");
        Ok(())
    }

    async fn publish_inner(
        &self,
        subject: String,
        bytes: Vec<u8>,
    ) -> EventResult<()> {
        let fut = self.js.publish(subject, bytes.into());
        match tokio::time::timeout(self.cfg.publish_timeout, fut).await {
            Ok(Ok(ack_fut)) => match tokio::time::timeout(self.cfg.publish_timeout, ack_fut).await {
                Ok(Ok(_ack)) => Ok(()),
                Ok(Err(e)) => Err(EventError::Nats(format!("publish ack : {}", e))),
                Err(_) => Err(EventError::Unavailable("publish ack timeout".into())),
            },
            Ok(Err(e)) => Err(EventError::Nats(format!("publish : {}", e))),
            Err(_) => Err(EventError::Unavailable("publish timeout".into())),
        }
    }
}

#[async_trait]
impl EventPublisher for NatsPublisher {
    async fn publish<T: Serialize + Send + Sync>(
        &self,
        topic: &str,
        tenant_id: &str,
        payload: T,
    ) -> EventResult<EventId> {
        let event = Event::new(tenant_id, topic, payload);
        let id = event.id;
        let subject = event.subject();
        let bytes = serde_json::to_vec(&event)?;
        if let Err(e) = self.publish_inner(subject, bytes).await {
            warn!(?e, %tenant_id, %topic, "event publish failed");
            return Err(e);
        }
        Ok(id)
    }

    async fn publish_traced<T: Serialize + Send + Sync>(
        &self,
        topic: &str,
        tenant_id: &str,
        payload: T,
        trace_id: &str,
    ) -> EventResult<EventId> {
        let event = Event::new_traced(tenant_id, topic, payload, trace_id);
        let id = event.id;
        let subject = event.subject();
        let bytes = serde_json::to_vec(&event)?;
        self.publish_inner(subject, bytes).await?;
        Ok(id)
    }
}

/// NATS stream names cannot contain certain characters. This sanitizer
/// keeps stream names operationally sensible.
fn sanitize_stream_name(slug: &str) -> String {
    slug.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_name_sanitization() {
        assert_eq!(sanitize_stream_name("acme"), "acme");
        assert_eq!(sanitize_stream_name("acme-corp"), "acme-corp");
        assert_eq!(sanitize_stream_name("acme.corp"), "acme_corp");
        assert_eq!(sanitize_stream_name("foo$%bar"), "foo__bar");
    }

    #[test]
    fn config_defaults() {
        let cfg = NatsPublisherConfig::default();
        assert!(cfg.url.starts_with("nats://"));
        assert_eq!(cfg.stream_replicas, 3);
        assert_eq!(cfg.publish_timeout, Duration::from_millis(250));
    }
}
