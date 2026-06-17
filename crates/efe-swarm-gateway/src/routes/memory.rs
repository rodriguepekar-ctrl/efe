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

use efe_swarm_memory::{seal_memory, verify_memory, FamilyKey};

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
    /// `pattern → victoires` (le prior de pattern ; vide en MEM-2, cf. doc SDK).
    pattern_wins: BTreeMap<String, u64>,
    /// Nombre d'exemplaires retenus (borné par le cap) + les meilleurs.
    exemplars: usize,
    top_exemplars: Vec<ExemplarView>,
    /// Nombre d'agents avec une réputation.
    agents: usize,
}

/// La **re-dérivation** du `memory_root` : la racine recomposée depuis le journal + verdict.
#[derive(Debug, Serialize)]
struct MemoryVerifyView {
    family: String,
    /// Nombre de replis (FoldRecords) dans le journal.
    records: usize,
    /// `memory_root` (hex) **recomposé** depuis le journal + la clé tenant — la propriété de
    /// **re-provabilité hors-ligne** : un tiers détenant le journal + la racine légitimement
    /// publiée retrouve la MÊME valeur (le moat appliqué à la mémoire, I-MEM5).
    memory_root: Option<String>,
    /// Le journal se re-scelle de façon cohérente. NB : ceci n'est PAS une détection de
    /// falsification du store — le journal EST la source de vérité (les deux stores le
    /// re-dérivent). Détecter une édition hors-bande des lignes exigerait un **témoin racine
    /// persisté** (comme la chaîne N17 persiste `n17_root`) ; c'est un durcissement Pg à venir
    /// (MEM-3.b). La détection+localisation d'une falsification, elle, est prouvée au niveau
    /// `efe_swarm_memory::verify_memory` (gardiens MEM-1, avec un témoin scellé).
    intact: bool,
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
    };
    Ok(Json(view).into_response())
}

/// **`GET /v1/swarm/memory/:family/verify`** — **re-dérive** le `memory_root` d'un tenant depuis
/// le journal des replis et la clé tenant (HKDF, server-side, jamais exposée), et l'expose. C'est
/// la surface de **re-provabilité** appliquée à la mémoire (I-MEM5) : un tiers détenant le journal
/// et la clé retrouve la MÊME racine. Tenant-scopé (canonicalisé) : la chaîne d'un autre tenant est
/// invérifiable avec cette clé (I-MEM6). **Honnêteté de portée** : l'endpoint re-DÉRIVE (le journal
/// est la source de vérité des deux stores) ; il ne compare PAS à un témoin racine persisté, donc
/// il ne détecte pas une édition hors-bande des lignes (détection et localisation par
/// `verify_memory`, gardiens MEM-1 ; un témoin persisté côté Pg est un durcissement à venir,
/// MEM-3.b). Toujours 200.
pub async fn verify_memory_chain(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Path(family): Path<String>,
) -> Result<Response, GatewayError> {
    caller.require_any_role(&["operator", "admin"])?;
    // Canonicalise le tenant (cf. get_memory) — la clé ET le journal résolus sous la MÊME
    // identité (sinon le verify d'un alias mélangeait clé canonique et journal brut).
    let tenant = s.tenant_id(&caller.tenant)?.to_string();
    let fam = FamilyKey::from_hex(family.clone());
    let key = s.derive_key(&caller.tenant)?;
    let journal = s
        .memory
        .journal(&tenant, &fam)
        .await
        .map_err(|e| GatewayError::Internal(format!("journal mémoire : {e}")))?;

    let sealed = seal_memory(&journal, &key);
    let verdict = verify_memory(&journal, &sealed.leaf_hashes, sealed.memory_root, &key);
    let view = MemoryVerifyView {
        family,
        records: verdict.records,
        memory_root: verdict.root_hex(),
        intact: verdict.intact,
    };
    Ok((axum::http::StatusCode::OK, Json(view)).into_response())
}
