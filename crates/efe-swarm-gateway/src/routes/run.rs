// SPDX-License-Identifier: UNLICENSED
//! `POST /v1/swarm/run` — exécute un essaim.
//!
//! - **Sync** (défaut, Inc.2) : exécute, persiste les records, renvoie le `SwarmRun`.
//! - **Job live** (`?stream=true`, Inc.3) : crée un `DashboardBus`, lance le run **observé**
//!   en tâche détachée, renvoie **immédiatement** `202 {run_id, status, events}`. Le flux SSE
//!   `…/runs/:id/events` rejoue puis streame en direct.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use efe_swarm_dashboard::{DashboardBus, DashboardEvent};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::auth::Caller;
use crate::dto::{JobAccepted, RunParams, RunRequest, RunResponse};
use crate::error::GatewayError;
use crate::state::SwarmGatewayState;

/// Taille du ring de rejeu d'un run (events conservés pour les abonnés tardifs).
const RING_CAP: usize = 8192;
/// Cadence du ticker de jauges (snapshots de métriques → `metrics_tick`).
const TICK_MS: u64 = 250;

pub async fn run(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Query(params): Query<RunParams>,
    Json(req): Json<RunRequest>,
) -> Result<Response, GatewayError> {
    caller.require_any_role(&["operator", "admin"])?;
    let (task, spec, mut budget, mode) = req.into_parts()?;
    // Inc.4 : résout le mode (Network → routeur server-side, sinon souverain). 501 si Network
    // demandé sans routeur câblé. `allow_remote` n'est vrai que pour le routeur Network.
    let (mode, allow_remote, price_override) = s.resolve_mode(mode)?;
    // Inc.4.b : préserve le tarif Network à travers la traduction → SelfHost (sinon la
    // gouvernance de budget facturerait 0¢/appel). On respecte un tarif explicite du client.
    if let Some(price) = price_override {
        budget.price_per_call_cents.get_or_insert(price);
    }

    if params.stream {
        // ── Mode JOB live (Inc.3) ────────────────────────────────────────────
        let run_id = Uuid::new_v4();
        let tid = s.tenant_id(&caller.tenant)?;
        let bus = DashboardBus::new_metered(tid, run_id, RING_CAP, Some(s.dash_metrics.clone()));
        s.dashboards.insert(bus.clone());

        // G9 : observateur avec redactor PII des previews si le tenant l'active.
        let observer = s.dashboard_observer(&caller.tenant, bus.clone());
        let swarm = s.swarm_for_observed(&caller.tenant, mode, observer, allow_remote)?;

        let state = s.clone();
        let tenant = caller.tenant.clone();
        tokio::spawn(async move {
            let ticker = spawn_metrics_ticker(state.clone(), bus.clone());
            match swarm.run_traced(task, spec, budget).await {
                Ok(mut outcome) => {
                    // G9 : caviarde la sortie au repos selon la politique du tenant (records intacts).
                    state.redact_output(&tenant, &mut outcome.run);
                    // persiste la PREUVE (records) — re-vérifiable via verify/:id.
                    let _ = state.store.put(run_id, &tenant, &outcome).await;
                    // (l'observateur a déjà émis `run_finished`.)
                }
                Err(e) => {
                    bus.emit(DashboardEvent::RunFailed { message: e.to_string() });
                }
            }
            ticker.abort();
        });

        let events = format!("/v1/swarm/runs/{run_id}/events");
        return Ok((StatusCode::ACCEPTED, Json(JobAccepted { run_id, status: "running", events })).into_response());
    }

    // ── Mode SYNC (Inc.2, inchangé) ──────────────────────────────────────────
    let swarm = s.swarm_for(&caller.tenant, mode, allow_remote)?;
    let mut outcome = swarm.run_traced(task, spec, budget).await?;
    // G9 : caviarde la sortie selon la politique PII du tenant (réponse + au repos).
    s.redact_output(&caller.tenant, &mut outcome.run);
    let run_id = Uuid::new_v4();
    s.store.put(run_id, &caller.tenant, &outcome).await?;
    Ok((StatusCode::OK, Json(RunResponse { run_id, run: outcome.run })).into_response())
}

/// Émet périodiquement un `metrics_tick` (jauges globales) jusqu'à l'`abort`. Hors chemin
/// chaud du run.
fn spawn_metrics_ticker(state: SwarmGatewayState, bus: Arc<DashboardBus>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(TICK_MS));
        loop {
            iv.tick().await;
            let p = state.prov_metrics.snapshot();
            let sm = state.substrate_metrics.snapshot();
            bus.emit(DashboardEvent::MetricsTick {
                bb_writes: sm.bb_writes_total,
                events_published: sm.ev_published_total,
                traced_ratio: p.traced_ratio,
                chain_breaks: p.chain_breaks_total,
            });
        }
    })
}
