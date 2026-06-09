// SPDX-License-Identifier: UNLICENSED
//! `GET /metrics` — exposition Prometheus (provenance + substrat), texte brut, **sans
//! dépendance** (les registres rendent eux-mêmes le format). Agrégé sur **tous** les runs.

use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;

use crate::state::SwarmGatewayState;

pub async fn metrics(State(s): State<SwarmGatewayState>) -> impl IntoResponse {
    let body = format!(
        "{}{}{}",
        s.prov_metrics.render_prometheus(),
        s.substrate_metrics.render_prometheus(),
        // Inc.3 : surfaces du dashboard (events émis/perdus + jauges live).
        s.dash_metrics.render_prometheus(s.dashboards.len(), s.dashboards.total_subscribers()),
    );
    ([(CONTENT_TYPE, "text/plain; version=0.0.4")], body)
}
