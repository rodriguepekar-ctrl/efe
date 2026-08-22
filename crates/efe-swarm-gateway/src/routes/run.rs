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
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use efe_swarm_dashboard::{DashboardBus, DashboardEvent};
use serde_json::json;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::auth::Caller;
use efe_swarm_sdk::MemoryHandle;

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
    // ── ADR-014 (CP-2) + ADR-018 : ADMISSION ÉQUITABLE — avant toute résolution coûteuse.
    // Borne les runs en vol sous DEUX plafonds : process global (ADR-014) ET par-tenant
    // (ADR-018, équité — un tenant bavard n'affame plus les autres). Refus = 429 +
    // Retry-After : backpressure VISIBLE, jamais une file invisible qui finit en timeouts.
    // Le permit composite (FIFO côté global, RAII côté tenant) suit le run jusqu'à sa fin
    // (déplacé dans la tâche du job, libéré au retour en sync).
    let permit = match s.admit(&caller.tenant) {
        Ok(p) => p,
        Err(reject) => {
            // admission_rejected est déjà incrémenté dans `admit()` (compteur unifié).
            return Ok((
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, "1")],
                // Code d'enveloppe stable (429, comme l'autre limite SSE) — le client
                // switche sur une seule valeur ; `reason` distingue la CAUSE (gateway_full
                // ⇒ réessayez ; tenant_quota ⇒ ralentissez VOTRE débit) sans casser le contrat.
                Json(json!({
                    "error": "too_many_requests",
                    "message": reject.message(),
                    "reason": reject.reason(),
                    "inflight_max": s.admission_max,
                    "inflight_per_tenant_max": s.admission_per_tenant_max,
                })),
            )
                .into_response());
        }
    };
    // ADR-014 (CP-2) : largeur demandée (plafonnée serveur dans `swarm_for`) — I-CP1.
    let width = req.width;
    // ADR-018 : fenêtre temporelle demandée, CLAMPÉE côté serveur ([50 ms, cap env]). Lue
    // avant `into_parts` (qui consomme la requête) ; posée sur le `Swarm` via with_deadline.
    let deadline = req.resolved_deadline();
    // ADR-019 (MEM-3) : poignée mémoire (opt-in). Construite depuis le store tenant-scopé de
    // l'état + la politique du DTO ; posée sur l'essaim via with_memory. `None` ⇒ mémoire OFF.
    let memory_handle = req
        .memory
        .as_ref()
        .map(|m| MemoryHandle::with_config(s.memory.clone(), m.to_config()));
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
        // ADR-018 : la fenêtre temporelle est posée sur l'essaim dérivé (with_deadline) —
        // sonde coopérative + backstop dur s'arment dans run_traced. None ⇒ run non borné.
        let swarm = s
            .swarm_for_observed(&caller.tenant, mode, observer, allow_remote, width)?
            .with_deadline(deadline)
            .with_memory(memory_handle.clone()); // ADR-019 : recall/learn si demandé

        let state = s.clone();
        let tenant = caller.tenant.clone();
        tokio::spawn(async move {
            // l'admission suit le run : le permit vit jusqu'à la fin du job (ADR-014).
            let _permit = permit;
            let ticker = spawn_metrics_ticker(state.clone(), bus.clone());
            match swarm.run_traced(task, spec, budget).await {
                Ok(mut outcome) => {
                    record_memory_metrics(&state, &outcome.run); // ADR-019 : compteurs recall/learn
                    // G9 : caviarde la sortie au repos selon la politique du tenant (records intacts).
                    state.redact_output(&tenant, &mut outcome.run);
                    // persiste la PREUVE (records) — re-vérifiable via verify/:id.
                    //
                    // **M2 — deux corrections, toutes deux vérifiées dans le code.**
                    //
                    // (1) L'émission d'un `RunFailed` MENTAIT : le run a réussi, c'est la
                    //     persistance qui a échoué. Un client qui lit `run_failed` conclut que
                    //     son travail est perdu alors que sa réponse était juste.
                    //
                    // (2) Personne ne l'entendait. `events.rs` ferme le flux SSE au PREMIER
                    //     événement terminal (`is_terminal`), et `RunFinished` en est un — or il
                    //     part AVANT la persistance, comme le notait le commentaire d'origine.
                    //     La promesse « jamais d'échec de persistance avalé » décrivait donc
                    //     exactement ce qui se produisait : l'échec était avalé par la fermeture
                    //     du flux. Le rejeu depuis le ring ne rattrape rien — il s'arrête, lui
                    //     aussi, au premier terminal.
                    //
                    // Les canaux qui atteignent réellement quelqu'un sont donc : le journal des
                    // brèches (`GET /v1/swarm/gaps`), `/metrics`, le log d'exploitation, et
                    // `GET /verify/:id` qui distingue désormais « inconnu » de « non persisté ».
                    //
                    // Ici la politique ne s'applique pas : il n'y a plus de requête à faire
                    // échouer, le 202 est parti depuis longtemps. La seule posture honnête est
                    // l'observation. Le journal enregistre des FAITS ; la politique décide
                    // seulement si une requête tombe.
                    let persistance = state.store.put(run_id, &tenant, &outcome).await;
                    if let Err(e) = &persistance {
                        eprintln!(
                            "⚠ run {run_id} (mode job) SERVI mais NON PERSISTÉ — jamais \
                             re-prouvable. Raison : {e}"
                        );
                    }
                    let _ = crate::capture::decider(
                        crate::capture::CapturePolicy::Observation,
                        run_id,
                        &tenant,
                        persistance,
                        &state.gaps,
                    );
                    // **M5** — le même geste qu'en synchrone. Ici il n'y a pas de réponse à
                    // porter (le 202 est parti depuis longtemps), donc l'échec se dit au
                    // journal d'exploitation ; le dossier reste re-dérivable par la route.
                    // Même raccourci qu'en synchrone : pas de clone quand le pont n'est pas câblé.
                    if let Some(r) = state
                        .reg_pont_cable()
                        .then(|| crate::store::StoredRun::from_outcome(&tenant, &outcome))
                        .and_then(|st| state.reg_dossier(run_id, &tenant, &st))
                    {
                        if !r.produced {
                            eprintln!(
                                "⚠ run {run_id} (mode job) : dossier réglementaire NON produit — {}",
                                r.degrade_reason.as_deref().unwrap_or("raison inconnue")
                            );
                        }
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
    // ADR-018 + ADR-019 : fenêtre temporelle + mémoire posées sur l'essaim (cf. mode JOB).
    let swarm = s
        .swarm_for(&caller.tenant, mode, allow_remote, width)?
        .with_deadline(deadline)
        .with_memory(memory_handle);
    let mut outcome = swarm.run_traced(task, spec, budget).await?;
    record_memory_metrics(&s, &outcome.run); // ADR-019 : compteurs recall/learn
    // G9 : caviarde la sortie selon la politique PII du tenant (réponse + au repos).
    s.redact_output(&caller.tenant, &mut outcome.run);
    let run_id = Uuid::new_v4();
    // ── M2 : la persistance ne peut plus renverser la réponse ─────────────────────────────
    // C'était un `?` : le run avait réussi, la réponse était là dans `outcome.run`, et l'échec
    // de PERSISTANCE la jetait. `decider` est le point de décision UNIQUE — sous la politique
    // par défaut il propage l'erreur exactement comme avant, sous la posture d'observation il
    // rend la réponse en disant la brèche.
    let persistance = s.store.put(run_id, &caller.tenant, &outcome).await;
    let capture = crate::capture::decider(s.capture_policy, run_id, &caller.tenant, persistance, &s.gaps)?;
    if !capture.persisted {
        // L'exploitant doit le voir dans ses logs même s'il ne lit ni `/metrics` ni `/gaps`.
        eprintln!(
            "⚠ run {run_id} SERVI mais NON PERSISTÉ — il ne sera jamais re-prouvable par \
             /v1/swarm/verify/{run_id}. Raison : {}",
            capture.degrade_reason.as_deref().unwrap_or("inconnue")
        );
    }
    // ── M5 : le dossier réglementaire, sans geste manuel ──────────────────────────────────
    // Le run vient d'avoir lieu ; le dossier se dérive DE LUI, ici, sans que personne n'ait à
    // rappeler la passerelle. `reg_dossier` ne peut pas échouer : un dossier manquant est
    // récupérable par `POST /v1/swarm/reg/:id`, contrairement à une persistance manquée.
    // `StoredRun::from_outcome` clone tout le matériel de preuve du run. On ne le construit donc
    // QUE si le pont est câblé : sinon chaque run du build par défaut paierait ce clone pour une
    // capacité que personne n'a demandée. L'autorité reste `reg_dossier` ; ceci est un raccourci,
    // et un test fige l'accord des deux.
    let reg = s
        .reg_pont_cable()
        .then(|| crate::store::StoredRun::from_outcome(&caller.tenant, &outcome))
        .and_then(|st| s.reg_dossier(run_id, &caller.tenant, &st));
    if let Some(r) = &reg {
        if !r.produced {
            eprintln!(
                "⚠ run {run_id} : dossier réglementaire NON produit — {} (le run est persisté, \
                 `POST /v1/swarm/reg/{run_id}` le re-dérive)",
                r.degrade_reason.as_deref().unwrap_or("raison inconnue")
            );
        }
    }
    Ok((StatusCode::OK, Json(RunResponse { run_id, run: outcome.run, capture, reg })).into_response())
}

/// **ADR-019 (MEM-3)** : incrémente les compteurs `/metrics` de recall/learn d'après le résumé
/// mémoire du run (`None` ⇒ mémoire OFF, no-op). Hors chemin chaud (deux `fetch_add` relaxés).
fn record_memory_metrics(state: &SwarmGatewayState, run: &efe_swarm_sdk::SwarmRun) {
    if let Some(m) = &run.memory {
        if m.recalled {
            state.memory_recalls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if m.learned {
            state.memory_learns.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
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
