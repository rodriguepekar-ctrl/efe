// SPDX-License-Identifier: UNLICENSED
//! **Les destinations d'egress sont configurables — et le reçu dit où les données sont parties.**
//!
//! ## Le grief
//!
//! Les URL de sortie des **trois** providers frontière étaient des **constantes compilées**. Trois
//! conséquences, dont la troisième est celle qui a rendu ce fichier nécessaire :
//!
//! 1. impossible de pointer un **faux fournisseur local**, donc l'aller-retour frontière n'a jamais
//!    été joué — c'est la première ligne de ce que `scripts/p1-check.sh` déclarait ne pas prouver ;
//! 2. impossible de pointer un **Azure OpenAI**, un serveur compatible auto-hébergé, un mandataire
//!    d'entreprise ou un point de terminaison régional : une passerelle qui se dit souveraine et
//!    dont les trois destinations sont gravées vers les États-Unis ne l'est qu'à moitié ;
//! 3. et si l'on rendait les URL configurables **en oubliant `endpoint()`**, le reçu scellé
//!    continuerait d'annoncer le point de terminaison public pendant que les données partiraient
//!    ailleurs. Ce serait un **faux** dans un artefact conçu pour être opposable. C'est le risque
//!    que ce fichier existe pour tenir.
//!
//! ## Deux sémantiques, et pourquoi elles diffèrent
//!
//! Anthropic n'a **qu'un** chemin (`/v1/messages`) : son bouton est l'URL entière — `with_endpoint`.
//! OpenAI en a **deux** (`/chat/completions` et `/models` pour la sonde) et Gemini **trois**
//! (`:generateContent`, `:streamGenerateContent`, la base nue) : leur bouton est la **base** —
//! `with_base_url`. Deux noms plutôt qu'un seul recouvrant deux comportements.
//!
//! ## Ce qu'il ne prouve pas
//!
//! Rien ici n'ouvre de socket. Ces tests vérifient la **déclaration** de la destination et le refus
//! des formes dangereuses. Que l'octet parte bien là est l'affaire de `aller_retour_frontiere.rs`.

// Ces providers vivent sous leurs features, absentes du défaut du crate (`default = []`). Sans
// cette ligne le fichier ne compilerait pas sous `cargo test --workspace` ; avec elle il est
// simplement vide — ce qui serait pire si personne ne le jouait jamais. C'est `scripts/p1-check.sh`
// qui le joue, et qui ÉCHOUE si zéro test a tourné. Le précédent est sa propre section 6 :
// « le critère de M3, que personne ne jouait ».
#![cfg(all(feature = "anthropic", feature = "openai", feature = "gemini", feature = "local-slm"))]

use efe_coordinator::llm::providers::auth::ApiKey;
use efe_coordinator::{ClaudeProvider, GeminiProvider, GptProvider, LlmProvider, LocalSlmProvider};

const CLAUDE_DEFAUT: &str = "https://api.anthropic.com/v1/messages";
const GPT_DEFAUT: &str = "https://api.openai.com/v1/chat/completions";
const GEMINI_DEFAUT: &str = "https://generativelanguage.googleapis.com/v1beta/models";

fn claude() -> ClaudeProvider {
    ClaudeProvider::new(ApiKey::new("sk-ant-test-0123456789abcdef"), "claude-test")
}
fn gpt() -> GptProvider {
    GptProvider::new(ApiKey::new("sk-test-0123456789abcdef"), "gpt-test")
}
fn gemini() -> GeminiProvider {
    GeminiProvider::new(ApiKey::new("cle-de-test-gemini-non-secrete"), "gemini-test")
}
/// Le QUATRIÈME. Il n'a pas de `with_*` : son endpoint se donne à la construction, et c'est
/// `new` qui valide — donc ici le constructeur rend un `Result`, là où les trois autres
/// construisent d'abord et valident ensuite.
fn slm(url: &str) -> efe_coordinator::error::CoordResult<LocalSlmProvider> {
    LocalSlmProvider::new("local-test", url, "modele-test")
}
/// Le refus, en texte. `unwrap_err` exigerait `Debug` sur `LocalSlmProvider` — on n'ajoute pas un
/// `derive` à un type de production pour la commodité d'un test.
fn refus_slm(url: &str) -> String {
    match slm(url) {
        Ok(_) => panic!("« {url} » aurait dû être refusé"),
        Err(e) => format!("{e}"),
    }
}

// ── 1 · Le défaut ne bouge pour personne ────────────────────────────────────────────────────────

#[test]
fn sans_rien_faire_les_trois_destinations_sont_inchangees() {
    assert_eq!(claude().endpoint().as_deref(), Some(CLAUDE_DEFAUT));
    assert_eq!(gpt().endpoint().as_deref(), Some(GPT_DEFAUT));
    assert_eq!(gemini().endpoint().as_deref(), Some(GEMINI_DEFAUT));
}

// ── 2 · LE TEST QUI COMPTE : le reçu annonce la destination réelle ──────────────────────────────

/// `endpoint()` alimente `ProvenanceProvider`, qui **scelle** la destination dans le reçu. S'il
/// rendait la constante, le reçu nommerait le fournisseur public pendant que les données
/// partiraient ailleurs — un faux, dans l'artefact même qui doit être opposable.
#[test]
fn le_recu_annonce_la_destination_reelle_pas_la_constante() {
    assert_eq!(
        claude().with_endpoint("http://127.0.0.1:8099/v1/messages").unwrap().endpoint().as_deref(),
        Some("http://127.0.0.1:8099/v1/messages")
    );
    assert_eq!(
        gpt().with_base_url("http://127.0.0.1:8099/v1").unwrap().endpoint().as_deref(),
        Some("http://127.0.0.1:8099/v1/chat/completions"),
        "GPT rend l'URL de COMPLÉTION — la destination réelle des prompts —, pas la base nue"
    );
    assert_eq!(
        gemini().with_base_url("http://127.0.0.1:8099/v1beta/models").unwrap().endpoint().as_deref(),
        Some("http://127.0.0.1:8099/v1beta/models")
    );
}

// ── 3 · Ce que la configurabilité achète ────────────────────────────────────────────────────────

#[test]
fn azure_un_mandataire_ou_un_point_de_terminaison_regional_sont_acceptes() {
    assert!(claude().with_endpoint("https://eu.exemple.test/v1/messages").is_ok());
    assert!(gpt().with_base_url("https://mon-azure.openai.azure.test/openai/v1").is_ok());
    assert!(gemini().with_base_url("https://mandataire.interne:8443/v1beta/models").is_ok());
}

/// Une base copiée-collée finit souvent par un `/`. L'accepter puis composer donnerait
/// `…/v1//chat/completions` — une URL qui marche parfois, et qui se retrouve **telle quelle dans le
/// reçu**. On la normalise, et on le vérifie.
#[test]
fn une_barre_finale_ne_se_retrouve_pas_dans_le_recu() {
    assert_eq!(
        gpt().with_base_url("https://exemple.test/v1/").unwrap().endpoint().as_deref(),
        Some("https://exemple.test/v1/chat/completions")
    );
    assert_eq!(
        gemini().with_base_url("https://exemple.test/v1beta/models/").unwrap().endpoint().as_deref(),
        Some("https://exemple.test/v1beta/models")
    );
}

// ── 4 · Les refus ───────────────────────────────────────────────────────────────────────────────

#[test]
fn http_distant_est_refuse_par_les_trois() {
    for m in [
        format!("{}", claude().with_endpoint("http://api.exemple.test/v1/m").unwrap_err()),
        format!("{}", gpt().with_base_url("http://api.exemple.test/v1").unwrap_err()),
        format!("{}", gemini().with_base_url("http://api.exemple.test/v1beta").unwrap_err()),
    ] {
        assert!(m.contains("machine locale"), "les prompts partiraient en clair : {m}");
    }
}

/// La forme exacte qui a coûté 875 cas à M1.b. Elle ne doit franchir la porte loopback d'aucun des
/// trois — c'est précisément ce qu'un second analyseur écrit à la main aurait laissé passer.
#[test]
fn la_ruse_du_userinfo_ne_franchit_la_porte_locale_daucun_des_trois() {
    for m in [
        format!("{}", claude().with_endpoint("http://127.0.0.1:9@evil.test/v1/m").unwrap_err()),
        format!("{}", gpt().with_base_url("http://localhost@evil.test/v1").unwrap_err()),
        format!("{}", gemini().with_base_url("http://127.0.0.1:9@evil.test/v1beta").unwrap_err()),
    ] {
        assert!(m.contains("machine locale"), "{m}");
        assert!(!m.contains("evil.test/v1"), "la raison ne doit pas recopier l'URL brute : {m}");
    }
}

#[test]
fn un_identifiant_dans_lurl_est_refuse_car_la_destination_est_scellee() {
    for m in [
        format!("{}", claude().with_endpoint("https://k:secrete@api.exemple.test/v1/m").unwrap_err()),
        format!("{}", gpt().with_base_url("https://k:secrete@api.exemple.test/v1").unwrap_err()),
        format!("{}", gemini().with_base_url("https://k:secrete@api.exemple.test/v1b").unwrap_err()),
    ] {
        assert!(m.contains("userinfo"), "{m}");
        assert!(!m.contains("secrete"), "le message NE DOIT PAS recopier le secret : {m}");
    }
}

/// Un refus qui ne nomme pas SA variable envoie l'exploitant chercher la mauvaise ligne.
#[test]
fn chaque_refus_nomme_sa_variable() {
    let c = format!("{}", claude().with_endpoint("http://evil.test/").unwrap_err());
    let g = format!("{}", gpt().with_base_url("http://evil.test/").unwrap_err());
    let m = format!("{}", gemini().with_base_url("http://evil.test/").unwrap_err());
    assert!(c.contains("EFE_ANTHROPIC_URL"), "{c}");
    assert!(g.contains("EFE_OPENAI_BASE_URL"), "{g}");
    assert!(m.contains("EFE_GEMINI_BASE_URL"), "{m}");
}

// ── 5 · Contre-épreuves : un garde incapable de tomber ne garde rien ────────────────────────────

#[test]
fn contre_epreuve_les_setters_changent_vraiment_quelque_chose() {
    assert_ne!(claude().endpoint(), claude().with_endpoint("https://a.test/v1/m").unwrap().endpoint());
    assert_ne!(gpt().endpoint(), gpt().with_base_url("https://a.test/v1").unwrap().endpoint());
    assert_ne!(gemini().endpoint(), gemini().with_base_url("https://a.test/v1b").unwrap().endpoint());
}

#[test]
fn contre_epreuve_un_refus_ne_laisse_rien_derriere() {
    assert!(claude().with_endpoint("ftp://x.test/").is_err());
    assert_eq!(claude().endpoint().as_deref(), Some(CLAUDE_DEFAUT));
    assert!(gpt().with_base_url("").is_err());
    assert_eq!(gpt().endpoint().as_deref(), Some(GPT_DEFAUT));
    assert!(gemini().with_base_url("pas-une-url").is_err());
    assert_eq!(gemini().endpoint().as_deref(), Some(GEMINI_DEFAUT));
}

// ── 6 · LE QUATRIÈME FOURNISSEUR — celui dont le nom dit « local » ──────────────────────────────
//
// Il n'avait AUCUN contrôle : `from_env` lisait `EFE_LOCAL_SLM_URL` et le passait tel quel à
// `new`. Les trois autres refusaient depuis toujours un `http://` distant ; celui-là l'acceptait
// en silence. Un « SLM local » pointé ailleurs est une violation de souveraineté déguisée en
// optimisation — et son reçu n'en disait rien, puisqu'il héritait du défaut `endpoint() -> None`.

#[test]
fn le_slm_local_accepte_ce_qui_est_reellement_local() {
    for u in ["http://127.0.0.1:8099", "http://localhost:8099", "https://slm.interne.test:8443"] {
        assert!(slm(u).is_ok(), "aurait dû passer : {u}");
    }
}

#[test]
fn le_slm_local_refuse_le_clair_distant_comme_les_trois_autres() {
    let e = refus_slm("http://slm.exemple.test:8099");
    assert!(e.contains("machine locale"), "{e}");
    assert!(e.contains("EFE_LOCAL_SLM_URL"), "le refus doit nommer SA variable : {e}");
}

#[test]
fn le_slm_local_ne_franchit_pas_la_porte_loopback_par_userinfo() {
    let e = refus_slm("http://127.0.0.1:8099@evil.test");
    assert!(e.contains("machine locale"), "{e}");
    assert!(!e.contains("evil.test:"), "la raison ne doit pas recopier l'URL brute : {e}");
}

/// Son `endpoint()` héritait du défaut du trait — `None` — donc le reçu ne disait **rien** de la
/// destination, y compris quand elle était distante. Le silence sur une sortie réelle est du même
/// genre que la nommer mal.
#[test]
fn le_recu_nomme_desormais_la_destination_du_slm_local() {
    let p = slm("http://127.0.0.1:8099").unwrap();
    assert_eq!(p.endpoint().as_deref(), Some("http://127.0.0.1:8099"));
}
