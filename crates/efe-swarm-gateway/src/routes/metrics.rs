// SPDX-License-Identifier: UNLICENSED
//! `GET /metrics` — exposition Prometheus (provenance + substrat), texte brut, **sans
//! dépendance** (les registres rendent eux-mêmes le format). Agrégé sur **tous** les runs.

use std::sync::atomic::Ordering;

use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;

use crate::state::SwarmGatewayState;

/// **ADR-014 (CP-2) + ADR-018** : surfaces d'admission — runs en vol (jauge dérivée des
/// permits global), plafond process, plafond **par-tenant** (équité ADR-018), tenants
/// actifs, refus cumulés. Rendu inline (même style sans-dépendance que les registres).
/// **M2** — les brèches : des runs SERVIS que l'on n'a pas pu persister.
///
/// Deux séries, et il faut les deux. Le **compteur** ne se perd jamais et dit l'ampleur ; la
/// **jauge** de posture dit si une brèche est seulement POSSIBLE. Sans la seconde, un compteur à
/// zéro se lit comme une bonne nouvelle alors qu'il peut être nul par construction — sous la
/// posture bloquante, aucun run non persistable n'est jamais servi.
fn render_capture(s: &SwarmGatewayState) -> String {
    format!(
        "# HELP efe_swarm_capture_gaps_total Runs servis mais NON persistés (M2) — jamais re-prouvables.\n\
         # TYPE efe_swarm_capture_gaps_total counter\n\
         efe_swarm_capture_gaps_total {}\n\
         # HELP efe_swarm_capture_observation Posture de capture : 1 = observation (une brèche est possible), 0 = bloquante.\n\
         # TYPE efe_swarm_capture_observation gauge\n\
         efe_swarm_capture_observation {}\n",
        s.gaps.total(),
        u8::from(s.capture_policy.peut_degrader()),
    )
}

/// **M5** — le pont vers le registre. **Trois** séries, et il en faut trois.
///
/// La jauge dit si un dossier est seulement POSSIBLE : sans elle, un compteur à zéro se lirait
/// comme « rien à signaler » alors qu'il peut être nul parce que le pont n'existe pas. C'était la
/// leçon de M2.
///
/// Mais deux ne suffisaient pas non plus, et l'audit l'a trouvé : avec le seul compteur de SUCCÈS,
/// « aucun run n'est passé » et « toutes les dérivations échouent » rendent le même zéro. Le
/// compteur d'ÉCHEC est le dénominateur qui les sépare — et il compte quelque chose de
/// **récupérable**, puisque `POST /v1/swarm/reg/:id` re-dérive.
fn render_reg(s: &SwarmGatewayState) -> String {
    format!(
        "# HELP efe_swarm_reg_dossiers_total Dossiers d'évidence réglementaire écrits (M5).\n\
         # TYPE efe_swarm_reg_dossiers_total counter\n\
         efe_swarm_reg_dossiers_total {}\n\
         # HELP efe_swarm_reg_echecs_total Dérivations réglementaires qui ont ÉCHOUÉ (M5) — récupérables par POST /v1/swarm/reg/:id.\n\
         # TYPE efe_swarm_reg_echecs_total counter\n\
         efe_swarm_reg_echecs_total {}\n\
         # HELP efe_swarm_reg_pont Pont vers le registre : 1 = câblé (un dossier est possible), 0 = non.\n\
         # TYPE efe_swarm_reg_pont gauge\n\
         efe_swarm_reg_pont {}\n",
        s.reg_dossiers.load(std::sync::atomic::Ordering::Relaxed),
        s.reg_echecs.load(std::sync::atomic::Ordering::Relaxed),
        u8::from(s.reg_pont_cable()),
    )
}

fn render_admission(s: &SwarmGatewayState) -> String {
    let inflight = s.admission_max.saturating_sub(s.admission.available_permits());
    format!(
        "# HELP efe_swarm_gateway_inflight_runs Runs en vol (admission process, ADR-014).\n\
         # TYPE efe_swarm_gateway_inflight_runs gauge\n\
         efe_swarm_gateway_inflight_runs {}\n\
         # HELP efe_swarm_gateway_inflight_max Permits d'admission configurés.\n\
         # TYPE efe_swarm_gateway_inflight_max gauge\n\
         efe_swarm_gateway_inflight_max {}\n\
         # HELP efe_swarm_gateway_inflight_per_tenant_max Plafond runs en vol par tenant (équité, ADR-018).\n\
         # TYPE efe_swarm_gateway_inflight_per_tenant_max gauge\n\
         efe_swarm_gateway_inflight_per_tenant_max {}\n\
         # HELP efe_swarm_gateway_admission_tenants_active Tenants avec ≥1 run en vol (ADR-018).\n\
         # TYPE efe_swarm_gateway_admission_tenants_active gauge\n\
         efe_swarm_gateway_admission_tenants_active {}\n\
         # HELP efe_swarm_gateway_admission_rejected_total Requêtes refusées 429 (capacité globale OU quota tenant).\n\
         # TYPE efe_swarm_gateway_admission_rejected_total counter\n\
         efe_swarm_gateway_admission_rejected_total {}\n\
         # HELP efe_swarm_gateway_max_width Plafond serveur du champ DTO width (ADR-014).\n\
         # TYPE efe_swarm_gateway_max_width gauge\n\
         efe_swarm_gateway_max_width {}\n\
         # HELP efe_swarm_gateway_memory_recalls_total Runs ayant lu un instantané mémoire (ADR-019).\n\
         # TYPE efe_swarm_gateway_memory_recalls_total counter\n\
         efe_swarm_gateway_memory_recalls_total {}\n\
         # HELP efe_swarm_gateway_memory_learns_total Runs propres ayant replié en mémoire (ADR-019).\n\
         # TYPE efe_swarm_gateway_memory_learns_total counter\n\
         efe_swarm_gateway_memory_learns_total {}\n",
        inflight,
        s.admission_max,
        s.admission_per_tenant_max,
        s.admission_tenants_active(),
        s.admission_rejected.load(Ordering::Relaxed),
        s.max_width,
        s.memory_recalls.load(Ordering::Relaxed),
        s.memory_learns.load(Ordering::Relaxed),
    )
}

/// **Couche 2 (M4 §2.4)** : compteurs d'egress frontière (coût/appels/R4). Gaté `frontier-llm` ;
/// hors feature ⟹ chaîne vide ⟹ sortie `/metrics` **byte-identique** au build par défaut.
#[cfg(feature = "frontier-llm")]
fn render_frontier(s: &SwarmGatewayState) -> String {
    s.frontier_metrics.render_prometheus()
}
#[cfg(not(feature = "frontier-llm"))]
fn render_frontier(_s: &SwarmGatewayState) -> String {
    String::new()
}

pub async fn metrics(State(s): State<SwarmGatewayState>) -> impl IntoResponse {
    let body = format!(
        "{}{}{}{}{}{}{}",
        s.prov_metrics.render_prometheus(),
        s.substrate_metrics.render_prometheus(),
        // Inc.3 : surfaces du dashboard (events émis/perdus + jauges live).
        s.dash_metrics.render_prometheus(s.dashboards.len(), s.dashboards.total_subscribers()),
        // ADR-014 : admission process.
        render_admission(&s),
        // M2 : brèches de capture + posture en vigueur.
        render_capture(&s),
        // M5 : dossiers réglementaires + état du pont.
        render_reg(&s),
        // Couche 2 (M4 §2.4) : egress frontière (vide hors feature).
        render_frontier(&s),
    );
    ([(CONTENT_TYPE, "text/plain; version=0.0.4")], body)
}
