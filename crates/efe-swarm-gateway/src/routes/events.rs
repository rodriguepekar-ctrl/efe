// SPDX-License-Identifier: UNLICENSED
//! Routes **SSE** — le dashboard live.
//!
//! - `GET /v1/swarm/runs/:id/events` : **rejeu + live** d'un run (tenant-scopé).
//! - `GET /v1/swarm/events` : firehose **tenant** (runs actifs à la connexion).
//!
//! Pattern axum 0.7 : `async_stream::stream!` + `Sse::new(s).keep_alive(...)`.

use std::convert::Infallible;

use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use efe_swarm_dashboard::DashboardEnvelope;
use futures::Stream;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::auth::Caller;
use crate::error::GatewayError;
use crate::state::SwarmGatewayState;

/// Plafond d'abonnés SSE simultanés **par tenant** (parade anti-DoS, §10).
const MAX_SSE_SUBSCRIBERS_PER_TENANT: usize = 64;

/// Refuse la connexion si le tenant a déjà saturé son quota d'abonnés SSE.
fn guard_subscriber_quota(s: &SwarmGatewayState, tid: &efe_nano_sdk::TenantId) -> Result<(), GatewayError> {
    if s.dashboards.subscribers_for(tid) >= MAX_SSE_SUBSCRIBERS_PER_TENANT {
        return Err(GatewayError::TooManyRequests(format!(
            "quota d'abonnés SSE atteint pour ce tenant (max {MAX_SSE_SUBSCRIBERS_PER_TENANT})"
        )));
    }
    Ok(())
}

/// Une enveloppe → une frame SSE (`event: <kind>` + `data: <json>`).
fn frame(env: &DashboardEnvelope) -> Event {
    Event::default()
        .event(env.event.kind())
        .json_data(env)
        .unwrap_or_else(|_| Event::default().comment("serialize error"))
}

/// `GET /v1/swarm/runs/:id/events` — rejeu (ring) puis live, jusqu'au terminal.
pub async fn run_events(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Path(id): Path<Uuid>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, GatewayError> {
    caller.require_any_role(&["viewer", "operator", "admin", "auditor"])?;
    let tid = s.tenant_id(&caller.tenant)?;
    guard_subscriber_quota(&s, &tid)?;
    let bus = s.dashboards.get(id, &tid).ok_or(GatewayError::NotFound)?;

    // S'abonner AVANT de prendre le snapshot du ring → aucun trou ; on dédoublonne par `seq`.
    let mut live = bus.subscribe();
    let replay = bus.replay();
    let dash_metrics = s.dash_metrics.clone();

    let stream = async_stream::stream! {
        let mut last_seq: Option<u64> = None;
        let mut done = false;
        for env in replay {
            last_seq = Some(env.seq);
            let terminal = env.is_terminal();
            yield Ok(frame(&env));
            if terminal { done = true; break; }
        }
        if !done {
            loop {
                match live.recv().await {
                    Ok(env) => {
                        if last_seq.map(|s| env.seq <= s).unwrap_or(false) {
                            continue; // déjà rejoué (chevauchement ring/live)
                        }
                        let terminal = env.is_terminal();
                        yield Ok(frame(&env));
                        if terminal { break; }
                    }
                    Err(RecvError::Lagged(n)) => { dash_metrics.add_dropped(n); continue; } // best-effort
                    Err(RecvError::Closed) => break,
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// `GET /v1/swarm/events` — firehose tenant : fusionne les bus **actifs à la connexion**
/// (les runs lancés ensuite n'y figurent pas — cf. `runs/:id/events` pour un run précis).
pub async fn tenant_events(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, GatewayError> {
    caller.require_any_role(&["viewer", "operator", "admin", "auditor"])?;
    let tid = s.tenant_id(&caller.tenant)?;
    guard_subscriber_quota(&s, &tid)?;
    let buses = s.dashboards.active_for(&tid);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DashboardEnvelope>(2048);
    for bus in buses {
        let tx = tx.clone();
        let replay = bus.replay();
        let mut live = bus.subscribe();
        let dash_metrics = s.dash_metrics.clone();
        tokio::spawn(async move {
            let mut last_seq: Option<u64> = None;
            for env in replay {
                last_seq = Some(env.seq);
                if tx.send(env).await.is_err() {
                    return;
                }
            }
            loop {
                match live.recv().await {
                    Ok(env) => {
                        if last_seq.map(|s| env.seq <= s).unwrap_or(false) {
                            continue;
                        }
                        if tx.send(env).await.is_err() {
                            return;
                        }
                    }
                    Err(RecvError::Lagged(n)) => { dash_metrics.add_dropped(n); continue; }
                    Err(RecvError::Closed) => return,
                }
            }
        });
    }
    drop(tx); // le flux se ferme quand tous les forwarders se terminent.

    let stream = async_stream::stream! {
        while let Some(env) = rx.recv().await {
            yield Ok(frame(&env));
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
