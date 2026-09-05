// SPDX-License-Identifier: UNLICENSED
//! **L'aller-retour frontière, sur une vraie socket — celui qui n'avait jamais eu lieu.**
//!
//! ## Ce que ce fichier ferme
//!
//! `scripts/p1-check.sh` déclarait, en tête de ce qu'il ne prouve pas :
//!
//! > *Un ALLER-RETOUR frontière réel. `ANTHROPIC_URL` est une constante EN DUR : il n'existe aucun
//! > moyen de pointer un faux fournisseur local, ni un Azure OpenAI, ni un mandataire d'entreprise.*
//!
//! La première moitié était une **conséquence** de la seconde. Tant que la destination était gravée
//! dans le binaire, aucun test ne pouvait la joindre sans appeler Anthropic pour de vrai — c'est-à-dire
//! sans une clé, de l'argent, et un réseau. Le chemin d'egress payant n'a donc jamais été parcouru
//! **de bout en bout**, dans aucun test de ce dépôt.
//!
//! Ici il l'est : un serveur HTTP réel écoute sur la boucle locale, le provider est pointé vers lui,
//! et l'appel part, arrive, et revient. Ce n'est pas une simulation du client — c'est le client.
//!
//! ## Ce qu'il ne prouve toujours pas
//!
//! Qu'Anthropic réponde ainsi. Le faux fournisseur rend la forme documentée de l'API ; si l'API
//! changeait, ce test resterait vert et la production tomberait. Il prouve le **chemin**, jamais le
//! **partenaire** — la distinction que tout ce dépôt tient par ailleurs entre mécanisme et substance.

// Même garde, même raison que `endpoint_configurable.rs` : `anthropic` n'est pas dans le défaut du
// crate. C'est `scripts/p1-check.sh` qui joue ce fichier, et qui échoue si zéro test a tourné.
#![cfg(feature = "anthropic")]

use efe_coordinator::llm::providers::auth::ApiKey;
use efe_coordinator::{ClaudeProvider, LlmCall, LlmProvider};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// La forme documentée d'une réponse `POST /v1/messages`.
fn reponse_anthropic(texte: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "msg_faux_fournisseur",
        "type": "message",
        "role": "assistant",
        "model": "claude-test",
        "content": [{ "type": "text", "text": texte }],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 11, "output_tokens": 7 }
    })
}

fn appel(prompt: &str) -> LlmCall {
    LlmCall {
        prompt: prompt.to_string(),
        max_tokens: 64,
        temperature: 0.0,
        deadline: None,
        trace_tag: Some("aller-retour".into()),
    }
}

#[tokio::test]
async fn lappel_part_arrive_et_revient() {
    let faux = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        // Les en-têtes que le provider DOIT poser. Si l'un manquait, le faux fournisseur ne
        // répondrait pas et le test tomberait — c'est le seul endroit du dépôt qui le vérifie
        // depuis l'extérieur, en regardant la requête reçue plutôt que le code qui la construit.
        .and(header("anthropic-version", "2023-06-01"))
        .and(header("x-api-key", "sk-ant-faux-0123456789abcdef"))
        .respond_with(ResponseTemplate::new(200).set_body_json(reponse_anthropic("bonjour")))
        .expect(1)
        .mount(&faux)
        .await;

    let url = format!("{}/v1/messages", faux.uri());
    let p = ClaudeProvider::new(ApiKey::new("sk-ant-faux-0123456789abcdef"), "claude-test")
        .with_endpoint(&url)
        .expect("un faux fournisseur sur la boucle locale doit être accepté");

    // La destination annoncée — donc scellée dans le reçu — est bien le faux fournisseur.
    assert_eq!(p.endpoint().as_deref(), Some(url.as_str()));

    let r = p.call(appel("dis bonjour")).await.expect("l'aller-retour doit aboutir");
    assert_eq!(r.text, "bonjour", "la réponse du faux fournisseur doit remonter telle quelle");
    assert_eq!(r.tokens_generated, 7, "les tokens d'output doivent être lus dans `usage`");

    // `expect(1)` ci-dessus est vérifié à la destruction du serveur : si le provider n'avait
    // appelé personne, ce `drop` ferait tomber le test. C'est le plancher anti-vacuité de ce
    // fichier — sans lui, un `call` qui rendrait une réponse fabriquée passerait.
    drop(faux);
}

/// Un aller-retour qui échoue doit échouer **en disant pourquoi**, pas en rendant du texte vide.
#[tokio::test]
async fn une_erreur_du_fournisseur_remonte_comme_une_erreur() {
    let faux = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_string("{\"error\":{\"message\":\"nope\"}}"))
        .mount(&faux)
        .await;

    let p = ClaudeProvider::new(ApiKey::new("sk-ant-faux-0123456789abcdef"), "claude-test")
        .with_endpoint(format!("{}/v1/messages", faux.uri()))
        .unwrap();

    let e = p.call(appel("x")).await.expect_err("un 400 ne doit pas passer pour un succès");
    let m = format!("{e}");
    assert!(
        m.contains("claude") || m.contains("400") || m.contains("nope"),
        "l'erreur doit nommer ce qui s'est passé, pas se taire : {m}"
    );
}

/// Contre-épreuve du fichier : sans elle, un `with_endpoint` inopérant laisserait le provider
/// appeler `api.anthropic.com` — le test tomberait sur le réseau, ou pire, réussirait un jour.
#[tokio::test]
async fn contre_epreuve_le_faux_fournisseur_est_bien_celui_qui_repond() {
    let faux = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(reponse_anthropic("SIGNATURE-LOCALE")))
        .mount(&faux)
        .await;

    let p = ClaudeProvider::new(ApiKey::new("sk-ant-faux-0123456789abcdef"), "claude-test")
        .with_endpoint(format!("{}/v1/messages", faux.uri()))
        .unwrap();

    let r = p.call(appel("x")).await.expect("aller-retour");
    assert_eq!(
        r.text, "SIGNATURE-LOCALE",
        "ce texte ne peut venir que du serveur monté ici : il prouve que l'octet est bien parti \
         vers la destination configurée, et nulle part ailleurs"
    );
}
