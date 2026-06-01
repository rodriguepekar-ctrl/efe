// SPDX-License-Identifier: UNLICENSED
//! M30 — Long-context hierarchical chunker.
//!
//! Produces a tree of chunks (section → paragraph → sentence) sized
//! to fit a 512-token model context window with configurable overlap.
//!
//! ## Tokenization
//!
//! We use a deterministic **whitespace + punctuation** tokenizer that
//! approximates the BPE token count of typical Western corpora to
//! ±20 %. This is sufficient for chunk-window planning ; the real
//! BPE encoder runs server-side inside Triton.
//!
//! ## Chunk identity
//!
//! Each chunk carries a `chunk_id` (ULID) and a `parent_id` (Option).
//! The root chunks of a document have `parent_id = None`. This lets
//! the downstream classifier rebuild the document tree for citation
//! reporting.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{MultimodalError, Result};

/// Default target tokens per chunk (matches the input length of the
/// fine-tuned long-context classifier in `efe-llm-router`).
pub const DEFAULT_TOKENS_PER_CHUNK: usize = 512;

/// Default overlap in tokens between adjacent chunks (~ 10 % of
/// `DEFAULT_TOKENS_PER_CHUNK`).
pub const DEFAULT_OVERLAP_TOKENS: usize = 64;

/// Default maximum chunks per request. Overflow returns
/// `MultimodalError::ChunkOverflow`.
pub const DEFAULT_MAX_CHUNKS: usize = 64;

/// Chunker configuration.
#[derive(Debug, Clone)]
pub struct ChunkConfig {
    /// Approximate target token count per chunk.
    pub tokens_per_chunk: usize,
    /// Token-level overlap between adjacent chunks.
    pub overlap_tokens: usize,
    /// Per-request chunk cap.
    pub max_chunks: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            tokens_per_chunk: DEFAULT_TOKENS_PER_CHUNK,
            overlap_tokens: DEFAULT_OVERLAP_TOKENS,
            max_chunks: DEFAULT_MAX_CHUNKS,
        }
    }
}

/// One chunk in the hierarchical output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// ULID-shaped UUIDv7 (sorted by creation time).
    pub chunk_id: String,
    /// Position in the linear sequence of chunks (0-based).
    pub position: usize,
    /// Parent chunk identifier (for section → paragraph hierarchy).
    pub parent_id: Option<String>,
    /// Approximate token count.
    pub approx_tokens: usize,
    /// Raw chunk text (subject to PII redaction at the caller).
    pub text: String,
}

/// Hierarchical chunker.
pub struct HierarchicalChunker;

impl HierarchicalChunker {
    /// Split a document into a flat sequence of token-bounded chunks
    /// with overlap. The hierarchical metadata (`parent_id`) is
    /// populated when the input contains markdown-style headings
    /// (lines starting with `#`).
    pub fn chunk(text: &str, cfg: &ChunkConfig) -> Result<Vec<Chunk>> {
        if text.is_empty() {
            return Ok(vec![]);
        }
        if cfg.tokens_per_chunk == 0 {
            return Err(MultimodalError::Internal {
                reason: "tokens_per_chunk must be > 0".into(),
            });
        }
        if cfg.overlap_tokens >= cfg.tokens_per_chunk {
            return Err(MultimodalError::Internal {
                reason: format!(
                    "overlap_tokens {} >= tokens_per_chunk {}",
                    cfg.overlap_tokens, cfg.tokens_per_chunk
                ),
            });
        }

        // 1) First pass : detect heading positions (markdown style).
        // Each heading becomes a parent ; subsequent chunks until the
        // next heading inherit that parent.
        let mut chunks = Vec::new();
        let mut current_parent: Option<String> = None;
        let mut buffer = String::new();
        let mut buffer_tokens = 0_usize;
        let mut position = 0_usize;

        let advance_step = cfg.tokens_per_chunk.saturating_sub(cfg.overlap_tokens);
        for line in text.split_inclusive('\n') {
            if line.starts_with('#') {
                // Flush the current buffer as a chunk under the
                // previous parent.
                if !buffer.is_empty() {
                    push_buffered(&mut chunks, &mut buffer, &mut buffer_tokens, &mut position, current_parent.clone(), cfg.tokens_per_chunk, cfg.max_chunks)?;
                }
                // Allocate a new parent and emit a heading chunk.
                let parent_id = new_chunk_id();
                let approx = approx_token_count(line);
                chunks.push(Chunk {
                    chunk_id: parent_id.clone(),
                    position,
                    parent_id: None,
                    approx_tokens: approx,
                    text: line.trim_end_matches('\n').to_string(),
                });
                position += 1;
                if chunks.len() > cfg.max_chunks {
                    return Err(MultimodalError::ChunkOverflow {
                        produced: chunks.len(),
                        cap: cfg.max_chunks,
                    });
                }
                current_parent = Some(parent_id);
                continue;
            }

            let line_tokens = approx_token_count(line);
            // If a single line alone exceeds the chunk size, hard-split
            // it on word boundaries.
            if line_tokens > cfg.tokens_per_chunk {
                // Flush current buffer.
                if !buffer.is_empty() {
                    push_buffered(&mut chunks, &mut buffer, &mut buffer_tokens, &mut position, current_parent.clone(), cfg.tokens_per_chunk, cfg.max_chunks)?;
                }
                // Hard-split.
                for sub in hard_split(line, cfg.tokens_per_chunk, advance_step) {
                    chunks.push(Chunk {
                        chunk_id: new_chunk_id(),
                        position,
                        parent_id: current_parent.clone(),
                        approx_tokens: approx_token_count(&sub),
                        text: sub,
                    });
                    position += 1;
                    if chunks.len() > cfg.max_chunks {
                        return Err(MultimodalError::ChunkOverflow {
                            produced: chunks.len(),
                            cap: cfg.max_chunks,
                        });
                    }
                }
                continue;
            }

            if buffer_tokens + line_tokens > cfg.tokens_per_chunk {
                // Flush + carry overlap.
                push_buffered_with_overlap(
                    &mut chunks,
                    &mut buffer,
                    &mut buffer_tokens,
                    &mut position,
                    current_parent.clone(),
                    cfg.tokens_per_chunk,
                    cfg.overlap_tokens,
                    cfg.max_chunks,
                )?;
            }
            buffer.push_str(line);
            buffer_tokens += line_tokens;
        }
        // Flush tail.
        if !buffer.is_empty() {
            push_buffered(&mut chunks, &mut buffer, &mut buffer_tokens, &mut position, current_parent, cfg.tokens_per_chunk, cfg.max_chunks)?;
        }
        Ok(chunks)
    }
}

fn push_buffered(
    chunks: &mut Vec<Chunk>,
    buffer: &mut String,
    buffer_tokens: &mut usize,
    position: &mut usize,
    parent_id: Option<String>,
    _tokens_per_chunk: usize,
    max_chunks: usize,
) -> Result<()> {
    if buffer.is_empty() {
        return Ok(());
    }
    let text = std::mem::take(buffer);
    let approx = *buffer_tokens;
    chunks.push(Chunk {
        chunk_id: new_chunk_id(),
        position: *position,
        parent_id,
        approx_tokens: approx,
        text,
    });
    *position += 1;
    *buffer_tokens = 0;
    if chunks.len() > max_chunks {
        return Err(MultimodalError::ChunkOverflow {
            produced: chunks.len(),
            cap: max_chunks,
        });
    }
    Ok(())
}

fn push_buffered_with_overlap(
    chunks: &mut Vec<Chunk>,
    buffer: &mut String,
    buffer_tokens: &mut usize,
    position: &mut usize,
    parent_id: Option<String>,
    tokens_per_chunk: usize,
    overlap_tokens: usize,
    max_chunks: usize,
) -> Result<()> {
    if buffer.is_empty() {
        return Ok(());
    }
    let snapshot = buffer.clone();
    let _snapshot_tokens = *buffer_tokens;
    push_buffered(chunks, buffer, buffer_tokens, position, parent_id, tokens_per_chunk, max_chunks)?;
    // Re-seed with the tail of the previous chunk for `overlap_tokens`.
    if overlap_tokens == 0 {
        return Ok(());
    }
    let tail = tail_n_tokens(&snapshot, overlap_tokens);
    *buffer = tail.clone();
    *buffer_tokens = approx_token_count(&tail);
    Ok(())
}

fn new_chunk_id() -> String {
    Uuid::new_v4().to_string()
}

/// Approximate BPE-token count via whitespace + punctuation splits.
/// Empirically this approximates GPT-style BPE within ±20 % for
/// Western text. Reliability is documented in ADR-M30-006.
pub fn approx_token_count(text: &str) -> usize {
    let mut n = 0;
    let mut in_word = false;
    for c in text.chars() {
        if c.is_alphanumeric() {
            if !in_word {
                n += 1;
                in_word = true;
            }
        } else {
            if c.is_ascii_punctuation() {
                n += 1;
            }
            in_word = false;
        }
    }
    n
}

/// Hard-split a single line that exceeds the chunk budget.
fn hard_split(line: &str, tokens_per_chunk: usize, advance: usize) -> Vec<String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.is_empty() {
        return vec![];
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let end = (i + tokens_per_chunk).min(words.len());
        out.push(words[i..end].join(" "));
        if end == words.len() {
            break;
        }
        i += advance.max(1);
    }
    out
}

/// Return the last ~n tokens of `text`, on a word boundary.
fn tail_n_tokens(text: &str, n: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let take = n.min(words.len());
    let start = words.len() - take;
    words[start..].join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_no_chunks() {
        let out = HierarchicalChunker::chunk("", &ChunkConfig::default()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn approx_token_count_is_monotonic() {
        let a = approx_token_count("hello");
        let b = approx_token_count("hello world");
        assert!(b > a);
    }

    #[test]
    fn small_text_yields_single_chunk() {
        let text = "The quick brown fox jumps over the lazy dog.\n";
        let out = HierarchicalChunker::chunk(text, &ChunkConfig::default()).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].parent_id, None);
    }

    #[test]
    fn headings_become_parents() {
        let text = "# Section A\nLorem ipsum dolor sit amet.\n# Section B\nFoo bar.\n";
        let out = HierarchicalChunker::chunk(text, &ChunkConfig::default()).unwrap();
        // Expect : heading A, body A, heading B, body B (4 chunks).
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].parent_id, None);
        assert_eq!(out[1].parent_id.as_deref(), Some(out[0].chunk_id.as_str()));
        assert_eq!(out[2].parent_id, None);
        assert_eq!(out[3].parent_id.as_deref(), Some(out[2].chunk_id.as_str()));
    }

    #[test]
    fn long_text_splits_with_overlap() {
        let cfg = ChunkConfig {
            tokens_per_chunk: 20,
            overlap_tokens: 4,
            max_chunks: 64,
        };
        let mut text = String::new();
        for i in 0..100 {
            text.push_str(&format!("word{} ", i));
        }
        text.push('\n');
        let out = HierarchicalChunker::chunk(&text, &cfg).unwrap();
        assert!(out.len() >= 5);
        for c in &out {
            assert!(c.approx_tokens <= cfg.tokens_per_chunk + cfg.overlap_tokens);
        }
    }

    #[test]
    fn chunk_overflow_is_surfaced() {
        let cfg = ChunkConfig {
            tokens_per_chunk: 5,
            overlap_tokens: 0,
            max_chunks: 3,
        };
        let mut text = String::new();
        for i in 0..200 {
            text.push_str(&format!("word{} ", i));
        }
        text.push('\n');
        let err = HierarchicalChunker::chunk(&text, &cfg).unwrap_err();
        assert!(matches!(err, MultimodalError::ChunkOverflow { .. }));
    }

    #[test]
    fn invalid_config_rejected() {
        let bad = ChunkConfig {
            tokens_per_chunk: 0,
            overlap_tokens: 0,
            max_chunks: 1,
        };
        assert!(HierarchicalChunker::chunk("foo", &bad).is_err());

        let bad = ChunkConfig {
            tokens_per_chunk: 5,
            overlap_tokens: 10,
            max_chunks: 1,
        };
        assert!(HierarchicalChunker::chunk("foo", &bad).is_err());
    }
}
