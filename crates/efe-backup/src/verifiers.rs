//! Sprint D17 — **Audit fix § AC4** — real Verifier implementations.
//!
//! Pre-audit, only the `MockVerifier` existed in `crate::verify`.
//! `efe-backup verify --post-restore` returned `Ok(VerifyAggregate::default)`
//! and the DR drill smoke step was effectively a no-op. Defense-in-depth
//! promised by architecture § 10.3 was not delivered.
//!
//! Post-audit, this module ships 5 production-grade verifiers, all
//! using the [`crate::CommandRunner`] trait (so unit tests use
//! `MockCommandRunner`, production uses `TokioCommandRunner`) :
//!
//!   1. [`AuditChainVerifier`]   — shells out to
//!      `efe-server admin verify-audit-chain --full` (D14-bis path).
//!   2. [`BundleSignatureVerifier`] — shells out to
//!      `efe-bundle verify --all` (D16 path).
//!   3. [`OidcReachabilityVerifier`] — shells out to `curl -fsS <jwks_url>`
//!      for each issuer.
//!   4. [`CatalogSmokeVerifier`] — shells out to
//!      `efe-server catalog list --tenant=<canary>`.
//!   5. [`ExecuteSmokeVerifier`] — shells out to a `grpcurl` invocation
//!      hitting the `Execute` RPC with a canary payload.
//!
//! Each verifier's `check` returns `Ok(VerifierStatus { ok: bool, ... })`
//! and ONLY returns `Err(VerifyError)` for infrastructure errors
//! (process spawn failed). A failing check that ran successfully but
//! reported anomaly returns `Ok(VerifierStatus { ok: false, ... })` ;
//! the suite runner aggregates.

use std::sync::Arc;

use async_trait::async_trait;

use crate::command_runner::CommandRunner;
use crate::verify::{Verifier, VerifierKind, VerifierStatus, VerifyError};

// ====================================================================
// 1. AuditChainVerifier — re-verifies the D14-bis HMAC chain
// ====================================================================

/// Verifier that re-runs the full HMAC chain integrity check
/// against the restored Postgres `audit_log` table.
///
/// **Audit fix § D17-AC4-bis** — pre-audit this verifier shelled
/// out to `efe-server admin verify-audit-chain --full`, which is
/// a phantom subcommand : `efe-server` is a daemon that takes
/// only env vars (M12 § 8.4). The real verifier binary is
/// `efe-verify-audit` (D14-bis), which verifies all chains by
/// default and exits non-zero if any chain is broken.
///
/// **Audit fix § D17-AC4-tris** — `efe-verify-audit` requires
/// TWO mandatory arguments :
///   * `--pg-url <DSN>` : Postgres connection URL ;
///   * `--keys-file <PATH>` : HMAC keys JSON file (D14-bis).
///
/// Pre-AC4-tris, the verifier omitted both. Spawn would fail with
/// "missing required argument", causing every drill to fail at
/// the AuditChain step. Discovered via real-binary smoke test
/// (tick 16) — the binary's `--help` output documented the args
/// but the verifier never read it.
#[derive(Debug)]
pub struct AuditChainVerifier {
    runner: Arc<dyn CommandRunner>,
    /// Path to the `efe-verify-audit` binary inside the verifier pod.
    pub verify_audit_binary: String,
    /// Postgres DSN passed via `--pg-url`. Typically obtained from
    /// the `EFE_PG_*` env vars rendered by the Helm ConfigMap.
    pub pg_url: String,
    /// Path to the HMAC keys JSON file (D14-bis trust store)
    /// passed via `--keys-file`.
    pub keys_file: String,
    /// Optional chain id to verify ; `None` verifies every chain.
    pub chain: Option<String>,
}

impl AuditChainVerifier {
    /// Construct with the canonical binary path + the two mandatory
    /// args.
    pub fn new(
        runner: Arc<dyn CommandRunner>,
        verify_audit_binary: impl Into<String>,
        pg_url: impl Into<String>,
        keys_file: impl Into<String>,
    ) -> Self {
        Self {
            runner,
            verify_audit_binary: verify_audit_binary.into(),
            pg_url: pg_url.into(),
            keys_file: keys_file.into(),
            chain: None,
        }
    }

    /// Restrict the verification to a single named chain.
    #[must_use]
    pub fn with_chain(mut self, chain_id: impl Into<String>) -> Self {
        self.chain = Some(chain_id.into());
        self
    }
}

#[async_trait]
impl Verifier for AuditChainVerifier {
    fn kind(&self) -> VerifierKind {
        VerifierKind::AuditChain
    }

    async fn check(&self, _target_env: &str) -> Result<VerifierStatus, VerifyError> {
        let mut args: Vec<&str> = vec![
            "--pg-url",
            &self.pg_url,
            "--keys-file",
            &self.keys_file,
        ];
        let chain_arg;
        if let Some(c) = &self.chain {
            args.push("--chain");
            chain_arg = c.clone();
            args.push(&chain_arg);
        }
        let out = self
            .runner
            .run(&self.verify_audit_binary, &args, &[])
            .await
            .map_err(|e| VerifyError::Infra(format!("efe-verify-audit spawn: {e}")))?;
        if out.success() {
            Ok(VerifierStatus {
                kind: VerifierKind::AuditChain,
                ok: true,
                detail: out.stdout_string(),
            })
        } else {
            Ok(VerifierStatus {
                kind: VerifierKind::AuditChain,
                ok: false,
                detail: format!("exit {} : {}", out.status, out.stderr_string()),
            })
        }
    }
}

// ====================================================================
// 2. BundleSignatureVerifier — re-verifies D16 bundle Ed25519 sigs
// ====================================================================

/// Verifier that re-checks the Ed25519 signature of a single
/// embedder bundle directory.
///
/// **Audit fix § D17-AC4-bis** — pre-audit this shelled out to
/// `efe-bundle verify --all`, but the real CLI subcommand is
/// `efe-bundle verify --bundle <PATH> --pubkeys <PATH>` (D16).
/// The "verify all" semantic is achieved by the caller looping
/// over every bundle directory.
#[derive(Debug)]
pub struct BundleSignatureVerifier {
    runner: Arc<dyn CommandRunner>,
    /// Path to the `efe-bundle` binary.
    pub bundle_binary: String,
    /// Path to the bundle directory to verify.
    pub bundle_path: String,
    /// Path to the trust store JSON.
    pub pubkeys_path: String,
}

impl BundleSignatureVerifier {
    /// Construct with explicit paths.
    pub fn new(
        runner: Arc<dyn CommandRunner>,
        bundle_binary: impl Into<String>,
        bundle_path: impl Into<String>,
        pubkeys_path: impl Into<String>,
    ) -> Self {
        Self {
            runner,
            bundle_binary: bundle_binary.into(),
            bundle_path: bundle_path.into(),
            pubkeys_path: pubkeys_path.into(),
        }
    }
}

#[async_trait]
impl Verifier for BundleSignatureVerifier {
    fn kind(&self) -> VerifierKind {
        VerifierKind::BundleSignatures
    }

    async fn check(&self, _target_env: &str) -> Result<VerifierStatus, VerifyError> {
        let out = self
            .runner
            .run(
                &self.bundle_binary,
                &[
                    "verify",
                    "--bundle",
                    &self.bundle_path,
                    "--pubkeys",
                    &self.pubkeys_path,
                ],
                &[],
            )
            .await
            .map_err(|e| VerifyError::Infra(format!("efe-bundle verify spawn: {e}")))?;
        if out.success() {
            Ok(VerifierStatus {
                kind: VerifierKind::BundleSignatures,
                ok: true,
                detail: out.stdout_string(),
            })
        } else {
            Ok(VerifierStatus {
                kind: VerifierKind::BundleSignatures,
                ok: false,
                detail: format!("exit {} : {}", out.status, out.stderr_string()),
            })
        }
    }
}

// ====================================================================
// 3. OidcReachabilityVerifier — JWKS endpoint reachability
// ====================================================================

/// Verifier that fetches the JWKS endpoint of every configured
/// OIDC issuer and asserts it returns at least one key.
#[derive(Debug)]
pub struct OidcReachabilityVerifier {
    runner: Arc<dyn CommandRunner>,
    /// List of JWKS URLs to probe.
    pub jwks_urls: Vec<String>,
    /// Curl binary path (default `curl`).
    pub curl_binary: String,
}

impl OidcReachabilityVerifier {
    /// Construct.
    pub fn new(runner: Arc<dyn CommandRunner>, jwks_urls: Vec<String>) -> Self {
        Self {
            runner,
            jwks_urls,
            curl_binary: "curl".into(),
        }
    }
}

#[async_trait]
impl Verifier for OidcReachabilityVerifier {
    fn kind(&self) -> VerifierKind {
        VerifierKind::OidcIssuers
    }

    async fn check(&self, _target_env: &str) -> Result<VerifierStatus, VerifyError> {
        if self.jwks_urls.is_empty() {
            return Ok(VerifierStatus {
                kind: VerifierKind::OidcIssuers,
                ok: true,
                detail: "no OIDC issuers configured (expected for OIDC-disabled clusters)".into(),
            });
        }
        let mut unreachable: Vec<String> = vec![];
        let mut probed = 0u32;
        for url in &self.jwks_urls {
            let out = self
                .runner
                .run(
                    &self.curl_binary,
                    &["-fsS", "--max-time", "5", url.as_str()],
                    &[],
                )
                .await
                .map_err(|e| VerifyError::Infra(format!("curl spawn: {e}")))?;
            probed += 1;
            if !out.success() {
                unreachable.push(url.clone());
                continue;
            }
            // Sanity : response body must contain "keys" (every JWKS spec).
            let body = out.stdout_string();
            if !body.contains("\"keys\"") {
                unreachable.push(format!("{url} (no `keys` field)"));
            }
        }
        if unreachable.is_empty() {
            Ok(VerifierStatus {
                kind: VerifierKind::OidcIssuers,
                ok: true,
                detail: format!("{probed} JWKS endpoints reachable"),
            })
        } else {
            Ok(VerifierStatus {
                kind: VerifierKind::OidcIssuers,
                ok: false,
                detail: format!("unreachable: {unreachable:?}"),
            })
        }
    }
}

// ====================================================================
// 4. CatalogSmokeVerifier — efe-server catalog list smoke
// ====================================================================

/// Verifier that calls `efe-server catalog list --tenant=<canary>`
/// and checks the output contains at least one row.
#[derive(Debug)]
pub struct CatalogSmokeVerifier {
    runner: Arc<dyn CommandRunner>,
    /// Path to the `efe-server` binary.
    pub server_binary: String,
    /// Canary tenant id to query.
    pub canary_tenant: String,
}

impl CatalogSmokeVerifier {
    /// Construct.
    pub fn new(
        runner: Arc<dyn CommandRunner>,
        server_binary: impl Into<String>,
        canary_tenant: impl Into<String>,
    ) -> Self {
        Self {
            runner,
            server_binary: server_binary.into(),
            canary_tenant: canary_tenant.into(),
        }
    }
}

#[async_trait]
impl Verifier for CatalogSmokeVerifier {
    fn kind(&self) -> VerifierKind {
        VerifierKind::CatalogSmoke
    }

    async fn check(&self, _target_env: &str) -> Result<VerifierStatus, VerifyError> {
        let tenant_arg = format!("--tenant={}", self.canary_tenant);
        let out = self
            .runner
            .run(
                &self.server_binary,
                &["catalog", "list", &tenant_arg],
                &[],
            )
            .await
            .map_err(|e| VerifyError::Infra(format!("catalog list spawn: {e}")))?;
        if !out.success() {
            return Ok(VerifierStatus {
                kind: VerifierKind::CatalogSmoke,
                ok: false,
                detail: format!("exit {} : {}", out.status, out.stderr_string()),
            });
        }
        let body = out.stdout_string();
        let row_count = body.lines().filter(|l| !l.trim().is_empty()).count();
        if row_count == 0 {
            return Ok(VerifierStatus {
                kind: VerifierKind::CatalogSmoke,
                ok: false,
                detail: "catalog list returned zero rows for canary tenant".into(),
            });
        }
        Ok(VerifierStatus {
            kind: VerifierKind::CatalogSmoke,
            ok: true,
            detail: format!("{row_count} catalog rows visible"),
        })
    }
}

// ====================================================================
// 5. ExecuteSmokeVerifier — Execute RPC smoke
// ====================================================================

/// Verifier that issues a canary `Execute` RPC via `grpcurl` against
/// the just-restored gRPC endpoint. The canary payload is hardcoded
/// to a minimal valid `ExecuteRequest`.
#[derive(Debug)]
pub struct ExecuteSmokeVerifier {
    runner: Arc<dyn CommandRunner>,
    /// gRPC endpoint, e.g. `efe-server.efe.svc.cluster.local:50051`.
    pub grpc_endpoint: String,
    /// Path to the `grpcurl` binary.
    pub grpcurl_binary: String,
    /// Canary request payload as a JSON string. Must match the
    /// `efe.swarm.v1.Execute` proto request shape.
    pub canary_payload: String,
}

impl ExecuteSmokeVerifier {
    /// Construct with a minimal canary payload.
    pub fn new(runner: Arc<dyn CommandRunner>, grpc_endpoint: impl Into<String>) -> Self {
        Self {
            runner,
            grpc_endpoint: grpc_endpoint.into(),
            grpcurl_binary: "grpcurl".into(),
            canary_payload: r#"{"tenant_id":"canary","catalog_name":"smoke","catalog_version":"v1","input":[]}"#
                .to_string(),
        }
    }
}

#[async_trait]
impl Verifier for ExecuteSmokeVerifier {
    fn kind(&self) -> VerifierKind {
        VerifierKind::ExecuteSmoke
    }

    async fn check(&self, _target_env: &str) -> Result<VerifierStatus, VerifyError> {
        // grpcurl is the universal CLI for gRPC ; we use plaintext (-plaintext)
        // since intra-cluster TLS is the cluster's job.
        let out = self
            .runner
            .run(
                &self.grpcurl_binary,
                &[
                    "-plaintext",
                    "-d",
                    &self.canary_payload,
                    &self.grpc_endpoint,
                    "efe.swarm.v1.SwarmService/Execute",
                ],
                &[],
            )
            .await
            .map_err(|e| VerifyError::Infra(format!("grpcurl spawn: {e}")))?;
        if out.success() {
            Ok(VerifierStatus {
                kind: VerifierKind::ExecuteSmoke,
                ok: true,
                detail: format!("Execute RPC returned ok ({} bytes)", out.stdout.len()),
            })
        } else {
            Ok(VerifierStatus {
                kind: VerifierKind::ExecuteSmoke,
                ok: false,
                detail: format!("exit {} : {}", out.status, out.stderr_string()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_runner::MockCommandRunner;

    #[tokio::test]
    async fn audit_chain_verifier_ok_when_subcommand_exits_zero() {
        let runner = Arc::new(
            MockCommandRunner::new().expect_ok("efe-verify-audit", "ok intact"),
        );
        let v = AuditChainVerifier::new(
            runner,
            "efe-verify-audit",
            "postgres://efe@localhost/efe",
            "/etc/efe/audit-keys.json",
        );
        let s = v.check("drill").await.unwrap();
        assert!(s.ok);
        assert_eq!(s.kind, VerifierKind::AuditChain);
    }

    #[tokio::test]
    async fn audit_chain_verifier_anomaly_when_nonzero_exit() {
        let runner = Arc::new(
            MockCommandRunner::new().expect_err("efe-verify-audit", 5, "row 42 hmac mismatch"),
        );
        let v = AuditChainVerifier::new(
            runner,
            "efe-verify-audit",
            "postgres://efe@localhost/efe",
            "/etc/efe/audit-keys.json",
        );
        let s = v.check("drill").await.unwrap();
        assert!(!s.ok);
        assert!(s.detail.contains("hmac mismatch"));
    }

    #[tokio::test]
    async fn audit_chain_verifier_with_chain_uses_arg() {
        let runner = Arc::new(
            // Match the full prefix : the verifier always passes
            // --pg-url + --keys-file BEFORE the optional --chain.
            MockCommandRunner::new().expect_ok("efe-verify-audit --pg-url", "ok"),
        );
        let v = AuditChainVerifier::new(
            runner.clone(),
            "efe-verify-audit",
            "postgres://efe@localhost/efe",
            "/etc/efe/audit-keys.json",
        )
        .with_chain("tenant:acme");
        let _ = v.check("drill").await.unwrap();
        let invs = runner.invocations();
        assert_eq!(invs.len(), 1);
        let chain_idx = invs[0].args.iter().position(|a| a == "--chain").unwrap();
        assert_eq!(invs[0].args[chain_idx + 1], "tenant:acme");
        // **Audit fix § AC4-tris** — also assert pg-url + keys-file
        // are passed.
        let pg_idx = invs[0].args.iter().position(|a| a == "--pg-url").unwrap();
        assert_eq!(invs[0].args[pg_idx + 1], "postgres://efe@localhost/efe");
        let keys_idx = invs[0]
            .args
            .iter()
            .position(|a| a == "--keys-file")
            .unwrap();
        assert_eq!(invs[0].args[keys_idx + 1], "/etc/efe/audit-keys.json");
    }

    #[tokio::test]
    async fn bundle_signature_verifier_ok_path() {
        let runner = Arc::new(
            MockCommandRunner::new().expect_ok("efe-bundle verify", "verified"),
        );
        let v = BundleSignatureVerifier::new(
            runner,
            "efe-bundle",
            "/etc/efe/bundles/sentence-transformers",
            "/etc/efe/pubkeys.json",
        );
        let s = v.check("drill").await.unwrap();
        assert!(s.ok);
    }

    #[tokio::test]
    async fn bundle_signature_verifier_passes_bundle_and_pubkeys_args() {
        let runner = Arc::new(
            MockCommandRunner::new().expect_ok("efe-bundle verify", ""),
        );
        let v = BundleSignatureVerifier::new(
            runner.clone(),
            "efe-bundle",
            "/path/to/bundle",
            "/path/to/keys.json",
        );
        let _ = v.check("drill").await.unwrap();
        let invs = runner.invocations();
        assert_eq!(invs.len(), 1);
        let bundle_idx = invs[0].args.iter().position(|a| a == "--bundle").unwrap();
        assert_eq!(invs[0].args[bundle_idx + 1], "/path/to/bundle");
        let pk_idx = invs[0].args.iter().position(|a| a == "--pubkeys").unwrap();
        assert_eq!(invs[0].args[pk_idx + 1], "/path/to/keys.json");
    }

    #[tokio::test]
    async fn oidc_reachability_verifier_no_issuers_returns_ok() {
        let runner = Arc::new(MockCommandRunner::new());
        let v = OidcReachabilityVerifier::new(runner, vec![]);
        let s = v.check("drill").await.unwrap();
        assert!(s.ok);
        assert!(s.detail.contains("no OIDC issuers"));
    }

    #[tokio::test]
    async fn oidc_reachability_verifier_unreachable_marks_anomaly() {
        let runner = Arc::new(
            MockCommandRunner::new().expect_err("curl", 7, "couldnt resolve"),
        );
        let v = OidcReachabilityVerifier::new(
            runner,
            vec!["https://idp/.well-known/jwks.json".into()],
        );
        let s = v.check("drill").await.unwrap();
        assert!(!s.ok);
    }

    #[tokio::test]
    async fn oidc_reachability_verifier_missing_keys_field_marks_anomaly() {
        let runner = Arc::new(
            MockCommandRunner::new().expect_ok("curl", "{\"unrelated\":\"json\"}"),
        );
        let v = OidcReachabilityVerifier::new(
            runner,
            vec!["https://idp/.well-known/jwks.json".into()],
        );
        let s = v.check("drill").await.unwrap();
        assert!(!s.ok);
    }

    #[tokio::test]
    async fn catalog_smoke_verifier_zero_rows_anomaly() {
        let runner = Arc::new(
            MockCommandRunner::new().expect_ok("efe-server catalog list", ""),
        );
        let v = CatalogSmokeVerifier::new(runner, "efe-server", "canary-tenant");
        let s = v.check("drill").await.unwrap();
        assert!(!s.ok);
    }

    #[tokio::test]
    async fn catalog_smoke_verifier_some_rows_ok() {
        let runner = Arc::new(MockCommandRunner::new().expect_ok(
            "efe-server catalog list",
            "row1\nrow2\nrow3\n",
        ));
        let v = CatalogSmokeVerifier::new(runner, "efe-server", "canary");
        let s = v.check("drill").await.unwrap();
        assert!(s.ok);
        assert!(s.detail.contains("3 catalog rows"));
    }

    #[tokio::test]
    async fn execute_smoke_verifier_happy_path() {
        let runner = Arc::new(
            MockCommandRunner::new().expect_ok("grpcurl", "{\"output\":[]}"),
        );
        let v = ExecuteSmokeVerifier::new(runner, "efe.svc:50051");
        let s = v.check("drill").await.unwrap();
        assert!(s.ok);
    }

    #[tokio::test]
    async fn execute_smoke_verifier_anomaly_on_grpc_error() {
        let runner = Arc::new(
            MockCommandRunner::new().expect_err("grpcurl", 14, "Unavailable"),
        );
        let v = ExecuteSmokeVerifier::new(runner, "efe.svc:50051");
        let s = v.check("drill").await.unwrap();
        assert!(!s.ok);
    }
}
