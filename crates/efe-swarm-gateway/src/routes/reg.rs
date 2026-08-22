// SPDX-License-Identifier: UNLICENSED
//! `POST /v1/swarm/reg/:id` — **M5** : re-dériver le dossier réglementaire d'un run.
//!
//! ## Pourquoi cette route existe, et pourquoi elle est indispensable
//!
//! `reg_pont::deriver` ne peut pas échouer : il rend un rapport, jamais une erreur. Cette
//! non-fatalité n'est défendable **qu'à une condition** — qu'un dossier manqué soit rattrapable.
//! Sans cette route, « le dossier n'a pas été écrit » deviendrait « le dossier n'existera jamais »,
//! et la décision de ne pas couper la requête du client se transformerait en perte silencieuse.
//!
//! C'est donc la contrepartie exacte de la dérivation automatique, pas une commodité.
//!
//! ## Elle re-dérive, elle ne re-fabrique pas
//!
//! Le run est relu depuis le store, ses nœuds sont **re-prouvés** (`reg_evidence::derive` refuse de
//! dériver sur du non-prouvé), et l'artefact est recalculé à l'identique. Rien n'est reconstitué de
//! mémoire : si la chaîne ne tient plus, la re-dérivation échoue — et c'est le comportement voulu.
//!
//! ## Elle passe par le chargeur honnête de M2
//!
//! Un run servi mais non persisté rend `410 Gone` et non `404` : « ce run a eu lieu et n'a pas pu
//! être persisté » est un fait, pas une absence. Le dire ici évite qu'un agent de conformité
//! conclue à l'inexistence d'un run qu'il a vu passer.

use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::auth::Caller;
use crate::error::GatewayError;
use crate::reg_pont::RegReport;
use crate::state::SwarmGatewayState;

/// Re-dérive le dossier d'un run persisté.
///
/// # Errors
/// [`GatewayError::Forbidden`] si le rôle ne convient pas · `410`/`404` selon que le run a été
/// servi sans être persisté ou qu'il est inconnu · [`GatewayError::NotImplemented`] si le pont
/// n'est pas câblé — refuser en le disant vaut mieux que rendre un rapport d'échec qui laisserait
/// croire à une panne alors qu'il s'agit d'une configuration absente.
pub async fn rederiver(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Path(id): Path<Uuid>,
) -> Result<Json<RegReport>, GatewayError> {
    // Mêmes rôles que l'export de bundle : produire une pièce réglementaire est un acte d'audit.
    caller.require_any_role(&["auditor", "operator", "admin"])?;

    // M2 : distingue « inconnu » (404) de « servi mais non persisté » (410). Un agent de
    // conformité ne doit jamais conclure à l'inexistence d'un run qu'il a vu passer.
    let stored = s.charger_run(id, &caller.tenant).await?;

    match s.reg_dossier(id, &caller.tenant, &stored) {
        Some(r) => Ok(Json(r)),
        None => Err(GatewayError::NotImplemented(
            "le pont réglementaire n'est pas câblé (EFE_SWARM_REG_EVIDENCE_DIR + \
             EFE_SWARM_REG_FRAMEWORK) — le rapport de démarrage le dit aussi"
                .into(),
        )),
    }
}
