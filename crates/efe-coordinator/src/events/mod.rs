// SPDX-License-Identifier: UNLICENSED
//! Bus d'évènements + protocole SSE.
//!
//! - [`EventBus`] : trait pub/sub abstrait.
//! - [`InMemoryBus`] : impl `tokio::broadcast` (tests + dev).
//! - `NatsBus` (feature `nats-bus`) : impl JetStream pour la prod (Phase 7).
//! - [`StreamEvent`] : enum typé des évènements exposés par SSE à l'UI.

mod bus;
mod inmem;
mod stream;

pub use bus::{
    filter_and, filter_by_swarm, filter_by_tag, filter_or, EventBus, EventFilter,
    SubscriberToken,
};
pub use inmem::InMemoryBus;
pub use stream::{MissionTag, StreamEvent, ALL_EVENT_TAGS, EVENT_SCHEMA_VERSION};

#[cfg(feature = "nats-bus")]
mod nats;
#[cfg(feature = "nats-bus")]
pub use self::nats::NatsBus;
