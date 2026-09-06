// SPDX-License-Identifier: UNLICENSED
//! Utilitaires partagés par les 2 providers Phase 3 R1.

use std::sync::Arc;

/// Type pour redacter du PII avant envoi LLM externe (invariant I15).
///
/// Closure-based : pas de dépendance forte sur le crate `efe-pii`.
/// En prod, injecter un wrapper qui appelle `efe_pii::Redactor::redact`.
///
/// Exemple d'usage simple (test) :
/// ```ignore
/// let redactor: PiiRedactor = Arc::new(|s: &str| s.replace("@example.com", "[EMAIL]"));
/// let provider = ClaudeProvider::from_env()?.with_redactor(redactor);
/// ```
pub type PiiRedactor = Arc<dyn Fn(&str) -> String + Send + Sync + 'static>;

/// Helper : applique un redactor optionnel.
#[must_use]
pub fn apply_redactor(redactor: Option<&PiiRedactor>, raw: &str) -> String {
    match redactor {
        Some(r) => r(raw),
        None => raw.to_string(),
    }
}

/// Strip toutes les paires `<thinking>...</thinking>` (Claude convention).
///
/// Garde tout ce qui est hors balises.  Si pas de balise → retourne l'input.
#[must_use]
pub fn strip_thinking_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        match rest.find("<thinking>") {
            None => {
                out.push_str(rest);
                return out;
            }
            Some(start) => {
                out.push_str(&rest[..start]);
                rest = &rest[start + "<thinking>".len()..];
                match rest.find("</thinking>") {
                    None => {
                        // Balise non fermée — on garde le contenu après "<thinking>" tel quel
                        // pour ne pas perdre de l'output utile en cas de truncation.
                        return out;
                    }
                    Some(end) => {
                        rest = &rest[end + "</thinking>".len()..];
                        // Optionnel : trim un \n\n résiduel pour propreté.
                        rest = rest.strip_prefix("\n\n").unwrap_or(rest);
                    }
                }
            }
        }
    }
}

/// Construit un client HTTP `reqwest` standardisé.
///
/// - timeout request total : 120 s (les inférences LLM peuvent être lentes).
/// - rustls-tls (pas openssl, ADR-P3R1-001).
/// - user-agent identifiable pour les abuse-reports providers.
#[cfg(any(feature = "anthropic", feature = "gemini", feature = "openai"))]
pub fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(10))
        .user_agent(concat!(
            "efe-coordinator/",
            env!("CARGO_PKG_VERSION"),
            " (efe-multi-llm Phase 3 R1)"
        ))
        .build()
        .expect("reqwest client build must succeed (rustls-tls always available)")
}

/// Split d'un prompt en (system, user) si le format `"SYSTEM: ...\n\nUSER: ..."` est détecté.
/// Sinon retourne `(None, prompt entier)`.
#[must_use]
pub fn split_system_user(prompt: &str) -> (Option<String>, String) {
    if let Some(rest) = prompt.strip_prefix("SYSTEM: ") {
        if let Some(idx) = rest.find("\n\nUSER: ") {
            return (
                Some(rest[..idx].to_string()),
                rest[idx + "\n\nUSER: ".len()..].to_string(),
            );
        }
    }
    (None, prompt.to_string())
}

/// Tronque un body de réponse erreur pour l'inclure proprement dans un message d'erreur.
#[must_use]
pub fn truncate_error_body(body: &str) -> String {
    const MAX: usize = 512;
    if body.len() <= MAX {
        body.to_string()
    } else {
        format!("{}... [truncated, {} chars]", &body[..MAX], body.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_system_user_no_system() {
        let (sys, user) = split_system_user("Hello, world!");
        assert!(sys.is_none());
        assert_eq!(user, "Hello, world!");
    }

    #[test]
    fn split_system_user_with_system() {
        let prompt = "SYSTEM: You are helpful.\n\nUSER: What is 2+2?";
        let (sys, user) = split_system_user(prompt);
        assert_eq!(sys, Some("You are helpful.".to_string()));
        assert_eq!(user, "What is 2+2?");
    }

    #[test]
    fn split_system_user_partial_match() {
        // "SYSTEM: ..." sans "\n\nUSER: ..." → traité comme tout user.
        let prompt = "SYSTEM: only system, no user marker";
        let (sys, user) = split_system_user(prompt);
        assert!(sys.is_none());
        assert!(user.starts_with("SYSTEM"));
    }

    #[test]
    fn truncate_error_body_short() {
        assert_eq!(truncate_error_body("short error"), "short error");
    }

    #[test]
    fn truncate_error_body_long() {
        let long = "x".repeat(1000);
        let trunc = truncate_error_body(&long);
        assert!(trunc.contains("truncated"));
        assert!(trunc.contains("1000"));
        assert!(trunc.len() < 600);
    }

    #[cfg(any(feature = "anthropic", feature = "gemini", feature = "openai"))]
    #[test]
    fn http_client_builds() {
        let _client = build_http_client();
    }

    #[test]
    fn strip_thinking_simple() {
        let s = "<thinking>plan</thinking>\n\nVisible answer.";
        assert_eq!(strip_thinking_tags(s), "Visible answer.");
    }

    #[test]
    fn strip_thinking_multiple() {
        let s = "<thinking>a</thinking>x<thinking>b</thinking>y";
        assert_eq!(strip_thinking_tags(s), "xy");
    }

    #[test]
    fn strip_thinking_none() {
        assert_eq!(strip_thinking_tags("plain"), "plain");
    }

    #[test]
    fn strip_thinking_unclosed_keeps_prefix() {
        // Balise <thinking> ouverte mais pas fermée → on garde le préfixe.
        let s = "before<thinking>open never close";
        assert_eq!(strip_thinking_tags(s), "before");
    }

    #[test]
    fn strip_thinking_preserves_inner_text_otherwise() {
        let s = "X<thinking>hidden</thinking>Y";
        assert_eq!(strip_thinking_tags(s), "XY");
    }

    #[test]
    fn apply_redactor_when_none() {
        assert_eq!(apply_redactor(None, "hello@x.com"), "hello@x.com");
    }

    #[test]
    fn apply_redactor_when_set() {
        let r: PiiRedactor = std::sync::Arc::new(|s: &str| {
            s.replace("@example.com", "[REDACTED-EMAIL]")
        });
        assert_eq!(
            apply_redactor(Some(&r), "alice@example.com"),
            "alice[REDACTED-EMAIL]"
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════════
//  L'endpoint de sortie, quand il n'est plus une constante
// ═════════════════════════════════════════════════════════════════════════════

/// Valide un endpoint de sortie fourni par l'opérateur. **Fail-closed.**
///
/// ## Pourquoi cette fonction existe
///
/// L'URL de sortie des providers frontière était une **constante compilée**. Une passerelle qui se
/// dit souveraine et dont la destination ne peut pas changer ne l'est pas : on ne peut la pointer ni
/// vers un point de terminaison européen, ni vers un mandataire d'entreprise, ni — et c'est ce qui
/// se voyait le plus — vers un faux fournisseur local, ce qui rendait l'aller-retour frontière
/// **intestable**. `p1-check.sh` le nomme en tête de ce qu'il ne prouve pas.
///
/// Rendre l'URL configurable sans la garder serait pire que la constante : une variable
/// d'environnement qui détourne l'egress est un trou, pas une fonctionnalité.
///
/// ## Les trois règles, et ce que chacune arrête
///
/// 1. **Autorité inanalysable ⟹ refus.** C'est la règle de [`scella_authority`] elle-même : dans le
///    doute, on refuse.
/// 2. **`https` accepté ; `http` seulement vers la machine locale.** Un `http://` distant enverrait
///    les prompts en clair. Le loopback est la seule exception, et c'est elle qui rend le faux
///    fournisseur possible.
/// 3. **Aucun `userinfo`.** `https://user:motdepasse@hôte/` serait accepté par les deux règles
///    précédentes — et l'endpoint est **SCELLÉ DANS LE REÇU** (`ProvenanceProvider` scelle
///    l'endpoint observé). On mettrait un secret dans un artefact conçu pour être conservé sept ans
///    et montré à un régulateur. Qui a besoin d'authentifier son mandataire le fait par en-tête.
///
/// ## Ce qu'elle ne fait PAS : un second analyseur
///
/// Elle n'analyse rien. Elle appelle [`scella_authority::authority_of`], l'analyseur **unique** du
/// dépôt — celui pour lequel M1.b a réduit cinq copies à une, après avoir découvert que
/// `127.0.0.1:8080@evil.com` faisait couper TLS vers `evil.com` sur 875 formes. Un `starts_with`
/// local ici recréerait la sixième.
///
/// La détection du `userinfo` réutilise le même analyseur au lieu d'en écrire un : après le schéma,
/// une URL sans `userinfo` **commence par son hôte** (entre crochets s'il est IPv6). Si ce n'est pas
/// le cas, c'est qu'autre chose se tient devant.
///
/// # Errors
/// Une raison **sûre à journaliser** : elle ne cite jamais l'entrée brute, seulement l'hôte extrait
/// par l'analyseur — recopier l'URL recopierait son `userinfo` dans un journal.
pub fn valider_endpoint_sortie(url: &str) -> Result<(), String> {
    let Some(autorite) = scella_authority::authority_of(url) else {
        return Err("autorité inanalysable — refusé (fail-closed)".to_string());
    };
    let hote = autorite.host();

    let apres_schema = if let Some(r) = url.strip_prefix("https://") {
        r
    } else if let Some(r) = url.strip_prefix("http://") {
        if !autorite.is_loopback() {
            return Err(format!(
                "« http:// » n'est permis que vers la machine locale ; « {hote} » n'en est pas — \
                 les prompts partiraient en clair"
            ));
        }
        r
    } else {
        return Err("schéma absent ou non supporté — attendu « https:// », ou « http:// » vers la \
                    machine locale"
            .to_string());
    };

    // ── Le `userinfo` — demandé à l'analyseur, pas approché ────────────────────────────────
    //
    //  Cette clause s'écrivait « l'autorité commence-t-elle par l'hôte ? ». La condition est
    //  NÉCESSAIRE mais PAS SUFFISANTE : un `userinfo` dont le texte commence par le nom de l'hôte
    //  la satisfait. Mesuré le 6 septembre 2026 — ces deux URL passaient :
    //
    //      https://api.anthropic.com:jeton@api.anthropic.com/v1
    //      https://api.anthropic.com.evil@api.anthropic.com/v1
    //
    //  L'endpoint étant SCELLÉ DANS LE REÇU, le jeton y serait entré — c'est-à-dire dans un
    //  artefact d'audit conservé, exporté, et remis à un tiers. Le cas dangereux de confusion
    //  d'hôte (`https://api.anthropic.com@evil.com/`) était, lui, correctement refusé.
    //
    //  La réponse exacte est « l'autorité contient-elle un `@` ? », et elle appartient à
    //  l'analyseur unique : `decomposer` coupe déjà l'autorité au premier `/`, `?` ou `#`, puis au
    //  premier `@`. Refaire ce découpage ici en aurait fait un second analyseur — ce que M1 a
    //  supprimé, et pour la raison qu'on voit ici : deux découpages qui divergent d'un caractère
    //  ne lisent pas la même URL.
    let porte_userinfo = scella_authority::decomposer(url).is_some_and(|c| c.porte_userinfo);
    if porte_userinfo {
        return Err(format!(
            "l'URL porte un « userinfo » devant « {hote} » — refusé : l'endpoint est SCELLÉ dans le \
             reçu, on n'y met pas d'identifiants. Authentifiez par en-tête."
        ));
    }
    // `apres_schema` reste employé plus haut pour trancher le schéma ; il n'a plus à porter la
    // détection du `userinfo`.
    let _ = apres_schema;
    Ok(())
}

#[cfg(test)]
mod tests_endpoint_sortie {
    use super::valider_endpoint_sortie as valider;

    #[test]
    fn le_defaut_et_les_formes_legitimes_passent() {
        for u in [
            "https://api.anthropic.com/v1/messages",
            "https://eu.example.test/v1/messages",
            "https://mandataire.interne:8443/v1/messages",
            "http://127.0.0.1:8099/v1/messages",
            "http://localhost:8099/v1/messages",
            "http://[::1]:8099/v1/messages",
        ] {
            assert!(valider(u).is_ok(), "aurait dû passer : {u}");
        }
    }

    #[test]
    fn http_distant_est_refuse_car_les_prompts_partiraient_en_clair() {
        let e = valider("http://api.example.test/v1/messages").unwrap_err();
        assert!(e.contains("machine locale"), "{e}");
    }

    /// Le cas exact qui a coûté 875 formes à M1.b — il ne doit PAS passer par la porte loopback.
    #[test]
    fn la_ruse_du_userinfo_loopback_est_refusee() {
        for u in [
            "http://127.0.0.1:8080@evil.test/v1/messages",
            "http://localhost@evil.test/v1/messages",
        ] {
            let e = valider(u).unwrap_err();
            assert!(e.contains("machine locale"), "{u} → {e}");
            assert!(!e.contains("evil.test/v1"), "la raison ne doit pas recopier l'URL brute : {e}");
        }
    }

    #[test]
    fn un_userinfo_meme_en_https_est_refuse_car_lendpoint_est_scelle() {
        let e = valider("https://cle:secrete@api.example.test/v1/messages").unwrap_err();
        assert!(e.contains("userinfo"), "{e}");
        assert!(!e.contains("secrete"), "la raison NE DOIT PAS recopier le secret : {e}");
    }

    /// **Le cas que le test précédent ne pouvait pas voir.**
    ///
    /// `un_userinfo_meme_en_https_est_refuse…` emploie `cle:secrete@api.example.test` — un
    /// `userinfo` qui ne commence PAS par le nom de l'hôte. L'ancienne implémentation approchait
    /// la clause par « l'autorité commence-t-elle par l'hôte ? », condition nécessaire mais **pas
    /// suffisante** : elle refusait bien ce cas-là, et laissait passer ceux-ci. Le test choisissait
    /// l'exemple facile, et le défaut a vécu derrière lui.
    ///
    /// Les trois formes ci-dessous étaient ACCEPTÉES jusqu'au 6 septembre 2026. L'endpoint étant
    /// scellé dans le reçu, le jeton serait entré dans un artefact d'audit conservé, exporté, et
    /// remis à un tiers.
    #[test]
    fn un_userinfo_qui_commence_par_lhote_est_refuse_lui_aussi() {
        for u in [
            "https://api.anthropic.com:jeton@api.anthropic.com/v1/messages",
            "https://api.anthropic.com.evil@api.anthropic.com/v1/messages",
            "https://api.anthropic.com@api.anthropic.com/v1/messages",
        ] {
            let e = valider(u).unwrap_err();
            assert!(e.contains("userinfo"), "{u} aurait dû être refusé pour userinfo : {e}");
            assert!(!e.contains("jeton"), "la raison NE DOIT PAS recopier le secret : {e}");
        }
    }

    /// **Anti-vacuité.** Les trois URL ci-dessus ne diffèrent de la forme légitime que par leur
    /// `userinfo` : sans cette assertion, un `valider` qui refuserait TOUT ferait passer le test
    /// précédent sans rien garder.
    #[test]
    fn la_meme_url_sans_userinfo_passe() {
        assert!(valider("https://api.anthropic.com/v1/messages").is_ok());
    }

    /// La confusion d'hôte — le cas dangereux — était déjà refusée, et doit le rester : ici l'hôte
    /// réel est `evil.com`, et `api.anthropic.com` n'est que le `userinfo`.
    #[test]
    fn la_confusion_dhote_reste_refusee() {
        assert!(valider("https://api.anthropic.com@evil.com/v1/messages").is_err());
    }

    #[test]
    fn les_schemas_absents_ou_exotiques_sont_refuses() {
        for u in ["api.anthropic.com", "ftp://x.test/", "file:///etc/passwd", ""] {
            assert!(valider(u).is_err(), "aurait dû être refusé : {u}");
        }
    }

    /// Contre-épreuve du garde lui-même : sans elle, un `valider` qui rendrait toujours `Ok`
    /// passerait le premier test et l'on ne verrait rien.
    #[test]
    fn le_validateur_sait_refuser() {
        assert!(valider("http://evil.test/").is_err());
        assert!(valider("https://api.anthropic.com/v1/messages").is_ok());
    }
}
