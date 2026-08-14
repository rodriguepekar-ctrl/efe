//! Multi-process integration harness.
//!
//! Owns the orchestration of controller + participant binaries
//! spawned as OS subprocesses.  Each test holds a [`ProcessHarness`]
//! for the duration of the test ; Drop SIGKILLs every child + cleans
//! the tempfile manifest.
//!
//! Unix-only — Phase 2 deployment target is Linux server (PR #10
//! § AD11).  Gated via `#[cfg(unix)]` in
//! [`tests/common/mod.rs`](super) ; on non-Unix platforms this module
//! is not compiled.
//!
//! ## Lifecycle invariants
//!
//!   1. No zombie processes after a test run : Drop calls `wait()`
//!      on every Child.
//!   2. No port collisions : every test allocates fresh ports via
//!      `allocate_port()` (bind-drop-rebind).
//!   3. No tempfile leaks : `NamedTempFile` drops with the harness.
//!   4. Process tree is flat : the harness spawns each binary
//!      directly ; no fork-exec chains.

#![allow(dead_code)] // each integration-test binary picks what it needs.

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use efe_meta_swarm_proto::controller::MetaSwarmServiceClient;
use efe_meta_swarm_proto::meta_swarm::HealthRequest;
use efe_meta_swarm_proto::participant::MetaParticipantServiceClient;
use efe_meta_swarm_server::manifest::ParticipantManifest;
use tempfile::NamedTempFile;
use tonic::transport::Endpoint;
use tonic::Request;

use super::allocate_port;

// ---------------------------------------------------------------------------
//  Public constants (single source of truth across PR #10 tests)
// ---------------------------------------------------------------------------

/// Maximum time we give the controller's graceful drain on SIGTERM.
/// `bin/controller.rs` uses a 30 s drain ; we add 5 s of slack so a
/// loaded CI runner doesn't false-positive.
pub const DRAIN_TIMEOUT_PLUS_SLACK: Duration = Duration::from_secs(35);

// ---------------------------------------------------------------------------
//  Public client helpers — promoted from per-test-file duplication so
//  every multiprocess test uses the same timeouts.
// ---------------------------------------------------------------------------

/// Build a controller gRPC client.  Timeouts match the architecture
/// spec § 14.1's wall-time budget (≤ 20 s per call ; 5 s connect).
pub async fn connect_controller(
    endpoint: String,
) -> MetaSwarmServiceClient<tonic::transport::Channel> {
    let channel = Endpoint::from_shared(endpoint)
        .expect("controller endpoint URI")
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
        .expect("controller connect");
    MetaSwarmServiceClient::new(channel)
}

/// Build a participant gRPC client.  Same timeout policy as
/// [`connect_controller`].
pub async fn connect_participant(
    endpoint: String,
) -> MetaParticipantServiceClient<tonic::transport::Channel> {
    let channel = Endpoint::from_shared(endpoint)
        .expect("participant endpoint URI")
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
        .expect("participant connect");
    MetaParticipantServiceClient::new(channel)
}

/// One participant subprocess.
pub struct ParticipantChild {
    pub id: String,
    pub grpc_addr: SocketAddr,
    pub metrics_addr: SocketAddr,
    child: Child,
}

impl ParticipantChild {
    /// PID of the spawned binary.  Used for `libc::kill` in
    /// graceful-shutdown tests.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

/// The controller subprocess.
pub struct ControllerChild {
    pub grpc_addr: SocketAddr,
    pub metrics_addr: SocketAddr,
    child: Child,
}

impl ControllerChild {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

/// Full process-tree handle.  Drop SIGKILLs everything.
pub struct ProcessHarness {
    pub controller: ControllerChild,
    pub participants: Vec<ParticipantChild>,
    _manifest: NamedTempFile,
}

// ---------------------------------------------------------------------------
//  Manifest rendering
// ---------------------------------------------------------------------------

/// Render a Phase-2 manifest YAML from a list of participant rows.
///
/// Schema matches `manifest::load_and_validate` 1-to-1 ; if the
/// schema changes, C1's smoke test catches it via a deserialize
/// failure inside the spawned binary.
///
/// `meta.dim = 16` and `safety.radius = 50.0` are constants used
/// across all PR #8/9 / PR #10 tests — generous to avoid spurious
/// safety violations.
fn render_manifest_yaml(rows: &[ParticipantManifest]) -> String {
    render_manifest_yaml_with_iteration(rows, 0.5, 0.2)
}

/// PR #12 — same as [`render_manifest_yaml`] but lets the caller pick
/// α / γ so reload integration tests can mutate them between two
/// `rewrite_manifest` calls.
fn render_manifest_yaml_with_iteration(
    rows: &[ParticipantManifest],
    alpha: f64,
    gamma: f64,
) -> String {
    let mut buf = String::with_capacity(512 + rows.len() * 96);
    buf.push_str("version: v0.2.0\n");
    buf.push_str("meta:\n");
    buf.push_str("  dim: 16\n");
    buf.push_str("  safety:\n");
    buf.push_str("    kind: box\n");
    buf.push_str("    radius: 50.0\n");
    buf.push_str("participants:\n");
    for r in rows {
        // Test ids are plain ASCII (no YAML-special chars).
        buf.push_str(&format!("  - id: {}\n", r.id));
        buf.push_str(&format!("    dim: {}\n", r.dim));
        // Mock harness rows always carry kappa/seed (Couche 3 M1.d made
        // them Options in the schema ; the rendered YAML keeps the old shape).
        buf.push_str(&format!("    kappa: {}\n", r.kappa.expect("harness rows are mock-shaped")));
        buf.push_str(&format!("    seed: {}\n", r.seed.expect("harness rows are mock-shaped")));
        buf.push_str("    blas_backend: \"none\"\n");
    }
    buf.push_str("iteration:\n");
    buf.push_str("  max_iters: 100\n");
    buf.push_str("  tolerance: 0.0001\n");
    buf.push_str(&format!("  alpha_default: {alpha}\n"));
    buf.push_str(&format!("  coupling_gamma: {gamma}\n"));
    buf.push_str("  deterministic: true\n");
    buf.push_str("  max_wall_ms: 15000\n");
    buf
}

fn write_manifest_tempfile(rows: &[ParticipantManifest]) -> (NamedTempFile, PathBuf) {
    let yaml = render_manifest_yaml(rows);
    let mut f = NamedTempFile::new().expect("tempfile");
    f.write_all(yaml.as_bytes()).expect("write manifest");
    let path = f.path().to_path_buf();
    (f, path)
}

// ---------------------------------------------------------------------------
//  Spawn helpers
// ---------------------------------------------------------------------------

fn kovant_rows() -> Vec<ParticipantManifest> {
    vec![
        ParticipantManifest {
            id: "supplier-scoring".into(),
            dim: 8,
            kappa: Some(0.6),
            seed: Some(11),
            blas_backend: "none".into(),
            controller: None,
        },
        ParticipantManifest {
            id: "contract-risk".into(),
            dim: 8,
            kappa: Some(0.7),
            seed: Some(22),
            blas_backend: "none".into(),
            controller: None,
        },
        ParticipantManifest {
            id: "spend-categorization".into(),
            dim: 8,
            kappa: Some(0.5),
            seed: Some(33),
            blas_backend: "none".into(),
            controller: None,
        },
    ]
}

fn spawn_participant_process(
    id: &str,
    grpc_addr: SocketAddr,
    metrics_addr: SocketAddr,
    manifest_path: &std::path::Path,
) -> Child {
    let bin = env!("CARGO_BIN_EXE_participant");
    Command::new(bin)
        .env("EFE_META_SWARM_PARTICIPANT_ID", id)
        .env("EFE_META_SWARM_PARTICIPANT_ADDR", format!("{grpc_addr}"))
        .env(
            "EFE_META_SWARM_PARTICIPANT_METRICS",
            format!("{metrics_addr}"),
        )
        .env("EFE_META_SWARM_MANIFEST", manifest_path)
        .env("EFE_META_SWARM_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn participant {id}: {e}"))
}

fn spawn_controller_process(
    grpc_addr: SocketAddr,
    metrics_addr: SocketAddr,
    manifest_path: &std::path::Path,
    participants_env: &str,
) -> Child {
    let bin = env!("CARGO_BIN_EXE_controller");
    Command::new(bin)
        .env("EFE_META_SWARM_CONTROLLER_ADDR", format!("{grpc_addr}"))
        .env(
            "EFE_META_SWARM_CONTROLLER_METRICS",
            format!("{metrics_addr}"),
        )
        .env("EFE_META_SWARM_MANIFEST", manifest_path)
        .env("EFE_META_SWARM_PARTICIPANTS", participants_env)
        .env("EFE_META_SWARM_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn controller: {e}"))
}

impl ProcessHarness {
    /// Default procurement topology : 3 participants on `dim = 8` with
    /// (κ, seed) = (0.6, 11), (0.7, 22), (0.5, 33).  Used by every
    /// happy-path test.
    pub async fn spawn_kovant() -> Self {
        Self::spawn_with_rows(kovant_rows()).await
    }

    /// Spawn with arbitrary participant rows ; allocates a fresh
    /// port for each.  The manifest declares exactly these rows.
    pub async fn spawn_with_rows(rows: Vec<ParticipantManifest>) -> Self {
        // 1. Allocate ports for every participant.
        let mut endpoints = Vec::with_capacity(rows.len());
        for _ in &rows {
            let grpc = allocate_port().await;
            let metrics = allocate_port().await;
            endpoints.push((grpc, metrics));
        }

        // 2. Allocate ports for the controller.
        let controller_grpc = allocate_port().await;
        let controller_metrics = allocate_port().await;

        // 3. Write manifest tempfile.
        let (manifest_file, manifest_path) = write_manifest_tempfile(&rows);

        // 4. Spawn every participant.
        let mut participants = Vec::with_capacity(rows.len());
        for (i, row) in rows.iter().enumerate() {
            let (grpc, metrics) = endpoints[i];
            let child = spawn_participant_process(&row.id, grpc, metrics, &manifest_path);
            participants.push(ParticipantChild {
                id: row.id.clone(),
                grpc_addr: grpc,
                metrics_addr: metrics,
                child,
            });
        }

        // 5. Build EFE_META_SWARM_PARTICIPANTS env from alive entries.
        let participants_env = participants
            .iter()
            .map(|p| format!("{}={}", p.id, p.grpc_addr))
            .collect::<Vec<_>>()
            .join(",");

        // 6. Spawn the controller.
        let controller_child = spawn_controller_process(
            controller_grpc,
            controller_metrics,
            &manifest_path,
            &participants_env,
        );

        Self {
            controller: ControllerChild {
                grpc_addr: controller_grpc,
                metrics_addr: controller_metrics,
                child: controller_child,
            },
            participants,
            _manifest: manifest_file,
        }
    }

    /// procurement manifest declares 3 participants ; one is **not** spawned.
    /// The controller's `EFE_META_SWARM_PARTICIPANTS` env var still
    /// includes the absent id, but pointing at a closed port —
    /// `RemoteParticipant::connect` will fail with `ECONNREFUSED`.
    ///
    /// Used by `iterate_fails_unavailable_when_participant_unreachable`.
    ///
    /// # Panics
    ///
    /// Panics if `absent_id` is not declared by [`kovant_rows`].
    /// Defence in depth : a typo in the caller would otherwise spawn
    /// all 3 participants and silently turn the failure-mode test
    /// into a happy-path test.
    pub async fn spawn_kovant_with_missing(absent_id: &str) -> Self {
        let rows = kovant_rows();
        assert!(
            rows.iter().any(|r| r.id == absent_id),
            "spawn_kovant_with_missing : absent_id {absent_id:?} \
             is not in kovant rows ({:?}) — the test would silently \
             spawn all 3 participants and never exercise the failure \
             scenario",
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        );

        // Allocate real ports for spawned participants.
        let mut endpoints = Vec::with_capacity(rows.len());
        for row in &rows {
            if row.id == absent_id {
                // Bogus address : allocate a port then drop it.  The
                // port is now closed ; subsequent connect attempts
                // fail with ECONNREFUSED.
                let bogus = allocate_port().await;
                endpoints.push((bogus, allocate_port().await));
            } else {
                endpoints.push((allocate_port().await, allocate_port().await));
            }
        }

        let controller_grpc = allocate_port().await;
        let controller_metrics = allocate_port().await;
        let (manifest_file, manifest_path) = write_manifest_tempfile(&rows);

        let mut participants = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            if row.id == absent_id {
                continue; // skip the absent one
            }
            let (grpc, metrics) = endpoints[i];
            let child = spawn_participant_process(&row.id, grpc, metrics, &manifest_path);
            participants.push(ParticipantChild {
                id: row.id.clone(),
                grpc_addr: grpc,
                metrics_addr: metrics,
                child,
            });
        }

        // The controller's env points at ALL 3 endpoints (including
        // the absent one's bogus port).
        let participants_env = rows
            .iter()
            .enumerate()
            .map(|(i, r)| format!("{}={}", r.id, endpoints[i].0))
            .collect::<Vec<_>>()
            .join(",");

        let controller_child = spawn_controller_process(
            controller_grpc,
            controller_metrics,
            &manifest_path,
            &participants_env,
        );

        Self {
            controller: ControllerChild {
                grpc_addr: controller_grpc,
                metrics_addr: controller_metrics,
                child: controller_child,
            },
            participants,
            _manifest: manifest_file,
        }
    }

    /// Manifest declares every row in `rows` ; only the participants
    /// whose id is in `alive_ids` are spawned **and** included in
    /// `EFE_META_SWARM_PARTICIPANTS`.  The controller's lookup
    /// for the absent id fails with `ServerError::Discovery`
    /// ("no static_participants entry").
    ///
    /// Used by `controller_with_manifest_mismatch_fails_at_iterate`.
    ///
    /// # Panics
    ///
    /// Panics if any entry in `alive_ids` is not declared in `rows`
    /// or if `alive_ids` is empty (the controller boot needs at
    /// least one alive participant to populate
    /// `EFE_META_SWARM_PARTICIPANTS`).
    pub async fn spawn_partial_kovant(rows: Vec<ParticipantManifest>, alive_ids: &[&str]) -> Self {
        assert!(
            !alive_ids.is_empty(),
            "spawn_partial_kovant : alive_ids must contain at least \
             one id (the controller's EFE_META_SWARM_PARTICIPANTS \
             env var cannot be empty)",
        );
        for id in alive_ids {
            assert!(
                rows.iter().any(|r| r.id == *id),
                "spawn_partial_kovant : alive_id {id:?} not declared \
                 in `rows` ({:?}) — the manifest must declare every \
                 id we want to spawn",
                rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            );
        }
        let mut endpoints = Vec::with_capacity(rows.len());
        for _ in &rows {
            endpoints.push((allocate_port().await, allocate_port().await));
        }

        let controller_grpc = allocate_port().await;
        let controller_metrics = allocate_port().await;
        let (manifest_file, manifest_path) = write_manifest_tempfile(&rows);

        let mut participants = Vec::new();
        let mut env_entries = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            if !alive_ids.contains(&row.id.as_str()) {
                continue;
            }
            let (grpc, metrics) = endpoints[i];
            let child = spawn_participant_process(&row.id, grpc, metrics, &manifest_path);
            participants.push(ParticipantChild {
                id: row.id.clone(),
                grpc_addr: grpc,
                metrics_addr: metrics,
                child,
            });
            env_entries.push(format!("{}={}", row.id, grpc));
        }

        let participants_env = env_entries.join(",");
        let controller_child = spawn_controller_process(
            controller_grpc,
            controller_metrics,
            &manifest_path,
            &participants_env,
        );

        Self {
            controller: ControllerChild {
                grpc_addr: controller_grpc,
                metrics_addr: controller_metrics,
                child: controller_child,
            },
            participants,
            _manifest: manifest_file,
        }
    }

    /// gRPC endpoint string `http://127.0.0.1:PORT` for the controller.
    pub fn controller_endpoint(&self) -> String {
        format!("http://{}", self.controller.grpc_addr)
    }

    /// gRPC endpoint for a participant by id ; `None` if the
    /// participant was not spawned by the harness.
    pub fn participant_endpoint(&self, id: &str) -> Option<String> {
        self.participants
            .iter()
            .find(|p| p.id == id)
            .map(|p| format!("http://{}", p.grpc_addr))
    }

    /// Poll Health on every spawned binary (controller + every alive
    /// participant) until all return SERVING, or fail after `timeout`.
    ///
    /// Pings run **concurrently** per cycle (architecture spec § 7.4)
    /// so a single slow binary doesn't multiply the cycle's wall time
    /// by N.  Cycle period : 250 ms.
    pub async fn wait_for_ready(&self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            let controller_fut = ping_controller(self.controller.grpc_addr);
            let participant_futs = self
                .participants
                .iter()
                .map(|p| ping_participant(p.grpc_addr));
            let (controller_ok, participant_oks) =
                tokio::join!(controller_fut, futures::future::join_all(participant_futs),);
            if controller_ok && participant_oks.iter().all(|&ok| ok) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "not ready within {timeout:?} : \
                     controller={controller_ok} participants={participant_oks:?}",
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Send SIGTERM to a participant by id.  Returns whether the
    /// participant exists in the harness.  Use [`wait_participant`]
    /// to reap it.
    pub fn sigterm_participant(&self, id: &str) -> bool {
        if let Some(p) = self.participants.iter().find(|p| p.id == id) {
            send_sigterm(p.pid());
            true
        } else {
            false
        }
    }

    /// Send SIGTERM to the controller.  Use [`wait_controller`] to reap.
    pub fn sigterm_controller(&self) {
        send_sigterm(self.controller.pid());
    }

    /// PR #12 — send SIGHUP to the controller, triggering a manifest
    /// reload.  Unlike SIGTERM, the controller keeps running ; the
    /// caller observes the reload via the gRPC `Health.reload_gen`
    /// counter.
    pub fn sighup_controller(&self) {
        send_sighup(self.controller.pid());
    }

    /// PR #12 — send SIGHUP to a participant by id, triggering its
    /// manifest reload.  Returns whether the participant exists in
    /// the harness.
    pub fn sighup_participant(&self, id: &str) -> bool {
        if let Some(p) = self.participants.iter().find(|p| p.id == id) {
            send_sighup(p.pid());
            true
        } else {
            false
        }
    }

    /// PR #12 — overwrite the manifest tempfile with a freshly-rendered
    /// YAML.  Used by reload integration tests to mutate iteration
    /// knobs / add or remove participants between two
    /// `sighup_controller()` calls.  The path stays the same so
    /// `EFE_META_SWARM_MANIFEST` (already pointing at this file) is
    /// re-read by the SIGHUP handler.
    ///
    /// `participants` MAY differ from the rows initially spawned by
    /// the harness (e.g. for add-participant reload tests where the
    /// new manifest declares a 4th participant) — the test is
    /// responsible for keeping `EFE_META_SWARM_PARTICIPANTS` in sync
    /// on its own if it relies on the controller's static-discovery
    /// lookup, which is **not** auto-refreshed by SIGHUP.
    pub fn rewrite_manifest(
        &mut self,
        participants: Vec<ParticipantManifest>,
        iteration_alpha: f64,
        iteration_gamma: f64,
    ) -> std::io::Result<()> {
        use std::io::Seek as _;
        let yaml =
            render_manifest_yaml_with_iteration(&participants, iteration_alpha, iteration_gamma);
        let f = self._manifest.as_file_mut();
        f.set_len(0)?;
        f.rewind()?;
        f.write_all(yaml.as_bytes())?;
        f.flush()?;
        Ok(())
    }

    /// PR #12 — controller endpoint as a `SocketAddr` (for tests that
    /// already have a `tonic::transport::Endpoint` builder around).
    pub fn controller_addr(&self) -> SocketAddr {
        self.controller.grpc_addr
    }

    /// SIGKILL a participant immediately (no drain).  Returns whether
    /// the participant exists in the harness.  Used by failure-mode
    /// tests to simulate a hard crash.
    pub fn sigkill_participant(&mut self, id: &str) -> bool {
        if let Some(p) = self.participants.iter_mut().find(|p| p.id == id) {
            let _ = p.child.kill();
            true
        } else {
            false
        }
    }

    /// Wait for the controller to exit, capped at `timeout`.  Returns
    /// the exit status on success.
    pub async fn wait_controller(&mut self, timeout: Duration) -> Result<ExitStatus, String> {
        wait_child(&mut self.controller.child, timeout).await
    }

    /// Wait for a participant to exit, capped at `timeout`.  Returns
    /// `Err` if no participant matches the id or if the wait times
    /// out.
    pub async fn wait_participant(
        &mut self,
        id: &str,
        timeout: Duration,
    ) -> Result<ExitStatus, String> {
        let p = self
            .participants
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| format!("no participant with id {id:?}"))?;
        wait_child(&mut p.child, timeout).await
    }
}

impl Drop for ProcessHarness {
    fn drop(&mut self) {
        // Last-resort SIGKILL.  Tests that wanted a clean shutdown
        // should have called `sigterm_*` + `wait_*` already.
        for p in &mut self.participants {
            let _ = p.child.kill();
            let _ = p.child.wait();
        }
        let _ = self.controller.child.kill();
        let _ = self.controller.child.wait();
        // `_manifest` drops, cleaning the tempfile.
    }
}

// ---------------------------------------------------------------------------
//  Internal helpers
// ---------------------------------------------------------------------------

fn send_sigterm(pid: u32) {
    // SAFETY : libc::kill is safe to call with any pid.  Failure
    // (e.g. ESRCH if the process is already gone) sets errno and
    // returns -1 ; we ignore both.
    //
    // PID can theoretically wrap on Linux ; the worst case is sending
    // SIGTERM to an unrelated process that happens to have recycled
    // the same PID.  The harness holds the `Child` until `Drop`, so
    // the PID stays reserved for our subprocess for the test's
    // lifetime.
    #[allow(clippy::cast_possible_wrap)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
}

/// PR #12 — send SIGHUP for live reload.  Same `libc::kill` SAFETY
/// argument as [`send_sigterm`] : the harness owns the `Child` so
/// the pid is reserved for the test's lifetime.
fn send_sighup(pid: u32) {
    #[allow(clippy::cast_possible_wrap)]
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGHUP);
    }
}

async fn wait_child(child: &mut Child, timeout: Duration) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "child {} did not exit within {timeout:?}",
                        child.id(),
                    ));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => return Err(format!("try_wait failed : {e}")),
        }
    }
}

async fn ping_controller(addr: SocketAddr) -> bool {
    let endpoint_str = format!("http://{addr}");
    let Ok(ep) = Endpoint::from_shared(endpoint_str) else {
        return false;
    };
    let Ok(ch) = ep
        .timeout(Duration::from_millis(200))
        .connect_timeout(Duration::from_millis(200))
        .connect()
        .await
    else {
        return false;
    };
    let mut client = MetaSwarmServiceClient::new(ch);
    client
        .health(Request::new(HealthRequest {}))
        .await
        .map(|r| r.into_inner().status == 1)
        .unwrap_or(false)
}

async fn ping_participant(addr: SocketAddr) -> bool {
    let endpoint_str = format!("http://{addr}");
    let Ok(ep) = Endpoint::from_shared(endpoint_str) else {
        return false;
    };
    let Ok(ch) = ep
        .timeout(Duration::from_millis(200))
        .connect_timeout(Duration::from_millis(200))
        .connect()
        .await
    else {
        return false;
    };
    let mut client = MetaParticipantServiceClient::new(ch);
    client
        .health(Request::new(HealthRequest {}))
        .await
        .map(|r| r.into_inner().status == 1)
        .unwrap_or(false)
}

// Sanity tests for the harness itself live in
// `tests/integration_multiprocess_harness.rs` so they run **once**
// rather than being recompiled into every integration-test binary
// that `mod common`-s this module.
