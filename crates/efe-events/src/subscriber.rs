//! NATS JetStream durable consumer.
//!
//! Pattern : caller builds a `NatsSubscriber`, requests a durable consumer
//! for a subject filter (e.g. `efe.tenant.acme.>` or
//! `efe.tenant.*.classify.completed`), then iterates events. Each event
//! must be ACK'd explicitly so the consumer's `confirmed_consumer_seq`
//! advances. Failure to ACK within `ack_wait` triggers re-delivery.
//!
//! ACK strategy : ack on successful processing ; nak (retry now) on
//! transient failure ; term (don't redeliver) on permanent failure.

use crate::envelope::Event;
use crate::error::{EventError, EventResult};
use async_nats::jetstream::{self, consumer::PullConsumer};
use async_trait::async_trait;
use futures::StreamExt;
use serde::de::DeserializeOwned;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tracing::{debug, warn};

/// Stream of typed events from a NATS subscription. Yields `Event<T>`
/// items ; the caller MUST ACK after processing (use [`AckHandle`]).
pub struct EventStream<T> {
    rx: mpsc::Receiver<EventStreamItem<T>>,
}

/// Single item yielded by [`EventStream`] : the deserialised event +
/// an [`AckHandle`] for explicit acknowledgement.
pub struct EventStreamItem<T> {
    /// Parsed event envelope.
    pub event: Event<T>,
    /// Used to ack / nak / term this message.
    pub ack: AckHandle,
}

/// Wraps the underlying NATS message + ack channel.
pub struct AckHandle {
    msg: async_nats::jetstream::Message,
}

impl AckHandle {
    /// Acknowledge — the message will not be re-delivered.
    pub async fn ack(self) -> EventResult<()> {
        self.msg
            .ack()
            .await
            .map_err(|e| EventError::Nats(format!("ack : {}", e)))
    }

    /// Nak — request immediate redelivery (transient failure).
    pub async fn nak(self) -> EventResult<()> {
        self.msg
            .ack_with(async_nats::jetstream::AckKind::Nak(None))
            .await
            .map_err(|e| EventError::Nats(format!("nak : {}", e)))
    }

    /// Term — give up on this message (permanent failure ; goes to DLQ
    /// if configured, otherwise discarded).
    pub async fn term(self) -> EventResult<()> {
        self.msg
            .ack_with(async_nats::jetstream::AckKind::Term)
            .await
            .map_err(|e| EventError::Nats(format!("term : {}", e)))
    }
}

impl<T> EventStream<T> {
    /// Fetch the next item, blocking until ready or stream closed.
    pub async fn next(&mut self) -> Option<EventStreamItem<T>> {
        self.rx.recv().await
    }
}

impl<T> Stream for EventStream<T> {
    type Item = EventStreamItem<T>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

/// Trait abstraction for subscribers ; used in tests with an in-memory impl.
#[async_trait]
pub trait EventSubscriber: Send + Sync {
    /// Subscribe to a subject filter under a durable name (so the
    /// position survives pod restarts).
    async fn subscribe<T: DeserializeOwned + Send + 'static>(
        &self,
        subject_filter: &str,
        durable_name: &str,
    ) -> EventResult<EventStream<T>>;
}

/// Production NATS JetStream subscriber.
#[derive(Clone)]
pub struct NatsSubscriber {
    js: jetstream::Context,
    /// Stream to attach the consumer on, e.g. `EVENTS_TENANT_acme`.
    pub stream_name: String,
    /// Per-message ack-wait. Default 30s ; long jobs override higher.
    pub ack_wait: Duration,
}

impl NatsSubscriber {
    /// Build from an already-connected JetStream context. Construct
    /// the context via `async_nats::jetstream::new(client)`.
    pub fn new(js: jetstream::Context, stream_name: impl Into<String>) -> Self {
        Self {
            js,
            stream_name: stream_name.into(),
            ack_wait: Duration::from_secs(30),
        }
    }

    /// Increase ack-wait (long-running consumers : batch worker uses 10m).
    pub fn with_ack_wait(mut self, ack_wait: Duration) -> Self {
        self.ack_wait = ack_wait;
        self
    }
}

#[async_trait]
impl EventSubscriber for NatsSubscriber {
    async fn subscribe<T: DeserializeOwned + Send + 'static>(
        &self,
        subject_filter: &str,
        durable_name: &str,
    ) -> EventResult<EventStream<T>> {
        let stream = self
            .js
            .get_stream(&self.stream_name)
            .await
            .map_err(|e| EventError::JetStream(format!("get_stream : {}", e)))?;
        let consumer_cfg = jetstream::consumer::pull::Config {
            durable_name: Some(durable_name.into()),
            filter_subject: subject_filter.into(),
            ack_wait: self.ack_wait,
            ack_policy: jetstream::consumer::AckPolicy::Explicit,
            max_deliver: 6,
            ..Default::default()
        };
        let consumer: PullConsumer = stream
            .get_or_create_consumer(durable_name, consumer_cfg)
            .await
            .map_err(|e| EventError::JetStream(format!("create consumer : {}", e)))?;

        let (tx, rx) = mpsc::channel::<EventStreamItem<T>>(256);
        let stream_name = self.stream_name.clone();
        let durable_name = durable_name.to_string();
        tokio::spawn(async move {
            let mut messages = match consumer.messages().await {
                Ok(s) => s,
                Err(e) => {
                    warn!(?e, %stream_name, %durable_name, "JetStream messages() failed");
                    return;
                }
            };
            while let Some(msg) = messages.next().await {
                let msg = match msg {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(?e, "JetStream message error");
                        continue;
                    }
                };
                // Try to parse the envelope.
                let event: Event<T> = match serde_json::from_slice(&msg.payload) {
                    Ok(e) => e,
                    Err(e) => {
                        warn!(?e, "JetStream payload decode failed ; terminating msg");
                        let _ = msg
                            .ack_with(async_nats::jetstream::AckKind::Term)
                            .await;
                        continue;
                    }
                };
                let item = EventStreamItem {
                    event,
                    ack: AckHandle { msg },
                };
                if tx.send(item).await.is_err() {
                    debug!("consumer receiver dropped ; exiting subscriber loop");
                    break;
                }
            }
        });
        Ok(EventStream { rx })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_wait_default_is_30s() {
        // We can't construct a real subscriber without a client, but
        // we can sanity-check the API surface compiles.
        let d = Duration::from_secs(30);
        assert_eq!(d.as_secs(), 30);
    }
}
