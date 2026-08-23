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
    // ── M0 : `--profile-report` — la forme MACHINE du profil servi ──────────────────
    // Un drapeau, jamais une route : le rapport dit ce qui est ÉTEINT (« capacité PII
    // absente »), ce qui est un renseignement pour un attaquant. Un drapeau n'ajoute
    // aucune surface d'écoute ; il calcule, imprime et sort — sans jamais binder.
    if std::env::args().any(|a| a == "--profile-report") {
        let report = efe_swarm_gateway::profile::Report::from_env();
        println!("{}", serde_json::to_string_pretty(&report.to_json())?);
        return Ok(());
    }

    let addr = efe_swarm_gateway::profile::read::gateway_addr();

    // ── Source de clé de provenance — le cœur du durcissement prod ──────────────
    let master_present = efe_swarm_gateway::profile::read::master_present();
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
    if efe_swarm_gateway::profile::read::frontier_enabled() {
        // #2 non-forgeable : le MÊME contrat que le scellement (dry-run derive, PAS master_present —
        // qui n'attrape pas un master < 32 o : il booterait "vert" puis 500 à chaque seal).
        let key_ok = HmacKeySource::EnvHkdf.derive(efe_nano_sdk::TenantId::system()).is_ok();
        // #7 authentifié : la clé publique OIDC frontière (aud=efe-llm) est-elle configurée ?
        #[cfg(feature = "oidc")]
        let auth_ok = efe_swarm_gateway::OidcVerifier::llm_from_env().map(|o| o.is_some()).unwrap_or(false);
        #[cfg(not(feature = "oidc"))]
        let auth_ok = false; // feature oidc absente ⟹ egress tiers sans authentification cryptographique ⟹ refus.
        // #3 durable : intention (postgres compilé + DATABASE_URL) ; connect/migrate re-valident plus bas.
        let store_durable = cfg!(feature = "postgres") && efe_swarm_gateway::profile::read::database_url().is_some();
        // §2.2 : la CAPACITÉ de caviarder — la feature, ou une reconnaissance explicite du risque.
        let accepted_cleartext =
            efe_swarm_gateway::profile::read::accept_cleartext_egress();
        let pii_ok = cfg!(feature = "pii") || accepted_cleartext;
        if let Err(why) = frontier_boot_guard(true, key_ok, auth_ok, store_durable, pii_ok) {
            eprintln!("FATAL (M4 gate production) : {why}");
            std::process::exit(78); // EX_CONFIG (sysexits.h) — config invalide, non-retryable
        }
        // Le message de préflight ne dit QUE ce qui est vrai : si la capacité PII est absente et
        // que l'opérateur l'a reconnu, on le répète ici plutôt que d'imprimer un « OK » lisse.
        if cfg!(feature = "pii") {
            println!(
                "→ préflight frontière OK (provenance non-forgeable + auth OIDC efe-llm + store \
                 durable + capacité PII compilée)."
            );
        } else {
            println!(
                "→ préflight frontière OK (provenance non-forgeable + auth OIDC efe-llm + store \
                 durable) — ATTENTION : capacité PII ABSENTE, reconnue par \
                 EFE_LLM_ACCEPT_CLEARTEXT_EGRESS : les prompts sortent EN CLAIR vers un tiers."
            );
        }
        println!(
            "→ plafond serveur de budget par requête : {} c (EFE_LLM_MAX_BUDGET_CENTS).",
            efe_swarm_gateway::server_budget_ceiling_cents()
        );
    }

    if !master_present {
        eprintln!(
            "AVERTISSEMENT: {MASTER_ENV} absent ou vide → repli sur la clé de TEST PUBLIQUE. \
             La provenance serait FORGEABLE (verify non fiable). NE JAMAIS utiliser en production."
        );
    }

    // **M2** : la posture de capture, lue par le REGISTRE (jamais par un `env::var` local) et
    // posée avant tout câblage. Défaut `Bloquante` ⟹ comportement d'avant M2 à l'octet près ;
    // l'observation est un opt-in déclaré, que le rapport de démarrage annonce.
    let capture_policy = efe_swarm_gateway::capture::CapturePolicy::from_str_or_default(
        efe_swarm_gateway::profile::read::capture_policy().as_deref(),
    );
    let mut state =
        SwarmGatewayState::in_memory().with_keys(keys).with_capture_policy(capture_policy);
    println!("→ clé de provenance : {key_label}");

    // ── M0 : on OBSERVE ce qui se câble, on ne DÉDUIT pas de l'environnement ──────────
    // Le rapport s'imprime en toute fin, alimenté par ce qui a réellement été branché.
    // L'imprimer ICI — avant le store, l'auth, le coordinator — annoncerait des capacités
    // que le processus ne sert pas encore : le défaut exact que M0 existe pour supprimer.
    let mut observed = efe_swarm_gateway::profile::Observed {
        provenance_non_forgeable: master_present,
        // M2 : la posture RÉELLEMENT posée sur l'état, pas celle que la config demande. C'est le
        // même geste que pour tout le reste du rapport servi : on observe le câblage.
        capture_observation: capture_policy.peut_degrader(),
        frontier_intended: efe_swarm_gateway::profile::read::frontier_enabled(),
        allowlist: efe_swarm_gateway::profile::read::tenant_allowlist().is_some(),
        // P1 : poser l'URL est une INTENTION. Le relais vit DANS le coordinator frontière ; tant
        // que celui-ci n'est pas bâti, l'URL est lue par ce rapport et consommée par personne.
        // L'annoncer « SERVIE » ici était le second mensonge de la chaîne du défaut SLM.
        slm_intended: efe_swarm_gateway::profile::read::slm_reformulator_url().is_some(),
        attestation: efe_swarm_gateway::profile::read::bundle_signing_seed().is_some(),
        ..Default::default()
    };

    // ── M5 : le pont vers le registre réglementaire ────────────────────────────────────────
    // Il exige les DEUX interrupteurs. Un dossier de sortie sans cadre ne peut rien dériver, et un
    // cadre sans dossier n'a nulle part où écrire — câbler un demi-pont produirait un rapport qui
    // annonce une capacité que rien ne sert.
    #[cfg(feature = "reg-evidence")]
    {
        let dir = efe_swarm_gateway::profile::read::reg_evidence_dir();
        let cadre = efe_swarm_gateway::reg_pont::cadre_de(
            efe_swarm_gateway::profile::read::reg_framework().as_deref(),
        );
        if let (Some(d), Some(f)) = (dir, cadre) {
            state = state.with_reg_pont(efe_swarm_gateway::reg_pont::RegPont {
                dir: std::path::PathBuf::from(d),
                framework: f,
            });
            observed.reg_pont = true;
        }
    }

    // ── Routeur frontière (Network) + mapping provider→modèle (Inc.4 / Inc.4.b) ──
    if let Some(router_ep) = efe_swarm_gateway::profile::read::router_endpoint() {
        state = state.with_router_endpoint(router_ep);
        observed.router = true;
        println!("→ routeur frontière câblé (Network activé) — clés provider détenues server-side.");
        if let Some(models) = efe_swarm_gateway::profile::read::router_models() {
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
             re-prouvable par verify-mini."
        );

        // ── Fil P (P.3) : la clé d'attestation de racine ──
        // Fail-fast DÉLIBÉRÉ : une graine posée mais illisible signifie qu'un opérateur CROIT
        // signer ses bundles. Servir en silence des bundles non attestés serait un « vert
        // malhonnête » — précisément la classe de défaut que ce fil corrige. On refuse de
        // démarrer plutôt que de laisser croire. Même discipline que la porte de boot M4.
        match efe_swarm_gateway::profile::read::bundle_signing_seed() {
            Some(seed_hex) if !seed_hex.is_empty() => {
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

        // ── Fil P (P.4) : la publication de l'ancre ──
        // Sans elle, un bundle tronqué re-prouve parfaitement : il ne prouve sa cohérence que
        // CONTRE LUI-MÊME. L'ancre est la racine attendue, écrite AILLEURS — et c'est ce
        // « ailleurs » qui fait tout le travail, pas la cryptographie.
        match efe_swarm_gateway::profile::read::anchor_log() {
            Some(path) if !path.is_empty() => {
                let journal = efe_swarm_gateway::FileAnchorSink::new(&path);

                // ── M9 : l'ancre TIERCE, si un horodateur est déclaré ──
                //
                // Sans elle, l'« ailleurs » de l'ancre est NOTRE fichier, remis par nous : il
                // détecte une troncature, il ne dit rien à qui doute de nous. Un horodateur atteste
                // avec SA clé — c'est le pas de « tamper-evident pour un détenteur de clé » à
                // « opposable au détenteur lui-même ».
                //
                // L'horodateur ENVELOPPE le journal, il ne le remplace pas : la ligne de journal est
                // ce qui détecte une troncature, et un horodatage ne fait pas ce travail-là.
                let url = efe_swarm_gateway::profile::read::tsa_url().unwrap_or_default();
                let sink: std::sync::Arc<dyn efe_swarm_gateway::AnchorSink> = if url.trim().is_empty()
                {
                    std::sync::Arc::new(journal)
                } else {
                    let dir = efe_swarm_gateway::profile::read::tsa_dir().unwrap_or_default();
                    if dir.trim().is_empty() {
                        eprintln!(
                            "\n╔═ DÉMARRAGE REFUSÉ ═══════════════════════════════════════════════\n\
                             ║ EFE_SWARM_TSA_URL est posé, mais EFE_SWARM_TSA_DIR manque : les\n\
                             ║ jetons n'auraient nulle part où être déposés.\n\
                             ║ Retomber en silence sur le journal local donnerait à un exploitant\n\
                             ║ qui a DEMANDÉ une ancre tierce une ancre qui est la nôtre.\n\
                             ╚══════════════════════════════════════════════════════════════════\n"
                        );
                        std::process::exit(2);
                    }
                    match provenance_bridge::rfc3161::Rfc3161Sink::nouveau(
                        &url,
                        &dir,
                        Box::new(journal),
                    ) {
                        Ok(s) => std::sync::Arc::new(s),
                        Err(e) => {
                            eprintln!(
                                "\n╔═ DÉMARRAGE REFUSÉ ═══════════════════════════════════════════\n\
                                 ║ EFE_SWARM_TSA_URL : {e}\n\
                                 ╚══════════════════════════════════════════════════════════════\n"
                            );
                            std::process::exit(2);
                        }
                    }
                };

                println!(
                    "→ ancrage ACTIF — {} ; REMETTEZ ce journal à l'auditeur par un canal DISTINCT \
                     du bundle, sans quoi l'ancre ne prouve rien.",
                    efe_swarm_gateway::AnchorSink::describe(sink.as_ref())
                );
                if url.trim().is_empty() {
                    println!(
                        "   ancre DÉCLARÉE (notre propre journal) : elle détecte une troncature, \
                         elle ne prouve rien CONTRE NOUS. Posez EFE_SWARM_TSA_URL pour une ancre \
                         tierce."
                    );
                } else {
                    println!(
                        "   ancre TIERCE : l'horodateur atteste avec SA clé. ⚠ La route de bundle \
                         en DÉPEND désormais — un horodateur muet fait échouer l'export, par le \
                         dessin qui refuse d'annoncer « anchored » sans ancre."
                    );
                }
                state = state.with_anchor_sink(sink);
            }
            _ => println!(
                "→ ancrage INACTIF (aucun EFE_SWARM_ANCHOR_LOG) — les bundles sortiront avec \
                 anchored=false : une TRONCATURE resterait indétectable sans clé."
            ),
        }
    }

    // ── Couche 2 (feature `frontier-llm`) : coordination multi-LLM (Claude/Gemini/GPT) ──
    // Runtime-gated par EFE_LLM_FRONTIER_ENABLED (défaut off) ; les clés provider sont lues
    // server-side par les `from_env` du coordinator. Off, ou aucune clé ⇒ frontière NON
    // servie (aucune route `/v1/llm/*` ne pourra l'utiliser — 501). Aucune clé loggée.
    #[cfg(feature = "frontier-llm")]
    if efe_swarm_gateway::profile::read::frontier_enabled() {
        match efe_swarm_gateway::coordinator_from_env() {
            Some(coord) => {
                state = state.with_coordinator(coord);
                observed.frontier = true;
                // P1 : c'est ICI, et seulement ici, que le relais SLM devient servi — le
                // coordinator qui le porte existe. La localité de son hôte décide ensuite si la
                // garantie de souveraineté tient : câblé hors périmètre, il fonctionne et laisse
                // sortir chaque prompt reformulé, ce que le rapport doit dire en toutes lettres.
                //
                // Limite assumée : on observe que le PORTEUR est bâti, pas que le client HTTP du
                // relais a pu se construire — `slm_clients_from` crie déjà ce cas-là au démarrage,
                // et le remonter jusqu'ici exigerait de traverser `efe-coordinator`.
                if let Some(u) = efe_swarm_gateway::profile::read::slm_reformulator_url() {
                    observed.slm = true;
                    observed.slm_local = provenance_capture::is_local_endpoint(&u);
                }
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
    if let Some(url) = efe_swarm_gateway::profile::read::database_url() {
        let pg = efe_swarm_gateway::PgRunStore::connect(&url).await?;
        pg.migrate().await?;
        state = state.with_store(std::sync::Arc::new(pg));
        // ADR-019 (MEM-3) : la mémoire inter-runs partage le backend durable + la MÊME source
        // de clé tenant que le reste (sinon le memory_root serait scellé sous une autre clé).
        let pg_mem = efe_swarm_gateway::PgMemoryStore::connect(&url, state.keys.clone()).await?;
        pg_mem.migrate().await?;
        state = state.with_memory_store(std::sync::Arc::new(pg_mem));
        observed.durable_store = true;
        println!("→ PgRunStore + PgMemoryStore câblés (persistance durable, write-once).");
    }

    // ── Prod (feature `oidc`) : auth JWT OIDC ES256 (clé PUBLIQUE server-side) ───
    #[cfg(feature = "oidc")]
    if let Some(verifier) = efe_swarm_gateway::OidcVerifier::from_env()? {
        state = state.with_oidc(std::sync::Arc::new(verifier));
        observed.oidc = true;
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
    if let Some(p) = efe_swarm_gateway::profile::read::pii_default_policy() {
        match p.parse::<efe_swarm_gateway::PolicyKind>() {
            Ok(policy) => {
                state = state.with_pii(efe_swarm_gateway::PiiConfig::new(policy));
                observed.pii = true;
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

    // ── M0 : le profil SERVI — après TOUT le câblage, depuis l'état réel ─────────────
    observed.anchor = state.anchor_sink.is_some();
    observed.bundle = state.bundle_export;
    print!(
        "{}",
        efe_swarm_gateway::profile::Report::from_observed(&observed)
            // Les valeurs REFUSÉES se lisent à la source : une fois `Observed` réduit à des
            // booléens, « posé avec une mauvaise valeur » est indistinguable de « pas posé ».
            .with_invalides(efe_swarm_gateway::profile::invalides_from_env())
            .render_human()
    );

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


/// **M4 (§1)** : le préflight go/no-go de la frontière — la **triade** de production. **Pur**
/// (testable sans muter l'env, sur le patron de `key_source_for`). Refuse de démarrer si la
/// frontière est activée sans l'un des trois invariants : non-forgeabilité (clé HMAC dérivable),
/// authentification (OIDC `aud=efe-llm`), durabilité (store Postgres). L'ordre des messages suit
/// la gravité : une clé forgeable rend TOUT le reste sans valeur.
#[cfg(feature = "frontier-llm")]
fn frontier_boot_guard(
    frontier_on: bool,
    key_ok: bool,
    auth_ok: bool,
    store_durable: bool,
    pii_ok: bool,
) -> Result<(), &'static str> {
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
    // **M4 (§2.2) — le quatrième invariant : la CAPACITÉ de caviarder.**
    //
    // Nuance qui décide de la forme du garde : un défaut PII `Off` **par policy** est légitime et
    // documenté. Ce qui ne l'était pas, c'est le **silence** — sans la feature `pii`, le site
    // d'appel disparaissait à la compilation, et l'opérateur lisait « préflight frontière OK »
    // pendant que chaque prompt sortait en clair vers un tiers. Il n'avait même pas le MOYEN de
    // l'activer.
    //
    // On ne force donc pas la caviardage (ce serait changer la politique produit) ; on exige que
    // l'absence de capacité soit **choisie**, jamais subie. Deux remèdes, tous deux explicites :
    // compiler avec `pii`, ou reconnaître le risque par `EFE_LLM_ACCEPT_CLEARTEXT_EGRESS=true`.
    if !pii_ok {
        return Err("EFE_LLM_FRONTIER_ENABLED exige la CAPACITÉ de caviarder (feature `pii`) — sinon \
                    le prompt utilisateur sort EN CLAIR vers un tiers, sans que rien ne le signale. \
                    Deux remèdes : compiler avec `--features pii` (le défaut de policy reste `Off`, \
                    donc rien ne change sans policy), ou reconnaître le risque explicitement avec \
                    EFE_LLM_ACCEPT_CLEARTEXT_EGRESS=true.");
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
        assert!(frontier_boot_guard(false, false, false, false, false).is_ok(), "frontière OFF ⟹ dev toujours OK");
        assert!(frontier_boot_guard(true, false, true, true, true).is_err(), "#2 clé forgeable ⟹ refus");
        assert!(frontier_boot_guard(true, true, false, true, true).is_err(), "#7 auth absente ⟹ refus");
        assert!(frontier_boot_guard(true, true, true, false, true).is_err(), "#3 store volatil ⟹ refus");
        assert!(frontier_boot_guard(true, true, true, true, true).is_ok(), "les quatre invariants ⟹ GO");
    }

    /// La fonction gardée est `#[cfg(feature = "frontier-llm")]` : sans ce gate, la cible **binaire**
    /// du profil par défaut ne compilait plus (`cannot find function frontier_boot_guard`). Le test
    /// jumeau ci-dessus le portait, celui-ci ne l'avait pas.
    #[cfg(feature = "frontier-llm")]
    #[test]
    fn frontier_boot_guard_refuses_a_silent_cleartext_egress() {
        // **M4 §2.2** — le correctif du fail-open PII. Sans la CAPACITÉ de caviarder (feature `pii`
        // absente ET risque non reconnu), la frontière refuse de démarrer : jusqu'ici elle
        // imprimait « préflight frontière OK » pendant que chaque prompt sortait en clair, et
        // l'opérateur n'avait même pas le moyen d'activer la caviardage.
        let err = frontier_boot_guard(true, true, true, true, false)
            .expect_err("capacité PII absente et non reconnue ⟹ refus de démarrer");
        assert!(err.contains("EN CLAIR"), "le message dit ce qui se passerait : {err}");
        assert!(err.contains("EFE_LLM_ACCEPT_CLEARTEXT_EGRESS"), "…et comment le reconnaître");
        assert!(err.contains("pii"), "…et comment le corriger vraiment");

        // Le défaut PII `Off` PAR POLICY reste légitime : on n'exige pas la caviardage, seulement
        // que son absence soit CHOISIE. Les deux remèdes débloquent.
        assert!(frontier_boot_guard(true, true, true, true, true).is_ok());
    }
}
