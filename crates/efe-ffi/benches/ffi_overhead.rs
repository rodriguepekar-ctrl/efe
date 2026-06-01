// **Audit fix Φ-C-FFI.A4** : `criterion_group!` macro expansion
// produces an undocumented function ; `#![allow(missing_docs)]` at
// the file root silences the lint without polluting individual bench
// fns (which are all documented).
#![allow(missing_docs)]

//! Criterion bench — Sprint **Φ-C-FFI Completion (D26)**.
//!
//! Cf. `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md` § 14.1.
//!
//! ## Mission
//!
//! Mesurer l'overhead **pur FFI** sans gRPC, sans Python interpréteur,
//! sans I/O bundle. Permet de localiser la dépense réelle d'un
//! `classify` côté Rust :
//!
//!   1. `synthesise_verdict_only` — coût du stub D25 (= base line ;
//!      pure SHA-256 mix + label lookup). Cible : < 5 µs sur tout
//!      runner moderne.
//!   2. `pyverdict_construct` — coût de la construction `PyVerdict`
//!      (= struct allocation + 5 string clones). Cible : < 1 µs.
//!   3. `cache_hit_lookup` — coût d'un hit LRU bundle cache.
//!      Cible : < 200 ns.
//!   4. `bundle_encode_d25_stub` — coût du SHA-256 encode du stub.
//!      Cible : < 50 µs (avec dim=384).
//!
//! ## Output
//!
//! Criterion écrit dans `target/criterion/<bench>/` :
//!   * `estimates.json` — point + intervalle de confiance bootstrapped.
//!   * `report/index.html` — graphes HTML.
//!
//! Le pont `scripts/bench/criterion_ffi_to_slo.py` lit
//! `estimates.json` et produit le JSON SLO-compliance parseable
//! par `check_slo.py` :
//!
//!   p99_seconds (= mean + 3σ approximation pour Criterion samples).
//!
//! ## Comment lancer
//!
//! ```bash
//! cargo bench -p efe-ffi --bench ffi_overhead
//! # ou pour un seul case :
//! cargo bench -p efe-ffi --bench ffi_overhead -- synthesise_verdict_only
//! ```

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

use efe_ffi::bundle::InnerBundle;
use efe_ffi::cache::CacheKey;
use efe_ffi::controller::synthesise_verdict;
use efe_ffi::verdict::PyVerdict;

// ────────────────────────────────────────────────────────────────────
// Fixtures synthétiques — ne touchent ni Python ni disque.
// ────────────────────────────────────────────────────────────────────

fn synth_inner_bundle() -> InnerBundle {
    InnerBundle {
        id: "bench-bundle".to_string(),
        version: "0.0.0-bench".to_string(),
        path: "/dev/null".into(),
        fingerprint: [0u8; 32],
        dim: 384,
    }
}

fn synth_labels() -> Vec<String> {
    vec![
        "billing-dispute".into(),
        "tech-support".into(),
        "refund-request".into(),
        "general-inquiry".into(),
        "other".into(),
    ]
}

fn synth_embedding(dim: usize) -> Vec<f32> {
    // Vecteur synthétique réaliste : 384 dims uniformes sur [-0.5, 0.5].
    // Évite NaN/Inf.
    let mut v = Vec::with_capacity(dim);
    for i in 0..dim {
        v.push(((i % 256) as f32) / 256.0 - 0.5);
    }
    v
}

// ────────────────────────────────────────────────────────────────────
// Bench 1 — synthesise_verdict_only (= stub D25 pure)
// ────────────────────────────────────────────────────────────────────

fn bench_synthesise_verdict_only(c: &mut Criterion) {
    let labels = synth_labels();
    let embedding = synth_embedding(384);
    let mut group = c.benchmark_group("synthesise_verdict_only");
    group.throughput(Throughput::Elements(1));
    group.bench_function("dim_384_5_labels", |b| {
        b.iter(|| {
            let v = synthesise_verdict(
                black_box(&embedding),
                black_box("router-v1"),
                black_box("1.0.0"),
                black_box(&labels),
                black_box(42),
                black_box(0.0),
            );
            black_box(v);
        })
    });
    group.finish();
}

// ────────────────────────────────────────────────────────────────────
// Bench 2 — PyVerdict construct (= measure struct alloc + string clones)
// ────────────────────────────────────────────────────────────────────

fn bench_pyverdict_construct(c: &mut Criterion) {
    c.bench_function("pyverdict_construct", |b| {
        b.iter(|| {
            let v = PyVerdict::new(
                black_box("billing-dispute".to_string()),
                black_box(0.9876),
                black_box("cone-router-v1".to_string()),
                black_box(12),
                black_box("swarm-abcdef0123456789".to_string()),
                black_box("1.0.0".to_string()),
                black_box(vec![]),
                black_box(0.0),
            );
            black_box(v);
        })
    });
}

// ────────────────────────────────────────────────────────────────────
// Bench 3 — CacheKey probe (= syscall fs::metadata)
//
// Mesure le coût d'une probe de clé pour le LRU cache. Ce
// `cache.get(key)` est l'opération hot en `classify_zero_copy`
// (= chaque appel commence par construire la clé). Cible : < 5 µs
// sur un FS local.
//
// Note : pas de bench `cache_hit_lookup` parce qu'un hit nécessite
// un `Arc<PyBundle>` qui n'est constructible qu'à travers `load_bundle`
// (= touche disque) ou un test-only constructor. Le bench
// `cache_key_probe` couvre la partie commune.
// ────────────────────────────────────────────────────────────────────

fn bench_cache_key_probe(c: &mut Criterion) {
    // Setup : touch un fichier réel sur le FS local.
    let path = std::env::temp_dir().join("efe_ffi_bench_probe");
    let _ = std::fs::write(&path, b"x");
    c.bench_function("cache_key_probe_existing_file", |b| {
        b.iter(|| {
            let key = CacheKey::probe(black_box(&path));
            black_box(key);
        })
    });
    let _ = std::fs::remove_file(&path);
}

// ────────────────────────────────────────────────────────────────────
// Bench 4 — bundle encode stub D25 (= SHA-256 mix + L2 normalize)
// ────────────────────────────────────────────────────────────────────

fn bench_bundle_encode_stub(c: &mut Criterion) {
    let inner = synth_inner_bundle();
    let mut group = c.benchmark_group("bundle_encode");
    group.throughput(Throughput::Elements(1));
    group.bench_function("d25_stub_dim_384", |b| {
        b.iter(|| {
            let v = inner.encode(black_box("the quick brown fox jumps over the lazy dog"));
            black_box(v);
        })
    });
    group.finish();
}

// ────────────────────────────────────────────────────────────────────
// Bench 5 — synthesise_verdict + pyverdict_construct end-to-end FFI
// stub (= total cost si on assemblait classify_zero_copy hors Python).
// ────────────────────────────────────────────────────────────────────

fn bench_end_to_end_stub(c: &mut Criterion) {
    let labels = synth_labels();
    let inner = synth_inner_bundle();
    c.bench_function("end_to_end_stub_no_python", |b| {
        b.iter(|| {
            // 1. Encode.
            let emb = inner.encode(black_box("hello world"));
            // 2. Synthesise verdict.
            let v = synthesise_verdict(
                black_box(&emb),
                black_box("router-v1"),
                black_box("1.0.0"),
                black_box(&labels),
                black_box(42),
                black_box(0.0),
            );
            black_box(v);
        })
    });
}

// ────────────────────────────────────────────────────────────────────
// Criterion glue.
// ────────────────────────────────────────────────────────────────────

criterion_group!(
    benches,
    bench_synthesise_verdict_only,
    bench_pyverdict_construct,
    bench_cache_key_probe,
    bench_bundle_encode_stub,
    bench_end_to_end_stub,
);
criterion_main!(benches);
