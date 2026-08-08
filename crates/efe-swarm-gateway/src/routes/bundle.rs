// SPDX-License-Identifier: UNLICENSED
//! `GET /v1/swarm/bundle/:id` — **Fil P (P.2)** : exporter le reçu sous une forme que la pile B
//! sait re-prouver.
//!
//! # Ce que cette route change, et ce qu'elle ne change pas
//!
//! Elle ne produit **aucune information neuve**. Le bundle est une **vue** du reçu déjà scellé : il
//! change le *vérifieur*, pas la vérité. Concrètement, l'auditeur qui reçoit ce bundle peut le
//! re-prouver avec `verify-mini` — un binaire dont l'arbre de dépendances est **littéralement
//! vide**, donc dont il peut lire la totalité de la source avant de lui accorder sa confiance.
//! C'est là toute la valeur : il n'a plus à croire notre vérificateur sur parole.
//!
//! # Ce qui ne sort JAMAIS par cette route
//!
//! - **La clé.** Elle voyage hors bande. Sans elle le bundle ne se re-prouve pas — c'est voulu.
//! - **Les sorties.** Le bundle ne porte que des empreintes, préfixées `blake3:`.
//!
//! # Les gardes
//!
//! Drapeau absent ⟹ la route n'est **pas montée** (404, exactement comme avant P.2 : le
//! comportement servi par défaut est byte-identique). Rôle : `auditor` ou `admin` — plus strict
//! que les routes de lecture, parce qu'un bundle est un export de la provenance entière. Run
//! inconnu ou d'un autre tenant ⟹ 404 (isolation, comme `verify`). Dérivation refusée ⟹ **500 sans
//! bundle partiel** : un bundle amputé serait indistinguable d'un bundle tronqué par un attaquant.
//!
//! # Le refus qui compte
//!
//! Si le run ne reproduit pas sa racine scellée, la dérivation **refuse** et la route renvoie 500.
//! Ce n'est pas une commodité : donner un bundle re-prouvable à un run tronqué **blanchirait**
//! l'attaque — on offrirait un témoin crédible à un reçu menteur, l'inverse exact du but du fil.
//! P.0 a rendu ce mensonge visible sur `verify` ; P.2 refuse de le rendre exportable.

use axum::extract::{Path, State};
use axum::Json;
use provenance_bridge::{bundle_from_run, RunView, MAPPING_VERSION};
use uuid::Uuid;

use crate::auth::Caller;
use crate::dto::BundleResponse;
use crate::error::GatewayError;
use crate::state::SwarmGatewayState;

pub async fn bundle(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Path(id): Path<Uuid>,
) -> Result<Json<BundleResponse>, GatewayError> {
    // Un export de provenance entière n'est pas une lecture ordinaire : `viewer` et `operator`
    // n'y ont pas accès.
    caller.require_any_role(&["auditor", "admin"])?;

    let stored = s.store.get(id, &caller.tenant).await?.ok_or(GatewayError::NotFound)?;
    // La clé de la PILE A : elle sert à re-prouver la complétude du run, puis à dériver la clé de
    // la pile B (voir `derive_bundle_key` — deux clés distinctes, dérivation à sens unique).
    let a_key = s.derive_key(&caller.tenant)?;

    let view = RunView::new(
        id,
        &caller.tenant,
        &stored.records,
        &stored.llm_records,
        stored.root.as_deref(),
    );

    // Jamais de bundle partiel : toute anomalie remonte, aucune n'est rattrapée en silence.
    let b = bundle_from_run(&view, &a_key).map_err(|e| {
        GatewayError::Internal(format!("bundle de provenance non produit — {e}"))
    })?;

    Ok(Json(BundleResponse {
        run_id: id,
        bundle_jsonl: b.jsonl,
        manifest_root: b.manifest_root,
        node_count: b.node_count,
        mapping_version: MAPPING_VERSION.to_string(),
        // P.4 n'est pas fait : la racine n'est publiée nulle part, donc le bundle ne prouve sa
        // cohérence que contre lui-même. Le dire ici plutôt que de laisser un lecteur le supposer.
        anchored: false,
    }))
}
