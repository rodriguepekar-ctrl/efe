// SPDX-License-Identifier: UNLICENSED
//! `GET /metrics` — exposition Prometheus (provenance + substrat), texte brut, **sans
//! dépendance** (les registres rendent eux-mêmes le format). Agrégé sur **tous** les runs.

use std::sync::atomic::Ordering;

use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;

use crate::state::SwarmGatewayState;

/// **ADR-014 (CP-2)** : surfaces d'admission process — runs en vol (jauge dérivée des
/// permits), plafond configuré, refus cumulés. Rendu inline (même style sans-dépendance
/// que les registres).
fn render_admission(s: &SwarmGatewayState) -> String {
    let inflight = s.admission_max.saturating_sub(s.admission.available_permits());
    format!(
        "# HELP efe_swarm_gateway_inflight_runs Runs en vol (admission process, ADR-014).\n\
         # TYPE efe_swarm_gateway_inflight_runs gauge\n\
         efe_swarm_gateway_inflight_runs {}\n\
         # HELP efe_swarm_gateway_inflight_max Permits d'admission configurés.\n\
         # TYPE efe_swarm_gateway_inflight_max gauge\n\
         efe_swarm_gateway_inflight_max {}\n\
         # HELP efe_swarm_gateway_admission_rejected_total Requêtes refusées 429 (capacité).\n\
         # TYPE efe_swarm_gateway_admission_rejected_total counter\n\
         efe_swarm_gateway_admission_rejected_total {}\n\
         # HELP efe_swarm_gateway_max_width Plafond serveur du champ DTO width (ADR-014).\n\
         # TYPE efe_swarm_gateway_max_width gauge\n\
         efe_swarm_gateway_max_width {}\n",
        inflight,
        s.admission_max,
        s.admission_rejected.load(Ordering::Relaxed),
        s.max_width,
    )
}

pub async fn metrics(State(s): State<SwarmGatewayState>) -> impl IntoResponse {
    let body = format!(
        "{}{}{}{}",
        s.prov_metrics.render_prometheus(),
        s.substrate_metrics.render_prometheus(),
        // Inc.3 : surfaces du dashboard (events émis/perdus + jauges live).
        s.dash_metrics.render_prometheus(s.dashboards.len(), s.dashboards.total_subscribers()),
        // ADR-014 : admission process.
        render_admission(&s),
    );
    ([(CONTENT_TYPE, "text/plain; version=0.0.4")], body)
}
