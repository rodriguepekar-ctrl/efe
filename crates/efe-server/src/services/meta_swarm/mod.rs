//! Phase-2 PR #11 — gRPC client adapter for the meta-swarm controller.
//!
//! In PR #11 this module only exposes [`MetaSwarmClient::health_probe`]
//! (the startup readiness check).  PR #15 will add
//! `iterate` / `step_once` / `reset` / `info` pass-through methods
//! that delegate to the underlying tonic-generated client with
//! shared timeout + tracing instrumentation.
//!
//! See `docs/EFE_A1_PHASE2_PR11_ARCHITECTURE.md` § 9.
//!
//! # Lifecycle
//!
//! `efe-server` calls [`MetaSwarmClient::connect`] **once** at
//! startup when `EFE_META_SWARM_ENABLED=true` ; the returned
//! handle is held by the runtime state, cheap to clone, and used
//! by PR #15 dispatch.  If connect or the optional Health probe
//! fails, `efe-server` **must** refuse to boot (fail-fast doctrine,
//! PR #11 AD3).

pub mod metrics;

use std::sync::Arc;
use std::time::Instant;

use efe_meta_swarm_proto::controller::MetaSwarmServiceClient;
use efe_meta_swarm_proto::meta_swarm::{HealthRequest, IterateRequest, IterateResponse};
use tokio::sync::Mutex;
use tonic::transport::{Channel, Endpoint};
use tonic::Request;
use tracing::{debug, instrument, warn};

use crate::config::MetaSwarmConfig;

pub use self::metrics::MetaSwarmMetrics;

/// Errors raised when constructing or probing the client.
#[derive(Debug, thiserror::Error)]
pub enum MetaSwarmClientError {
    /// `EFE_META_SWARM_CONTROLLER_ENDPOINT` is not a syntactically
    /// valid URI.  Should be caught by `ServerConfig::validate` first ;
    /// this variant is defence in depth.
    #[error("invalid endpoint URI : {0}")]
    InvalidEndpoint(String),

    /// The TCP/HTTP-2 handshake to the controller failed within
    /// `connect_timeout`.  Most likely the controller is not running
    /// or the endpoint port is wrong.
    #[error("connect to controller failed : {0}")]
    ConnectFailed(#[source] tonic::transport::Error),

    /// The controller answered Health but with a status code other
    /// than `STATUS_SERVING (1)`.
    #[error("Health probe returned non-SERVING status : got status code {0}")]
    UnhealthyController(i32),

    /// The Health RPC itself failed (network drop, timeout, etc.).
    #[error("Health probe RPC failed : {0}")]
    HealthRpcFailed(#[source] tonic::Status),

    /// Couche 3 M2 — l'`Iterate` servi a échoué côté contrôleur.
    ///
    /// Distincte de [`Self::HealthRpcFailed`] à dessein : un `Iterate` peut échouer pour des
    /// raisons **métier** que l'appelant doit pouvoir distinguer d'une panne de transport —
    /// `ABORTED` (non-convergence fail-closed, Couche 3 M1/M2), `FAILED_PRECONDITION` (sortie de
    /// boîte de sûreté), `DEADLINE_EXCEEDED` (budget mural). Écraser tout cela en « erreur de
    /// santé » ferait perdre à la façade HTTP la seule information qui compte pour son client.
    #[error("Iterate RPC failed : {0}")]
    IterateRpcFailed(#[source] tonic::Status),
}

/// Cheap-to-clone gRPC client wrapper.  Holds an `Arc`'d tonic
/// channel internally ; per-RPC code paths just clone the wrapper.
#[derive(Debug, Clone)]
pub struct MetaSwarmClient {
    inner: Arc<MetaSwarmClientInner>,
}

#[derive(Debug)]
struct MetaSwarmClientInner {
    /// The tonic-generated client.  Wrapped in `Mutex` because
    /// `MetaSwarmServiceClient<Channel>` is `!Sync`.  Contention is
    /// non-existent in PR #11 (only the startup probe takes the lock).
    client: Mutex<MetaSwarmServiceClient<Channel>>,
    endpoint: String,
    metrics: MetaSwarmMetrics,
}

impl MetaSwarmClient {
    /// Build the channel + optionally probe Health.  Called once at
    /// `efe-server` startup when `config.enabled` is `true`.
    ///
    /// # Errors
    ///
    /// * [`MetaSwarmClientError::InvalidEndpoint`] — the endpoint URI
    ///   is malformed.
    /// * [`MetaSwarmClientError::ConnectFailed`] — the channel can't
    ///   be established within `config.connect_timeout`.
    /// * [`MetaSwarmClientError::UnhealthyController`] — Health probe
    ///   is enabled and returned non-SERVING.
    /// * [`MetaSwarmClientError::HealthRpcFailed`] — Health probe RPC
    ///   itself returned an error status.
    pub async fn connect(
        config: &MetaSwarmConfig,
        metrics: MetaSwarmMetrics,
    ) -> Result<Self, MetaSwarmClientError> {
        let endpoint = Endpoint::from_shared(config.controller_endpoint.clone())
            .map_err(|e| MetaSwarmClientError::InvalidEndpoint(e.to_string()))?
            .connect_timeout(config.connect_timeout)
            .timeout(config.call_timeout);

        // **PR #11 AC11** — when `health_probe_at_startup=false`, the
        // operator explicitly opts out of fail-fast.  We must use
        // `connect_lazy()` so a transient unreachable controller does
        // NOT block `efe-server` boot.  `connect_lazy()` returns a
        // Channel that defers TCP setup until the first RPC ; the
        // first dispatch in PR #15 will surface any persistent
        // connect failure via the call-level `Unavailable` status.
        //
        // When `health_probe_at_startup=true` (default), we use the
        // eager `connect()` so the operator gets immediate feedback
        // at boot if the controller is unreachable (AC10 fail-fast).
        let channel = if config.health_probe_at_startup {
            endpoint
                .connect()
                .await
                .map_err(MetaSwarmClientError::ConnectFailed)?
        } else {
            endpoint.connect_lazy()
        };

        let client = MetaSwarmServiceClient::new(channel);
        let inner = Arc::new(MetaSwarmClientInner {
            client: Mutex::new(client),
            endpoint: config.controller_endpoint.clone(),
            metrics,
        });
        let this = Self { inner };

        if config.health_probe_at_startup {
            this.health_probe().await?;
        }
        Ok(this)
    }

    /// Probe `MetaSwarmService::Health`.  Returns `Ok` if status is
    /// `STATUS_SERVING (1)` **or** `STATUS_RELOADING (3)`.
    /// Increments `health_probes_total` always and
    /// `health_probe_errors_total` on failure.
    ///
    /// ## PR #12 AD13 — accepting `RELOADING (3)` as transient
    ///
    /// PR #11 rejected every status code ≠ 1 with `UnhealthyController`.
    /// PR #12 ships SIGHUP-driven hot reload, which briefly flips the
    /// controller's `HealthState` to `RELOADING (3)` for the (~µs)
    /// write-lock window.  Without the relaxation below, an
    /// `efe-server` boot probe that lands in that window would
    /// fail-fast spuriously.  Accepting `RELOADING` is operationally
    /// equivalent to "this controller is fine for your purposes"
    /// because in-flight Iterates use their pre-reload snapshot
    /// (see `EFE_A1_PHASE2_PR12_ARCHITECTURE.md` § 14).
    ///
    /// # Errors
    ///
    /// * [`MetaSwarmClientError::HealthRpcFailed`] on transport error.
    /// * [`MetaSwarmClientError::UnhealthyController`] on any other
    ///   status code (e.g. `NOT_SERVING (2)` ; `UNSPECIFIED (0)`).
    #[instrument(skip(self), fields(endpoint = %self.inner.endpoint))]
    pub async fn health_probe(&self) -> Result<(), MetaSwarmClientError> {
        let start = Instant::now();
        self.inner.metrics.health_probes_total.inc();
        let mut client = self.inner.client.lock().await;
        let resp = match client.health(Request::new(HealthRequest {})).await {
            Ok(r) => r.into_inner(),
            Err(s) => {
                self.inner.metrics.health_probe_errors_total.inc();
                return Err(MetaSwarmClientError::HealthRpcFailed(s));
            }
        };
        let elapsed = start.elapsed();
        self.inner
            .metrics
            .health_probe_duration_seconds
            .observe(elapsed.as_secs_f64());
        // STATUS_SERVING == 1, STATUS_RELOADING == 3 — both accepted as
        // "the controller is up and willing to take traffic".  Other
        // codes (UNSPECIFIED 0, NOT_SERVING 2, future variants) remain
        // hard failures.
        match resp.status {
            1 => {
                debug!(elapsed_us = %elapsed.as_micros(), "Health probe OK (SERVING)");
                Ok(())
            }
            3 => {
                // PR #12 AD13 : accept as transient.
                debug!(
                    elapsed_us = %elapsed.as_micros(),
                    "Health probe OK (RELOADING) — controller is mid-reload, accepting as transient (PR #12 AD13)",
                );
                Ok(())
            }
            other => {
                self.inner.metrics.health_probe_errors_total.inc();
                warn!(
                    status = other,
                    "Health probe returned non-SERVING / non-RELOADING status",
                );
                Err(MetaSwarmClientError::UnhealthyController(other))
            }
        }
    }

    /// The endpoint URL the client is bound to.  Used by metrics
    /// labels + tracing spans.
    #[must_use]
    /// Couche 3 M2 — exécute une ronde de consensus sur le contrôleur méta-essaim.
    ///
    /// Le passage est **transparent** : la requête part telle quelle et la réponse revient telle
    /// quelle, `meta_provenance_root` et `meta_provenance_bundle` compris. C'est délibéré — cette
    /// façade ne doit ni fabriquer ni retoucher un reçu. Si elle re-sérialisait le bundle, ce que
    /// l'auditeur re-prouve ne serait plus ce que le contrôleur a scellé, et le seul moyen de s'en
    /// apercevoir serait un échec de re-preuve chez le tiers.
    ///
    /// # Errors
    /// [`MetaSwarmClientError::IterateRpcFailed`] — le contrôleur a refusé ou échoué ; le
    /// [`tonic::Status`] d'origine est conservé, code compris.
    #[instrument(skip_all, fields(endpoint = %self.inner.endpoint))]
    pub async fn iterate(&self, req: IterateRequest) -> Result<IterateResponse, MetaSwarmClientError> {
        let mut client = self.inner.client.lock().await;
        client
            .iterate(Request::new(req))
            .await
            .map(tonic::Response::into_inner)
            .map_err(MetaSwarmClientError::IterateRpcFailed)
    }

    pub fn endpoint(&self) -> &str {
        &self.inner.endpoint
    }
}

/// Test helper : allocate a localhost port then immediately release
/// it, returning a `SocketAddr` that is **guaranteed closed** for the
/// brief window before any other process binds it.  The same pattern
/// is used by PR #10's `ProcessHarness::allocate_port` ; on localhost
/// it is empirically race-free.
#[cfg(test)]
async fn allocate_closed_port() -> std::net::SocketAddr {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind 127.0.0.1:0");
    let addr = l.local_addr().expect("local_addr");
    drop(l);
    addr
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::Registry;
    use std::time::Duration;

    fn test_metrics() -> MetaSwarmMetrics {
        MetaSwarmMetrics::register(&Registry::new()).expect("metrics register")
    }

    #[tokio::test]
    async fn connect_rejects_malformed_endpoint() {
        let cfg = MetaSwarmConfig {
            enabled: true,
            controller_endpoint: "not a valid uri".into(),
            ..MetaSwarmConfig::default()
        };
        let err = MetaSwarmClient::connect(&cfg, test_metrics())
            .await
            .unwrap_err();
        assert!(
            matches!(err, MetaSwarmClientError::InvalidEndpoint(_)),
            "{err:?}",
        );
    }

    #[tokio::test]
    async fn connect_fails_when_endpoint_unreachable() {
        // Allocate a localhost port then drop the listener ; the
        // port is now closed.  Subsequent connect attempts get
        // ECONNREFUSED.
        let addr = allocate_closed_port().await;
        let cfg = MetaSwarmConfig {
            enabled: true,
            controller_endpoint: format!("http://{addr}"),
            connect_timeout: Duration::from_millis(500),
            ..MetaSwarmConfig::default()
        };
        let err = MetaSwarmClient::connect(&cfg, test_metrics())
            .await
            .unwrap_err();
        assert!(
            matches!(err, MetaSwarmClientError::ConnectFailed(_)),
            "{err:?}",
        );
    }

    #[tokio::test]
    async fn endpoint_accessor_returns_configured_url() {
        // We only need to reach the InvalidEndpoint branch ; no
        // network involved.  Then we directly construct a client to
        // assert the getter (since `connect` fails, we synthesise
        // the inner state without going through it).
        let cfg = MetaSwarmConfig {
            enabled: true,
            controller_endpoint: "http://example.invalid:1".into(),
            connect_timeout: Duration::from_millis(50),
            ..MetaSwarmConfig::default()
        };
        // Will fail (no listener) but exercises the InvalidEndpoint
        // / ConnectFailed branches.  The accessor test below uses
        // a directly-built handle.
        let _ = MetaSwarmClient::connect(&cfg, test_metrics()).await;

        // Build a fake client by hand to exercise `endpoint()`.
        // We can't construct `MetaSwarmServiceClient<Channel>`
        // without an actual channel, so we go through
        // `Endpoint::from_static(..).connect_lazy()` which never
        // performs network I/O.
        let chan = Endpoint::from_static("http://example.invalid:1").connect_lazy();
        let inner = Arc::new(MetaSwarmClientInner {
            client: Mutex::new(MetaSwarmServiceClient::new(chan)),
            endpoint: "http://example.invalid:1".to_string(),
            metrics: test_metrics(),
        });
        let h = MetaSwarmClient { inner };
        assert_eq!(h.endpoint(), "http://example.invalid:1");
    }

    #[tokio::test]
    async fn metrics_counter_stays_zero_when_eager_connect_fails_before_probe() {
        // `health_probe_at_startup=true` triggers the eager TCP
        // connect ; against a closed port that fails with
        // `ConnectFailed` BEFORE the probe ever runs.  Both probe
        // counters must therefore stay at zero — they only tick
        // when the probe RPC is actually dispatched.
        let addr = allocate_closed_port().await;
        let cfg = MetaSwarmConfig {
            enabled: true,
            controller_endpoint: format!("http://{addr}"),
            connect_timeout: Duration::from_millis(500),
            health_probe_at_startup: true,
            ..MetaSwarmConfig::default()
        };
        let m = test_metrics();
        let err = MetaSwarmClient::connect(&cfg, m.clone()).await.unwrap_err();
        assert!(matches!(err, MetaSwarmClientError::ConnectFailed(_)));
        assert_eq!(
            m.health_probes_total.get() as u64,
            0,
            "probes_total must NOT increment when eager connect fails before any probe",
        );
        assert_eq!(
            m.health_probe_errors_total.get() as u64,
            0,
            "errors_total must NOT increment when probe never runs",
        );
    }

    #[tokio::test]
    async fn probe_disabled_against_closed_port_returns_lazy_client() {
        // **AC11 unit-level mirror** : with `health_probe_at_startup
        // = false`, `connect` uses `connect_lazy()` and therefore
        // succeeds even against a closed port.  Boot does not
        // fail-fast ; the operator deliberately opted out.  PR #15
        // dispatch will surface the unreachability via per-RPC
        // `Unavailable` status.
        let addr = allocate_closed_port().await;
        let cfg = MetaSwarmConfig {
            enabled: true,
            controller_endpoint: format!("http://{addr}"),
            connect_timeout: Duration::from_millis(500),
            health_probe_at_startup: false, // <-- the key contract
            ..MetaSwarmConfig::default()
        };
        let m = test_metrics();
        let client = MetaSwarmClient::connect(&cfg, m.clone())
            .await
            .expect("lazy connect must succeed when probe is off (AC11)");
        assert_eq!(client.endpoint(), format!("http://{addr}"));
        // Probe never ran — counters stay at zero.
        assert_eq!(m.health_probes_total.get() as u64, 0);
        assert_eq!(m.health_probe_errors_total.get() as u64, 0);
    }

    #[tokio::test]
    async fn lazy_connect_lets_probe_fail_with_transport_error() {
        // tonic's `connect_lazy()` returns a Channel that defers
        // TCP setup until first RPC.  We use it to bypass `connect`
        // (which would fail) and observe `health_probe`'s error
        // path, including metric increments.
        let addr = allocate_closed_port().await;
        let chan = Endpoint::from_shared(format!("http://{addr}"))
            .unwrap()
            .timeout(Duration::from_millis(500))
            .connect_timeout(Duration::from_millis(500))
            .connect_lazy();
        let m = test_metrics();
        let inner = Arc::new(MetaSwarmClientInner {
            client: Mutex::new(MetaSwarmServiceClient::new(chan)),
            endpoint: format!("http://{addr}"),
            metrics: m.clone(),
        });
        let client = MetaSwarmClient { inner };
        let err = client.health_probe().await.unwrap_err();
        assert!(
            matches!(err, MetaSwarmClientError::HealthRpcFailed(_)),
            "{err:?}",
        );
        assert_eq!(
            m.health_probes_total.get() as u64,
            1,
            "probes_total must increment on every attempt",
        );
        assert_eq!(
            m.health_probe_errors_total.get() as u64,
            1,
            "errors_total must increment on transport failure",
        );
    }
}
