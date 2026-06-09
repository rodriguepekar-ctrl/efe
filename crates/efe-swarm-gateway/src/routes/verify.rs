// SPDX-License-Identifier: UNLICENSED
//! `GET /v1/swarm/verify/:id` — **re-prouver (le moat)**.
//!
//! `verify_swarm` **recalcule** chaîne + contenu + DAG depuis les records + la clé tenant.
//! Même si le store a été altéré après coup, la chaîne casse → `verified=false`,
//! `first_break` localise la faute. L'endpoint ne demande pas de *croire* le store ; il
//! *re-prouve*.

use axum::extract::{Path, State};
use axum::Json;
use efe_swarm_provenance::verify_swarm;
use uuid::Uuid;

use crate::auth::Caller;
use crate::dto::VerifyResponse;
use crate::error::GatewayError;
use crate::state::SwarmGatewayState;

pub async fn verify(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Path(id): Path<Uuid>,
) -> Result<Json<VerifyResponse>, GatewayError> {
    caller.require_any_role(&["viewer", "operator", "admin", "auditor"])?;

    let stored = s.store.get(id, &caller.tenant).await?.ok_or(GatewayError::NotFound)?;
    let key = s.derive_key(&caller.tenant)?; // HKDF par tenant (server-side)

    // ← le vérificateur INDÉPENDANT : ne consulte aucun provider, recalcule tout.
    let report = verify_swarm(&stored.records, &key);
    // Comptabilise la re-preuve dans les métriques partagées (ruptures détectées).
    s.prov_metrics.observe_verify(&report);

    Ok(Json(VerifyResponse {
        run_id: id,
        verified: report.ok(),
        root: report.root_hex(),
        chain_intact: report.chain_intact,
        content_intact: report.content_intact,
        dag_consistent: report.dag_consistent,
        traced_pct: report.traced_pct,
        records: report.records,
        first_break: report.first_break,
    }))
}
