//! gRPC-backed implementation of the Phase-1 [`SwarmParticipant`]
//! trait.
//!
//! # Architecture
//!
//! See [`docs/EFE_A1_PHASE2_PR08_ARCHITECTURE.md`][doc] § 5 for the
//! sync ↔ async bridge rationale.  TL;DR : the trait method is sync,
//! the gRPC client is async, and we bridge via
//! `Handle::current().block_on()` — safe because the controller's
//! `iterate.rs` invokes us from inside `tokio::task::spawn_blocking`.
//!
//! [`SwarmParticipant`]: efe_meta_swarm::participants::SwarmParticipant
//! [doc]: ../../../../docs/EFE_A1_PHASE2_PR08_ARCHITECTURE.md
//!
//! # Phase-2 simplification : `gamma` + `alpha` are dropped
//!
//! The Phase-1 trait method `step_with_pull(pull, gamma, alpha)`
//! requires the caller to pass the coupling coefficients.  Phase-2
//! `StepRequest` does NOT carry them on the wire — the participant
//! binary applies its locally-known values from its manifest.  For
//! determinism, both binaries MUST read the same manifest version
//! (enforced by the L2 graduation soak protocol).  The unused
//! parameters in `RemoteParticipant::step_with_pull` are silently
//! ignored ; documented in [`step_with_pull`] below.
//!
//! Phase-3 follow-up : `DT-A3-STEP-PARAMS-ON-WIRE` adds `gamma` +
//! `alpha` fields to `StepRequest` for explicit propagation.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use efe_meta_swarm::prelude::{
    MetaSwarmError, NonExpansiveCertificate, Result as MetaResult, SwarmParticipant, Tensor,
};
use efe_meta_swarm_proto::meta_swarm::{StepRequest, TensorF32};
use efe_meta_swarm_proto::participant::MetaParticipantServiceClient;
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;
use tonic::Request;
use tracing::{debug, instrument, warn};

use crate::tracing_otel::propagator::{inject_current_context, W3CTraceContextPropagator};

/// Convert a Phase-1 `Tensor` into a wire-format `TensorF32`.
#[must_use]
pub fn tensor_to_proto(t: &Tensor) -> TensorF32 {
    TensorF32 {
        dim: u32::try_from(t.dim()).expect("tensor dim fits in u32"),
        data: t.as_slice().to_vec(),
    }
}

/// Validate a wire-format `TensorF32` and convert to Phase-1
/// `Tensor`.  Returns an error if `dim != data.len()`.
pub fn proto_to_tensor(p: &TensorF32) -> Result<Tensor, MetaSwarmError> {
    if p.dim as usize != p.data.len() {
        return Err(MetaSwarmError::Internal(format!(
            "TensorF32 : dim={} but data.len()={}",
            p.dim,
            p.data.len(),
        )));
    }
    Ok(Tensor::from_slice(&p.data))
}

/// gRPC client wrapper that implements the Phase-1
/// [`SwarmParticipant`] trait.
///
/// Used by the controller in place of the in-process `MockParticipant`.
///
/// [`SwarmParticipant`]: efe_meta_swarm::participants::SwarmParticipant
pub struct RemoteParticipant {
    inner: Arc<RemoteParticipantInner>,
}

struct RemoteParticipantInner {
    id: String,
    dim: usize,
    /// gRPC client.  Wrapped in `Mutex` because tonic clients are
    /// not `Sync` (the inner stream state mutates on each call).
    client: Mutex<MetaParticipantServiceClient<Channel>>,
    /// Cancellation token observed on every Step.
    cancel: CancellationToken,
    /// Per-step deadline (defensive ; primary deadline lives on the
    /// outer iterate's `max_wall_ms`).
    step_deadline: Duration,
    /// Tokio runtime handle captured at `connect` time.  Phase-1's
    /// `MetaSwarm::iterate` parallelises participants via
    /// `rayon::par_iter` ; those rayon workers are NOT tokio-managed
    /// and `Handle::try_current()` would fail there.  By stashing the
    /// handle, the sync trait methods can always `runtime.block_on(...)`
    /// regardless of which thread pool drives them.
    runtime: Handle,
    /// PR #18 AC6/AC7/AC8 — W3C Trace Context propagator instance
    /// reused across all outbound calls so the controller's currently
    /// active `efe.meta_swarm.iterate` span context is injected as
    /// `traceparent` on every Step / Reset / State RPC.  This is the
    /// client-side complement of `ServerTraceInterceptor` installed on
    /// the participant binary.
    propagator: W3CTraceContextPropagator,
    /// PR #18 audit-pass — monotonic step counter sent as
    /// `StepRequest.step_index` on every Step RPC.  Resets to 0 on
    /// every successful Reset RPC (the participant's reset_gen bumps,
    /// so it makes sense to restart our local counter).  Phase-2 has
    /// no out-of-band channel for the controller-side iteration number
    /// to reach this struct ; this per-instance counter is the next
    /// best thing for the `meta_swarm.step.step_index` attribute on
    /// the participant-side `efe.meta_swarm.step` span.
    step_counter: AtomicU32,
}

impl std::fmt::Debug for RemoteParticipant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteParticipant")
            .field("id", &self.inner.id)
            .field("dim", &self.inner.dim)
            .finish_non_exhaustive()
    }
}

impl Clone for RemoteParticipant {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl RemoteParticipant {
    /// Build a `RemoteParticipant` connected to `endpoint` (e.g.
    /// `http://localhost:50091`).
    ///
    /// The connection itself is established eagerly at construction
    /// time so the controller catches network errors before any
    /// `Step` call.
    ///
    /// # Errors
    ///
    /// Returns `tonic::transport::Error` if the URI is malformed,
    /// DNS resolution fails, or the TCP connect times out.
    pub async fn connect(
        id: impl Into<String>,
        dim: usize,
        endpoint: impl Into<String>,
        cancel: CancellationToken,
    ) -> Result<Self, tonic::transport::Error> {
        Self::connect_with_tls(id, dim, endpoint, cancel, None).await
    }

    /// Build a `RemoteParticipant` connected to `endpoint`, optionally
    /// over mutual TLS.
    ///
    /// PR #16 — when `tls` is `Some`, the channel is configured with
    /// `tonic::transport::ClientTlsConfig` :
    ///
    ///   * `domain_name = tls.expected_server_san` (forces rustls's
    ///     WebPKI verifier to check the server cert SAN matches).
    ///   * `ca_certificate(server_ca_pem)` (trust root).
    ///   * `identity(cert_chain_pem, key_pem)` (our client identity ;
    ///     consumed when the peer enables mTLS).
    ///
    /// The `endpoint` SHOULD use `https://...` when TLS is enabled ;
    /// tonic accepts `http://` too but emits a runtime warning.
    pub async fn connect_with_tls(
        id: impl Into<String>,
        dim: usize,
        endpoint: impl Into<String>,
        cancel: CancellationToken,
        tls: Option<crate::tls::TlsClientConfig>,
    ) -> Result<Self, tonic::transport::Error> {
        let endpoint_str = endpoint.into();

        // PR #16 AD18 / AC25 / T44 — loopback connections bypass TLS
        // regardless of `tls.enabled`.  Production K8s never routes
        // controller↔participant via loopback ; this only fires in
        // dev with `discovery: static-localhost`.  Skipping TLS on
        // loopback removes per-connection handshake overhead in the
        // hot inner-loop of unit tests + dev-mode benchmarks.
        //
        // ── M1.b — deux corrections ici, et la seconde n'est pas cosmétique ─────────────────
        //
        // 1. `endpoint_is_loopback` est désormais l'analyseur UNIQUE (ré-export). L'ancienne
        //    lecture déclarait `http://127.0.0.1:8080@evil.com` loopback alors que
        //    `tonic::transport::Endpoint` — construit trois lignes plus bas — part vers
        //    `evil.com`. Cette ligne-ci retirait donc TLS sur un lien distant.
        //
        // 2. Le journal imprimait `endpoint_str` **en entier**. Or les formes qui trompaient
        //    l'analyseur sont précisément celles qui portent un `@` : `http://user:pass@hôte/`
        //    déposait un secret dans une sortie collectée, archivée et relue. On nomme l'HÔTE
        //    (le geste de P1 sur l'avertissement de boot du SLM), jamais l'URL. `host_of` ne
        //    peut pas rendre `None` ici — nous sommes dans la branche où il a rendu un hôte
        //    loopback — mais le repli reste explicite plutôt que dénoué par `unwrap`.
        //
        // 3. Le niveau passe de `debug!` à `info!` : retirer TLS d'un lien pour lequel un
        //    appelant a fourni des certificats est un acte de souveraineté, pas un détail de mise
        //    au point. Il reste conditionné à `tls.is_some()` — annoncer un contournement là où
        //    rien n'était demandé serait une fausse alarme, et c'est ainsi qu'une vraie finit
        //    ignorée.
        //
        // ⚠ Exactitude sur la portée de cette branche : depuis M1.b, `controller::iterate` ne
        // fabrique PLUS de configuration TLS pour un lien local (le schéma et le SAN suivent le
        // même prédicat). La combinaison « `Some(tls)` + endpoint local » ne vient donc plus du
        // chemin de production, et cette branche est une ceinture pour les AUTRES appelants de
        // cette fonction publique. Elle reste utile — `connect_with_tls` doit être sûre pour qui
        // l'appelle — mais le journal d'`info!` qu'elle produit ne s'observe pas en itération
        // normale, et le prétendre serait la fausse alarme qu'on vient d'éviter.
        let effective_tls = if crate::tls::endpoint_is_loopback(&endpoint_str) {
            if tls.is_some() {
                tracing::info!(
                    host = scella_authority::host_of(&endpoint_str).unwrap_or("(illisible)"),
                    "tls.loopback_bypass: connection to loopback skipping TLS (AD18)"
                );
            }
            None
        } else {
            tls
        };

        let mut ep = tonic::transport::Endpoint::from_shared(endpoint_str)?
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .keep_alive_timeout(Duration::from_secs(10));

        if let Some(cfg) = effective_tls {
            let identity = tonic::transport::Identity::from_pem(&cfg.cert_chain_pem, &cfg.key_pem);
            let ca = tonic::transport::Certificate::from_pem(&cfg.server_ca_pem);
            let tls_cfg = tonic::transport::ClientTlsConfig::new()
                .domain_name(cfg.expected_server_san.clone())
                .ca_certificate(ca)
                .identity(identity);
            ep = ep.tls_config(tls_cfg)?;
        }

        let channel = ep.connect().await?;

        Ok(Self {
            inner: Arc::new(RemoteParticipantInner {
                id: id.into(),
                dim,
                client: Mutex::new(MetaParticipantServiceClient::new(channel)),
                cancel,
                step_deadline: Duration::from_secs(5),
                runtime: Handle::current(),
                propagator: W3CTraceContextPropagator::new(),
                step_counter: AtomicU32::new(0),
            }),
        })
    }

    /// Build a `RemoteParticipant` from an already-built `Channel`.
    /// Useful for tests that want to inject a mock channel.  MUST be
    /// called from within a tokio runtime (panics otherwise — see
    /// [`Handle::current`]).
    #[must_use]
    pub fn from_channel(
        id: impl Into<String>,
        dim: usize,
        channel: Channel,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            inner: Arc::new(RemoteParticipantInner {
                id: id.into(),
                dim,
                client: Mutex::new(MetaParticipantServiceClient::new(channel)),
                cancel,
                step_deadline: Duration::from_secs(5),
                runtime: Handle::current(),
                propagator: W3CTraceContextPropagator::new(),
                step_counter: AtomicU32::new(0),
            }),
        }
    }

    /// Async Step call.  Called from the sync trait method via
    /// `block_on`.
    async fn step_async(&self, pulled: &Tensor) -> MetaResult<Tensor> {
        // Defence in depth : controller validates dim, but the
        // remote side should also receive a sane tensor.
        if pulled.dim() != self.inner.dim {
            return Err(MetaSwarmError::DimensionMismatch {
                expected: self.inner.dim,
                actual: pulled.dim(),
            });
        }

        let pulled_proto = tensor_to_proto(pulled);

        // Acquire client lock for the call duration (tonic clients
        // are not `Sync`).
        let mut client = self.inner.client.lock().await;

        // PR #18 audit-pass — increment the per-instance step counter
        // and send it as `StepRequest.step_index` so the participant-
        // side `efe.meta_swarm.step` span gets a real
        // `meta_swarm.step.step_index` attribute (not the previous
        // always-0 stub).  `fetch_add` returns the *prior* value, so
        // the first call sends index 0, second sends 1, etc., matching
        // the participant's local LocalParticipant step counter
        // semantics.
        let step_index = self.inner.step_counter.fetch_add(1, Ordering::Relaxed);

        let mut req = Request::new(StepRequest {
            pulled_from_meta: Some(pulled_proto),
            step_index,
        });
        req.set_timeout(self.inner.step_deadline);
        // PR #18 AC6/AC7/AC8 — inject `traceparent` so the participant
        // can extract the controller's iterate span as parent.
        inject_current_context(&self.inner.propagator, req.metadata_mut());

        let resp_fut = client.step(req);

        // Race against cancellation.
        let resp = tokio::select! {
            biased;
            () = self.inner.cancel.cancelled() => {
                debug!("step cancelled mid-flight");
                return Err(MetaSwarmError::Internal("cancelled".into()));
            }
            outcome = resp_fut => outcome,
        };

        let resp = resp.map_err(|s| {
            warn!(error = %s, "Step RPC failed");
            MetaSwarmError::Internal(format!(
                "participant {} : Step RPC failed : {}",
                self.inner.id, s,
            ))
        })?;

        let proto_tensor = resp.into_inner().new_state.ok_or_else(|| {
            MetaSwarmError::Internal(format!(
                "participant {} : StepResponse.new_state is None",
                self.inner.id,
            ))
        })?;

        proto_to_tensor(&proto_tensor)
    }

    async fn reset_async(&self) -> MetaResult<()> {
        use efe_meta_swarm_proto::meta_swarm::ResetParticipantRequest;
        let mut client = self.inner.client.lock().await;
        let mut req = Request::new(ResetParticipantRequest { initial: None });
        // PR #18 AC6/AC7/AC8 — propagate traceparent on Reset RPCs.
        inject_current_context(&self.inner.propagator, req.metadata_mut());
        client.reset(req).await.map_err(|s| {
            MetaSwarmError::Internal(format!(
                "participant {} : Reset RPC failed : {}",
                self.inner.id, s,
            ))
        })?;
        // PR #18 audit-pass — reset the step counter on a successful
        // Reset RPC so the participant-side `step_index` attribute
        // restarts at 0 for the new convergence window.  Matches the
        // participant's own `LocalParticipant::reset` which bumps
        // `reset_gen` and zeroes its internal step index.
        self.inner.step_counter.store(0, Ordering::Relaxed);
        Ok(())
    }

    async fn state_async(&self) -> MetaResult<Tensor> {
        use efe_meta_swarm_proto::meta_swarm::StateRequest;
        let mut client = self.inner.client.lock().await;
        let mut req = Request::new(StateRequest {});
        // PR #18 AC6/AC7/AC8 — propagate traceparent on State RPCs.
        inject_current_context(&self.inner.propagator, req.metadata_mut());
        let resp = client.state(req).await.map_err(|s| {
            MetaSwarmError::Internal(format!(
                "participant {} : State RPC failed : {}",
                self.inner.id, s,
            ))
        })?;
        let proto_tensor = resp.into_inner().current.ok_or_else(|| {
            MetaSwarmError::Internal(format!(
                "participant {} : StateResponse.current is None",
                self.inner.id,
            ))
        })?;
        proto_to_tensor(&proto_tensor)
    }
}

impl SwarmParticipant for RemoteParticipant {
    fn id(&self) -> &str {
        &self.inner.id
    }

    fn dim(&self) -> usize {
        self.inner.dim
    }

    fn current_state(&self) -> MetaResult<Tensor> {
        if self.inner.cancel.is_cancelled() {
            return Err(MetaSwarmError::Internal("cancelled".into()));
        }
        self.inner
            .runtime
            .block_on(async move { self.state_async().await })
    }

    /// SAFETY: this method MUST be called from a thread that is *not*
    /// driving the captured tokio runtime (typically a `spawn_blocking`
    /// worker or a rayon pool worker — both of which Phase-1's
    /// `MetaSwarm::iterate` uses).  Calling from a tokio worker
    /// thread would deadlock the runtime.
    ///
    /// Phase-2 note : `_gamma` and `_alpha` are silently dropped.
    /// The participant binary uses its locally-known values from
    /// the manifest.  Determinism is preserved iff controller and
    /// participant read the same manifest version (enforced by the
    /// L2 graduation soak protocol).
    #[instrument(skip(self, pull, _gamma, _alpha), fields(participant_id = %self.inner.id))]
    fn step_with_pull(&self, pull: &Tensor, _gamma: f64, _alpha: f64) -> MetaResult<Tensor> {
        if self.inner.cancel.is_cancelled() {
            return Err(MetaSwarmError::Internal("cancelled".into()));
        }
        self.inner
            .runtime
            .block_on(async move { self.step_async(pull).await })
    }

    fn verify_non_expansive(&self) -> MetaResult<NonExpansiveCertificate> {
        // Phase-2 returns the Phase-1 stub certificate ; Phase-3
        // will fetch a signed certificate from the participant via
        // a new RPC.
        Ok(NonExpansiveCertificate::Stub)
    }

    fn reset(&self) -> MetaResult<()> {
        if self.inner.cancel.is_cancelled() {
            return Err(MetaSwarmError::Internal("cancelled".into()));
        }
        self.inner
            .runtime
            .block_on(async move { self.reset_async().await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tensor_to_proto_roundtrip() {
        let t = Tensor::from_slice(&[1.0, 2.0, 3.0]);
        let p = tensor_to_proto(&t);
        assert_eq!(p.dim, 3);
        assert_eq!(p.data, vec![1.0, 2.0, 3.0]);
        let back = proto_to_tensor(&p).unwrap();
        assert_eq!(back.dim(), 3);
        assert_eq!(back.as_slice(), &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn proto_to_tensor_rejects_dim_mismatch() {
        let p = TensorF32 {
            dim: 5,
            data: vec![1.0, 2.0],
        };
        let err = proto_to_tensor(&p).unwrap_err();
        assert!(matches!(err, MetaSwarmError::Internal(_)));
    }

    #[test]
    fn proto_to_tensor_accepts_empty() {
        let p = TensorF32 {
            dim: 0,
            data: vec![],
        };
        let t = proto_to_tensor(&p).unwrap();
        assert_eq!(t.dim(), 0);
    }
}
