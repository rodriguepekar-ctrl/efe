// SPDX-License-Identifier: UNLICENSED
//! # `local_receipt_demo` — le reçu servi, **sans une seule clé**.
//!
//! ```text
//! bash scripts/demo-local-receipt.sh
//! ```
//!
//! ## Ce que cette démo prouve
//!
//! Qu'on peut faire tourner la passerelle **entièrement en local** — aucun compte, aucune carte,
//! aucune clé d'API, aucun octet qui sort — et en obtenir un reçu qu'un tiers re-prouve **hors
//! ligne** avec un binaire dont l'arbre de dépendances est littéralement vide.
//!
//! C'est le seul argument de souveraineté qu'un acheteur ne peut pas contester : il ne le lit pas
//! sur une plaquette, il l'exécute sur sa propre machine.
//!
//! ## Les quatre actes
//!
//! 1. **Le reçu servi.** `GET /v1/swarm/verify/:id` re-prouve la chaîne d'essaim, la chaîne
//!    forensique N17 et le pont qui les lie.
//! 2. **La falsification, localisée.** On réécrit le **modèle déclaré** d'un appel — sans la clé.
//!    La chaîne d'essaim reste intègre ; la chaîne forensique, elle, casse, et le verdict dit
//!    **quel appel** a été muté. Un contrôle qui ne peut pas échouer ne prouve rien.
//! 3. **Le bundle.** `GET /v1/swarm/bundle/:id` rend la vue que `verify-mini` sait lire. La route
//!    ne produit aucune information neuve : elle change le *vérifieur*, pas la vérité.
//! 4. **L'ancre.** Le préfixe d'une chaîne valide est lui-même valide : une troncature se
//!    re-prouve **parfaitement**. Seule la racine publiée hors bundle la démasque — et ce
//!    contrôle-là ne demande **aucune clé**.
//!
//! ## Ce que la démo ne prouve pas, et qu'il faut dire
//!
//! Le run est **synthétique** : il déclare un appel `ollama/<modèle>/local` sans appeler de modèle.
//! C'est le *mécanisme* du reçu qui est démontré, pas la substance de ce qu'il atteste. Le chemin
//! avec un vrai modèle local existe et est exercé — `selfhost_e2e_real`, qui exige
//! `ollama serve` — mais il demande un modèle de plusieurs gigaoctets ; le mettre dans le chemin
//! d'une première démonstration la rendrait indémontrable.
//!
//! La clé est une clé de **développement**, publique : le reçu produit ici est **forgeable**. En
//! production elle est dérivée d'un master côté serveur que personne d'autre ne détient.
//!
//! Et le reçu atteste l'**intégrité** de ce qui est enregistré — jamais la **correction** de la
//! décision.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use efe_swarm_gateway::test_support::synthetic_mode_c_run;
use efe_swarm_gateway::{router, InMemoryRunStore, RunStore, SwarmGatewayState};
use efe_swarm_sdk::HmacKeySource;
use http_body_util::BodyExt;
use tower::ServiceExt;
use uuid::Uuid;

/// Le dossier de sortie : tout ce que la démo écrit est ici, et nulle part ailleurs.
const OUT_DIR: &str = "demo-bundles/local-receipt";
/// Le modèle *déclaré* par le run synthétique — celui qu'un vrai `ollama serve` servirait.
const MODEL: &str = "gemma3:12b";

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::path::Path::new(OUT_DIR);
    std::fs::create_dir_all(out)?;
    let anchor_path = out.join("anchors.log");
    let _ = std::fs::remove_file(&anchor_path); // démo : on repart d'un journal propre.

    println!("┌─ Scella · le reçu servi, SANS CLÉ");
    println!("│  aucun compte, aucune carte, aucune clé d'API, aucun octet qui sort");
    println!("│  clé de DÉMONSTRATION (publique) ⟹ ce reçu est FORGEABLE — il démontre le mécanisme");
    println!("│");

    // ── L'état servi : mémoire, clé de démo, export de bundle, journal d'ancres ───────────────
    let store = Arc::new(InMemoryRunStore::default());
    let state = SwarmGatewayState::in_memory()
        .with_store(store.clone() as Arc<dyn RunStore>)
        .with_keys(HmacKeySource::test_key())
        .with_bundle_export(true)
        .with_anchor_sink(Arc::new(provenance_bridge::FileAnchorSink::new(&anchor_path)));
    let app = router(state);

    // Deux runs issus du MÊME fixture : l'un intact, l'autre falsifié. Les fabriquer ensemble
    // garantit qu'ils ne diffèrent que par la mutation — sinon la démo comparerait deux choses.
    let (_, honest) = synthetic_mode_c_run(MODEL);
    let (_, mut forged) = synthetic_mode_c_run(MODEL);
    forged.llm_records[0].model_identity =
        efe_nano_sdk::ModelIdentity::hosted("ollama", "MENTEUR:70b", "local");

    let id_ok = Uuid::new_v4();
    let id_ko = Uuid::new_v4();
    store.put_stored(id_ok, honest).await?;
    store.put_stored(id_ko, forged).await?;

    // ── Acte 1 — le reçu servi ────────────────────────────────────────────────────────────────
    println!("  ── (a) le reçu servi : la passerelle re-prouve ce qu'elle a enregistré ──");
    let v = get_json(&app, &format!("/v1/swarm/verify/{id_ok}")).await?;
    println!("  verdict GLOBAL      : {}", verdict(&v["verified"]));
    println!("  chaîne d'essaim     : {}", verdict(&v["chain_intact"]));
    println!("  chaîne forensique   : {} ({} record(s))", verdict(&v["n17"]["chain_intact"]), v["n17"]["records"]);
    println!("  pont essaim↔appel   : {}", verdict(&v["n17"]["bridge_consistent"]));
    println!("  racine              : {}", v["root"].as_str().unwrap_or("—"));
    if v["verified"] != serde_json::json!(true) {
        return Err("le run intact aurait dû se re-prouver".into());
    }
    println!();

    // ── Acte 2 — la falsification, localisée ─────────────────────────────────────────────────
    println!("  ── (b) un attaquant réécrit le MODÈLE déclaré — sans la clé ──");
    println!("  il remplace « {MODEL} » par « MENTEUR:70b » dans l'enregistrement persisté.");
    let f = get_json(&app, &format!("/v1/swarm/verify/{id_ko}")).await?;
    println!("  verdict GLOBAL      : {}", verdict(&f["verified"]));
    println!("  chaîne d'essaim     : {}  ← elle, reste intègre", verdict(&f["chain_intact"]));
    println!("  chaîne forensique   : {}  ← c'est elle qui mord", verdict(&f["n17"]["chain_intact"]));
    if let Some(b) = f["n17"]["first_break"].as_str() {
        println!("  faute LOCALISÉE     : {b}");
    }
    if f["verified"] != serde_json::json!(false) {
        return Err("ÉCHEC : une forensique falsifiée a été acceptée — le contrôle ne mord pas".into());
    }
    println!("  ↑ le reçu ne dit pas seulement « quelque chose a bougé » : il dit QUOI.");
    println!();

    // ── Acte 3 — le bundle, pour un vérifieur sans dépendance ────────────────────────────────
    println!("  ── (c) le bundle : changer de vérifieur, pas de vérité ──");
    let b = get_json(&app, &format!("/v1/swarm/bundle/{id_ok}")).await?;
    let jsonl = b["bundle_jsonl"].as_str().ok_or("bundle_jsonl absent")?;
    let root = b["manifest_root"].as_str().ok_or("manifest_root absent")?;
    let nodes = b["node_count"].as_u64().unwrap_or(0);
    std::fs::write(out.join("records.sealed.jsonl"), jsonl)?;
    println!("  nœuds               : {nodes}");
    println!("  manifest-root       : {root}");
    println!("  ancré hors bundle   : {}", b["anchored"]);
    println!("  écrit               : {}/records.sealed.jsonl", OUT_DIR);

    // Le piège de l'acte 4 : un préfixe. On coupe le dernier nœud et on n'y touche plus.
    let lines: Vec<&str> = jsonl.lines().collect();
    if lines.len() >= 2 {
        let short = lines[..lines.len() - 1].join("\n") + "\n";
        std::fs::write(out.join("records.TRUNCATED.jsonl"), short)?;
        println!("  écrit               : {}/records.TRUNCATED.jsonl  ({} nœuds au lieu de {})",
                 OUT_DIR, lines.len() - 1, lines.len());
    } else {
        println!("  ⚠ bundle trop court pour illustrer la troncature ({} nœud)", lines.len());
    }
    println!();

    // ── Acte 4 — l'ancre ─────────────────────────────────────────────────────────────────────
    println!("  ── (d) l'ancre : ce que la clé seule NE VOIT PAS ──");
    match std::fs::read_to_string(&anchor_path) {
        Ok(j) if !j.trim().is_empty() => {
            println!("  journal d'ancres    : {}  ({} ligne(s))", anchor_path.display(), j.lines().count());
            println!("  ↑ publié HORS du bundle, au moment de l'export. C'est le seul geste qui rende");
            println!("    une troncature détectable par quelqu'un ne détenant AUCUNE clé — à condition");
            println!("    que l'auditeur le reçoive par un canal DISTINCT de celui du bundle.");
        }
        _ => println!("  (aucune ancre publiée — le support n'était pas configuré)"),
    }

    println!();
    println!("└─ deux contrôles, et un seul demande la clé :");
    println!("   · SANS CLÉ — le nombre de nœuds annoncé par l'ancre contredit un bundle tronqué.");
    println!("     C'est un comptage, pas une vérification cryptographique.");
    println!("   · AVEC la clé du tenant, hors bande — la re-preuve complète :");
    println!("       verify-mini {OUT_DIR}/records.sealed.jsonl \\");
    println!("         --key-hex <clé tenant> --root {root}");
    println!("     La clé n'est PAS écrite ici : en production elle vient du responsable de");
    println!("     traitement, pas d'un dossier de démonstration.");
    Ok(())
}

/// `GET` sur le routeur **réellement servi**, et le corps en JSON.
async fn get_json(
    app: &axum::Router,
    uri: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty())?)
        .await?;
    let status = resp.status();
    let bytes = resp.into_body().collect().await?.to_bytes();
    if status != StatusCode::OK {
        return Err(format!("{uri} → HTTP {status} : {}", String::from_utf8_lossy(&bytes)).into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}

/// Rend un booléen JSON lisible sans dépendre d'une bibliothèque de couleurs.
fn verdict(v: &serde_json::Value) -> &'static str {
    match v.as_bool() {
        Some(true) => "OK",
        Some(false) => "REFUSÉ",
        None => "—",
    }
}
