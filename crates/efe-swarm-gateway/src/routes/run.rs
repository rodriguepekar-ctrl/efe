// SPDX-License-Identifier: UNLICENSED
//! `POST /v1/swarm/run` — exécute un essaim.
//!
//! - **Sync** (défaut, Inc.2) : exécute, persiste les records, renvoie le `SwarmRun`.
//! - **Job live** (`?stream=true`, Inc.3) : crée un `DashboardBus`, lance le run **observé**
//!   en tâche détachée, renvoie **immédiatement** `202 {run_id, status, events}`. Le flux SSE
//!   `…/runs/:id/events` rejoue puis streame en direct.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use efe_swarm_dashboard::{DashboardBus, DashboardEvent};
use serde_json::json;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::auth::Caller;
use crate::dto::{JobAccepted, RunParams, RunRequest, RunResponse};
use crate::error::{GatewayError, GatewayJson};
use crate::state::SwarmGatewayState;

/// Taille du ring de rejeu d'un run (events conservés pour les abonnés tardifs).
const RING_CAP: usize = 8192;
/// Cadence du ticker de jauges (snapshots de métriques → `metrics_tick`).
const TICK_MS: u64 = 250;

pub async fn run(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Query(params): Query<RunParams>,
    // GatewayJson (et non Json nu) : un corps malformé sort en 400 + enveloppe JSON
    // stable, jamais en 422 text/plain opaque (revue ADR-014).
    GatewayJson(req): GatewayJson<RunRequest>,
) -> Result<Response, GatewayError> {
    caller.require_any_role(&["operator", "admin"])?;
    // ── ADR-014 (CP-2) : ADMISSION PROCESS — avant toute résolution coûteuse. ────
    // Borne les runs en vol (sync ET jobs). Refus = 429 + Retry-After : du
    // backpressure VISIBLE, plutôt qu'une file invisible qui finit en timeouts.
    // Le permit (FIFO) suit le run jusqu'à sa fin (déplacé dans la tâche du job).
    let permit = match s.admission.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            s.admission_rejected.fetch_add(1, Ordering::Relaxed);
            return Ok((
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, "1")],
                // le code machine est CELUI de l'enveloppe stable (= l'autre 429 de
                // l'API, quota SSE) — un client switche sur une seule valeur ; les
                // champs additifs (inflight_max) précisent la cause.
                Json(json!({
                    "error": "too_many_requests",
                    "message": "passerelle à capacité (admission ADR-014) : réessayez (Retry-After)",
                    "inflight_max": s.admission_max,
                })),
            )
                .into_response());
        }
    };
    // ADR-014 (CP-2) : largeur demandée (plafonnée serveur dans `swarm_for`) — I-CP1.
    let width = req.width;
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
        let swarm = s.swarm_for_observed(&caller.tenant, mode, observer, allow_remote, width)?;

        let state = s.clone();
        let tenant = caller.tenant.clone();
        tokio::spawn(async move {
            // l'admission suit le run : le permit vit jusqu'à la fin du job (ADR-014).
            let _permit = permit;
            let ticker = spawn_metrics_ticker(state.clone(), bus.clone());
            match swarm.run_traced(task, spec, budget).await {
                Ok(mut outcome) => {
                    // G9 : caviarde la sortie au repos selon la politique du tenant (records intacts).
                    state.redact_output(&tenant, &mut outcome.run);
                    // persiste la PREUVE (records) — re-vérifiable via verify/:id. Un échec
                    // ici rendrait le run invérifiable : il se SIGNALE sur le bus (revue
                    // ADR-014 — jamais d'échec de persistance avalé, le moat n'est pas
                    // négociable), même si `run_finished` est déjà parti.
                    if let Err(e) = state.store.put(run_id, &tenant, &outcome).await {
                        bus.emit(DashboardEvent::RunFailed {
                            message: format!("persistance des records échouée (run invérifiable) : {e}"),
                        });
                    }
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

    // ── Mode SYNC (Inc.2) — le permit d'admission reste tenu jusqu'au retour. ──
    let _permit = permit;
    let swarm = s.swarm_for(&caller.tenant, mode, allow_remote, width)?;
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
