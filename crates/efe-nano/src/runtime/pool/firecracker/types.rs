//! **Sprint 8 — N11** : core types for the Firecracker pool.
//!
//! These types are **internal** to the `firecracker` submodule.  External
//! callers use the parent module's `VmKind`, `VmRequest`, `VmResponse`,
//! `VmLease`, `VmPool` (frozen Sprint 4 trait surface).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::super::VmKind;

// ─────────────────────────────────────────────────────────────────────
//  Identifiers
// ─────────────────────────────────────────────────────────────────────

/// Monotonic identifier of a VM instance within the pool.  Distinct from
/// `Uuid`-based external `vm_id` exposed on `VmLease` — `VmId` is
/// internal-only and tracks pool slots.
///
/// Allocated by [`VmIdAllocator`] at spawn time.  Re-used after the
/// underlying VM dies (the pool guarantees no two LIVE VMs share an id,
/// not that ids are globally unique forever).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct VmId(pub u64);

impl VmId {
    pub fn raw(&self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for VmId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "vm-{}", self.0)
    }
}

/// Monotonic atomic VmId allocator.
#[derive(Debug, Default)]
pub struct VmIdAllocator {
    next: AtomicU64,
}

impl VmIdAllocator {
    pub fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
        }
    }

    pub fn allocate(&self) -> VmId {
        VmId(self.next.fetch_add(1, Ordering::Relaxed))
    }
}

/// **Sprint 8 — N11** : `AF_VSOCK` Context Identifier.
///
/// In Firecracker semantics : guest's CID is host-allocated, must be ≥ 3
/// (1 = local, 2 = host).  Linux exposes `VMADDR_CID_ANY = 0xFFFFFFFF`
/// for the guest to bind.  Each VM gets a unique CID per host so the
/// host can `connect(cid, port)` to reach it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct VsockCid(pub u32);

impl VsockCid {
    /// Lowest valid guest CID per Firecracker (3 + below reserved).
    pub const MIN_GUEST: u32 = 3;
    /// Reserved : host.
    pub const HOST: u32 = 2;

    pub fn raw(&self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for VsockCid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cid-{}", self.0)
    }
}

/// Per-host monotonic CID allocator with wrap-around guard.
///
/// CIDs are 32-bit on Linux.  At 100 spawns/sec we exhaust the space in
/// ~ 1.4 years uptime — long-running operators should rotate the pool
/// (process restart) before then.  The allocator panics on overflow
/// rather than wrapping, because CID reuse must be coupled with
/// confirmed VM death (out of scope for this allocator).
#[derive(Debug)]
pub struct VsockCidAllocator {
    next: AtomicU64,
}

impl VsockCidAllocator {
    pub fn new() -> Self {
        Self {
            next: AtomicU64::new(VsockCid::MIN_GUEST as u64),
        }
    }

    pub fn allocate(&self) -> VsockCid {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        if n > u32::MAX as u64 {
            panic!("VsockCidAllocator exhausted ({n} > u32::MAX) — restart the pool");
        }
        VsockCid(n as u32)
    }
}

impl Default for VsockCidAllocator {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  TenantClass — pool partition key
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : partition class for the pool.
///
/// Pre-Sprint-8 the StubVmPool had one pool per `VmKind`.  Sprint 8
/// adds a second dimension — the **stability tier** of the requesting
/// tenant — so a noisy TIER-1 tenant cannot starve a TIER-AUDIT tenant
/// with strict SLAs.
///
/// 3 classes :
/// - `TierAudit`     : dedicated reservation (guaranteed `min_warm`).
/// - `Tier1`         : fair share, FIFO queueing.
/// - `Experimental`  : best-effort, no SLA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum TenantClass {
    TierAudit,
    Tier1,
    Tier2,
    Experimental,
}

impl TenantClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TierAudit => "tier_audit",
            Self::Tier1 => "tier_1",
            Self::Tier2 => "tier_2",
            Self::Experimental => "experimental",
        }
    }

    /// Derive from the agent's [`efe_nano_sdk::StabilityTier`].
    pub fn from_stability_tier(tier: efe_nano_sdk::StabilityTier) -> Self {
        match tier {
            efe_nano_sdk::StabilityTier::TierAudit => Self::TierAudit,
            efe_nano_sdk::StabilityTier::Tier1 => Self::Tier1,
            efe_nano_sdk::StabilityTier::Tier2 => Self::Tier2,
            efe_nano_sdk::StabilityTier::TierExperimental => Self::Experimental,
        }
    }
}

impl Default for TenantClass {
    fn default() -> Self {
        Self::Tier1
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Model key — the sub-pool partition for Mode B
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : composite key uniquely identifying a (kind,
/// model, tenant_class) sub-pool.  A Firecracker VM is pre-baked with
/// ONE specific model, so different models cannot share a warm slot.
///
/// For `VmKind::LlmCaller`, `model_id` and `model_version` are the
/// sentinel `""` — there's one LlmCaller sub-pool per `tenant_class`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubPoolKey {
    pub kind: VmKind,
    pub model_id: String,
    pub model_version: String,
    pub tenant_class: TenantClass,
}

impl SubPoolKey {
    pub fn mode_b(
        model_id: impl Into<String>,
        model_version: impl Into<String>,
        tenant_class: TenantClass,
    ) -> Self {
        Self {
            kind: VmKind::ModelInference,
            model_id: model_id.into(),
            model_version: model_version.into(),
            tenant_class,
        }
    }

    pub fn mode_c(tenant_class: TenantClass) -> Self {
        Self {
            kind: VmKind::LlmCaller,
            model_id: String::new(),
            model_version: String::new(),
            tenant_class,
        }
    }

    /// Human-readable label for metrics + tracing.
    pub fn label(&self) -> String {
        match self.kind {
            VmKind::ModelInference => format!(
                "model={}@{}:{}",
                self.model_id,
                self.model_version,
                self.tenant_class.as_str()
            ),
            VmKind::LlmCaller => format!("llm_caller:{}", self.tenant_class.as_str()),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  VmState — the lifecycle FSM
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : VM lifecycle state machine.
///
/// See `EFE_M50_N11_FIRECRACKER_POOL_ARCHITECTURE.md` § 7 for the
/// transition diagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmState {
    /// `spawn()` issued, process forking, kernel booting, daemon coming
    /// up.  Cannot be acquired.
    Provisioning,

    /// Ready to serve : in `KindPool.warm`.
    Warm,

    /// Checked out from the warm-set but not yet executing
    /// (between `acquire` and the first `execute` call).
    Reserved,

    /// Vsock RPC in flight.
    InUse,

    /// `execute` returned, about to checkin back to `warm`.
    Returning,

    /// Marked for destruction (reaper-claimed).  Cannot be re-acquired.
    Reaping,

    /// Process exited.  Pending cleanup ; not present in any pool.
    Dead,
}

impl VmState {
    /// True if a reaper should consider this VM dead.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Dead)
    }

    /// True if `acquire` is allowed to take this VM.
    pub fn is_warm(self) -> bool {
        matches!(self, Self::Warm)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  WarmVm — the runtime state of one pre-warmed VM
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : one VM instance + its runtime metadata.
///
/// Held by `KindPool.warm` (when idle) or `KindPool.in_use` (when leased).
/// State transitions go through `state.store(...)` (atomic, lock-free).
#[derive(Debug)]
pub struct WarmVm {
    pub id: VmId,
    pub key: SubPoolKey,
    pub cid: VsockCid,
    /// PID of the host-side Firecracker process.  `0` for MockLifecycle.
    pub firecracker_pid: u32,
    pub created_at: Instant,
    /// Last `acquire` time — used for LRU eviction by the reaper.
    pub last_used_at: std::sync::Mutex<Instant>,
    /// Lock-free state cell.  Held in a `Mutex` (cheap : critical section
    /// is 1 instruction) rather than `AtomicU8` to keep `VmState` extensible
    /// to non-`Copy` variants in Phase 4 (e.g. carrying error reasons).
    pub state: std::sync::Mutex<VmState>,
    /// Lifecycle handle — opaque token returned by `Lifecycle::spawn`,
    /// consumed by `Lifecycle::reap`.  In `MockLifecycle` this is a
    /// numeric tag ; in `FirecrackerLifecycle` it carries the
    /// Firecracker API socket path + child process handle.
    pub lifecycle_handle: LifecycleHandle,
}

impl WarmVm {
    pub fn age(&self) -> Duration {
        self.created_at.elapsed()
    }

    pub fn idle_for(&self) -> Duration {
        self.last_used_at
            .lock()
            .map(|t| t.elapsed())
            .unwrap_or_default()
    }

    pub fn touch(&self) {
        if let Ok(mut t) = self.last_used_at.lock() {
            *t = Instant::now();
        }
    }
}

/// **Sprint 8 — N11** : opaque handle owned by the [`super::lifecycle::Lifecycle`]
/// impl.  Encapsulates "how to reap this specific VM" — could be a
/// process handle, a socket path, a mock tag.
#[derive(Debug)]
pub enum LifecycleHandle {
    /// Mock variant : carries a numeric tag so the test harness can
    /// correlate spawn → reap.
    Mock { tag: u64 },
    /// Real Firecracker variant (Linux only).  Holds the API socket
    /// path + the JoinHandle of the watchdog task that owns the child.
    #[cfg(target_os = "linux")]
    Firecracker {
        api_socket: std::path::PathBuf,
        pid: u32,
    },
    /// Cross-platform inert variant — used by the macOS dev path where
    /// FirecrackerLifecycle is compile-only.
    #[cfg(not(target_os = "linux"))]
    Firecracker { _phantom: () },
}

// ─────────────────────────────────────────────────────────────────────
//  Outcome — what happened to a checked-out VM
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : the outcome of an `execute()` on a leased VM.
/// Determines whether the VM returns to the warm-set or gets reaped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionOutcome {
    /// Inference succeeded ; VM is healthy.  Return to warm-set.
    Success,
    /// Inference failed with a recoverable error (input schema,
    /// deadline within the VM's tolerance) ; VM is still healthy.
    /// Return to warm-set.
    SoftError,
    /// Vsock crashed, daemon exited, or model OOM'd.  Reap the VM.
    HardError,
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_id_allocator_monotonic() {
        let a = VmIdAllocator::new();
        let id1 = a.allocate();
        let id2 = a.allocate();
        let id3 = a.allocate();
        assert_eq!(id1, VmId(1));
        assert_eq!(id2, VmId(2));
        assert_eq!(id3, VmId(3));
    }

    #[test]
    fn vm_id_display() {
        assert_eq!(VmId(42).to_string(), "vm-42");
    }

    #[test]
    fn vsock_cid_allocator_starts_at_3() {
        let a = VsockCidAllocator::new();
        assert_eq!(a.allocate(), VsockCid(3));
        assert_eq!(a.allocate(), VsockCid(4));
    }

    #[test]
    fn vsock_cid_display() {
        assert_eq!(VsockCid(42).to_string(), "cid-42");
    }

    #[test]
    fn tenant_class_from_stability_tier_maps_correctly() {
        use efe_nano_sdk::StabilityTier;
        assert_eq!(
            TenantClass::from_stability_tier(StabilityTier::TierAudit),
            TenantClass::TierAudit
        );
        assert_eq!(
            TenantClass::from_stability_tier(StabilityTier::Tier1),
            TenantClass::Tier1
        );
        assert_eq!(
            TenantClass::from_stability_tier(StabilityTier::TierExperimental),
            TenantClass::Experimental
        );
        assert_eq!(
            TenantClass::from_stability_tier(StabilityTier::Tier2),
            TenantClass::Tier2
        );
    }

    #[test]
    fn tenant_class_default_is_tier1() {
        assert_eq!(TenantClass::default(), TenantClass::Tier1);
    }

    #[test]
    fn subpool_key_mode_b_label() {
        let k = SubPoolKey::mode_b("ner_clinical", "1.0.0", TenantClass::TierAudit);
        assert_eq!(k.label(), "model=ner_clinical@1.0.0:tier_audit");
    }

    #[test]
    fn subpool_key_mode_c_label() {
        let k = SubPoolKey::mode_c(TenantClass::Tier1);
        assert_eq!(k.label(), "llm_caller:tier_1");
    }

    #[test]
    fn subpool_key_eq_and_hash() {
        use std::collections::HashSet;
        let a = SubPoolKey::mode_b("ner_clinical", "1.0.0", TenantClass::Tier1);
        let b = SubPoolKey::mode_b("ner_clinical", "1.0.0", TenantClass::Tier1);
        let c = SubPoolKey::mode_b("ner_clinical", "1.0.1", TenantClass::Tier1);
        assert_eq!(a, b);
        assert_ne!(a, c);
        let mut s = HashSet::new();
        s.insert(a.clone());
        s.insert(b);
        s.insert(c);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn vm_state_transitions() {
        assert!(VmState::Warm.is_warm());
        assert!(!VmState::Reserved.is_warm());
        assert!(VmState::Dead.is_terminal());
        assert!(!VmState::Warm.is_terminal());
    }

    #[test]
    fn execution_outcome_variants_distinct() {
        assert_ne!(ExecutionOutcome::Success, ExecutionOutcome::SoftError);
        assert_ne!(ExecutionOutcome::SoftError, ExecutionOutcome::HardError);
    }

    #[test]
    fn concurrent_vm_id_allocation_is_safe() {
        use std::sync::Arc;
        use std::thread;
        let alloc = Arc::new(VmIdAllocator::new());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let alloc = Arc::clone(&alloc);
            handles.push(thread::spawn(move || {
                (0..100).map(|_| alloc.allocate().raw()).collect::<Vec<_>>()
            }));
        }
        let mut all_ids: Vec<u64> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        all_ids.sort();
        all_ids.dedup();
        assert_eq!(all_ids.len(), 8 * 100, "no duplicate ids under concurrency");
    }
}
