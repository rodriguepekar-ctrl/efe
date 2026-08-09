// SPDX-License-Identifier: UNLICENSED
//! # `consensus_receipt_demo` — une ronde de consensus **servie**, et son reçu nominatif.
//!
//! ```text
//! bash scripts/demo-consensus-receipt.sh          # participants MOCK (rapide)
//! bash scripts/demo-consensus-receipt.sh --real   # participants efe-core RÉELS
//! ```
//!
//! ## Ce que cette démo montre, et en quoi elle diffère de l'existante
//!
//! `efe-meta-swarm/examples/meta_provenance.rs` montre le **moteur** : il scelle une ronde en
//! bibliothèque et produit un bundle v3. Celle-ci montre le **système servi** :
//!
//! * les participants tournent derrière de vrais serveurs gRPC (tonic) sur des ports TCP
//!   loopback, et le contrôleur les appelle par le réseau — ce n'est pas un appel de fonction
//!   déguisé ;
//! * le reçu est produit par le chemin `run_iterate` **réellement servi**, pas par un chemin de
//!   démonstration parallèle ;
//! * et il est en **schéma v4** : il NOMME ses acteurs et sa règle d'agrégation, là où la v3 ne
//!   scellait que des empreintes positionnelles.
//!
//! ## Les deux modes, et pourquoi le second existe
//!
//! **Sans drapeau** (défaut) — trois participants `Φ(x) = κ·x`. Rapide, mais la colonne de
//! configuration du reçu est **vide** : un mock n'a aucune version à attester. Le mécanisme est
//! démontré, la substance de ce qu'il atteste reste synthétique. C'est la première chose qu'un
//! évaluateur attentif remarque, et il a raison.
//!
//! **`--engine`** — trois participants **efe-core réels** (fusion Mode-1 latente, topologie fixe),
//! chacun déclaré par un bloc `controller` dont le SHA-256 est scellé dans la liaison. Le reçu
//! affiche alors trois `config_hash` réels : « cet agent, dans **cette version précise**, a
//! contribué ceci ». C'est la phrase qu'un acheteur conformité veut entendre, et elle n'est
//! atteignable qu'ici. Chaque participant passe en plus le **canari H1 de boot** — fail-closed :
//! un opérateur dont la non-expansivité est falsifiée refuse de servir.
//!
//! Ce mode exige la feature `core-integration` (elle tire efe-core + efe-server).
//!
//! ## Ce que la démo ne montre pas, et qu'il faut dire
//!
//! Les participants vivent dans le processus de la démo, pas dans des processus séparés. Le
//! transport gRPC est réel, l'isolation OS ne l'est pas — choix de fiabilité, l'orchestration
//! multi-processus de ce dépôt flake sous charge, et une démo qui échoue une fois sur cinq ne
//! démontre rien. Les tests `integration_multiprocess_*` couvrent la séparation OS.
//!
//! Le reçu atteste l'**intégrité** de la suite de pas — jamais la **correction** de la décision.
//! Et il est *tamper-evident pour un détenteur de clé*, pas *trustless pour un tiers*. La démo
//! écrit d'ailleurs une variante **falsifiée** pour que l'on voie le vérifieur la refuser : un
//! contrôle qui ne peut pas échouer ne prouve rien.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

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
    ControllerManifest, IterationManifest, Manifest, MetaSpaceManifest, ParticipantManifest,
    SafetyManifest,
};
use efe_meta_swarm_server::participant::{
    IterationParams, LocalParticipant, ParticipantService, ParticipantState,
};
use efe_meta_swarm_server::proto::MetaSwarmService;
use tonic::transport::Server;
use tonic::Request;

/// Le trio « procurement » — le même que l'exemple moteur, pour que les deux se lisent ensemble.
const AGENTS: [(&str, f32, u64); 3] = [
    ("supplier-scoring", 0.6, 11),
    ("contract-risk", 0.7, 22),
    ("spend-categorization", 0.5, 33),
];
/// 16 — la dimension de la recette efe-core éprouvée (e2e M1.d). Les mocks y sont indifférents.
const DIM: u32 = 16;
const OUT_DIR: &str = "demo-bundles/meta-consensus-served";

/// Une ligne de participant : mock (κ, seed) ou moteur réel (bloc `controller`).
struct Agent {
    row: ParticipantManifest,
    addr: SocketAddr,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine_mode = std::env::args().any(|a| a == "--engine");
    let started = Instant::now();

    println!("┌─ EFE · Couche 3 — ronde de consensus SERVIE, et son reçu re-prouvable");
    if engine_mode {
        println!("│  mode : PARTICIPANTS efe-core RÉELS (fusion Mode-1 latente, topologie fixe)");
    } else {
        println!("│  mode : participants MOCK (Φ = κ·x) — la colonne « config » du reçu sera VIDE,");
        println!("│         un mock n'a aucune version à attester. Utiliser --engine pour du réel.");
    }
    println!("│");

    // ── 1. Les participants, derrière de VRAIS serveurs gRPC ────────────────────────────────
    let mut agents: Vec<Agent> = Vec::new();
    for (id, kappa, seed) in AGENTS {
        let row = participant_row(id, kappa, seed, engine_mode)?;
        let (addr, is_core) = spawn_participant(&row, engine_mode).await?;
        // Cohérence déclaré ↔ servi : un écart ici voudrait dire qu'on sert autre chose que ce
        // que le reçu va attester. On préfère s'arrêter que le laisser passer.
        if is_core != row.controller.is_some() {
            return Err(format!(
                "participant {id} : la ligne déclare {} mais le participant sert {} — \
                 le reçu attesterait une configuration qui n'a pas tourné",
                if row.controller.is_some() { "efe-core" } else { "mock" },
                if is_core { "efe-core" } else { "mock" },
            )
            .into());
        }
        let kind = if is_core { "efe-core  ✓ canari H1 passé au boot" } else { "mock" };
        println!("│  participant « {id} » ({kind}) écoute en gRPC sur {addr}");
        agents.push(Agent { row, addr });
    }
    println!("│");

    // ── 2. Le contrôleur, AVEC provenance ───────────────────────────────────────────────────
    //
    // Clé de DÉVELOPPEMENT : le reçu produit ici est FORGEABLE par quiconque connaît cette clé
    // (elle est publique). Acceptable pour une démonstration, et il faut le dire — présenter un
    // reçu sous clé de dev comme une preuve serait exactement la malhonnêteté que tout ce
    // dispositif existe pour empêcher.
    let mut state = build_controller_state(&agents);
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
    let round_started = Instant::now();
    let resp = svc
        .iterate(Request::new(iterate_request()))
        .await
        .map_err(|s| format!("l'Iterate servi a échoué : {s}"))?
        .into_inner();

    println!(
        "│  ronde terminée : {} itérations, résidu {:.3e}  ({} ms de calcul servi)",
        resp.iterations,
        resp.final_residual,
        round_started.elapsed().as_millis(),
    );
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

    // La variante FALSIFIÉE cible ce que la v4 protège : une IDENTITÉ réécrite dans le manifeste.
    // La chaîne des pas, elle, reste parfaitement intacte — c'est tout l'intérêt du cas : sans le
    // recoupement de liaison, ce bundle passerait, et l'auditeur lirait un faux nom sous un root
    // valide.
    let forged = bundle.manifest_json.replacen("supplier-scoring", "acme-consulting", 1);
    assert_ne!(forged, bundle.manifest_json, "la falsification doit mordre");
    std::fs::write(dir.join("meta_manifest.FORGED.json"), &forged)?;

    println!("├─ ARTEFACTS écrits dans {OUT_DIR}/");
    println!("│    meta_steps.jsonl            ({} pas scellés)", bundle.steps_jsonl.lines().count());
    println!("│    meta_manifest.json          (la revendication)");
    println!("│    meta_manifest.FORGED.json   (une IDENTITÉ réécrite — doit être REFUSÉ)");
    println!("│");
    println!(
        "└─ à re-prouver avec un binaire dont l'arbre de dépendances est VIDE   [{} ms au total]",
        started.elapsed().as_millis(),
    );
    println!("     verify_meta_mini {OUT_DIR}/meta_steps.jsonl {OUT_DIR}/meta_manifest.json \\");
    println!("       --key-hex {}", hex(&HmacKeySource::test_key().derive(TenantId::system())?));
    Ok(())
}

/// Affiche la liaison v4 — qui a contribué, sous quelle configuration, sous quelle règle. C'est la
/// réponse à la question qu'un auditeur pose en premier, et qu'un reçu v3 ne pouvait pas donner.
fn print_binding(manifest_json: &str) {
    let Ok(m) = serde_json::from_str::<serde_json::Value>(manifest_json) else {
        println!("│  (manifeste illisible)");
        return;
    };
    let schema = m.get("schema_version").and_then(serde_json::Value::as_u64).unwrap_or(0);
    println!(
        "│  schéma : v{schema}{}",
        if schema >= 4 { "  (le reçu NOMME ses acteurs)" } else { "  (positionnel seulement)" }
    );
    let ids = m.get("binding_participant_ids").and_then(serde_json::Value::as_array);
    let cfgs = m.get("binding_declared_config_hashes").and_then(serde_json::Value::as_array);
    if let (Some(ids), Some(cfgs)) = (ids, cfgs) {
        println!("│  agents (dans l'ordre où leurs votes sont scellés) :");
        for (i, id) in ids.iter().enumerate() {
            let raw = cfgs.get(i).and_then(serde_json::Value::as_str).unwrap_or("");
            let cfg = if raw.is_empty() {
                "—  (mock : aucune configuration à attester)".to_string()
            } else {
                format!("config {}…{}", &raw[..8], &raw[raw.len() - 8..])
            };
            println!("│    [{i}] {:<24} ·  {cfg}", id.as_str().unwrap_or("?"));
        }
    }
    if let Some(op) = m.get("binding_operator").and_then(serde_json::Value::as_str) {
        let a = m.get("binding_alpha").and_then(serde_json::Value::as_f64).unwrap_or(0.0);
        let g = m.get("binding_gamma").and_then(serde_json::Value::as_f64).unwrap_or(0.0);
        println!("│  règle d'agrégation : « {op} »   (α = {a}, γ = {g})");
    }
}

/// La ligne de manifeste d'un participant. **Mock et moteur s'excluent** : une ligne efe-core
/// OMET `kappa`/`seed` (concepts mock), ce que `validate_manifest` impose fail-closed dans les
/// deux sens — un binaire antérieur à M1.d refuse alors de parser la ligne plutôt que de la
/// dégrader silencieusement en mock.
fn participant_row(
    id: &str,
    kappa: f32,
    seed: u64,
    engine: bool,
) -> Result<ParticipantManifest, Box<dyn std::error::Error>> {
    if !engine {
        return Ok(ParticipantManifest {
            id: id.into(),
            dim: DIM,
            kappa: Some(kappa),
            seed: Some(seed),
            blas_backend: "none".into(),
            controller: None,
        });
    }
    Ok(ParticipantManifest {
        id: id.into(),
        dim: DIM,
        kappa: None,
        seed: None,
        blas_backend: "none".into(),
        controller: Some(engine_block(id)),
    })
}

/// Le bloc `controller` d'un participant efe-core — la recette éprouvée de l'e2e M1.d.
///
/// `tenant_id` varie par agent, donc **chaque agent a son propre `config_hash`** : le reçu
/// distingue trois configurations et non trois copies de la même, ce qui est le cas réaliste.
fn engine_block(id: &str) -> ControllerManifest {
    use efe_meta_swarm_server::manifest::{
        AlignmentConeManifest, H1CanaryManifest, PopulationManifest, SafetyBoxManifest,
        TopologyManifest,
    };
    ControllerManifest {
        schema: 1,
        population: vec![PopulationManifest { template: "na-sem".into(), count: 4 }],
        max_rounds: 3,
        alignment_cone: AlignmentConeManifest {
            anchors: vec![vec![0.1; DIM as usize]],
            max_projection_distance: 0.5,
        },
        safety_box: SafetyBoxManifest { lo: 0.05, hi: 0.30 },
        // Palier-2 : topologie FIXE ⟹ Φ continu par construction (la discontinuité du routage
        // génésique, seul mécanisme connu de rupture de H1, est éliminée).
        topology: TopologyManifest::Fixed,
        tenant_id: format!("demo-{id}"),
        // Sonde de boot RÉDUITE (~18 évaluations de Φ) : la production utilise les défauts du
        // schéma, plus larges. Une démo n'a pas à durer trois minutes pour démontrer un gate.
        h1_canary: H1CanaryManifest {
            mode: "boot".into(),
            n_seeds: 6,
            n_pairs: 8,
            n_directions: 1,
            n_bisect_pairs: 1,
            bisect_depth: 6,
            max_evals: 24,
            margin: 0.05,
        },
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

async fn spawn_participant(
    row: &ParticipantManifest,
    engine: bool,
) -> Result<(SocketAddr, bool), Box<dyn std::error::Error>> {
    if engine && !cfg!(feature = "core-integration") {
        return Err("le mode --engine exige la feature `core-integration` \
                    (cargo run --features core-integration …). Sans elle, le participant REFUSE \
                    de servir un mock à la place d'un bloc `controller` déclaré — fail-closed."
            .into());
    }
    let local = LocalParticipant::from_manifest_row(row)?;

    // Le gate H1 de boot : le canari adversarial de M1.c tourne AVANT que le participant ne
    // serve. Un `L̂ ≥ 1 − marge` (non-expansivité falsifiée) refuse de démarrer. C'est un vrai
    // garde-fou, pas une formalité — et il coûte les ~18 évaluations de Φ annoncées plus haut.
    if engine {
        local.h1_boot_gate().await.map_err(|e| {
            format!("canari H1 : le participant {:?} REFUSE de servir — {e}", row.id)
        })?;
    }
    // Ce que le participant EST, demandé au participant — pas ce que la ligne DÉCLARE. La
    // distinction n'est pas de la coquetterie : tout ce dispositif existe parce qu'un système peut
    // afficher une chose et en faire une autre, et une démo qui répète la déclaration ne prouve
    // rien de plus que le manifeste.
    let is_core = local.is_efe_core();

    let config = ParticipantConfig {
        id: row.id.clone(),
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
    let id_owned = row.id.clone();
    tokio::spawn(async move {
        if let Err(e) = Server::builder()
            .add_service(MetaParticipantServiceServer::new(svc))
            .serve(addr)
            .await
        {
            eprintln!("participant {id_owned} : serveur arrêté — {e}");
        }
    });

    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return Ok((addr, is_core));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(format!("le participant {} n'a pas démarré sur {addr}", row.id).into())
}

fn build_controller_state(agents: &[Agent]) -> ControllerState {
    let config = ControllerConfig {
        grpc_bind: "127.0.0.1:0".parse().unwrap(),
        metrics_bind: "127.0.0.1:0".parse().unwrap(),
        discovery: DiscoveryMode::StaticLocalhost,
        static_participants: agents
            .iter()
            .map(|a| StaticParticipant { id: a.row.id.clone(), addr: a.addr.to_string() })
            .collect(),
        manifest_path: "/dev/null".into(),
        max_inflight: 4,
        default_max_wall_ms: 120_000,
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
    // Le manifeste porte les MÊMES lignes que celles servies : c'est de là que la liaison tire
    // les identités et les `config_hash` déclarés.
    let manifest = Manifest {
        version: "v0.2.0".into(),
        meta: MetaSpaceManifest { dim: DIM, safety: SafetyManifest::Box { radius: 50.0 } },
        participants: agents.iter().map(|a| a.row.clone()).collect(),
        iteration: IterationManifest {
            max_iters: 100,
            tolerance: 1.0e-4,
            alpha_default: 0.5,
            coupling_gamma: 0.2,
            deterministic: true,
            max_wall_ms: 120_000,
            require_convergence: None,
        },
    };
    ControllerState::new(config, manifest)
}

fn iterate_request() -> IterateRequest {
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
            // Un Φ efe-core coûte ~0,2-1 s ; une ronde en enchaîne des dizaines. Le budget mural
            // est donc large, et le plafond serveur (30 s) le borne de toute façon.
            max_wall_ms: 120_000,
        }),
        initial_states: vec![],
        traceparent: String::new(),
        include_provenance_bundle: true,
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
