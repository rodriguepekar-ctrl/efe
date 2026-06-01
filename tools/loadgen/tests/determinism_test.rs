//! PR #20 § 33.4 — determinism integration tests.
//!
//! Verifies that same `(seed, profile)` produces bit-identical request
//! sequences.  This is the contractual property CI relies on for
//! regression detection.

use efe_meta_swarm_loadgen::determinism::{SeedRegistry, SubStream};
use rand::Rng;

#[test]
fn same_seed_produces_same_payload_stream() {
    let reg = SeedRegistry::new(42, 0);
    let mut a = reg.rng(SubStream::Payload);
    let mut b = SeedRegistry::new(42, 0).rng(SubStream::Payload);
    for _ in 0..1000 {
        let va: f64 = a.gen_range(-1.0..=1.0);
        let vb: f64 = b.gen_range(-1.0..=1.0);
        assert!((va - vb).abs() < 1e-12, "payload stream diverged : {va} vs {vb}");
    }
}

#[test]
fn different_seed_diverges_quickly() {
    let mut a = SeedRegistry::new(42, 0).rng(SubStream::Payload);
    let mut b = SeedRegistry::new(43, 0).rng(SubStream::Payload);
    let aa: Vec<f64> = (0..8).map(|_| a.gen_range(-1.0..=1.0)).collect();
    let bb: Vec<f64> = (0..8).map(|_| b.gen_range(-1.0..=1.0)).collect();
    // Two seeds differing by 1 should diverge in the first sample for xoshiro.
    assert_ne!(aa, bb);
}

#[test]
fn pod_ordinal_partitions_seed_space() {
    let pod0 = SeedRegistry::new(42, 0);
    let pod1 = SeedRegistry::new(42, 1);
    assert_ne!(pod0.pod_seed(), pod1.pod_seed());
    let mut a = pod0.rng(SubStream::Payload);
    let mut b = pod1.rng(SubStream::Payload);
    let aa: Vec<u64> = (0..8).map(|_| rand::RngCore::next_u64(&mut a)).collect();
    let bb: Vec<u64> = (0..8).map(|_| rand::RngCore::next_u64(&mut b)).collect();
    assert_ne!(aa, bb);
}

#[test]
fn substreams_are_independent() {
    let reg = SeedRegistry::new(42, 0);
    let mut payload = reg.rng(SubStream::Payload);
    let mut target = reg.rng(SubStream::TargetSelection);
    let mut think = reg.rng(SubStream::ThinkTime);
    let p1: u64 = rand::RngCore::next_u64(&mut payload);
    let t1: u64 = rand::RngCore::next_u64(&mut target);
    let h1: u64 = rand::RngCore::next_u64(&mut think);
    // All three streams should differ on the first sample.
    assert_ne!(p1, t1);
    assert_ne!(p1, h1);
    assert_ne!(t1, h1);
}

#[test]
fn iterate_workload_build_input_is_deterministic() {
    use efe_meta_swarm_loadgen::workload::iterate::IterateWorkload;
    let w = IterateWorkload::new(64);
    let mut rng1 = SeedRegistry::new(42, 0).rng(SubStream::Payload);
    let mut rng2 = SeedRegistry::new(42, 0).rng(SubStream::Payload);
    let a = w.build_input(&mut rng1, 0);
    let b = w.build_input(&mut rng2, 0);
    assert_eq!(a.initial, b.initial, "iterate payload diverged across runs");
}
