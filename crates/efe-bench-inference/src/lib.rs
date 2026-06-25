// SPDX-License-Identifier: UNLICENSED
//! # `efe-bench-inference` — B1, le harnais d'inférence (le déblocage).
//!
//! L'instrument de mesure de la thèse : « beaucoup de très petits modèles en parallèle valent —
//! voire battent — un gros modèle, pour moins cher ». Une affirmation quantitative se **mesure**.
//! Ce harnais répond aux trois questions à la fois — **est-ce juste ? à quel coût ? prouvable ?** —
//! sur un corpus labellisé, et il se construit et se **prouve entièrement sur Mac** via le seam
//! (B0.3). Le GPU (B2) ne change qu'**une ligne** : le backend scripté devient un vrai endpoint vLLM.
//!
//! Il **pilote le SDK directement** (Mode C) ; il n'écrit PAS de client HTTP (le SDK parle déjà à
//! vLLM/Ollama) et ne touche PAS au loadgen gRPC de `bench/` (charge, pas justesse).
//!
//! ## Les sept invariants
//!
//! - **I-BENCH-TRUTH** — toute tâche mesurée porte un label ; l'exactitude = `scorer(gagnant, label)` ;
//!   le scorer doit correspondre au type de tâche (v1 fermée). Cf. [`dataset`] / [`scorer`].
//! - **I-BENCH-BLIND** — les générateurs ne voient jamais le label ; il n'atteint que le scorer
//!   post-hoc (toujours) et l'oracle interne (opt-in). Cf. [`dataset::BenchCase::blind_task`].
//! - **I-BENCH-SAME-ORACLE** — le scorer `Exact` et l'oracle interne partagent un unique noyau
//!   (`EqualsExpected`). Cf. [`scorer::ScorerKind::is_correct`].
//! - **I-BENCH-FAIR** — les comparaisons partagent dataset, scorer et comptabilité. Cf. [`report::Comparison`].
//! - **I-BENCH-PROXY** — le coût-jetons est estimé (proxy) sur backend scripté, mesuré dès qu'un
//!   vLLM rapporte l'usage ; le drapeau remonte au rapport. Cf. [`runner::RunRecord`].
//! - **I-BENCH-REPROVE** — chaque run est scellé + R0-re-prouvable ; le bundle est re-vérifiable et
//!   infalsifiable (atteste la computation, pas la correction du label). Cf. [`report::BenchBundle`].
//! - **I-BENCH-MAC** — tout est exercé sur Mac via le seam (justesse scriptée contrôlable) avant le
//!   GPU ; latence/débit/coût-mesuré réels sont GPU-only (B2). Cf. `tests/bench_mac_seam.rs`.
//!
//! ## Usage
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use efe_bench_inference::{BenchBackend, BenchConfig, Comparison, Dataset, PriceModel, run_config};
//! use efe_swarm_sdk::TopologySpec;
//!
//! let dataset = Dataset::from_jsonl(include_str!("../fixtures/topics.jsonl"))?;
//!
//! // Chemin réel (GPU / B2) : l'unique aiguillage. Sur Mac, BenchBackend::Scripted(...) via le seam.
//! let backend = BenchBackend::SelfHost { endpoint: "http://127.0.0.1:8000/v1".into(), model: "qwen2.5:3b".into() };
//!
//! // La thèse : l'essaim (K petits) vs un gros modèle de référence (K=1), même dataset/scorer.
//! let swarm_cfg = BenchConfig::new("swarm-k16", TopologySpec::consensus("Classify the topic in one word.", &["consistency"], 16));
//! let base_cfg  = BenchConfig::new("baseline-k1", TopologySpec::GenerateVerify {
//!     generator: "Classify the topic in one word.".into(), verifier: "non_empty_text".into(), k: 1usize.into(),
//! });
//!
//! let a = run_config(&dataset, &swarm_cfg, &backend, 1_000_000, 8).await?;
//! let b = run_config(&dataset, &base_cfg, &backend, 1_000_000, 8).await?;
//!
//! let cmp = Comparison::new(vec![a, b]).with_price(PriceModel::new().with("swarm-k16", 0.2).with("baseline-k1", 4.0));
//! println!("{}", cmp.table());
//! # Ok(()) }
//! ```

pub mod aggregation;
pub mod corpus;
pub mod dataset;
pub mod reexec;
pub mod report;
pub mod runner;
pub mod scorer;
pub mod substitution;

pub use aggregation::{
    curve, knee, verdict_at_k, verdict_sup_k, CurvePoint, DeployableVerifier, HbVerdict, IoPrice, Mechanism,
    PositiveInteger, ReasoningWeigher, SampleSet, SupVerdict,
};
pub use reexec::ReExecAudit;
pub use dataset::{BenchCase, Dataset, DatasetError};
pub use report::{reprove, BenchBundle, BundleVerdict, Comparison, ConfigReport, PriceModel, ReproofVerdict};
pub use runner::{
    consensus_sweep, run_config, run_config_with_evidence, BenchBackend, BenchConfig, RunEvidence, RunRecord,
};
pub use scorer::ScorerKind;
pub use substitution::{
    achieved_power, analyze_cell, condition_on_present, discordance, mcnemar, non_inferiority, normal_cdf,
    pair_by_case, probit, required_n, wilson_ci, Discordance, GapCell, GapMap, McNemar, NonInferiority, PairedItem,
    SubstitutionError, Verdict,
};
