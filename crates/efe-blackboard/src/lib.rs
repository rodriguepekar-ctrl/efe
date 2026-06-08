// SPDX-License-Identifier: UNLICENSED
//! # `efe-blackboard` — M50 Sprint 13 N21 shared swarm state.
//!
//! See `docs/architecture/EFE_M50_N21_SWARM_ORCHESTRATION_AGENTIC_BRIDGE_ARCHITECTURE.md`.
//!
//! Two primitives :
//! - [`Blackboard`] — concurrent JSON-pointer-keyed store with
//!   per-field versioning ; mutated by every agent in a swarm DAG.
//! - [`EventBus`] — bounded `tokio::broadcast` pub/sub for
//!   inter-agent events.
//!
//! Both are designed to be held in an `Arc` and shared across
//! parallel `tokio::spawn` tasks within a single swarm run.

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod blackboard;
pub mod error;
pub mod eventbus;
pub mod metrics;
pub mod observer;
pub mod reliable;
pub mod replay;
pub mod sink;
pub mod snapshot;
pub mod streaming;
pub mod topic_router;
pub mod work_queue;

pub use blackboard::{Blackboard, BlackboardValue, BlackboardWrite, DEFAULT_RECENT_HISTORY};
pub use error::{BlackboardError, EventBusError};
pub use eventbus::{EventBus, EventBusSubscription, SwarmEvent};
pub use metrics::{LatencyHistogram, MetricsObserver, MetricsSnapshot, SubstrateMetrics};
pub use observer::{BlackboardObserver, ObserverRegistration};
pub use reliable::{ReliableItem, ReliableReader, ReliableTopic};
pub use replay::EventLog;
pub use sink::{CapturingSink, ChannelSink, CountingSink, NoopSink, WriteSink};
pub use snapshot::BlackboardSnapshot;
pub use streaming::{
    AgentCtx, AgentHandler, AgentReport, AsyncAgentHandler, BoxedHandlerFuture, QuiescenceTracker,
    StreamingSwarm, SwarmReport,
};
pub use topic_router::{MergedSubscription, TopicRouter};
pub use work_queue::{
    shard_owner, shard_topic, shard_topic_for_item, ClaimModel, ClaimOutcome, WorkQueue,
    WorkQueueStats,
};

/// **N21** : Crate semver.
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// **N21 + I10** : maximum cumulative byte size of the blackboard
/// before writes are rejected with `BlackboardError::SizeLimitExceeded`.
/// Matches the `ContextBundle` cap from the SDK.
pub const MAX_BLACKBOARD_BYTES: usize = efe_nano_sdk::MAX_CONTEXT_BUNDLE_BYTES;
