// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : `AgenticBridge` trait — § 7.4.
//!
//! **Sprint 19 — N27** : adds `call_with_context_stream` that
//! emits a `Stream<AgenticEvent>` ; the legacy
//! `call_with_context` is retained for buffered callers and
//! grows a default impl that drains the stream.

use async_trait::async_trait;
use futures::stream::BoxStream;

use efe_nano_sdk::ContextBundle;

use crate::error::BridgeError;
use crate::task::{AgenticEvent, AgenticResponse, AgenticTask, StopReason, TokenUsage};

/// **N21 + N27** : the agentic bridge trait.
///
/// Has two methods :
///
/// * [`AgenticBridge::call_with_context`] — buffered ; returns
///   the full [`AgenticResponse`] after the model finishes.
/// * [`AgenticBridge::call_with_context_stream`] — streaming ;
///   returns a `BoxStream` of [`AgenticEvent`] that emits
///   incremental deltas and finishes with a `Done` (or `Error`).
///
/// Concrete bridges MUST implement at least one of the two.
/// The default impls delegate to each other so a bridge that
/// only implements the buffered surface gets streaming "for
/// free" (with a single synthetic `TextDelta + Done`) — and
/// vice-versa.
#[async_trait]
pub trait AgenticBridge: Send + Sync + 'static {
    /// **N21** : buffered call.  Default impl drains the
    /// streaming path.
    async fn call_with_context(
        &self,
        bundle: &ContextBundle,
        task: AgenticTask,
    ) -> Result<AgenticResponse, BridgeError> {
        use futures::StreamExt;
        let mut stream = self.call_with_context_stream(bundle, task).await?;
        let mut text = String::new();
        let mut stop = StopReason::EndTurn;
        let mut usage = TokenUsage::default();
        let mut identity = efe_nano_sdk::ModelIdentity::hosted("unknown", "unknown", "v0");
        let mut cost: i64 = 0;
        while let Some(ev) = stream.next().await {
            match ev {
                AgenticEvent::TextDelta(s) => text.push_str(&s),
                AgenticEvent::Done {
                    stop_reason,
                    total_usage,
                    model_identity,
                    cost_microcents,
                    ..
                } => {
                    stop = stop_reason;
                    usage = total_usage;
                    identity = model_identity;
                    cost = cost_microcents;
                }
                AgenticEvent::Error(e) => return Err(e),
                _ => {}
            }
        }
        Ok(AgenticResponse {
            text,
            stop_reason: stop,
            usage,
            model_identity: identity,
            cost_microcents: cost,
        })
    }

    /// **N27** : streaming call.  Default impl synthesises a
    /// single `TextDelta` + `Done` from the buffered call.
    async fn call_with_context_stream(
        &self,
        bundle: &ContextBundle,
        task: AgenticTask,
    ) -> Result<BoxStream<'static, AgenticEvent>, BridgeError> {
        // Default : call the buffered surface, then emit one
        // text-delta + one done.  Useful for stubs and for
        // bridges that don't have native streaming.
        let resp = self.call_with_context(bundle, task).await?;
        let identity = resp.model_identity.clone();
        let usage = resp.usage;
        let text = resp.text.clone();
        let cost = resp.cost_microcents;
        let stop = resp.stop_reason;
        let s = futures::stream::iter(vec![
            AgenticEvent::TextDelta(text),
            AgenticEvent::Done {
                stop_reason: stop,
                total_usage: usage,
                model_identity: identity,
                cost_microcents: cost,
                time_to_first_token_ms: None,
                provider_response_time_ms: 0,
                retry_count: 0,
                retry_outcome: efe_nano_sdk::RetryOutcome::NoRetry,
            },
        ]);
        Ok(Box::pin(s))
    }
}
