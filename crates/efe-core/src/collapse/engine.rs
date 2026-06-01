//! `CollapseEngine` — orchestration layer for M7.
//!
//! Responsibilities:
//!
//! 1. Translate a `ConsensusVerdict` into a `CollapseTrigger`.
//! 2. Enforce a per-swarm circuit breaker (caps the number of collapses
//!    to prevent infinite loops).
//! 3. Seed a deterministic or wall-clock PRNG.
//! 4. Walk the escalation ladder in `CollapsePolicy`, trying each
//!    applicable strategy until one succeeds.
//! 5. Record telemetry via `CollapseEvent` ring buffer.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rand::SeedableRng;
use rand_pcg::Pcg64;

use crate::collapse::config::CollapseConfig;
use crate::collapse::context::CollapseContext;
use crate::collapse::event::CollapseEvent;
use crate::collapse::helpers::fnv1a64;
use crate::collapse::outcome::{CollapseAction, CollapseOutcome, CollapseWarning};
use crate::collapse::policy::CollapsePolicy;
use crate::collapse::trigger::CollapseTrigger;
use crate::consensus::verdict::ConsensusVerdict;
use crate::els::EpistemicLatentSpace;
use crate::errors::Result;
use crate::fusion::strategy::ReplayableFusion;
use crate::memory::Arena;
use crate::types::SwarmId;

/// Orchestrates collapse recovery for swarms.
///
/// One engine instance serves many swarms; a per-swarm counter is kept
/// to enforce the circuit breaker.
pub struct CollapseEngine {
    policy: CollapsePolicy,
    config: CollapseConfig,
    events: VecDeque<CollapseEvent>,
    events_capacity: usize,
    collapse_counts: HashMap<SwarmId, u32>,
}

impl std::fmt::Debug for CollapseEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `config` and `events_capacity` intentionally elided from the
        // Debug impl — they're redundant with `policy_strategies` and
        // `events_len` at a glance. Use the typed accessors for deep
        // introspection.
        f.debug_struct("CollapseEngine")
            .field("policy_strategies", &self.policy.len())
            .field("events_len", &self.events.len())
            .field("swarms_tracked", &self.collapse_counts.len())
            .finish_non_exhaustive()
    }
}

impl CollapseEngine {
    /// Construct an engine. Validates the config eagerly.
    ///
    /// # Errors
    ///
    /// Propagates [`CollapseConfig::validate`] failures.
    pub fn new(policy: CollapsePolicy, config: CollapseConfig) -> Result<Self> {
        config.validate()?;
        let cap = config.events_capacity as usize;
        Ok(Self {
            policy,
            config,
            events: VecDeque::with_capacity(cap),
            events_capacity: cap,
            collapse_counts: HashMap::new(),
        })
    }

    /// Current collapse count for a swarm (0 if never collapsed).
    #[must_use]
    pub fn collapse_count(&self, swarm_id: SwarmId) -> u32 {
        self.collapse_counts.get(&swarm_id).copied().unwrap_or(0)
    }

    /// Reset the collapse counter for a given swarm — typically called
    /// by M1 after a successful respawn via M3.
    pub fn reset_swarm_counter(&mut self, swarm_id: SwarmId) {
        self.collapse_counts.remove(&swarm_id);
    }

    /// Iterate over recorded events, oldest-first.
    pub fn events(&self) -> impl DoubleEndedIterator<Item = &CollapseEvent> + ExactSizeIterator {
        self.events.iter()
    }

    /// Current number of recorded events. O(1).
    #[must_use]
    pub fn events_len(&self) -> usize {
        self.events.len()
    }

    /// Maximum number of events retained in the ring buffer.
    #[must_use]
    pub fn events_capacity(&self) -> usize {
        self.events_capacity
    }

    /// Policy in use.
    #[must_use]
    pub fn policy(&self) -> &CollapsePolicy {
        &self.policy
    }

    /// Config in use.
    #[must_use]
    pub fn config(&self) -> &CollapseConfig {
        &self.config
    }

    /// Top-level collapse entry point.
    ///
    /// # Errors
    ///
    /// Propagates strategy-level errors (contract violations). Normal
    /// failures (replay empty, etc.) are encoded in the `Ok(outcome)`
    /// with `success = false`.
    pub async fn handle(
        &mut self,
        verdict: &ConsensusVerdict,
        els: &mut EpistemicLatentSpace,
        arena: &Arena,
        fusion: Arc<dyn ReplayableFusion>,
        round: u32,
    ) -> Result<CollapseOutcome> {
        let start = Instant::now();
        let swarm_id = els.swarm_id();

        // 1. Derive trigger. Non-collapse verdicts return NoOp.
        let Some(trigger) = CollapseTrigger::from_verdict(verdict, round) else {
            return Ok(Self::no_op_outcome(verdict, round, start));
        };

        // 2. Circuit breaker. Increment FIRST so the count includes the
        //    current attempt, then check. The current attempt still
        //    runs only if we have not *exceeded* the limit.
        let count = {
            let entry = self.collapse_counts.entry(swarm_id).or_insert(0);
            *entry += 1;
            *entry
        };
        if count > self.config.max_collapses_per_swarm {
            let outcome = Self::abort_outcome(
                &trigger,
                start,
                Some(CollapseWarning::CircuitBreaker {
                    swarm_collapse_count: count,
                }),
            );
            self.record_event(swarm_id, &trigger, &outcome, count);
            return Ok(outcome);
        }

        // 3. Seed PRNG.
        let seed = self.build_seed(swarm_id, round);
        let rng = Pcg64::seed_from_u64(seed);
        let deadline = start + Duration::from_millis(u64::from(self.config.deadline_ms));

        // 4. Escalation loop.
        //
        // We clone the strategy Arcs into a local Vec so that the
        // immutable borrow of `self.policy` ends before we need a
        // mutable borrow of `self` for `record_event`. Arc clone is
        // ~10 ns per strategy; negligible vs. a collapse.
        let strategies: Vec<Arc<dyn crate::collapse::strategy::CollapseStrategy>> =
            self.policy.escalation_order.clone();
        let mut final_outcome: Option<CollapseOutcome> = None;
        for strategy in &strategies {
            if Instant::now() >= deadline {
                let outcome = Self::deadline_outcome(&trigger, start, &self.config);
                self.record_event(swarm_id, &trigger, &outcome, count);
                return Ok(outcome);
            }
            if !strategy.applicable_to(&trigger) {
                continue;
            }
            let mut ctx = CollapseContext {
                els,
                arena,
                fusion: fusion.clone(),
                round,
                trigger: trigger.clone(),
                verdict: Some(verdict.clone()),
                rng: rng.clone(),
                deadline,
                config: &self.config,
            };
            let outcome = strategy.apply(&mut ctx).await?;
            let recovered = outcome.is_recoverable();
            let terminal = outcome.is_terminal();
            self.record_event(swarm_id, &trigger, &outcome, count);
            if recovered {
                return Ok(outcome);
            }
            if terminal {
                // e.g. FullAbort reached mid-policy — stop escalating.
                return Ok(outcome);
            }
            final_outcome = Some(outcome);
        }

        // 5. Escalation exhausted without recovery. Fall through to a
        //    synthetic FullAbort outcome. Prefer the last real failure's
        //    warnings for diagnostics if present.
        let _ = final_outcome; // intentionally not propagated; kept for future M11 use
        let outcome = Self::abort_outcome(&trigger, start, Some(CollapseWarning::Abandoned));
        self.record_event(swarm_id, &trigger, &outcome, count);
        Ok(outcome)
    }

    // ---- private helpers --------------------------------------------------

    fn build_seed(&self, swarm_id: SwarmId, round: u32) -> u64 {
        Self::build_seed_impl(&self.config, swarm_id, round)
    }

    fn build_seed_impl(config: &CollapseConfig, swarm_id: SwarmId, round: u32) -> u64 {
        if config.deterministic_seed {
            let key = format!("{swarm_id:?}-{round}");
            fnv1a64(key.as_bytes())
        } else {
            #[allow(clippy::cast_possible_truncation)]
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as u64);
            ts.wrapping_add(u64::from(round).wrapping_mul(0xA5A5_A5A5_A5A5_A5A5))
        }
    }

    fn record_event(
        &mut self,
        swarm_id: SwarmId,
        trigger: &CollapseTrigger,
        outcome: &CollapseOutcome,
        collapse_index: u32,
    ) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let event = CollapseEvent {
            timestamp_nanos: ts,
            swarm_id,
            trigger: trigger.clone(),
            outcome: outcome.clone(),
            collapse_index,
        };
        if self.events.len() == self.events_capacity {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    fn no_op_outcome(verdict: &ConsensusVerdict, round: u32, start: Instant) -> CollapseOutcome {
        // For non-collapse verdicts (Continue, Converged) we synthesize
        // a trigger purely for the outcome shape. Use Manual as a
        // neutral container.
        let trigger = CollapseTrigger::Manual {
            reason: format!("non-collapse verdict: {}", verdict.label()),
            round,
        };
        CollapseOutcome {
            action_taken: CollapseAction::NoOp,
            success: true, // nothing to do → "succeeded"
            trigger,
            new_round: Some(round),
            variance_after: None,
            recovery_confidence: 1.0,
            warnings: vec![],
            duration_us: u32::try_from(start.elapsed().as_micros()).unwrap_or(u32::MAX),
            strategy_name: "engine".into(),
        }
    }

    fn abort_outcome(
        trigger: &CollapseTrigger,
        start: Instant,
        warning: Option<CollapseWarning>,
    ) -> CollapseOutcome {
        CollapseOutcome {
            action_taken: CollapseAction::FullAbort,
            success: false,
            trigger: trigger.clone(),
            new_round: None,
            variance_after: None,
            recovery_confidence: 0.0,
            warnings: warning.into_iter().collect(),
            duration_us: u32::try_from(start.elapsed().as_micros()).unwrap_or(u32::MAX),
            strategy_name: "engine".into(),
        }
    }

    fn deadline_outcome(
        trigger: &CollapseTrigger,
        start: Instant,
        config: &CollapseConfig,
    ) -> CollapseOutcome {
        let actual_us = u32::try_from(start.elapsed().as_micros()).unwrap_or(u32::MAX);
        CollapseOutcome {
            action_taken: CollapseAction::NoOp,
            success: false,
            trigger: trigger.clone(),
            new_round: None,
            variance_after: None,
            recovery_confidence: 0.0,
            warnings: vec![CollapseWarning::DeadlineExceeded {
                budget_us: config.deadline_ms.saturating_mul(1000),
                actual_us,
            }],
            duration_us: actual_us,
            strategy_name: "engine".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collapse::strategies::{StubBehavior, StubStrategy};
    use crate::collapse::strategy::CollapseStrategy;
    use crate::consensus::verdict::{AnomalyCode, ConsensusVerdict};
    use std::sync::Arc;

    fn mk_engine_with_stub(behavior: StubBehavior) -> (CollapseEngine, Arc<StubStrategy>) {
        let stub = Arc::new(StubStrategy::new("only", behavior));
        let stub_dyn: Arc<dyn CollapseStrategy> = stub.clone();
        let policy = CollapsePolicy::custom(vec![stub_dyn]);
        let engine = CollapseEngine::new(policy, CollapseConfig::default()).unwrap();
        (engine, stub)
    }

    #[test]
    fn new_validates_config() {
        let cfg = CollapseConfig::default().with_max_collapses(0);
        let policy = CollapsePolicy::default_mvp();
        assert!(CollapseEngine::new(policy, cfg).is_err());
    }

    #[test]
    fn collapse_count_starts_at_zero() {
        let (engine, _) = mk_engine_with_stub(StubBehavior::AlwaysSucceed);
        assert_eq!(engine.collapse_count(SwarmId::new()), 0);
    }

    #[test]
    fn reset_swarm_counter_removes_entry() {
        let (mut engine, _) = mk_engine_with_stub(StubBehavior::AlwaysSucceed);
        let sid = SwarmId::new();
        engine.collapse_counts.insert(sid, 3);
        engine.reset_swarm_counter(sid);
        assert_eq!(engine.collapse_count(sid), 0);
    }

    #[test]
    fn no_op_outcome_for_continue_verdict() {
        let outcome = CollapseEngine::no_op_outcome(&ConsensusVerdict::Continue, 0, Instant::now());
        assert_eq!(outcome.action_taken, CollapseAction::NoOp);
        assert!(outcome.success);
    }

    #[test]
    fn abort_outcome_attaches_warning_when_supplied() {
        let trigger = CollapseTrigger::FromAnomaly {
            code: AnomalyCode::NanInf,
            round: 0,
        };
        let outcome = CollapseEngine::abort_outcome(
            &trigger,
            Instant::now(),
            Some(CollapseWarning::Abandoned),
        );
        assert_eq!(outcome.action_taken, CollapseAction::FullAbort);
        assert!(matches!(
            outcome.warnings.first().unwrap(),
            CollapseWarning::Abandoned
        ));
    }

    #[test]
    fn deadline_outcome_reports_budget() {
        let cfg = CollapseConfig::default();
        let trigger = CollapseTrigger::Manual {
            reason: "x".into(),
            round: 0,
        };
        let outcome = CollapseEngine::deadline_outcome(&trigger, Instant::now(), &cfg);
        assert!(matches!(
            outcome.warnings.first(),
            Some(CollapseWarning::DeadlineExceeded { .. })
        ));
    }

    #[test]
    fn build_seed_deterministic_when_flag_set() {
        let cfg = CollapseConfig::default().with_deterministic_seed(true);
        let policy = CollapsePolicy::default_mvp();
        let engine = CollapseEngine::new(policy, cfg).unwrap();
        let sid = SwarmId::new();
        let a = engine.build_seed(sid, 3);
        let b = engine.build_seed(sid, 3);
        assert_eq!(a, b);
        let c = engine.build_seed(sid, 4);
        assert_ne!(a, c);
    }

    #[test]
    fn events_len_matches_iter_count() {
        let (mut engine, _) = mk_engine_with_stub(StubBehavior::AlwaysSucceed);
        assert_eq!(engine.events_len(), 0);
        let trigger = CollapseTrigger::FromAnomaly {
            code: AnomalyCode::NanInf,
            round: 0,
        };
        let outcome = CollapseEngine::abort_outcome(&trigger, Instant::now(), None);
        engine.record_event(SwarmId::new(), &trigger, &outcome, 1);
        engine.record_event(SwarmId::new(), &trigger, &outcome, 2);
        assert_eq!(engine.events_len(), 2);
        assert_eq!(engine.events_len(), engine.events().count());
    }

    #[test]
    fn events_capacity_eviction_keeps_newest() {
        let cfg = CollapseConfig::default();
        cfg.validate().unwrap();
        let cap = cfg.events_capacity as usize;
        let stub = Arc::new(StubStrategy::new("only", StubBehavior::AlwaysSucceed));
        let stub_dyn: Arc<dyn CollapseStrategy> = stub;
        let policy = CollapsePolicy::custom(vec![stub_dyn]);
        let mut engine = CollapseEngine::new(policy, cfg).unwrap();
        let trigger = CollapseTrigger::FromAnomaly {
            code: AnomalyCode::NanInf,
            round: 0,
        };
        let outcome = CollapseEngine::abort_outcome(&trigger, Instant::now(), None);
        // Push capacity+3 events — oldest three should be evicted.
        let swarm_id = SwarmId::new();
        for i in 0..(cap + 3) {
            engine.record_event(swarm_id, &trigger, &outcome, u32::try_from(i).unwrap());
        }
        assert_eq!(engine.events_len(), cap);
        assert_eq!(engine.events().next().unwrap().collapse_index, 3);
    }

    #[test]
    fn events_capacity_accessor_matches_config() {
        let cfg = CollapseConfig::default();
        let expected = cfg.events_capacity as usize;
        let (engine, _) = mk_engine_with_stub(StubBehavior::AlwaysSucceed);
        assert_eq!(engine.events_capacity(), expected);
    }
}
