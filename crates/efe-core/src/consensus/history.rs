//! `ConvergenceHistory` — fixed-capacity ring buffer of per-round stats.
//!
//! We keep a compact [`StatsRecord`] for each of the last `W` rounds.
//! Only the most-recent centroid is retained in a separate slot (see
//! [`ConvergenceHistory::last_centroid`]) — we don't need the full
//! per-round centroid for the consensus signals, and storing it would
//! balloon memory to `W × D × 4` bytes per detector.

use std::collections::VecDeque;

use crate::fusion::FusionStats;

/// Compact per-round record retained in the history window.
///
/// Mirrors the fields of [`FusionStats`] that M6 needs for the six
/// signals. Does *not* own the centroid — centroids are large (up to
/// `D × 4` bytes) and we only compare the two most-recent ones.
#[derive(Clone, Debug, PartialEq)]
pub struct StatsRecord {
    /// Round index (0-based).
    pub round: u32,
    /// Variance across active nano-agents.
    pub variance: f32,
    /// `sqrt(variance)` in MVP.
    pub gradient_norm: f32,
    /// Mean row-entropy of the attention weights.
    pub mean_attention_entropy: f32,
    /// Outlier ratio.
    pub outlier_ratio: f32,
    /// Number of active agents.
    pub n_active: u32,
    /// `has_nan || has_inf` — precomputed for fast anomaly checks.
    pub has_anomaly: bool,
    /// Confidence produced by the scorer *after* this record was pushed.
    /// `0.0` until `ConvergenceHistory::update_last_confidence` is called.
    pub confidence: f32,
}

impl StatsRecord {
    /// Derive from a `FusionStats`. Does not copy the centroid.
    #[must_use]
    pub fn from_stats(stats: &FusionStats) -> Self {
        Self {
            round: stats.round,
            variance: stats.variance,
            gradient_norm: stats.gradient_norm,
            mean_attention_entropy: stats.mean_attention_entropy,
            outlier_ratio: stats.outlier_ratio,
            n_active: stats.n_active,
            has_anomaly: stats.has_numerical_issues(),
            confidence: 0.0,
        }
    }
}

/// Ring buffer over the last `capacity` rounds.
///
/// Invariant: records are stored **oldest-first**. Pushing when full
/// evicts the oldest.
#[derive(Debug, Clone)]
pub struct ConvergenceHistory {
    buf: VecDeque<StatsRecord>,
    capacity: usize,
    /// Cached copy of the most-recent centroid (for S3 computation).
    /// `None` until the first push.
    last_centroid: Option<Vec<f32>>,
    /// Cached copy of the *previous* centroid (before the last push).
    prev_centroid: Option<Vec<f32>>,
}

impl ConvergenceHistory {
    /// Create an empty history with the given capacity.
    ///
    /// # Panics
    ///
    /// Panics if `capacity == 0` — a history of zero rounds is nonsense.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "ConvergenceHistory: capacity must be > 0");
        Self {
            buf: VecDeque::with_capacity(capacity),
            capacity,
            last_centroid: None,
            prev_centroid: None,
        }
    }

    /// Push a new record (and centroid), evicting the oldest if full.
    ///
    /// The previous `last_centroid` becomes `prev_centroid` — stored so
    /// the signal S3 (drift) can compare two rounds without needing to
    /// walk the whole buffer.
    pub fn push(&mut self, record: StatsRecord, centroid: &[f32]) {
        if self.buf.len() == self.capacity {
            self.buf.pop_front();
        }
        self.buf.push_back(record);
        self.prev_centroid = self.last_centroid.take();
        self.last_centroid = Some(centroid.to_vec());
    }

    /// Overwrite the confidence field of the newest record.
    pub fn update_last_confidence(&mut self, confidence: f32) {
        if let Some(last) = self.buf.back_mut() {
            last.confidence = confidence;
        }
    }

    /// Clear all state. Used by `ConsensusDetector::reset`.
    pub fn clear(&mut self) {
        self.buf.clear();
        self.last_centroid = None;
        self.prev_centroid = None;
    }

    /// Number of records currently stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// True iff empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// True iff at capacity.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.buf.len() == self.capacity
    }

    /// Capacity of the window.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Iterate over records, oldest-first.
    ///
    /// Returns a [`DoubleEndedIterator`] so callers can use `.rev()`
    /// to walk newest-first without an intermediate allocation.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &StatsRecord> + ExactSizeIterator {
        self.buf.iter()
    }

    /// Latest record (newest), if any.
    #[must_use]
    pub fn latest(&self) -> Option<&StatsRecord> {
        self.buf.back()
    }

    /// Oldest record, if any.
    #[must_use]
    pub fn oldest(&self) -> Option<&StatsRecord> {
        self.buf.front()
    }

    /// The most-recent centroid, if any round has been pushed.
    #[must_use]
    pub fn last_centroid(&self) -> Option<&[f32]> {
        self.last_centroid.as_deref()
    }

    /// The centroid from the round *before* the latest, if any.
    #[must_use]
    pub fn prev_centroid(&self) -> Option<&[f32]> {
        self.prev_centroid.as_deref()
    }

    /// Peak confidence ever observed in the window.
    ///
    /// Returns `None` if the window is empty.
    #[must_use]
    pub fn peak_confidence(&self) -> Option<f32> {
        self.buf.iter().map(|r| r.confidence).fold(None, |acc, c| {
            Some(match acc {
                Some(a) if a >= c => a,
                _ => c,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fusion::FusionStats;

    fn mk_stats(round: u32, variance: f32) -> FusionStats {
        let mut s = FusionStats::synthetic_with(round, 4, variance, variance.sqrt());
        s.n_active = 4;
        s
    }

    #[test]
    fn new_is_empty() {
        let h = ConvergenceHistory::new(4);
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
        assert_eq!(h.capacity(), 4);
        assert!(h.latest().is_none());
        assert!(h.oldest().is_none());
        assert!(h.last_centroid().is_none());
        assert!(h.prev_centroid().is_none());
    }

    #[test]
    #[should_panic(expected = "capacity must be > 0")]
    fn zero_capacity_panics() {
        let _ = ConvergenceHistory::new(0);
    }

    #[test]
    fn push_grows_until_capacity_then_evicts() {
        let mut h = ConvergenceHistory::new(3);
        let c = vec![0.1f32; 4];
        for r in 0..5 {
            let s = mk_stats(r, 1.0);
            h.push(StatsRecord::from_stats(&s), &c);
        }
        assert_eq!(h.len(), 3);
        assert!(h.is_full());
        let rounds: Vec<u32> = h.iter().map(|r| r.round).collect();
        assert_eq!(rounds, vec![2, 3, 4]); // oldest three evicted
    }

    #[test]
    fn centroids_track_latest_and_previous() {
        let mut h = ConvergenceHistory::new(5);
        let c0 = vec![1.0f32; 4];
        let c1 = vec![2.0f32; 4];
        let c2 = vec![3.0f32; 4];
        h.push(StatsRecord::from_stats(&mk_stats(0, 0.5)), &c0);
        assert_eq!(h.last_centroid(), Some(c0.as_slice()));
        assert!(h.prev_centroid().is_none());
        h.push(StatsRecord::from_stats(&mk_stats(1, 0.3)), &c1);
        assert_eq!(h.last_centroid(), Some(c1.as_slice()));
        assert_eq!(h.prev_centroid(), Some(c0.as_slice()));
        h.push(StatsRecord::from_stats(&mk_stats(2, 0.1)), &c2);
        assert_eq!(h.last_centroid(), Some(c2.as_slice()));
        assert_eq!(h.prev_centroid(), Some(c1.as_slice()));
    }

    #[test]
    fn update_last_confidence_sets_field() {
        let mut h = ConvergenceHistory::new(3);
        let c = vec![0.0f32; 4];
        h.push(StatsRecord::from_stats(&mk_stats(0, 1.0)), &c);
        h.update_last_confidence(0.73);
        assert!((h.latest().unwrap().confidence - 0.73).abs() < 1e-6);
    }

    #[test]
    fn peak_confidence_returns_max() {
        let mut h = ConvergenceHistory::new(5);
        let c = vec![0.0f32; 4];
        for (r, conf) in [(0, 0.3f32), (1, 0.9), (2, 0.5), (3, 0.8)] {
            h.push(StatsRecord::from_stats(&mk_stats(r, 1.0)), &c);
            h.update_last_confidence(conf);
        }
        assert!((h.peak_confidence().unwrap() - 0.9).abs() < 1e-6);
    }

    #[test]
    fn peak_confidence_none_on_empty() {
        let h = ConvergenceHistory::new(3);
        assert!(h.peak_confidence().is_none());
    }

    #[test]
    fn clear_resets_all() {
        let mut h = ConvergenceHistory::new(3);
        let c = vec![0.0f32; 4];
        h.push(StatsRecord::from_stats(&mk_stats(0, 1.0)), &c);
        h.push(StatsRecord::from_stats(&mk_stats(1, 0.5)), &c);
        assert!(!h.is_empty());
        h.clear();
        assert!(h.is_empty());
        assert!(h.last_centroid().is_none());
        assert!(h.prev_centroid().is_none());
    }

    #[test]
    fn from_stats_flags_anomaly() {
        let mut s = mk_stats(0, 1.0);
        s.has_nan = true;
        let r = StatsRecord::from_stats(&s);
        assert!(r.has_anomaly);
    }

    #[test]
    fn latest_oldest_track_correct_ends() {
        let mut h = ConvergenceHistory::new(5);
        let c = vec![0.0f32; 4];
        for r in 0..4 {
            h.push(StatsRecord::from_stats(&mk_stats(r, 1.0)), &c);
        }
        assert_eq!(h.oldest().unwrap().round, 0);
        assert_eq!(h.latest().unwrap().round, 3);
    }
}
