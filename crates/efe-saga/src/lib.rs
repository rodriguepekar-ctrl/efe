//! Sprint Φ-D-LATACT D93 — Saga orchestrator (= distributed transaction).
//!
//! Cf. `docs/EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md` § 9.
//!
//! # Pattern
//!
//! A `Saga` runs N `SagaStep`s sequentially. If step k fails, the saga
//! rolls back by calling `compensate()` on steps (k-1, k-2, ..., 0) in
//! reverse order. Each step is :
//!
//! * **Idempotent** : safe to retry with the same `idempotency_key`.
//! * **Compensable** : has a side-effect-undoing inverse action.
//!
//! # Use cases
//!
//! 1. **Tenant deploy** (P2 self-service POC) :
//!    `Provision PG → Migrate schema → Seed fixtures → Notify tenant → Mark active`
//! 2. **GDPR delete cascade** :
//!    `Audit start → Drop sessions → Tombstone audit_events → Purge Redis → Purge S3 → Audit complete`
//!
//! # SLO target
//!
//! Rollback p99 ≤ 10 min (`SLO-P2-ROLLBACK`).
//!
//! # Doctrines opposables
//!
//! * **D-LATACT-7 — Rollback atomique** : tout step failure ⇒ rollback
//!   complete before the saga returns.
//! * **D-LATACT-8 — GDPR delete ≤ 30 days** : same orchestrator,
//!   extended cascade.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use thiserror::Error;

/// Generic step in a saga.
///
/// Each step has a `name()` (= for audit logs) and two methods :
/// `execute()` does the forward action, `compensate()` undoes it.
#[async_trait]
pub trait SagaStep: Send + Sync {
    /// Step name (= used in audit chain : `activation.saga_step_<name>`).
    fn name(&self) -> &'static str;

    /// Execute the forward action. Must be idempotent.
    async fn execute(&self, ctx: &SagaContext) -> Result<(), SagaError>;

    /// Compensate (= undo) the forward action. Called in reverse order
    /// on saga rollback. Must also be idempotent.
    async fn compensate(&self, ctx: &SagaContext) -> Result<(), SagaError>;
}

/// Context shared across all steps of a single saga run.
///
/// Holds the `idempotency_key` (= for retry safety) and arbitrary
/// state passed between steps via the `state` mutex (= future feature).
#[derive(Debug)]
pub struct SagaContext {
    pub saga_id: uuid::Uuid,
    pub idempotency_key: String,
    pub tenant_id: String,
    pub started_at: Instant,
}

impl SagaContext {
    pub fn new(tenant_id: String, idempotency_key: String) -> Self {
        Self {
            saga_id: uuid::Uuid::new_v4(),
            idempotency_key,
            tenant_id,
            started_at: Instant::now(),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }
}

#[derive(Debug, Error)]
pub enum SagaError {
    #[error("step {step_name} failed at index {step_index} : {message}")]
    StepFailed {
        step_index: usize,
        step_name: &'static str,
        message: String,
    },
    #[error("compensation of step {step_name} (index {step_index}) failed : {message}")]
    CompensationFailed {
        step_index: usize,
        step_name: &'static str,
        message: String,
    },
    #[error("saga timeout after {elapsed_ms} ms (cap = {cap_ms} ms)")]
    Timeout { elapsed_ms: u64, cap_ms: u64 },
    #[error("saga aborted : {message}")]
    Aborted { message: String },
}

/// Outcome of a saga run.
#[derive(Debug)]
pub enum SagaOutcome {
    /// All steps executed successfully.
    Committed { steps_run: usize, elapsed_ms: u64 },
    /// Step at `failed_at` failed ; rollback completed successfully.
    RolledBack {
        failed_at: usize,
        compensated_steps: usize,
        rollback_elapsed_ms: u64,
        error: SagaError,
    },
    /// A compensation step itself failed (= rare but catastrophic).
    CompensationFailure {
        failed_at: usize,
        compensation_failed_at: usize,
        error: SagaError,
    },
}

/// The orchestrator.
///
/// Saga<S> is parameterised by a homogeneous `S: SagaStep` for type-safety.
/// For heterogeneous steps, use `Saga<Box<dyn SagaStep>>` and box each step.
pub struct Saga {
    pub name: String,
    pub steps: Vec<Box<dyn SagaStep>>,
    pub rollback_cap_ms: u64,
    pub chunks_emitted: AtomicU64,
}

impl Saga {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            steps: Vec::new(),
            rollback_cap_ms: 10 * 60 * 1000,   // 10 min (= SLO-P2-ROLLBACK)
            chunks_emitted: AtomicU64::new(0),
        }
    }

    pub fn add_step(mut self, step: Box<dyn SagaStep>) -> Self {
        self.steps.push(step);
        self
    }

    pub fn with_rollback_cap_ms(mut self, cap: u64) -> Self {
        self.rollback_cap_ms = cap;
        self
    }

    /// Run the saga end-to-end. Returns the outcome.
    pub async fn run(&self, ctx: &SagaContext) -> SagaOutcome {
        let started = Instant::now();
        for (idx, step) in self.steps.iter().enumerate() {
            tracing::debug!(
                target: "efe.saga",
                saga = %self.name,
                step_idx = idx,
                step_name = step.name(),
                "executing step"
            );
            match step.execute(ctx).await {
                Ok(()) => {
                    self.chunks_emitted.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    tracing::warn!(
                        target: "efe.saga",
                        saga = %self.name,
                        step_idx = idx,
                        error = ?e,
                        "step failed, rolling back"
                    );
                    return self.rollback(ctx, idx, e).await;
                }
            }
        }
        SagaOutcome::Committed {
            steps_run: self.steps.len(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        }
    }

    async fn rollback(
        &self,
        ctx: &SagaContext,
        failed_at: usize,
        original_error: SagaError,
    ) -> SagaOutcome {
        let rollback_started = Instant::now();
        let mut compensated_steps = 0;
        // Compensate from `failed_at - 1` down to `0`.
        if failed_at > 0 {
            for idx in (0..failed_at).rev() {
                let step = &self.steps[idx];
                tracing::debug!(
                    target: "efe.saga",
                    saga = %self.name,
                    step_idx = idx,
                    step_name = step.name(),
                    "compensating step"
                );
                // Timeout check (= D-LATACT-7).
                let elapsed_ms = rollback_started.elapsed().as_millis() as u64;
                if elapsed_ms > self.rollback_cap_ms {
                    return SagaOutcome::CompensationFailure {
                        failed_at,
                        compensation_failed_at: idx,
                        error: SagaError::Timeout {
                            elapsed_ms,
                            cap_ms: self.rollback_cap_ms,
                        },
                    };
                }
                if let Err(e) = step.compensate(ctx).await {
                    tracing::error!(
                        target: "efe.saga",
                        saga = %self.name,
                        step_idx = idx,
                        error = ?e,
                        "compensation failed"
                    );
                    return SagaOutcome::CompensationFailure {
                        failed_at,
                        compensation_failed_at: idx,
                        error: SagaError::CompensationFailed {
                            step_index: idx,
                            step_name: step.name(),
                            message: e.to_string(),
                        },
                    };
                }
                compensated_steps += 1;
            }
        }
        SagaOutcome::RolledBack {
            failed_at,
            compensated_steps,
            rollback_elapsed_ms: rollback_started.elapsed().as_millis() as u64,
            error: original_error,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Pre-built steps for P2 tenant deploy + GDPR delete sagas.
// ─────────────────────────────────────────────────────────────────────

pub mod steps;
pub mod tenant_lifecycle;

#[cfg(test)]
mod tests {
    use super::*;

    struct PassingStep(&'static str);

    #[async_trait]
    impl SagaStep for PassingStep {
        fn name(&self) -> &'static str { self.0 }
        async fn execute(&self, _: &SagaContext) -> Result<(), SagaError> { Ok(()) }
        async fn compensate(&self, _: &SagaContext) -> Result<(), SagaError> { Ok(()) }
    }

    struct FailingStep(&'static str);

    #[async_trait]
    impl SagaStep for FailingStep {
        fn name(&self) -> &'static str { self.0 }
        async fn execute(&self, _: &SagaContext) -> Result<(), SagaError> {
            Err(SagaError::Aborted { message: format!("step {} forced fail", self.0) })
        }
        async fn compensate(&self, _: &SagaContext) -> Result<(), SagaError> { Ok(()) }
    }

    #[tokio::test]
    async fn saga_runs_all_steps_committed() {
        let saga = Saga::new("test")
            .add_step(Box::new(PassingStep("s1")))
            .add_step(Box::new(PassingStep("s2")))
            .add_step(Box::new(PassingStep("s3")));
        let ctx = SagaContext::new("tenant-test".to_string(), "key-1".to_string());
        let outcome = saga.run(&ctx).await;
        match outcome {
            SagaOutcome::Committed { steps_run, .. } => assert_eq!(steps_run, 3),
            _ => panic!("expected committed"),
        }
    }

    #[tokio::test]
    async fn saga_rolls_back_on_step_failure() {
        let saga = Saga::new("test_rb")
            .add_step(Box::new(PassingStep("s1")))
            .add_step(Box::new(PassingStep("s2")))
            .add_step(Box::new(FailingStep("s3_fail")));
        let ctx = SagaContext::new("tenant-test".to_string(), "key-2".to_string());
        let outcome = saga.run(&ctx).await;
        match outcome {
            SagaOutcome::RolledBack { failed_at, compensated_steps, .. } => {
                assert_eq!(failed_at, 2);
                assert_eq!(compensated_steps, 2);   // s2 + s1
            }
            _ => panic!("expected rolled back"),
        }
    }

    #[tokio::test]
    async fn saga_compensate_called_in_reverse_order() {
        use std::sync::Mutex;
        let order: std::sync::Arc<Mutex<Vec<&str>>> = std::sync::Arc::new(Mutex::new(Vec::new()));

        struct TrackedStep {
            name: &'static str,
            order: std::sync::Arc<Mutex<Vec<&'static str>>>,
            fail: bool,
        }
        #[async_trait]
        impl SagaStep for TrackedStep {
            fn name(&self) -> &'static str { self.name }
            async fn execute(&self, _: &SagaContext) -> Result<(), SagaError> {
                if self.fail {
                    Err(SagaError::Aborted { message: "boom".into() })
                } else { Ok(()) }
            }
            async fn compensate(&self, _: &SagaContext) -> Result<(), SagaError> {
                self.order.lock().unwrap().push(self.name);
                Ok(())
            }
        }

        let saga = Saga::new("rev")
            .add_step(Box::new(TrackedStep { name: "A", order: order.clone(), fail: false }))
            .add_step(Box::new(TrackedStep { name: "B", order: order.clone(), fail: false }))
            .add_step(Box::new(TrackedStep { name: "C", order: order.clone(), fail: true }));
        let ctx = SagaContext::new("t".into(), "k".into());
        let _ = saga.run(&ctx).await;
        let seq: Vec<&str> = order.lock().unwrap().clone();
        assert_eq!(seq, vec!["B", "A"], "compensation order must be reverse");
    }
}
