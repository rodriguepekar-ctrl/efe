//! Hand-rolled mock strategies used across integration tests.
//!
//! We don't use mockall for the MVP because the overhead doesn't justify
//! the extra learning curve and the async_trait friction. These mocks
//! are simple structs with interior mutability (`Mutex`) that record
//! calls and return configurable outputs.
//!
//! The module is included by several integration test files; not every
//! helper is used by every consumer, hence the blanket `dead_code` allow.

#![allow(dead_code)]

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use efe_core::els::EpistemicLatentSpace;
use efe_core::errors::{EfeError, Result};
use efe_core::fusion::FusionStats;
use efe_core::memory::NanoAgentBatch;
use efe_core::swarm::{
    AlignmentProjector, ConsensusDetector, DecodeMode, DecodedOutput, Dissolver, FusionStrategy,
    GenesisStrategy, Request, TelemetryCollector, TelemetryGuard, Verdict,
};
use efe_core::types::{AlignmentCone, LatentDim, SwarmId, SwarmSpec};

/// Recording mock for [`GenesisStrategy`].
pub struct MockGenesis {
    pub dim: LatentDim,
    pub count: u32,
    pub spawn_calls: Mutex<u32>,
    pub respawn_calls: Mutex<Vec<u32>>, // counts of each respawn request
}

impl MockGenesis {
    pub fn new(dim: LatentDim, count: u32) -> Self {
        Self {
            dim,
            count,
            spawn_calls: Mutex::new(0),
            respawn_calls: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl GenesisStrategy for MockGenesis {
    async fn spawn(&self, spec: &SwarmSpec) -> Result<(NanoAgentBatch, EpistemicLatentSpace)> {
        *self.spawn_calls.lock().expect("lock") += 1;
        let batch = NanoAgentBatch::new_stub(spec.id, self.count);
        let els = EpistemicLatentSpace::new_stub(spec.id, self.dim, self.count);
        Ok((batch, els))
    }

    async fn respawn_diversify(&self, _batch: &mut NanoAgentBatch, count: u32) -> Result<()> {
        self.respawn_calls.lock().expect("lock").push(count);
        Ok(())
    }
}

/// Configurable fusion mock.
pub struct MockFusion {
    /// Vector of stats to return in order (one per round). After the
    /// list is exhausted, reuses the last entry.
    pub stats: Vec<FusionStats>,
    pub round_calls: Mutex<u32>,
    pub noise_calls: Mutex<Vec<f32>>,
}

impl MockFusion {
    pub fn new(stats: Vec<FusionStats>) -> Self {
        Self {
            stats,
            round_calls: Mutex::new(0),
            noise_calls: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl FusionStrategy for MockFusion {
    async fn run_round(
        &self,
        _els: &mut EpistemicLatentSpace,
        _batch: &NanoAgentBatch,
        round: u32,
    ) -> Result<FusionStats> {
        *self.round_calls.lock().expect("lock") += 1;
        let idx = ((round as usize).saturating_sub(1)).min(self.stats.len().saturating_sub(1));
        Ok(self.stats[idx].clone())
    }

    async fn inject_noise(&self, _els: &mut EpistemicLatentSpace, sigma: f32) -> Result<()> {
        self.noise_calls.lock().expect("lock").push(sigma);
        Ok(())
    }
}

/// Consensus detector driven by a scripted verdict sequence.
pub struct ScriptedDetector {
    pub verdicts: Vec<Verdict>,
    pub idx: Mutex<usize>,
    pub reset_calls: Mutex<u32>,
}

impl ScriptedDetector {
    pub fn new(verdicts: Vec<Verdict>) -> Self {
        Self {
            verdicts,
            idx: Mutex::new(0),
            reset_calls: Mutex::new(0),
        }
    }
}

impl ConsensusDetector for ScriptedDetector {
    fn evaluate(&mut self, _round: u32, _stats: &FusionStats) -> Verdict {
        let mut idx = self.idx.lock().expect("lock");
        let v = self
            .verdicts
            .get(*idx)
            .copied()
            .unwrap_or(Verdict::Continue);
        *idx += 1;
        v
    }

    fn reset(&mut self) {
        *self.idx.lock().expect("lock") = 0;
        *self.reset_calls.lock().expect("lock") += 1;
    }
}

/// Dissolver that returns a fixed centroid and decodes into a fixed string.
pub struct MockDissolver {
    pub centroid: Vec<f32>,
    pub output: String,
}

impl MockDissolver {
    pub fn new(dim: usize, output: impl Into<String>) -> Self {
        Self {
            centroid: vec![0.1; dim],
            output: output.into(),
        }
    }
}

#[async_trait]
impl Dissolver for MockDissolver {
    async fn extract_centroid(&self, _els: &EpistemicLatentSpace) -> Result<Vec<f32>> {
        Ok(self.centroid.clone())
    }

    async fn decode(&self, centroid: &[f32], mode: &DecodeMode) -> Result<DecodedOutput> {
        match mode {
            DecodeMode::Text { .. } => Ok(DecodedOutput::Text(self.output.clone())),
            DecodeMode::PassThrough => Ok(DecodedOutput::Raw(centroid.to_vec())),
            // Non-exhaustive enum: fall back to pass-through on unknown modes.
            _ => Ok(DecodedOutput::Raw(centroid.to_vec())),
        }
    }
}

/// Alignment projector that multiplies the centroid by `scale` and reports
/// a configurable `delta`.
pub struct MockAligner {
    pub delta_to_return: f32,
    pub scale: f32,
    pub calls: Mutex<u32>,
}

impl MockAligner {
    pub fn new(delta: f32, scale: f32) -> Self {
        Self {
            delta_to_return: delta,
            scale,
            calls: Mutex::new(0),
        }
    }

    pub fn safe() -> Self {
        Self::new(0.01, 1.0)
    }

    pub fn breaching() -> Self {
        Self::new(0.99, 1.0)
    }
}

impl AlignmentProjector for MockAligner {
    fn project(&self, centroid: &[f32], _cone: &AlignmentCone) -> Result<(Vec<f32>, f32)> {
        *self.calls.lock().expect("lock") += 1;
        let scaled = centroid.iter().map(|x| x * self.scale).collect();
        Ok((scaled, self.delta_to_return))
    }
}

/// Dissolver + aligner combo that can produce failure modes on demand.
pub struct FailingDissolver;

#[async_trait]
impl Dissolver for FailingDissolver {
    async fn extract_centroid(&self, _els: &EpistemicLatentSpace) -> Result<Vec<f32>> {
        Err(EfeError::KernelFailure("frechet failed".into()))
    }

    async fn decode(&self, _centroid: &[f32], _mode: &DecodeMode) -> Result<DecodedOutput> {
        unreachable!("should never reach decode")
    }
}

/// Telemetry collector that records every signal.
#[derive(Default, Clone)]
pub struct RecordingTelemetry {
    pub events: Arc<Mutex<Vec<String>>>,
}

impl TelemetryCollector for RecordingTelemetry {
    fn begin_swarm(&self, id: SwarmId, _req: &Request) -> Box<dyn TelemetryGuard> {
        self.events
            .lock()
            .expect("lock")
            .push(format!("begin:{id}"));
        Box::new(RecordingGuard {
            events: self.events.clone(),
        })
    }
}

pub struct RecordingGuard {
    events: Arc<Mutex<Vec<String>>>,
}

impl TelemetryGuard for RecordingGuard {
    fn record_phase_complete(&mut self, phase: &str, elapsed: Duration) {
        self.events
            .lock()
            .expect("lock")
            .push(format!("phase_complete:{phase}:{}ms", elapsed.as_millis()));
    }

    fn record_phase_error(&mut self, phase: &str, err: &EfeError) {
        self.events
            .lock()
            .expect("lock")
            .push(format!("phase_error:{phase}:{}", err.code()));
    }

    fn record_round(&mut self, round: u32, _stats: &FusionStats) {
        self.events
            .lock()
            .expect("lock")
            .push(format!("round:{round}"));
    }

    fn record_event(&mut self, round: u32, event: &str) {
        self.events
            .lock()
            .expect("lock")
            .push(format!("event:{round}:{event}"));
    }

    fn finalize(&mut self, total_ms: f32, _stats: &FusionStats) {
        self.events
            .lock()
            .expect("lock")
            .push(format!("finalize:{}ms", total_ms as u32));
    }
}
