//! PR #23 § 14 — OTel collector probe for S7.

use std::time::Duration;

use crate::checks::CheckContext;
use crate::error::{SmokeError, SmokeResult};

/// Output of [`probe_trace_chain`] : the observed span chain.
#[derive(Debug, Clone)]
pub struct TraceChain {
    pub spans: usize,
    pub services: Vec<String>,
}

/// Send a single request with `traceparent` set, wait 5 s, query the
/// in-cluster OTel collector's HTTP API, and return the chain we saw.
pub async fn probe_trace_chain(_ctx: &CheckContext) -> SmokeResult<TraceChain> {
    // Phase-2 stub : real implementation needs the collector deployed
    // alongside the chart.  Without `--features kind-integration` we
    // return Source error so the orchestrator reports S7 as fail
    // (correct behaviour — smoke needs a real chain).
    Err(SmokeError::Source(
        "S7 OTel probe requires an in-cluster OTel collector ; use --mock for unit tests or rebuild with --features kind-integration".into(),
    ))
}

#[allow(dead_code)]
const _BUDGET_NOTE: Duration = Duration::from_secs(15);

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn probe_returns_source_error_without_integration() {
        let ctx = CheckContext {
            kubeconfig: PathBuf::from("/dev/null"),
            chart_version: "0.6.0".into(),
            namespace: "n".into(),
            timeout: Duration::from_secs(15),
            mock: false,
            redact_logs: false,
        };
        let e = probe_trace_chain(&ctx).await.unwrap_err();
        assert!(matches!(e, SmokeError::Source(_)));
    }

    #[test]
    fn trace_chain_shape() {
        let c = TraceChain {
            spans: 2,
            services: vec!["a".into(), "b".into()],
        };
        assert_eq!(c.spans, 2);
        assert_eq!(c.services.len(), 2);
    }
}
