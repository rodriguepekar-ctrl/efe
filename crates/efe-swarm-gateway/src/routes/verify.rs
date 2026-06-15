// SPDX-License-Identifier: UNLICENSED
//! `GET /v1/swarm/verify/:id` — **re-prouver (le moat)**.
//!
//! `verify_run_offline` **recalcule** la chaîne d'essaim (contenu + DAG), la chaîne
//! forensique N17, ET le pont Mode A ↔ Mode C, depuis les seuls records persistés + la
//! clé tenant. Même si le store a été altéré après coup, une chaîne casse → `verified=false`
//! et la faute est localisée. L'endpoint ne demande pas de *croire* le store ; il
//! *re-prouve* — la forensique LLM comprise (ADR-016).

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

    let stored = s.store.get(id, &caller.tenant).await?.ok_or(GatewayError::NotFound)?;
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

    Ok(Json(VerifyResponse {
        run_id: id,
        // ADR-016 : le verdict GLOBAL inclut désormais la forensique N17 + le pont.
        verified: v.fully_proven,
        root: v.swarm.root_hex(),
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
        },
    }))
}
