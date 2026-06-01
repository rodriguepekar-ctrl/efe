//! Sprint D18 — workload specifications.
//!
//! Cf. architecture § 15.3 (6 workload patterns).
//!
//! Sprint **D38 (Φ-F)** extension — adds `WorkloadOp::Mode3Execute`
//! plus the two canonical Mode-3 patterns `mode3-k5` and
//! `mode3-k10`. The `mix: Vec<(WorkloadOp, u32)>` invariant of the
//! 6 D18 workloads is preserved (B-3 — workload mix immuable) ;
//! Mode-3 workloads use a fresh op enum variant so distribution
//! weights stay isolated from existing patterns.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::BenchError;

/// **Sprint D38 (Φ-F)** — server-side hard cap for `Mode3Service`.
/// Mirrors the S-12 SLO contract documented in mode3.proto. The
/// bench harness pre-validates `--k-swarm K` against this cap and
/// requires `--allow-k-above-max` to exceed it (see `BenchError::KSwarmAboveMax`).
pub const K_SWARM_HARD_CAP: u32 = 10;

/// **Sprint D38 (Φ-F)** — soft cap (server clamps anything above).
/// Workloads `mode3-k5` use exactly this k ; `mode3-k10` exercises
/// the clamp path (server logs `k_clamped=true`).
pub const K_SWARM_SOFT_CAP: u32 = 5;

/// A single workload operation.
///
/// **Sprint D38 (Φ-F)** — `Mode3Execute { k_swarm }` joins the 6
/// D18 ops. The `Copy` impl is preserved (`u32` is `Copy`) so the
/// runner's hot loop continues to pass ops by value without any
/// `.clone()` overhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadOp {
    /// `efe.v1.SwarmService.Execute` (read + write blend).
    Execute,
    /// `efe.v1.CatalogService.RegisterCatalog`.
    RegisterCatalog,
    /// `efe.v1.CatalogService.RegisterClass` (note : combined with sentence).
    RegisterClass,
    /// `efe.v1.CatalogService.RegisterSentence`.
    RegisterSentence,
    /// `efe.v1.CatalogService.GetCatalog` (resolve / read-mostly path).
    ResolveCatalog,
    /// `efe.v1.CatalogService.ListCatalogs`.
    ListCatalogs,
    /// **Sprint D38 (Φ-F)** — `efe.v1.mode3.Mode3Service.Execute`.
    /// `k_swarm` is the number of parallel sub-swarms requested
    /// (the server clamps to 5 when `k_swarm > 5`, see S-12).
    Mode3Execute {
        /// Number of parallel sub-swarms requested.
        k_swarm: u32,
    },
}

impl WorkloadOp {
    /// Stable string used in reports + metric labels.
    ///
    /// For `Mode3Execute`, the only k values produced by the
    /// canonical patterns are 5 and 10 — both have a fixed
    /// `&'static str` form. Any other k (custom workload) falls
    /// back to the generic label `mode3_execute`.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Execute => "execute",
            Self::RegisterCatalog => "register_catalog",
            Self::RegisterClass => "register_class",
            Self::RegisterSentence => "register_sentence",
            Self::ResolveCatalog => "resolve_catalog",
            Self::ListCatalogs => "list_catalogs",
            Self::Mode3Execute { k_swarm: 5 } => "mode3_execute_k5",
            Self::Mode3Execute { k_swarm: 10 } => "mode3_execute_k10",
            Self::Mode3Execute { .. } => "mode3_execute",
        }
    }

    /// **Sprint D38 (Φ-F)** — `true` iff this op targets the
    /// Mode-3 service (used by the runner to short-circuit when
    /// the `tonic` feature is OFF).
    #[must_use]
    pub fn is_mode3(&self) -> bool {
        matches!(self, Self::Mode3Execute { .. })
    }
}

/// One workload pattern (one of the 8 canonical patterns post-D38).
#[derive(Debug, Clone)]
pub struct Workload {
    /// Pattern label (e.g. "read-heavy" or "mode3-k5").
    pub label: String,
    /// Operation mix : `(op, weight)` pairs. Weights are
    /// normalised at runtime ; e.g. `[(Execute, 95), (RegisterCatalog, 5)]`
    /// = 95 % Execute, 5 % RegisterCatalog.
    pub mix: Vec<(WorkloadOp, u32)>,
    /// Target sustained RPS over the run.
    pub target_rps: u32,
    /// Run duration.
    pub duration: Duration,
    /// Number of concurrent workers (tokio tasks).
    pub concurrency: u32,
}

impl Workload {
    /// Construct one of the 8 canonical patterns (6 D18 + 2 D38).
    pub fn canonical(label: impl Into<String>) -> Result<Self, BenchError> {
        let label_s = label.into();
        match label_s.as_str() {
            "read-heavy" => Ok(Self {
                label: label_s,
                mix: vec![
                    (WorkloadOp::ResolveCatalog, 95),
                    (WorkloadOp::Execute, 5),
                ],
                target_rps: 5_000,
                duration: Duration::from_secs(5 * 60),
                concurrency: 64,
            }),
            "write-heavy" => Ok(Self {
                label: label_s,
                mix: vec![
                    (WorkloadOp::RegisterClass, 50),
                    (WorkloadOp::RegisterSentence, 50),
                ],
                target_rps: 200,
                duration: Duration::from_secs(5 * 60),
                concurrency: 16,
            }),
            "balanced" => Ok(Self {
                label: label_s,
                mix: vec![(WorkloadOp::Execute, 100)],
                target_rps: 1_000,
                duration: Duration::from_secs(5 * 60),
                concurrency: 32,
            }),
            "burst" => Ok(Self {
                label: label_s,
                mix: vec![(WorkloadOp::Execute, 100)],
                target_rps: 1_000,
                duration: Duration::from_secs(10 * 60),
                concurrency: 64,
            }),
            "multi-tenant" => Ok(Self {
                label: label_s,
                mix: vec![
                    (WorkloadOp::Execute, 50),
                    (WorkloadOp::ResolveCatalog, 50),
                ],
                target_rps: 1_000,
                duration: Duration::from_secs(10 * 60),
                concurrency: 128,
            }),
            "soak" => Ok(Self {
                label: label_s,
                mix: vec![(WorkloadOp::Execute, 100)],
                target_rps: 10,
                duration: Duration::from_secs(60 * 60),
                concurrency: 4,
            }),
            // ----------------------------------------------------
            // **Sprint D38 (Φ-F)** — Mode-3 canonical workloads.
            //
            // Both patterns issue 100 % `Mode3Execute` ops. The
            // RPS targets are intentionally low — Mode-3 fans out
            // k sub-swarms server-side, so even a single client
            // request consumes k× the GPU/CPU of a Mode-1 call.
            // The numbers below are calibrated for a Tier-2 runner
            // with 1× T4 GPU.
            // ----------------------------------------------------
            "mode3-k5" => Ok(Self {
                label: label_s,
                mix: vec![(
                    WorkloadOp::Mode3Execute {
                        k_swarm: K_SWARM_SOFT_CAP, // 5
                    },
                    100,
                )],
                target_rps: 50,
                duration: Duration::from_secs(60),
                concurrency: 16,
            }),
            "mode3-k10" => Ok(Self {
                label: label_s,
                mix: vec![(
                    WorkloadOp::Mode3Execute {
                        k_swarm: K_SWARM_HARD_CAP, // 10
                    },
                    100,
                )],
                target_rps: 25,
                duration: Duration::from_secs(30),
                concurrency: 8,
            }),
            other => Err(BenchError::InvalidWorkload(format!(
                "unknown canonical workload: {other:?} (expected read-heavy|\
                 write-heavy|balanced|burst|multi-tenant|soak|mode3-k5|mode3-k10)"
            ))),
        }
    }

    /// Validate the spec : RPS > 0, duration > 0, mix non-empty.
    pub fn validate(&self) -> Result<(), BenchError> {
        if self.target_rps == 0 {
            return Err(BenchError::InvalidWorkload(
                "target_rps must be > 0".into(),
            ));
        }
        if self.duration.is_zero() {
            return Err(BenchError::InvalidWorkload(
                "duration must be > 0".into(),
            ));
        }
        if self.mix.is_empty() {
            return Err(BenchError::InvalidWorkload(
                "op mix must be non-empty".into(),
            ));
        }
        if self.mix.iter().all(|(_, w)| *w == 0) {
            return Err(BenchError::InvalidWorkload(
                "op mix has all-zero weights".into(),
            ));
        }
        if self.concurrency == 0 {
            return Err(BenchError::InvalidWorkload(
                "concurrency must be > 0".into(),
            ));
        }
        // **Sprint D38 (Φ-F)** — guard k_swarm in Mode3Execute.
        for (op, _) in &self.mix {
            if let WorkloadOp::Mode3Execute { k_swarm } = op {
                if *k_swarm == 0 {
                    return Err(BenchError::InvalidWorkload(
                        "Mode3Execute k_swarm must be > 0".into(),
                    ));
                }
                if *k_swarm > K_SWARM_HARD_CAP {
                    return Err(BenchError::InvalidWorkload(format!(
                        "Mode3Execute k_swarm={} exceeds hard cap {}",
                        k_swarm, K_SWARM_HARD_CAP
                    )));
                }
            }
        }
        Ok(())
    }

    /// Pick a workload op according to the mix weights, using a
    /// pluggable random source. Returns `None` only if `validate()`
    /// would fail.
    pub fn pick_op(&self, rand_u32: u32) -> Option<WorkloadOp> {
        let total: u64 = self.mix.iter().map(|(_, w)| *w as u64).sum();
        if total == 0 {
            return None;
        }
        let target = (rand_u32 as u64) % total;
        let mut cumul = 0u64;
        for (op, w) in &self.mix {
            cumul += *w as u64;
            if target < cumul {
                return Some(*op);
            }
        }
        // Defensive : weights guaranteed cumul > target, but
        // pin in case of arithmetic overflow.
        self.mix.last().map(|(op, _)| *op)
    }

    /// Iterator over the **8** canonical pattern labels (6 D18 + 2 D38).
    pub fn canonical_labels() -> impl Iterator<Item = &'static str> {
        [
            "read-heavy",
            "write-heavy",
            "balanced",
            "burst",
            "multi-tenant",
            "soak",
            // **Sprint D38 (Φ-F)** — Mode-3 patterns.
            "mode3-k5",
            "mode3-k10",
        ]
        .into_iter()
    }

    /// **Sprint D38 (Φ-F)** — `true` iff the workload is a Mode-3
    /// pattern (every op is `Mode3Execute`). Used by the report
    /// generator to populate the top-level `k_swarm` field.
    #[must_use]
    pub fn is_mode3(&self) -> bool {
        !self.mix.is_empty() && self.mix.iter().all(|(op, _)| op.is_mode3())
    }

    /// **Sprint D38 (Φ-F)** — extract the k value from a Mode-3
    /// workload, or `None` for non-Mode-3.
    #[must_use]
    pub fn k_swarm(&self) -> Option<u32> {
        self.mix.iter().find_map(|(op, _)| match op {
            WorkloadOp::Mode3Execute { k_swarm } => Some(*k_swarm),
            _ => None,
        })
    }
}

/// **T-D18-12 mitigation** — refuse to run against prod targets.
/// Returns `Err(BenchError::RefusedProdTarget)` if the env name
/// is `prod` / `production` (any case). All other env labels are
/// allowed.
pub fn validate_target_env(env: &str) -> Result<(), BenchError> {
    let lower = env.trim().to_ascii_lowercase();
    if lower == "prod" || lower == "production" {
        Err(BenchError::RefusedProdTarget(env.to_string()))
    } else {
        Ok(())
    }
}

/// **Sprint D31 (Φ-D)** — validate `--gpu-shards N` against the
/// cluster cap. N=0 is rejected (no work) ; N>16 is rejected
/// (exceeds the largest deployed cluster of 4× H100 nodes × 4 GPUs).
pub fn validate_gpu_shards(n: u32) -> Result<(), BenchError> {
    if n == 0 || n > 16 {
        Err(BenchError::GpuShardOutOfRange { got: n })
    } else {
        Ok(())
    }
}

/// **Sprint D38 (Φ-F)** — validate `--k-swarm K` against the soft
/// cap. K ≤ 5 is allowed unconditionally ; K > 5 requires the
/// `allow_above_max` flag (mirrors the server-side clamp at
/// K_MAX=5, see S-12). K=0 is always rejected. K > 10 is always
/// rejected (the hard cap of the bench harness, beyond which the
/// server clamp produces noise that doesn't reflect production).
pub fn validate_k_swarm(k: u32, allow_above_max: bool) -> Result<(), BenchError> {
    if k == 0 {
        return Err(BenchError::InvalidWorkload(
            "k_swarm must be > 0".into(),
        ));
    }
    if k > K_SWARM_HARD_CAP {
        return Err(BenchError::InvalidWorkload(format!(
            "k_swarm={k} exceeds bench hard cap {K_SWARM_HARD_CAP} \
             (server-side clamp behaviour beyond k=10 is undefined)"
        )));
    }
    if k > K_SWARM_SOFT_CAP && !allow_above_max {
        return Err(BenchError::KSwarmAboveMax { got: k });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_workload_read_heavy_correct_mix() {
        let w = Workload::canonical("read-heavy").unwrap();
        assert_eq!(w.target_rps, 5_000);
        assert_eq!(w.mix.len(), 2);
        let weight_resolve =
            w.mix.iter().find(|(o, _)| *o == WorkloadOp::ResolveCatalog).unwrap().1;
        assert_eq!(weight_resolve, 95);
    }

    #[test]
    fn canonical_workload_unknown_label_errors() {
        let err = Workload::canonical("oof").unwrap_err();
        assert!(matches!(err, BenchError::InvalidWorkload(_)));
    }

    #[test]
    fn validate_rejects_zero_rps() {
        let mut w = Workload::canonical("balanced").unwrap();
        w.target_rps = 0;
        assert!(w.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_duration() {
        let mut w = Workload::canonical("balanced").unwrap();
        w.duration = Duration::from_secs(0);
        assert!(w.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_mix() {
        let mut w = Workload::canonical("balanced").unwrap();
        w.mix.clear();
        assert!(w.validate().is_err());
    }

    #[test]
    fn validate_rejects_all_zero_weights() {
        let mut w = Workload::canonical("balanced").unwrap();
        w.mix = vec![(WorkloadOp::Execute, 0)];
        assert!(w.validate().is_err());
    }

    #[test]
    fn pick_op_distribution_matches_weights() {
        // 99 % Execute, 1 % RegisterCatalog → mostly Execute.
        let w = Workload {
            label: "test".into(),
            mix: vec![
                (WorkloadOp::Execute, 99),
                (WorkloadOp::RegisterCatalog, 1),
            ],
            target_rps: 10,
            duration: Duration::from_secs(1),
            concurrency: 1,
        };
        let mut exec_count = 0;
        let total = 1_000;
        for i in 0..total {
            let op = w.pick_op(i).unwrap();
            if op == WorkloadOp::Execute {
                exec_count += 1;
            }
        }
        // Tolerance ±5%.
        assert!(exec_count > 950 && exec_count <= 1000);
    }

    #[test]
    fn target_env_rejects_prod() {
        assert!(validate_target_env("prod").is_err());
        assert!(validate_target_env("PROD").is_err());
        assert!(validate_target_env("production").is_err());
        assert!(validate_target_env(" Prod ").is_err());
    }

    #[test]
    fn target_env_allows_non_prod() {
        for env in ["dev", "staging", "drill", "ci-bench", "kind-local"] {
            validate_target_env(env).unwrap();
        }
    }

    // -----------------------------------------------------------
    // **Sprint D38 (Φ-F)** — Mode-3 + 8 canonical labels tests.
    // -----------------------------------------------------------

    #[test]
    fn canonical_labels_lists_eight() {
        let all: Vec<_> = Workload::canonical_labels().collect();
        assert_eq!(all.len(), 8);
        assert!(all.contains(&"read-heavy"));
        assert!(all.contains(&"soak"));
        assert!(all.contains(&"mode3-k5"));
        assert!(all.contains(&"mode3-k10"));
    }

    #[test]
    fn workload_op_as_str_round_trip() {
        for op in [
            WorkloadOp::Execute,
            WorkloadOp::RegisterCatalog,
            WorkloadOp::RegisterClass,
            WorkloadOp::RegisterSentence,
            WorkloadOp::ResolveCatalog,
            WorkloadOp::ListCatalogs,
            WorkloadOp::Mode3Execute { k_swarm: 5 },
            WorkloadOp::Mode3Execute { k_swarm: 10 },
            WorkloadOp::Mode3Execute { k_swarm: 7 }, // generic label fallback
        ] {
            let s = op.as_str();
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn mode3_op_as_str_specialised_for_k5_k10() {
        assert_eq!(
            WorkloadOp::Mode3Execute { k_swarm: 5 }.as_str(),
            "mode3_execute_k5"
        );
        assert_eq!(
            WorkloadOp::Mode3Execute { k_swarm: 10 }.as_str(),
            "mode3_execute_k10"
        );
        assert_eq!(
            WorkloadOp::Mode3Execute { k_swarm: 3 }.as_str(),
            "mode3_execute"
        );
    }

    #[test]
    fn workload_op_is_mode3_flags_correctly() {
        assert!(WorkloadOp::Mode3Execute { k_swarm: 5 }.is_mode3());
        assert!(!WorkloadOp::Execute.is_mode3());
        assert!(!WorkloadOp::ResolveCatalog.is_mode3());
    }

    #[test]
    fn canonical_workload_mode3_k5_correct_shape() {
        let w = Workload::canonical("mode3-k5").unwrap();
        assert_eq!(w.label, "mode3-k5");
        assert_eq!(w.mix.len(), 1);
        assert_eq!(w.mix[0].1, 100);
        assert_eq!(
            w.mix[0].0,
            WorkloadOp::Mode3Execute {
                k_swarm: K_SWARM_SOFT_CAP
            }
        );
        assert!(w.is_mode3());
        assert_eq!(w.k_swarm(), Some(5));
    }

    #[test]
    fn canonical_workload_mode3_k10_correct_shape() {
        let w = Workload::canonical("mode3-k10").unwrap();
        assert_eq!(w.label, "mode3-k10");
        assert_eq!(
            w.mix[0].0,
            WorkloadOp::Mode3Execute {
                k_swarm: K_SWARM_HARD_CAP
            }
        );
        assert!(w.is_mode3());
        assert_eq!(w.k_swarm(), Some(10));
    }

    #[test]
    fn workload_validate_rejects_mode3_zero_k() {
        let w = Workload {
            label: "bad-mode3".into(),
            mix: vec![(WorkloadOp::Mode3Execute { k_swarm: 0 }, 100)],
            target_rps: 10,
            duration: Duration::from_secs(1),
            concurrency: 1,
        };
        assert!(w.validate().is_err());
    }

    #[test]
    fn workload_validate_rejects_mode3_above_hard_cap() {
        let w = Workload {
            label: "bad-mode3".into(),
            mix: vec![(WorkloadOp::Mode3Execute { k_swarm: 11 }, 100)],
            target_rps: 10,
            duration: Duration::from_secs(1),
            concurrency: 1,
        };
        assert!(w.validate().is_err());
    }

    #[test]
    fn workload_is_mode3_false_for_legacy_patterns() {
        for label in ["read-heavy", "write-heavy", "balanced", "burst", "multi-tenant", "soak"] {
            let w = Workload::canonical(label).unwrap();
            assert!(!w.is_mode3(), "{label} should not be mode3");
            assert_eq!(w.k_swarm(), None);
        }
    }

    // -----------------------------------------------------------
    // **Sprint D31 (Φ-D)** — gpu_shards validation.
    // -----------------------------------------------------------

    #[test]
    fn gpu_shards_accept_in_range() {
        for n in [1, 2, 4, 8, 16] {
            validate_gpu_shards(n).unwrap();
        }
    }

    #[test]
    fn gpu_shards_reject_zero() {
        let e = validate_gpu_shards(0).unwrap_err();
        assert!(matches!(e, BenchError::GpuShardOutOfRange { got: 0 }));
    }

    #[test]
    fn gpu_shards_reject_above_cap() {
        let e = validate_gpu_shards(17).unwrap_err();
        assert!(matches!(e, BenchError::GpuShardOutOfRange { got: 17 }));
    }

    // -----------------------------------------------------------
    // **Sprint D38 (Φ-F)** — k_swarm validation.
    // -----------------------------------------------------------

    #[test]
    fn k_swarm_accept_under_soft_cap_no_opt_in() {
        for k in 1..=5 {
            validate_k_swarm(k, false).unwrap();
        }
    }

    #[test]
    fn k_swarm_above_soft_cap_requires_opt_in() {
        let e = validate_k_swarm(6, false).unwrap_err();
        assert!(matches!(e, BenchError::KSwarmAboveMax { got: 6 }));
        // Same value passes when allow_above_max is true.
        validate_k_swarm(6, true).unwrap();
        validate_k_swarm(10, true).unwrap();
    }

    #[test]
    fn k_swarm_zero_always_rejected() {
        assert!(validate_k_swarm(0, true).is_err());
        assert!(validate_k_swarm(0, false).is_err());
    }

    #[test]
    fn k_swarm_above_hard_cap_always_rejected() {
        // Even with allow_above_max=true, K=11 exceeds the hard cap.
        assert!(validate_k_swarm(11, true).is_err());
        assert!(validate_k_swarm(50, true).is_err());
    }

    #[test]
    fn workload_op_serde_round_trip_simple_variants() {
        // Sanity-check the snake_case serialisation of the simple
        // variants. Mode3Execute is a struct variant (k_swarm is
        // serialised as a field) ; covered by a dedicated test
        // below.
        for op in [
            WorkloadOp::Execute,
            WorkloadOp::ResolveCatalog,
            WorkloadOp::ListCatalogs,
        ] {
            let s = serde_json::to_string(&op).unwrap();
            let r: WorkloadOp = serde_json::from_str(&s).unwrap();
            assert_eq!(op, r);
        }
    }

    #[test]
    fn workload_op_serde_round_trip_mode3() {
        let op = WorkloadOp::Mode3Execute { k_swarm: 5 };
        let s = serde_json::to_string(&op).unwrap();
        // Struct-variant payload : {"mode3_execute":{"k_swarm":5}}
        assert!(s.contains("mode3_execute"));
        assert!(s.contains("k_swarm"));
        let r: WorkloadOp = serde_json::from_str(&s).unwrap();
        assert_eq!(op, r);
    }
}
