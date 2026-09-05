// SPDX-License-Identifier: UNLICENSED
//! **L'aller-retour frontière, sur une vraie socket, pour les TROIS fournisseurs.**
//!
//! ## Ce que ce fichier ferme
//!
//! `scripts/p1-check.sh` déclarait, en tête de ce qu'il ne prouve pas :
//!
//! > *Un ALLER-RETOUR frontière réel. `ANTHROPIC_URL` est une constante EN DUR : il n'existe aucun
//! > moyen de pointer un faux fournisseur local, ni un Azure OpenAI, ni un mandataire d'entreprise.*
//!
//! La première moitié était une **conséquence** de la seconde, et c'était vrai des trois providers.
//! Tant que les destinations étaient gravées dans le binaire, aucun test ne pouvait les joindre sans
//! appeler Anthropic, OpenAI ou Google pour de vrai — c'est-à-dire sans clé, sans argent, sans
//! réseau. Le chemin d'egress payant n'a donc jamais été parcouru **de bout en bout**, dans aucun
//! test de ce dépôt.
//!
//! Ici il l'est, trois fois : un serveur HTTP réel écoute sur la boucle locale, le provider est
//! pointé vers lui, et l'appel part, arrive, et revient. Ce n'est pas une simulation du client —
//! c'est le client.
//!
//! ## Ce qu'il ne prouve toujours pas
//!
//! Que les fournisseurs répondent ainsi. Les faux rendent la forme **documentée** de chaque API ;
//! si l'une changeait, ces tests resteraient verts et la production tomberait. Ils prouvent le
//! **chemin**, jamais le **partenaire** — la distinction que tout ce dépôt tient par ailleurs entre
//! mécanisme et substance.

// Même garde, même raison que `endpoint_configurable.rs` : ces features ne sont pas dans le défaut
// du crate. C'est `scripts/p1-check.sh` qui joue ce fichier, et qui échoue si zéro test a tourné.
#![cfg(all(feature = "anthropic", feature = "openai", feature = "gemini"))]

use efe_coordinator::llm::providers::auth::ApiKey;
use efe_coordinator::{ClaudeProvider, GeminiProvider, GptProvider, LlmCall, LlmProvider};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn appel(prompt: &str) -> LlmCall {
    LlmCall {
        prompt: prompt.to_string(),
        max_tokens: 64,
        temperature: 0.0,
        deadline: None,
        trace_tag: Some("aller-retour".into()),
    }
}

// ── Les trois formes documentées de réponse ─────────────────────────────────────────────────────

fn rep_anthropic(t: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "msg_faux", "type": "message", "role": "assistant", "model": "claude-test",
        "content": [{ "type": "text", "text": t }],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 11, "output_tokens": 7 }
    })
}

fn rep_openai(t: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-faux", "object": "chat.completion", "model": "gpt-test",
        "choices": [{ "index": 0, "message": { "role": "assistant", "content": t },
                      "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18 }
    })
}

fn rep_gemini(t: &str) -> serde_json::Value {
    serde_json::json!({
        "candidates": [{ "content": { "parts": [{ "text": t }], "role": "model" },
                         "finishReason": "STOP" }],
        "usageMetadata": { "promptTokenCount": 11, "candidatesTokenCount": 7 }
    })
}

// ── 1 · L'appel part, arrive et revient ─────────────────────────────────────────────────────────

#[tokio::test]
async fn claude_lappel_part_arrive_et_revient() {
    let faux = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        // Les en-têtes que le provider DOIT poser, vérifiés depuis L'EXTÉRIEUR — en regardant la
        // requête reçue, jamais le code qui la construit. Si l'un manquait, le faux fournisseur ne
        // répondrait pas et ce test tomberait.
        .and(header("anthropic-version", "2023-06-01"))
        .and(header("x-api-key", "sk-ant-faux-0123456789abcdef"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rep_anthropic("bonjour")))
        .expect(1)
        .mount(&faux)
        .await;

    let url = format!("{}/v1/messages", faux.uri());
    let p = ClaudeProvider::new(ApiKey::new("sk-ant-faux-0123456789abcdef"), "claude-test")
        .with_endpoint(&url)
        .expect("un faux fournisseur local doit être accepté");
    assert_eq!(p.endpoint().as_deref(), Some(url.as_str()), "la destination scellée est le faux");

    let r = p.call(appel("dis bonjour")).await.expect("l'aller-retour doit aboutir");
    assert_eq!(r.text, "bonjour");
    assert_eq!(r.tokens_generated, 7, "les tokens d'output doivent être lus dans `usage`");

    // `expect(1)` est vérifié à la destruction : si le provider n'avait appelé personne, ce `drop`
    // ferait tomber le test. C'est le plancher anti-vacuité de ce fichier.
    drop(faux);
}

#[tokio::test]
async fn gpt_lappel_part_arrive_et_revient() {
    let faux = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-faux-0123456789abcdef"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rep_openai("bonjour")))
        .expect(1)
        .mount(&faux)
        .await;

    let base = format!("{}/v1", faux.uri());
    let p = GptProvider::new(ApiKey::new("sk-faux-0123456789abcdef"), "gpt-test")
        .with_base_url(&base)
        .expect("une base locale doit être acceptée");
    assert_eq!(
        p.endpoint().as_deref(),
        Some(format!("{base}/chat/completions").as_str()),
        "la destination scellée est l'URL de complétion dérivée de la base"
    );

    let r = p.call(appel("dis bonjour")).await.expect("l'aller-retour doit aboutir");
    assert_eq!(r.text, "bonjour");
    assert_eq!(r.tokens_generated, 7);
    drop(faux);
}

#[tokio::test]
async fn gemini_lappel_part_arrive_et_revient() {
    let faux = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-test:generateContent"))
        // §2.3 du provider : la clé va en EN-TÊTE, jamais dans l'URL. Ce matcher le vérifie de
        // l'extérieur — une régression qui la remettrait en query string ferait tomber ce test.
        // La fixture ne commence PAS par `AIza`, et c'est délibéré : une fausse clé qui ressemble
        // à une vraie fait tomber `gitleaks` sur l'arbre de travail. Le provider n'exige aucun
        // préfixe (`ApiKey::from_env("GOOGLE_AI_KEY", "")`), donc rien n'obligeait à imiter le
        // format — et changer la fixture coûte moins qu'une exemption, qui est de la dette.
        .and(header("x-goog-api-key", "cle-de-test-gemini-non-secrete"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rep_gemini("bonjour")))
        .expect(1)
        .mount(&faux)
        .await;

    let base = format!("{}/v1beta/models", faux.uri());
    let p = GeminiProvider::new(ApiKey::new("cle-de-test-gemini-non-secrete"), "gemini-test")
        .with_base_url(&base)
        .expect("une base locale doit être acceptée");
    assert_eq!(p.endpoint().as_deref(), Some(base.as_str()));

    let r = p.call(appel("dis bonjour")).await.expect("l'aller-retour doit aboutir");
    assert_eq!(r.text, "bonjour");
    assert_eq!(r.tokens_generated, 7);
    drop(faux);
}

// ── 2 · Une panne du fournisseur remonte comme une panne ────────────────────────────────────────

/// Un aller-retour qui échoue doit échouer **en disant pourquoi**, pas en rendant du texte vide.
#[tokio::test]
async fn une_erreur_du_fournisseur_remonte_comme_une_erreur() {
    for cas in ["claude", "gpt", "gemini"] {
        let faux = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("{\"error\":\"nope\"}"))
            .mount(&faux)
            .await;

        let e = match cas {
            "claude" => {
                let p = ClaudeProvider::new(ApiKey::new("sk-ant-faux-0123456789ab"), "claude-test")
                    .with_endpoint(format!("{}/v1/messages", faux.uri()))
                    .unwrap();
                format!("{}", p.call(appel("x")).await.expect_err("un 400 n'est pas un succès"))
            }
            "gpt" => {
                let p = GptProvider::new(ApiKey::new("sk-faux-0123456789ab"), "gpt-test")
                    .with_base_url(format!("{}/v1", faux.uri()))
                    .unwrap();
                format!("{}", p.call(appel("x")).await.expect_err("un 400 n'est pas un succès"))
            }
            _ => {
                let p = GeminiProvider::new(ApiKey::new("cle-de-test-gemini-non-secrete"), "gemini-test")
                    .with_base_url(format!("{}/v1beta/models", faux.uri()))
                    .unwrap();
                format!("{}", p.call(appel("x")).await.expect_err("un 400 n'est pas un succès"))
            }
        };
        assert!(!e.trim().is_empty(), "{cas} : l'erreur doit dire quelque chose, pas se taire");
    }
}

// ── 3 · Contre-épreuve : c'est bien le faux fournisseur qui répond ──────────────────────────────

/// Sans elle, un `with_*` inopérant laisserait les providers appeler leur point de terminaison
/// public — le test tomberait sur le réseau, ou pire, réussirait un jour.
#[tokio::test]
async fn contre_epreuve_le_faux_fournisseur_est_bien_celui_qui_repond() {
    const SIGNATURE: &str = "SIGNATURE-LOCALE-QUI-NE-PEUT-VENIR-QUE-DICI";

    let f1 = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rep_anthropic(SIGNATURE)))
        .mount(&f1)
        .await;
    let c = ClaudeProvider::new(ApiKey::new("sk-ant-faux-0123456789ab"), "claude-test")
        .with_endpoint(format!("{}/v1/messages", f1.uri()))
        .unwrap();
    assert_eq!(c.call(appel("x")).await.unwrap().text, SIGNATURE);

    let f2 = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rep_openai(SIGNATURE)))
        .mount(&f2)
        .await;
    let g = GptProvider::new(ApiKey::new("sk-faux-0123456789ab"), "gpt-test")
        .with_base_url(format!("{}/v1", f2.uri()))
        .unwrap();
    assert_eq!(g.call(appel("x")).await.unwrap().text, SIGNATURE);

    let f3 = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(rep_gemini(SIGNATURE)))
        .mount(&f3)
        .await;
    let m = GeminiProvider::new(ApiKey::new("cle-de-test-gemini-non-secrete"), "gemini-test")
        .with_base_url(format!("{}/v1beta/models", f3.uri()))
        .unwrap();
    assert_eq!(m.call(appel("x")).await.unwrap().text, SIGNATURE);
}
