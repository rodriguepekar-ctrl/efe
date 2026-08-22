// SPDX-License-Identifier: UNLICENSED
//! `GET /v1/swarm/gaps` — **M2** : le canal de l'auditeur.
//!
//! ## Pourquoi une route, alors que `/metrics` compte déjà
//!
//! Les trois canaux d'une brèche ne s'adressent pas au même lecteur, et aucun ne remplace l'autre :
//!
//! * **le client** apprend la sienne dans la réponse du run — mais lui seul, et une seule fois ;
//! * **l'exploitant** voit un compteur monter dans `/metrics` — mais un compteur ne dit pas *quels*
//!   runs manquent, et un auditeur ne peut rien faire d'un nombre ;
//! * **l'auditeur** a besoin des identifiants, des horodatages et des raisons. C'est cette route.
//!
//! ## Ce que cette route ne prétend pas être
//!
//! Elle n'est **pas** une preuve. Le journal vit en mémoire : il ne survit pas au redémarrage du
//! processus, et un exploitant malveillant peut le vider en relançant le binaire. Il détecte la
//! **panne**, pas la **malveillance** — la même honnêteté que le reste du dépôt, où le reçu est
//! *tamper-evident pour un détenteur de clé*, jamais *trustless pour un tiers*.
//!
//! Elle n'est pas non plus **complète** : l'échantillon récent est borné, le décompte total ne
//! l'est pas. Servir l'un pour l'autre ferait croire à une panne bornée alors qu'elle ne l'est pas,
//! et c'est pourquoi la réponse porte les deux grandeurs côte à côte.

use axum::extract::State;
use axum::Json;
use serde_json::json;

use crate::auth::Caller;
use crate::error::GatewayError;
use crate::state::SwarmGatewayState;

/// Le journal des brèches **du tenant appelant**.
///
/// # Errors
/// [`GatewayError::Forbidden`] si l'appelant n'a pas un rôle d'audit ou d'exploitation.
pub async fn gaps(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
) -> Result<Json<serde_json::Value>, GatewayError> {
    // Mêmes rôles que l'export de bundle : une brèche dit ce qui MANQUE à la preuve, ce qui est
    // un renseignement d'exploitation — pas une donnée de lecture ordinaire.
    caller.require_any_role(&["auditor", "operator", "admin"])?;

    let recentes = s.gaps.recentes_du_tenant(&caller.tenant);
    Ok(Json(json!({
        // La posture en vigueur : sous `blocking`, aucune brèche ne PEUT exister, parce qu'un run
        // non persistable fait échouer sa requête. Le dire évite de lire un journal vide comme
        // une bonne nouvelle alors qu'il est vide par construction.
        "policy": s.capture_policy.as_str(),
        // Le décompte TOTAL du processus, tous tenants confondus — il ne se perd pas quand
        // l'échantillon tourne. Volontairement non cloisonné : c'est une grandeur d'exploitation,
        // et elle ne révèle l'activité d'aucun tenant en particulier.
        "total_process": s.gaps.total(),
        // L'échantillon récent, cloisonné par tenant. Borné : son silence ne prouve rien.
        "recent": recentes,
        "recent_cap": crate::capture::JOURNAL_CAP,
        "note": "Journal en mémoire, borné, non scellé : il détecte la PANNE, pas la malveillance. \
                 Un run absent d'ici peut malgré tout avoir été perdu — l'échantillon ne remonte \
                 pas indéfiniment, et il ne survit pas au redémarrage du processus.",
    })))
}
