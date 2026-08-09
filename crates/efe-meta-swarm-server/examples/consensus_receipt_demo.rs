// SPDX-License-Identifier: UNLICENSED
//! # `consensus_receipt_demo` — une ronde de consensus **servie**, et son reçu nominatif.
//!
//! ```text
//! bash scripts/demo-consensus-receipt.sh
//! ```
//!
//! ## Ce que cette démo montre, et en quoi elle diffère de l'existante
//!
//! `efe-meta-swarm/examples/meta_provenance.rs` montre le **moteur** : il scelle une ronde en
//! bibliothèque et produit un bundle v3. Celle-ci montre le **système servi** :
//!
//! * trois participants tournent derrière de vrais serveurs gRPC (tonic) sur des ports TCP
//!   loopback, et le contrôleur les appelle par le réseau — ce n'est pas un appel de fonction
//!   déguisé ;
//! * le reçu est produit par le chemin `run_iterate` **réellement servi**, pas par un chemin de
//!   démonstration parallèle ;
//! * et il est en **schéma v4** : il NOMME ses acteurs et sa règle d'agrégation, là où la v3 ne
//!   scellait que des empreintes positionnelles.
//!
//! **Ce qu'elle ne montre pas, et qu'il faut dire** : les participants vivent dans le processus de
//! la démo, pas dans des processus séparés. Le transport gRPC est réel, l'isolation OS ne l'est
//! pas. C'est un choix de fiabilité — l'orchestration multi-processus de ce dépôt flake sous
//! charge, et une démo qui échoue une fois sur cinq ne démontre rien. Les tests
//! `integration_multiprocess_*` couvrent la séparation OS.
//!
//! ## Le point qui doit rester en tête du spectateur
//!
//! Le reçu atteste l'**intégrité** de la suite de pas — jamais la **correction** de la décision.
//! Et il est *tamper-evident pour un détenteur de clé*, pas *trustless pour un tiers* : la
//! re-preuve exige la clé du tenant. La démo écrit d'ailleurs une variante **falsifiée** pour que
//! l'on voie le vérifieur la refuser : un contrôle qui ne peut pas échouer ne prouve rien.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use efe_meta_swarm::provenance::{HmacKeySource, TenantId};
use efe_meta_swarm_proto::meta_swarm::{
    meta_space_config::SafetyGeometry, BoxSafety as ProtoBoxSafety, IterateRequest, IterationConfig,
    MetaSpaceConfig,
};
use efe_meta_swarm_proto::participant::MetaParticipantServiceServer;
use efe_meta_swarm_server::config::controller::{ControllerConfig, DiscoveryMode, StaticParticipant};
use efe_meta_swarm_server::config::participant::ParticipantConfig;
use efe_meta_swarm_server::controller::provenance::ProvenanceConfig;
use efe_meta_swarm_server::controller::{ControllerService, ControllerState};
use efe_meta_swarm_server::manifest::{
    IterationManifest, Manifest, MetaSpaceManifest, ParticipantManifest, SafetyManifest,
};
use efe_meta_swarm_server::participant::{IterationParams, LocalParticipant, ParticipantService, ParticipantState};
use efe_meta_swarm_server::proto::MetaSwarmService;
use tonic::transport::Server;
use tonic::Request;

/// Le méta-essaim de la démo — le même trio « procurement » que l'exemple moteur, pour que les
/// deux se lisent ensemble.
const AGENTS: [(&str, f32, u64); 3] = [
    ("supplier-scoring", 0.6, 11),
    ("contract-risk", 0.7, 22),
    ("spend-categorization", 0.5, 33),
];
const DIM: u32 = 8;
const OUT_DIR: &str = "demo-bundles/meta-consensus-served";

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("┌─ EFE · Couche 3 — ronde de consensus SERVIE, et son reçu re-prouvable");
    println!("│");

    // ── 1. Trois participants derrière de VRAIS serveurs gRPC ───────────────────────────────
    let mut statics: Vec<(String, usize, f32, u64, SocketAddr)> = Vec::new();
    for (id, kappa, seed) in AGENTS {
        let addr = spawn_participant(id, kappa, seed).await?;
        println!("│  participant « {id} » écoute en gRPC sur {addr}");
        statics.push((id.into(), DIM as usize, kappa, seed, addr));
    }

    // ── 2. Le contrôleur, AVEC provenance ───────────────────────────────────────────────────
    //
    // Clé de DÉVELOPPEMENT : le reçu produit ici est donc FORGEABLE par quiconque connaît cette
    // clé (elle est publique). C'est acceptable pour une démonstration, et il faut le dire —
    // présenter un reçu sous clé de dev comme une preuve serait exactement la malhonnêteté que
    // tout ce dispositif existe pour empêcher.
    let mut state = build_controller_state(&statics);
    state.set_provenance(std::sync::Arc::new(ProvenanceConfig::new(
        HmacKeySource::test_key(),
        TenantId::system(),
        "test_key() (DÉMO — reçu forgeable)",
        true,
    )));
    let svc = ControllerService::new(state);
    println!("│  contrôleur prêt — provenance ACTIVE (clé de démo, reçu forgeable)");
    println!("│");

    // ── 3. La ronde, par le chemin SERVI ────────────────────────────────────────────────────
    let resp = svc
        .iterate(Request::new(iterate_request(true)))
        .await
        .map_err(|s| format!("l'Iterate servi a échoué : {s}"))?
        .into_inner();

    println!("│  ronde terminée : {} itérations, résidu {:.3e}", resp.iterations, resp.final_residual);
    let bundle = resp
        .meta_provenance_bundle
        .as_ref()
        .ok_or("le contrôleur n'a pas rendu de reçu — la provenance était-elle active ?")?;

    // ── 4. Le reçu, en clair — CE QUE LA v4 A CHANGÉ ────────────────────────────────────────
    println!("│");
    println!("├─ LE REÇU");
    println!("│  root : {}", resp.meta_provenance_root);
    print_binding(&bundle.manifest_json);
    println!("│");

    // ── 5. Les artefacts, tels qu'un vérifieur les consomme ─────────────────────────────────
    let dir = PathBuf::from(OUT_DIR);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("meta_steps.jsonl"), &bundle.steps_jsonl)?;
    std::fs::write(dir.join("meta_manifest.json"), &bundle.manifest_json)?;

    // La variante FALSIFIÉE cible ce que la v4 protège : on réécrit une IDENTITÉ dans le
    // manifeste. La chaîne, elle, reste parfaitement intacte — c'est tout l'intérêt du cas : sans
    // le recoupement de liaison, ce bundle passerait, et l'auditeur lirait un faux nom sous un
    // root valide.
    let forged = bundle.manifest_json.replacen("supplier-scoring", "acme-consulting", 1);
    assert_ne!(forged, bundle.manifest_json, "la falsification doit mordre");
    std::fs::write(dir.join("meta_manifest.FORGED.json"), &forged)?;

    println!("├─ ARTEFACTS écrits dans {OUT_DIR}/");
    println!("│    meta_steps.jsonl            ({} pas scellés)", bundle.steps_jsonl.lines().count());
    println!("│    meta_manifest.json          (la revendication)");
    println!("│    meta_manifest.FORGED.json   (une IDENTITÉ réécrite — doit être REFUSÉ)");
    println!("│");
    println!("└─ à re-prouver avec un binaire dont l'arbre de dépendances est VIDE :");
    println!("     verify_meta_mini {OUT_DIR}/meta_steps.jsonl {OUT_DIR}/meta_manifest.json \\");
    println!("       --key-hex {}", hex(&HmacKeySource::test_key().derive(TenantId::system())?));
    Ok(())
}

/// Affiche la liaison v4 — qui a contribué, sous quelle règle. C'est la réponse à la question
/// qu'un auditeur pose en premier, et qu'un reçu v3 ne pouvait pas donner.
fn print_binding(manifest_json: &str) {
    let Ok(m) = serde_json::from_str::<serde_json::Value>(manifest_json) else {
        println!("│  (manifeste illisible)");
        return;
    };
    let schema = m.get("schema_version").and_then(serde_json::Value::as_u64).unwrap_or(0);
    println!("│  schéma : v{schema}{}", if schema >= 4 { "  (le reçu NOMME ses acteurs)" } else { "  (positionnel seulement)" });
    let ids = m.get("binding_participant_ids").and_then(serde_json::Value::as_array);
    let cfgs = m.get("binding_declared_config_hashes").and_then(serde_json::Value::as_array);
    if let (Some(ids), Some(cfgs)) = (ids, cfgs) {
        println!("│  agents (dans l'ordre où leurs votes sont scellés) :");
        for (i, id) in ids.iter().enumerate() {
            let cfg = cfgs.get(i).and_then(serde_json::Value::as_str).unwrap_or("");
            let cfg = if cfg.is_empty() { "—  (mock : aucune configuration à attester)".to_string() } else { format!("config {}…", &cfg[..12.min(cfg.len())]) };
            println!("│    [{i}] {}  ·  {cfg}", id.as_str().unwrap_or("?"));
        }
    }
    if let Some(op) = m.get("binding_operator").and_then(serde_json::Value::as_str) {
        let a = m.get("binding_alpha").and_then(serde_json::Value::as_f64).unwrap_or(0.0);
        let g = m.get("binding_gamma").and_then(serde_json::Value::as_f64).unwrap_or(0.0);
        println!("│  règle d'agrégation : « {op} »   (α = {a}, γ = {g})");
    }
}

// =====================================================================
// Le harnais — délibérément dupliqué depuis `tests/common`
// =====================================================================
//
// `tests/common` n'est pas visible depuis un exemple. Plutôt que de l'exposer dans la
// bibliothèque (une surface publique qui n'existerait que pour la démo), on recopie les quelques
// lignes nécessaires : un exemple qui dérive du harnais de test est moins grave qu'une API
// publique dont personne ne se sert en production.

async fn spawn_participant(id: &str, kappa: f32, seed: u64) -> Result<SocketAddr, Box<dyn std::error::Error>> {
    let row = ParticipantManifest {
        id: id.into(),
        dim: DIM,
        kappa: Some(kappa),
        seed: Some(seed),
        blas_backend: "none".into(),
        controller: None,
    };
    let local = LocalParticipant::from_manifest_row(&row)?;
    let config = ParticipantConfig {
        id: id.into(),
        grpc_bind: "127.0.0.1:0".parse()?,
        metrics_bind: "127.0.0.1:0".parse()?,
        worker_threads: 1,
        tls_required: false,
        tls_cert_path: None,
        tls_key_path: None,
        tls_ca_path: None,
        tls_mutual: false,
        tls_min_protocol: efe_meta_swarm_server::tls::ProtocolVersion::Tls13,
        tls_allowed_peer_sans: Vec::new(),
        tls_peer_san: None,
    };
    let svc = ParticipantService::new(ParticipantState::new(
        config,
        local,
        IterationParams { alpha_default: 0.5, coupling_gamma: 0.2 },
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    drop(listener);
    let id_owned = id.to_string();
    tokio::spawn(async move {
        if let Err(e) = Server::builder()
            .add_service(MetaParticipantServiceServer::new(svc))
            .serve(addr)
            .await
        {
            eprintln!("participant {id_owned} : serveur arrêté — {e}");
        }
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return Ok(addr);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(format!("le participant {id} n'a pas démarré sur {addr}").into())
}

fn build_controller_state(statics: &[(String, usize, f32, u64, SocketAddr)]) -> ControllerState {
    let config = ControllerConfig {
        grpc_bind: "127.0.0.1:0".parse().unwrap(),
        metrics_bind: "127.0.0.1:0".parse().unwrap(),
        discovery: DiscoveryMode::StaticLocalhost,
        static_participants: statics
            .iter()
            .map(|(id, _, _, _, addr)| StaticParticipant { id: id.clone(), addr: addr.to_string() })
            .collect(),
        manifest_path: "/dev/null".into(),
        max_inflight: 4,
        default_max_wall_ms: 15_000,
        worker_threads: 1,
        reflection_enabled: false,
        tls_required: false,
        tls_cert_path: None,
        tls_key_path: None,
        tls_ca_path: None,
        tls_mutual: false,
        tls_min_protocol: efe_meta_swarm_server::tls::ProtocolVersion::Tls13,
        tls_allowed_peer_sans: Vec::new(),
        tls_peer_san: None,
        drain_timeout: Duration::from_secs(30),
    };
    let manifest = Manifest {
        version: "v0.2.0".into(),
        meta: MetaSpaceManifest { dim: DIM, safety: SafetyManifest::Box { radius: 50.0 } },
        participants: statics
            .iter()
            .map(|(id, dim, kappa, seed, _)| ParticipantManifest {
                id: id.clone(),
                dim: u32::try_from(*dim).unwrap_or(DIM),
                kappa: Some(*kappa),
                seed: Some(*seed),
                blas_backend: "none".into(),
                controller: None,
            })
            .collect(),
        iteration: IterationManifest {
            max_iters: 100,
            tolerance: 1.0e-4,
            alpha_default: 0.5,
            coupling_gamma: 0.2,
            deterministic: true,
            max_wall_ms: 15_000,
            require_convergence: None,
        },
    };
    ControllerState::new(config, manifest)
}

fn iterate_request(include_receipt: bool) -> IterateRequest {
    IterateRequest {
        expected_participants: vec![],
        meta: Some(MetaSpaceConfig {
            dim: DIM,
            safety_geometry: Some(SafetyGeometry::Box(ProtoBoxSafety { radius: 50.0 })),
        }),
        iteration: Some(IterationConfig {
            max_iters: 100,
            tolerance: 1.0e-3,
            alpha_default: 0.5,
            coupling_gamma: 0.2,
            deterministic: true,
            max_wall_ms: 15_000,
        }),
        initial_states: vec![],
        traceparent: String::new(),
        include_provenance_bundle: include_receipt,
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
