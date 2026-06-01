// SPDX-License-Identifier: UNLICENSED
//! M32 — BM25 inverted index.
//!
//! Per-collection inverted index. Tokenization is whitespace +
//! ASCII-lowercase + punctuation strip. **Not** a full tantivy
//! replacement — designed for ≤ 100 k unique terms per collection
//! and ≤ 1 M documents.
//!
//! ## Algorithm
//!
//! Standard BM25 (Robertson 1994) :
//!
//! ```text
//!     score(D, Q) = Σ over q in Q of
//!         IDF(q) × ( f(q, D) × (k1+1) ) /
//!                  ( f(q, D) + k1 × (1 - b + b × |D| / avgdl) )
//! ```
//!
//! Defaults : `k1 = 1.2`, `b = 0.75`.

use std::collections::HashMap;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::error::{Result, RetrievalError};

/// BM25 tunables.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Bm25Params {
    /// k1 (term-frequency saturation). Default 1.2.
    pub k1: f32,
    /// b (length normalization). Default 0.75.
    pub b: f32,
}

impl Default for Bm25Params {
    fn default() -> Self {
        Self { k1: 1.2, b: 0.75 }
    }
}

/// One BM25 hit.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bm25Hit {
    /// Document identifier.
    pub id: String,
    /// BM25 score.
    pub score: f32,
}

/// Per-document stats.
#[derive(Debug, Clone)]
struct DocStats {
    length: u32,
    /// term frequency : token → count.
    tf: HashMap<String, u32>,
}

/// Per-term stats.
///
/// **M32 perf (V3)** — the M33 BM25 rewrite stores a **posting
/// list** per term : `(doc_id, tf)` pairs. Search iterates only
/// docs that contain ≥ 1 query token instead of scanning all N
/// docs. On a 100 k-doc corpus with a 3-token query the wall-clock
/// goes from O(N · |q|) ≈ 300 k lookups to O(Σ_t |postings(t)|)
/// — typically 100-1000× faster for low-frequency tokens.
#[derive(Debug, Clone, Default)]
struct TermStats {
    /// Number of docs containing the term (document frequency).
    /// Kept in sync with `postings.len()`.
    df: u32,
    /// Posting list : `doc_id → tf`. `HashMap` gives O(1) insert /
    /// remove for the upsert / delete paths ; iteration is still
    /// cache-friendly on small terms (avg ~ 100 docs / term in our
    /// corpora).
    postings: HashMap<String, u32>,
}

struct Bm25State {
    docs: HashMap<String, DocStats>,
    terms: HashMap<String, TermStats>,
    /// Σ doc lengths (cached for avgdl).
    total_length: u64,
}

impl Bm25State {
    fn new() -> Self {
        Self {
            docs: HashMap::new(),
            terms: HashMap::new(),
            total_length: 0,
        }
    }

    fn avgdl(&self) -> f32 {
        if self.docs.is_empty() {
            return 0.0;
        }
        (self.total_length as f32) / (self.docs.len() as f32)
    }
}

/// BM25 inverted index.
pub struct Bm25Index {
    params: Bm25Params,
    state: RwLock<Bm25State>,
}

impl Default for Bm25Index {
    fn default() -> Self {
        Self::new(Bm25Params::default())
    }
}

impl Bm25Index {
    /// Construct a new empty index.
    #[must_use]
    pub fn new(params: Bm25Params) -> Self {
        Self {
            params,
            state: RwLock::new(Bm25State::new()),
        }
    }

    /// Returns the configured params.
    #[must_use]
    pub fn params(&self) -> Bm25Params {
        self.params
    }

    /// Number of indexed documents.
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.read().docs.len()
    }

    /// `true` if no docs are indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.state.read().docs.is_empty()
    }

    /// Insert (or upsert) a document. Returns the new doc count.
    pub fn upsert(&self, id: &str, text: &str) -> Result<usize> {
        if id.is_empty() {
            return Err(RetrievalError::Internal {
                reason: "doc id is empty".into(),
            });
        }
        let tokens = tokenize(text);
        if tokens.is_empty() {
            // Indexing an empty doc is allowed but no-op for the
            // inverted index (BM25 score will be 0 for any query).
            // Still record the doc so deletes are idempotent.
        }
        let mut tf: HashMap<String, u32> = HashMap::new();
        for t in &tokens {
            *tf.entry(t.clone()).or_insert(0) += 1;
        }
        let length = tokens.len() as u32;

        let mut g = self.state.write();
        // If the doc already exists, decrement old df counts and
        // **remove this doc from each previous-term posting list**.
        if let Some(prev) = g.docs.remove(id) {
            g.total_length -= prev.length as u64;
            for k in prev.tf.keys() {
                if let Some(ts) = g.terms.get_mut(k) {
                    if ts.postings.remove(id).is_some() {
                        ts.df = ts.df.saturating_sub(1);
                    }
                }
            }
        }
        // **V3** : populate the per-term posting list with the new tf
        // values. Also increment df.
        for (k, count) in &tf {
            let ts = g.terms.entry(k.clone()).or_default();
            if ts.postings.insert(id.to_string(), *count).is_none() {
                ts.df += 1;
            }
        }
        g.total_length += length as u64;
        g.docs.insert(
            id.to_string(),
            DocStats {
                length,
                tf,
            },
        );
        Ok(g.docs.len())
    }

    /// Delete a doc. Idempotent. **V3** : also drops every posting-
    /// list entry referencing this doc.
    pub fn delete(&self, id: &str) -> Result<()> {
        let mut g = self.state.write();
        if let Some(prev) = g.docs.remove(id) {
            g.total_length -= prev.length as u64;
            for k in prev.tf.keys() {
                if let Some(ts) = g.terms.get_mut(k) {
                    if ts.postings.remove(id).is_some() {
                        ts.df = ts.df.saturating_sub(1);
                    }
                }
            }
        }
        Ok(())
    }

    /// Search top-k by BM25 score. Empty query → empty result.
    pub fn search(&self, query: &str, k: usize) -> Result<Vec<Bm25Hit>> {
        let q_tokens = tokenize(query);
        if q_tokens.is_empty() {
            return Err(RetrievalError::EmptyQuery);
        }
        if k == 0 {
            return Ok(Vec::new());
        }
        let g = self.state.read();
        let n = g.docs.len() as f32;
        if n == 0.0 {
            return Ok(Vec::new());
        }
        let avgdl = g.avgdl().max(1.0);
        let Bm25Params { k1, b } = self.params;

        // **V3** : iterate posting lists (only docs containing ≥ 1
        // query token) instead of scanning all N docs. Per-token
        // IDF + per-doc length normalization are computed once.
        let mut scores: HashMap<&str, f32> = HashMap::with_capacity(64);
        // Deduplicate query tokens once — common in real-world
        // queries ("the the the" → "the").
        let mut seen: std::collections::HashSet<&str> =
            std::collections::HashSet::with_capacity(q_tokens.len());
        for tok in q_tokens.iter() {
            if !seen.insert(tok.as_str()) {
                continue;
            }
            let ts = match g.terms.get(tok) {
                Some(t) => t,
                None => continue,
            };
            let df = ts.df as f32;
            if df == 0.0 {
                continue;
            }
            // Robertson-Walker IDF (add-1 smoothing).
            let idf = (((n - df + 0.5) / (df + 0.5)) + 1.0).ln();
            for (doc_id, &f_u32) in &ts.postings {
                let f = f_u32 as f32;
                let dl = g.docs.get(doc_id).map(|d| d.length).unwrap_or(0) as f32;
                let denom = f + k1 * (1.0 - b + b * dl / avgdl);
                let term_score = idf * (f * (k1 + 1.0)) / denom;
                *scores.entry(doc_id.as_str()).or_insert(0.0) += term_score;
            }
        }

        // Top-k sort — early-truncate via partial sort.
        let mut out: Vec<Bm25Hit> = scores
            .into_iter()
            .map(|(id, score)| Bm25Hit {
                id: id.to_string(),
                score,
            })
            .collect();
        out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out.truncate(k);
        Ok(out)
    }
}

/// Tokenize : lowercase + strip ASCII punctuation + split on
/// whitespace. Stopwords NOT filtered (kept for recall ; BM25 IDF
/// naturally suppresses common words).
fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_basic() {
        let toks = tokenize("Hello, world! 123 foo-bar");
        assert_eq!(toks, vec!["hello", "world", "123", "foo", "bar"]);
    }

    #[test]
    fn empty_index_search_returns_empty() {
        let idx = Bm25Index::default();
        let out = idx.search("hello", 5).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn upsert_then_search_finds_doc() {
        let idx = Bm25Index::default();
        idx.upsert("d1", "the quick brown fox").unwrap();
        idx.upsert("d2", "the lazy dog").unwrap();
        let out = idx.search("fox", 5).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "d1");
        assert!(out[0].score > 0.0);
    }

    #[test]
    fn search_rejects_empty_query() {
        let idx = Bm25Index::default();
        idx.upsert("d1", "hello").unwrap();
        let e = idx.search("", 5).unwrap_err();
        assert!(matches!(e, RetrievalError::EmptyQuery));
    }

    #[test]
    fn rare_term_outranks_common_term() {
        let idx = Bm25Index::default();
        for i in 0..50 {
            idx.upsert(&format!("d{i}"), "the the the the the").unwrap();
        }
        idx.upsert("rare", "the quick").unwrap();
        let out = idx.search("the quick", 5).unwrap();
        // `quick` is rare → `rare` should rank #1.
        assert_eq!(out[0].id, "rare");
    }

    #[test]
    fn delete_is_idempotent() {
        let idx = Bm25Index::default();
        idx.delete("never").unwrap();
        idx.upsert("d1", "hello").unwrap();
        idx.delete("d1").unwrap();
        idx.delete("d1").unwrap();
        assert_eq!(idx.len(), 0);
    }

    #[test]
    fn upsert_replaces_existing() {
        let idx = Bm25Index::default();
        idx.upsert("d1", "hello world").unwrap();
        idx.upsert("d1", "goodbye world").unwrap();
        // `hello` should no longer match.
        let out = idx.search("hello", 5).unwrap();
        assert!(out.is_empty());
        // `goodbye` should match.
        let out = idx.search("goodbye", 5).unwrap();
        assert_eq!(out[0].id, "d1");
    }

    #[test]
    fn empty_text_upsert_is_searchable_as_zero_score() {
        let idx = Bm25Index::default();
        idx.upsert("d1", "").unwrap();
        let out = idx.search("anything", 5).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn top_k_truncates() {
        let idx = Bm25Index::default();
        for i in 0..20 {
            idx.upsert(&format!("d{i}"), &format!("foo {i}")).unwrap();
        }
        let out = idx.search("foo", 5).unwrap();
        assert!(out.len() <= 5);
    }
}
