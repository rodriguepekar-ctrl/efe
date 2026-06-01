//! [`ResourceLimits`] — per-call resource caps for nano agent execution.

use serde::{Deserialize, Serialize};

/// Per-call resource limits enforced by the runtime.
///
/// Mode A (pure Rust) enforces : `cpu_ms`, `wall_clock_ms`.
/// Mode B (local model) adds : `memory_mb`.
/// Mode C (LLM-augmented) adds : `llm_tokens`, `network_egress_kb`.
///
/// # Examples
///
/// ```
/// use efe_nano_sdk::types::ResourceLimits;
///
/// let lim = ResourceLimits::default_mode_a();
/// assert_eq!(lim.memory_mb, 256);
/// assert_eq!(lim.cpu_ms, 1000);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Max resident memory (MB).
    pub memory_mb: u32,

    /// Max CPU time (ms).
    pub cpu_ms: u32,

    /// Max wall-clock time (ms).
    pub wall_clock_ms: u32,

    /// Max LLM tokens consumable (None = unlimited at this layer, deferred to budget).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_tokens: Option<u32>,

    /// Max network egress in KB (Mode A = 0, Mode B/C variable).
    pub network_egress_kb: u32,
}

impl ResourceLimits {
    /// Default limits for Mode A (pure Rust) nano agents.
    ///
    /// - 256 MB memory, 1 second CPU, 5 seconds wall-clock, 0 network egress.
    pub fn default_mode_a() -> Self {
        Self {
            memory_mb: 256,
            cpu_ms: 1_000,
            wall_clock_ms: 5_000,
            llm_tokens: None,
            network_egress_kb: 0,
        }
    }

    /// Default limits for Mode B (local model) nano agents.
    ///
    /// - 2 GB memory, 5 seconds CPU, 10 seconds wall-clock, 0 network.
    pub fn default_mode_b() -> Self {
        Self {
            memory_mb: 2_048,
            cpu_ms: 5_000,
            wall_clock_ms: 10_000,
            llm_tokens: None,
            network_egress_kb: 0,
        }
    }

    /// Default limits for Mode C (LLM-augmented) nano agents.
    ///
    /// - 512 MB memory, 10 seconds CPU, 30 seconds wall-clock, 4096 tokens.
    pub fn default_mode_c() -> Self {
        Self {
            memory_mb: 512,
            cpu_ms: 10_000,
            wall_clock_ms: 30_000,
            llm_tokens: Some(4_096),
            network_egress_kb: 256,
        }
    }
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self::default_mode_a()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_a_values() {
        let l = ResourceLimits::default_mode_a();
        assert_eq!(l.memory_mb, 256);
        assert_eq!(l.cpu_ms, 1_000);
        assert_eq!(l.wall_clock_ms, 5_000);
        assert_eq!(l.llm_tokens, None);
        assert_eq!(l.network_egress_kb, 0);
    }

    #[test]
    fn default_mode_b_more_memory() {
        let a = ResourceLimits::default_mode_a();
        let b = ResourceLimits::default_mode_b();
        assert!(b.memory_mb > a.memory_mb);
        assert!(b.wall_clock_ms > a.wall_clock_ms);
    }

    #[test]
    fn default_mode_c_has_tokens() {
        let c = ResourceLimits::default_mode_c();
        assert!(c.llm_tokens.is_some());
        assert!(c.network_egress_kb > 0);
    }

    #[test]
    fn serde_round_trip_mode_a() {
        let l = ResourceLimits::default_mode_a();
        let s = serde_json::to_string(&l).unwrap();
        // llm_tokens skipped because None
        assert!(!s.contains("llm_tokens"));
        let l2: ResourceLimits = serde_json::from_str(&s).unwrap();
        assert_eq!(l, l2);
    }

    #[test]
    fn serde_round_trip_mode_c() {
        let l = ResourceLimits::default_mode_c();
        let s = serde_json::to_string(&l).unwrap();
        // llm_tokens is Some(4096) → present
        assert!(s.contains("llm_tokens"));
        assert!(s.contains("4096"));
        let l2: ResourceLimits = serde_json::from_str(&s).unwrap();
        assert_eq!(l, l2);
    }
}
