// SPDX-License-Identifier: UNLICENSED
//! # `efe-swarm-dashboard` — Phase 5 Inc.3 : le **cœur** du dashboard live (sans HTTP).
//!
//! On **voit** les petits agents se passer l'info en parallèle. Le [`DashboardBus`] se branche
//! sur les **seams substrat déjà existants** — via le hook générique `SwarmObserver` du SDK —
//! et diffuse chaque micro-événement ([`DashboardEvent`]) à 4 familles de panneaux :
//! trafic inter-agent · consensus · provenance · budget.
//!
//! ```
//! use std::sync::Arc;
//! use efe_swarm_dashboard::{DashboardBus, DashboardObserver};
//! use efe_swarm_sdk::{Swarm, TopologySpec, Budget};
//! use efe_nano_sdk::TenantId;
//! use serde_json::json;
//! use uuid::Uuid;
//!
//! # let rt = tokio::runtime::Runtime::new().unwrap();
//! # rt.block_on(async {
//! let bus = DashboardBus::new(TenantId::system(), Uuid::new_v4(), 8192);
//! let swarm = Swarm::builder()
//!     .with_observer(Arc::new(DashboardObserver::new(bus.clone())))   // ← branche l'observateur
//!     .build().unwrap();
//! swarm.run(json!({ "text": "hi", "expected": "hi" }), TopologySpec::Auto, Budget::cents(100)).await.unwrap();
//! assert!(!bus.replay().is_empty());                                  // ← le run a été observé
//! # });
//! ```
//!
//! **Discipline EFE** : le dashboard est un **observateur**, pas un acteur — il tape des seams
//! existants (`WriteSink`/`EventSink`/`SwarmObserver`) **sans réécrire un seul agent**. Le flux
//! est **best-effort/lossy** ; la vérité durable reste la chaîne de provenance (`verify_swarm`).

#![deny(unsafe_code)]
#![warn(rust_2018_idioms)]

mod bus;
mod event;
mod metrics;
mod registry;
mod tap;

pub use bus::{DashboardBus, DashboardEnvelope};
pub use event::{preview, DashboardEvent, Lane};
pub use metrics::DashboardMetrics;
pub use registry::DashboardRegistry;
pub use tap::{DashboardObserver, DECIDED_TOPIC};

/// Version du crate (semver).
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");
