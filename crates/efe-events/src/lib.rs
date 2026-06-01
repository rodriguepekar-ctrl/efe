//! `efe-events` — M28 typed event envelope + NATS JetStream pub/sub.
//!
//! Three layers, all built on top of [`Event`] :
//!
//! 1. **Envelope** ([`Event<T>`]) — common shape : ULID id, tenant_id,
//!    topic, occurred_at, schema_version, optional trace_id, typed
//!    payload. Serialized as JSON.
//! 2. **Publisher** ([`EventPublisher`] trait + [`NatsPublisher`]) —
//!    publishes envelopes to JetStream subjects of the form
//!    `efe.tenant.{slug}.{topic}` or `efe.system.{topic}`.
//! 3. **Subscriber** ([`EventSubscriber`] trait + [`NatsSubscriber`]) —
//!    consumes a durable subject filter, yields a stream of
//!    deserialised events. ACK semantics handled per message.
//!
//! Failure mode : NATS unreachable. Publishers buffer in memory (10k
//! cap, ring buffer) and retry. If buffer overflows, drop with a
//! `events_dropped_total` metric. NEVER block the caller path.
//!
//! ## Subject hierarchy
//!
//! Following ADR-M28-001 :
//!
//! ```text
//! efe.tenant.{slug}.classify.completed
//! efe.tenant.{slug}.agent.run.{created|completed|failed}
//! efe.tenant.{slug}.agent.approval.{requested|granted|rejected}
//! efe.tenant.{slug}.drift.detected
//! efe.tenant.{slug}.bundle.{deployed|rolled_back}
//! efe.tenant.{slug}.batch.{submitted|completed|failed}
//! efe.tenant.{slug}.dpia.{signed|expired}
//! efe.tenant.{slug}.graduation.{acknowledged|revoked}
//!
//! efe.system.audit.{verb}
//! efe.batch.jobs.queue                      (workqueue for batch workers)
//! ```
//!
//! ## Schema versioning
//!
//! Each topic has an integer `schema_version` in the envelope. Today
//! every event is `1`. Breaking changes bump the version ; subscribers
//! filter by version explicitly.

#![warn(missing_docs)]

pub mod envelope;
pub mod error;
pub mod publisher;
pub mod subscriber;
pub mod topics;

pub use envelope::{Event, EventId};
pub use error::{EventError, EventResult};
pub use publisher::{EventPublisher, NatsPublisher, NatsPublisherConfig};
pub use subscriber::{EventStream, EventSubscriber, NatsSubscriber};
pub use topics::{Topic, BatchTopic, ClassifyTopic, AgentTopic};
