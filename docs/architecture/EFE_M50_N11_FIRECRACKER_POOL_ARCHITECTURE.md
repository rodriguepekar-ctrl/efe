# EFE M50 — Sprint 8 — N11 Firecracker Pool Architecture

**Status**            : DRAFT v1
**Date drafted**      : 2026-05-19
**Owner**             : Rodrigue Pekar (founding engineer) + Platform Engineer #2 (TBH)
**Sprint**            : Sprint 8 (Phase 3 of M50)
**Predecessor**       : Sprint 7 (N1-N10 closed)
**Companion docs**    :
- [`EFE_M50_AGENTIC_ARCHITECTURE.md`](EFE_M50_AGENTIC_ARCHITECTURE.md) (M50 master)
- [`EFE_M50_PHASE_2_ARCHITECTURE.md`](EFE_M50_PHASE_2_ARCHITECTURE.md) (Sprint 4-6 origin of `StubVmPool`)
- [`EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md`](../EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md) (M25 — the Firecracker substrate to reuse)
- [`../runbooks/EFE_M50_FIRECRACKER_RUNBOOK.md`](../runbooks/EFE_M50_FIRECRACKER_RUNBOOK.md) (operator playbook target — TBH at Sprint 8 close)

**Contrat additif** : This work touches **only** `crates/efe-nano/src/runtime/pool/` and adds a new submodule `firecracker` alongside the existing `stub`.  No existing crate signatures change.  The Sprint 4-Sprint 7 [`VmPool`] trait is **frozen** — `FirecrackerVmPool` implements the same interface so `MultiModeRuntime`, `LocalModelRuntime`, and `LlmAugmentedRuntime` are wired by Cargo features alone.  The 791 Sprint-7 tests stay green ; CI gate enforced.

---

## Table of contents

0. [Executive summary](#0-executive-summary)
1. [Problem statement & current state](#1-problem-statement--current-state)
2. [Goals & non-goals](#2-goals--non-goals)
3. [Architecture overview](#3-architecture-overview)
4. [Component breakdown — 7 components](#4-component-breakdown--7-components)
5. [Data structures](#5-data-structures)
6. [Host ↔ guest RPC protocol](#6-host--guest-rpc-protocol)
7. [State machine of a VM lease](#7-state-machine-of-a-vm-lease)
8. [Concurrency & locking model](#8-concurrency--locking-model)
9. [Failure modes & recovery — 12 distinct paths](#9-failure-modes--recovery--12-distinct-paths)
10. [Security analysis & threat model](#10-security-analysis--threat-model)
11. [Performance budget](#11-performance-budget)
12. [Resource sizing & cost model](#12-resource-sizing--cost-model)
13. [Tenant isolation guarantees](#13-tenant-isolation-guarantees)
14. [Integration with M25 (efe-agent-runner)](#14-integration-with-m25-efe-agent-runner)
15. [Migration plan : StubVmPool → FirecrackerVmPool](#15-migration-plan--stubvmpool--firecrackervmpool)
16. [Testing strategy — 5 tiers](#16-testing-strategy--5-tiers)
17. [Operational runbook](#17-operational-runbook)
18. [Deployment topology](#18-deployment-topology)
19. [Sprint 8 day-by-day plan (20 working days)](#19-sprint-8-day-by-day-plan-20-working-days)
20. [Open questions / risks register](#20-open-questions--risks-register)
21. [Decisions log (5 ADRs)](#21-decisions-log-5-adrs)
22. [Annexe A — Guest InferenceDaemon protocol spec](#annexe-a--guest-inferencedaemon-protocol-spec)
23. [Annexe B — Rootfs build pipeline spec](#annexe-b--rootfs-build-pipeline-spec)
24. [Annexe C — Helm chart values](#annexe-c--helm-chart-values)

---

## 0. Executive summary

The Sprint 4 [`StubVmPool`] runs Mode B / Mode C inference **in-process** with the dispatcher.  This is a deliberate Sprint-4 stub : it ships the trait surface ([`VmPool`], [`VmLease`], [`VmRequest`], [`VmResponse`]) and proves the pool semantics (acquire / release / health / hit-rate metric) without the operational complexity of a real microVM.  **It does not provide isolation.**  A bug in a Mode B model — say a regex DFA explosion in `ner_clinical` — can OOM the entire EFE Nano runtime, taking down every tenant of that node.

Sprint 8 ships [`FirecrackerVmPool`] which runs each Mode B / Mode C inference inside a **dedicated Firecracker microVM**, pre-warmed and pooled per `(VmKind, model_id, version, tenant_class)` tuple.  Each VM has :

- A statically-linked Rust inference daemon listening on **vsock port 5000**.
- A pre-loaded ML model in its 256-512 MB of RAM.
- A hard cgroup ceiling on CPU, memory, IO bandwidth.
- A dedicated KVM context — **no shared kernel state** with the host or with other VMs.

The trait surface is **byte-identical** to Sprint 4's : the dispatcher, executors, audit chain, replay engine, and Merkle sealer don't change.  Wire-up is a single Cargo feature flag : `--features firecracker`.

**Sprint 8 commitment** (4 weeks, founder + 1 senior platform eng) :

| Deliverable | Verification |
|-------------|--------------|
| `FirecrackerVmPool` impl of `VmPool` | trait-conformance tests (≥ 30) GREEN |
| 5 model rootfs images built | OCI artifacts pushed to private registry |
| Cold start ≤ 500 ms p99 | `bench_firecracker_cold_start.rs` GREEN |
| Pool hit rate ≥ 95 % at 100 RPS Mode B | `bench_firecracker_hit_rate.rs` GREEN |
| Reaper kills dead VMs ≤ 10 s p99 | `tests/firecracker_reaper_smoke.rs` GREEN |
| Per-tenant cgroup isolation | `tests/firecracker_cgroup_isolation.rs` GREEN |
| Vsock latency p99 ≤ 5 ms | `bench_vsock_round_trip.rs` GREEN |
| Helm chart `efe-nano-firecracker` | `helm template` byte-identical to baseline when feature off |
| Migration shim StubVmPool ↔ FirecrackerVmPool | `EFE_NANO_VM_BACKEND=stub|firecracker` env var |
| 7-day soak in staging | zero Sev1/Sev2, hit rate ≥ 95 %, crash rate ≤ 1/1000 |

---

## 1. Problem statement & current state

### 1.1 The StubVmPool reality (Sprint 4 ship)

```rust
// crates/efe-nano/src/runtime/pool/stub.rs (excerpt — current Sprint 7 state)
pub struct StubVmPool {
    inner: Arc<tokio::sync::Semaphore>,
    max_slots: u32,
    hit_count: AtomicU64,
    miss_count: AtomicU64,
}

impl VmPool for StubVmPool {
    async fn acquire(&self, kind: VmKind, wait_budget: Duration) -> Result<VmLease, VmPoolError> {
        let permit = self.inner.clone().acquire_owned()
            .timeout(wait_budget).await??;
        self.hit_count.fetch_add(1, Ordering::Relaxed);
        Ok(VmLease { kind, permit, inference_fn: in_process_inference })
    }
}
```

The `in_process_inference` closure executes **in the same Tokio runtime** as the dispatcher.  Mode B's regex-extract handler, NER classifier, sentiment scorer — they all share :

- The same kernel.
- The same heap.
- The same file descriptors.
- The same signal mask.

### 1.2 The blast radius

| Failure | Today's blast radius | Sprint 8 blast radius |
|---------|---------------------|----------------------|
| Regex DFA explosion (CVE-2022-24713 class) | Whole node OOM, every tenant down | One VM dies, others unaffected |
| Model unsafe-Rust UB (ONNX, candle, ort) | Memory corruption, undefined behavior in host | VM crashes, contained |
| Side-channel between tenants A+B | Possible (shared cache) | Cache flushed on KVM context switch |
| Infinite loop / CPU exhaustion | Tokio scheduler starvation | Cgroup CPU quota kicks in, VM throttled |
| Disk I/O exhaustion | Host I/O storms | Cgroup blkio limits per VM |
| Memory leak | Host OOM after hours | VM RSS capped, restart on threshold |

### 1.3 Compliance gap

**SOC 2 Type I CC6.6** ("logical access controls — separation of duties") and **CC6.7** ("transmission of sensitive information") both require **tenant data isolation by technical control, not procedural**.  Shared-kernel Mode B fails this — auditors will flag.  AI Act Article 9 (high-risk AI systems) requires "demonstrable isolation between subject populations".  Today's StubVmPool can't pass either audit.

**Without N11, M50 cannot ship to a regulated customer.**  Internal pilots (founding investors, friendly betas) — OK.  Anything else — blocked.

### 1.4 Performance baseline (StubVmPool measurements)

Measured on a 16-core M1 Pro, no contention :
- Acquire p50 : 12 µs (semaphore acquire).
- Acquire p99 : 87 µs.
- Inference (Mode B ner_clinical) : 8 ms p50, 23 ms p99.
- Inference (Mode C summarize_strict) : 1.8 s p50, 4.2 s p99 (LLM-dominated).
- Throughput : 1 200 calls/sec/core Mode B ; 22 calls/sec/core Mode C.

Sprint 8 must **preserve** these for hit cases.  Cold misses are new : ≤ 500 ms p99.

---

## 2. Goals & non-goals

### 2.1 Sprint 8 goals (commitments)

| # | Goal | Acceptance |
|---|------|------------|
| G1 | Hard isolation per VM (KVM + cgroups) | `tests/firecracker_cgroup_isolation.rs` (kills VM with `dd if=/dev/urandom of=/tmp/x bs=1G` — must NOT impact host) |
| G2 | Cold start p99 ≤ 500 ms | `bench_firecracker_cold_start.rs` over 100 cold acquires |
| G3 | Pool hit rate ≥ 95 % at 100 RPS Mode B (Mode C : ≥ 90 %) | `bench_firecracker_hit_rate.rs` 60-second soak |
| G4 | Per-tenant pool partitioning | `tests/firecracker_tenant_isolation.rs` |
| G5 | Vsock RPC latency p99 ≤ 5 ms (ping/pong) | `bench_vsock_round_trip.rs` |
| G6 | Reaper detects + replaces dead VMs ≤ 10 s p99 | `tests/firecracker_reaper_smoke.rs` (kills VM, measures replacement) |
| G7 | Helm chart deploys cleanly on EKS Nitro nodes | `helm template` byte-identical when off ; install on a real cluster GREEN |
| G8 | Feature-flag rollout : `EFE_NANO_VM_BACKEND` | Both `stub` and `firecracker` paths tested |
| G9 | 5 model rootfs images : `ner_clinical_v1.0.0`, `ner_legal_v1.0.0`, `intent_classify_v1.0.0`, `lang_detect_v1.0.0`, `embedder_v1.0.0` | Each ≤ 1 GB, OCI-pushed to registry |
| G10 | 7-day staging soak, 0 Sev1/2, hit ≥ 95 % | `docs/EFE_M50_N11_SOAK_REPORT.md` published |

### 2.2 Sprint 8 non-goals (deferred)

| # | Non-goal | When |
|---|----------|------|
| NG1 | Snapshot/restore (Knative-style cold start ~ 100 ms) | Phase 4 — Sprint 11 |
| NG2 | Autoscaling pool size based on demand | Phase 4 — Sprint 12 |
| NG3 | GPU passthrough for Mode B (transformers) | Phase 5 (M51) |
| NG4 | Cross-region VM migration | Phase 5+ |
| NG5 | Wasm-based guest runtime (drop kernel boundary) | M53+ |
| NG6 | Customer-published guest runtimes (BYO-model) | Phase 4 — Sprint 13 |
| NG7 | jailer (Firecracker's seccomp jail) tuning | Phase 4 — Sprint 11 |
| NG8 | UEFI boot vs. legacy boot benchmarking | Out of scope ; pick legacy |
| NG9 | Hardware-attested VM (TPM, SEV-SNP) | Phase 5+ |
| NG10 | Mode B on-the-fly model loading (vs. pre-baked) | Phase 4 |

---

## 3. Architecture overview

### 3.1 The 4-layer diagram

```
┌──────────────────────────────────────────────────────────────────────────┐
│  HOST  —  EFE Nano runtime process                                      │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ Layer 1 :  FirecrackerVmPool (impls VmPool trait)                │    │
│  │   - DashMap<(VmKind, ModelKey, TenantClass), KindPool>            │    │
│  │   - Acquire → KindPool::checkout                                  │    │
│  │   - Release → KindPool::checkin (on Lease::drop)                  │    │
│  └────────────────────┬─────────────────────────────────────────────┘    │
│                       ↓                                                  │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ Layer 2 :  VmLifecycleController                                 │    │
│  │   - spawn(kind, config) → Firecracker child process              │    │
│  │   - reap(pid)  → SIGTERM then SIGKILL                            │    │
│  │   - cgroup_create(tenant_class)                                   │    │
│  └────────────────────┬─────────────────────────────────────────────┘    │
│                       ↓ (Unix-domain socket — Firecracker API)           │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ Layer 3 :  VsockRpcClient                                        │    │
│  │   - connect(cid, port=5000)                                       │    │
│  │   - send(json_request) → recv(json_response)                      │    │
│  │   - timeout, length-prefix, magic-byte                            │    │
│  └────────────────────┬─────────────────────────────────────────────┘    │
└───────────────────────┼──────────────────────────────────────────────────┘
                        │ vsock (AF_VSOCK, CID per VM)
                        ▼
┌──────────────────────────────────────────────────────────────────────────┐
│  GUEST  —  Firecracker microVM (kernel + Alpine rootfs)                 │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │ Layer 4 :  InferenceDaemon  (statically-linked Rust binary)      │    │
│  │   - on boot : load /opt/models/{id}/{version}/                    │    │
│  │   - vsock listen :5000                                            │    │
│  │   - per-request : parse → infer → respond                         │    │
│  │   - health endpoint, exit on SIGTERM                              │    │
│  └──────────────────────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────────────────────┘
```

### 3.2 Lifecycle in one sentence

The dispatcher calls `pool.acquire(VmKind::LocalModel { model_id, model_version }, deadline)` ; the pool finds a warm VM (or spawns a new one) ; it returns a `VmLease` holding the vsock connection ; the executor calls `lease.execute(VmRequest)` which round-trips JSON over vsock ; on `Drop`, the lease returns the VM to the warm pool (or reaps it on error).

---

## 4. Component breakdown — 7 components

### 4.1 `FirecrackerVmPool` — the trait impl

**Responsibility** : own the inventory of warm VMs ; serve `acquire` / `release` ; expose `health()`.

**Internal state** :

```rust
pub struct FirecrackerVmPool {
    config: FirecrackerPoolConfig,
    lifecycle: Arc<VmLifecycleController>,
    /// Per (VmKind, ModelKey, TenantClass) sub-pool.  The TenantClass
    /// (TIER-1, TIER-AUDIT, ...) partitions pools so a noisy TIER-1
    /// tenant can't starve TIER-AUDIT.
    pools: DashMap<PoolKey, Arc<KindPool>>,
    reaper_task: Option<tokio::task::JoinHandle<()>>,
    metrics: Arc<FirecrackerMetrics>,
}

struct KindPool {
    config: KindConfig,
    warm: Mutex<VecDeque<WarmVm>>,        // ready-to-use
    in_use: Mutex<HashMap<VmId, InUseVm>>, // currently leased
    permits: Arc<Semaphore>,               // gates `max_total`
    cond: Notify,                          // signals warm-set growth
}
```

**`PoolKey` rationale** : pools are partitioned to avoid head-of-line blocking :

```rust
#[derive(Hash, Eq, PartialEq, Clone)]
struct PoolKey {
    kind: VmKind,             // LocalModel { id, version } | LlmCaller
    tenant_class: TenantClass, // Tier1 | TierAudit | Experimental
}
```

This **does not** partition per `tenant_id` directly — that would explode the pool count.  Instead, tenants are bucketed by their **TIER** (stability tier of the agents they invoke).  TIER-AUDIT customers get a guaranteed reservation ; TIER-1 customers share a fair pool ; EXPERIMENTAL customers share a best-effort pool.

### 4.2 `VmLifecycleController` — the Firecracker spawner

**Responsibility** : translate "I want a VM of kind X" → a running Firecracker child process with a vsock CID.

```rust
pub struct VmLifecycleController {
    binary_path: PathBuf,            // /opt/firecracker/firecracker
    kernel_image: PathBuf,           // /opt/firecracker/vmlinux
    cgroup_root: PathBuf,            // /sys/fs/cgroup/efe-nano
    next_cid: AtomicU32,             // monotonic CID allocator
    next_vm_id: AtomicU64,           // monotonic VmId allocator
    children: Mutex<HashMap<u32, ChildProcess>>,
    api_socket_dir: PathBuf,         // /var/run/efe-nano/sockets/
}

impl VmLifecycleController {
    pub async fn spawn(
        &self,
        kind: VmKind,
        kind_config: &KindConfig,
        tenant_class: TenantClass,
    ) -> Result<WarmVm, VmPoolError> {
        // 1. Allocate VmId + CID
        let vm_id = VmId(self.next_vm_id.fetch_add(1, Relaxed));
        let cid = self.next_cid.fetch_add(1, Relaxed);

        // 2. Create cgroup
        let cgroup = self.create_cgroup(tenant_class, vm_id, kind_config)?;

        // 3. Render Firecracker JSON config to api socket dir
        let api_socket = self.api_socket_dir.join(format!("{vm_id}.sock"));
        self.write_vm_config(vm_id, cid, kind_config, &api_socket).await?;

        // 4. Fork Firecracker child
        let child = self.fork_firecracker(&api_socket, &cgroup).await?;

        // 5. POST /machine-config, /drives, /vsock to API socket
        self.send_api_config(&api_socket, cid, kind_config).await?;

        // 6. POST /actions { "action_type": "InstanceStart" }
        self.start_instance(&api_socket).await?;

        // 7. Wait for vsock health-ping
        self.wait_for_vsock_ready(cid, kind_config.boot_budget).await?;

        Ok(WarmVm {
            id: vm_id,
            kind,
            cid,
            firecracker_pid: child.id(),
            api_socket,
            cgroup_path: cgroup,
            created_at: Instant::now(),
            last_used_at: Instant::now(),
            state: AtomicCell::new(VmState::Warm),
        })
    }

    pub async fn reap(&self, vm: &WarmVm) -> Result<(), VmPoolError> {
        // SIGTERM, wait 2s, SIGKILL.
        unsafe { libc::kill(vm.firecracker_pid as i32, libc::SIGTERM) };
        // ... wait with timeout ...
        unsafe { libc::kill(vm.firecracker_pid as i32, libc::SIGKILL) };
        // Remove cgroup.
        std::fs::remove_dir(&vm.cgroup_path).ok();
        // Remove api socket.
        std::fs::remove_file(&vm.api_socket).ok();
        Ok(())
    }
}
```

### 4.3 `VsockRpcClient` — host-side RPC

**Responsibility** : open an `AF_VSOCK` connection to a VM, send length-prefixed JSON, receive response.

```rust
pub struct VsockRpcClient {
    cid: u32,
    port: u32,
    timeout: Duration,
    socket: Mutex<Option<tokio_vsock::VsockStream>>,
}

impl VsockRpcClient {
    pub async fn call(
        &self,
        request: VsockRpcRequest,
    ) -> Result<VsockRpcResponse, VsockError> {
        let mut sock_guard = self.socket.lock().await;
        if sock_guard.is_none() {
            *sock_guard = Some(tokio_vsock::VsockStream::connect(self.cid, self.port).await?);
        }
        let sock = sock_guard.as_mut().unwrap();

        // Length-prefixed framing.  Magic byte 0xEF (EFe).
        let body = serde_json::to_vec(&request)?;
        let len = body.len() as u32;
        if len > MAX_RPC_BYTES { return Err(VsockError::TooLarge); }
        let mut frame = Vec::with_capacity(5 + body.len());
        frame.push(0xEF);
        frame.extend_from_slice(&len.to_be_bytes());
        frame.extend_from_slice(&body);
        sock.write_all(&frame).await?;

        // Read response.
        let mut magic = [0u8; 1];
        timeout(self.timeout, sock.read_exact(&mut magic)).await??;
        if magic[0] != 0xEF { return Err(VsockError::ProtocolError); }
        let mut len_buf = [0u8; 4];
        timeout(self.timeout, sock.read_exact(&mut len_buf)).await??;
        let resp_len = u32::from_be_bytes(len_buf);
        if resp_len > MAX_RPC_BYTES { return Err(VsockError::TooLarge); }
        let mut resp_body = vec![0u8; resp_len as usize];
        timeout(self.timeout, sock.read_exact(&mut resp_body)).await??;
        let resp = serde_json::from_slice(&resp_body)?;
        Ok(resp)
    }
}
```

**Why length-prefixed framing** : vsock is stream-oriented (no message boundaries).  Without framing, the host can't tell where one response ends and the next begins.  RFC-style length prefix + magic byte is the standard answer.

**Why one connection per VM** : keeping the vsock open across calls amortizes the connect cost (~ 200 µs).  Reconnect-on-error.

### 4.4 Guest `InferenceDaemon` — the in-VM workhorse

Written in Rust, statically linked with `musl` for size + portability.  See [§ Annexe A](#annexe-a--guest-inferencedaemon-protocol-spec) for the wire protocol.

**Lifecycle** :

```text
1. init     — read /proc/cmdline for `efe.model_id=X efe.model_version=Y`
2. load     — mmap /opt/models/{X}/{Y}/model.onnx into memory
3. listen   — bind AF_VSOCK on (cid_any, port=5000)
4. serve    — one connection at a time, single-threaded inference
5. shutdown — SIGTERM → unbind → exit 0
```

**Why single-threaded inference per VM** : ONNX runtime + most ML models are not internally parallel-safe (or are, but at huge memory cost).  Cleaner contract : 1 VM = 1 inference at a time.  Concurrency comes from pool width.

**Why static-musl** : no glibc dependency in the guest, single binary, ~ 8 MB stripped.  Faster boot.

### 4.5 `RootfsBuilder` — the OCI artifact pipeline

**Responsibility** : produce reproducible ext4 images bundling kernel + InferenceDaemon + model.

**Build pipeline** (GitHub Actions) :

```yaml
# .github/workflows/firecracker-rootfs-build.yml
on:
  push:
    paths:
      - 'crates/efe-nano/firecracker-rootfs/**'
      - 'crates/efe-nano/src/starter_pack/**'

jobs:
  build-rootfs:
    runs-on: [self-hosted, x86_64, kvm]
    strategy:
      matrix:
        model:
          - { id: ner_clinical, version: 1.0.0, size_mb: 256 }
          - { id: ner_legal, version: 1.0.0, size_mb: 256 }
          - { id: intent_classify, version: 1.0.0, size_mb: 128 }
          - { id: lang_detect, version: 1.0.0, size_mb: 64 }
          - { id: embedder, version: 1.0.0, size_mb: 384 }
    steps:
      - run: ./scripts/firecracker/build-rootfs.sh
             --model-id ${{matrix.model.id}}
             --model-version ${{matrix.model.version}}
             --size-mb ${{matrix.model.size_mb}}
      - run: crane push ./out/rootfs-${{matrix.model.id}}.ext4
             registry.efe.internal/firecracker/${{matrix.model.id}}:${{matrix.model.version}}
      - run: ./scripts/firecracker/sign-rootfs.sh
             registry.efe.internal/firecracker/${{matrix.model.id}}:${{matrix.model.version}}
             --cosign-key kms://key/efe-rootfs
```

**The `build-rootfs.sh` recipe** :
1. Start from a 1 GB sparse file, `mkfs.ext4`.
2. Mount it under `./rootfs/`.
3. Extract `alpine-minirootfs-3.20.tar.gz` into it.
4. Copy `target/x86_64-unknown-linux-musl/release/inference-daemon` to `/opt/bin/`.
5. Copy `models/{model_id}/{model_version}/*.onnx` to `/opt/models/{id}/{version}/`.
6. Install `init` script in `/etc/init.d/efe-inference` that runs `/opt/bin/inference-daemon`.
7. Unmount, shrink.
8. Output : `out/rootfs-{model_id}.ext4`.

**Size budget per kind** :
- `ner_clinical_v1` : 256 MB (model 200 MB + base 50 MB + daemon 6 MB).
- `ner_legal_v1` : 256 MB.
- `intent_classify_v1` : 128 MB.
- `lang_detect_v1` : 64 MB.
- `embedder_v1` : 384 MB (sentence-transformers MiniLM L6 quantized).
- **TOTAL fleet** : ~ 1 GB on disk, mostly read-once per VM (rootfs loaded on Firecracker boot).

### 4.6 `Reaper` — the background janitor

**Responsibility** : detect dead VMs (vsock unreachable, pid exited) ; reap them ; provision replacements to maintain `min_warm`.

```rust
pub struct Reaper {
    pool: Weak<FirecrackerVmPool>,
    interval: Duration,
    metrics: Arc<FirecrackerMetrics>,
}

impl Reaper {
    pub fn spawn(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(self.interval);
            loop {
                ticker.tick().await;
                let Some(pool) = self.pool.upgrade() else { break };
                self.sweep_once(&pool).await;
            }
        })
    }

    async fn sweep_once(&self, pool: &FirecrackerVmPool) {
        for entry in pool.pools.iter() {
            let kind_pool = entry.value();
            // 1. Health-ping every warm VM.
            let dead_ids = self.find_dead_vms(kind_pool).await;
            for dead_id in dead_ids {
                self.reap_vm(pool, kind_pool, dead_id).await;
                self.metrics.crashes_total.inc();
            }
            // 2. Provision replacements to reach min_warm.
            let warm_count = kind_pool.warm.lock().await.len();
            let deficit = (kind_pool.config.min_warm as usize)
                .saturating_sub(warm_count);
            for _ in 0..deficit {
                if let Err(e) = self.provision_one(pool, kind_pool).await {
                    tracing::warn!(error = ?e, "reaper provision failed");
                }
            }
        }
    }
}
```

**Why `Weak<FirecrackerVmPool>`** : avoid keeping the pool alive past `pool.shutdown()`.  Reaper exits when the pool is dropped.

### 4.7 `FirecrackerMetrics` — Prometheus surface

14 metrics, all per `(kind, model_id, model_version, tenant_class)` :

| Metric | Type | Description |
|--------|------|-------------|
| `efe_nano_firecracker_warm_count` | Gauge | warm VMs in pool |
| `efe_nano_firecracker_in_use_count` | Gauge | leased VMs |
| `efe_nano_firecracker_cold_start_total` | Counter | cold provisioning events |
| `efe_nano_firecracker_cold_start_duration_seconds` | Histogram | cold start latency |
| `efe_nano_firecracker_crash_total` | Counter | VM crashes detected by reaper |
| `efe_nano_firecracker_reap_total` | Counter | VMs reaped |
| `efe_nano_firecracker_inference_duration_seconds` | Histogram | end-to-end inference (vsock + model) |
| `efe_nano_firecracker_vsock_rtt_seconds` | Histogram | vsock round-trip |
| `efe_nano_firecracker_vsock_error_total` | Counter | vsock protocol errors |
| `efe_nano_firecracker_acquire_wait_seconds` | Histogram | pool acquire latency |
| `efe_nano_firecracker_acquire_timeout_total` | Counter | acquire timeouts |
| `efe_nano_firecracker_pool_exhausted_total` | Counter | `max_total` cap hits |
| `efe_nano_firecracker_health_check_failure_total` | Counter | reaper health-ping failures |
| `efe_nano_firecracker_pid_exit_total` | Counter | observed Firecracker pid exits |

---

## 5. Data structures

```rust
// crates/efe-nano/src/runtime/pool/firecracker/types.rs (new module)

/// Sprint 8 — N11 : configuration for the FirecrackerVmPool.
#[derive(Debug, Clone)]
pub struct FirecrackerPoolConfig {
    pub binary_path: PathBuf,            // /opt/firecracker/firecracker
    pub kernel_image: PathBuf,           // /opt/firecracker/vmlinux
    pub cgroup_root: PathBuf,            // /sys/fs/cgroup/efe-nano
    pub api_socket_dir: PathBuf,         // /var/run/efe-nano/sockets/
    pub per_kind: HashMap<VmKind, KindConfig>,
    pub reaper_interval: Duration,
    pub vsock_timeout: Duration,
    pub boot_budget: Duration,           // max time to wait for vsock-ready
    pub idle_eviction_after: Duration,   // reap VMs idle > N min
}

#[derive(Debug, Clone)]
pub struct KindConfig {
    pub rootfs_path: PathBuf,            // /opt/efe-nano/rootfs/ner_clinical_v1.ext4
    pub min_warm: u32,                   // background-provisioned
    pub max_total: u32,                  // hard ceiling
    pub memory_mb: u32,                  // 256 for Mode B, 512 for Mode C
    pub vcpus: u32,                      // 1 for Mode B, 2 for Mode C
    pub cgroup_cpu_quota_us: i64,        // 1_000_000 = 1 full core
    pub cgroup_memory_high: u64,         // soft limit (B)
    pub cgroup_memory_max: u64,          // hard limit (B)
    pub cgroup_blkio_weight: u64,        // 10..1000
}

#[derive(Debug)]
pub struct WarmVm {
    pub id: VmId,
    pub kind: VmKind,
    pub cid: u32,
    pub firecracker_pid: u32,
    pub api_socket: PathBuf,
    pub cgroup_path: PathBuf,
    pub created_at: Instant,
    pub last_used_at: Mutex<Instant>,
    pub state: AtomicCell<VmState>,
    pub rpc_client: Arc<VsockRpcClient>,
}

#[derive(Debug, Clone, Copy)]
pub enum VmState {
    Provisioning,
    Warm,
    Reserved,    // checked-out but not yet executing
    InUse,
    Returning,   // execute finished, returning to pool
    Reaping,
    Dead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantClass {
    TierAudit,      // dedicated reservation
    Tier1,          // fair pool
    Experimental,   // best-effort pool
}

#[derive(Debug, Clone, Copy)]
pub struct VmId(pub u64);
```

---

## 6. Host ↔ guest RPC protocol

### 6.1 Frame format

Every message (request or response) is wrapped in a 5-byte header :

```text
┌────────┬──────────────────────┬──────────────────────────────────────┐
│ 0xEF   │   length : u32 BE    │   JSON body : `length` bytes         │
└────────┴──────────────────────┴──────────────────────────────────────┘
  magic           4 bytes                 variable
```

- **Magic byte** `0xEF` (mnemonic : EFE).  Defends against protocol confusion.
- **Length** big-endian u32 (max body 16 MB hard cap).
- **Body** : UTF-8 JSON.

### 6.2 Request schema

```json
{
  "schema_version": 1,
  "request_id": "<uuid>",                  // for telemetry correlation
  "method": "infer" | "health" | "shutdown",
  "params": { ... method-specific ... }
}
```

#### `infer` method

```json
{
  "method": "infer",
  "params": {
    "input": <serde_json::Value>,        // the OperationInput value
    "agent_id": "ner_clinical",
    "agent_version": "1.0.0",
    "trace_id": "<uuid>",                 // echoed in response for correlation
    "deadline_micros": 1726747200000000   // unix-micros (absolute)
  }
}
```

#### `health` method

```json
{ "method": "health", "params": {} }
```

#### `shutdown` method

```json
{ "method": "shutdown", "params": { "grace_secs": 5 } }
```

### 6.3 Response schema

```json
{
  "schema_version": 1,
  "request_id": "<echoed>",
  "result": { ... method-specific ... }
  // OR
  "error": { "code": "...", "message": "..." }
}
```

#### `infer` success result

```json
{
  "result": {
    "output": <serde_json::Value>,
    "model_invocation_micros": 18432,
    "model_memory_high_water_mb": 187
  }
}
```

#### Error codes (closed enum)

| Code | Meaning | Host action |
|------|---------|-------------|
| `input_schema_invalid` | Guest rejected input | Return `SchemaValidationFailed` |
| `model_oom` | Model exhausted memory | Reap VM, return `VmCrashed` |
| `deadline_exceeded` | Inference took > deadline | Return `Timeout` |
| `internal` | Daemon bug | Reap VM, return `Internal` |
| `not_loaded` | Model not pre-loaded for this VmKind | Reap, alarm — should never happen |

---

## 7. State machine of a VM lease

```text
                            spawn() succeeds
              ┌─────────────────┐
              │                 ▼
   ┌─────────────────┐    ┌────────┐    acquire()
   │  Provisioning   │    │  Warm  │ ─────────────▶  ┌──────────┐
   └─────────────────┘    └────────┘                 │ Reserved │
              ▲                 ▲                    └──────────┘
              │ provision_one() │ check_in()              │
              │ (reaper)        │                         │ execute()
              │                 │                         ▼
              │                 │                    ┌──────────┐
              │                 │                    │  InUse   │
              │                 │                    └──────────┘
              │                 │                         │
              │                 │   execute() OK          │ execute() ERR
              │                 │   (return to pool)      │ (reap)
              │                 │                         ▼
              │            ┌──────────┐              ┌─────────┐
              │            │ Returning│ ◀────────────│ Reaping │
              │            └──────────┘              └─────────┘
              │                 │                         │
              │                 │ checkin OK              │ reap done
              └─────────────────┘                         ▼
                                                   ┌─────────────┐
                                                   │    Dead     │
                                                   └─────────────┘
```

**Allowed transitions (state × event)** :

| From | Event | To | Side effects |
|------|-------|----|----|
| (none) | spawn() | Provisioning | fork Firecracker, write cgroup |
| Provisioning | boot success | Warm | push to `pool.warm` |
| Provisioning | boot fail | Dead | cleanup cgroup, log, metric |
| Warm | acquire() | Reserved | move to `pool.in_use` |
| Reserved | execute() called | InUse | start vsock RPC |
| InUse | RPC ok | Returning | record output |
| InUse | RPC err / timeout | Reaping | log error, schedule reap |
| InUse | deadline_exceeded | Reaping | send SIGTERM to daemon's inference task (Phase 4 ; Sprint 8 reaps whole VM) |
| Returning | checkin() | Warm | push back to `pool.warm` |
| Reaping | reap done | Dead | cgroup cleanup, metric |
| (any) | shutdown() | Reaping | graceful teardown |

---

## 8. Concurrency & locking model

### 8.1 Lock hierarchy (acquired in this order, never reversed)

1. `FirecrackerVmPool.pools` — `DashMap` shard lock (very short).
2. `KindPool.permits` — Semaphore (no lock, semaphore semantics).
3. `KindPool.warm` — Mutex (short, only push/pop).
4. `KindPool.in_use` — Mutex (short, insert/remove).
5. `WarmVm.state` — Atomic (no lock).
6. `WarmVm.rpc_client.socket` — Mutex (held during one RPC).

**Invariant** : never hold `warm` and `in_use` simultaneously.

### 8.2 The acquire happy path (warm hit)

```text
1. pool.pools.get(&pool_key)           // DashMap lock, ~ 50 ns
2. kind_pool.permits.acquire()         // semaphore, ~ 100 ns
3. kind_pool.warm.lock().pop_front()   // mutex, ~ 100 ns
4. kind_pool.in_use.lock().insert(...) // mutex, ~ 100 ns
5. vm.state.store(Reserved)            // atomic, ~ 20 ns
6. return VmLease { ... }
                                       ────────
                                       ~ 400 ns
```

### 8.3 The acquire cold path (miss + provision)

```text
1.   pool.pools.get(&pool_key)              // DashMap, 50 ns
2.   kind_pool.permits.acquire()            // semaphore, 100 ns
3.   kind_pool.warm.lock().pop_front()      // returns None
4.   lifecycle.spawn(kind, config).await    // 400 ms (the cold start budget)
5.   kind_pool.in_use.lock().insert(...)    // 100 ns
6.   return VmLease
                                            ─────────
                                            ~ 400 ms
```

**The 400 ms breakdown** (target) :
- `fork+exec firecracker` : 10 ms
- Linux kernel boot (custom vmlinux, no init scripts) : 80 ms
- `init.d/efe-inference` start + daemon main() : 50 ms
- Model mmap load : 200 ms (dominated by I/O)
- Vsock bind + first ping/pong : 5 ms
- Margin : 55 ms.

### 8.4 The release path (drop of `VmLease`)

```rust
impl Drop for VmLease {
    fn drop(&mut self) {
        // Hot path : no blocking ; defer to a tokio task.
        let pool = Arc::clone(&self.pool);
        let kind_key = self.kind_key.clone();
        let vm_id = self.vm_id;
        let outcome = self.outcome.take();
        tokio::spawn(async move {
            pool.check_in(kind_key, vm_id, outcome).await;
        });
    }
}
```

**Why spawn** : `Drop` is sync ; we don't want to block on the pool mutex.  Tokio spawn = fire-and-forget.

---

## 9. Failure modes & recovery — 12 distinct paths

| # | Failure | Detection | Recovery | User-visible |
|---|---------|-----------|----------|--------------|
| F1 | Firecracker `fork+exec` fails (missing binary, no /dev/kvm) | spawn returns `IoError` | metric `cold_start_failure_total`, fall back to `Exhausted` after retry | `VmPoolError::ProvisionFailed` |
| F2 | Kernel fails to boot (corrupted vmlinux) | boot watchdog timeout (boot_budget) | reap, alarm SRE | `VmPoolError::ProvisionFailed` |
| F3 | Rootfs corrupted | Firecracker API returns 400 on `/drives` | reap, alarm | `VmPoolError::ProvisionFailed` |
| F4 | Vsock bind fails inside guest | host doesn't see :5000 listening within boot_budget | reap, alarm | `VmPoolError::ProvisionFailed` |
| F5 | Model OOM during inference | guest reports `model_oom` OR vsock EOF | reap VM, return error to caller, replenish pool | `VmPoolError::VmCrashed` |
| F6 | Inference exceeds per-call deadline | host RPC timeout fires | reap, return | `VmPoolError::AcquireTimeout` (re-purposed) |
| F7 | Cgroup memory.max breached | kernel OOM-killer SIGKILLs the guest | reaper sees pid exit, replenishes | `VmPoolError::VmCrashed` |
| F8 | Cgroup CPU quota saturated | inference latency degrades, doesn't fail | metric `inference_duration_p99` alerts ops | (silent — investigate) |
| F9 | Vsock connection refused | guest crashed before connect | reap, retry once, fail | `VmPoolError::VmCrashed` |
| F10 | RPC malformed (magic byte wrong, length off) | host parser rejects | reap (assume guest compromised), alarm SEV-1 | `VmPoolError::Internal` |
| F11 | Reaper task itself panics | tokio JoinHandle returns Err | alarm SEV-1, restart Reaper, runtime continues | (silent — escalation) |
| F12 | Host out of disk (rootfs copy fails) | spawn returns ENOSPC | alarm SEV-1, refuse all new spawns | `VmPoolError::ProvisionFailed` |

### 9.1 Backpressure semantics

When `KindPool.permits` is empty (every VM in use, every slot in `max_total` filled), `acquire` blocks on the semaphore up to `wait_budget`.  Past the budget → `VmPoolError::AcquireTimeout`.  Caller (the dispatcher) returns `OperationErrorKind::ResourceExceeded` or `Timeout` to the customer based on retry policy.

---

## 10. Security analysis & threat model

### 10.1 Threat actors

| Actor | Capability | Mitigation |
|-------|------------|------------|
| Malicious tenant A | Submits a crafted input to exploit a model bug | Hard VM isolation, cgroup memory cap, reap on crash |
| Insider with shell on host | Tries to dump cross-tenant VM memory | Host runs in unprivileged container with restricted /proc ; VMs are root-only |
| External attacker via vsock | Sends malformed RPC to another VM | Vsock CID per VM is per-process random ; vsock socket only accessible from host (not cross-VM) |
| Hardware side-channel (Spectre, Meltdown) | Speculative execution leak | Firecracker uses IBPB / RSB stuffing on every VM exit ; KVM L1TF mitigations |
| Model file tampering on disk | Replaces `model.onnx` with malicious | Rootfs is read-only at runtime ; cosign-signed at build time ; verified at boot |

### 10.2 Defense-in-depth layers

```text
Layer 5 │ App-level audit chain (HMAC + Merkle) — Sprint 6-7
Layer 4 │ Vsock RPC framing (magic byte, length cap) — Sprint 8
Layer 3 │ Cgroups v2 (cpu, memory, io, pids) — Sprint 8
Layer 2 │ Firecracker jailer (seccomp filter + chroot) — Sprint 8 (basic) / Phase 4 (tuned)
Layer 1 │ KVM hardware isolation — kernel
Layer 0 │ CPU mitigations (IBPB, RSB) — firmware/kernel
```

### 10.3 The "vsock RPC injection" subtle attack

An attacker who compromises one VM might try to inject vsock packets to a sibling VM's CID.  **Mitigation** : vsock CIDs are **per-VM, allocated by host**.  A guest only sees its own CID (`VMADDR_CID_LOCAL`) and the host (`VMADDR_CID_HOST`).  It **cannot** dial another VM's CID — Firecracker filters.

### 10.4 Rootfs integrity

At boot time, the daemon computes SHA-256 of `model.onnx` and compares to the expected hash baked into `/etc/efe/model.sha256`.  Mismatch → daemon exits 1 → reap.  The expected hash is signed at build time by cosign ; verification on the host before mounting.

---

## 11. Performance budget

### 11.1 Sprint 8 SLO targets

| SLO | Target | Verification |
|-----|--------|--------------|
| `efe_nano_firecracker_warm_count` | ≥ `min_warm` per (kind, model, class) 99 % of time | Grafana dashboard alert |
| Acquire p50 (hit) | ≤ 1 ms | bench |
| Acquire p99 (hit) | ≤ 10 ms | bench |
| Acquire p99 (cold) | ≤ 500 ms | bench |
| Hit rate | ≥ 95 % at 100 RPS Mode B | bench (60s soak) |
| Vsock RTT p99 | ≤ 5 ms | bench |
| Inference p99 (ner_clinical) | ≤ 200 ms (model 150 + RPC 50) | bench |
| Throughput Mode B per node | ≥ 100 calls/s/node | bench |
| Crash rate | ≤ 1/1000 calls | soak |
| Memory headroom | ≥ 20 % host free | Grafana alert |

### 11.2 Latency budget breakdown (Mode B, warm hit)

```
TOTAL : ≤ 200 ms p99

  ├── Dispatcher → pool.acquire()                10 ms p99
  │     ├── Pool lookup (DashMap)                 1 ms
  │     ├── Semaphore acquire                     2 ms
  │     └── Lease construction                    7 ms
  │
  ├── lease.execute() vsock RPC                    5 ms p99
  │     ├── Serialize request                     0.5 ms
  │     ├── Write frame to vsock                  0.5 ms
  │     ├── Guest demux + dispatch                1 ms
  │     ├── Read response                         1 ms
  │     └── Deserialize                           2 ms
  │
  ├── Guest inference                           150 ms p99
  │     (ner_clinical ONNX over CPU, single shot)
  │
  └── Return to pool (Drop → tokio::spawn)          0 ms (async, off-path)
```

### 11.3 Cold start budget

```
TOTAL : ≤ 500 ms p99

  ├── fork + exec firecracker                     10 ms
  ├── Firecracker API config (4 round trips)      20 ms
  ├── InstanceStart                                5 ms
  ├── Linux kernel boot                           80 ms
  ├── rcS + init scripts                          30 ms
  ├── inference-daemon main() + load model       300 ms
  ├── Vsock bind + first health-ping              5 ms
  └── Margin                                      50 ms
```

---

## 12. Resource sizing & cost model

### 12.1 Per-VM resource profile

| Kind | Memory | vCPU | Disk (rootfs) | Cold start | Inference p99 |
|------|--------|------|---------------|------------|---------------|
| `ner_clinical` (Mode B) | 256 MB | 1 | 256 MB | 450 ms | 150 ms |
| `ner_legal` (Mode B) | 256 MB | 1 | 256 MB | 450 ms | 150 ms |
| `intent_classify` (Mode B) | 128 MB | 1 | 128 MB | 300 ms | 25 ms |
| `lang_detect` (Mode B) | 64 MB | 1 | 64 MB | 200 ms | 10 ms |
| `embedder` (Mode B) | 384 MB | 1 | 384 MB | 480 ms | 30 ms |
| `llm_caller` (Mode C) | 512 MB | 2 | 64 MB | 250 ms | (5 s LLM-bound) |

### 12.2 Pool sizing per node (single 32-core / 128 GB instance)

| Kind | min_warm | max_total | Memory budget | Notes |
|------|----------|-----------|---------------|-------|
| `ner_clinical` | 20 | 50 | 12.8 GB | high-traffic clinical extraction |
| `ner_legal` | 10 | 30 | 7.7 GB | moderate-traffic legal |
| `intent_classify` | 30 | 80 | 10.2 GB | classification volume |
| `lang_detect` | 20 | 50 | 3.2 GB | cheap, abundant |
| `embedder` | 30 | 80 | 30.7 GB | most-called for vector store |
| `llm_caller` | 30 | 80 | 41 GB | Mode C concurrency |
| **TOTAL warm** | 140 VMs | 370 VMs | **105.6 GB** | leaves 22 GB host headroom |

### 12.3 Cost model

Per AWS `c7i.metal-24xl` ($4.20/hour spot) :
- 32 GB allocated to host OS + EFE Nano runtime.
- 95 GB allocated to warm VMs.
- ≤ 370 concurrent inferences.

Per-call cost (amortized at 1 000 calls/sec/node sustained) :
- Compute : $4.20/hr ÷ 3 600 ÷ 1 000 = **$0.00000117 per call**.
- Negligible — pricing is gated by LLM provider cost (Mode C) not infra.

---

## 13. Tenant isolation guarantees

### 13.1 What's isolated

| Resource | Isolation mechanism | Cross-tenant leak risk |
|----------|--------------------|-----------------------|
| Memory | Firecracker KVM per VM | Negligible — separate address spaces |
| CPU | Cgroup cpu.max per VM + KVM scheduler | Possible side-channel via cache — mitigated by KVM cache flush on VM exit |
| Disk I/O | Cgroup io.max + read-only rootfs | None (no shared writeable disk) |
| Network | Vsock only ; no inter-VM network | None — vsock CIDs filtered by Firecracker |
| Model weights | Per-VM copy in private memory | None — models are read-only post-mmap |
| Trace audit | Per-tenant HMAC chain (Sprint 6) | None (Sprint 6 invariant) |

### 13.2 What's pooled (intentionally shared)

| Resource | Sharing scope | Justification |
|----------|---------------|---------------|
| Warm VM | (kind, model, tenant_class) | Pool efficiency ; class boundary preserves SLA |
| Rootfs image | All VMs of same kind | Read-only ; no information flow |
| Kernel image (vmlinux) | All VMs of all kinds | Pinned at build, signed, read-only |

### 13.3 The tenant_class partition argument

A naive "one pool per tenant_id" creates pool fragmentation : with 10 000 tenants, 100 000 warm VMs to maintain.  Memory cost prohibitive.

A naive "one pool for everyone" has noisy-neighbor risk : a TIER-1 tenant spamming `embedder` starves a TIER-AUDIT tenant's reserved capacity.

**The TenantClass partition** : 3 pools per kind (TIER-AUDIT, TIER-1, EXPERIMENTAL).  TIER-AUDIT customers get **guaranteed reservation** (their portion of `min_warm`).  TIER-1 share fair-queue.  EXPERIMENTAL get best-effort.  This matches the M50 stability tier ladder.

---

## 14. Integration with M25 (efe-agent-runner)

### 14.1 What M25 provides today

`crates/efe-agent-runner/src/sandbox/firecracker.rs` ships :

- `FirecrackerSandbox` — wraps `firectl` to spawn a VM.
- `CgroupController` — creates per-run cgroups.
- `VsockClient` — basic vsock connect/send/recv.
- `Reaper` — kills runs that exceed `cost_cap_per_run_usd`.

### 14.2 What N11 adds vs. reuses

| Capability | Source | Notes |
|-----------|--------|-------|
| Firecracker spawn / API | **Reuse M25** | `M25::sandbox::firecracker::FirecrackerSandbox::spawn` |
| Cgroup mgmt | **Reuse M25** | Extended with per-VM tenant_class tag |
| Vsock client | **Extend M25** | M25 vsock is simple `connect+send` ; N11 adds magic-byte framing + retry |
| Pool management | **N11 new** | M25 is one-VM-per-run ; N11 is many-VM-pool |
| Reaper | **N11 new** | M25's reaper is run-scoped ; N11 is pool-scoped (health-pings, idle eviction) |
| Inference daemon (guest) | **N11 new** | M25's guest runs arbitrary agent code ; N11's guest is a fixed-purpose inference loop |
| Rootfs builder | **N11 new** | M25 customer-publishes ; N11 ships EFE-curated images |

### 14.3 Where the seam goes

```text
crates/efe-nano/src/runtime/pool/
  ├── mod.rs                       (existing — VmPool trait + VmLease)
  ├── stub.rs                      (existing — StubVmPool, Sprint 4)
  └── firecracker/                 (NEW — Sprint 8)
      ├── mod.rs                   (FirecrackerVmPool, re-exports)
      ├── config.rs                (FirecrackerPoolConfig, KindConfig)
      ├── lifecycle.rs             (VmLifecycleController — wraps M25 sandbox)
      ├── kind_pool.rs             (KindPool : warm + in_use + permits)
      ├── reaper.rs                (background sweep + provision)
      ├── rpc.rs                   (VsockRpcClient with magic+length framing)
      ├── metrics.rs               (14 Prometheus counters)
      └── tests/
          ├── lifecycle_tests.rs
          ├── pool_tests.rs
          ├── reaper_tests.rs
          └── rpc_tests.rs

crates/efe-agent-runner/src/sandbox/firecracker.rs
  ├── (existing M25 code untouched)
  └── + 1 new pub fn : `spawn_for_inference()`
          (variant of M25's `spawn()` configured for N11 inference daemons)

crates/efe-nano-inference-daemon/  (NEW workspace member — Sprint 8)
  ├── Cargo.toml                   (musl-static target)
  ├── src/main.rs                  (boot + load model + vsock listen)
  ├── src/model.rs                 (ONNX runtime wrapper)
  ├── src/rpc.rs                   (vsock server, mirror of host rpc.rs)
  └── tests/
```

---

## 15. Migration plan : StubVmPool → FirecrackerVmPool

### 15.1 The feature flag

A single env var **`EFE_NANO_VM_BACKEND`** controls which pool the runtime constructs at startup :

```text
EFE_NANO_VM_BACKEND=stub        → StubVmPool (Sprint 4-7 default)
EFE_NANO_VM_BACKEND=firecracker → FirecrackerVmPool (Sprint 8 ship)
```

Both impl the same `VmPool` trait → no dispatcher / runtime code change.

```rust
// crates/efe-nano/src/runtime/pool/mod.rs (extension)
pub fn make_pool_from_env() -> Result<Arc<dyn VmPool>, anyhow::Error> {
    match std::env::var("EFE_NANO_VM_BACKEND").as_deref() {
        Ok("firecracker") => {
            #[cfg(feature = "firecracker")]
            {
                let cfg = firecracker::FirecrackerPoolConfig::from_env()?;
                Ok(Arc::new(firecracker::FirecrackerVmPool::new(cfg)?))
            }
            #[cfg(not(feature = "firecracker"))]
            {
                anyhow::bail!("firecracker backend requested but feature not compiled");
            }
        }
        Ok("stub") | Err(_) => Ok(Arc::new(StubVmPool::default())),
        Ok(other) => anyhow::bail!("unknown VM backend: {other}"),
    }
}
```

### 15.2 Rollout phases

| Phase | Date | Action | Risk |
|-------|------|--------|------|
| **Phase A** | Sprint 8 days 1-15 | Build N11, internal CI uses `stub` | None (production untouched) |
| **Phase B** | Sprint 8 days 16-18 | Internal staging cluster runs `firecracker` | Bug surface area : staging only |
| **Phase C** | Sprint 8 days 19-20 | 7-day soak ; collect SLO data | Pool degradation, cold start drift |
| **Phase D** | Sprint 9 day 1 | Flip 1 % of TIER-EXPERIMENTAL traffic | Limited blast radius |
| **Phase E** | Sprint 9 days 2-7 | Ramp to 10 % then 50 % then 100 % | Same trait surface → safe rollback |
| **Phase F** | Sprint 9 day 8 | Flip TIER-1 customers | Higher impact ; requires Phase E green |
| **Phase G** | Sprint 9 day 14 | Flip TIER-AUDIT customers | Last (most demanding SLAs) |
| **Phase H** | Sprint 10+ | Deprecate `stub` from prod, keep for tests | None |

### 15.3 Rollback procedure

`kubectl set env deployment/efe-nano EFE_NANO_VM_BACKEND=stub && kubectl rollout restart deployment/efe-nano` — under 30 s.

---

## 16. Testing strategy — 5 tiers

| Tier | Environment | What it tests | Test count target |
|------|-------------|--------------|-------------------|
| **Tier 1** | Pure Rust (no Firecracker) | `VmPool` trait conformance ; KindPool semantics ; RPC framing ; reaper FSM | ≥ 60 unit tests |
| **Tier 2** | Mock Firecracker (fake spawner returns canned WarmVm) | Pool lifecycle ; concurrency ; failure mode injection | ≥ 30 integration tests |
| **Tier 3** | Real Firecracker on dev workstation (Linux + KVM, macOS via Lima) | End-to-end cold start, vsock RPC, model load | ≥ 20 smoke tests |
| **Tier 4** | Kind cluster (K8s + DaemonSet) | Helm deploy, multi-node, K8s networking | ≥ 10 e2e tests |
| **Tier 5** | Production staging soak | SLO compliance, crash rate, memory leak | 7-day continuous run |

### 16.1 Tier 1 tests (no Firecracker)

Use a `MockLifecycleController` that returns pre-canned `WarmVm` instances without actually forking processes.  Tests run in CI on every PR.

```rust
#[tokio::test]
async fn pool_serves_warm_vm_under_a_millisecond() {
    let pool = FirecrackerVmPool::with_mock_lifecycle(
        MockLifecycleController::with_warm(VmKind::LocalModel { .. }, 10),
        config(),
    );
    let start = Instant::now();
    let lease = pool.acquire(VmKind::LocalModel { .. }, Duration::from_secs(1)).await.unwrap();
    assert!(start.elapsed() < Duration::from_millis(1));
    drop(lease);
}

#[tokio::test]
async fn pool_provisions_on_miss_within_budget() { ... }

#[tokio::test]
async fn pool_returns_acquire_timeout_when_exhausted() { ... }

#[tokio::test]
async fn reaper_replaces_dead_vm_within_10_seconds() { ... }

#[tokio::test]
async fn rpc_framing_rejects_bad_magic_byte() { ... }

#[tokio::test]
async fn rpc_framing_rejects_oversized_body() { ... }
```

### 16.2 Tier 3 smoke tests (real Firecracker)

CI runs on a self-hosted x86_64 + KVM runner (`runs-on: [self-hosted, kvm]`).  Mac dev path : Lima VM with nested virt.

```rust
#[tokio::test]
#[ignore = "requires KVM"]
async fn firecracker_cold_start_under_500ms() {
    let pool = FirecrackerVmPool::new(real_config()).unwrap();
    let start = Instant::now();
    let lease = pool.acquire(VmKind::LocalModel {
        model_id: "ner_clinical".into(),
        model_version: "1.0.0".into(),
    }, Duration::from_secs(2)).await.unwrap();
    assert!(start.elapsed() < Duration::from_millis(500),
        "cold start was {:?}, must be ≤ 500ms", start.elapsed());
}

#[tokio::test]
#[ignore = "requires KVM"]
async fn firecracker_inference_returns_real_ner_output() { ... }

#[tokio::test]
#[ignore = "requires KVM"]
async fn firecracker_vm_oom_does_not_crash_host() { ... }
```

### 16.3 Tier 4 K8s e2e

```bash
make e2e-kind
# spins up kind cluster
# installs Helm chart efe-nano-firecracker
# runs `bench_e2e_load.rs` for 60s
# asserts hit_rate >= 0.95, p99_inference < 200ms
```

### 16.4 Tier 5 staging soak

7-day continuous run with synthetic load at 100 RPS Mode B + 10 RPS Mode C.  Daily SLO snapshot.  Auto-rollback if hit_rate < 0.9 for 5+ min OR crash_rate > 1/100.

---

## 17. Operational runbook

### 17.1 Pool degraded alarm

**Symptom** : `efe_nano_firecracker_warm_count{kind="ner_clinical"} < 10` for > 60 s.

**Diagnostic** :
1. `kubectl logs -n efe-nano <pod> | grep "cold_start"` — provision failures?
2. `kubectl exec <pod> -- ls /sys/fs/cgroup/efe-nano/` — orphan cgroups?
3. `df -h /var/lib/firecracker/rootfs/` — disk full?

**Remediation** :
- Disk full → expand PVC, restart pod.
- Provision failures → check `/dev/kvm` permissions on node.
- Orphan cgroups → manual cleanup, restart reaper.

### 17.2 VM crash spike

**Symptom** : `rate(efe_nano_firecracker_crash_total[5m]) > 0.5/s`.

**Diagnostic** :
1. `kubectl logs <pod> | grep "vm_id=" | tail -100` — crash signatures?
2. Identify : OOM ? vsock EOF ? exit_code ?
3. If OOM → which model_id ? compare to recent traffic spike.

**Remediation** :
- Single model OOM → bump `memory_mb` for that kind, redeploy rootfs.
- Cross-model → revert recent rootfs build, page on-call.

### 17.3 Vsock latency spike

**Symptom** : `histogram_quantile(0.99, rate(efe_nano_firecracker_vsock_rtt_seconds_bucket[5m])) > 0.05`.

**Diagnostic** :
1. Host CPU saturated ?
2. Single VM responsible ? Look at per-vm_id distribution.
3. Cgroup CPU quota too tight ?

**Remediation** :
- Throttle non-essential workloads on the host.
- Bump cgroup CPU quota if budget allows.
- Reap the slow VM and re-provision.

### 17.4 Out of memory on host

**Symptom** : `node_memory_MemAvailable_bytes < 4GiB` AND `efe_nano_firecracker_warm_count` at max.

**Remediation** :
1. Reaper triggers aggressive LRU eviction (idle > 1 min) — automatic.
2. If still tight : kill highest-memory kind first (`embedder`).
3. Manual : `kubectl exec <pod> -- efe-nano-admin pool drain --kind embedder --count 10`.

---

## 18. Deployment topology

### 18.1 Kubernetes shape

```yaml
# DaemonSet — one EFE Nano pod per KVM-capable node.
apiVersion: apps/v1
kind: DaemonSet
metadata:
  name: efe-nano-firecracker
spec:
  selector:
    matchLabels:
      app: efe-nano-firecracker
  template:
    spec:
      hostPID: true                          # cgroup mgmt
      nodeSelector:
        node.efe.io/firecracker-ready: "true"
      containers:
      - name: efe-nano
        image: registry.efe.internal/efe-nano:v0.8.0
        env:
        - name: EFE_NANO_VM_BACKEND
          value: firecracker
        - name: EFE_NANO_AUDIT_HMAC_MASTER
          valueFrom:
            secretKeyRef: { name: efe-audit-master, key: master }
        securityContext:
          privileged: true                   # for /dev/kvm + cgroup
          capabilities:
            add: ["SYS_ADMIN", "NET_ADMIN"]
        volumeMounts:
        - { mountPath: /dev/kvm, name: kvm }
        - { mountPath: /var/lib/firecracker, name: rootfs }
        - { mountPath: /sys/fs/cgroup, name: cgroup }
        resources:
          requests:  { cpu: 16, memory: 64Gi }
          limits:    { cpu: 32, memory: 128Gi }
      volumes:
      - { name: kvm, hostPath: { path: /dev/kvm } }
      - { name: rootfs, hostPath: { path: /var/lib/firecracker } }
      - { name: cgroup, hostPath: { path: /sys/fs/cgroup } }
```

### 18.2 Node requirements

| Resource | Min | Recommended |
|----------|-----|-------------|
| KVM | required | required |
| CPU | 16 cores | 32 cores (c7i.metal-24xl) |
| RAM | 64 GB | 128 GB |
| Disk (rootfs cache) | 10 GB | 50 GB (room for 5+ kinds × multiple versions) |
| Kernel | ≥ 5.10 | ≥ 6.1 (better cgroup v2) |
| Firecracker version | 1.5.0 | 1.6.0+ |

### 18.3 Cloud node SKUs that satisfy

- **AWS** : `c7i.metal-24xl`, `i4i.metal`, `m6i.metal` (all expose `/dev/kvm`).
- **GCP** : Sole-tenant `c3-standard-44` with nested virt enabled.
- **Azure** : `Dv5-series` confidential VMs with AMD SEV-SNP (Phase 5 hardware-attested).
- **On-prem** : any modern Xeon / EPYC with KVM enabled in BIOS.

---

## 19. Sprint 8 day-by-day plan (20 working days)

### Week 1 — Foundation (days 1-5)

| Day | Owner | Deliverable | Verification |
|-----|-------|-------------|--------------|
| 1 | founder | M25 sandbox API exploration ; identify exact integration seam | design notes + 2 ADRs |
| 2 | senior eng | `crates/efe-nano-inference-daemon/` scaffold + main.rs | `cargo build --target x86_64-unknown-linux-musl` clean |
| 3 | senior eng | Inference daemon : vsock listen + health endpoint | `cargo test -p efe-nano-inference-daemon` GREEN |
| 4 | founder | Inference daemon : ONNX runtime + model load | manual run, ner_clinical model loads in < 200 ms |
| 5 | both | Inference daemon : infer method end-to-end (no host) | unit test : seeded input → expected output |

### Week 2 — Host plumbing (days 6-10)

| Day | Owner | Deliverable | Verification |
|-----|-------|-------------|--------------|
| 6 | senior eng | `crates/efe-nano/src/runtime/pool/firecracker/config.rs` + types.rs | compile + unit tests for `KindConfig` validation |
| 7 | senior eng | `lifecycle.rs` : spawn / reap / cgroup wiring | mock-based unit tests GREEN |
| 8 | founder | `rpc.rs` : VsockRpcClient with magic byte framing | unit tests for framing edge cases (12 tests) |
| 9 | founder | `kind_pool.rs` : checkout/checkin/permits | concurrency tests with `tokio::join!` |
| 10 | both | `FirecrackerVmPool` wires Layers 1+2+3 (no real Firecracker yet) | Tier 1 trait-conformance tests GREEN (60+) |

### Week 3 — Real Firecracker + reaper (days 11-15)

| Day | Owner | Deliverable | Verification |
|-----|-------|-------------|--------------|
| 11 | senior eng | Rootfs build script for ner_clinical | OCI artifact pushed to dev registry |
| 12 | senior eng | Rootfs builds for ner_legal + intent_classify + lang_detect + embedder | 5 OCI artifacts available |
| 13 | founder | `reaper.rs` : background sweep + provision | Tier 1 reaper tests GREEN |
| 14 | both | First end-to-end on dev workstation (Linux + KVM) | manual : ner_clinical extraction on real model |
| 15 | both | Tier 3 smoke tests on self-hosted runner | CI green for `#[ignore = "requires KVM"]` suite |

### Week 4 — Polish + soak (days 16-20)

| Day | Owner | Deliverable | Verification |
|-----|-------|-------------|--------------|
| 16 | senior eng | Helm chart `efe-nano-firecracker` | `helm template` GREEN, kind cluster install GREEN |
| 17 | founder | Tier 4 K8s e2e tests | `make e2e-kind` GREEN |
| 18 | senior eng | Metrics dashboard (Grafana JSON) + 4 alert rules | manual review by SRE-lead |
| 19 | both | Staging deploy + 24h smoke | hit rate ≥ 95 %, no crashes |
| 20 | both | Sprint 8 retro + Sprint 9 handoff | retro doc published |

### Sprint 9 (post-Sprint-8) — Soak + rollout

- Days 1-7 : 7-day production-grade soak in staging.
- Days 8-14 : staged production rollout (1 % → 10 % → 50 % → 100 % over TIER-EXPERIMENTAL → TIER-1 → TIER-AUDIT).

---

## 20. Open questions / risks register

| # | Risk | Likelihood | Impact | Mitigation | Owner |
|---|------|-----------|--------|------------|-------|
| R1 | KVM unavailable on managed K8s (EKS/GKE without nested virt) | Low | High | Pin to specific instance types ; document on-prem fallback | platform lead |
| R2 | Firecracker boot time exceeds 500 ms target | Medium | Medium | Phase 4 snapshot/restore ; Sprint 8 may need to accept 700 ms | founder |
| R3 | ONNX runtime size bloats rootfs > 1 GB | Medium | Medium | Switch to tinier runtime (e.g. tract) ; quantize models more aggressively | senior eng |
| R4 | Vsock RTT > 5 ms under load | Low | Medium | Bench early ; consider unix-socket fallback for same-host | founder |
| R5 | Cgroup v2 API quirks on RHEL kernels | Medium | Low | Pin to Amazon Linux 2023 + Ubuntu 22.04 | platform lead |
| R6 | Rootfs build pipeline becomes bottleneck (5 models × per-PR) | High | Medium | Lazy build : only models that changed ; cache OCI layers | senior eng |
| R7 | Operator confusion about cgroup hierarchy | High | Low | Runbook + Grafana dashboards | SRE lead |
| R8 | Model file tampering at registry | Low | Critical | Cosign-signed images ; verify on boot | security lead |
| R9 | Side-channel leak across tenants on same node | Low | Critical | KVM cache flush, Spectre mitigations, defense in depth | security lead |
| R10 | Sprint 8 slips past 4 weeks | Medium | Medium | Cut scope to 3 kinds (drop ner_legal + embedder to Sprint 9) | founder |

---

## 21. Decisions log (5 ADRs)

### ADR-N11-001 : Re-use M25 vs. build standalone

**Context** : M25 (`efe-agent-runner`) already ships Firecracker integration for autonomous LLM agents.  Sprint 8 needs the same primitive for nano-agent inference.

**Decision** : **Extend M25's `sandbox::firecracker` module** with a `spawn_for_inference()` variant.  Do not duplicate the spawner.

**Consequences** :
- ✅ DRY : one Firecracker integration to maintain.
- ✅ Battle-tested : M25 has been in EXPERIMENTAL since Sprint M25 ship.
- ⚠️ Coupling : N11 inherits any M25 vulnerabilities ; mitigated by shared security review.
- ⚠️ Cross-crate refactor : extracting the shared bits to `efe-firecracker` (a new crate) is the right Phase 4 cleanup, but Sprint 8 just adds a pub fn.

**Alternatives rejected** :
- Standalone : duplicates 2 000 LOC of spawner logic. Rejected.
- Wholesale extract to new crate : Sprint 8 scope creep. Defer to Phase 4.

### ADR-N11-002 : Guest runtime language

**Context** : The inference daemon runs inside the microVM.  Choices : Python+torch, Rust+ONNX, C+++libtorch, WASM.

**Decision** : **Rust + ONNX runtime (`ort` crate), statically linked with musl**.

**Consequences** :
- ✅ Smallest cold start (no Python interpreter init).
- ✅ Smallest binary (~ 12 MB stripped).
- ✅ Same language as host (review surface is one ecosystem).
- ⚠️ ONNX model conversion required from PyTorch checkpoints. Done at build time.
- ⚠️ ONNX feature gap : some PyTorch ops don't have ONNX equivalents. Mitigated : starter pack models are simple (NER, classify, embed) and ONNX-friendly.

**Alternatives rejected** :
- Python + torch : 200 MB interpreter, 500 ms cold start regression. Rejected.
- C++ + libtorch : team has no C++ expertise on inference. Rejected.
- Wasm + wasmtime : promising for Phase 5, but no mature ML runtime yet. Defer.

### ADR-N11-003 : Model storage : pre-baked vs. side-loaded

**Context** : Models can either live in the rootfs (pre-baked) or be mounted from a separate device at boot.

**Decision** : **Pre-baked into rootfs**.

**Consequences** :
- ✅ One artifact per (model, version) — simple build pipeline.
- ✅ Tamper-evidence : rootfs is cosign-signed.
- ✅ Cold start optimization : model is already on the VM disk at boot.
- ⚠️ Per-model rootfs grows (256-384 MB).  Mitigation : aggressive quantization (int8 / fp16).

**Alternatives rejected** :
- Side-loaded via virtio-blk : extra device per VM, more complex Firecracker config. Rejected.
- Hot-reload from S3 : adds network dependency at boot. Defer to Phase 4 (BYO-model).

### ADR-N11-004 : Snapshot/restore for cold start optimization

**Context** : Firecracker supports snapshot/restore — boot once, dump VM memory + state to disk, restore for new VMs.  Cold start drops from ~ 500 ms to ~ 100 ms.

**Decision** : **Defer to Phase 4**.

**Consequences** :
- Sprint 8 ships with 500 ms cold start budget — acceptable for ≥ 95 % hit rate.
- Phase 4 (Sprint 11) adds snapshot/restore : 5× cold start improvement.
- ⚠️ Implementation is non-trivial : snapshot timing (post-model-load), restore correctness (vsock ports, network state), storage of snapshots (~ 200 MB per kind × tenant_class).

**Alternatives rejected** :
- Sprint 8 inclusion : adds 1 week to the timeline.  Cut scope for now.
- Never : leaves 5× perf on the table at scale.  Mandatory Phase 4.

### ADR-N11-005 : Networking : vsock-only vs. tap-based

**Context** : Guests need to communicate with the host (inference RPC).  Mode C guests also need to reach the M21 LLM router on the host.

**Decision** : **Vsock-only for Sprint 8**.  Mode C guests reach M21 via a host-side **vsock proxy** that forwards to the LLM router.

**Consequences** :
- ✅ Maximum simplicity : no tap interface, no DHCP, no network namespaces.
- ✅ Strongest isolation : guests have **no IP stack**, can't be tricked into reaching the internet.
- ✅ Sufficient : Mode B doesn't need network at all ; Mode C only needs to reach the LLM router proxy.
- ⚠️ Phase 4 may want tap for : multimodal blob transfers (large image inputs > 16 MB RPC cap), debug shell. Defer.

**Alternatives rejected** :
- Tap-based : adds 4-5 components (DHCP, bridge, MAC tracking, NAT). Scope cut.
- Hybrid : both vsock + tap. Complexity nightmare. Rejected.

---

## Annexe A — Guest InferenceDaemon protocol spec

### A.1 Boot sequence

```text
1. Kernel boots.
2. /sbin/init runs /etc/init.d/efe-inference.
3. /etc/init.d/efe-inference :
     - reads /proc/cmdline for `efe.model_id=X efe.model_version=Y`
     - sets ulimit -v <memory_high_kib>
     - sets ulimit -t <cpu_quota_sec>
     - execs /opt/bin/inference-daemon
4. Daemon main() :
     - parse env (EFE_MODEL_ID, EFE_MODEL_VERSION from cmdline)
     - verify SHA-256 of /opt/models/{X}/{Y}/model.onnx against /etc/efe/model.sha256
     - if mismatch : exit 2
     - mmap model
     - create ONNX inference session
     - bind AF_VSOCK on (cid=VMADDR_CID_ANY, port=5000)
     - register SIGTERM handler (exit 0)
     - send health-ready ping over vsock (host waits on this)
     - enter accept loop
5. Accept loop :
     - on connect : read length-prefixed JSON
     - dispatch method
     - write length-prefixed JSON response
     - close connection (one request per connection — simplifies host reconnect logic)
```

### A.2 Detailed method semantics

#### `infer`

**Steps inside the daemon** :
1. Parse `params.input` against the agent's input schema (compiled at boot).
2. Convert to tensor (one tensor per ONNX input port).
3. Call `session.run(inputs)`.
4. Convert output tensors back to JSON.
5. Return success with `output` + metrics.

**Per-call latency budget (ner_clinical Mode B)** :
- Parse + validate : 1 ms
- Tensor conversion : 2 ms
- ONNX inference : 140 ms (CPU only, batch=1)
- Output conversion : 2 ms
- Total : 145 ms.

#### `health`

Returns immediately :

```json
{ "result": { "status": "ready", "uptime_secs": 312, "model_id": "ner_clinical", "model_version": "1.0.0" } }
```

Used by the host reaper for liveness.

#### `shutdown`

Daemon ACKs immediately, then exits after `params.grace_secs`.  Allows host to gracefully drain.

### A.3 Daemon error taxonomy

| Code | Meaning | Daemon exit code |
|------|---------|------------------|
| `input_schema_invalid` | Caller-side | 0 (keep VM alive) |
| `model_oom` | OOM during inference | 137 (SIGKILL by cgroup) |
| `deadline_exceeded` | Inference > deadline | 0 (keep VM alive — caller retry on different VM) |
| `internal` | Daemon bug | 1 (host reaps) |
| `not_loaded` | Should never happen post-boot | 2 (host alarms) |

---

## Annexe B — Rootfs build pipeline spec

### B.1 Image layout

```text
/etc/efe/
  ├── model.sha256             # SHA-256 of the model file
  ├── kind.json                # { "id": "ner_clinical", "version": "1.0.0", ... }
  └── daemon.toml              # vsock port, log level, etc.

/opt/bin/
  └── inference-daemon         # statically-linked musl Rust binary, ~ 12 MB

/opt/models/{model_id}/{version}/
  ├── model.onnx               # quantized model (int8 or fp16)
  ├── input_schema.json        # for guest-side input validation
  ├── output_schema.json       # for guest-side output validation
  └── tokenizer.json           # if applicable (NER, classify)

/sbin/init                     # Alpine BusyBox init
/etc/init.d/efe-inference      # OpenRC init script
```

### B.2 Build dependency graph

```text
inference-daemon binary  ──┐
   (Rust + ONNX runtime)   │
                           ▼
                    ┌──────────────┐
ONNX model file ──▶│ build-rootfs │──▶  rootfs-{kind}-{version}.ext4
                    └──────────────┘
                           ▲
Alpine base + Rust musl ───┘
```

### B.3 Reproducibility guarantees

- Pinned Alpine version (`alpine:3.20` digest).
- Pinned Rust toolchain (`rust:1.85-musl` digest).
- Pinned model files via SHA-256 (frozen at training time).
- Resulting ext4 image is **deterministic** : same inputs → same SHA-256 output.

This lets us audit-trail "what was running" : a Sprint 8 trace records `rootfs_sha256` ; the auditor recomputes from inputs to verify integrity.

---

## Annexe C — Helm chart values

```yaml
# charts/efe-nano-firecracker/values.yaml
image:
  repository: registry.efe.internal/efe-nano
  tag: v0.8.0
  pullPolicy: IfNotPresent

vmBackend: firecracker  # or "stub" for fallback

firecracker:
  binaryPath: /opt/firecracker/firecracker
  kernelImage: /opt/firecracker/vmlinux
  rootfsRegistry: registry.efe.internal/firecracker
  bootBudget: 500ms
  reaperInterval: 10s
  vsockTimeout: 30s
  idleEvictionAfter: 5m

  cgroup:
    root: /sys/fs/cgroup/efe-nano

  apiSocketDir: /var/run/efe-nano/sockets

  kinds:
    ner_clinical:
      modelId: ner_clinical
      modelVersion: 1.0.0
      rootfsTag: "1.0.0"
      minWarm: 20
      maxTotal: 50
      memoryMb: 256
      vcpus: 1
      cgroupCpuQuotaUs: 1000000
      cgroupMemoryHigh: 230686720       # 220 MB soft
      cgroupMemoryMax: 268435456        # 256 MB hard
      cgroupBlkioWeight: 100
    # ... 4 more kinds ...
```

---

## Sign-off

- [ ] Founder review (rodrigue.pekar)
- [ ] Senior platform eng review
- [ ] SRE review (cgroup + Helm + runbook)
- [ ] Security review (threat model + isolation layers)
- [ ] Compliance review (SOC 2 + AI Act mapping)

**Sprint 8 kickoff blocked on** : 5 sign-offs + Sprint 7 N1-N10 merged + M25 sandbox API frozen.

**Sprint 8 success criteria** : 10 goals (G1-G10) all GREEN by day 20.

**Sprint 9 success criteria** : 100 % traffic on Firecracker backend by day 14 ; zero rollback events ; 7-day SLO compliance ≥ 99 %.
