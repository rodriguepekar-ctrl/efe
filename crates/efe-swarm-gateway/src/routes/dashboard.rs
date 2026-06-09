// SPDX-License-Identifier: UNLICENSED
//! `GET /dashboard` — l'**UI vanilla** (HTML/CSS/JS embarqué), 4 panneaux. Aucune dépendance
//! externe : `EventSource` natif, tout `include_str!` (offline, sans CDN).

use axum::response::Html;

/// L'UI statique du dashboard live (embarquée dans le binaire).
pub async fn dashboard() -> Html<&'static str> {
    Html(include_str!("../../assets/dashboard.html"))
}
