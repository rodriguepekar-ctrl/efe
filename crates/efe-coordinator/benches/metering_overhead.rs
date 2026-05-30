// SPDX-License-Identifier: UNLICENSED
//! **R3.S7** — Bench Criterion : overhead du hook télémétrie.
//!
//! Prouve le critère d'acceptation §16 : `record_mission` < 1 µs (donc le
//! hook ajoute < 1 ms / mission, négligeable devant une latence LLM réseau).
//!
//! Usage :
//! ```bash
//! cargo bench -p efe-coordinator --bench metering_overhead
//! ```
//! Résultats HTML : `target/criterion/`.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use efe_coordinator::{
    FailureKind, MetricsRegistry, MissionRecord, MissionType, Outcome, Phase, SavingsMode, Verdict,
};

/// Enregistrement typique d'une mission réussie (cas chaud du hook).
fn typical_record() -> MissionRecord {
    MissionRecord {
        ty: MissionType::A,
        outcome: Outcome::Success,
        provider: "claude_claude-sonnet-4-6".to_string(),
        cost_microcents: 558_000,
        phase_latencies: vec![
            (Phase::Analyzer, 40),
            (Phase::Compressor, 120),
            (Phase::Verifier, 80),
            (Phase::Llm, 480),
        ],
        savings: Some((SavingsMode::Speculative, 5_000)),
        verdicts: vec![Verdict::Supported, Verdict::Unsupported],
        failure_kinds: vec![FailureKind::LlmFallbackTriggered],
    }
}

fn bench_record_mission(c: &mut Criterion) {
    let reg = MetricsRegistry::new();
    let rec = typical_record();
    // Cible : < 1 µs / itération (cf. §16). Lookup enum + quelques fetch_add.
    c.bench_function("metering/record_mission", |b| {
        b.iter(|| reg.record_mission(black_box(&rec)));
    });
}

fn bench_record_error(c: &mut Criterion) {
    let reg = MetricsRegistry::new();
    let rec = MissionRecord::error(MissionType::B);
    c.bench_function("metering/record_mission_error", |b| {
        b.iter(|| reg.record_mission(black_box(&rec)));
    });
}

fn bench_render(c: &mut Criterion) {
    // Le rendu n'a lieu qu'au scrape (hors chemin mission) — on le mesure
    // quand même pour dimensionner le coût d'un scrape Prometheus.
    let reg = MetricsRegistry::new();
    let rec = typical_record();
    for _ in 0..1_000 {
        reg.record_mission(&rec);
    }
    c.bench_function("metering/render_prometheus", |b| {
        b.iter(|| black_box(reg.render_prometheus(black_box(&[]))));
    });
}

criterion_group!(benches, bench_record_mission, bench_record_error, bench_render);
criterion_main!(benches);
