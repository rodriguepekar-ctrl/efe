// SPDX-License-Identifier: UNLICENSED
//! `POST /v1/swarm/run` — exécute un essaim (le SDK), **persiste ses records**, renvoie le
//! `SwarmRun`. Le `Swarm` est construit avec la clé HKDF **du tenant** (server-side).

use axum::extract::State;
use axum::Json;
use uuid::Uuid;

use crate::auth::Caller;
use crate::dto::{RunRequest, RunResponse};
use crate::error::GatewayError;
use crate::state::SwarmGatewayState;

pub async fn run(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Json(req): Json<RunRequest>,
) -> Result<Json<RunResponse>, GatewayError> {
    caller.require_any_role(&["operator", "admin"])?;

    let (task, spec, budget, mode) = req.into_parts()?;
    let swarm = s.swarm_for(&caller.tenant, mode)?;

    // ← SDK : run + SealedProvenance (les records bruts, la preuve).
    let outcome = swarm.run_traced(task, spec, budget).await?;

    // Persiste l'ÉVIDENCE (records) sous un id, tenant-scopé.
    let run_id = Uuid::new_v4();
    s.store.put(run_id, &caller.tenant, &outcome).await?;

    Ok(Json(RunResponse { run_id, run: outcome.run }))
}
