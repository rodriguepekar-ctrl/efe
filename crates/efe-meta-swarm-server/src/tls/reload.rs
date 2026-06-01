//! Cert reload coordinator.
//!
//! Owns the *current* [`TlsServerConfig`] and [`TlsClientConfig`]
//! values in `ArcSwap`s — readers (the tonic server / client builders)
//! get a lock-free, wait-free read of the active config.  On a
//! cert-rotation event, the reloader re-reads the cert / key / CA
//! files from disk, rebuilds the config, and performs an atomic
//! [`arc_swap::ArcSwap::store`].
//!
//! ## Connection drain
//!
//! `tonic`'s `Server::tls_config(...)` is set at server-build time —
//! changing the rustls config does NOT propagate to the underlying
//! `tokio_rustls::TlsAcceptor` after `serve()` has been called.
//! Instead, we let the reloader own the canonical config and a
//! `tokio::sync::broadcast::Sender<TlsReloadEvent>` that the
//! controller's gRPC layer listens on : when a reload happens, the
//! gRPC layer rotates the per-participant `tonic::Channel`s (each
//! handshakes anew using the new client config) and the server
//! lets existing connections drain naturally over the
//! [`super::DEFAULT_DRAIN_GRACE_S`] window.
//!
//! Per AD10 : in-flight RPCs **never** get force-killed.  HTTP/2
//! streams complete on the current TLS session keys (which were
//! derived during the handshake and don't depend on the config
//! anymore).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use tokio::sync::broadcast;
use tokio::time::interval;
use tracing::{error, info, info_span, warn};

use super::audit::{default_audit_emitter, SharedAuditEmitter, TlsAuditEvent};
use super::config::{TlsClientConfig, TlsServerConfig};
use super::metrics::{ReloadOutcome, Role, TlsMetrics};
use super::reader::{
    load_ca_bundle, load_cert_chain, load_private_key, parse_cert_expiry, parse_cert_subject_cn,
};
use super::{TlsError, CERT_EXPIRY_SAMPLE_INTERVAL_S, DEFAULT_DRAIN_GRACE_S};

/// Strongly-typed paths to the cert / key / CA files plus identity
/// metadata used by the reader + the metrics layer.
#[derive(Debug, Clone)]
pub struct TlsPaths {
    /// Path to the leaf+intermediate chain PEM (`tls.crt`).
    pub cert_chain: PathBuf,
    /// Path to the matching private key PEM (`tls.key`).
    pub key: PathBuf,
    /// Path to the CA bundle PEM (`ca.crt`).
    pub ca: PathBuf,
}

/// Alias preserving the architecture-doc Appendix A name
/// (`ReloadResult`) ; the canonical enum is
/// [`crate::tls::metrics::ReloadOutcome`].
pub type ReloadResult = super::metrics::ReloadOutcome;

/// Reload event broadcast to subscribers when the config changes.
#[derive(Debug, Clone)]
pub enum TlsReloadEvent {
    /// Reload completed successfully ; subscribers should rotate any
    /// per-peer state that captures the *client* identity (e.g.
    /// outbound `tonic::Channel`s).
    Reloaded {
        /// Wall-clock timestamp of the reload.
        at: std::time::SystemTime,
    },
    /// Reload failed ; subscribers retain their existing state.  The
    /// failure has already been logged + metric-incremented.
    Failed {
        /// Human-readable reason (the [`TlsError::to_string`] form).
        reason: String,
    },
}

/// The reload coordinator.  Holds the current configs + the file
/// paths and brokers reload events to the gRPC layer.
pub struct TlsReloader {
    /// Strongly-typed paths to the cert + key + CA bundle.
    paths: TlsPaths,
    /// mTLS / server-side TLS policy (toggles client cert verification).
    require_mutual: bool,
    /// Allowed peer SANs (mTLS only).
    allowed_peer_sans: Vec<String>,
    /// Expected server SAN (controller-as-client → participant-as-server).
    expected_server_san: String,
    /// Minimum protocol version.
    min_protocol: super::ProtocolVersion,
    /// Current server config (atomic).
    current_server: ArcSwap<TlsServerConfig>,
    /// Current client config (atomic).
    current_client: ArcSwap<TlsClientConfig>,
    /// Subscriber bus for reload events.
    events: broadcast::Sender<TlsReloadEvent>,
    /// Metrics handle.
    metrics: Arc<TlsMetrics>,
    /// Drain grace window for the gRPC layer.
    drain_grace: Duration,
    /// PR #16 § 23 — audit-chain emitter (no-op until audit infra
    /// lands).  Replaceable via [`Self::set_audit_emitter`].
    audit: SharedAuditEmitter,
    /// PR #16 audit-pass — the cert-file watcher.  Stored here (vs
    /// `Box::leak`'d at the binary's `bootstrap_tls`) so the OS
    /// inotify FD is released when the reloader's `Arc` count
    /// reaches zero (i.e. binary shutdown).  Wrapped in a `Mutex`
    /// only to allow late attachment via [`Self::attach_watcher`].
    watcher: std::sync::Mutex<Option<super::CertWatcher>>,
}

impl std::fmt::Debug for TlsReloader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsReloader")
            .field("paths", &self.paths)
            .field("require_mutual", &self.require_mutual)
            .field("allowed_peer_sans", &self.allowed_peer_sans)
            .field("expected_server_san", &self.expected_server_san)
            .field("min_protocol", &self.min_protocol)
            .field("drain_grace", &self.drain_grace)
            .finish_non_exhaustive()
    }
}

impl TlsReloader {
    /// Build a reloader by reading the initial config from disk.
    ///
    /// Errors if any of the cert / key / CA files are missing or
    /// unparseable (fail-closed, per AD25).
    pub fn bootstrap(
        paths: TlsPaths,
        require_mutual: bool,
        allowed_peer_sans: Vec<String>,
        expected_server_san: String,
        min_protocol: super::ProtocolVersion,
        metrics: Arc<TlsMetrics>,
    ) -> Result<Self, TlsError> {
        let server = read_server_config(
            &paths,
            require_mutual,
            allowed_peer_sans.clone(),
            min_protocol,
        )?;
        let client = read_client_config(&paths, expected_server_san.clone(), min_protocol)?;

        let (events, _) = broadcast::channel(16);

        Ok(Self {
            paths,
            require_mutual,
            allowed_peer_sans,
            expected_server_san,
            min_protocol,
            current_server: ArcSwap::from_pointee(server),
            current_client: ArcSwap::from_pointee(client),
            events,
            metrics,
            drain_grace: Duration::from_secs(DEFAULT_DRAIN_GRACE_S),
            audit: default_audit_emitter(),
            watcher: std::sync::Mutex::new(None),
        })
    }

    /// Replace the audit-chain emitter.  Useful for tests + for the
    /// future audit-chain PR that will install a `PgAuditEmitter`.
    pub fn set_audit_emitter(&mut self, emitter: SharedAuditEmitter) {
        self.audit = emitter;
    }

    /// PR #16 audit-pass — store the cert-file watcher inside the
    /// reloader so its OS handles live as long as the reloader does.
    ///
    /// Callable after `Arc::new(reloader)` because of the inner Mutex.
    /// Replaces the previous `Box::leak` pattern.
    pub fn attach_watcher(&self, watcher: super::CertWatcher) {
        if let Ok(mut g) = self.watcher.lock() {
            *g = Some(watcher);
        }
    }

    /// True if a watcher has been attached.  Tests + diagnostics.
    pub fn has_watcher(&self) -> bool {
        self.watcher.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// Hand out the current server config to a tonic builder.
    #[must_use]
    pub fn current_server_config(&self) -> Arc<TlsServerConfig> {
        self.current_server.load_full()
    }

    /// Hand out the current client config to a tonic builder.
    #[must_use]
    pub fn current_client_config(&self) -> Arc<TlsClientConfig> {
        self.current_client.load_full()
    }

    /// Subscribe to reload events.  Receivers MUST stay subscribed
    /// for the duration they care about reloads (a dropped receiver
    /// will miss future events).
    pub fn subscribe(&self) -> broadcast::Receiver<TlsReloadEvent> {
        self.events.subscribe()
    }

    /// Configured drain grace window.
    #[must_use]
    pub fn drain_grace(&self) -> Duration {
        self.drain_grace
    }

    /// Re-read every file from disk, rebuild configs, atomically
    /// swap, broadcast the event.
    pub async fn handle_cert_change(&self, path: PathBuf) -> Result<(), TlsError> {
        let span = info_span!("tls.reload", path = %path.display());
        let _enter = span.enter();
        let start = Instant::now();

        match self.reload_inner() {
            Ok(()) => {
                self.metrics.record_reload(ReloadOutcome::Ok, Role::Server);
                info!(elapsed_ms = ?start.elapsed().as_millis(), "tls.reload OK");
                let _ = self.events.send(TlsReloadEvent::Reloaded {
                    at: std::time::SystemTime::now(),
                });
                // PR #16 § 23 — audit chain emission (no-op today,
                // wired when audit infra lands).
                let new_cn = self.snapshot_leaf_cn();
                self.audit.emit(TlsAuditEvent::Reload {
                    success: true,
                    triggering_path: path.display().to_string(),
                    new_cert_cn: new_cn,
                });
                Ok(())
            }
            Err(e) => {
                self.metrics
                    .record_reload(ReloadOutcome::Fail, Role::Server);
                error!(error = %e, "tls.reload FAILED — keeping previous config");
                let _ = self.events.send(TlsReloadEvent::Failed {
                    reason: e.to_string(),
                });
                self.audit.emit(TlsAuditEvent::Reload {
                    success: false,
                    triggering_path: path.display().to_string(),
                    new_cert_cn: None,
                });
                Err(e)
            }
        }
    }

    /// Read the current leaf cert's CN, for audit metadata.
    fn snapshot_leaf_cn(&self) -> Option<String> {
        let cert_chain = load_cert_chain(&self.paths.cert_chain).ok()?;
        cert_chain.first().map(parse_cert_subject_cn)
    }

    fn reload_inner(&self) -> Result<(), TlsError> {
        let server = read_server_config(
            &self.paths,
            self.require_mutual,
            self.allowed_peer_sans.clone(),
            self.min_protocol,
        )?;
        let client = read_client_config(
            &self.paths,
            self.expected_server_san.clone(),
            self.min_protocol,
        )?;

        self.current_server.store(Arc::new(server));
        self.current_client.store(Arc::new(client));

        Ok(())
    }

    /// Spawn the periodic cert-expiry sampling loop ; emits gauge
    /// updates every [`CERT_EXPIRY_SAMPLE_INTERVAL_S`] seconds.
    ///
    /// Returns a `JoinHandle` so callers can await graceful shutdown.
    pub fn spawn_cert_expiry_loop(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(CERT_EXPIRY_SAMPLE_INTERVAL_S));
            // First tick fires immediately ; subsequent at cadence.
            loop {
                ticker.tick().await;
                if let Err(e) = self.sample_cert_expiry() {
                    warn!(error = %e, "tls cert-expiry sampling failed");
                }
            }
        })
    }

    /// Snapshot the current server config's leaf cert + push the
    /// remaining-seconds gauge.
    ///
    /// PR #16 AC23 — emits an OpenTelemetry-compatible
    /// `tracing::info_span!("tls.cert_check")` so distributed traces
    /// can correlate a cert-expiry observation with the upstream
    /// `tls.reload` events.
    pub fn sample_cert_expiry(&self) -> Result<(), TlsError> {
        let span = tracing::info_span!(
            "tls.cert_check",
            cert_file = %self.paths.cert_chain.display(),
            min_protocol = ?self.min_protocol,
        );
        let _enter = span.enter();
        let server = self.current_server.load();
        let cert_chain = read_cert_chain(&self.paths.cert_chain)?;
        // The leaf is the first cert in the chain (rustls
        // convention).
        if let Some(leaf) = cert_chain.first() {
            let secs = parse_cert_expiry(leaf)?;
            let cn = parse_cert_subject_cn(leaf);
            tracing::debug!(
                cn = %cn,
                seconds_remaining = secs,
                "tls.cert_check: cert expiry observed"
            );
            self.metrics.record_cert_expiry(
                self.paths
                    .cert_chain
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("leaf"),
                &cn,
                secs,
            );
        }
        drop(server);
        Ok(())
    }

    /// Glue task : drain events from the watcher and call
    /// [`Self::handle_cert_change`] for each one.
    pub fn spawn_reload_task(
        self: Arc<Self>,
        mut rx: tokio::sync::mpsc::Receiver<super::CertChangeEvent>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                if let Err(e) = self.handle_cert_change(ev.path.clone()).await {
                    warn!(error = %e, path = %ev.path.display(), "reload error");
                }
            }
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
// Helpers.
// ─────────────────────────────────────────────────────────────────────

fn read_cert_chain(
    path: &std::path::Path,
) -> Result<Vec<rustls_pki_types::CertificateDer<'static>>, TlsError> {
    load_cert_chain(path)
}

fn read_server_config(
    paths: &TlsPaths,
    require_mutual: bool,
    allowed_peer_sans: Vec<String>,
    min_protocol: super::ProtocolVersion,
) -> Result<TlsServerConfig, TlsError> {
    let cert_chain_pem = std::fs::read(&paths.cert_chain).map_err(|source| TlsError::Io {
        path: paths.cert_chain.clone(),
        source,
    })?;
    let key_pem = std::fs::read(&paths.key).map_err(|source| TlsError::Io {
        path: paths.key.clone(),
        source,
    })?;
    let client_ca_pem = if require_mutual {
        let ca = std::fs::read(&paths.ca).map_err(|source| TlsError::Io {
            path: paths.ca.clone(),
            source,
        })?;
        Some(ca)
    } else {
        None
    };

    // Sanity-check parseability before handing to ArcSwap (avoids
    // landing a half-baked config into a hot path).
    let _certs = load_cert_chain(&paths.cert_chain)?;
    let _key = load_private_key(&paths.key)?;
    if require_mutual {
        let _ca = load_ca_bundle(&paths.ca)?;
    }

    Ok(TlsServerConfig {
        cert_chain_pem,
        key_pem,
        client_ca_pem,
        min_protocol,
        require_mutual,
        allowed_peer_sans,
    })
}

fn read_client_config(
    paths: &TlsPaths,
    expected_server_san: String,
    min_protocol: super::ProtocolVersion,
) -> Result<TlsClientConfig, TlsError> {
    let cert_chain_pem = std::fs::read(&paths.cert_chain).map_err(|source| TlsError::Io {
        path: paths.cert_chain.clone(),
        source,
    })?;
    let key_pem = std::fs::read(&paths.key).map_err(|source| TlsError::Io {
        path: paths.key.clone(),
        source,
    })?;
    let server_ca_pem = std::fs::read(&paths.ca).map_err(|source| TlsError::Io {
        path: paths.ca.clone(),
        source,
    })?;

    // Sanity-check parseability before publishing.
    let _certs = load_cert_chain(&paths.cert_chain)?;
    let _key = load_private_key(&paths.key)?;
    let _ca = load_ca_bundle(&paths.ca)?;

    Ok(TlsClientConfig {
        cert_chain_pem,
        key_pem,
        server_ca_pem,
        min_protocol,
        expected_server_san,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::ProtocolVersion;
    use prometheus::Registry;

    fn dummy_paths(tmp: &std::path::Path) -> TlsPaths {
        let cert = tmp.join("tls.crt");
        let key = tmp.join("tls.key");
        let ca = tmp.join("ca.crt");
        // Touch the files so the IO error path is the only one we
        // can trigger without rcgen.
        std::fs::write(&cert, b"").unwrap();
        std::fs::write(&key, b"").unwrap();
        std::fs::write(&ca, b"").unwrap();
        TlsPaths {
            cert_chain: cert,
            key,
            ca,
        }
    }

    #[test]
    fn bootstrap_propagates_empty_file_error() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = dummy_paths(tmp.path());
        let metrics = Arc::new(TlsMetrics::register(&Registry::new()).unwrap());
        let err = TlsReloader::bootstrap(
            paths,
            true,
            vec!["controller.efe.svc".into()],
            "participant.efe.svc".into(),
            ProtocolVersion::Tls13,
            metrics,
        )
        .unwrap_err();
        // Empty cert file is the first failure we hit.
        assert!(matches!(err, TlsError::EmptyCertFile(_)));
    }

    #[tokio::test]
    async fn handle_cert_change_with_unreadable_paths_reports_fail_metric() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TlsPaths {
            cert_chain: tmp.path().join("missing.crt"),
            key: tmp.path().join("missing.key"),
            ca: tmp.path().join("missing.ca"),
        };
        let metrics = Arc::new(TlsMetrics::register(&Registry::new()).unwrap());

        // Force-construct the reloader bypassing the bootstrap check.
        let reloader = TlsReloader {
            paths,
            require_mutual: false,
            allowed_peer_sans: vec![],
            expected_server_san: "n/a".into(),
            min_protocol: ProtocolVersion::Tls13,
            current_server: ArcSwap::from_pointee(TlsServerConfig {
                cert_chain_pem: vec![],
                key_pem: vec![],
                client_ca_pem: None,
                min_protocol: ProtocolVersion::Tls13,
                require_mutual: false,
                allowed_peer_sans: vec![],
            }),
            current_client: ArcSwap::from_pointee(TlsClientConfig {
                cert_chain_pem: vec![],
                key_pem: vec![],
                server_ca_pem: vec![],
                min_protocol: ProtocolVersion::Tls13,
                expected_server_san: "n/a".into(),
            }),
            events: broadcast::channel(4).0,
            metrics: metrics.clone(),
            drain_grace: Duration::from_secs(DEFAULT_DRAIN_GRACE_S),
            audit: default_audit_emitter(),
            watcher: std::sync::Mutex::new(None),
        };

        let err = reloader
            .handle_cert_change(PathBuf::from("/tmp/missing.crt"))
            .await
            .unwrap_err();
        assert!(matches!(err, TlsError::Io { .. }));

        // The "fail" counter must be 1 — we wired the metric in the
        // error branch of `handle_cert_change`.
        let count = metrics
            .reload_total
            .get_metric_with_label_values(&["fail"])
            .expect("metric")
            .get();
        assert_eq!(count, 1);
    }

    #[test]
    fn drain_grace_defaults_to_30s() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TlsPaths {
            cert_chain: tmp.path().join("a"),
            key: tmp.path().join("b"),
            ca: tmp.path().join("c"),
        };
        let metrics = Arc::new(TlsMetrics::register(&Registry::new()).unwrap());
        // Bypass bootstrap to test drain_grace constant directly.
        let reloader = TlsReloader {
            paths,
            require_mutual: false,
            allowed_peer_sans: vec![],
            expected_server_san: "n/a".into(),
            min_protocol: ProtocolVersion::Tls13,
            current_server: ArcSwap::from_pointee(TlsServerConfig {
                cert_chain_pem: vec![],
                key_pem: vec![],
                client_ca_pem: None,
                min_protocol: ProtocolVersion::Tls13,
                require_mutual: false,
                allowed_peer_sans: vec![],
            }),
            current_client: ArcSwap::from_pointee(TlsClientConfig {
                cert_chain_pem: vec![],
                key_pem: vec![],
                server_ca_pem: vec![],
                min_protocol: ProtocolVersion::Tls13,
                expected_server_san: "n/a".into(),
            }),
            events: broadcast::channel(4).0,
            metrics,
            drain_grace: Duration::from_secs(DEFAULT_DRAIN_GRACE_S),
            audit: default_audit_emitter(),
            watcher: std::sync::Mutex::new(None),
        };
        assert_eq!(reloader.drain_grace(), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn subscribe_receives_failed_event_on_failed_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = TlsPaths {
            cert_chain: tmp.path().join("nope.crt"),
            key: tmp.path().join("nope.key"),
            ca: tmp.path().join("nope.ca"),
        };
        let metrics = Arc::new(TlsMetrics::register(&Registry::new()).unwrap());
        let (events_tx, _) = broadcast::channel(4);
        let reloader = TlsReloader {
            paths,
            require_mutual: false,
            allowed_peer_sans: vec![],
            expected_server_san: "n/a".into(),
            min_protocol: ProtocolVersion::Tls13,
            current_server: ArcSwap::from_pointee(TlsServerConfig {
                cert_chain_pem: vec![],
                key_pem: vec![],
                client_ca_pem: None,
                min_protocol: ProtocolVersion::Tls13,
                require_mutual: false,
                allowed_peer_sans: vec![],
            }),
            current_client: ArcSwap::from_pointee(TlsClientConfig {
                cert_chain_pem: vec![],
                key_pem: vec![],
                server_ca_pem: vec![],
                min_protocol: ProtocolVersion::Tls13,
                expected_server_san: "n/a".into(),
            }),
            events: events_tx.clone(),
            metrics,
            drain_grace: Duration::from_secs(DEFAULT_DRAIN_GRACE_S),
            audit: default_audit_emitter(),
            watcher: std::sync::Mutex::new(None),
        };

        let mut rx = events_tx.subscribe();
        let _ = reloader
            .handle_cert_change(PathBuf::from("/dev/null"))
            .await;
        let ev = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timeout")
            .expect("event");
        assert!(matches!(ev, TlsReloadEvent::Failed { .. }));
    }
}
