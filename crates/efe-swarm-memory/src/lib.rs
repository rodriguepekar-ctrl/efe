// SPDX-License-Identifier: UNLICENSED
//! # `efe-swarm-memory` — la mémoire/apprentissage inter-runs (ADR-SWARM-019, lot MEM-1)
//!
//! Un essaim EFE était, jusqu'ici, une **fonction sans mémoire** : chaque run repartait de
//! zéro. Ce crate donne à l'essaim une **mémoire qui apprend d'un run à l'autre — sans
//! détruire le moat** (déterminisme ADR-007, re-preuve ADR-017). La thèse, en une phrase :
//!
//! > **La mémoire est une ENTRÉE — épinglée, hachée, tracée** — au même titre que la tâche.
//! > On apprend **entre** les runs (un repli explicite, signé, post-run, gardé) ; on lit un
//! > **instantané gelé** pendant le run (identifié par un `content_hash` qui entre dans la
//! > provenance). La mémoire elle-même est **re-prouvable** : sa chaîne
//! > `vN = HMAC(vN-1, FoldRecord)` se re-scelle hors ligne en un `memory_root` — *le moat
//! > appliqué à la mémoire*.
//!
//! ## Ce que couvre MEM-1 (ce crate) — le socle PUR, testable seul
//! - [`family`] — la `FamilyKey` qui regroupe les tâches « semblables » (empreinte **exacte**
//!   ou **structurelle**, toutes deux *pures et déterministes* ; la sémantique est un opt-in
//!   de MEM-3).
//! - [`cells`] — les **cinq espèces** de cellules apprises (priors de taille / pattern,
//!   exemplaires, réputation, mémoire d'échec) + l'instantané [`cells::MemorySnapshot`].
//!   **Entiers uniquement** (aucun `f64` dans le contenu haché) → `content_hash` byte-stable.
//! - [`fold`] — le `RunSummary` (la projection minimale d'un run que la mémoire consomme) et
//!   la **fonction de repli pure** par espèce, plus [`fold::materialize`] (replie le journal en
//!   ordre canonique → cellules). **Influence bornée** : la mémoire AMORCE, ne gouverne jamais.
//! - [`chain`] — le `FoldRecord`, [`chain::seal_memory`] et [`chain::verify_memory`] — le
//!   **jumeau exact** de `seal_n17`/`verify_n17_chain` (même `chain_hmac`, même clé tenant,
//!   ordre canonique ADR-007) → le `memory_root` re-prouvable + localisation d'une falsification.
//! - [`store`] — le trait [`store::MemoryStore`] (async, tenant-scopé, jumeau du `RunStore`) +
//!   l'impl en mémoire : **porte d'apprentissage** (seuls les runs `clean` replient, I-MEM3),
//!   **idempotence** par `run_id`, **isolation tenant** (clé HKDF par tenant).
//!
//! ## Ce que MEM-1 NE fait PAS (lots suivants)
//! - **MEM-2** : câbler recall/learn dans `run_traced` (le record `MemoryRead` dans la
//!   provenance, le warm-start du sizer, le repli post-scellement gardé par `is_clean()`).
//! - **MEM-3** : la passerelle (champ DTO `memory`, endpoints, gouvernance, store Postgres,
//!   extension R1 d'ADR-017, empreinte sémantique).
//!
//! Ce crate est **déterministe et sans réseau** : tout y est une fonction pure du journal des
//! replis. C'est ce qui le rend re-prouvable — et testable seul.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod cells;
pub mod chain;
pub mod family;
pub mod fold;
pub mod store;

pub use cells::{
    AgentRep, Exemplar, FailRec, FailureMemory, MemoryCells, MemorySnapshot, PatternBias,
    PatternPrior, Reputation, SizePrior,
};
pub use chain::{
    seal_memory, verify_memory, FoldRecord, MemoryBreak, MemoryChainVerdict, MemoryWitness, SealedMemory,
};
pub use family::{family_key, FamilyKey, FamilyMode};
pub use fold::{materialize, AgentStat, RunSummary};
pub use store::{FoldOutcome, InMemoryMemoryStore, MemError, MemoryStore, MemoryUpdate};

/// Encodage hexadécimal minuscule d'un hash 32 octets (persistance / fil). Local au crate
/// pour rester découplé des internes de `efe-swarm-provenance` (qui a son propre `hex32`).
pub(crate) fn hex32(b: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for byte in b {
        s.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap());
    }
    s
}

/// Le hash de contenu **déterministe** d'une valeur sérialisable : **BLAKE3** de sa forme JSON
/// **canonique** (clés triées, sans espace — `efe_swarm_provenance::canonical_bytes`). C'est
/// l'épingle d'ADR-017 appliquée à la mémoire (ADR-019 §P1.5), et la primitive partagée par la
/// feuille de chaîne et le `content_hash` d'instantané : insensible à l'ordre de sérialisation,
/// donc reproductible par un tiers. **BLAKE3** (et non SHA-256) pour rester COHÉRENT avec tout le
/// reste de la couche provenance — `input_hash`/`output_hash` (`blake3_value`) et l'adresse de
/// contenu des exemplaires (`Exemplar.content_hash`) sont déjà BLAKE3 : un tiers re-dérivant
/// « l'épingle » selon l'ADR calcule BLAKE3 et retrouve la MÊME valeur.
pub(crate) fn content_hash_of<T: serde::Serialize>(value: &T) -> [u8; 32] {
    let v = serde_json::to_value(value).expect("type sérialisable → Value total");
    efe_swarm_provenance::blake3_value(&v).0
}

/// Le hash **SHA-256** du contenu canonique — la **feuille** de la chaîne `memory_root`. Distinct
/// de [`content_hash_of`] (BLAKE3) À DESSEIN : la chaîne mémoire est le **jumeau exact** de la
/// chaîne N17 (ADR-016), dont la feuille est `SHA-256(canonical_bytes(record))`
/// (`efe_swarm_provenance::seal_n17`). Garder la MÊME crypto de feuille rend le « jumeau exact »
/// littéralement vrai. Règle de la couche : les **adresses de contenu** (épingle d'instantané,
/// clé de famille, hash d'exemplaire) sont en BLAKE3 ; les **feuilles de chaîne tamper-evidentes**
/// (N17, mémoire) sont en SHA-256.
pub(crate) fn sha256_content_of<T: serde::Serialize>(value: &T) -> [u8; 32] {
    let v = serde_json::to_value(value).expect("type sérialisable → Value total");
    efe_swarm_provenance::sha256(&efe_swarm_provenance::canonical_bytes(&v))
}
