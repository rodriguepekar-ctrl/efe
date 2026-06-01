// SPDX-License-Identifier: UNLICENSED
//! M31 — Integration wiring examples (compile-checked).
//!
//! The metering plane is intentionally trait-abstracted (the
//! concrete `Counter` impl lives in the runtime binary, which owns
//! the `redis::Client`). To prevent the trait surface from rotting
//! ("designed but never wired"), this module includes one
//! compile-checked example **per public entry point** showing how
//! a handler would invoke `meter_inc`.
//!
//! All functions in this module are
//! `#[cfg_attr(not(test), allow(dead_code))]` — they exist purely
//! to anchor the design contract. The compiler still type-checks
//! them on every build.

#![allow(dead_code)]

use std::sync::Arc;

use crate::{Meter, MeterClient};

/// **Sketch** : how `efe-server::v1::classify::handle_text` would
/// emit a meter increment. Returns the underlying client for unit
/// inspection in tests.
pub async fn emit_classify_meter(
    client: &MeterClient,
    tenant: &Arc<str>,
) -> crate::Result<()> {
    client.meter_inc(tenant, Meter::ClassifyRequests, 1)
}

/// **Sketch** : agent runtime emits a `AgentRunSeconds` meter at the
/// end of every step (delta = step duration in seconds).
pub async fn emit_agent_run_seconds(
    client: &MeterClient,
    tenant: &Arc<str>,
    seconds: u64,
) -> crate::Result<()> {
    client.meter_inc(tenant, Meter::AgentRunSeconds, seconds)
}

/// **Sketch** : multimodal vision encoder bumps `MultimodalVisionRequests`
/// after each successful SigLIP inference.
pub async fn emit_multimodal_vision(
    client: &MeterClient,
    tenant: &Arc<str>,
) -> crate::Result<()> {
    client.meter_inc(tenant, Meter::MultimodalVisionRequests, 1)
}

/// **Sketch** : multimodal audio encoder bumps
/// `MultimodalAudioSeconds` by the (estimated) audio duration in
/// seconds.
pub async fn emit_multimodal_audio_seconds(
    client: &MeterClient,
    tenant: &Arc<str>,
    seconds: u64,
) -> crate::Result<()> {
    client.meter_inc(tenant, Meter::MultimodalAudioSeconds, seconds)
}

/// **Sketch** : webhook delivery worker bumps `WebhookDeliveries`
/// after each delivery attempt (success or retry).
pub async fn emit_webhook_delivery(
    client: &MeterClient,
    tenant: &Arc<str>,
) -> crate::Result<()> {
    client.meter_inc(tenant, Meter::WebhookDeliveries, 1)
}

/// **Sketch** : batch worker bumps `BatchClassifiedRows` at the end
/// of each processed batch (delta = batch row count).
pub async fn emit_batch_classified(
    client: &MeterClient,
    tenant: &Arc<str>,
    rows: u64,
) -> crate::Result<()> {
    client.meter_inc(tenant, Meter::BatchClassifiedRows, rows)
}

/// **Sketch** : audit-export worker bumps `AuditExportGb` at the
/// end of each successful PUT to the customer sink.
pub async fn emit_audit_export_gb(
    client: &MeterClient,
    tenant: &Arc<str>,
    gb_pushed: u64,
) -> crate::Result<()> {
    client.meter_inc(tenant, Meter::AuditExportGb, gb_pushed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counter::Counter;
    use crate::{current_bucket, InMemoryCounter};

    fn s(x: &str) -> Arc<str> {
        Arc::from(x)
    }

    #[tokio::test]
    async fn all_sketches_produce_increments() {
        let counter = Arc::new(InMemoryCounter::new());
        let client = MeterClient::with_defaults(counter.clone());
        let t = s("acme");

        emit_classify_meter(&client, &t).await.unwrap();
        emit_agent_run_seconds(&client, &t, 10).await.unwrap();
        emit_multimodal_vision(&client, &t).await.unwrap();
        emit_multimodal_audio_seconds(&client, &t, 5).await.unwrap();
        emit_webhook_delivery(&client, &t).await.unwrap();
        emit_batch_classified(&client, &t, 1000).await.unwrap();
        emit_audit_export_gb(&client, &t, 2).await.unwrap();

        client.flush_now().await.unwrap();
        let bucket = current_bucket();
        let m = counter.read_bucket("acme", &bucket).await.unwrap();
        assert_eq!(m[&Meter::ClassifyRequests], 1);
        assert_eq!(m[&Meter::AgentRunSeconds], 10);
        assert_eq!(m[&Meter::MultimodalVisionRequests], 1);
        assert_eq!(m[&Meter::MultimodalAudioSeconds], 5);
        assert_eq!(m[&Meter::WebhookDeliveries], 1);
        assert_eq!(m[&Meter::BatchClassifiedRows], 1000);
        assert_eq!(m[&Meter::AuditExportGb], 2);
        // All 7 meters covered. Sanity check : confirm no unmapped
        // meter slipped through by counting.
        assert_eq!(m.len(), 7);
    }
}
