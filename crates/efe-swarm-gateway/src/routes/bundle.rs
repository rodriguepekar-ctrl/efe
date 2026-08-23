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
//! # L'attestation (P.3)
//!
//! Si — et seulement si — une clé privée d'attestation est déployée, la réponse porte une
//! signature **ed25519** de la racine. Un tiers qui a **épinglé la clé publique hors bande** peut
//! alors conclure que cette racine a bien été émise ici, **sans détenir aucun secret**. Sans clé
//! déployée, le champ est **absent** — jamais vide, ce qui se confondrait avec une signature.
//!
//! La route ne sert **jamais la clé publique** : la servir à côté du bundle rendrait la
//! vérification circulaire (n'importe qui peut engendrer une paire et signer). Seule l'empreinte
//! (`key_id`) est rendue, pour que l'auditeur sache *laquelle* de ses clés épinglées appliquer.
//!
//! # L'ancre (P.4)
//!
//! Si un support de publication est configuré, la route écrit l'**ancre** du bundle — la racine
//! attendue et son compte de nœuds — **hors** du bundle, dans un journal append-only. C'est le seul
//! geste qui rende une troncature détectable par quelqu'un ne détenant **aucune clé**, à condition
//! que l'auditeur reçoive ce journal par un **canal distinct** de celui du bundle.
//!
//! La publication échoue **fort** : si elle rate, la route rend une erreur au lieu de servir un
//! bundle annonçant `anchored: true` — un auditeur chercherait alors une ancre inexistante.
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
use provenance_bridge::{anchor_of, bundle_from_run, RunView, MAPPING_VERSION};
use uuid::Uuid;

use crate::auth::Caller;
use crate::dto::{AttestationView, BundleResponse};
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

    let stored = s.charger_run(id, &caller.tenant).await?;
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

    // ── P.3 : l'attestation de racine ──────────────────────────────────────────────────────
    // Aucune clé déployée ⟹ le champ est ABSENT, jamais vide : un champ vide se confondrait avec
    // une signature. L'horloge est prise ICI et non dans le pont, qui reste pur.
    let attestation = s.bundle_signer.as_ref().map(|signer| {
        let signed_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        AttestationView::from(signer.attest(id, &caller.tenant, &b, signed_at))
    });

    // ── P.4 : publier l'ancre HORS du bundle ───────────────────────────────────────────────
    // C'est le seul geste qui rend une troncature détectable par quelqu'un ne détenant AUCUNE clé.
    // Il échoue FORT : servir un bundle en annonçant `anchored: true` alors que la publication a
    // échoué serait un « vert malhonnête » — l'auditeur chercherait une ancre qui n'existe pas.
    // Mieux vaut refuser la réponse que mentir sur sa garantie.
    let anchored = match s.anchor_sink.as_ref() {
        None => false,
        Some(sink) => {
            let derived_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
                .unwrap_or(0);
            let anchor = anchor_of(id, &caller.tenant, &b, derived_at)
                .map_err(|e| GatewayError::Internal(format!("ancre inconstructible — {e}")))?;
            sink.publish(&anchor)
                .map_err(|e| GatewayError::Internal(format!("ancre NON publiée — {e}")))?;
            true
        }
    };

    Ok(Json(BundleResponse {
        run_id: id,
        bundle_jsonl: b.jsonl,
        manifest_root: b.manifest_root,
        node_count: b.node_count,
        mapping_version: MAPPING_VERSION.to_string(),
        // `true` SEULEMENT si la publication a réellement réussi (sinon on a déjà rendu une erreur).
        // Sans support d'ancrage, `false` : le bundle ne prouve alors sa cohérence que contre
        // lui-même, et le dire vaut mieux que laisser un lecteur le supposer.
        anchored,
        attestation,
    }))
}
