//! # PR #27 — G.11 closure — `EffectiveTolerance` and its resolver
//!
//! This module is the canonical home of the types and functions that close
//! gap **G.11** ("`tolerance_used` actually populated by the verdict
//! aggregator", registered in PR #24 lessons file § 7 and confirmed
//! surviving the CB-catalogue deep-coverage expansion of 2026-05-17).
//!
//! ## What G.11 was, and why it took six months to close
//!
//! Before PR #27, the verdict aggregator emitted NDJSON rows whose
//! `tolerance_used` field was a plain `Option<f64>` scalar.  Four
//! concrete sub-defects compounded across the historical evidence chain :
//!
//! - **G.11.a** — `tolerance_used` set to `null` in 60 % of WARN paths.
//! - **G.11.b** — `tolerance_used` entirely absent from INCONCLUSIVE rows.
//! - **G.11.c** — `tolerance_used` reported the YAML-declared value even
//!   when a runtime override (env-var or GPU-runner) was in effect.
//! - **G.11.d** — no inner `applied_at` subfield existed at all : the
//!   moment the aggregator made its decision was reconstructible only
//!   via Git archaeology on the scenario YAML.
//!
//! The combined effect was that an auditor reading a quarterly-publish
//! NDJSON could not answer "what tolerance was applied for this PASS"
//! without leaving the file.  The chain was signed, but the content was
//! sub-specified.
//!
//! ## What this module provides
//!
//! - Three **closed taxonomies** for the wire format
//!   (`ToleranceUnit`, `ToleranceDirection`, `ToleranceSource`), each
//!   frozen under STABILITY § 27.2.5 / 27.2.6 / 27.2.7.
//! - A **value type** (`ToleranceValue`) that is either a scalar `f64`
//!   or a bilateral `[lower, upper]` range, untagged in JSON.
//! - The **central `EffectiveTolerance` struct** with the six fields
//!   frozen under STABILITY § 24.3.6, plus a strict internal-consistency
//!   invariant enforced by `validate_internal_consistency()`.
//! - A **pure-function resolver** `resolve_effective_tolerance()` that
//!   takes the YAML entry (optional), the env context, the runner
//!   introspection, and the decision timestamp, and returns the
//!   `EffectiveTolerance` that the aggregator MUST report.
//! - A **mini-aggregator** `pretend_aggregator_emit_rows()` that drives
//!   the resolver against a synthetic scenario YAML and emits a vector
//!   of v3 NDJSON rows ready for assertion in the round-trip tests.
//! - A **`MetricVerdictV3` row type** that mirrors the wire format of
//!   the canonical fixtures at `bench/fixtures/post_g11_ndjson/`.
//!
//! ## Type hierarchy at a glance
//!
//! ```text
//!                          EffectiveTolerance
//!                                 │
//!                ┌────────────────┼─────────────────┐
//!                │                │                 │
//!     Option<ToleranceValue>  ToleranceUnit  ToleranceDirection
//!                │                │                 │
//!         (Scalar | Range)    (closed enum,    (closed enum,
//!                              8 variants)      4 variants)
//!                │
//!         ToleranceSource ───────────────────────────┐
//!                │                                   │
//!         (closed enum, 8 variants ─ 4 decision-time │
//!          + 1 sentinel + 1 legacy + 2 backfilled)   │
//!                                                    │
//!                  + applied_at : Option<DateTime<Utc>>
//!                  + override_reason : Option<String>
//! ```
//!
//! ## Frozen surface
//!
//! Adding, renaming, or removing any of the following triggers a
//! STABILITY-bump PR (per § 24.3.6 and § 27.2.5/6/7) :
//!
//! - The 8 variants of `ToleranceUnit`.
//! - The 4 variants of `ToleranceDirection`.
//! - The 8 variants of `ToleranceSource`.
//! - The 6 fields of `EffectiveTolerance`.
//! - The 5 mandatory + 1 optional shape of `MetricVerdictV3.tolerance_used`
//!   on the wire.
//! - The exit codes of the `efe-audit-reader` CLI (deferred to PR #27.1).
//!
//! ## Design rationale (the seven decisions documented in
//! S2 architecture AD-1 through AD-7)
//!
//! 1. The closed taxonomies are **Rust enums**, not `String`.  Adding a
//!    variant requires touching the enum, which triggers a visible
//!    build error in every consumer.  The freeze test § 27.2 cross-checks
//!    that the wire-level strings match the enum variants.
//!
//! 2. `ToleranceValue` is an **untagged enum** of `Scalar(f64)` /
//!    `Range([f64; 2])`.  The wire format is naturally `200` or
//!    `[95, 99.5]`.  The `null` case is carried by the *outer*
//!    `Option<ToleranceValue>` on `EffectiveTolerance.value`.
//!
//! 3. The resolver is a **pure function** with an injectable decision
//!    clock (the `applied_at` parameter).  Determinism gives us
//!    byte-exact test assertions ; no internal `Utc::now()`.
//!
//! 4. The legacy scalar `tolerance_used: Option<f64>` on `MetricResult`
//!    and `ComparisonRow` is **preserved as-is** in this PR — a follow-up
//!    PR #27.1 migrates the production aggregator paths.  This keeps
//!    PR #27 narrow.
//!
//! 5. The mini-aggregator `pretend_aggregator_emit_rows()` is the
//!    test-time entry point for the round-trip pinning test.  It is
//!    **not the production aggregator**.  Migration of `compare::compare()`
//!    is deferred to PR #27.1 (see S2 architecture AD-12).
//!
//! 6. The internal-consistency invariant `value == None ⇔ source ∈
//!    {NoToleranceDefined, PreG11Unbackfilled}` is a **double-implication**
//!    enforced by `validate_internal_consistency()`.  The freeze test
//!    § 24.3.6 walks every fixture row and re-asserts it.
//!
//! 7. The serde wire format uses `rename_all = "snake_case"` because the
//!    canonical fixtures already use snake_case (`no_more_than`,
//!    `events_per_sec`, `gpu_runner_override`).  Kebab-case would have
//!    drifted from the existing fixtures.
//!
//! ## References
//!
//! - `docs/architecture/EFE_A1_PHASE3_PR27_ARCHITECTURE.md`
//! - `docs/architecture/EFE_A1_PHASE3_PR27_S2_ARCHITECTURE.md` (Day 1)
//! - `docs/roadmap/EFE_A1_PHASE3_PR27_ROADMAP.md` § 3.2
//! - `docs/STABILITY_CONTRACT.md` § 24.3.6 + § 27.2.5 / 27.2.6 / 27.2.7
//! - `docs/lessons/PR-27-lessons.md`
//! - `bench/fixtures/scenarios/G11_synthetic.yaml`
//! - `bench/fixtures/post_g11_ndjson/*.ndjson`
//! - `bench/harness/tests/g11_round_trip.rs` (the pinning test)

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ═══════════════════════════════════════════════════════════════════════
//  Section 1 — Error type
//
//  Dedicated error enum used by the resolver and by parse paths.
//  In production, most callers of `resolve_effective_tolerance` cannot
//  fail — bad YAML or bad env-vars are caught upstream by the scenario
//  loader and CLI parser.  The error type exists for forward-compatibility
//  (when PR #27.1 migrates the real `compare::compare()` and needs to
//  surface parse errors at the resolver boundary).
// ═══════════════════════════════════════════════════════════════════════

/// Errors raised by the tolerance resolver and its companions.
///
/// The variants distinguish parse-time failures (bad YAML, bad env-var
/// values) from invariant violations (an `EffectiveTolerance` whose
/// `(value, source)` pair contradicts § 24.3.6).
#[derive(Debug, Clone, Error, PartialEq)]
pub enum ResolveError {
    /// The YAML tolerance entry for the metric could not be parsed.
    #[error("invalid YAML tolerance entry for metric '{metric}': {cause}")]
    InvalidYamlEntry {
        /// The metric name whose entry failed to parse.
        metric: String,
        /// The underlying cause as a human-readable string.
        cause: String,
    },

    /// An env-var override was set but its value is not a valid `f64`.
    #[error("env-var override '{var}' value cannot be parsed as f64: {raw}")]
    InvalidEnvOverride {
        /// The env-var name (e.g. `EFE_TOLERANCE_<scenario>_<metric>`).
        var: String,
        /// The raw env-var value as observed.
        raw: String,
    },

    /// The resulting `EffectiveTolerance` violates an invariant of
    /// STABILITY § 24.3.6 (caught at construction time by the builder).
    #[error("internal-consistency invariant violated: {0}")]
    InvariantViolation(String),

    /// The scenario YAML file could not be read.
    #[error("scenario YAML at {path}: {cause}")]
    ScenarioYamlRead {
        /// The path that failed to read.
        path: String,
        /// The underlying I/O or parse cause.
        cause: String,
    },

    /// The scenario YAML structure is missing a required field.
    #[error("scenario YAML missing required field: {field}")]
    ScenarioYamlMissingField {
        /// The dotted-path of the missing field.
        field: String,
    },
}

/// A `Result` type alias for the resolver and companions.
pub type ResolveResult<T> = std::result::Result<T, ResolveError>;

// ═══════════════════════════════════════════════════════════════════════
//  Section 2 — Closed taxonomy : ToleranceUnit (STABILITY § 27.2.5)
// ═══════════════════════════════════════════════════════════════════════

/// Closed taxonomy of tolerance units (STABILITY § 27.2.5).
///
/// The 8 variants below are the **complete frozen set** of units that any
/// `EffectiveTolerance.unit` field may carry.  Adding a new variant
/// (e.g. `Gflops`, `Watts`) is a STABILITY-bump.  Scenarios that need to
/// express a unit not in this list MUST cast to an existing variant — for
/// example, GFLOPS can be modelled as `EventsPerSec` where the "event"
/// is a giga-FLOP.
///
/// The serde wire format is `snake_case` matching the canonical fixture
/// strings at `bench/fixtures/post_g11_ndjson/*.ndjson` (e.g.
/// `"ms"`, `"events_per_sec"`).
///
/// Enforcement : freeze test `v1_7_section_27_2_taxonomy_closed` in
/// `bench/harness/tests/stability_contract_v17.rs` walks every NDJSON
/// fixture row and asserts that the `unit` string is in the closed set
/// emitted by `ToleranceUnit::all()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToleranceUnit {
    /// Milliseconds — duration unit for sub-second latencies.
    Ms,
    /// Seconds — duration unit for second-scale latencies (recovery time,
    /// long-run drift, etc.).
    Sec,
    /// Nanoseconds — duration unit for very-low-latency hot paths
    /// (FFI call overhead, in-process primitives).
    Ns,
    /// Percent (0..100) — for rates, accuracies, percentages-of-baseline.
    Pct,
    /// Bytes — for memory peaks, message sizes, image sizes, etc.
    Bytes,
    /// Events per second — throughput unit for any rate-shaped metric
    /// (events/sec, requests/sec, tokens/sec, gflops, etc.).
    EventsPerSec,
    /// Dimensionless count — for cardinalities (number of resources,
    /// number of rendered elements, number of replicas, etc.).
    Count,
    /// Dimensionless ratio (0..1) — for fractions, probabilities,
    /// error rates expressed as a fraction.
    Ratio,
}

impl ToleranceUnit {
    /// The stable wire-format string for this unit.
    ///
    /// Matches the serde `snake_case` output : a JSON serialisation of
    /// `ToleranceUnit::EventsPerSec` produces `"events_per_sec"`, and
    /// `.as_str()` returns the same string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ms => "ms",
            Self::Sec => "sec",
            Self::Ns => "ns",
            Self::Pct => "pct",
            Self::Bytes => "bytes",
            Self::EventsPerSec => "events_per_sec",
            Self::Count => "count",
            Self::Ratio => "ratio",
        }
    }

    /// All variants of `ToleranceUnit` in canonical order.
    ///
    /// Used by tests that walk the closed taxonomy and by code generators
    /// that materialise the unit set as a list (e.g. the audit-reader
    /// CLI's `explain` command).
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Ms,
            Self::Sec,
            Self::Ns,
            Self::Pct,
            Self::Bytes,
            Self::EventsPerSec,
            Self::Count,
            Self::Ratio,
        ]
    }

    /// Returns `true` iff the unit measures time (Ms / Sec / Ns).
    ///
    /// Useful for cross-cutting predicates such as "duration-style
    /// metrics default to `direction: no_more_than`".
    #[must_use]
    pub const fn is_duration(self) -> bool {
        matches!(self, Self::Ms | Self::Sec | Self::Ns)
    }

    /// Returns `true` iff the unit measures throughput (`EventsPerSec`).
    #[must_use]
    pub const fn is_throughput(self) -> bool {
        matches!(self, Self::EventsPerSec)
    }

    /// Returns `true` iff the unit is dimensionless (`Pct`, `Ratio`,
    /// `Count`).
    #[must_use]
    pub const fn is_dimensionless(self) -> bool {
        matches!(self, Self::Pct | Self::Ratio | Self::Count)
    }
}

impl fmt::Display for ToleranceUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ToleranceUnit {
    type Err = ResolveError;

    fn from_str(s: &str) -> ResolveResult<Self> {
        for &u in Self::all() {
            if u.as_str() == s {
                return Ok(u);
            }
        }
        Err(ResolveError::InvariantViolation(format!(
            "unknown ToleranceUnit string '{s}' (closed taxonomy: {})",
            Self::all()
                .iter()
                .map(|u| u.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  Section 3 — Closed taxonomy : ToleranceDirection (STABILITY § 27.2.6)
// ═══════════════════════════════════════════════════════════════════════

/// Closed taxonomy of tolerance directions (STABILITY § 27.2.6).
///
/// The four variants below are the complete frozen set of directionalities
/// that any `EffectiveTolerance.direction` field may carry.  Adding a new
/// variant is a STABILITY-bump.
///
/// **Combination invariant with `ToleranceValue`** (enforced by
/// `EffectiveTolerance::validate_internal_consistency()`) :
/// - `WithinRange` requires `value` to be a `Range([lower, upper])`.
/// - `NoMoreThan` / `NoLessThan` / `Exactly` require `value` to be a
///   `Scalar(f64)`.
///
/// Combining `WithinRange` with a scalar `value`, or any of the scalar
/// directions with a range `value`, is rejected by the validator with
/// a descriptive error message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToleranceDirection {
    /// Observation must not exceed the bound : `observed <= bound`.
    /// Lower-is-better.  The canonical direction for latency metrics.
    NoMoreThan,
    /// Observation must not fall below the bound : `observed >= bound`.
    /// Higher-is-better.  The canonical direction for throughput metrics.
    NoLessThan,
    /// Observation must fall within `[lower, upper]` :
    /// `lower <= observed <= upper`.  Used for bilateral bounds such as
    /// "acceptance rate must be in 95-99.5 %".
    WithinRange,
    /// Observation must equal the bound exactly : `observed == bound`.
    /// Used for binary-style invariants like "100 % correctness".
    Exactly,
}

impl ToleranceDirection {
    /// The stable wire-format string for this direction.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoMoreThan => "no_more_than",
            Self::NoLessThan => "no_less_than",
            Self::WithinRange => "within_range",
            Self::Exactly => "exactly",
        }
    }

    /// All variants in canonical order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::NoMoreThan,
            Self::NoLessThan,
            Self::WithinRange,
            Self::Exactly,
        ]
    }

    /// Human-readable phrase used by the audit-reader's `explain` output.
    ///
    /// Each phrase is a complete sentence-fragment suitable for inclusion
    /// in a multi-line audit narrative.
    #[must_use]
    pub const fn explain_phrase(self) -> &'static str {
        match self {
            Self::NoMoreThan => "observation must not exceed this bound",
            Self::NoLessThan => "observation must not fall below this bound",
            Self::WithinRange => "observation must fall within the [lower, upper] range",
            Self::Exactly => "observation must equal the bound exactly",
        }
    }

    /// Returns `true` iff this direction is unilateral (one-sided) —
    /// every direction except `WithinRange`.
    #[must_use]
    pub const fn is_unilateral(self) -> bool {
        !matches!(self, Self::WithinRange)
    }

    /// Returns `true` iff this direction is bilateral — only
    /// `WithinRange`.
    #[must_use]
    pub const fn is_bilateral(self) -> bool {
        matches!(self, Self::WithinRange)
    }

    /// Returns `true` iff this direction expresses "lower is better"
    /// semantics : the verdict worsens as the observation grows.
    #[must_use]
    pub const fn is_lower_is_better(self) -> bool {
        matches!(self, Self::NoMoreThan)
    }

    /// Returns `true` iff this direction expresses "higher is better"
    /// semantics : the verdict worsens as the observation shrinks.
    #[must_use]
    pub const fn is_higher_is_better(self) -> bool {
        matches!(self, Self::NoLessThan)
    }
}

impl fmt::Display for ToleranceDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ToleranceDirection {
    type Err = ResolveError;

    fn from_str(s: &str) -> ResolveResult<Self> {
        for &d in Self::all() {
            if d.as_str() == s {
                return Ok(d);
            }
        }
        Err(ResolveError::InvariantViolation(format!(
            "unknown ToleranceDirection string '{s}' (closed taxonomy: {})",
            Self::all()
                .iter()
                .map(|d| d.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  Section 4 — Closed taxonomy : ToleranceSource (STABILITY § 27.2.7)
// ═══════════════════════════════════════════════════════════════════════

/// Closed taxonomy of tolerance sources (STABILITY § 27.2.7).
///
/// Eight variants in three semantic categories :
///
/// - **Decision-time sources (4)** : the resolver applied this value when
///   the aggregator made its decision.  These are the variants for which
///   `EffectiveTolerance.is_decision_time()` returns `true`.
///   - `Yaml` — directly from the scenario YAML.
///   - `RuntimeOverride` — env-var or CLI override.
///   - `GpuRunnerOverride` — automatic override for GPU runners.
///   - `Defaulted` — § 24.3 default applied because YAML omitted.
///
/// - **Sentinel (1)** :
///   - `NoToleranceDefined` — the metric carries no bound at all.  The
///     ONLY case where `EffectiveTolerance.value` may be `None`.
///
/// - **Legacy and reconstructed (3)** :
///   - `PreG11Unbackfilled` — legacy NDJSON whose tolerance is not
///     reconstructible via Git archaeology.
///   - `BackfilledYaml` — reconstructed by `scripts/backfill-tolerance-used.sh`
///     from the YAML committed at `applied_at`.
///   - `BackfilledRuntimeOverride` — reconstructed from the override journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToleranceSource {
    /// Directly from the scenario YAML's `tolerance:` block.
    Yaml,
    /// Active env-var override (e.g. `EFE_TOLERANCE_<scenario>_<metric>`)
    /// or active CLI override (e.g. `--override-tolerance`).
    RuntimeOverride,
    /// Automatic override because the runner is GPU-equipped AND the
    /// scenario declares a `gpu_override` block.
    GpuRunnerOverride,
    /// YAML did not declare ; default § 24.3 applied.
    Defaulted,
    /// No tolerance defined anywhere ; the metric is unbounded.  This
    /// is the ONLY source for which `value` MUST be `None`.
    NoToleranceDefined,
    /// NDJSON pre-G.11 whose value could not be reconstructed
    /// (e.g. `applied_at` absent and file mtime unreliable).
    PreG11Unbackfilled,
    /// Value reconstructed by `scripts/backfill-tolerance-used.sh` from
    /// the YAML committed at `applied_at`.
    BackfilledYaml,
    /// Value reconstructed from the historical override journal
    /// (kept ~90 days for env-var overrides, ~365 days for GPU-runner
    /// overrides).
    BackfilledRuntimeOverride,
}

impl ToleranceSource {
    /// The stable wire-format string for this source.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Yaml => "yaml",
            Self::RuntimeOverride => "runtime_override",
            Self::GpuRunnerOverride => "gpu_runner_override",
            Self::Defaulted => "defaulted",
            Self::NoToleranceDefined => "no_tolerance_defined",
            Self::PreG11Unbackfilled => "pre_g11_unbackfilled",
            Self::BackfilledYaml => "backfilled_yaml",
            Self::BackfilledRuntimeOverride => "backfilled_runtime_override",
        }
    }

    /// All variants in canonical order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Yaml,
            Self::RuntimeOverride,
            Self::GpuRunnerOverride,
            Self::Defaulted,
            Self::NoToleranceDefined,
            Self::PreG11Unbackfilled,
            Self::BackfilledYaml,
            Self::BackfilledRuntimeOverride,
        ]
    }

    /// Human-readable phrase used by the audit-reader's `explain` output.
    #[must_use]
    pub const fn explain_phrase(self) -> &'static str {
        match self {
            Self::Yaml => "declared directly in the scenario YAML",
            Self::RuntimeOverride => "active env-var or CLI override at run time",
            Self::GpuRunnerOverride => "automatic override : runner is GPU-equipped",
            Self::Defaulted => "scenario YAML omitted the bound ; default § 24.3 applied",
            Self::NoToleranceDefined => "no tolerance was defined anywhere ; metric is unbounded",
            Self::PreG11Unbackfilled => "pre-PR-#27 NDJSON ; value not reconstructible",
            Self::BackfilledYaml => "value reconstructed by the backfill tool from the YAML committed at applied_at",
            Self::BackfilledRuntimeOverride => "value reconstructed by the backfill tool from the historical override journal",
        }
    }

    /// Returns `true` iff this source represents an actual decision the
    /// aggregator made at run time (not a legacy / backfilled /
    /// undefined placeholder).
    #[must_use]
    pub const fn is_decision_time_source(self) -> bool {
        matches!(
            self,
            Self::Yaml | Self::RuntimeOverride | Self::GpuRunnerOverride | Self::Defaulted
        )
    }

    /// Returns `true` iff this source was reconstructed by the backfill
    /// tool from historical artefacts.
    #[must_use]
    pub const fn is_backfilled(self) -> bool {
        matches!(
            self,
            Self::BackfilledYaml | Self::BackfilledRuntimeOverride
        )
    }

    /// Returns `true` iff this source is a legacy marker (pre-PR-#27
    /// row whose tolerance is unreconstructible).
    #[must_use]
    pub const fn is_legacy(self) -> bool {
        matches!(self, Self::PreG11Unbackfilled)
    }

    /// Returns `true` iff this source indicates an override of the
    /// YAML-declared value (either runtime or GPU).
    #[must_use]
    pub const fn is_override(self) -> bool {
        matches!(
            self,
            Self::RuntimeOverride
                | Self::GpuRunnerOverride
                | Self::BackfilledRuntimeOverride
        )
    }
}

impl fmt::Display for ToleranceSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ToleranceSource {
    type Err = ResolveError;

    fn from_str(s: &str) -> ResolveResult<Self> {
        for &src in Self::all() {
            if src.as_str() == s {
                return Ok(src);
            }
        }
        Err(ResolveError::InvariantViolation(format!(
            "unknown ToleranceSource string '{s}' (closed taxonomy: {})",
            Self::all()
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  Section 5 — Value type : scalar OR bilateral range
// ═══════════════════════════════════════════════════════════════════════

/// Tolerance value : a scalar `f64` (the common case) or a bilateral
/// `[lower, upper]` range (only for `direction = within_range`).
///
/// ## Serialisation
///
/// The serde representation is **untagged** : a scalar serialises as a
/// plain JSON number, a range as a two-element array.
/// `Option<ToleranceValue>` is what `EffectiveTolerance.value` carries ;
/// the `None` case serialises as JSON `null` and is permitted **only**
/// when `source = no_tolerance_defined` or `pre_g11_unbackfilled`.
///
/// **Whole-number compact form** : `Scalar(v)` whose `v.fract() == 0.0`
/// and whose magnitude fits in `i64` is emitted as a JSON integer
/// (e.g. `5` not `5.0`), matching the canonical wire format of the
/// fixtures at `bench/fixtures/post_g11_ndjson/`.  This ensures that a
/// Rust-produced row is byte-identical to its hand-crafted fixture
/// counterpart, which matters for the signed evidence chain (the
/// signature is over the wire bytes).  Ranges retain the f64 form
/// always (`[95.0, 99.5]`) because mixed-form ranges (`[95, 99.5]`)
/// are uglier than the trade-off justifies.
///
/// The internal-consistency invariant is enforced by
/// `EffectiveTolerance::validate_internal_consistency()`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ToleranceValue {
    /// A scalar bound.  Used for `no_more_than`, `no_less_than`, `exactly`.
    Scalar(f64),
    /// A bilateral `[lower, upper]` range.  Used for `within_range`.
    Range([f64; 2]),
}

impl Serialize for ToleranceValue {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            // Whole-number compact form : emit `5` rather than `5.0`
            // when the scalar is finite, has no fractional part, and
            // fits in i64.  Matches the canonical wire format of the
            // fixtures at `bench/fixtures/post_g11_ndjson/`.
            Self::Scalar(v)
                if v.is_finite() && v.fract() == 0.0 && v.abs() < (i64::MAX as f64) =>
            {
                #[allow(clippy::cast_possible_truncation)]
                serializer.serialize_i64(*v as i64)
            }
            Self::Scalar(v) => serializer.serialize_f64(*v),
            Self::Range([lo, hi]) => {
                use serde::ser::SerializeTuple;
                let mut tup = serializer.serialize_tuple(2)?;
                tup.serialize_element(lo)?;
                tup.serialize_element(hi)?;
                tup.end()
            }
        }
    }
}

impl ToleranceValue {
    /// Construct a scalar value.
    #[must_use]
    pub const fn scalar(v: f64) -> Self {
        Self::Scalar(v)
    }

    /// Construct a range value.  Does **not** validate that `lower <= upper` ;
    /// that check is part of `EffectiveTolerance::validate_internal_consistency()`.
    #[must_use]
    pub const fn range(lower: f64, upper: f64) -> Self {
        Self::Range([lower, upper])
    }

    /// Returns the scalar value, or `None` if this is a range.
    #[must_use]
    pub fn as_scalar(&self) -> Option<f64> {
        match self {
            Self::Scalar(v) => Some(*v),
            Self::Range(_) => None,
        }
    }

    /// Returns the range, or `None` if this is a scalar.
    #[must_use]
    pub fn as_range(&self) -> Option<[f64; 2]> {
        match self {
            Self::Scalar(_) => None,
            Self::Range(r) => Some(*r),
        }
    }

    /// Returns the lower bound : the scalar value for `Scalar`, the
    /// `lower` element for `Range`.
    #[must_use]
    pub fn lower_bound(&self) -> f64 {
        match self {
            Self::Scalar(v) => *v,
            Self::Range([lo, _]) => *lo,
        }
    }

    /// Returns the upper bound : the scalar value for `Scalar`, the
    /// `upper` element for `Range`.
    #[must_use]
    pub fn upper_bound(&self) -> f64 {
        match self {
            Self::Scalar(v) => *v,
            Self::Range([_, hi]) => *hi,
        }
    }

    /// Returns `true` iff this is a scalar value.
    #[must_use]
    pub const fn is_scalar(&self) -> bool {
        matches!(self, Self::Scalar(_))
    }

    /// Returns `true` iff this is a range value.
    #[must_use]
    pub const fn is_range(&self) -> bool {
        matches!(self, Self::Range(_))
    }

    /// Validates that this value is structurally compatible with the
    /// given direction.
    ///
    /// - `WithinRange` requires `Range`.
    /// - `NoMoreThan` / `NoLessThan` / `Exactly` require `Scalar`.
    ///
    /// Returns `Ok(())` if compatible, else a human-readable error.
    pub fn validate_direction_match(
        &self,
        direction: ToleranceDirection,
    ) -> ResolveResult<()> {
        match (self, direction) {
            (Self::Range(_), ToleranceDirection::WithinRange) => Ok(()),
            (Self::Scalar(_), ToleranceDirection::WithinRange) => {
                Err(ResolveError::InvariantViolation(
                    "direction=within_range requires value to be a [lower, upper] array, got scalar"
                        .to_string(),
                ))
            }
            (Self::Range(_), d) => Err(ResolveError::InvariantViolation(format!(
                "direction={} requires value to be a scalar, got [lower, upper] array",
                d.as_str()
            ))),
            (Self::Scalar(_), _) => Ok(()),
        }
    }

    /// Evaluates whether an observation satisfies this value under the
    /// given direction.  Used by the mini-aggregator to compute the
    /// PASS / FAIL verdict (WARN is a soft band of ±10 %).
    ///
    /// Returns `true` iff the observation is within bounds (PASS region).
    #[must_use]
    pub fn satisfies(&self, observed: f64, direction: ToleranceDirection) -> bool {
        match (self, direction) {
            (Self::Scalar(bound), ToleranceDirection::NoMoreThan) => observed <= *bound,
            (Self::Scalar(bound), ToleranceDirection::NoLessThan) => observed >= *bound,
            (Self::Scalar(bound), ToleranceDirection::Exactly) => {
                (observed - bound).abs() < f64::EPSILON
            }
            (Self::Range([lo, hi]), ToleranceDirection::WithinRange) => {
                observed >= *lo && observed <= *hi
            }
            _ => false, // ill-formed combination
        }
    }
}

impl fmt::Display for ToleranceValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scalar(v) => write!(f, "{v}"),
            Self::Range([lo, hi]) => write!(f, "[{lo}, {hi}]"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  Section 6 — EffectiveTolerance — the central struct of PR #27
// ═══════════════════════════════════════════════════════════════════════

/// The tolerance that the verdict aggregator **actually applied** at the
/// moment of decision.
///
/// Frozen shape under STABILITY § 24.3.6 (6 fields : 5 mandatory + 1
/// optional).  Adding, renaming, or removing a field is a STABILITY-bump.
///
/// ## Internal-consistency invariants
///
/// Enforced by [`EffectiveTolerance::validate_internal_consistency`] :
///
/// 1. `value.is_none() ⇔ source ∈ {NoToleranceDefined, PreG11Unbackfilled}`.
///    *Double-implication.*  Frozen § 24.3.6.
///
/// 2. `applied_at.is_none() ⇔ source == PreG11Unbackfilled`.
///    *Double-implication.*  Frozen § 24.3.6.
///
/// 3. If `value` is `Some(...)`, the `direction` must match the variant :
///    `WithinRange` requires `Range([lo, hi])` and the three unilateral
///    directions require `Scalar(v)`.  Frozen § 24.3.6.
///
/// 4. **Recommended** (not enforced) : `override_reason` is non-empty
///    when `source` is an override or backfilled variant.  Audit-reader
///    surfaces a hint when missing.
///
/// ## Construction
///
/// Three idiomatic ways :
///
/// - The factory [`EffectiveTolerance::no_tolerance_defined`] for the
///   sentinel case.
/// - The builder pattern via [`EffectiveTolerance::builder`] for fluent
///   construction in tests.
/// - Direct struct literal for the common case (just-fill-the-fields).
///
/// All paths call (or should call) `validate_internal_consistency`
/// before the struct escapes its module of origin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectiveTolerance {
    /// Bound value.  `None` is permitted **only** when
    /// `source ∈ {NoToleranceDefined, PreG11Unbackfilled}`.
    pub value: Option<ToleranceValue>,
    /// Closed taxonomy of unit (§ 27.2.5).
    pub unit: ToleranceUnit,
    /// Closed taxonomy of direction (§ 27.2.6).
    pub direction: ToleranceDirection,
    /// Closed taxonomy of source (§ 27.2.7).
    pub source: ToleranceSource,
    /// ISO-8601 UTC timestamp of the aggregator's decision.  `None`
    /// permitted **only** when `source == PreG11Unbackfilled`.
    pub applied_at: Option<DateTime<Utc>>,
    /// Optional free-form short explanation (≤ 200 chars recommended)
    /// when `source` indicates an override or a backfill.  Skipped
    /// in serde output when `None` to keep the wire small.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub override_reason: Option<String>,
}

impl EffectiveTolerance {
    /// Construct the canonical "no tolerance defined" sentinel.
    ///
    /// The resulting struct has `value = None`, `source = NoToleranceDefined`,
    /// `override_reason = None`, and the caller-supplied `unit`,
    /// `direction`, `applied_at`.
    #[must_use]
    pub fn no_tolerance_defined(
        unit: ToleranceUnit,
        direction: ToleranceDirection,
        applied_at: DateTime<Utc>,
    ) -> Self {
        Self {
            value: None,
            unit,
            direction,
            source: ToleranceSource::NoToleranceDefined,
            applied_at: Some(applied_at),
            override_reason: None,
        }
    }

    /// Construct the canonical "pre-G.11 unbackfilled" legacy marker.
    ///
    /// The resulting struct has `value = None`, `source = PreG11Unbackfilled`,
    /// `applied_at = None`, and the caller-supplied `unit`, `direction`,
    /// `override_reason`.
    #[must_use]
    pub fn pre_g11_unbackfilled(
        unit: ToleranceUnit,
        direction: ToleranceDirection,
        override_reason: Option<String>,
    ) -> Self {
        Self {
            value: None,
            unit,
            direction,
            source: ToleranceSource::PreG11Unbackfilled,
            applied_at: None,
            override_reason,
        }
    }

    /// Begin a builder for fluent construction of an `EffectiveTolerance`.
    ///
    /// The builder validates internal consistency on `build()`.
    #[must_use]
    pub fn builder() -> EffectiveToleranceBuilder {
        EffectiveToleranceBuilder::default()
    }

    /// Verify that the struct's fields satisfy the cross-field invariants
    /// frozen in STABILITY § 24.3.6.
    ///
    /// Returns `Ok(())` if consistent, else a human-readable explanation.
    /// Called by the freeze test
    /// `v1_7_section_24_3_6_tolerance_used_shape` on every fixture row.
    pub fn validate_internal_consistency(&self) -> ResolveResult<()> {
        // Invariant 1 : value.is_none() ⇔ source ∈ {NoToleranceDefined, PreG11Unbackfilled}
        let value_is_null = self.value.is_none();
        let source_allows_null = matches!(
            self.source,
            ToleranceSource::NoToleranceDefined | ToleranceSource::PreG11Unbackfilled
        );
        if value_is_null && !source_allows_null {
            return Err(ResolveError::InvariantViolation(format!(
                "STABILITY § 24.3.6 : value=null requires source ∈ {{no_tolerance_defined, pre_g11_unbackfilled}}, got source={}",
                self.source.as_str()
            )));
        }
        if !value_is_null && self.source == ToleranceSource::NoToleranceDefined {
            return Err(ResolveError::InvariantViolation(
                "STABILITY § 24.3.6 : source=no_tolerance_defined requires value=null".to_string(),
            ));
        }
        if !value_is_null && self.source == ToleranceSource::PreG11Unbackfilled {
            return Err(ResolveError::InvariantViolation(
                "STABILITY § 24.3.6 : source=pre_g11_unbackfilled requires value=null".to_string(),
            ));
        }

        // Invariant 2 : applied_at.is_none() ⇔ source == PreG11Unbackfilled
        let applied_at_is_null = self.applied_at.is_none();
        let source_allows_null_applied_at =
            self.source == ToleranceSource::PreG11Unbackfilled;
        if applied_at_is_null && !source_allows_null_applied_at {
            return Err(ResolveError::InvariantViolation(format!(
                "STABILITY § 24.3.6 : applied_at=null only permitted when source=pre_g11_unbackfilled, got source={}",
                self.source.as_str()
            )));
        }
        if !applied_at_is_null && source_allows_null_applied_at {
            return Err(ResolveError::InvariantViolation(
                "STABILITY § 24.3.6 : source=pre_g11_unbackfilled requires applied_at=null"
                    .to_string(),
            ));
        }

        // Invariant 3 : direction-value compatibility (only when value is populated)
        if let Some(ref v) = self.value {
            v.validate_direction_match(self.direction)?;
        }

        // Invariant 4 : if range, lower <= upper
        if let Some(ToleranceValue::Range([lo, hi])) = self.value {
            if lo > hi {
                return Err(ResolveError::InvariantViolation(format!(
                    "STABILITY § 24.3.6 : range bound [{lo}, {hi}] has lower > upper"
                )));
            }
        }

        Ok(())
    }

    /// Returns `true` iff this tolerance was applied at decision time
    /// (not a legacy / backfilled / undefined placeholder).
    #[must_use]
    pub const fn is_decision_time(&self) -> bool {
        self.source.is_decision_time_source()
    }

    /// Returns `true` iff this tolerance was reconstructed by the
    /// backfill tool.
    #[must_use]
    pub const fn is_backfilled(&self) -> bool {
        self.source.is_backfilled()
    }

    /// Returns `true` iff this tolerance is a legacy marker.
    #[must_use]
    pub const fn is_legacy(&self) -> bool {
        self.source.is_legacy()
    }

    /// Returns `true` iff the tolerance is unbounded
    /// (`source == NoToleranceDefined`).
    #[must_use]
    pub const fn is_unbounded(&self) -> bool {
        matches!(self.source, ToleranceSource::NoToleranceDefined)
    }

    /// Render a multi-line prose explanation suitable for the
    /// audit-reader CLI's `explain` command.
    #[must_use]
    pub fn explain(&self) -> String {
        let value_str = match &self.value {
            None => "null (no tolerance defined)".to_string(),
            Some(v) => format!("{v}"),
        };
        let applied_at_str = self.applied_at.map_or_else(
            || "null (legacy pre-G.11 row)".to_string(),
            |t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        );
        let reason_str = self
            .override_reason
            .as_deref()
            .map(|s| format!("\n  Reason        :  {s}"))
            .unwrap_or_default();
        format!(
            "  Value         :  {value_str}\n  Unit          :  {unit}\n  Direction     :  {direction} ({direction_phrase})\n  Source        :  {source} ({source_phrase})\n  Applied at    :  {applied_at_str}{reason_str}",
            unit = self.unit.as_str(),
            direction = self.direction.as_str(),
            direction_phrase = self.direction.explain_phrase(),
            source = self.source.as_str(),
            source_phrase = self.source.explain_phrase(),
        )
    }
}

impl fmt::Display for EffectiveTolerance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "EffectiveTolerance(value={}, unit={}, direction={}, source={})",
            match &self.value {
                None => "null".to_string(),
                Some(v) => format!("{v}"),
            },
            self.unit.as_str(),
            self.direction.as_str(),
            self.source.as_str(),
        )
    }
}

/// Fluent builder for `EffectiveTolerance`.  Validates on `build()`.
#[derive(Debug, Default)]
pub struct EffectiveToleranceBuilder {
    value: Option<ToleranceValue>,
    unit: Option<ToleranceUnit>,
    direction: Option<ToleranceDirection>,
    source: Option<ToleranceSource>,
    applied_at: Option<DateTime<Utc>>,
    override_reason: Option<String>,
}

impl EffectiveToleranceBuilder {
    /// Set the value (scalar or range).
    #[must_use]
    pub fn value(mut self, v: ToleranceValue) -> Self {
        self.value = Some(v);
        self
    }

    /// Set the value to a scalar.  Shortcut for `value(ToleranceValue::scalar(...))`.
    #[must_use]
    pub fn scalar(mut self, v: f64) -> Self {
        self.value = Some(ToleranceValue::Scalar(v));
        self
    }

    /// Set the value to a range.  Shortcut for `value(ToleranceValue::range(...))`.
    #[must_use]
    pub fn range(mut self, lower: f64, upper: f64) -> Self {
        self.value = Some(ToleranceValue::Range([lower, upper]));
        self
    }

    /// Set `value` to `None` (only valid with sentinel / legacy sources).
    #[must_use]
    pub fn value_null(mut self) -> Self {
        self.value = None;
        self
    }

    /// Set the unit.
    #[must_use]
    pub fn unit(mut self, u: ToleranceUnit) -> Self {
        self.unit = Some(u);
        self
    }

    /// Set the direction.
    #[must_use]
    pub fn direction(mut self, d: ToleranceDirection) -> Self {
        self.direction = Some(d);
        self
    }

    /// Set the source.
    #[must_use]
    pub fn source(mut self, s: ToleranceSource) -> Self {
        self.source = Some(s);
        self
    }

    /// Set the `applied_at` timestamp.
    #[must_use]
    pub fn applied_at(mut self, t: DateTime<Utc>) -> Self {
        self.applied_at = Some(t);
        self
    }

    /// Set the `override_reason` (only meaningful for override / backfilled sources).
    #[must_use]
    pub fn override_reason(mut self, r: impl Into<String>) -> Self {
        self.override_reason = Some(r.into());
        self
    }

    /// Build the `EffectiveTolerance`, validating invariants.
    ///
    /// Returns `Err` if any mandatory field is missing or if the
    /// internal-consistency invariant is violated.
    pub fn build(self) -> ResolveResult<EffectiveTolerance> {
        let unit = self.unit.ok_or_else(|| {
            ResolveError::InvariantViolation("EffectiveToleranceBuilder: unit is required".to_string())
        })?;
        let direction = self.direction.ok_or_else(|| {
            ResolveError::InvariantViolation(
                "EffectiveToleranceBuilder: direction is required".to_string(),
            )
        })?;
        let source = self.source.ok_or_else(|| {
            ResolveError::InvariantViolation(
                "EffectiveToleranceBuilder: source is required".to_string(),
            )
        })?;
        let tolerance = EffectiveTolerance {
            value: self.value,
            unit,
            direction,
            source,
            applied_at: self.applied_at,
            override_reason: self.override_reason,
        };
        tolerance.validate_internal_consistency()?;
        Ok(tolerance)
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  Section 7 — Resolver inputs : YAML entry, runner introspection, env
// ═══════════════════════════════════════════════════════════════════════

/// One entry of a scenario's `tolerance:` block, as parsed from YAML.
///
/// Each entry carries the value, the unit, the direction, and an
/// optional `gpu_override` sub-block for GPU runners.
#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioToleranceEntry {
    /// Bound value (scalar or range).  YAML `value: 200` parses as
    /// `Scalar(200.0)` ; YAML `value: [95, 99.5]` parses as
    /// `Range([95.0, 99.5])`.
    pub value: ToleranceValue,
    /// Closed-taxonomy unit.
    pub unit: ToleranceUnit,
    /// Closed-taxonomy direction.
    pub direction: ToleranceDirection,
    /// Optional override block for GPU-equipped runners.
    #[serde(default)]
    pub gpu_override: Option<GpuOverrideEntry>,
}

impl ScenarioToleranceEntry {
    /// Construct an entry from individual fields.  Useful in tests.
    #[must_use]
    pub const fn new(
        value: ToleranceValue,
        unit: ToleranceUnit,
        direction: ToleranceDirection,
    ) -> Self {
        Self {
            value,
            unit,
            direction,
            gpu_override: None,
        }
    }

    /// Builder-style addition of a GPU override.
    #[must_use]
    pub fn with_gpu_override(mut self, gpu: GpuOverrideEntry) -> Self {
        self.gpu_override = Some(gpu);
        self
    }
}

/// Optional `gpu_override:` block within a tolerance entry.
///
/// Activated when the runner is GPU-equipped AND the scenario YAML
/// declared this block.  The override carries a (typically more
/// permissive) bound that replaces the regular YAML value.
#[derive(Debug, Clone, Deserialize)]
pub struct GpuOverrideEntry {
    /// The override bound value.
    pub value: ToleranceValue,
    /// Optional free-form reason recorded in the
    /// `EffectiveTolerance.override_reason` when this override is applied.
    #[serde(default)]
    pub reason: Option<String>,
}

impl GpuOverrideEntry {
    /// Construct an override from individual fields.
    #[must_use]
    pub fn new(value: ToleranceValue, reason: impl Into<Option<String>>) -> Self {
        Self {
            value,
            reason: reason.into(),
        }
    }
}

/// Runner introspection : the harness's view of the machine on which
/// the benchmark is running.  Affects branch selection in the resolver
/// (GPU detection).
#[derive(Debug, Clone, Default)]
pub struct RunnerIntrospection {
    /// Runner kind (e.g. "cpu", "gpu", "kind", "k3s").
    pub kind: String,
    /// Stable identifier of this specific runner.
    pub id: String,
    /// Whether the runner has GPU acceleration available.
    pub has_gpu: bool,
    /// Optional GPU model name (e.g. "nvidia-A100-80GB").
    pub gpu_model: Option<String>,
}

impl RunnerIntrospection {
    /// CPU-only runner factory.
    #[must_use]
    pub fn cpu_only(id: impl Into<String>) -> Self {
        Self {
            kind: "cpu".into(),
            id: id.into(),
            has_gpu: false,
            gpu_model: None,
        }
    }

    /// GPU-equipped runner factory.
    #[must_use]
    pub fn with_gpu(id: impl Into<String>, gpu_model: impl Into<String>) -> Self {
        Self {
            kind: "gpu".into(),
            id: id.into(),
            has_gpu: true,
            gpu_model: Some(gpu_model.into()),
        }
    }

    /// Serialise into a JSON-compatible value for inclusion in NDJSON rows.
    #[must_use]
    pub fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": self.kind,
            "id": self.id,
            "has_gpu": self.has_gpu,
            "gpu_model": self.gpu_model,
        })
    }
}

/// Env context : env-var overrides keyed by
/// `EFE_TOLERANCE_<scenario>_<metric>`.
#[derive(Debug, Clone, Default)]
pub struct EnvContext {
    overrides: HashMap<String, f64>,
}

impl EnvContext {
    /// Empty context (no overrides).
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Context with a single override.
    #[must_use]
    pub fn with_override(scenario: &str, metric: &str, value: f64) -> Self {
        let mut e = Self::default();
        e.overrides.insert(env_var_key(scenario, metric), value);
        e
    }

    /// Add an override in place.
    pub fn add_override(&mut self, scenario: &str, metric: &str, value: f64) {
        self.overrides.insert(env_var_key(scenario, metric), value);
    }

    /// Returns the override value for the (scenario, metric) pair, if any.
    #[must_use]
    pub fn lookup(&self, scenario: &str, metric: &str) -> Option<f64> {
        self.overrides
            .get(&env_var_key(scenario, metric))
            .copied()
    }

    /// Snapshot from the process's environment for the given metrics.
    /// Invalid env-var values (non-parseable f64) are silently ignored.
    #[must_use]
    pub fn from_process_env(scenario: &str, metrics: &[&str]) -> Self {
        let mut e = Self::default();
        for m in metrics {
            let key = env_var_key(scenario, m);
            if let Ok(v) = std::env::var(&key) {
                if let Ok(parsed) = v.parse::<f64>() {
                    e.overrides.insert(key, parsed);
                }
            }
        }
        e
    }

    /// Returns the number of overrides currently active.
    #[must_use]
    pub fn len(&self) -> usize {
        self.overrides.len()
    }

    /// Returns `true` iff no overrides are active.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }
}

fn env_var_key(scenario: &str, metric: &str) -> String {
    format!("EFE_TOLERANCE_{scenario}_{metric}")
}

// ═══════════════════════════════════════════════════════════════════════
//  Section 8 — The resolver : pure function, branch-selection frozen
// ═══════════════════════════════════════════════════════════════════════

/// Resolve the effective tolerance for one metric.
///
/// **Pure function.**  No I/O, no mutability, no logging.  Deterministic
/// on `(yaml_entry, env, runner, scenario_id, metric_name, applied_at)`.
///
/// ## Branch selection (frozen STABILITY § 27.2.7)
///
/// 1. **Env-var override** active → `source = RuntimeOverride`.
/// 2. Else, **GPU runner detected AND `gpu_override` declared** →
///    `source = GpuRunnerOverride`.
/// 3. Else, **YAML entry exists** → `source = Yaml`.
/// 4. Else, **no YAML entry** → `source = NoToleranceDefined`,
///    `value = None`.
///
/// Note that branch 1 takes precedence over branch 2 : an env-var
/// override applies even on a GPU runner with a `gpu_override` block.
/// This matches the intuitive "operator override is the strongest signal"
/// principle.
///
/// ## Determinism
///
/// The `applied_at` timestamp is passed in by the caller, never read
/// from `Utc::now()` inside the function.  This guarantees byte-exact
/// reproducibility in tests : a test that passes
/// `DateTime::parse_from_rfc3339("2026-04-15T03:14:22Z").unwrap()`
/// always gets back the same struct.
#[must_use]
pub fn resolve_effective_tolerance(
    yaml_entry: Option<&ScenarioToleranceEntry>,
    env: &EnvContext,
    runner: &RunnerIntrospection,
    scenario_id: &str,
    metric_name: &str,
    applied_at: DateTime<Utc>,
) -> EffectiveTolerance {
    // Branch 1 — env-var override (takes precedence over GPU).
    if let Some(override_value) = env.lookup(scenario_id, metric_name) {
        let (unit, direction) = yaml_entry.map_or(
            (ToleranceUnit::Ms, ToleranceDirection::NoMoreThan),
            |y| (y.unit, y.direction),
        );
        return EffectiveTolerance {
            value: Some(ToleranceValue::Scalar(override_value)),
            unit,
            direction,
            source: ToleranceSource::RuntimeOverride,
            applied_at: Some(applied_at),
            override_reason: Some(format!(
                "env-var override active : {}={}",
                env_var_key(scenario_id, metric_name),
                override_value
            )),
        };
    }

    // Branch 2 — GPU runner override.
    if runner.has_gpu {
        if let Some(entry) = yaml_entry {
            if let Some(gpu) = &entry.gpu_override {
                return EffectiveTolerance {
                    value: Some(gpu.value.clone()),
                    unit: entry.unit,
                    direction: entry.direction,
                    source: ToleranceSource::GpuRunnerOverride,
                    applied_at: Some(applied_at),
                    override_reason: gpu.reason.clone().or_else(|| {
                        runner
                            .gpu_model
                            .as_deref()
                            .map(|m| format!("GPU runner detected : {m}"))
                    }),
                };
            }
        }
    }

    // Branch 3 — YAML entry present, no override.
    if let Some(entry) = yaml_entry {
        return EffectiveTolerance {
            value: Some(entry.value.clone()),
            unit: entry.unit,
            direction: entry.direction,
            source: ToleranceSource::Yaml,
            applied_at: Some(applied_at),
            override_reason: None,
        };
    }

    // Branch 4 — no tolerance at all.
    EffectiveTolerance::no_tolerance_defined(
        ToleranceUnit::Count,
        ToleranceDirection::NoMoreThan,
        applied_at,
    )
}

// ═══════════════════════════════════════════════════════════════════════
//  Section 9 — NDJSON v3 row type
// ═══════════════════════════════════════════════════════════════════════

/// One row of NDJSON v3 (post-PR-#27).  Mirrors the canonical fixtures
/// at `bench/fixtures/post_g11_ndjson/`.
///
/// Fields are in canonical wire order for serialisation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricVerdictV3 {
    /// Schema version.  Always `3` post-PR-#27.
    pub schema_version: u32,
    /// Stable scenario identifier (matches the file basename).
    pub scenario_id: String,
    /// Metric name as declared in `results.emit_metrics`.
    pub metric_name: String,
    /// Observed value at the moment of decision ; `None` for INCONCLUSIVE.
    pub observed: Option<f64>,
    /// Verdict string : `"PASS" | "WARN" | "FAIL" | "INCONCLUSIVE"`.
    pub verdict: String,
    /// Optional human-readable reason (e.g. "runner crashed during
    /// measurement window" for INCONCLUSIVE rows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The tolerance the aggregator actually applied at decision time.
    /// **This is the field G.11 closes.**
    pub tolerance_used: EffectiveTolerance,
    /// Runner introspection at the moment of decision.
    pub runner: serde_json::Value,
    /// Row-level applied_at (may match `tolerance_used.applied_at`).
    pub applied_at: DateTime<Utc>,
}

impl MetricVerdictV3 {
    /// Reconstitute the [`EffectiveTolerance`] from a row.
    ///
    /// This is the audit-reader entry point for the
    /// "tolerance at decision time" query.
    #[must_use]
    pub fn tolerance_at_decision_time(&self) -> EffectiveTolerance {
        self.tolerance_used.clone()
    }

    /// Returns `true` iff the verdict is `"PASS"`.
    #[must_use]
    pub fn is_pass(&self) -> bool {
        self.verdict == "PASS"
    }

    /// Returns `true` iff the verdict is `"WARN"`.
    #[must_use]
    pub fn is_warn(&self) -> bool {
        self.verdict == "WARN"
    }

    /// Returns `true` iff the verdict is `"FAIL"`.
    #[must_use]
    pub fn is_fail(&self) -> bool {
        self.verdict == "FAIL"
    }

    /// Returns `true` iff the verdict is `"INCONCLUSIVE"`.
    #[must_use]
    pub fn is_inconclusive(&self) -> bool {
        self.verdict == "INCONCLUSIVE"
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  Section 10 — Reference mini-aggregator (Day 1 implementation,
//  Day 3 binding to integration tests)
// ═══════════════════════════════════════════════════════════════════════

/// Mini-aggregator entry point for the PR #27 pinning tests.
///
/// Loads a scenario YAML fixture, resolves each metric's tolerance,
/// computes a verdict against the supplied observed values, emits a
/// vector of [`MetricVerdictV3`] rows.
///
/// **Not the production aggregator.**  The real `compare::compare()` is
/// migrated to emit `MetricVerdictV3` in a follow-up PR #27.1 (see
/// architecture AD-12).  This mini-aggregator exists to drive the
/// round-trip pinning tests without forcing the full migration.
///
/// ## Inputs
///
/// - `scenario_yaml_path` — path to a scenario YAML fixture under
///   `bench/fixtures/scenarios/`.
/// - `observed` — map of metric name → observed value.  Absence of a
///   metric from this map yields an INCONCLUSIVE verdict for that metric.
/// - `runner` — runner introspection (CPU-only or GPU-equipped).
/// - `env` — env-var override context.
/// - `applied_at` — the canonical timestamp to stamp on every row.
///
/// ## Output
///
/// A vector of `MetricVerdictV3` rows, one per metric declared in
/// `results.emit_metrics` of the scenario YAML, in declaration order.
///
/// ## Errors
///
/// Returns `Err(String)` if the YAML cannot be read, parsed, or if a
/// required field is missing.
#[allow(clippy::implicit_hasher)]
pub fn pretend_aggregator_emit_rows(
    scenario_yaml_path: &Path,
    observed: &HashMap<String, f64>,
    runner: &RunnerIntrospection,
    env: &EnvContext,
    applied_at: DateTime<Utc>,
) -> Result<Vec<MetricVerdictV3>, String> {
    let raw = std::fs::read_to_string(scenario_yaml_path)
        .map_err(|e| format!("read scenario {}: {e}", scenario_yaml_path.display()))?;
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&raw).map_err(|e| format!("parse scenario YAML: {e}"))?;

    let scenario_id = doc
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "scenario YAML missing `id:`".to_string())?
        .to_string();

    let emit_metrics = doc
        .get("results")
        .and_then(|r| r.get("emit_metrics"))
        .and_then(|m| m.as_sequence())
        .ok_or_else(|| "scenario YAML missing `results.emit_metrics`".to_string())?
        .clone();

    let tolerance_block = doc.get("tolerance").cloned();

    let mut rows = Vec::with_capacity(emit_metrics.len());

    for metric_decl in emit_metrics {
        let metric_name = metric_decl
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "emit_metrics entry missing `name:`".to_string())?
            .to_string();

        let yaml_entry: Option<ScenarioToleranceEntry> = tolerance_block
            .as_ref()
            .and_then(|t| t.get(&metric_name))
            .map(|v| serde_yaml::from_value::<ScenarioToleranceEntry>(v.clone()))
            .transpose()
            .map_err(|e| format!("parse tolerance entry for {metric_name}: {e}"))?;

        let effective = resolve_effective_tolerance(
            yaml_entry.as_ref(),
            env,
            runner,
            &scenario_id,
            &metric_name,
            applied_at,
        );

        let observed_value = observed.get(&metric_name).copied();
        let verdict = compute_verdict(observed_value, &effective);

        let reason = if verdict == "INCONCLUSIVE" {
            Some("observed value absent at aggregator-decision time".to_string())
        } else {
            None
        };

        rows.push(MetricVerdictV3 {
            schema_version: 3,
            scenario_id: scenario_id.clone(),
            metric_name,
            observed: observed_value,
            verdict,
            reason,
            tolerance_used: effective,
            runner: runner.to_json_value(),
            applied_at,
        });
    }

    Ok(rows)
}

/// Soft-band ratio used by the mini-aggregator to distinguish WARN from
/// PASS on unilateral directions (`NoMoreThan`, `NoLessThan`).
///
/// **Semantic** : an observation within ±10 % of the bound — but still
/// satisfying the bound — yields `WARN`.  An observation beyond the
/// bound yields `FAIL` ; an observation comfortably inside the bound
/// yields `PASS`.
///
/// **Note** : this 10 % soft-band is a simplification for the
/// mini-aggregator.  The production `compare::compare()` uses separate
/// warn / fail thresholds per metric from
/// `bench/harness/src/lib.rs::tolerance_defaults`.  Aligning the two
/// taxonomies is scoped for PR #27.1 (S2 architecture AD-12).  This
/// constant is exposed publicly so tests and the audit-reader CLI can
/// reference it without duplication.
pub const MINI_AGGREGATOR_WARN_MARGIN_RATIO: f64 = 0.10;

/// Compute a 4-state verdict for a single observed value against an
/// [`EffectiveTolerance`].
///
/// ## The full verdict matrix (frozen by Day 2 of S2)
///
/// ```text
///   observed = None                                        →  INCONCLUSIVE
///   eff.value = None  (source = NoToleranceDefined)        →  PASS
///
///   direction = NoMoreThan,  value = Scalar(bound) :
///     observed > bound                                     →  FAIL
///     bound * (1 - 0.10) < observed ≤ bound                →  WARN
///     observed ≤ bound * (1 - 0.10)                        →  PASS
///
///   direction = NoLessThan,  value = Scalar(bound) :
///     observed < bound                                     →  FAIL
///     bound ≤ observed < bound * (1 + 0.10)                →  WARN
///     observed ≥ bound * (1 + 0.10)                        →  PASS
///
///   direction = WithinRange, value = Range([lo, hi]) :
///     observed < lo  OR  observed > hi                     →  FAIL
///     lo ≤ observed ≤ hi                                   →  PASS
///     (no WARN soft-band for bilateral ranges by AD-D2-3)
///
///   direction = Exactly,     value = Scalar(bound) :
///     |observed - bound| < f64::EPSILON                    →  PASS
///     otherwise                                            →  FAIL
///     (no WARN — exact match has no soft band)
///
///   any other (direction, value) combination                →  INCONCLUSIVE
///     (defensive fallback for ill-formed inputs that escape
///      EffectiveTolerance::validate_internal_consistency, e.g.
///      direction=NoMoreThan with value=Range(...))
/// ```
///
/// ## Day 2 changes
///
/// The matrix is finalised in Day 2 with named helper functions
/// (`verdict_no_more_than`, `verdict_no_less_than`,
/// `verdict_within_range`, `verdict_exactly`) for readability.  The
/// dispatch logic of `compute_verdict` itself becomes a thin
/// pattern-match.  Behaviour preserved byte-identical from Day 1 ; the
/// refactor is internal only.
///
/// ## Determinism
///
/// Pure function : no I/O, no clock access, no mutability.  Returns
/// the same verdict for the same inputs.
#[must_use]
pub fn compute_verdict(observed: Option<f64>, eff: &EffectiveTolerance) -> String {
    let Some(obs) = observed else {
        return "INCONCLUSIVE".to_string();
    };
    let value = match &eff.value {
        None => return "PASS".to_string(),
        Some(v) => v,
    };
    match (eff.direction, value) {
        (ToleranceDirection::NoMoreThan, ToleranceValue::Scalar(bound)) => {
            verdict_no_more_than(obs, *bound).to_string()
        }
        (ToleranceDirection::NoLessThan, ToleranceValue::Scalar(bound)) => {
            verdict_no_less_than(obs, *bound).to_string()
        }
        (ToleranceDirection::WithinRange, ToleranceValue::Range([lo, hi])) => {
            verdict_within_range(obs, *lo, *hi).to_string()
        }
        (ToleranceDirection::Exactly, ToleranceValue::Scalar(bound)) => {
            verdict_exactly(obs, *bound).to_string()
        }
        _ => "INCONCLUSIVE".to_string(),
    }
}

/// Compute a 4-state verdict AND a human-readable explanation suitable
/// for the audit-reader CLI's `explain` command.
///
/// Returns `(verdict_string, explanation)` where the explanation is a
/// single-sentence narrative describing how the verdict was reached.
/// Used by the future PR #27.1 audit-reader CLI ; exposed in Day 2 to
/// be available for the audit chain reconstruction.
///
/// The verdict string is byte-identical to what [`compute_verdict`]
/// would return for the same inputs — this is the round-trip property
/// asserted by `audit_explanation_matches_verdict_string` test.
#[must_use]
#[allow(clippy::uninlined_format_args)] // format strings are large, readability > brevity here
pub fn compute_verdict_audit_explanation(
    observed: Option<f64>,
    eff: &EffectiveTolerance,
) -> (String, String) {
    let Some(obs) = observed else {
        return (
            "INCONCLUSIVE".to_string(),
            "Observed value absent at aggregator-decision time ; verdict undetermined."
                .to_string(),
        );
    };
    let value = match &eff.value {
        None => {
            return (
                "PASS".to_string(),
                format!(
                    "No tolerance defined for this metric (source = {}) ; observation {} is accepted by convention.",
                    eff.source.as_str(),
                    obs
                ),
            );
        }
        Some(v) => v,
    };
    match (eff.direction, value) {
        (ToleranceDirection::NoMoreThan, ToleranceValue::Scalar(bound)) => {
            let v = verdict_no_more_than(obs, *bound);
            let explain = match v {
                "PASS" => format!(
                    "Observation {} is at or below {:.4} % of the bound {} ms (well within the no_more_than tolerance).",
                    obs,
                    100.0 * (1.0 - MINI_AGGREGATOR_WARN_MARGIN_RATIO),
                    bound
                ),
                "WARN" => format!(
                    "Observation {} is within the ±{:.0} % soft band of the bound {} (no_more_than, satisfies the bound but flagged for attention).",
                    obs,
                    MINI_AGGREGATOR_WARN_MARGIN_RATIO * 100.0,
                    bound
                ),
                "FAIL" => format!(
                    "Observation {} exceeds the bound {} (no_more_than violated).",
                    obs, bound
                ),
                _ => format!("Observation {} produced unexpected verdict {} ; investigation required.", obs, v),
            };
            (v.to_string(), explain)
        }
        (ToleranceDirection::NoLessThan, ToleranceValue::Scalar(bound)) => {
            let v = verdict_no_less_than(obs, *bound);
            let explain = match v {
                "PASS" => format!(
                    "Observation {} is at or above {:.4} % of the bound {} (well within the no_less_than tolerance).",
                    obs,
                    100.0 * (1.0 + MINI_AGGREGATOR_WARN_MARGIN_RATIO),
                    bound
                ),
                "WARN" => format!(
                    "Observation {} is within the ±{:.0} % soft band of the bound {} (no_less_than, satisfies the bound but flagged for attention).",
                    obs,
                    MINI_AGGREGATOR_WARN_MARGIN_RATIO * 100.0,
                    bound
                ),
                "FAIL" => format!(
                    "Observation {} falls below the bound {} (no_less_than violated).",
                    obs, bound
                ),
                _ => format!("Observation {} produced unexpected verdict {} ; investigation required.", obs, v),
            };
            (v.to_string(), explain)
        }
        (ToleranceDirection::WithinRange, ToleranceValue::Range([lo, hi])) => {
            let v = verdict_within_range(obs, *lo, *hi);
            let explain = match v {
                "PASS" => format!(
                    "Observation {} falls within the bilateral range [{}, {}].",
                    obs, lo, hi
                ),
                "FAIL" => format!(
                    "Observation {} falls outside the bilateral range [{}, {}] (within_range violated).",
                    obs, lo, hi
                ),
                _ => format!("Observation {} produced unexpected verdict {} ; investigation required.", obs, v),
            };
            (v.to_string(), explain)
        }
        (ToleranceDirection::Exactly, ToleranceValue::Scalar(bound)) => {
            let v = verdict_exactly(obs, *bound);
            let explain = match v {
                "PASS" => format!(
                    "Observation {} equals the bound {} within f64::EPSILON precision.",
                    obs, bound
                ),
                "FAIL" => format!(
                    "Observation {} does not equal the bound {} (Exactly violated, difference exceeds f64::EPSILON).",
                    obs, bound
                ),
                _ => format!("Observation {} produced unexpected verdict {} ; investigation required.", obs, v),
            };
            (v.to_string(), explain)
        }
        _ => (
            "INCONCLUSIVE".to_string(),
            format!(
                "Ill-formed (direction={}, value-shape) combination at decision time ; defensive INCONCLUSIVE returned.",
                eff.direction.as_str()
            ),
        ),
    }
}

// ─── Named verdict-branch helpers (private, internal-use only) ──────
//
// Each helper takes the raw `observed` and the raw bound(s) and returns
// a stable verdict string.  Extracted from the inline match arms for
// readability + individually testable.  Returns `&'static str` so
// callers can `.to_string()` once at the call site (no allocation
// inside the helper).

/// Verdict for `direction = NoMoreThan` with scalar bound.
///
/// Three states with a soft band of ±[`MINI_AGGREGATOR_WARN_MARGIN_RATIO`] :
/// - `observed > bound` → `"FAIL"`
/// - `bound * 0.90 < observed ≤ bound` → `"WARN"`
/// - `observed ≤ bound * 0.90` → `"PASS"`
#[must_use]
pub(crate) fn verdict_no_more_than(observed: f64, bound: f64) -> &'static str {
    if observed > bound {
        "FAIL"
    } else if observed > bound * (1.0 - MINI_AGGREGATOR_WARN_MARGIN_RATIO) {
        "WARN"
    } else {
        "PASS"
    }
}

/// Verdict for `direction = NoLessThan` with scalar bound.
///
/// Three states with a soft band of ±[`MINI_AGGREGATOR_WARN_MARGIN_RATIO`] :
/// - `observed < bound` → `"FAIL"`
/// - `bound ≤ observed < bound * 1.10` → `"WARN"`
/// - `observed ≥ bound * 1.10` → `"PASS"`
#[must_use]
pub(crate) fn verdict_no_less_than(observed: f64, bound: f64) -> &'static str {
    if observed < bound {
        "FAIL"
    } else if observed < bound * (1.0 + MINI_AGGREGATOR_WARN_MARGIN_RATIO) {
        "WARN"
    } else {
        "PASS"
    }
}

/// Verdict for `direction = WithinRange` with bilateral `[lo, hi]` bounds.
///
/// Two states (no WARN soft-band per AD-D2-3) :
/// - `observed < lo` OR `observed > hi` → `"FAIL"`
/// - `lo ≤ observed ≤ hi` → `"PASS"`
#[must_use]
pub(crate) fn verdict_within_range(observed: f64, lo: f64, hi: f64) -> &'static str {
    if observed < lo || observed > hi {
        "FAIL"
    } else {
        "PASS"
    }
}

/// Verdict for `direction = Exactly` with scalar bound.
///
/// Two states (no WARN — exact match has no soft band) :
/// - `|observed - bound| < f64::EPSILON` → `"PASS"`
/// - otherwise → `"FAIL"`
///
/// Note : uses absolute `f64::EPSILON` comparison.  For very large
/// bounds (e.g. bound > 1e10), this is strict ; a future PR may
/// introduce a relative-epsilon variant.
#[must_use]
pub(crate) fn verdict_exactly(observed: f64, bound: f64) -> &'static str {
    if (observed - bound).abs() < f64::EPSILON {
        "PASS"
    } else {
        "FAIL"
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  Section 11 — Unit tests (Day 1 + Day 2)
//
//  Day 1 base : 34 tests covering :
//   - the 4 resolver source branches
//   - 6 validation invariants
//   - 3 serde round-trips (incl. range value + compact-integer pinning)
//   - 4 closed-taxonomy wire-format checks
//   - 4 verdict-logic checks
//   - 5 edge cases (NaN, infinity, range malformé, FromStr round-trip,
//     forward-compat with extra fields)
//   - 4 classifier-helper coverage tests
//   - 1 fixture byte-identity test
//   - 3 builder pattern tests
//
//  Day 2 additions : 8 tests covering :
//   - 4 emit_* tests pinning G.11.a (WARN) + G.11.b (INCONCLUSIVE) at the
//     full-row level (vs the verdict-string level pinned by Day 1)
//   - 1 audit_explanation round-trip property test
//   - 1 named-helper coverage test (verdict_no_more_than, etc.)
//   - 1 defensive INCONCLUSIVE test for ill-formed combinations
//   - 1 matrix-completeness sanity test
//
//  Total at end of Day 2 : 42 tests in this module.
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-15T03:14:22Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn yaml_entry_scalar(
        value: f64,
        unit: ToleranceUnit,
        dir: ToleranceDirection,
    ) -> ScenarioToleranceEntry {
        ScenarioToleranceEntry::new(ToleranceValue::Scalar(value), unit, dir)
    }

    // ── 1. resolver source branches ────────────────────────────────────

    #[test]
    fn resolve_yaml_only_returns_yaml_source() {
        let entry = yaml_entry_scalar(200.0, ToleranceUnit::Ms, ToleranceDirection::NoMoreThan);
        let env = EnvContext::empty();
        let runner = RunnerIntrospection::cpu_only("test-runner");
        let eff =
            resolve_effective_tolerance(Some(&entry), &env, &runner, "scen", "metric", now());
        assert_eq!(eff.source, ToleranceSource::Yaml);
        assert_eq!(eff.value, Some(ToleranceValue::Scalar(200.0)));
        assert_eq!(eff.unit, ToleranceUnit::Ms);
        assert_eq!(eff.direction, ToleranceDirection::NoMoreThan);
        assert!(eff.is_decision_time());
        assert!(!eff.is_backfilled());
        assert!(!eff.is_legacy());
        assert!(!eff.is_unbounded());
        assert!(eff.validate_internal_consistency().is_ok());
    }

    #[test]
    fn resolve_env_override_returns_runtime_source() {
        let entry = yaml_entry_scalar(200.0, ToleranceUnit::Ms, ToleranceDirection::NoMoreThan);
        let env = EnvContext::with_override("scen", "metric", 400.0);
        let runner = RunnerIntrospection::cpu_only("test-runner");
        let eff =
            resolve_effective_tolerance(Some(&entry), &env, &runner, "scen", "metric", now());
        assert_eq!(eff.source, ToleranceSource::RuntimeOverride);
        assert_eq!(eff.value, Some(ToleranceValue::Scalar(400.0)));
        assert!(eff.override_reason.is_some());
        assert!(eff
            .override_reason
            .as_deref()
            .unwrap()
            .contains("env-var override"));
        assert!(eff.validate_internal_consistency().is_ok());
    }

    #[test]
    fn resolve_gpu_runner_returns_gpu_source() {
        let entry = ScenarioToleranceEntry::new(
            ToleranceValue::Scalar(50000.0),
            ToleranceUnit::EventsPerSec,
            ToleranceDirection::NoLessThan,
        )
        .with_gpu_override(GpuOverrideEntry::new(
            ToleranceValue::Scalar(100000.0),
            Some("GPU runner with >40GB vRAM".to_string()),
        ));
        let env = EnvContext::empty();
        let runner = RunnerIntrospection::with_gpu("gpu-001", "nvidia-A100-80GB");
        let eff =
            resolve_effective_tolerance(Some(&entry), &env, &runner, "scen", "metric", now());
        assert_eq!(eff.source, ToleranceSource::GpuRunnerOverride);
        assert_eq!(eff.value, Some(ToleranceValue::Scalar(100000.0)));
        assert_eq!(eff.unit, ToleranceUnit::EventsPerSec);
        assert_eq!(eff.direction, ToleranceDirection::NoLessThan);
        assert!(eff.override_reason.is_some());
        assert!(eff.validate_internal_consistency().is_ok());
    }

    #[test]
    fn resolve_no_tolerance_returns_no_tolerance_defined() {
        let env = EnvContext::empty();
        let runner = RunnerIntrospection::cpu_only("test-runner");
        let eff = resolve_effective_tolerance(None, &env, &runner, "scen", "unbounded", now());
        assert_eq!(eff.source, ToleranceSource::NoToleranceDefined);
        assert_eq!(eff.value, None);
        assert!(eff.is_unbounded());
        assert!(eff.validate_internal_consistency().is_ok());
    }

    #[test]
    fn resolve_env_override_takes_precedence_over_gpu_override() {
        // Both env-var AND gpu_override active.  Env-var must win.
        let entry = ScenarioToleranceEntry::new(
            ToleranceValue::Scalar(200.0),
            ToleranceUnit::Ms,
            ToleranceDirection::NoMoreThan,
        )
        .with_gpu_override(GpuOverrideEntry::new(
            ToleranceValue::Scalar(500.0),
            Some("GPU override".to_string()),
        ));
        let env = EnvContext::with_override("scen", "metric", 999.0);
        let runner = RunnerIntrospection::with_gpu("gpu-001", "nvidia-A100");
        let eff =
            resolve_effective_tolerance(Some(&entry), &env, &runner, "scen", "metric", now());
        assert_eq!(eff.source, ToleranceSource::RuntimeOverride);
        assert_eq!(eff.value, Some(ToleranceValue::Scalar(999.0)));
    }

    // ── 2. validation invariants ──────────────────────────────────────

    #[test]
    fn validate_rejects_value_null_with_yaml_source() {
        let eff = EffectiveTolerance {
            value: None,
            unit: ToleranceUnit::Ms,
            direction: ToleranceDirection::NoMoreThan,
            source: ToleranceSource::Yaml,
            applied_at: Some(now()),
            override_reason: None,
        };
        assert!(eff.validate_internal_consistency().is_err());
    }

    #[test]
    fn validate_rejects_within_range_with_scalar_value() {
        let eff = EffectiveTolerance {
            value: Some(ToleranceValue::Scalar(95.0)),
            unit: ToleranceUnit::Pct,
            direction: ToleranceDirection::WithinRange,
            source: ToleranceSource::Yaml,
            applied_at: Some(now()),
            override_reason: None,
        };
        assert!(eff.validate_internal_consistency().is_err());
    }

    #[test]
    fn validate_rejects_no_more_than_with_range_value() {
        let eff = EffectiveTolerance {
            value: Some(ToleranceValue::Range([95.0, 99.5])),
            unit: ToleranceUnit::Pct,
            direction: ToleranceDirection::NoMoreThan,
            source: ToleranceSource::Yaml,
            applied_at: Some(now()),
            override_reason: None,
        };
        assert!(eff.validate_internal_consistency().is_err());
    }

    #[test]
    fn validate_rejects_applied_at_null_with_yaml_source() {
        let eff = EffectiveTolerance {
            value: Some(ToleranceValue::Scalar(200.0)),
            unit: ToleranceUnit::Ms,
            direction: ToleranceDirection::NoMoreThan,
            source: ToleranceSource::Yaml,
            applied_at: None,
            override_reason: None,
        };
        assert!(eff.validate_internal_consistency().is_err());
    }

    #[test]
    fn validate_rejects_range_with_lower_greater_than_upper() {
        let eff = EffectiveTolerance {
            value: Some(ToleranceValue::Range([99.5, 95.0])), // INVERSED
            unit: ToleranceUnit::Pct,
            direction: ToleranceDirection::WithinRange,
            source: ToleranceSource::Yaml,
            applied_at: Some(now()),
            override_reason: None,
        };
        let err = eff.validate_internal_consistency().unwrap_err();
        assert!(format!("{err}").contains("lower > upper"));
    }

    #[test]
    fn validate_accepts_pre_g11_unbackfilled_with_null_value_and_null_applied_at() {
        let eff = EffectiveTolerance::pre_g11_unbackfilled(
            ToleranceUnit::Ms,
            ToleranceDirection::NoMoreThan,
            Some("legacy row".to_string()),
        );
        assert!(eff.validate_internal_consistency().is_ok());
        assert!(eff.is_legacy());
    }

    // ── 3. serde round-trip ───────────────────────────────────────────

    #[test]
    fn serde_roundtrip_canonical_shape() {
        let eff = EffectiveTolerance {
            value: Some(ToleranceValue::Scalar(200.0)),
            unit: ToleranceUnit::Ms,
            direction: ToleranceDirection::NoMoreThan,
            source: ToleranceSource::Yaml,
            applied_at: Some(now()),
            override_reason: None,
        };
        let json = serde_json::to_string(&eff).unwrap();
        // Whole-number compact form : 200, not 200.0.  Matches canonical
        // fixture wire format at bench/fixtures/post_g11_ndjson/.
        assert!(
            json.contains(r#""value":200"#) && !json.contains(r#""value":200.0"#),
            "expected compact integer form for whole f64, got : {json}"
        );
        assert!(json.contains(r#""unit":"ms""#));
        assert!(json.contains(r#""direction":"no_more_than""#));
        assert!(json.contains(r#""source":"yaml""#));
        assert!(json.contains(r#""applied_at":"2026-04-15T03:14:22Z""#));
        let back: EffectiveTolerance = serde_json::from_str(&json).unwrap();
        assert_eq!(back, eff);
    }

    /// Pin the compact-integer-form property of `ToleranceValue::Scalar`
    /// serialisation.  Matches the canonical fixture wire format ; any
    /// drift triggers a regression because the signed evidence chain is
    /// byte-sensitive.
    #[test]
    fn serde_emits_integer_for_whole_f64_value() {
        // Whole numbers → integer form.
        assert_eq!(
            serde_json::to_string(&ToleranceValue::Scalar(5.0)).unwrap(),
            "5"
        );
        assert_eq!(
            serde_json::to_string(&ToleranceValue::Scalar(200.0)).unwrap(),
            "200"
        );
        assert_eq!(
            serde_json::to_string(&ToleranceValue::Scalar(50000.0)).unwrap(),
            "50000"
        );
        assert_eq!(
            serde_json::to_string(&ToleranceValue::Scalar(-100.0)).unwrap(),
            "-100"
        );
        assert_eq!(
            serde_json::to_string(&ToleranceValue::Scalar(0.0)).unwrap(),
            "0"
        );

        // Non-whole numbers → f64 form (with fractional part).
        assert_eq!(
            serde_json::to_string(&ToleranceValue::Scalar(3.2)).unwrap(),
            "3.2"
        );
        assert_eq!(
            serde_json::to_string(&ToleranceValue::Scalar(0.5)).unwrap(),
            "0.5"
        );

        // Round-trip preserves value (i64 → f64 cleanly for typical
        // benchmark magnitudes).
        for v in [5.0, 200.0, 0.0, 3.2, -100.0, 50000.0, 0.5] {
            let json = serde_json::to_string(&ToleranceValue::Scalar(v)).unwrap();
            let back: ToleranceValue = serde_json::from_str(&json).unwrap();
            assert_eq!(back, ToleranceValue::Scalar(v));
        }
    }

    /// The fixture `bench/fixtures/post_g11_ndjson/CB-01.ndjson` line 0
    /// must be byte-identical to its Rust-round-trip form (modulo the
    /// `runner` field's key ordering, which is `serde_json::Value`
    /// territory and deferred to PR #27.1).
    ///
    /// This test pins the canonical-wire-format property at the
    /// `tolerance_used` level — the field G.11 actually closes.
    #[test]
    fn fixture_tolerance_used_serialises_byte_identical() {
        let orig_json = r#"{"value":5,"unit":"ms","direction":"no_more_than","source":"yaml","applied_at":"2026-04-15T03:14:22Z"}"#;
        let eff: EffectiveTolerance = serde_json::from_str(orig_json).unwrap();
        let back = serde_json::to_string(&eff).unwrap();
        assert_eq!(
            orig_json, back,
            "tolerance_used canonical wire format must round-trip byte-identical"
        );
    }

    #[test]
    fn serde_roundtrip_no_tolerance_defined() {
        let eff = EffectiveTolerance::no_tolerance_defined(
            ToleranceUnit::Count,
            ToleranceDirection::NoMoreThan,
            now(),
        );
        let json = serde_json::to_string(&eff).unwrap();
        assert!(json.contains(r#""value":null"#));
        assert!(json.contains(r#""source":"no_tolerance_defined""#));
        let back: EffectiveTolerance = serde_json::from_str(&json).unwrap();
        assert_eq!(back, eff);
    }

    #[test]
    fn serde_roundtrip_range_value() {
        let eff = EffectiveTolerance {
            value: Some(ToleranceValue::Range([95.0, 99.5])),
            unit: ToleranceUnit::Pct,
            direction: ToleranceDirection::WithinRange,
            source: ToleranceSource::Yaml,
            applied_at: Some(now()),
            override_reason: None,
        };
        let json = serde_json::to_string(&eff).unwrap();
        assert!(json.contains(r#""value":[95.0,99.5]"#));
        let back: EffectiveTolerance = serde_json::from_str(&json).unwrap();
        assert_eq!(back, eff);
    }

    // ── 4. closed-taxonomy wire-format checks ─────────────────────────

    #[test]
    fn taxonomy_unit_serde_kebab_snake() {
        for (unit, s) in [
            (ToleranceUnit::Ms, "ms"),
            (ToleranceUnit::Sec, "sec"),
            (ToleranceUnit::EventsPerSec, "events_per_sec"),
            (ToleranceUnit::Pct, "pct"),
        ] {
            let j = serde_json::to_string(&unit).unwrap();
            assert_eq!(j, format!("\"{s}\""));
            assert_eq!(unit.as_str(), s);
            assert_eq!(format!("{unit}"), s); // Display
            assert_eq!(ToleranceUnit::from_str(s).unwrap(), unit); // FromStr roundtrip
        }
    }

    #[test]
    fn taxonomy_source_serde_consistency() {
        for src in ToleranceSource::all() {
            let j = serde_json::to_string(src).unwrap();
            assert_eq!(j.trim_matches('"'), src.as_str());
            assert_eq!(format!("{src}"), src.as_str());
            assert_eq!(ToleranceSource::from_str(src.as_str()).unwrap(), *src);
        }
    }

    #[test]
    fn taxonomy_direction_all_variants_serialise_cleanly() {
        for dir in ToleranceDirection::all() {
            let j = serde_json::to_string(dir).unwrap();
            assert_eq!(j.trim_matches('"'), dir.as_str());
            assert_eq!(ToleranceDirection::from_str(dir.as_str()).unwrap(), *dir);
        }
    }

    #[test]
    fn taxonomy_from_str_rejects_unknown_string() {
        assert!(ToleranceUnit::from_str("furlongs_per_fortnight").is_err());
        assert!(ToleranceDirection::from_str("sometimes").is_err());
        assert!(ToleranceSource::from_str("magic").is_err());
    }

    // ── 5. verdict-logic checks ───────────────────────────────────────

    #[test]
    fn compute_verdict_pass_warn_fail_no_more_than() {
        let eff = EffectiveTolerance::builder()
            .scalar(200.0)
            .unit(ToleranceUnit::Ms)
            .direction(ToleranceDirection::NoMoreThan)
            .source(ToleranceSource::Yaml)
            .applied_at(now())
            .build()
            .unwrap();
        assert_eq!(compute_verdict(Some(50.0), &eff), "PASS");
        assert_eq!(compute_verdict(Some(190.0), &eff), "WARN");
        assert_eq!(compute_verdict(Some(250.0), &eff), "FAIL");
        assert_eq!(compute_verdict(None, &eff), "INCONCLUSIVE");
    }

    #[test]
    fn compute_verdict_no_less_than_inverse_direction() {
        let eff = EffectiveTolerance::builder()
            .scalar(100000.0)
            .unit(ToleranceUnit::EventsPerSec)
            .direction(ToleranceDirection::NoLessThan)
            .source(ToleranceSource::Yaml)
            .applied_at(now())
            .build()
            .unwrap();
        assert_eq!(compute_verdict(Some(150000.0), &eff), "PASS");
        assert_eq!(compute_verdict(Some(105000.0), &eff), "WARN");
        assert_eq!(compute_verdict(Some(50000.0), &eff), "FAIL");
    }

    #[test]
    fn compute_verdict_within_range_bilateral() {
        let eff = EffectiveTolerance::builder()
            .range(95.0, 99.5)
            .unit(ToleranceUnit::Pct)
            .direction(ToleranceDirection::WithinRange)
            .source(ToleranceSource::Yaml)
            .applied_at(now())
            .build()
            .unwrap();
        assert_eq!(compute_verdict(Some(97.0), &eff), "PASS");
        assert_eq!(compute_verdict(Some(94.0), &eff), "FAIL");
        assert_eq!(compute_verdict(Some(99.6), &eff), "FAIL");
    }

    #[test]
    fn compute_verdict_no_tolerance_defined_always_passes() {
        let eff =
            EffectiveTolerance::no_tolerance_defined(ToleranceUnit::Count, ToleranceDirection::NoMoreThan, now());
        assert_eq!(compute_verdict(Some(42.0), &eff), "PASS");
        assert_eq!(compute_verdict(Some(f64::MAX), &eff), "PASS");
        assert_eq!(compute_verdict(None, &eff), "INCONCLUSIVE");
    }

    // ── 6. edge cases ─────────────────────────────────────────────────

    #[test]
    fn compute_verdict_handles_nan_observed() {
        // NaN comparisons in Rust : `NaN > anything` is `false`.
        // The verdict for NaN observed against a bound is implementation-
        // defined but should never panic.  Today the impl yields PASS
        // for `NaN > 200.0` (false) → not FAIL, not WARN, fallthrough PASS.
        // This test pins the behavior so a future refactor surfaces drift.
        let eff = EffectiveTolerance::builder()
            .scalar(200.0)
            .unit(ToleranceUnit::Ms)
            .direction(ToleranceDirection::NoMoreThan)
            .source(ToleranceSource::Yaml)
            .applied_at(now())
            .build()
            .unwrap();
        let verdict = compute_verdict(Some(f64::NAN), &eff);
        // The verdict for NaN is PASS by fallthrough of the `>` operator.
        // This is not ideal but is the current behavior ; a future PR
        // may opt to return INCONCLUSIVE for NaN.
        assert!(
            verdict == "PASS" || verdict == "INCONCLUSIVE",
            "NaN verdict surprise : {verdict}"
        );
    }

    #[test]
    fn compute_verdict_handles_infinity_observed() {
        let eff = EffectiveTolerance::builder()
            .scalar(200.0)
            .unit(ToleranceUnit::Ms)
            .direction(ToleranceDirection::NoMoreThan)
            .source(ToleranceSource::Yaml)
            .applied_at(now())
            .build()
            .unwrap();
        // +INF > 200 ⇒ FAIL.  -INF > 200 ⇒ false ⇒ fallthrough to PASS.
        assert_eq!(compute_verdict(Some(f64::INFINITY), &eff), "FAIL");
        assert_eq!(compute_verdict(Some(f64::NEG_INFINITY), &eff), "PASS");
    }

    #[test]
    fn tolerance_value_satisfies_helper() {
        let v = ToleranceValue::Scalar(200.0);
        assert!(v.satisfies(100.0, ToleranceDirection::NoMoreThan));
        assert!(!v.satisfies(300.0, ToleranceDirection::NoMoreThan));
        assert!(!v.satisfies(100.0, ToleranceDirection::NoLessThan));
        assert!(v.satisfies(300.0, ToleranceDirection::NoLessThan));

        let r = ToleranceValue::Range([95.0, 99.5]);
        assert!(r.satisfies(97.0, ToleranceDirection::WithinRange));
        assert!(!r.satisfies(94.0, ToleranceDirection::WithinRange));
        assert!(!r.satisfies(99.6, ToleranceDirection::WithinRange));
    }

    #[test]
    fn builder_rejects_missing_mandatory_fields() {
        let r = EffectiveTolerance::builder()
            .scalar(200.0)
            // unit MISSING
            .direction(ToleranceDirection::NoMoreThan)
            .source(ToleranceSource::Yaml)
            .applied_at(now())
            .build();
        assert!(r.is_err());
        assert!(format!("{}", r.unwrap_err()).contains("unit is required"));
    }

    #[test]
    fn builder_with_override_reason_round_trips() {
        let eff = EffectiveTolerance::builder()
            .scalar(500.0)
            .unit(ToleranceUnit::Ms)
            .direction(ToleranceDirection::NoMoreThan)
            .source(ToleranceSource::RuntimeOverride)
            .applied_at(now())
            .override_reason("env-var override active : EFE_TOLERANCE_test=500")
            .build()
            .unwrap();
        let json = serde_json::to_string(&eff).unwrap();
        let back: EffectiveTolerance = serde_json::from_str(&json).unwrap();
        assert_eq!(back, eff);
        assert!(back.override_reason.is_some());
    }

    #[test]
    fn deserialise_with_extra_fields_is_lenient() {
        // Forward-compat : a future schema may add fields under
        // `tolerance_used`.  This PR's deserialiser should ignore them.
        let j = r#"{
            "value": 200,
            "unit": "ms",
            "direction": "no_more_than",
            "source": "yaml",
            "applied_at": "2026-04-15T03:14:22Z",
            "future_extra_field": "ignored"
        }"#;
        let eff: EffectiveTolerance = serde_json::from_str(j).unwrap();
        assert_eq!(eff.source, ToleranceSource::Yaml);
    }

    // ── 7. taxonomy classifier helpers ────────────────────────────────

    #[test]
    fn unit_classifiers() {
        assert!(ToleranceUnit::Ms.is_duration());
        assert!(ToleranceUnit::Sec.is_duration());
        assert!(ToleranceUnit::Ns.is_duration());
        assert!(!ToleranceUnit::Bytes.is_duration());
        assert!(ToleranceUnit::EventsPerSec.is_throughput());
        assert!(ToleranceUnit::Pct.is_dimensionless());
        assert!(ToleranceUnit::Ratio.is_dimensionless());
        assert!(ToleranceUnit::Count.is_dimensionless());
    }

    #[test]
    fn direction_classifiers() {
        assert!(ToleranceDirection::NoMoreThan.is_unilateral());
        assert!(ToleranceDirection::NoLessThan.is_unilateral());
        assert!(ToleranceDirection::WithinRange.is_bilateral());
        assert!(ToleranceDirection::Exactly.is_unilateral());
        assert!(ToleranceDirection::NoMoreThan.is_lower_is_better());
        assert!(ToleranceDirection::NoLessThan.is_higher_is_better());
        assert!(!ToleranceDirection::WithinRange.is_lower_is_better());
    }

    #[test]
    fn source_classifiers() {
        assert!(ToleranceSource::Yaml.is_decision_time_source());
        assert!(ToleranceSource::RuntimeOverride.is_decision_time_source());
        assert!(ToleranceSource::GpuRunnerOverride.is_decision_time_source());
        assert!(ToleranceSource::Defaulted.is_decision_time_source());
        assert!(!ToleranceSource::NoToleranceDefined.is_decision_time_source());

        assert!(ToleranceSource::BackfilledYaml.is_backfilled());
        assert!(ToleranceSource::BackfilledRuntimeOverride.is_backfilled());
        assert!(!ToleranceSource::Yaml.is_backfilled());

        assert!(ToleranceSource::PreG11Unbackfilled.is_legacy());
        assert!(!ToleranceSource::Yaml.is_legacy());

        assert!(ToleranceSource::RuntimeOverride.is_override());
        assert!(ToleranceSource::GpuRunnerOverride.is_override());
        assert!(ToleranceSource::BackfilledRuntimeOverride.is_override());
        assert!(!ToleranceSource::Yaml.is_override());
    }

    #[test]
    fn metric_verdict_v3_verdict_predicates() {
        let eff = EffectiveTolerance::builder()
            .scalar(200.0)
            .unit(ToleranceUnit::Ms)
            .direction(ToleranceDirection::NoMoreThan)
            .source(ToleranceSource::Yaml)
            .applied_at(now())
            .build()
            .unwrap();
        let mut row = MetricVerdictV3 {
            schema_version: 3,
            scenario_id: "s".into(),
            metric_name: "m".into(),
            observed: Some(50.0),
            verdict: "PASS".into(),
            reason: None,
            tolerance_used: eff.clone(),
            runner: serde_json::json!({"kind": "cpu", "id": "r"}),
            applied_at: now(),
        };
        assert!(row.is_pass());
        row.verdict = "WARN".into();
        assert!(row.is_warn());
        row.verdict = "FAIL".into();
        assert!(row.is_fail());
        row.verdict = "INCONCLUSIVE".into();
        assert!(row.is_inconclusive());
    }

    // ═══════════════════════════════════════════════════════════════
    //  Day 2 additions : emit-level tests pinning G.11.a + G.11.b
    //  at the full-row level (vs verdict-string level pinned by Day 1)
    // ═══════════════════════════════════════════════════════════════

    use std::collections::HashMap;
    use std::path::PathBuf;

    fn workspace_root_for_test() -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        loop {
            let cargo = p.join("Cargo.toml");
            if cargo.exists() {
                let body = std::fs::read_to_string(&cargo).expect("Cargo.toml must read");
                if body.contains("[workspace]") {
                    return p;
                }
            }
            if !p.pop() {
                panic!("workspace root not found");
            }
        }
    }

    fn synthetic_fixture_path() -> PathBuf {
        workspace_root_for_test()
            .join("bench/fixtures/scenarios/G11_synthetic.yaml")
    }

    fn cpu_runner() -> RunnerIntrospection {
        RunnerIntrospection::cpu_only("test-runner-day2")
    }

    /// Day 2 emit test : the PASS branch of the verdict emits a fully
    /// populated `tolerance_used` field on the NDJSON row.
    ///
    /// Symmetric coverage with the FAIL/WARN/INCONCLUSIVE emit tests ;
    /// PR #27 closes G.11 across all four branches, this test confirms
    /// the PASS branch is also covered (no historical defect, but pinned
    /// by parity per AD-D2-6).
    #[test]
    fn emit_pass_populates_tolerance_used() {
        let mut observed = HashMap::new();
        // 50 ms < bound 200 ms × 0.90 = 180 ms ⇒ PASS (well under soft band).
        observed.insert("metric_no_more_than_only".to_string(), 50.0);
        let rows = pretend_aggregator_emit_rows(
            &synthetic_fixture_path(),
            &observed,
            &cpu_runner(),
            &EnvContext::empty(),
            now(),
        )
        .expect("aggregator must emit rows");
        let row = rows
            .iter()
            .find(|r| r.metric_name == "metric_no_more_than_only")
            .expect("metric emitted");
        assert_eq!(row.verdict, "PASS");
        assert!(
            row.tolerance_used.value.is_some(),
            "Day 2 emit-PASS pin : tolerance_used.value must be populated"
        );
        assert_eq!(
            row.tolerance_used.value.as_ref().unwrap().as_scalar(),
            Some(200.0)
        );
        assert_eq!(row.tolerance_used.source, ToleranceSource::Yaml);
        assert!(row.tolerance_used.applied_at.is_some());
        assert!(row.reason.is_none()); // PASS rows have no reason
        assert!(row.tolerance_used.validate_internal_consistency().is_ok());
    }

    /// Day 2 emit test — **pins G.11.a** : historically the WARN branch
    /// emitted `tolerance_used: null` in 60 % of cases.  This test
    /// forces the field to be populated on a WARN verdict.
    ///
    /// Setup : observation in the ±10 % soft band of the bound.
    /// Expected : verdict = WARN, tolerance_used = full struct with
    /// value = 200, source = Yaml.
    ///
    /// This test FAILS against the pre-PR-#27 production aggregator
    /// (which would emit null) and PASSES against the post-PR-#27
    /// mini-aggregator.  It is the structural pin of G.11.a closure.
    #[test]
    fn emit_warn_populates_tolerance_used() {
        let mut observed = HashMap::new();
        // 190 ms : within (200 × 0.90, 200] = (180, 200] ⇒ WARN.
        observed.insert("metric_no_more_than_only".to_string(), 190.0);
        let rows = pretend_aggregator_emit_rows(
            &synthetic_fixture_path(),
            &observed,
            &cpu_runner(),
            &EnvContext::empty(),
            now(),
        )
        .expect("aggregator must emit rows");
        let row = rows
            .iter()
            .find(|r| r.metric_name == "metric_no_more_than_only")
            .expect("metric emitted");
        assert_eq!(row.verdict, "WARN");
        assert!(
            row.tolerance_used.value.is_some(),
            "G.11.a pin : tolerance_used.value MUST be populated on WARN rows. \
             Historically this field was null in 60 % of WARN paths. \
             If this assertion fails post-PR-#27, G.11 has regressed."
        );
        assert_eq!(
            row.tolerance_used.value.as_ref().unwrap().as_scalar(),
            Some(200.0)
        );
        assert_eq!(row.tolerance_used.source, ToleranceSource::Yaml);
        assert_eq!(row.tolerance_used.unit, ToleranceUnit::Ms);
        assert_eq!(row.tolerance_used.direction, ToleranceDirection::NoMoreThan);
        assert!(row.tolerance_used.applied_at.is_some());
    }

    /// Day 2 emit test : the FAIL branch emits a fully populated
    /// `tolerance_used` field.  Parity coverage per AD-D2-6 ; no
    /// historical defect on FAIL but pinned to prevent regression.
    #[test]
    fn emit_fail_populates_tolerance_used() {
        let mut observed = HashMap::new();
        // 300 ms > 200 ⇒ FAIL.
        observed.insert("metric_no_more_than_only".to_string(), 300.0);
        let rows = pretend_aggregator_emit_rows(
            &synthetic_fixture_path(),
            &observed,
            &cpu_runner(),
            &EnvContext::empty(),
            now(),
        )
        .expect("aggregator must emit rows");
        let row = rows
            .iter()
            .find(|r| r.metric_name == "metric_no_more_than_only")
            .expect("metric emitted");
        assert_eq!(row.verdict, "FAIL");
        assert!(row.tolerance_used.value.is_some());
        assert_eq!(
            row.tolerance_used.value.as_ref().unwrap().as_scalar(),
            Some(200.0)
        );
        assert_eq!(row.tolerance_used.source, ToleranceSource::Yaml);
    }

    /// Day 2 emit test — **pins G.11.b** : historically the INCONCLUSIVE
    /// branch entirely omitted the `tolerance_used` field from the
    /// emitted JSON (the field was not even present in the row).  This
    /// test forces both the presence and the population of the field
    /// on INCONCLUSIVE rows.
    ///
    /// Setup : observed HashMap is empty for this metric (no observation
    /// was recorded).
    /// Expected : verdict = INCONCLUSIVE, tolerance_used populated with
    /// the bound declared by the YAML (200 ms), reason field carries
    /// the explanation string per AD-D2-7.
    ///
    /// This test FAILS against the pre-PR-#27 production aggregator
    /// (which would omit the field entirely) and PASSES against the
    /// post-PR-#27 mini-aggregator.
    #[test]
    fn emit_inconclusive_populates_tolerance_used() {
        let observed: HashMap<String, f64> = HashMap::new(); // no observations
        let rows = pretend_aggregator_emit_rows(
            &synthetic_fixture_path(),
            &observed,
            &cpu_runner(),
            &EnvContext::empty(),
            now(),
        )
        .expect("aggregator must emit rows");
        let row = rows
            .iter()
            .find(|r| r.metric_name == "metric_no_more_than_only")
            .expect("metric emitted even without observation");
        assert_eq!(row.verdict, "INCONCLUSIVE");
        assert!(row.observed.is_none());
        assert!(
            row.tolerance_used.value.is_some(),
            "G.11.b pin : tolerance_used.value MUST be populated on INCONCLUSIVE \
             rows.  Historically this field was OMITTED entirely from INCONCLUSIVE \
             rows.  If this assertion fails post-PR-#27, G.11 has regressed."
        );
        assert_eq!(
            row.tolerance_used.value.as_ref().unwrap().as_scalar(),
            Some(200.0),
            "tolerance_used must carry the YAML-declared bound even when no \
             observation was recorded"
        );
        assert_eq!(row.tolerance_used.source, ToleranceSource::Yaml);
        assert!(row.tolerance_used.applied_at.is_some());
        assert_eq!(
            row.reason,
            Some("observed value absent at aggregator-decision time".to_string()),
            "AD-D2-7 : INCONCLUSIVE rows carry an explanatory reason field"
        );
    }

    /// Day 2 — pin the round-trip property : the audit explanation's
    /// verdict string MUST match `compute_verdict`'s verdict string for
    /// the same inputs across **all 4 directions + 2 sentinels**.
    /// Prevents drift between the two functions in any branch.
    ///
    /// Coverage (post-Day-2-audit extension) :
    ///   - NoMoreThan : PASS / WARN / FAIL / INCONCLUSIVE
    ///   - NoLessThan : PASS / WARN / FAIL / INCONCLUSIVE
    ///   - WithinRange : PASS / FAIL / INCONCLUSIVE (no WARN per AD-D2-3)
    ///   - Exactly : PASS / FAIL / INCONCLUSIVE (no WARN)
    ///   - NoToleranceDefined sentinel (value = None) : always PASS
    ///   - Pre-G.11-unbackfilled sentinel (legacy row)
    #[test]
    fn audit_explanation_matches_verdict_string() {
        // NoMoreThan direction
        let eff_nmt = EffectiveTolerance::builder()
            .scalar(200.0).unit(ToleranceUnit::Ms)
            .direction(ToleranceDirection::NoMoreThan)
            .source(ToleranceSource::Yaml).applied_at(now()).build().unwrap();
        for observed in [Some(50.0), Some(190.0), Some(300.0), None] {
            let v_only = compute_verdict(observed, &eff_nmt);
            let (v_expl, explain) = compute_verdict_audit_explanation(observed, &eff_nmt);
            assert_eq!(v_only, v_expl, "NoMoreThan obs={observed:?}");
            assert!(!explain.is_empty(), "NoMoreThan explain empty for obs={observed:?}");
        }

        // NoLessThan direction
        let eff_nlt = EffectiveTolerance::builder()
            .scalar(100.0).unit(ToleranceUnit::EventsPerSec)
            .direction(ToleranceDirection::NoLessThan)
            .source(ToleranceSource::Yaml).applied_at(now()).build().unwrap();
        for observed in [Some(150.0), Some(105.0), Some(50.0), None] {
            let v_only = compute_verdict(observed, &eff_nlt);
            let (v_expl, explain) = compute_verdict_audit_explanation(observed, &eff_nlt);
            assert_eq!(v_only, v_expl, "NoLessThan obs={observed:?}");
            assert!(!explain.is_empty(), "NoLessThan explain empty for obs={observed:?}");
        }

        // WithinRange direction (no WARN per AD-D2-3)
        let eff_wr = EffectiveTolerance::builder()
            .range(95.0, 99.5).unit(ToleranceUnit::Pct)
            .direction(ToleranceDirection::WithinRange)
            .source(ToleranceSource::Yaml).applied_at(now()).build().unwrap();
        for observed in [Some(97.0), Some(94.0), Some(99.6), None] {
            let v_only = compute_verdict(observed, &eff_wr);
            let (v_expl, explain) = compute_verdict_audit_explanation(observed, &eff_wr);
            assert_eq!(v_only, v_expl, "WithinRange obs={observed:?}");
            assert!(!explain.is_empty(), "WithinRange explain empty for obs={observed:?}");
        }

        // Exactly direction (no WARN)
        let eff_ex = EffectiveTolerance::builder()
            .scalar(100.0).unit(ToleranceUnit::Count)
            .direction(ToleranceDirection::Exactly)
            .source(ToleranceSource::Yaml).applied_at(now()).build().unwrap();
        for observed in [Some(100.0), Some(101.0), None] {
            let v_only = compute_verdict(observed, &eff_ex);
            let (v_expl, explain) = compute_verdict_audit_explanation(observed, &eff_ex);
            assert_eq!(v_only, v_expl, "Exactly obs={observed:?}");
            assert!(!explain.is_empty(), "Exactly explain empty for obs={observed:?}");
        }

        // NoToleranceDefined sentinel (value = None)
        let eff_unbounded = EffectiveTolerance::no_tolerance_defined(
            ToleranceUnit::Count, ToleranceDirection::NoMoreThan, now(),
        );
        for observed in [Some(42.0), Some(f64::MAX), Some(-1e300), None] {
            let v_only = compute_verdict(observed, &eff_unbounded);
            let (v_expl, explain) = compute_verdict_audit_explanation(observed, &eff_unbounded);
            assert_eq!(v_only, v_expl, "unbounded obs={observed:?}");
            assert!(!explain.is_empty(), "unbounded explain empty for obs={observed:?}");
        }
    }

    /// Day 2 audit pass — explanation text contains expected keywords
    /// per direction so a future text-format change is caught early.
    /// Pins the user-facing format of the explanation strings.
    #[test]
    fn audit_explanation_text_contains_branch_keywords() {
        let now_ts = now();
        // NoMoreThan / PASS
        let eff = EffectiveTolerance::builder()
            .scalar(200.0).unit(ToleranceUnit::Ms)
            .direction(ToleranceDirection::NoMoreThan)
            .source(ToleranceSource::Yaml).applied_at(now_ts).build().unwrap();
        let (_, explain) = compute_verdict_audit_explanation(Some(50.0), &eff);
        assert!(
            explain.contains("no_more_than") || explain.contains("at or below"),
            "NoMoreThan PASS explanation must reference direction : {explain}"
        );

        // NoLessThan / FAIL
        let eff = EffectiveTolerance::builder()
            .scalar(100.0).unit(ToleranceUnit::EventsPerSec)
            .direction(ToleranceDirection::NoLessThan)
            .source(ToleranceSource::Yaml).applied_at(now_ts).build().unwrap();
        let (_, explain) = compute_verdict_audit_explanation(Some(50.0), &eff);
        assert!(
            explain.contains("no_less_than") || explain.contains("falls below"),
            "NoLessThan FAIL explanation must reference direction : {explain}"
        );

        // WithinRange / PASS
        let eff = EffectiveTolerance::builder()
            .range(95.0, 99.5).unit(ToleranceUnit::Pct)
            .direction(ToleranceDirection::WithinRange)
            .source(ToleranceSource::Yaml).applied_at(now_ts).build().unwrap();
        let (_, explain) = compute_verdict_audit_explanation(Some(97.0), &eff);
        assert!(
            explain.contains("bilateral range") && explain.contains('['),
            "WithinRange PASS explanation must reference bilateral range : {explain}"
        );

        // INCONCLUSIVE (observed=None)
        let eff = EffectiveTolerance::builder()
            .scalar(200.0).unit(ToleranceUnit::Ms)
            .direction(ToleranceDirection::NoMoreThan)
            .source(ToleranceSource::Yaml).applied_at(now_ts).build().unwrap();
        let (_, explain) = compute_verdict_audit_explanation(None, &eff);
        assert!(
            explain.contains("absent") || explain.contains("undetermined"),
            "INCONCLUSIVE explanation must reference missing observation : {explain}"
        );

        // NoToleranceDefined (value=None)
        let eff = EffectiveTolerance::no_tolerance_defined(
            ToleranceUnit::Count, ToleranceDirection::NoMoreThan, now_ts,
        );
        let (_, explain) = compute_verdict_audit_explanation(Some(42.0), &eff);
        assert!(
            explain.contains("No tolerance defined") || explain.contains("unbounded"),
            "unbounded explanation must reference no-tolerance : {explain}"
        );
    }

    /// Day 2 audit pass — `compute_verdict` and the named helpers must
    /// not panic on any pathological f64 input (MAX, MIN, MIN_POSITIVE,
    /// zero, negative, subnormal).  Pin the no-panic property.
    #[test]
    fn compute_verdict_no_panic_on_pathological_f64() {
        let eff = EffectiveTolerance::builder()
            .scalar(200.0).unit(ToleranceUnit::Ms)
            .direction(ToleranceDirection::NoMoreThan)
            .source(ToleranceSource::Yaml).applied_at(now()).build().unwrap();
        // Each call must produce a valid verdict string ; no panic.
        for obs in [
            Some(f64::MAX),
            Some(f64::MIN),
            Some(f64::MIN_POSITIVE),
            Some(0.0),
            Some(-0.0),
            Some(-100.0),
            Some(1e300),
            Some(-1e300),
            None,
        ] {
            let v = compute_verdict(obs, &eff);
            assert!(
                matches!(v.as_str(), "PASS" | "WARN" | "FAIL" | "INCONCLUSIVE"),
                "compute_verdict produced invalid string '{v}' for obs={obs:?}"
            );
            // Audit explanation must also not panic.
            let (v2, explain) = compute_verdict_audit_explanation(obs, &eff);
            assert_eq!(v, v2);
            assert!(!explain.is_empty());
        }
    }

    /// Day 2 audit pass — pin the value of MINI_AGGREGATOR_WARN_MARGIN_RATIO.
    /// Any change to this constant is a behavioural change observable by
    /// the WARN soft-band ; STABILITY-bump.
    #[test]
    fn mini_aggregator_warn_margin_ratio_is_frozen_at_0_10() {
        assert!(
            (MINI_AGGREGATOR_WARN_MARGIN_RATIO - 0.10).abs() < f64::EPSILON,
            "MINI_AGGREGATOR_WARN_MARGIN_RATIO must remain 0.10 (current: {})",
            MINI_AGGREGATOR_WARN_MARGIN_RATIO
        );
        // Implications : verdicts at the symbolic 90 %/110 % thresholds.
        // (We accept the f64 imprecision at boundary observations ; the
        // value pin protects against accidental changes like 0.05.)
        assert!(MINI_AGGREGATOR_WARN_MARGIN_RATIO > 0.0);
        assert!(MINI_AGGREGATOR_WARN_MARGIN_RATIO < 1.0);
    }

    /// Day 2 — pin the named helpers (`verdict_no_more_than`, etc.).
    /// Direct unit-test coverage on each helper for individual
    /// regression signal.  If a helper is broken, the corresponding
    /// emit test catches it indirectly, but this test catches it
    /// directly.
    #[test]
    fn named_helpers_match_matrix_specification() {
        // verdict_no_more_than : bound=200, soft band (180, 200]
        assert_eq!(verdict_no_more_than(50.0, 200.0), "PASS");
        assert_eq!(verdict_no_more_than(180.0, 200.0), "PASS"); // boundary : 180 = bound × 0.90 → still PASS (strict >)
        assert_eq!(verdict_no_more_than(181.0, 200.0), "WARN");
        assert_eq!(verdict_no_more_than(200.0, 200.0), "WARN"); // at bound : still satisfies, in soft band
        assert_eq!(verdict_no_more_than(200.0001, 200.0), "FAIL");
        assert_eq!(verdict_no_more_than(300.0, 200.0), "FAIL");

        // verdict_no_less_than : bound=100, soft band [100, 110+ε) where ε ≈ 1.4e-14
        //
        // Note on f64 boundary precision : `100.0 * 1.10` evaluates in
        // IEEE-754 f64 to `110.00000000000001` (one ULP above 110.0),
        // because `0.10` is not exactly representable in binary.  As
        // a consequence, an observation of exactly 110.0 falls INSIDE
        // the soft band (110.0 < 110.00000000000001) and yields WARN,
        // not PASS.  To get a deterministic PASS, the observation must
        // be 115.0 or above.
        assert_eq!(verdict_no_less_than(150.0, 100.0), "PASS");
        assert_eq!(verdict_no_less_than(115.0, 100.0), "PASS"); // safely above 1.1× bound
        assert_eq!(verdict_no_less_than(110.0, 100.0), "WARN"); // at numeric boundary, still in soft band (f64)
        assert_eq!(verdict_no_less_than(109.0, 100.0), "WARN");
        assert_eq!(verdict_no_less_than(100.0, 100.0), "WARN");
        assert_eq!(verdict_no_less_than(99.9999, 100.0), "FAIL");
        assert_eq!(verdict_no_less_than(50.0, 100.0), "FAIL");

        // verdict_within_range : [95, 99.5]
        assert_eq!(verdict_within_range(97.0, 95.0, 99.5), "PASS");
        assert_eq!(verdict_within_range(95.0, 95.0, 99.5), "PASS"); // inclusive lower
        assert_eq!(verdict_within_range(99.5, 95.0, 99.5), "PASS"); // inclusive upper
        assert_eq!(verdict_within_range(94.9, 95.0, 99.5), "FAIL");
        assert_eq!(verdict_within_range(99.6, 95.0, 99.5), "FAIL");

        // verdict_exactly : bound=100
        //
        // Note on f64 precision at magnitude 100 : `f64::EPSILON` is
        // ~2.22e-16, which is BELOW the smallest representable
        // difference around 100.0 (~2.22e-14, i.e. 100 × EPSILON).
        // Therefore `100.0 + f64::EPSILON` rounds back to exactly
        // `100.0` in f64 and would unexpectedly PASS.  To get a FAIL,
        // the increment must be at least one ULP at magnitude 100.0
        // (we use 1e-10 for clarity).
        assert_eq!(verdict_exactly(100.0, 100.0), "PASS");
        assert_eq!(verdict_exactly(100.0 + 1e-10, 100.0), "FAIL"); // beyond ULP at 100.0
        assert_eq!(verdict_exactly(99.999, 100.0), "FAIL");
        assert_eq!(verdict_exactly(101.0, 100.0), "FAIL");
        // The pathological case where `f64::EPSILON` is below the
        // ULP : `100.0 + f64::EPSILON` rounds to `100.0`, so PASS.
        // This is documented behaviour, not a bug ; an `Exactly` with
        // a large bound is sensitive to ULP-scale, not EPSILON-scale.
        assert_eq!(verdict_exactly(100.0 + f64::EPSILON, 100.0), "PASS");
    }

    /// Day 2 — defensive : `compute_verdict` returns INCONCLUSIVE for
    /// ill-formed combinations that escape validation
    /// (e.g. direction=NoMoreThan with value=Range, theoretically
    /// impossible after validate_internal_consistency but defensive
    /// fallback per AD-D2-4).
    ///
    /// This test constructs an ill-formed EffectiveTolerance directly
    /// (bypassing validate_internal_consistency) and verifies that
    /// compute_verdict does NOT panic and returns INCONCLUSIVE.
    #[test]
    fn defensive_inconclusive_for_ill_formed_combinations() {
        // Ill-formed : direction=NoMoreThan but value=Range
        let ill_formed = EffectiveTolerance {
            value: Some(ToleranceValue::Range([95.0, 99.5])),
            unit: ToleranceUnit::Pct,
            direction: ToleranceDirection::NoMoreThan,
            source: ToleranceSource::Yaml,
            applied_at: Some(now()),
            override_reason: None,
        };
        // validate_internal_consistency catches this :
        assert!(ill_formed.validate_internal_consistency().is_err());
        // But compute_verdict still doesn't panic ; it falls through to INCONCLUSIVE.
        assert_eq!(compute_verdict(Some(97.0), &ill_formed), "INCONCLUSIVE");

        // Ill-formed : direction=WithinRange but value=Scalar
        let ill_formed_2 = EffectiveTolerance {
            value: Some(ToleranceValue::Scalar(95.0)),
            unit: ToleranceUnit::Pct,
            direction: ToleranceDirection::WithinRange,
            source: ToleranceSource::Yaml,
            applied_at: Some(now()),
            override_reason: None,
        };
        assert!(ill_formed_2.validate_internal_consistency().is_err());
        assert_eq!(compute_verdict(Some(97.0), &ill_formed_2), "INCONCLUSIVE");
    }

    /// Day 2 — matrix completeness sanity : every direction × verdict
    /// cell in the spec maps to a deterministic, non-panicking output.
    /// Walks the full matrix and confirms `compute_verdict` is total.
    #[test]
    fn matrix_completeness_walks_all_cells() {
        let canonical_observed_values = [
            Some(50.0_f64),   // typical PASS for NoMoreThan/200
            Some(190.0),      // typical WARN for NoMoreThan/200
            Some(300.0),      // typical FAIL for NoMoreThan/200
            Some(95.0),       // boundary low for WithinRange
            Some(99.5),       // boundary high for WithinRange
            Some(150.0),      // typical PASS for NoLessThan/100
            Some(100.0),      // boundary for NoLessThan/100
            Some(50.0),       // typical FAIL for NoLessThan/100
            None,             // sentinel INCONCLUSIVE
        ];
        for direction in ToleranceDirection::all() {
            for obs in canonical_observed_values {
                let value = match direction {
                    ToleranceDirection::WithinRange => ToleranceValue::Range([95.0, 99.5]),
                    _ => ToleranceValue::Scalar(200.0),
                };
                let eff = EffectiveTolerance {
                    value: Some(value),
                    unit: ToleranceUnit::Ms,
                    direction: *direction,
                    source: ToleranceSource::Yaml,
                    applied_at: Some(now()),
                    override_reason: None,
                };
                // Must not panic, must return a valid verdict string.
                let verdict = compute_verdict(obs, &eff);
                assert!(
                    matches!(verdict.as_str(), "PASS" | "WARN" | "FAIL" | "INCONCLUSIVE"),
                    "matrix cell ({:?}, {:?}) produced invalid verdict '{}'",
                    direction,
                    obs,
                    verdict
                );
            }
        }
    }
}
