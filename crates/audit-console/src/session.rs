// SPDX-License-Identifier: UNLICENSED
//! **L'authentification de session → `AuthContext`.** C'est la frontière que l'`auth.rs` de S2 décrit :
//! « l'authentification est le travail de la coquille de transport, qui construit un `AuthContext` **déjà
//! vérifié** ». S2 enforce ensuite l'**autorisation** (qui peut quoi) — jamais la console.
//!
//! Le mécanisme d'authentification (SSO, mTLS, OIDC…) est **orthogonal** à la confiance de provenance :
//! on fournit ici une résolution simple **jeton-porteur → session**, qu'un déploiement remplace par son
//! IdP. Le point d'architecture qui compte : la session porte le **tenant**, le **rôle** et le **sujet**,
//! et rien d'autre — la **clé tenant n'est jamais ici** (elle vit dans le backend, cf. [`crate::app`]).

use crate::http::HttpRequest;
use audit_api::{AuthContext, Role};
use std::collections::HashMap;

/// Une session authentifiée (le triplet que S2 exige).
#[derive(Debug, Clone)]
pub struct Session {
    pub tenant: String,
    pub role: Role,
    pub subject: String,
}

/// Le registre des sessions actives — jeton opaque → session. (Un déploiement réel branche son IdP ici ;
/// la forme du contrat — `resolve(token) -> AuthContext` — ne change pas.)
#[derive(Default)]
pub struct Sessions {
    by_token: HashMap<String, Session>,
}

impl Sessions {
    pub fn new() -> Self {
        Sessions { by_token: HashMap::new() }
    }

    /// Enregistre (ou remplace) une session pour un jeton.
    pub fn insert(&mut self, token: &str, tenant: &str, role: Role, subject: &str) {
        self.by_token.insert(
            token.to_string(),
            Session { tenant: tenant.to_string(), role, subject: subject.to_string() },
        );
    }

    /// Résout un jeton en `AuthContext` **déjà vérifié** (ou `None` si inconnu → la coquille rend 401).
    pub fn resolve(&self, token: &str) -> Option<AuthContext> {
        self.by_token.get(token).map(|s| AuthContext::new(&s.tenant, s.role, &s.subject))
    }

    pub fn is_empty(&self) -> bool {
        self.by_token.is_empty()
    }
}

/// Extrait le jeton de session d'une requête : en-tête `Authorization: Bearer <token>` **ou** un cookie
/// `session=<token>`. Insensible à la casse pour le schéma porteur. `None` = pas de jeton (→ 401).
pub fn extract_token(req: &HttpRequest) -> Option<String> {
    if let Some(h) = req.header("authorization") {
        let h = h.trim();
        // « Bearer <token> » (schéma insensible à la casse).
        if let Some(rest) = h.get(..7) {
            if rest.eq_ignore_ascii_case("bearer ") {
                let tok = h[7..].trim();
                if !tok.is_empty() {
                    return Some(tok.to_string());
                }
            }
        }
    }
    if let Some(cookie) = req.header("cookie") {
        for part in cookie.split(';') {
            let part = part.trim();
            if let Some(v) = part.strip_prefix("session=") {
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req_with(header: &str, value: &str) -> HttpRequest {
        let raw = format!("GET / HTTP/1.1\r\n{header}: {value}\r\n\r\n");
        HttpRequest::parse(raw.as_bytes()).unwrap()
    }

    #[test]
    fn resolves_a_known_token_to_authcontext() {
        let mut s = Sessions::new();
        s.insert("tok-abc", "acme", Role::ComplianceOfficer, "officer:hanna");
        let auth = s.resolve("tok-abc").unwrap();
        assert_eq!(auth.tenant.as_str(), "acme");
        assert_eq!(auth.role, Role::ComplianceOfficer);
        assert_eq!(auth.subject, "officer:hanna");
        assert!(s.resolve("inconnu").is_none(), "jeton inconnu → None (→ 401)");
    }

    #[test]
    fn extracts_bearer_and_cookie() {
        assert_eq!(extract_token(&req_with("Authorization", "Bearer tok-1")).as_deref(), Some("tok-1"));
        assert_eq!(extract_token(&req_with("authorization", "bearer tok-2")).as_deref(), Some("tok-2"), "schéma insensible à la casse");
        assert_eq!(extract_token(&req_with("Cookie", "theme=dark; session=tok-3; x=y")).as_deref(), Some("tok-3"));
        assert!(extract_token(&req_with("Host", "localhost")).is_none(), "aucun jeton → None");
        assert!(extract_token(&req_with("Authorization", "Bearer ")).is_none(), "porteur vide → None");
    }
}
