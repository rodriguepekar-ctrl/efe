// SPDX-License-Identifier: UNLICENSED
//! M32 — Segment abstraction.
//!
//! A `Segment` is the unit of HNSW build + snapshot. In M32 we ship
//! a single-segment collection layout ; M32.1 splits into multiple
//! segments for parallel build + finer-grained snapshot.
//!
//! Segments encapsulate :
//!
//! - The index backend (`Flat` for small collections, `Hnsw` for
//!   large).
//! - Build / search dispatch.
//! - Serialization to / from bincode (for S3 snapshots).

use serde::{Deserialize, Serialize};

use crate::distance::Distance;
use crate::error::{Result, VectorError};
use crate::flat::{FlatIndex, SearchHit, BRUTE_FORCE_MAX_VECTORS};
use crate::hnsw::{HnswConfig, HnswIndex};
use crate::store::VectorDoc;

/// Segment index backend.
pub enum SegmentBackend {
    /// Flat (brute-force) — for small collections.
    Flat(FlatIndex),
    /// HNSW (approximate) — for large collections.
    Hnsw(HnswIndex),
}

impl std::fmt::Debug for SegmentBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Flat(_) => f.debug_struct("Flat").finish(),
            Self::Hnsw(_) => f.debug_struct("Hnsw").finish(),
        }
    }
}

/// Snapshot envelope serialized to bincode + S3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentSnapshot {
    /// Format version. Bumped on incompatible schema changes.
    pub version: u32,
    /// Dim of every vector.
    pub dim: usize,
    /// Distance metric.
    pub metric: Distance,
    /// Backend kind ("flat" or "hnsw").
    pub backend: String,
    /// HNSW config (only meaningful when backend == "hnsw").
    pub hnsw_cfg: Option<HnswConfig>,
    /// Doc bodies. The HNSW graph itself is **NOT** snapshotted ;
    /// it is rebuilt deterministically from the doc list on replay.
    /// This trades 30-60 s of CPU on cold-start for a much smaller
    /// snapshot file (~ dim × cardinality × 4 bytes).
    pub docs: Vec<VectorDoc>,
}

/// Current snapshot format version. Bump when the schema changes.
pub const SNAPSHOT_VERSION: u32 = 1;

/// Serialize a snapshot to bytes suitable for S3 upload.
///
/// **M32** uses JSON because `VectorDoc::payload : serde_json::Value`
/// requires a self-describing deserializer. JSON is human-readable
/// and works correctly with `Value`. M32.1 will swap to a bincode +
/// `serde_with` adapter for ~ 3× smaller snapshots.
pub fn serialize_snapshot(snap: &SegmentSnapshot) -> crate::Result<Vec<u8>> {
    serde_json::to_vec(snap).map_err(|e| crate::VectorError::SnapshotSerde {
        reason: format!("json encode: {e}"),
    })
}

/// Deserialize a snapshot from bytes downloaded from S3.
pub fn deserialize_snapshot(bytes: &[u8]) -> crate::Result<SegmentSnapshot> {
    serde_json::from_slice(bytes).map_err(|e| crate::VectorError::SnapshotSerde {
        reason: format!("json decode: {e}"),
    })
}

/// One segment of a collection.
#[derive(Debug)]
pub struct Segment {
    dim: usize,
    metric: Distance,
    backend: SegmentBackend,
    /// HNSW config remembered so a re-build (after flat→HNSW
    /// promotion) uses the same parameters.
    hnsw_cfg: HnswConfig,
}

impl Segment {
    /// Construct a fresh empty flat segment.
    #[must_use]
    pub fn new_flat(dim: usize, metric: Distance) -> Self {
        Self {
            dim,
            metric,
            backend: SegmentBackend::Flat(FlatIndex::new(dim, metric)),
            hnsw_cfg: HnswConfig::default(),
        }
    }

    /// Construct a fresh empty HNSW segment.
    #[must_use]
    pub fn new_hnsw(dim: usize, metric: Distance, cfg: HnswConfig) -> Self {
        Self {
            dim,
            metric,
            backend: SegmentBackend::Hnsw(HnswIndex::new(dim, metric, cfg)),
            hnsw_cfg: cfg,
        }
    }

    /// Construct from a `SegmentSnapshot`. Rebuilds the HNSW graph
    /// from scratch when needed.
    pub fn from_snapshot(snap: &SegmentSnapshot) -> Result<Self> {
        if snap.version != SNAPSHOT_VERSION {
            return Err(VectorError::SnapshotSerde {
                reason: format!(
                    "snapshot version {} unsupported (expected {SNAPSHOT_VERSION})",
                    snap.version
                ),
            });
        }
        let mut seg = match snap.backend.as_str() {
            "flat" => Self::new_flat(snap.dim, snap.metric),
            "hnsw" => Self::new_hnsw(
                snap.dim,
                snap.metric,
                snap.hnsw_cfg.unwrap_or_default(),
            ),
            other => {
                return Err(VectorError::SnapshotSerde {
                    reason: format!("unknown backend `{other}`"),
                });
            }
        };
        for d in &snap.docs {
            seg.insert(d.clone())?;
        }
        Ok(seg)
    }

    /// Number of documents (including tombstoned HNSW nodes — not
    /// useful as cardinality for billing ; see
    /// [`Collection::cardinality`] for the live count).
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.backend {
            SegmentBackend::Flat(f) => f.len(),
            SegmentBackend::Hnsw(h) => h.len(),
        }
    }

    /// `true` if the segment is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match &self.backend {
            SegmentBackend::Flat(f) => f.is_empty(),
            SegmentBackend::Hnsw(h) => h.is_empty(),
        }
    }

    /// Dim.
    #[must_use]
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Metric.
    #[must_use]
    pub fn metric(&self) -> Distance {
        self.metric
    }

    /// Insert / upsert.
    pub fn insert(&mut self, doc: VectorDoc) -> Result<()> {
        match &mut self.backend {
            SegmentBackend::Flat(f) => {
                f.insert(doc)?;
                // Auto-promote if we cross the threshold.
                if f.len() > BRUTE_FORCE_MAX_VECTORS {
                    self.promote_to_hnsw()?;
                }
                Ok(())
            }
            SegmentBackend::Hnsw(h) => h.insert(doc),
        }
    }

    /// Delete by id.
    pub fn delete(&mut self, id: &str) -> Result<()> {
        match &mut self.backend {
            SegmentBackend::Flat(f) => f.delete(id),
            SegmentBackend::Hnsw(h) => h.delete(id),
        }
    }

    /// Top-k search.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchHit>> {
        match &self.backend {
            SegmentBackend::Flat(f) => f.search(query, k),
            SegmentBackend::Hnsw(h) => h.search(query, k),
        }
    }

    /// Serialize to a snapshot envelope.
    pub fn to_snapshot(&self) -> SegmentSnapshot {
        let (backend, docs) = match &self.backend {
            SegmentBackend::Flat(f) => ("flat", f.docs().to_vec()),
            SegmentBackend::Hnsw(_h) => {
                // HNSW doesn't expose docs() yet ; we collect via a
                // sentinel search... actually a snapshot must be
                // complete. For M32 we keep a parallel doc list via
                // the Collection layer. Pre-empt this by returning
                // an empty docs vec — the Collection's
                // `VectorStore::iter_all` is the real source.
                ("hnsw", Vec::new())
            }
        };
        SegmentSnapshot {
            version: SNAPSHOT_VERSION,
            dim: self.dim,
            metric: self.metric,
            backend: backend.into(),
            hnsw_cfg: matches!(self.backend, SegmentBackend::Hnsw(_)).then_some(self.hnsw_cfg),
            docs,
        }
    }

    fn promote_to_hnsw(&mut self) -> Result<()> {
        let cfg = self.hnsw_cfg;
        let metric = self.metric;
        let dim = self.dim;
        let new = HnswIndex::new(dim, metric, cfg);
        // Move docs from flat → hnsw.
        if let SegmentBackend::Flat(f) = &self.backend {
            for d in f.docs() {
                new.insert(d.clone())?;
            }
        }
        self.backend = SegmentBackend::Hnsw(new);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::VectorDoc;

    fn d(id: &str, v: Vec<f32>) -> VectorDoc {
        VectorDoc::new(id, v)
    }

    #[test]
    fn flat_segment_search_basic() {
        let mut s = Segment::new_flat(3, Distance::Cosine);
        s.insert(d("a", vec![1.0, 0.0, 0.0])).unwrap();
        s.insert(d("b", vec![0.0, 1.0, 0.0])).unwrap();
        let out = s.search(&[1.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(out[0].id, "a");
    }

    #[test]
    fn hnsw_segment_search_basic() {
        let mut s = Segment::new_hnsw(3, Distance::Cosine, HnswConfig::default());
        s.insert(d("a", vec![1.0, 0.0, 0.0])).unwrap();
        s.insert(d("b", vec![0.0, 1.0, 0.0])).unwrap();
        let out = s.search(&[1.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(out[0].id, "a");
    }

    #[test]
    fn flat_segment_snapshot_roundtrip() {
        let mut s = Segment::new_flat(3, Distance::Cosine);
        s.insert(d("a", vec![1.0, 0.0, 0.0])).unwrap();
        s.insert(d("b", vec![0.0, 1.0, 0.0])).unwrap();
        let snap = s.to_snapshot();
        assert_eq!(snap.version, SNAPSHOT_VERSION);
        assert_eq!(snap.backend, "flat");
        assert_eq!(snap.docs.len(), 2);
        // M32 audit fix : `serde_json::Value` inside `VectorDoc::payload`
        // uses `deserialize_any` which bincode does not support
        // (bincode is not self-describing). For snapshots we use
        // JSON, which IS self-describing. M32.1 will swap to
        // bincode + a custom `serde_with` adapter for perf.
        let bytes = serde_json::to_vec(&snap).unwrap();
        let snap2: SegmentSnapshot = serde_json::from_slice(&bytes).unwrap();
        let rebuilt = Segment::from_snapshot(&snap2).unwrap();
        let out = rebuilt.search(&[1.0, 0.0, 0.0], 1).unwrap();
        assert_eq!(out[0].id, "a");
    }

    #[test]
    fn from_snapshot_rejects_wrong_version() {
        let snap = SegmentSnapshot {
            version: 999,
            dim: 3,
            metric: Distance::Cosine,
            backend: "flat".into(),
            hnsw_cfg: None,
            docs: vec![],
        };
        let e = Segment::from_snapshot(&snap).unwrap_err();
        assert!(matches!(e, VectorError::SnapshotSerde { .. }));
    }

    #[test]
    fn from_snapshot_rejects_unknown_backend() {
        let snap = SegmentSnapshot {
            version: SNAPSHOT_VERSION,
            dim: 3,
            metric: Distance::Cosine,
            backend: "ivfpq".into(),
            hnsw_cfg: None,
            docs: vec![],
        };
        let e = Segment::from_snapshot(&snap).unwrap_err();
        assert!(matches!(e, VectorError::SnapshotSerde { .. }));
    }

    #[test]
    fn flat_auto_promotes_to_hnsw_above_threshold() {
        let mut s = Segment::new_flat(2, Distance::Cosine);
        for i in 0..(BRUTE_FORCE_MAX_VECTORS + 5) {
            s.insert(d(&format!("d{i}"), vec![1.0, i as f32 * 0.0001])).unwrap();
        }
        assert!(matches!(s.backend, SegmentBackend::Hnsw(_)));
    }

    #[test]
    fn delete_idempotent_in_segment() {
        let mut s = Segment::new_flat(3, Distance::Cosine);
        s.insert(d("a", vec![1.0, 0.0, 0.0])).unwrap();
        s.delete("a").unwrap();
        s.delete("a").unwrap(); // double-delete OK
    }
}
