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
