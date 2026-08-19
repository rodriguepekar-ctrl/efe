// SPDX-License-Identifier: UNLICENSED
//! # `/v1/consensus` — Couche 3 M2 : la façade HTTP de la ronde de consensus **avec son reçu**.
//!
//! `POST /v1/consensus` exécute une ronde sur le contrôleur méta-essaim et rend le consensus
//! **accompagné de son reçu re-prouvable** : le `meta_provenance_root`, et — sur demande — les deux
//! artefacts que les vérifieurs hors-ligne consomment tels quels.
//!
//! ```text
//! POST /v1/consensus            → { "consensus": [...], "provenance": { "root": "...", ... } }
//!   ↓  (le client enregistre les deux artefacts)
//! verify_meta_mini meta_steps.jsonl meta_manifest.json --key-hex <64hex>
//! ```
//!
//! ## La seule règle de conception de ce module : **ne rien toucher**
//!
//! Cette façade est un **passe-plat**. Elle ne fabrique pas de reçu, ne le re-sérialise pas, ne le
//! reformate pas — elle transporte les octets que le contrôleur a scellés. La raison est
//! mécanique, pas esthétique : le root est le HMAC d'un contenu **exact**. Une ré-écriture, fût-ce
//! d'une espace ou d'un `\n` final, produirait un bundle qui ne se re-prouve plus — et l'échec ne
//! serait découvert que chez l'auditeur, qui n'a aucun moyen de savoir que c'est la façade, et non
//! le sceau, qui a menti. `receipt_survives_the_http_round_trip` épingle exactement cela.
//!
//! ## Ce que le reçu atteste, et ce qu'il n'atteste pas
//!
//! **Intégrité**, jamais **correction** : il prouve que la suite de pas rendue est bien celle qui a
//! été scellée, pas que la décision de consensus est juste. Et il est *tamper-evident pour un
//! détenteur de clé*, pas *trustless pour un tiers* : la re-preuve exige la clé du tenant, donc un
//! détenteur peut fabriquer une chaîne cohérente. C'est le même cadrage que le reçu R4 de la
//! Couche 2 et que la pile de provenance P1 ; il est répété ici parce qu'une façade HTTP est
//! précisément l'endroit où un lecteur pressé conclurait « vérifié = correct ».

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::post, Json, Router};
use serde::{Deserialize, Serialize};

use crate::services::meta_swarm::{MetaSwarmClient, MetaSwarmClientError};

/// Corps de `POST /v1/consensus`.
#[derive(Debug, Clone, Deserialize)]
pub struct ConsensusBody {
    /// Dimension de l'espace méta (doit correspondre au manifeste du contrôleur).
    pub meta_dim: u32,
    /// Borne d'itérations.
    #[serde(default = "default_max_iters")]
    pub max_iters: u32,
    /// Tolérance d'arrêt sur le résidu joint.
    #[serde(default = "default_tolerance")]
    pub tolerance: f64,
    /// Réclamer le reçu **complet** (les deux artefacts), et pas seulement son root.
    ///
    /// Par défaut `false` : le bundle coûte O(itérations × dim) et un appelant qui ne re-prouvera
    /// jamais ne devrait pas le payer. Le root, lui, vient toujours.
    #[serde(default)]
    pub include_receipt: bool,
}

fn default_max_iters() -> u32 {
    100
}
fn default_tolerance() -> f64 {
    1.0e-3
}

/// Le reçu, tel qu'il est rendu au client HTTP — **les octets du contrôleur, inchangés**.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsensusReceipt {
    /// `meta_provenance_root` — 64 hex. Vide ⟺ le contrôleur ne scelle pas (aucun reçu).
    pub root: String,
    /// `meta_steps.jsonl` verbatim. `None` si le reçu complet n'a pas été demandé.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps_jsonl: Option<String>,
    /// `meta_manifest.json` verbatim. `None` si le reçu complet n'a pas été demandé.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_json: Option<String>,
    /// Comment re-prouver, littéralement — pour que le lecteur n'ait pas à deviner.
    pub how_to_verify: String,
    /// Ce que le reçu atteste, dit sur le reçu lui-même plutôt que dans une doc qu'on ne lira pas.
    pub attests: String,
}

/// La commande de re-preuve, mot pour mot.
const HOW_TO_VERIFY: &str =
    "écrire steps_jsonl dans meta_steps.jsonl et manifest_json dans meta_manifest.json, puis : \
     verify_meta_mini meta_steps.jsonl meta_manifest.json --key-hex <clé tenant, 64 hex>";

/// Le cadrage honnête, transporté AVEC le reçu.
const ATTESTS: &str =
    "INTÉGRITÉ de la suite de pas scellée — PAS la correction de la décision de consensus. \
     Tamper-evident pour un DÉTENTEUR DE CLÉ, pas trustless pour un tiers.";

/// Réponse de `POST /v1/consensus`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusResponse {
    /// Le vecteur de consensus méta.
    pub consensus: Vec<f32>,
    /// Nombre d'itérations effectuées.
    pub iterations: u32,
    /// Résidu joint final.
    pub final_residual: f64,
    /// Raison d'arrêt, en clair (`converged` / `max_iters` / …).
    pub stop_reason: String,
    /// Durée murale côté contrôleur.
    pub wall_us: u64,
    /// Le reçu. **Toujours présent**, même quand le contrôleur ne scelle pas — auquel cas son
    /// `root` est vide. Rendre le champ absent laisserait croire à un oubli ; un root vide dit
    /// explicitement « aucun reçu », ce qui est le signal honnête.
    pub provenance: ConsensusReceipt,
}

/// Erreurs de la façade.
#[derive(Debug, thiserror::Error)]
pub enum ConsensusApiError {
    /// Le sous-système méta-essaim n'est pas activé sur ce serveur.
    #[error("meta-swarm subsystem disabled (EFE_META_SWARM_ENABLED=false)")]
    Disabled,
    /// Corps de requête invalide.
    #[error("invalid body : {0}")]
    InvalidBody(String),
    /// Le contrôleur a refusé ou échoué. Le code gRPC est **conservé** puis traduit.
    #[error("controller : {0}")]
    Controller(#[from] MetaSwarmClientError),
}

impl IntoResponse for ConsensusApiError {
    fn into_response(self) -> axum::response::Response {
        // La traduction gRPC → HTTP est délibérée, pas mécanique : un `ABORTED` (non-convergence
        // fail-closed) n'est PAS une erreur serveur — le contrôleur a fonctionné exactement comme
        // demandé et a refusé d'étiqueter « consensus » un point non convergé. Le rendre en 500
        // enverrait un opérateur chercher une panne qui n'existe pas.
        let (code, msg) = match &self {
            Self::Disabled => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            Self::InvalidBody(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Controller(MetaSwarmClientError::IterateRpcFailed(s)) => {
                let http = match s.code() {
                    tonic::Code::Aborted => StatusCode::CONFLICT, // non convergé : demande légitime, refus légitime
                    tonic::Code::InvalidArgument | tonic::Code::FailedPrecondition => StatusCode::BAD_REQUEST,
                    tonic::Code::DeadlineExceeded => StatusCode::GATEWAY_TIMEOUT,
                    tonic::Code::Unavailable => StatusCode::BAD_GATEWAY,
                    tonic::Code::Cancelled => StatusCode::REQUEST_TIMEOUT,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                };
                (http, format!("controller {:?} : {}", s.code(), s.message()))
            }
            Self::Controller(e) => (StatusCode::BAD_GATEWAY, e.to_string()),
        };
        (code, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

/// `POST /v1/consensus`
///
/// # Errors
/// Voir [`ConsensusApiError`].
pub async fn consensus(
    State(client): State<Option<Arc<MetaSwarmClient>>>,
    Json(body): Json<ConsensusBody>,
) -> Result<Json<ConsensusResponse>, ConsensusApiError> {
    let client = client.ok_or(ConsensusApiError::Disabled)?;
    if body.meta_dim == 0 {
        return Err(ConsensusApiError::InvalidBody("meta_dim doit être > 0".into()));
    }
    if body.max_iters == 0 {
        return Err(ConsensusApiError::InvalidBody("max_iters doit être > 0".into()));
    }

    let resp = client.iterate(build_request(&body)).await?;
    Ok(Json(to_response(resp)))
}

/// Construit la requête gRPC. Isolé du handler pour être testable sans contrôleur.
fn build_request(body: &ConsensusBody) -> efe_meta_swarm_proto::meta_swarm::IterateRequest {
    use efe_meta_swarm_proto::meta_swarm::{
        meta_space_config::SafetyGeometry, BoxSafety, IterateRequest, IterationConfig, MetaSpaceConfig,
    };
    IterateRequest {
        expected_participants: vec![],
        meta: Some(MetaSpaceConfig {
            dim: body.meta_dim,
            safety_geometry: Some(SafetyGeometry::Box(BoxSafety { radius: 50.0 })),
        }),
        iteration: Some(IterationConfig {
            max_iters: body.max_iters,
            tolerance: body.tolerance,
            alpha_default: 0.5,
            coupling_gamma: 0.2,
            deterministic: true,
            max_wall_ms: 10_000,
        }),
        initial_states: vec![],
        traceparent: String::new(),
        include_provenance_bundle: body.include_receipt,
    }
}

/// Projette la réponse gRPC en JSON. **Le reçu traverse sans être retouché** — c'est l'invariant
/// que `receipt_survives_the_http_round_trip` épingle.
fn to_response(r: efe_meta_swarm_proto::meta_swarm::IterateResponse) -> ConsensusResponse {
    use efe_meta_swarm_proto::meta_swarm::StopReason;
    let stop_reason = match StopReason::try_from(r.stop_reason) {
        Ok(StopReason::Converged) => "converged",
        Ok(StopReason::MaxIters) => "max_iters",
        Ok(StopReason::ParticipantDown) => "participant_down",
        _ => "unspecified",
    }
    .to_string();

    let (steps_jsonl, manifest_json) = match r.meta_provenance_bundle {
        Some(b) => (Some(b.steps_jsonl), Some(b.manifest_json)),
        None => (None, None),
    };

    ConsensusResponse {
        consensus: r.meta_consensus.map(|t| t.data).unwrap_or_default(),
        iterations: r.iterations,
        final_residual: r.final_residual,
        stop_reason,
        wall_us: r.wall_us,
        provenance: ConsensusReceipt {
            root: r.meta_provenance_root,
            steps_jsonl,
            manifest_json,
            how_to_verify: HOW_TO_VERIFY.to_string(),
            attests: ATTESTS.to_string(),
        },
    }
}

/// Construit le routeur `/v1/consensus`.
///
/// `client = None` ⟹ les routes répondent `503` avec un message qui NOMME le drapeau : monter la
/// route sur un serveur où le sous-système est éteint doit produire une erreur diagnosticable, pas
/// un 404 qui laisse croire à une faute de chemin.
#[must_use]
pub fn router(client: Option<Arc<MetaSwarmClient>>) -> Router {
    Router::new().route("/consensus", post(consensus)).with_state(client)
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_meta_swarm_proto::meta_swarm::{
        IterateResponse, MetaProvenanceBundle, StopReason, TensorF32,
    };

    /// **L'invariant central de cette façade.** Un reçu qui traverse la projection gRPC → JSON →
    /// (sérialisation HTTP) → JSON ressort **byte-identique**, et se re-prouve encore.
    ///
    /// Le risque que ce test écarte est précis : le root est le HMAC d'un contenu exact. Si la
    /// façade re-sérialisait le JSONL (ré-indentation, `\n` final perdu, ré-ordonnancement des
    /// clés), le bundle rendu ne se re-prouverait plus — et l'échec apparaîtrait chez l'auditeur,
    /// qui conclurait à un sceau menteur plutôt qu'à une façade bavarde.
    #[test]
    fn receipt_survives_the_http_round_trip() {
        // Un bundle porteur de tout ce qui casse une sérialisation naïve : espaces, guillemets
        // échappés, unicode, et un `\n` final significatif.
        let steps = "{\"meta_id\":\"m\",\"step\":0,\"residual\":0.5,\"consensus\":[1.0]}\n\
                     {\"meta_id\":\"m\",\"step\":1,\"residual\":0.25,\"note\":\"a\\\"b — é\"}\n";
        let manifest = "{\n  \"meta_id\": \"m\",\n  \"n_steps\": 2,\n  \"root\": \"ab\"\n}\n";

        let grpc = IterateResponse {
            iterations: 2,
            final_residual: 0.25,
            meta_consensus: Some(TensorF32 { dim: 1, data: vec![1.0] }),
            final_states: vec![],
            stop_reason: StopReason::Converged as i32,
            wall_us: 42,
            trace_id: String::new(),
            meta_provenance_root: "ab".into(),
            meta_provenance_bundle: Some(MetaProvenanceBundle {
                steps_jsonl: steps.into(),
                manifest_json: manifest.into(),
            }),
        };

        let http = to_response(grpc);
        // Aller-retour par le transport RÉEL de la façade (serde_json), pas par une copie mémoire.
        let wire = serde_json::to_string(&http).expect("sérialisable");
        let back: ConsensusResponse = serde_json::from_str(&wire).expect("désérialisable");

        assert_eq!(
            back.provenance.steps_jsonl.as_deref(),
            Some(steps),
            "les pas doivent ressortir OCTET POUR OCTET",
        );
        assert_eq!(
            back.provenance.manifest_json.as_deref(),
            Some(manifest),
            "le manifeste doit ressortir OCTET POUR OCTET (indentation et \\n final compris)",
        );
        assert_eq!(back.provenance.root, "ab");
        assert_eq!(back.stop_reason, "converged");
    }

    /// Sans provenance côté contrôleur, le champ existe et son root est **vide** — jamais absent,
    /// jamais un placeholder. Un champ manquant se lit comme un oubli ; un root vide se lit comme
    /// « ce contrôleur ne scelle pas », ce qui est l'information juste.
    #[test]
    fn an_unsealed_round_yields_an_empty_root_not_a_missing_field() {
        let grpc = IterateResponse {
            iterations: 3,
            final_residual: 0.1,
            meta_consensus: Some(TensorF32 { dim: 1, data: vec![0.5] }),
            final_states: vec![],
            stop_reason: StopReason::MaxIters as i32,
            wall_us: 7,
            trace_id: String::new(),
            meta_provenance_root: String::new(),
            meta_provenance_bundle: None,
        };
        let http = to_response(grpc);
        assert!(http.provenance.root.is_empty());
        assert!(http.provenance.steps_jsonl.is_none());

        let wire = serde_json::to_string(&http).expect("sérialisable");
        assert!(wire.contains("\"root\":\"\""), "le root VIDE est présent sur le fil : {wire}");
        // On vise la CLÉ, pas le mot : « steps_jsonl » figure aussi dans le texte de
        // `how_to_verify` (« écrire steps_jsonl dans meta_steps.jsonl … »). La première
        // version de cette assertion cherchait le mot et échouait sur un comportement
        // pourtant juste — un test qui accuse le code à tort est un test à corriger.
        assert!(!wire.contains("\"steps_jsonl\":"), "aucune CLÉ steps_jsonl vide : {wire}");
        assert!(!wire.contains("\"manifest_json\":"), "aucune CLÉ manifest_json vide : {wire}");
        assert!(wire.contains("PAS la correction"), "le cadrage voyage AVEC le reçu : {wire}");
    }

    /// La demande de reçu se propage : sans elle, le contrôleur n'enverra pas le bundle.
    #[test]
    fn the_receipt_request_flag_reaches_the_controller() {
        let body = ConsensusBody { meta_dim: 8, max_iters: 50, tolerance: 1e-4, include_receipt: true };
        assert!(build_request(&body).include_provenance_bundle);

        let body = ConsensusBody { include_receipt: false, ..body };
        assert!(!build_request(&body).include_provenance_bundle);
    }

    /// Une non-convergence fail-closed (`ABORTED`) est un **409**, pas un 500 : la demande était
    /// légitime, le refus aussi. Un 500 enverrait chercher une panne inexistante.
    #[test]
    fn a_fail_closed_non_convergence_is_not_reported_as_a_server_fault() {
        let err = ConsensusApiError::Controller(MetaSwarmClientError::IterateRpcFailed(
            tonic::Status::aborted("convergence failed"),
        ));
        assert_eq!(err.into_response().status(), StatusCode::CONFLICT);

        let err = ConsensusApiError::Disabled;
        assert_eq!(err.into_response().status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
