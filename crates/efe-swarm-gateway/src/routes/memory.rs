// SPDX-License-Identifier: UNLICENSED
//! `GET /v1/swarm/memory/:family` (instantané) + `GET …/memory/:family/verify` (re-preuve du
//! `memory_root`) — ADR-019 (MEM-3). Le pendant de `/runs/:id` + `/verify/:id` pour la mémoire :
//! lire ce qu'un tenant a appris d'une famille, et **re-prouver** que la chaîne mémoire est
//! honnête (le moat appliqué à la mémoire, I-MEM5). **Tenant-scopé** : un tenant ne voit + ne
//! re-prouve QUE sa propre mémoire (I-MEM6) — la famille d'un autre tenant est invisible (vide).

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use efe_swarm_memory::{verify_memory, FamilyKey};

use crate::auth::Caller;
use crate::error::GatewayError;
use crate::state::SwarmGatewayState;

/// Un exemplaire (few-shot) tel qu'exposé : hash de contenu + qualité + aperçu (déjà court).
#[derive(Debug, Serialize)]
struct ExemplarView {
    content_hash: String,
    quality_milli: u32,
    preview: String,
}

/// Un compteur d'échec tel qu'exposé : runs observés, échecs, et le taux en milli-unités.
#[derive(Debug, Serialize)]
struct FailRecView {
    runs: u64,
    fails: u64,
    fail_rate_milli: u32,
}

/// La **vue d'instantané** d'une famille : version + épingle + un résumé lisible des cellules.
#[derive(Debug, Serialize)]
struct MemorySnapshotView {
    family: String,
    /// = nombre de replis pliés (0 = famille jamais vue).
    version: u64,
    /// L'épingle (hex) — ce qu'un run recall-erait + ce que R1 ré-épinglerait (I-MEM1/I-MEM7).
    content_hash: String,
    /// Histogramme `k_used → compte` (le prior de taille qui amorce le warm-start).
    size_histogram: BTreeMap<u32, u64>,
    /// L'amorce `k_min` (p25 du prior) — informative, NON bornée ici (le run la clampe, I-MEM4).
    suggested_k_min_p25: Option<u32>,
    /// `pattern → victoires` (le prior de pattern, peuplé depuis MEM-2.b par les runs consensus).
    pattern_wins: BTreeMap<String, u64>,
    /// Nombre d'exemplaires retenus (borné par le cap) + les meilleurs.
    exemplars: usize,
    top_exemplars: Vec<ExemplarView>,
    /// Nombre d'agents avec une réputation.
    agents: usize,
    /// `clé (famille/agent) → {runs, fails, taux}` — la **mémoire d'échec** (Cell 5), peuplée
    /// au niveau famille par les runs propres (`fails` = runs sans réponse utilisable).
    failures: BTreeMap<String, FailRecView>,
}

/// La **re-preuve** du `memory_root` : le journal courant re-dérivé et comparé au **témoin
/// persisté** (MEM-3.b) → verdict honnête (peut casser).
#[derive(Debug, Serialize)]
struct MemoryVerifyView {
    family: String,
    /// Nombre de replis (FoldRecords) dans le journal.
    records: usize,
    /// `memory_root` (hex) re-dérivé du journal courant — la **re-provabilité hors-ligne** : un
    /// tiers détenant le journal + la clé retrouve la MÊME valeur (le moat appliqué, I-MEM5).
    memory_root: Option<String>,
    /// **Intègre** : le journal courant re-dérivé correspond au **témoin racine persisté** (root +
    /// feuilles scellées au dernier repli, conservé SÉPARÉMENT du journal — le jumeau de
    /// `n17_root`). `false` = soit une **falsification** (édition **hors-bande** d'une ligne →
    /// `localized_break` la situe), soit un journal **non ancré** (`unanchored:true` — témoin perdu,
    /// pas une attaque). Sans la clé tenant, le témoin HMAC n'est pas re-forgeable → la détection
    /// d'une falsification est réelle, pas tautologique.
    intact: bool,
    /// Le repli fautif **localisé** (par `run_id`), quand une mutation de contenu est détectée.
    localized_break: Option<String>,
    /// **NON ANCRÉ** : un journal NON vide sans témoin persisté (table des témoins tronquée /
    /// témoin perdu) — `intact:false` mais ce n'est **PAS** une falsification détectée, juste une
    /// absence d'ancre (on ne peut pas prouver). Distingue la **perte de donnée** d'une **attaque**
    /// (revue adversariale) : ne pas crier à l'édition hors-bande là où il n'y a pas de témoin.
    unanchored: bool,
}

/// **`GET /v1/swarm/memory/:family`** — l'instantané courant de la mémoire d'un tenant pour une
/// famille (hex). Lecture seule, tenant-scopée. Famille jamais vue ⇒ instantané genèse (v0, vide).
pub async fn get_memory(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Path(family): Path<String>,
) -> Result<Response, GatewayError> {
    caller.require_any_role(&["operator", "admin"])?;
    // CANONICALISE le tenant comme le chemin de run (tenant_id : "system"→UUID système, valide
    // l'UUID). Sinon, l'alias "system" (ou un UUID non canonique) lirait sous une AUTRE clé de
    // map que celle où le run a replié → vue vide trompeuse. Bonus : 400 sur un tenant bogué.
    let tenant = s.tenant_id(&caller.tenant)?.to_string();
    let fam = FamilyKey::from_hex(family.clone());
    let snap = s
        .memory
        .snapshot(&tenant, &fam)
        .await
        .map_err(|e| GatewayError::Internal(format!("lecture mémoire : {e}")))?;

    let cells = &snap.cells;
    let top_exemplars = cells
        .exemplars
        .top(8)
        .iter()
        .map(|e| ExemplarView {
            content_hash: e.content_hash.clone(),
            quality_milli: e.quality_milli,
            preview: e.preview.clone(),
        })
        .collect();
    let view = MemorySnapshotView {
        family,
        version: snap.version,
        content_hash: snap.content_hash_hex(),
        size_histogram: cells.size.histogram.clone(),
        suggested_k_min_p25: cells.size.percentile(25),
        pattern_wins: cells.pattern.wins.clone(),
        exemplars: cells.exemplars.items.len(),
        top_exemplars,
        agents: cells.reputation.agents.len(),
        failures: cells
            .failure
            .keys
            .iter()
            .map(|(k, r)| {
                (k.clone(), FailRecView { runs: r.runs, fails: r.fails, fail_rate_milli: r.fail_rate_milli() })
            })
            .collect(),
    };
    Ok(Json(view).into_response())
}

/// **`GET /v1/swarm/memory/:family/verify`** — re-prouve la chaîne `memory_root` d'un tenant : on
/// re-dérive le **journal courant** et on le compare au **témoin racine persisté** (root + feuilles
/// scellées au dernier repli, conservé SÉPARÉMENT du journal — le jumeau de `n17_root`). C'est la
/// **re-provabilité + tamper-evidence** appliquée à la mémoire (I-MEM5, MEM-3.b) : une édition
/// **hors-bande** d'une ligne du store (le journal n'est plus celui qui a produit le témoin) casse
/// la chaîne (`intact:false`) et est **localisée**. Sans la clé tenant, le témoin HMAC n'est pas
/// re-forgeable → la détection est RÉELLE (≠ re-sceller le journal contre lui-même, tautologique).
/// Tenant-scopé/canonicalisé (clé + journal sous la MÊME identité, I-MEM6). Famille jamais repliée
/// ⇒ témoin absent ⇒ journal vide ⇒ intègre (genèse). Toujours 200 (le verdict est dans le corps).
pub async fn verify_memory_chain(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Path(family): Path<String>,
) -> Result<Response, GatewayError> {
    caller.require_any_role(&["operator", "admin"])?;
    // Canonicalise le tenant (cf. get_memory) — la clé, le journal ET le témoin résolus sous la
    // MÊME identité (sinon le verify d'un alias mélangeait clé canonique et journal brut).
    let tenant = s.tenant_id(&caller.tenant)?.to_string();
    let fam = FamilyKey::from_hex(family.clone());
    let key = s.derive_key(&caller.tenant)?;
    let journal = s
        .memory
        .journal(&tenant, &fam)
        .await
        .map_err(|e| GatewayError::Internal(format!("journal mémoire : {e}")))?;
    let witness = s
        .memory
        .witness(&tenant, &fam)
        .await
        .map_err(|e| GatewayError::Internal(format!("témoin mémoire : {e}")))?;

    // On compare au TÉMOIN persisté (≠ re-sceller le journal contre lui-même). Famille neuve ⇒
    // pas de témoin ⇒ feuilles vides + racine None ⇒ journal vide trivialement intègre.
    // **NON ANCRÉ** (revue) : un journal NON vide SANS témoin (table tronquée / témoin perdu) n'est
    // PAS une falsification — juste une absence d'ancre. On le signale à part (`unanchored`), sans
    // localisation (aucune édition de ligne détectée), plutôt que de le confondre avec une attaque.
    let unanchored = witness.is_none() && !journal.is_empty();
    let (leaves, root) = match &witness {
        Some(w) => (w.leaf_hashes.as_slice(), Some(w.memory_root)),
        None => ([].as_slice(), None),
    };
    let verdict = verify_memory(&journal, leaves, root, &key);
    let view = MemoryVerifyView {
        family,
        records: verdict.records,
        memory_root: verdict.root_hex(),
        intact: verdict.intact,
        // Pas de localisation pour un journal non ancré : il n'y a pas de repli « fautif ».
        localized_break: if unanchored { None } else { verdict.first_break.map(|u| u.to_string()) },
        unanchored,
    };
    Ok((axum::http::StatusCode::OK, Json(view)).into_response())
}
