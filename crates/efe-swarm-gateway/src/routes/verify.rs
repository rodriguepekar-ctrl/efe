// SPDX-License-Identifier: UNLICENSED
//! `GET /v1/swarm/verify/:id` — **re-prouver (le moat)**.
//!
//! `verify_run_offline` **recalcule** la chaîne d'essaim (contenu + DAG), la chaîne
//! forensique N17, ET le pont Mode A ↔ Mode C, depuis les seuls records persistés + la
//! clé tenant. Même si le store a été altéré après coup, une chaîne casse → `verified=false`
//! et la faute est localisée. L'endpoint ne demande pas de *croire* le store ; il
//! *re-prouve* — la forensique LLM comprise (ADR-016).
//!
//! # P.0 — l'intégrité ne suffit pas : il faut aussi la COMPLÉTUDE
//!
//! Re-calculer prouve que *ce qui est là* n'a pas bougé ; cela ne prouve **pas** que *tout* est
//! encore là. Un **préfixe** d'une chaîne HMAC valide est lui-même une chaîne HMAC valide :
//! retirer les derniers enregistrements d'un run laisse `chain_intact`, `content_intact` et
//! `dag_consistent` à `true`. Seule la **racine** change.
//!
//! La racine scellée à l'exécution est pourtant persistée (`StoredRun::root`) — et la comparaison
//! correcte existait déjà un fichier plus loin, dans le replay R0
//! (`routes/replay.rs` : `reproduced_root == stored.root`). Elle manquait **ici**, sur la route
//! qui porte le verdict public. Sans elle, un tiers disposant de l'écriture en base et
//! **d'aucune clé** obtenait `verified: true` sur un run tronqué.
//!
//! Ce module confronte donc la racine **reconstruite** à la racine **scellée** et intègre le
//! résultat au verdict. Le principe de non-accusation rétroactive est préservé : un run sans
//! racine persistée rend `root_matches: None` et conserve son verdict.

use axum::extract::{Path, State};
use axum::Json;
use efe_swarm_provenance::verify_run_offline;
use uuid::Uuid;

use crate::auth::Caller;
use crate::dto::{N17VerifyView, VerifyResponse};
use crate::error::GatewayError;
use crate::state::SwarmGatewayState;

pub async fn verify(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Path(id): Path<Uuid>,
) -> Result<Json<VerifyResponse>, GatewayError> {
    caller.require_any_role(&["viewer", "operator", "admin", "auditor"])?;

    // **M2.** Un 404 nu confond deux faits très différents : « cet identifiant n'existe pas » et
    // « ce run a bien eu lieu, et nous n'avons pas pu le persister ». Le second est une BRÈCHE, et
    // la présenter comme une absence reviendrait à EFFACER un fait au lieu de le documenter —
    // exactement ce que la chaîne de provenance existe pour empêcher. On consulte donc le journal
    // avant de conclure, et on répond 410 (« la ressource a existé, elle est définitivement
    // partie ») plutôt que 404.
    // **M2** : `charger_run` distingue « inconnu » (404) de « servi mais non persisté » (410).
    // Un 404 nu EFFACERAIT un fait au lieu de le documenter. La décision vit en un seul endroit —
    // `capture::absence_expliquee` — parce que sept routes lisent un run, et que sept décisions
    // indépendantes divergent : le premier audit de M2 avait justement corrigé celle-ci et laissé
    // les six autres.
    let stored = s.charger_run(id, &caller.tenant).await?;
    let key = s.derive_key(&caller.tenant)?; // HKDF par tenant (server-side)

    // ← le vérificateur INDÉPENDANT : essaim + chaîne N17 + pont, sans aucun provider.
    let v = verify_run_offline(
        &stored.records,
        &stored.llm_records,
        &stored.n17_leaf_bytes(),
        stored.n17_root_bytes(),
        &key,
    );
    // Comptabilise la re-preuve d'essaim dans les métriques partagées (ruptures détectées).
    s.prov_metrics.observe_verify(&v.swarm);

    // ── P.0 : la COMPLÉTUDE ────────────────────────────────────────────────────────────────
    // `root_hex()` ne rend `Some` que si la chaîne se recompose (`verify_chain` pose
    // `root: None` sur toute rupture) ; il rend aussi `None` sur un run **vide**, que
    // `verify_swarm` déclare par ailleurs trivialement intègre.
    //
    // D'où la règle, volontairement asymétrique : **c'est la racine SCELLÉE qui commande**.
    // Dès qu'un run en porte une, la re-preuve DOIT la reproduire — sinon le run n'est pas
    // celui qui a été scellé. Cette formulation ferme trois cas d'un coup :
    //   • troncature de queue  → reconstruite ≠ scellée      ⟹ Some(false)
    //   • suppression totale   → reconstruite None           ⟹ Some(false)  (le trou du run vide)
    //   • rupture de chaîne    → reconstruite None           ⟹ Some(false)  (déjà non vérifié)
    // Et elle n'accuse jamais un run sans racine scellée (⟹ None, verdict inchangé).
    //
    // Comparaison de chaînes exacte : les deux racines sont produites par le même `hex32`
    // (minuscules, 64 caractères) — côté scellement comme côté re-preuve.
    let recomputed_root = v.swarm.root_hex();
    let root_matches: Option<bool> =
        stored.root.as_deref().map(|sealed| recomputed_root.as_deref() == Some(sealed));

    // Le verdict public ne peut pas être positif sur un run incomplet.
    let verified = v.fully_proven && root_matches != Some(false);

    Ok(Json(VerifyResponse {
        run_id: id,
        // ADR-016 : le verdict GLOBAL inclut la forensique N17 + le pont.
        // P.0 : il inclut désormais aussi la complétude (racine scellée reproduite).
        verified,
        root: recomputed_root,
        root_matches,
        chain_intact: v.swarm.chain_intact,
        content_intact: v.swarm.content_intact,
        dag_consistent: v.swarm.dag_consistent,
        traced_pct: v.swarm.traced_pct,
        records: v.swarm.records,
        first_break: v.swarm.first_break,
        n17: N17VerifyView {
            records: stored.llm_records.len(),
            chain_intact: v.n17_chain.intact,
            root: v.n17_chain.root_hex(),
            bridge_consistent: v.bridge.all_mode_c_consistent(),
            mode_c_consistent: v.bridge.consistent,
            mode_c_unlinked: v.bridge.unlinked,
            mode_c_broken: v.bridge.broken,
            // ADR-016 : surface la faute forensique localisée (QUEL appel LLM a été muté).
            first_break: v.n17_chain.first_break.map(|p| p.as_uuid().to_string()),
        },
    }))
}
