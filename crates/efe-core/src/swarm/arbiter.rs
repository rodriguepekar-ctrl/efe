//! Resource arbiter: enforces per-request wall-clock and energy budgets.
//!
//! The arbiter is **not** responsible for HBM enforcement (delegated to
//! the memory pool — M2) nor for tenant-level rate limiting (delegated
//! to the API layer — M10). Its single concern is making sure the
//! controller does not continue executing after a budget is exhausted.
//!
//! # Energy metering
//!
//! Energy is read through an [`EnergyMeter`] trait. The default
//! [`NullEnergyMeter`] always returns `0.0`, effectively disabling the
//! energy check. A real NVML-backed meter lives in M11 and is injected
//! at server startup.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::errors::{EfeError, Result};
use crate::types::ResourceBudget;

/// Reads instantaneous energy consumption (Watt-hours).
///
/// Implementations must be cheap to call (< 1 ms) because the arbiter
/// may call them once per round.
pub trait EnergyMeter: Send + Sync {
    /// Cumulative energy since process start, in Watt-hours.
    ///
    /// # Errors
    ///
    /// Returns a descriptive string when the underlying driver is
    /// unavailable. The arbiter treats this as "best-effort" and does
    /// not fail the request.
    fn energy_now_wh(&self) -> std::result::Result<f32, String>;
}

/// Default energy meter that is always unavailable.
#[derive(Clone, Default)]
pub struct NullEnergyMeter;

impl EnergyMeter for NullEnergyMeter {
    fn energy_now_wh(&self) -> std::result::Result<f32, String> {
        Err("NullEnergyMeter: not available".into())
    }
}

/// Factory for per-request arbiter sessions.
#[derive(Clone)]
pub struct SwarmArbiter {
    meter: Arc<dyn EnergyMeter>,
}

impl SwarmArbiter {
    /// Build a new arbiter with the given energy meter.
    #[must_use]
    pub fn new(meter: Arc<dyn EnergyMeter>) -> Self {
        Self { meter }
    }

    /// Convenience constructor with a no-op energy meter.
    #[must_use]
    pub fn without_energy_metering() -> Self {
        Self::new(Arc::new(NullEnergyMeter))
    }

    /// Open a session bound to the given budget.
    pub fn new_session(&self, budget: ResourceBudget) -> ArbiterSession {
        ArbiterSession {
            budget,
            start: Instant::now(),
            start_energy_wh: self.meter.energy_now_wh().ok(),
            meter: self.meter.clone(),
        }
    }
}

/// An in-flight arbiter session, owned for the lifetime of one request.
pub struct ArbiterSession {
    budget: ResourceBudget,
    start: Instant,
    start_energy_wh: Option<f32>,
    meter: Arc<dyn EnergyMeter>,
}

impl ArbiterSession {
    /// Verify that no budget has been exhausted.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::BudgetExhausted`] with the specific kind
    /// (`wall_clock` or `energy`).
    pub fn check(&self, elapsed_override: Duration) -> Result<()> {
        // Wall-clock check (authoritative).
        let elapsed_ms = elapsed_override.as_secs_f32() * 1000.0;
        if elapsed_ms > self.budget.max_wall_clock_ms as f32 {
            return Err(EfeError::BudgetExhausted { kind: "wall_clock" });
        }

        // Energy check (best-effort — skip silently when unavailable).
        // **Audit-B8 fix #16**: a buggy meter could return non-finite
        // energy (NaN or ±Inf). The naive comparison `NaN > x` is
        // false, so without this guard we'd silently pass even an
        // energy-budget violation. We now log a warning and treat the
        // reading as "unavailable" — exactly as if the meter had
        // returned `Err`.
        if let Some(start) = self.start_energy_wh {
            if let Ok(now_wh) = self.meter.energy_now_wh() {
                let delta = now_wh - start;
                if !delta.is_finite() {
                    tracing::warn!(
                        now_wh,
                        start_wh = start,
                        "energy meter produced non-finite delta; skipping budget check"
                    );
                } else if delta > self.budget.max_energy_wh {
                    return Err(EfeError::BudgetExhausted { kind: "energy" });
                }
            }
        }

        Ok(())
    }

    /// Remaining wall-clock time in milliseconds (saturating).
    #[must_use]
    pub fn remaining_ms(&self) -> u32 {
        let e = self.start.elapsed().as_secs_f32() * 1000.0;
        (self.budget.max_wall_clock_ms as f32 - e).max(0.0) as u32
    }

    /// Access the underlying budget (read-only).
    #[must_use]
    pub fn budget(&self) -> &ResourceBudget {
        &self.budget
    }

    /// Wall-clock elapsed since session start.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedEnergyMeter(f32);
    impl EnergyMeter for FixedEnergyMeter {
        fn energy_now_wh(&self) -> std::result::Result<f32, String> {
            Ok(self.0)
        }
    }

    #[test]
    fn session_passes_when_within_budget() {
        let arb = SwarmArbiter::without_energy_metering();
        let budget = ResourceBudget {
            max_wall_clock_ms: 1000,
            ..ResourceBudget::default()
        };
        let session = arb.new_session(budget);
        session.check(Duration::from_millis(10)).expect("in budget");
    }

    #[test]
    fn session_fails_on_wall_clock() {
        let arb = SwarmArbiter::without_energy_metering();
        let budget = ResourceBudget {
            max_wall_clock_ms: 100,
            ..ResourceBudget::default()
        };
        let session = arb.new_session(budget);
        let err = session.check(Duration::from_millis(500)).unwrap_err();
        assert_eq!(err.code(), "budget_exhausted");
        if let EfeError::BudgetExhausted { kind } = err {
            assert_eq!(kind, "wall_clock");
        } else {
            panic!("expected BudgetExhausted");
        }
    }

    #[test]
    fn session_energy_best_effort_when_unavailable() {
        let arb = SwarmArbiter::without_energy_metering();
        let budget = ResourceBudget {
            max_wall_clock_ms: 1000,
            max_energy_wh: 0.001,
            ..ResourceBudget::default()
        };
        let session = arb.new_session(budget);
        // Energy meter is unavailable → no failure, only wall_clock check runs.
        session
            .check(Duration::from_millis(10))
            .expect("best-effort");
    }

    #[test]
    fn session_fails_on_energy() {
        use std::sync::atomic::{AtomicU32, Ordering};

        /// Meter reporting `initial` on first read, then
        /// `initial + step_wh` on every subsequent read — simulates
        /// energy accumulating during execution.
        struct GrowingMeter {
            reads: AtomicU32,
            initial: f32,
            step: f32,
        }
        impl EnergyMeter for GrowingMeter {
            fn energy_now_wh(&self) -> std::result::Result<f32, String> {
                let n = self.reads.fetch_add(1, Ordering::Relaxed);
                if n == 0 {
                    Ok(self.initial)
                } else {
                    Ok(self.initial + self.step * n as f32)
                }
            }
        }

        // Start at 100 Wh, +10 Wh per subsequent read.
        let meter = Arc::new(GrowingMeter {
            reads: AtomicU32::new(0),
            initial: 100.0,
            step: 10.0,
        });
        let arb = SwarmArbiter::new(meter);
        let budget = ResourceBudget {
            max_wall_clock_ms: 100_000,
            max_energy_wh: 5.0,
            ..ResourceBudget::default()
        };
        let session = arb.new_session(budget);
        // After one growth step (100 -> 110), delta = 10 Wh > 5 Wh budget.
        let err = session.check(Duration::from_millis(1)).unwrap_err();
        if let EfeError::BudgetExhausted { kind } = err {
            assert_eq!(kind, "energy");
        } else {
            panic!("expected energy budget exhausted, got {err:?}");
        }
    }

    #[test]
    fn session_energy_does_not_fail_when_below_budget() {
        let arb = SwarmArbiter::new(Arc::new(FixedEnergyMeter(42.0)));
        let budget = ResourceBudget {
            max_wall_clock_ms: 100_000,
            max_energy_wh: 5.0,
            ..ResourceBudget::default()
        };
        let session = arb.new_session(budget);
        // Fixed meter: delta is always 0.
        session.check(Duration::from_millis(1)).expect("no growth");
    }

    #[test]
    fn remaining_ms_decreases() {
        let arb = SwarmArbiter::without_energy_metering();
        let budget = ResourceBudget {
            max_wall_clock_ms: 1000,
            ..ResourceBudget::default()
        };
        let session = arb.new_session(budget);
        let r1 = session.remaining_ms();
        std::thread::sleep(Duration::from_millis(20));
        let r2 = session.remaining_ms();
        assert!(r2 <= r1);
    }
}
