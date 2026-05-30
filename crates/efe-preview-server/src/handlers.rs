// SPDX-License-Identifier: UNLICENSED
//! Handlers HTTP du bridge.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json, Response};
use futures::stream::{self, Stream};
use serde_json::json;

use efe_coordinator::MissionStream;

use crate::dto::{MissionADto, MissionBDto, SpecDto};
use crate::{AppState, SchemaResponse};

/// `GET /healthz` — liveness (non authentifié).
pub async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok\n")
}

/// `GET /v1/schema` — contrat d'événements (garde-fou non-dérive UI).
pub async fn schema() -> Json<SchemaResponse> {
    Json(SchemaResponse::current())
}

/// `POST /v1/missions/a` — lance une Mission A et **streame** ses `StreamEvent`
/// en `text/event-stream`.
///
/// Erreurs de validation (DTO) → `400`. JSON malformé → `400` (axum).
pub async fn post_mission_a(
    State(state): State<AppState>,
    Json(dto): Json<MissionADto>,
) -> Response {
    let req = match dto.into_request(&state.config.default_llms) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    // P4.S1 : abonnement sans course (s'abonne avant de spawn la mission).
    let mission = state.coord.run_mission_a_streamed(req);
    mission_sse(mission).into_response()
}

/// `POST /v1/missions/b` — Mission B (fan-out N LLMs + vote) live SSE.
pub async fn post_mission_b(
    State(state): State<AppState>,
    Json(dto): Json<MissionBDto>,
) -> Response {
    let req = match dto.into_request(&state.config.default_llms) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let mission = state.coord.run_mission_b_streamed(req);
    mission_sse(mission).into_response()
}

/// `POST /v1/missions/speculative` — Mission Speculative (drafter+verifier) live SSE.
pub async fn post_mission_speculative(
    State(state): State<AppState>,
    Json(dto): Json<SpecDto>,
) -> Response {
    let req = match dto.into_request() {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let mission = state.coord.run_speculative_streamed(req);
    mission_sse(mission).into_response()
}

/// `GET /v1/savings-report` — sert le livrable `CHEWING_SAVINGS_REPORT_v2.json`
/// (R3.S9). `404` si le fichier n'a pas encore été généré.
pub async fn savings_report(State(state): State<AppState>) -> Response {
    let path = &state.config.savings_report_path;
    match tokio::fs::read(path).await {
        Ok(bytes) => ([(CONTENT_TYPE, "application/json")], bytes).into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "savings report introuvable",
                "hint": "générer via le live test R3.S9 (live_chewing_savings_report_v2)",
                "path": path.display().to_string(),
            })),
        )
            .into_response(),
    }
}

/// `GET /metrics` — réexpose la télémétrie Prometheus (R3.S7).
///
/// État des circuits SLM lu au scrape (`snapshot_circuits`) + compteurs/histos
/// du `MetricsRegistry`. `# telemetry disabled` si la télémétrie n'est pas
/// câblée (`coordinator.metrics == None`).
pub async fn metrics(State(state): State<AppState>) -> Response {
    let body = match &state.coord.metrics {
        Some(reg) => {
            let circuits =
                efe_coordinator::metrics_server::snapshot_circuits(&state.coord.clients);
            reg.render_prometheus(&circuits)
        }
        None => "# telemetry disabled (coordinator.metrics = None)\n".to_string(),
    };
    ([(CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response()
}

/// Transforme un [`MissionStream`] en réponse SSE.
///
/// **Point critique 1 — propriété (RAII)** : le stream **possède** le
/// `MissionStream` entier (via `unfold`). Sa garde RAII reste donc vivante tant
/// que le stream est consommé ; à la fin ou à la déconnexion (axum drop le
/// body), le `MissionStream` est droppé → `unsubscribe`. Si l'on déplaçait
/// seulement `events`, la garde libèrerait le subscriber trop tôt.
///
/// **Point critique 2 — terminaison** : `InMemoryBus` ne ferme PAS le canal du
/// subscriber quand la mission finit (le `Sender` vit jusqu'au `unsubscribe`).
/// Un `recv()` naïf bloquerait donc **à l'infini** après le dernier event. On
/// termine donc le stream sur (a) l'event terminal `SWARM_FINISHED`/`SWARM_FAILED`
/// (chemin normal) OU (b) la complétion du `JoinHandle` de la mission (filet de
/// sécurité pour les chemins d'erreur sans event terminal). `biased` privilégie
/// la livraison des events bufferisés avant de réagir au join.
pub(crate) fn mission_sse<R>(
    mission: MissionStream<R>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
where
    R: Send + 'static,
{
    let body = stream::unfold(Some(mission), |state| async move {
        let mut ms = state?; // `None` ⇒ stream terminé (itération précédente)
        tokio::select! {
            biased;
            maybe_ev = ms.events.recv() => match maybe_ev {
                Some(ev) => {
                    let terminal = matches!(ev.tag(), "SWARM_FINISHED" | "SWARM_FAILED");
                    let json = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".to_string());
                    // Frame `data: <json>\n\n` (le `type` discriminant est dans le JSON).
                    let next = if terminal { None } else { Some(ms) };
                    Some((Ok::<Event, Infallible>(Event::default().data(json)), next))
                }
                None => None, // canal fermé (subscriber retiré)
            },
            // Filet de sécurité : la mission a fini sans event terminal capté.
            _ = &mut ms.join => None,
        }
    });
    Sse::new(body).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// Middleware bearer (no-op si `config.token` est `None`).
pub async fn auth_mw(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if let Some(expected) = &state.config.token {
        let provided = req
            .headers()
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if provided != Some(expected.as_str()) {
            return (StatusCode::UNAUTHORIZED, "bearer manquant ou invalide").into_response();
        }
    }
    next.run(req).await
}
