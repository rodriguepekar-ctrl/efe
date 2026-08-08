// SPDX-License-Identifier: UNLICENSED
//! **Binaire production** de la passerelle d'essaim souveraine (`efe-swarm-gateway`).
//!
//! ```bash
//! # prod (signe la provenance avec le master server-side) :
//! EFE_NANO_AUDIT_HMAC_MASTER=… DATABASE_URL=… cargo run -p efe-swarm-gateway \
//!     --features "postgres oidc pii" --bin efe-swarm-gateway
//! ```
//!
//! Différences vs l'exemple `serve_standalone` (dev, en mémoire, `test_key`) :
//! - **Clé de provenance** : sélectionne [`HmacKeySource::EnvHkdf`] (master HKDF lu
//!   **server-side** via `EFE_NANO_AUDIT_HMAC_MASTER`) dès qu'il est **présent** ; sinon repli
//!   `test_key()` **dev** avec un AVERTISSEMENT bruyant — car le master de test est **public**
//!   (dans la source), donc la re-preuve (`verify`, le *moat*) serait **forgeable**. C'est le
//!   trou que ce binaire ferme : le chart Helm monte un Secret HKDF que l'exemple n'utilisait pas.
//! - **Bind** par défaut `0.0.0.0:8080` (dans le pod) au lieu de `127.0.0.1`.
//! - **Arrêt gracieux** (SIGTERM k8s / Ctrl-C) → rollouts propres, connexions drainées.
//! - Câble store (`postgres`) / auth (`oidc`) / PII (`pii`) / routeur Network **depuis l'env**.
//!
//! **Aucune clé manipulée ici** : on teste seulement la **présence** du master (jamais sa
//! valeur, jamais loggée) pour choisir la source ; `EnvHkdf::derive` lit l'env **au moment de
//! signer**, server-side. Le routeur Network détient les clés provider (jamais ce binaire).

use efe_swarm_gateway::{router, SwarmGatewayState};
use efe_swarm_provenance::{HmacKeySource, MASTER_ENV};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = std::env::var("EFE_SWARM_GATEWAY_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    // ── Source de clé de provenance — le cœur du durcissement prod ──────────────
    let master_present = std::env::var(MASTER_ENV).map(|v| !v.trim().is_empty()).unwrap_or(false);
    let (keys, key_label) = key_source_for(master_present);

    // GAP-SHORT-MASTER : un master FOURNI mais invalide (< 32 o, non-blanc) booterait "vert" en
    // EnvHkdf puis échouerait 500 à CHAQUE scellement de provenance. On refuse tout de suite (tous
    // chemins, pas seulement la frontière) via le MÊME contrat que le scellement (dry-run derive).
    if master_present && HmacKeySource::EnvHkdf.derive(efe_nano_sdk::TenantId::system()).is_err() {
        eprintln!(
            "FATAL : {MASTER_ENV} fourni mais invalide (≥ 32 octets requis) — refus de démarrer \
             (provenance non-scellable, chaque run échouerait 500)."
        );
        std::process::exit(78);
    }

    // ── M4 : préflight go/no-go de la frontière — la TRIADE (non-forgeable + authentifiée + durable) ──
    // La frontière est un egress PAYANT vers un TIERS : chaque invariant manquant REFUSE de démarrer,
    // jamais ne dégrade en silence. Calculé depuis l'INTENTION env, AVANT tout câblage d'état (with_keys,
    // with_coordinator) : aucun état forgeable/non-authentifié/volatil n'est même assemblé.
    #[cfg(feature = "frontier-llm")]
    if frontier_flag_enabled() {
        // #2 non-forgeable : le MÊME contrat que le scellement (dry-run derive, PAS master_present —
        // qui n'attrape pas un master < 32 o : il booterait "vert" puis 500 à chaque seal).
        let key_ok = HmacKeySource::EnvHkdf.derive(efe_nano_sdk::TenantId::system()).is_ok();
        // #7 authentifié : la clé publique OIDC frontière (aud=efe-llm) est-elle configurée ?
        #[cfg(feature = "oidc")]
        let auth_ok = efe_swarm_gateway::OidcVerifier::llm_from_env().map(|o| o.is_some()).unwrap_or(false);
        #[cfg(not(feature = "oidc"))]
        let auth_ok = false; // feature oidc absente ⟹ egress tiers sans authentification cryptographique ⟹ refus.
        // #3 durable : intention (postgres compilé + DATABASE_URL) ; connect/migrate re-valident plus bas.
        let store_durable = cfg!(feature = "postgres") && std::env::var("DATABASE_URL").is_ok();
        if let Err(why) = frontier_boot_guard(true, key_ok, auth_ok, store_durable) {
            eprintln!("FATAL (M4 gate production) : {why}");
            std::process::exit(78); // EX_CONFIG (sysexits.h) — config invalide, non-retryable
        }
        println!("→ préflight frontière OK (provenance non-forgeable + auth OIDC efe-llm + store durable).");
    }

    if !master_present {
        eprintln!(
            "AVERTISSEMENT: {MASTER_ENV} absent ou vide → repli sur la clé de TEST PUBLIQUE. \
             La provenance serait FORGEABLE (verify non fiable). NE JAMAIS utiliser en production."
        );
    }

    let mut state = SwarmGatewayState::in_memory().with_keys(keys);
    println!("→ clé de provenance : {key_label}");

    // ── Routeur frontière (Network) + mapping provider→modèle (Inc.4 / Inc.4.b) ──
    if let Ok(router_ep) = std::env::var("EFE_SWARM_ROUTER_ENDPOINT") {
        state = state.with_router_endpoint(router_ep);
        println!("→ routeur frontière câblé (Network activé) — clés provider détenues server-side.");
        if let Ok(models) = std::env::var("EFE_SWARM_ROUTER_MODELS") {
            state = state.with_router_models(efe_swarm_gateway::RouterMap::from_env_str(&models));
            println!("→ mapping provider→modèle surchargé (EFE_SWARM_ROUTER_MODELS).");
        }
    }

    // ── Fil P (P.2) : l'export du bundle de provenance (pile B) ──
    // OFF par défaut : sans opt-in, la route n'est PAS montée (l'URI n'existe pas), donc la
    // surface servie est byte-identique à l'avant-P.2. Le bundle n'expose aucune clé — seulement
    // des empreintes ; la clé tenant de la pile B voyage hors bande vers l'auditeur.
    if efe_swarm_gateway::bundle_export_from_env() {
        state = state.with_bundle_export(true);
        println!(
            "→ export du bundle de provenance ACTIVÉ (GET /v1/swarm/bundle/:id, rôle auditor) — \
             re-prouvable par verify-mini ; racine NON encore ancrée hors bundle (P.4)."
        );

        // ── Fil P (P.3) : la clé d'attestation de racine ──
        // Fail-fast DÉLIBÉRÉ : une graine posée mais illisible signifie qu'un opérateur CROIT
        // signer ses bundles. Servir en silence des bundles non attestés serait un « vert
        // malhonnête » — précisément la classe de défaut que ce fil corrige. On refuse de
        // démarrer plutôt que de laisser croire. Même discipline que la porte de boot M4.
        match std::env::var("EFE_SWARM_BUNDLE_SIGNING_SEED") {
            Ok(seed_hex) if !seed_hex.is_empty() => {
                match efe_swarm_gateway::BundleSigner::from_seed_hex(&seed_hex) {
                    Ok(signer) => {
                        // On imprime l'EMPREINTE et la clé PUBLIQUE — jamais la graine. La publique
                        // est destinée à être épinglée hors bande par l'auditeur ; sans cet
                        // épinglage, la signature ne prouve rien.
                        println!(
                            "→ attestation de racine ACTIVE — key_id={} ; clé PUBLIQUE à publier \
                             hors bande : {}",
                            signer.key_id(),
                            signer.public_key_hex()
                        );
                        state = state.with_bundle_signer(signer);
                    }
                    Err(e) => {
                        eprintln!(
                            "ERREUR FATALE: EFE_SWARM_BUNDLE_SIGNING_SEED illisible ({e}). \
                             Attendu 64 caractères hexadécimaux (graine ed25519 de 32 octets). \
                             Refus de démarrer : servir des bundles NON attestés alors qu'une clé \
                             est configurée ferait croire à une signature inexistante."
                        );
                        std::process::exit(1);
                    }
                }
            }
            _ => println!(
                "→ attestation de racine INACTIVE (aucune graine) — les bundles sortiront SANS \
                 champ d'attestation."
            ),
        }
    }

    // ── Couche 2 (feature `frontier-llm`) : coordination multi-LLM (Claude/Gemini/GPT) ──
    // Runtime-gated par EFE_LLM_FRONTIER_ENABLED (défaut off) ; les clés provider sont lues
    // server-side par les `from_env` du coordinator. Off, ou aucune clé ⇒ frontière NON
    // servie (aucune route `/v1/llm/*` ne pourra l'utiliser — 501). Aucune clé loggée.
    #[cfg(feature = "frontier-llm")]
    if frontier_flag_enabled() {
        match efe_swarm_gateway::coordinator_from_env() {
            Some(coord) => {
                state = state.with_coordinator(coord);
                println!("→ coordination frontière (Couche 2) activée — providers câblés depuis l'env.");
            }
            None => eprintln!(
                "AVERTISSEMENT: EFE_LLM_FRONTIER_ENABLED=true mais aucune clé provider \
                 (ANTHROPIC_API_KEY / OPENAI_API_KEY / GOOGLE_AI_KEY) — frontière NON servie."
            ),
        }
    }

    // ── Prod (feature `postgres`) : persistance durable write-once des records ───
    #[cfg(feature = "postgres")]
    if let Ok(url) = std::env::var("DATABASE_URL") {
        let pg = efe_swarm_gateway::PgRunStore::connect(&url).await?;
        pg.migrate().await?;
        state = state.with_store(std::sync::Arc::new(pg));
        // ADR-019 (MEM-3) : la mémoire inter-runs partage le backend durable + la MÊME source
        // de clé tenant que le reste (sinon le memory_root serait scellé sous une autre clé).
        let pg_mem = efe_swarm_gateway::PgMemoryStore::connect(&url, state.keys.clone()).await?;
        pg_mem.migrate().await?;
        state = state.with_memory_store(std::sync::Arc::new(pg_mem));
        println!("→ PgRunStore + PgMemoryStore câblés (persistance durable, write-once).");
    }

    // ── Prod (feature `oidc`) : auth JWT OIDC ES256 (clé PUBLIQUE server-side) ───
    #[cfg(feature = "oidc")]
    if let Some(verifier) = efe_swarm_gateway::OidcVerifier::from_env()? {
        state = state.with_oidc(std::sync::Arc::new(verifier));
        println!("→ auth OIDC activée (JWT ES256, clé publique server-side).");
    }

    // ── Couche 2 (M3, feature `oidc`) : auth DÉDIÉE du chemin frontière (aud=efe-llm) ───
    // Séparée de l'auth essaim : un jeton souverain ne déverrouille pas l'egress tiers coûteux.
    #[cfg(feature = "oidc")]
    if let Some(verifier) = efe_swarm_gateway::OidcVerifier::llm_from_env()? {
        state = state.with_oidc_llm(std::sync::Arc::new(verifier));
        println!("→ auth OIDC frontière activée (JWT aud=efe-llm, clé publique server-side).");
    }

    // ── G9 (feature `pii`) : politique PII par défaut depuis l'env (détection locale) ──
    #[cfg(feature = "pii")]
    if let Ok(p) = std::env::var("EFE_PII_DEFAULT_POLICY") {
        match p.parse::<efe_swarm_gateway::PolicyKind>() {
            Ok(policy) => {
                state = state.with_pii(efe_swarm_gateway::PiiConfig::new(policy));
                println!("→ PII opt-in : politique par défaut = {}", policy.as_str());
            }
            // GAP-PII : une valeur invalide (typo 'reduct') ne doit PAS être ignorée en silence —
            // sinon redact_for_egress devient no-op et le prompt sort EN CLAIR vers le tiers.
            Err(_) => eprintln!(
                "AVERTISSEMENT: EFE_PII_DEFAULT_POLICY='{p}' invalide (attendu off|detect|redact|strict) — \
                 PII par défaut OFF : les prompts frontière sortiront EN CLAIR vers les providers."
            ),
        }
    }

    let app = router(state);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("efe-swarm-gateway (prod) en écoute sur http://{addr}");
    println!("  POST http://{addr}/v1/swarm/run   ·   GET …/verify/:id (le moat)   ·   GET /metrics");

    axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()).await?;
    println!("arrêt gracieux terminé — connexions drainées.");
    Ok(())
}

/// Décision **pure** de la source de clé selon la présence (non sa valeur) du master.
/// Extraite pour être testable sans muter l'environnement du process.
fn key_source_for(master_present: bool) -> (HmacKeySource, &'static str) {
    if master_present {
        (HmacKeySource::EnvHkdf, "EnvHkdf (master server-side — PRODUCTION)")
    } else {
        (HmacKeySource::test_key(), "test_key() (DEV — provenance NON sécurisée)")
    }
}

/// **M4 (§1.4)** : le flag d'activation de la frontière — **source UNIQUE** de vérité,
/// réutilisée par le préflight ET le câblage du coordinator (interdit tout drift). Égalité
/// stricte `"true"` (fail-closed) ; une valeur non-vide ≠ `"true"` émet un avertissement ops
/// (activation silencieusement ignorée, footgun `1`/`TRUE`/`yes`).
fn frontier_flag_enabled() -> bool {
    match std::env::var("EFE_LLM_FRONTIER_ENABLED") {
        Ok(v) if v == "true" => true,
        Ok(v) if !v.trim().is_empty() => {
            eprintln!("AVERTISSEMENT: EFE_LLM_FRONTIER_ENABLED='{v}' ≠ 'true' ⟹ frontière NON activée (attendu exactement 'true').");
            false
        }
        _ => false,
    }
}

/// **M4 (§1)** : le préflight go/no-go de la frontière — la **triade** de production. **Pur**
/// (testable sans muter l'env, sur le patron de `key_source_for`). Refuse de démarrer si la
/// frontière est activée sans l'un des trois invariants : non-forgeabilité (clé HMAC dérivable),
/// authentification (OIDC `aud=efe-llm`), durabilité (store Postgres). L'ordre des messages suit
/// la gravité : une clé forgeable rend TOUT le reste sans valeur.
#[cfg(feature = "frontier-llm")]
fn frontier_boot_guard(frontier_on: bool, key_ok: bool, auth_ok: bool, store_durable: bool) -> Result<(), &'static str> {
    if !frontier_on {
        return Ok(()); // frontière OFF ⟹ dev/essaim souverain intact (InMemory + test_key légitimes)
    }
    if !key_ok {
        return Err("EFE_LLM_FRONTIER_ENABLED exige EFE_NANO_AUDIT_HMAC_MASTER valide (≥ 32 o) — \
                    sinon la provenance est scellée sous test_key() PUBLIQUE ⟹ reçus R4 FORGEABLES.");
    }
    if !auth_ok {
        return Err("EFE_LLM_FRONTIER_ENABLED exige l'auth OIDC frontière (feature `oidc` + clé publique, \
                    aud=efe-llm) — sinon egress tiers PAYANT sans authentification (tenant/rôles en clair).");
    }
    if !store_durable {
        return Err("EFE_LLM_FRONTIER_ENABLED exige un store DURABLE (feature `postgres` + DATABASE_URL) — \
                    sinon les reçus R4 (la preuve) sont perdus à chaque rollout.");
    }
    Ok(())
}

/// Attend **SIGTERM** (rollout k8s) ou **Ctrl-C** (local) → arrêt gracieux.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    println!("signal d'arrêt reçu — fermeture en cours…");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_present_selects_envhkdf() {
        let (src, label) = key_source_for(true);
        assert!(matches!(src, HmacKeySource::EnvHkdf), "master présent → EnvHkdf (prod)");
        assert!(label.contains("PRODUCTION"));
    }

    #[test]
    fn master_absent_falls_back_to_test_key() {
        let (src, label) = key_source_for(false);
        assert!(matches!(src, HmacKeySource::StaticMaster(_)), "master absent → repli test_key");
        assert!(label.contains("DEV"));
    }

    /// **M4 (§1.5)** : la table de vérité du préflight — la triade non-forgeable/authentifié/durable.
    #[cfg(feature = "frontier-llm")]
    #[test]
    fn frontier_boot_guard_enforces_the_triad() {
        assert!(frontier_boot_guard(false, false, false, false).is_ok(), "frontière OFF ⟹ dev toujours OK");
        assert!(frontier_boot_guard(true, false, true, true).is_err(), "#2 clé forgeable ⟹ refus");
        assert!(frontier_boot_guard(true, true, false, true).is_err(), "#7 auth absente ⟹ refus");
        assert!(frontier_boot_guard(true, true, true, false).is_err(), "#3 store volatil ⟹ refus");
        assert!(frontier_boot_guard(true, true, true, true).is_ok(), "triade complète ⟹ GO");
    }
}
