//! PR #21 § 17 — chaos-mesh CRD applier + scheduling.

pub mod applier;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tracing::info;

use crate::error::BenchResult;
use crate::result::ChaosEvent;
use crate::scenario::ChaosFaultRef;

pub use applier::ChaosApplier;

/// Plans + executes the chaos timeline for a scenario.
///
/// `at_offset` strings are parsed via humantime ; each fault is
/// applied via [`ChaosApplier::apply`] at the scheduled time.  When
/// the duration is `permanent`, the fault is NOT auto-deleted (the
/// scenario typically follows up with a `delete-fault` action).
pub struct ChaosTimeline<'a> {
    pub faults: &'a [ChaosFaultRef],
    pub crd_root: PathBuf,
    pub namespace: String,
    events: Mutex<Vec<ChaosEvent>>,
}

impl<'a> ChaosTimeline<'a> {
    #[must_use]
    pub fn new(faults: &'a [ChaosFaultRef], crd_root: PathBuf, namespace: String) -> Self {
        Self {
            faults,
            crd_root,
            namespace,
            events: Mutex::new(Vec::new()),
        }
    }

    /// Returns the parsed faults sorted by offset.
    pub fn schedule(&self) -> Vec<(Duration, &ChaosFaultRef)> {
        let mut out = Vec::with_capacity(self.faults.len());
        for f in self.faults {
            if let Ok(d) = humantime::parse_duration(&f.at_offset) {
                out.push((d, f));
            }
        }
        out.sort_by_key(|(d, _)| *d);
        out
    }

    /// Drive the timeline against a chaos applier.  Returns the
    /// recorded events.
    pub async fn run(
        &self,
        applier: &dyn ChaosApplier,
        start: Instant,
    ) -> BenchResult<Vec<ChaosEvent>> {
        let schedule = self.schedule();
        for (offset, fault) in schedule {
            // Sleep until the offset.
            let elapsed = start.elapsed();
            if offset > elapsed {
                tokio::time::sleep(offset - elapsed).await;
            }
            match fault.action.as_ref() {
                Some(crate::scenario::ChaosFaultAction::DeleteFault) => {
                    info!(fault_id = %fault.id, "delete chaos fault");
                    applier.delete_fault(&fault.id, &self.namespace).await?;
                    self.record_event(start, fault, "delete");
                }
                None => {
                    if let Some(crd_file) = &fault.crd_file {
                        let path = self.resolve_crd(crd_file);
                        info!(fault_id = %fault.id, crd = %path.display(), "apply chaos fault");
                        applier
                            .apply_fault(&fault.id, &path, &self.namespace)
                            .await?;
                        self.record_event(start, fault, "apply");
                    }
                }
            }
        }
        Ok(self.events.lock().clone())
    }

    /// Cleanup all applied faults (idempotent).
    pub async fn cleanup(&self, applier: &dyn ChaosApplier) -> BenchResult<()> {
        for fault in self.faults {
            if fault.action.is_none() {
                let _ = applier.delete_fault(&fault.id, &self.namespace).await;
            }
        }
        Ok(())
    }

    fn resolve_crd(&self, file: &str) -> PathBuf {
        // crd_file is `pr21_meta_swarm/participant-loss.yaml`.
        self.crd_root.join(file)
    }

    fn record_event(&self, start: Instant, fault: &ChaosFaultRef, event_type: &str) {
        let mut e = self.events.lock();
        e.push(ChaosEvent {
            at_offset_s: start.elapsed().as_secs_f64(),
            fault_id: fault.id.clone(),
            event_type: event_type.into(),
            crd_kind: None,
        });
    }
}

/// Default CRD root : `deploy/chaos/` (relative to repo root).
#[must_use]
pub fn default_crd_root() -> PathBuf {
    PathBuf::from("deploy/chaos")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_sorts_by_offset() {
        let faults = vec![
            ChaosFaultRef {
                id: "second".into(),
                at_offset: "30s".into(),
                crd_file: Some("pr21_meta_swarm/x.yaml".into()),
                action: None,
                duration: None,
            },
            ChaosFaultRef {
                id: "first".into(),
                at_offset: "10s".into(),
                crd_file: Some("pr21_meta_swarm/y.yaml".into()),
                action: None,
                duration: None,
            },
        ];
        let timeline = ChaosTimeline::new(&faults, default_crd_root(), "test".into());
        let schedule = timeline.schedule();
        assert_eq!(schedule.len(), 2);
        assert_eq!(schedule[0].1.id, "first");
        assert_eq!(schedule[1].1.id, "second");
    }
}
