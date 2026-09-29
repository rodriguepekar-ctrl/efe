// SPDX-License-Identifier: UNLICENSED
//! `OidcVerifier` — auth `Caller` via **JWT OIDC ES256** (feature `oidc`).
//!
//! **Règle absolue tenue** : on ne manipule **que** la **clé PUBLIQUE** (vérification de
//! signature), lue depuis l'env **server-side** (`EFE_OIDC_PUBLIC_KEY_PEM`) — jamais une clé
//! privée, jamais un secret saisi. Le JWT porte le `tenant` + les `roles` ; on les projette
//! sur le [`Caller`] (même surface que l'extracteur dev → les handlers ne changent pas).

use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

use crate::auth::Caller;
use crate::error::GatewayError;

/// Claims OIDC reconnues. `tenant`/`tid` portent le tenant — l'un des deux est EXIGÉ (voir
/// [`claims_to_caller`]) ; `roles` (ou `realm_access.roles` façon Keycloak) portent le RBAC.
/// `exp`/`iss`/`aud` sont validés par `jsonwebtoken`.
#[derive(Debug, Deserialize)]
struct Claims {
    #[allow(dead_code)]
    sub: String,
    #[serde(default)]
    tenant: Option<String>,
    #[serde(default)]
    tid: Option<String>,
    #[serde(default)]
    roles: Option<Vec<String>>,
    #[serde(default)]
    realm_access: Option<RealmAccess>,
}

#[derive(Debug, Deserialize)]
struct RealmAccess {
    #[serde(default)]
    roles: Vec<String>,
}

/// Vérificateur de JWT OIDC (clé publique ES256 + politique de validation).
pub struct OidcVerifier {
    key: DecodingKey,
    validation: Validation,
    /// L'algorithme épinglé, DÉRIVÉ DE LA CLÉ. Conservé pour pouvoir le nommer.
    alg: Algorithm,
}

/// **L'algorithme se déduit de la CLÉ, jamais du jeton.** C'est toute la sûreté de ce fichier.
///
/// ## Pourquoi RS256 devait exister
///
/// Ce vérifieur n'acceptait que `ES256`. **Keycloak et Auth0 — les deux fournisseurs d'identité
/// d'entreprise les plus déployés — émettent en RS256 par défaut.** Un prospect devait donc
/// reconfigurer son IdP pour une seule audience avant même de voir le produit. `p1-check.sh`
/// nommait ce blocage depuis toujours dans « ce que ce contrôle ne prouve pas » ; il est levé ici.
///
/// ## Pourquoi le DÉDUIRE de la clé, et pas le lire dans le jeton
///
/// La **confusion d'algorithmes** est la faille classique du JWT : l'attaquant réécrit l'en-tête,
/// et un vérifieur qui fait confiance à `alg` vérifie avec le mauvais schéma. La forme la plus
/// connue : annoncer `HS256` et signer en HMAC **avec la clé publique comme secret** — publique,
/// donc connue de tous. `Validation::new(alg)` pose `algorithms = vec![alg]`, une liste d'UN
/// élément : un jeton qui annonce autre chose est refusé avant toute vérification de signature.
///
/// ⚠ `Algorithm::default()` vaut **HS256**. Un `Validation::default()` accepterait donc le
/// symétrique. Ce fichier ne construit JAMAIS une validation par défaut, et un test le tient.
///
/// ## Pourquoi « essayer EC, puis RSA » est sûr
///
/// `jsonwebtoken` classe le PEM par l'**OID de son DER**, pas par son en-tête : `from_ec_pem`
/// rend `InvalidKeyFormat` sur une clé RSA, et réciproquement. Une clé ne peut donc pas se voir
/// épingler l'algorithme de l'autre famille. Un test le vérifie plutôt que de le supposer.
///
/// ## Ce qui n'est PAS accepté, et c'est délibéré
///
/// `ES384`, `RS512`, `PS256`, `EdDSA` : chacun se déduirait mal (une clé RSA sert RS256 comme
/// RS512 — le choix appartient à l'IdP, pas à nous) et chacun est une décision. On préfère
/// refuser un jeton qu'accepter une famille qu'on n'a pas éprouvée.
///
/// # Errors
/// Un PEM qui n'est ni EC ni RSA.
fn cle_et_algorithme(pem: &str) -> Result<(DecodingKey, Algorithm), GatewayError> {
    if let Ok(k) = DecodingKey::from_ec_pem(pem.as_bytes()) {
        return Ok((k, Algorithm::ES256));
    }
    if let Ok(k) = DecodingKey::from_rsa_pem(pem.as_bytes()) {
        return Ok((k, Algorithm::RS256));
    }
    Err(GatewayError::Internal(
        "clé publique OIDC invalide : ni EC (ES256) ni RSA (RS256). Les autres familles ne sont \
         pas acceptées — voir la documentation de `cle_et_algorithme`."
            .to_string(),
    ))
}

impl OidcVerifier {
    /// Construit depuis une **clé publique PEM** (EC P-256 → ES256, ou RSA → RS256) +
    /// issuer/audience optionnels.
    ///
    /// # Errors
    /// Un PEM qui n'est ni EC ni RSA.
    pub fn from_pem(public_key_pem: &str, issuer: Option<&str>, audience: Option<&str>) -> Result<Self, GatewayError> {
        let (key, alg) = cle_et_algorithme(public_key_pem)?;
        // `Validation::new(alg)` pose `algorithms = vec![alg]` — UN seul. Un jeton dont l'en-tête
        // annonce autre chose est refusé avant toute vérification de signature.
        let mut validation = Validation::new(alg);
        if let Some(iss) = issuer {
            validation.set_issuer(&[iss]);
        }
        match audience {
            Some(aud) => validation.set_audience(&[aud]),
            None => validation.validate_aud = false,
        }
        Ok(Self { key, validation, alg })
    }

    /// L'algorithme réellement épinglé — pour que le rapport de démarrage puisse le NOMMER.
    /// Un exploitant qui voit « ES256 » alors que son IdP émet du RS256 a sa réponse en une ligne.
    #[must_use]
    pub fn algorithme(&self) -> &'static str {
        match self.alg {
            Algorithm::RS256 => "RS256",
            _ => "ES256",
        }
    }

    /// Construit depuis l'env **server-side** : `EFE_OIDC_PUBLIC_KEY_PEM` (requis),
    /// `EFE_OIDC_ISSUER_URL` + `EFE_OIDC_AUDIENCE` (optionnels). `None` si non configuré.
    pub fn from_env() -> Result<Option<Self>, GatewayError> {
        let Some(pem) = crate::profile::read::oidc_public_key_pem() else {
            return Ok(None);
        };
        let issuer = crate::profile::read::oidc_issuer_url();
        let audience = crate::profile::read::oidc_audience();
        Ok(Some(Self::from_pem(&pem, issuer.as_deref(), audience.as_deref())?))
    }

    /// **Couche 2 (M3)** : vérificateur **dédié au chemin frontière** — audience **épinglée à
    /// `efe-llm`** (NON conditionnelle : ignore `EFE_OIDC_AUDIENCE`). Un jeton d'essaim
    /// souverain (`aud` absent ou autre) est ainsi **rejeté 401** avant tout egress tiers
    /// coûteux (§4). `EFE_OIDC_LLM_AUDIENCE` surcharge l'audience (défaut `efe-llm`) ; même
    /// clé publique + issuer que [`Self::from_env`]. `None` si `EFE_OIDC_PUBLIC_KEY_PEM` absent.
    pub fn llm_from_env() -> Result<Option<Self>, GatewayError> {
        let Some(pem) = crate::profile::read::oidc_public_key_pem() else {
            return Ok(None);
        };
        let issuer = crate::profile::read::oidc_issuer_url();
        let audience = crate::profile::read::oidc_llm_audience();
        Ok(Some(Self::from_pem(&pem, issuer.as_deref(), Some(&audience))?))
    }

    /// Vérifie le jeton → `Caller`. Signature/exp/iss/aud invalides → 401 ; jeton sans tenant → 401.
    pub fn verify(&self, token: &str) -> Result<Caller, GatewayError> {
        let data = decode::<Claims>(token, &self.key, &self.validation)
            .map_err(|e| GatewayError::Unauthenticated(format!("JWT invalide : {e}")))?;
        claims_to_caller(data.claims)
    }
}

/// **Le tenant se DÉCLARE, il ne se devine pas.**
///
/// Cette fonction rangeait un jeton sans `tenant` ni `tid` dans le tenant système — le même pour
/// tous les jetons dans ce cas. La dérivation restait cloisonnée, mais l'ATTRIBUTION échouait
/// ouverte : selon la configuration de l'IdP, deux organisations partageaient une clé et un
/// magasin sans que rien ne le signale. `profiles/production.env` exigeait pourtant déjà un
/// jeton « portant `aud`, `tenant` (ou `tid`) et `roles` » ; le code ne le tenait pas.
///
/// Refus en **401** plutôt qu'en 403 : le jeton est authentique, mais il ne vaut pas titre pour
/// ce serveur — le remède est côté IdP (émettre la claim), pas dans les rôles.
///
/// Une claim vide ou blanche compte pour absente, et c'est AVANT le repli sur `tid` qu'on l'écarte :
/// filtrer après `or` rendait `tenant: ""` prioritaire sur un `tid` valide.
///
/// Un jeton qui DÉCLARE le tenant système (`system` ou son UUID) reste accepté : c'est une
/// attribution signée par l'IdP, pas un défaut.
fn claims_to_caller(c: Claims) -> Result<Caller, GatewayError> {
    let presente = |s: &String| !s.trim().is_empty();
    let tenant = c.tenant.filter(presente).or(c.tid.filter(presente)).ok_or_else(|| {
        GatewayError::Unauthenticated(
            "jeton sans tenant : ni `tenant` ni `tid` — l'IdP doit émettre l'un des deux".into(),
        )
    })?;
    let mut roles = c.roles.unwrap_or_default();
    if let Some(ra) = c.realm_access {
        roles.extend(ra.roles);
    }
    Ok(Caller { tenant, roles })
}

/// Extrait le jeton d'un en-tête `Authorization: Bearer <token>`.
pub(crate) fn parse_bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    let raw = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// **La paire de test, partagée** — pour que le critère de M3 soit prouvable SOUS `oidc`.
///
/// Elle vivait dans le `mod tests` de ce fichier, donc invisible ailleurs. Conséquence : les deux
/// tests qui portent le critère de M3 s'authentifiaient par en-têtes de développement et étaient
/// `#[cfg(not(feature = "oidc"))]` — c'est-à-dire qu'ils **ne tournaient sous aucun jeu de features
/// de production**. Le trou n'était pas dans M3 : il était dans l'accessibilité de cette fixture.
#[cfg(test)]
pub(crate) mod fixture {
    use super::{Algorithm, GatewayError, OidcVerifier};
    use jsonwebtoken::{encode, EncodingKey, Header};

    // ── Paire de clés EC P-256 de TEST (fixture déterministe — PAS un secret) ──────
    // Générée localement pour les tests unitaires de validation JWT. Ne protège rien.
    pub(crate) const TEST_PRIV_PKCS8: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgv4ad14ofQ83JXwDG\n\
VEWEF6e59s+JE5EvkQaqvNuxN1ShRANCAARoN8FXUFCqiAOqKjEZBXA9x83gosue\n\
1m0JJbEEeOUyiLXTD1lsnCVeQKm3hfz6Dkk06VYWCtBVfNBO13YdRLC5\n\
-----END PRIVATE KEY-----\n";
    pub(crate) const TEST_PUB: &str = "-----BEGIN PUBLIC KEY-----\n\
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEaDfBV1BQqogDqioxGQVwPcfN4KLL\n\
ntZtCSWxBHjlMoi10w9ZbJwlXkCpt4X8+g5JNOlWFgrQVXzQTtd2HUSwuQ==\n\
-----END PUBLIC KEY-----\n";

    /// **Une clé publique RSA de test — PUBLIQUE, donc aucun secret.**
    ///
    /// Sa privée n'est **délibérément pas** dans le dépôt : elle serait un secret committé, et une
    /// exemption gitleaks de plus. Elle n'est pas nécessaire ici — les propriétés de sûreté
    /// (épinglage de l'algorithme, refus de la confusion) se prouvent avec la publique seule.
    /// L'aller-retour RS256 **réel** est joué par `scripts/p1-check.sh`, qui fabrique sa paire avec
    /// `openssl` au moment de l'épreuve et ne laisse rien derrière.
    pub(crate) const TEST_PUB_RSA: &str = "-----BEGIN PUBLIC KEY-----\n\
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAp3ssW7w58100NBZPqpky\n\
dbWtkHxKZJ8aP+o7aHlGCCZtHXjAeOScOoHm7FUNpE0Ln6dM45TPlXzEdQ4iWU0d\n\
qyB3sIEa1j8aoCzY85PkAi8j9zy6zxJRcKP6JGUmf3/nuIsONNmSFH56E0hNJZa6\n\
Y/ND10tHF6zYIpBUXBkQl2hsJRNChoVNuPlIJn083dis9w6/XnRsrt+NiEYJNvmy\n\
UzJ3cBmS/I3Bcs8NSU+SwxMg2SdA8xfpuWQimaAw5rRNX3v3K5C5E3Y/p6C9FiqP\n\
KhlxtIJHpmtTi1YnRRTInYT8MyoNoSNkdvuxuzijnJbOGD6ipu9Lk3Ywn0QbwWEL\n\
XwIDAQAB\n\
-----END PUBLIC KEY-----\n";

    /// Signe des revendications arbitraires avec la clé privée de test.
    pub(crate) fn sign(claims: serde_json::Value) -> String {
        encode(
            &Header::new(Algorithm::ES256),
            &claims,
            &EncodingKey::from_ec_pem(TEST_PRIV_PKCS8.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    /// Un vérifieur du chemin frontière — audience épinglée `efe-llm`, comme `llm_from_env`.
    pub(crate) fn verifieur_llm() -> Result<OidcVerifier, GatewayError> {
        OidcVerifier::from_pem(TEST_PUB, None, Some("efe-llm"))
    }

    /// Un jeton d'opérateur valide pour le chemin frontière.
    pub(crate) fn jeton_operateur(tenant: &str) -> String {
        sign(serde_json::json!({
            "sub": "test-operateur", "tenant": tenant, "aud": "efe-llm",
            "roles": ["operator"], "exp": 9_999_999_999i64
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::fixture::{sign, TEST_PUB};

    // Les constantes vivent désormais dans `fixture` (cf. ci-dessus). Ce bloc les emploie.

    #[test]
    fn verifies_valid_jwt_and_maps_tenant_and_roles() {
        let token = sign(serde_json::json!({
            "sub": "u-1", "tenant": "11111111-1111-1111-1111-111111111111",
            "roles": ["operator", "viewer"], "exp": 9_999_999_999i64
        }));
        let v = OidcVerifier::from_pem(TEST_PUB, None, None).unwrap();
        let caller = v.verify(&token).unwrap();
        assert_eq!(caller.tenant, "11111111-1111-1111-1111-111111111111");
        assert!(caller.roles.contains(&"operator".to_string()));
        assert!(caller.require_any_role(&["operator"]).is_ok());
    }

    #[test]
    fn keycloak_realm_access_roles_are_extracted() {
        let token = sign(serde_json::json!({
            "sub": "u-2", "tid": "system",
            "realm_access": { "roles": ["admin"] }, "exp": 9_999_999_999i64
        }));
        let caller = OidcVerifier::from_pem(TEST_PUB, None, None).unwrap().verify(&token).unwrap();
        assert!(caller.roles.contains(&"admin".to_string()));
    }

    #[test]
    fn expired_token_is_rejected() {
        let token = sign(serde_json::json!({ "sub": "u", "exp": 1i64 })); // 1970 → expiré
        let err = OidcVerifier::from_pem(TEST_PUB, None, None).unwrap().verify(&token);
        assert!(matches!(err, Err(GatewayError::Unauthenticated(_))));
    }

    #[test]
    fn wrong_issuer_is_rejected() {
        let token = sign(serde_json::json!({ "sub": "u", "iss": "https://evil.example", "exp": 9_999_999_999i64 }));
        let v = OidcVerifier::from_pem(TEST_PUB, Some("https://trusted.example"), None).unwrap();
        assert!(matches!(v.verify(&token), Err(GatewayError::Unauthenticated(_))));
    }

    #[test]
    fn parse_bearer_handles_scheme() {
        let mut h = axum::http::HeaderMap::new();
        h.insert(axum::http::header::AUTHORIZATION, "Bearer abc.def.ghi".parse().unwrap());
        assert_eq!(parse_bearer(&h).as_deref(), Some("abc.def.ghi"));
        assert!(parse_bearer(&axum::http::HeaderMap::new()).is_none());
    }
}

// ═════════════════════════════════════════════════════════════════════════════════════════════
//  L'algorithme, épinglé par la CLÉ — et la confusion d'algorithmes, refusée
// ═════════════════════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests_algorithme {
    use super::fixture::{sign, TEST_PUB, TEST_PUB_RSA};
    use super::{Algorithm, OidcVerifier};
    use jsonwebtoken::{encode, EncodingKey, Header};

    fn claims() -> serde_json::Value {
        serde_json::json!({ "sub": "u", "tenant": "system", "exp": 9_999_999_999i64 })
    }

    /// Le grief que ce lot ferme : Keycloak et Auth0 émettent en RS256 par défaut, et ce
    /// vérifieur n'acceptait qu'ES256. Une clé RSA doit être acceptée, ET épingler RS256.
    #[test]
    fn une_cle_rsa_est_acceptee_et_epingle_rs256() {
        let v = OidcVerifier::from_pem(TEST_PUB_RSA, None, None).expect("une clé RSA doit passer");
        assert_eq!(v.algorithme(), "RS256");
    }

    #[test]
    fn une_cle_ec_epingle_toujours_es256() {
        let v = OidcVerifier::from_pem(TEST_PUB, None, None).expect("la clé EC passait déjà");
        assert_eq!(v.algorithme(), "ES256");
    }

    /// **LE TEST QUI COMPTE.** La confusion d'algorithmes dans sa forme la plus connue : l'attaquant
    /// annonce `HS256` et signe en HMAC **avec la clé publique comme secret** — publique, donc
    /// connue de quiconque a lu la configuration. Un vérifieur qui ferait confiance à l'en-tête
    /// `alg` du jeton accepterait ce jeton forgé par n'importe qui.
    #[test]
    fn un_jeton_hs256_signe_avec_la_cle_publique_est_refuse() {
        for pem in [TEST_PUB, TEST_PUB_RSA] {
            let forge = encode(
                &Header::new(Algorithm::HS256),
                &claims(),
                &EncodingKey::from_secret(pem.as_bytes()),
            )
            .expect("forger le jeton");
            let v = OidcVerifier::from_pem(pem, None, None).unwrap();
            assert!(
                v.verify(&forge).is_err(),
                "un jeton HS256 signé avec la clé PUBLIQUE doit être refusé — sinon n'importe qui \
                 forge un jeton d'opérateur avec une valeur qui est, par définition, publique"
            );
        }
    }

    /// L'autre sens de la confusion : une clé d'une famille ne doit jamais accepter un jeton de
    /// l'autre. C'est ce que garantit `algorithms = vec![alg]`, et ce test le tient plutôt que de
    /// s'en remettre à la lecture de la bibliothèque.
    #[test]
    fn un_jeton_es256_est_refuse_par_un_verifieur_rsa() {
        let jeton_ec = sign(claims());
        let v_rsa = OidcVerifier::from_pem(TEST_PUB_RSA, None, None).unwrap();
        assert!(v_rsa.verify(&jeton_ec).is_err(), "familles croisées : refus");
    }

    /// L'hypothèse sur laquelle repose « essayer EC, puis RSA » : chaque constructeur REFUSE la
    /// mauvaise famille. Vérifiée, pas supposée — si `jsonwebtoken` cessait de classer par l'OID
    /// du DER, une clé RSA se verrait épingler ES256 et tous les jetons tomberaient sans qu'on
    /// sache pourquoi.
    #[test]
    fn chaque_constructeur_refuse_la_mauvaise_famille() {
        use jsonwebtoken::DecodingKey;
        assert!(DecodingKey::from_ec_pem(TEST_PUB_RSA.as_bytes()).is_err(), "EC ne lit pas du RSA");
        assert!(DecodingKey::from_rsa_pem(TEST_PUB.as_bytes()).is_err(), "RSA ne lit pas de l'EC");
    }

    #[test]
    fn un_pem_dune_autre_famille_est_refuse_en_nommant_les_deux_acceptees() {
        // `unwrap_err` exigerait `Debug` sur `OidcVerifier` — on n'ajoute pas un `derive` à un
        // type qui détient une clé pour la commodité d'un test (et un `Debug` sur une clé finit
        // tôt ou tard dans un journal).
        let m = match OidcVerifier::from_pem("pas un PEM", None, None) {
            Ok(_) => panic!("un PEM invalide aurait dû être refusé"),
            Err(e) => format!("{e}"),
        };
        assert!(m.contains("EC") && m.contains("RSA"), "le refus doit dire ce qui EST accepté : {m}");
    }
}

// ═════════════════════════════════════════════════════════════════════════════════════════════
//  L'attribution du tenant — un jeton valide qui ne porte ni `tenant` ni `tid`
// ═════════════════════════════════════════════════════════════════════════════════════════════

/// ## L'histoire de ce module, parce qu'elle fait partie de la preuve
///
/// Ses deux premiers tests ont d'abord été écrits pour affirmer le DÉFAUT : un jeton valide sans
/// tenant recevait le tenant système (même clé pour deux organisations), et en HTTP
/// l'organisation B re-prouvait le run de A (200). Ils étaient verts sur le code fautif. Le
/// correctif posé, SANS toucher aux tests, les a fait tomber tous les deux — l'un sur
/// `Unauthenticated("jeton sans tenant …")`, l'autre sur `401 ≠ 200` — pendant que le témoin
/// restait vert. Ils ont ensuite été retournés en gardes du comportement corrigé.
///
/// Aucun test ne branchait `with_oidc` sur le routeur avant celui-ci : le chemin essaim sous
/// OIDC n'avait jamais été joué en HTTP — seul le chemin frontière l'était.
#[cfg(test)]
mod tests_attribution {
    use super::fixture::{sign, verifieur_llm, TEST_PUB};
    use super::{Caller, GatewayError, OidcVerifier};
    use crate::SwarmGatewayState;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use efe_nano_sdk::TenantId;
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tower::ServiceExt; // oneshot

    const ISS: &str = "https://idp.example";
    const AUD: &str = "efe-swarm";
    const EXP: i64 = 9_999_999_999;

    /// Un vérifieur qui exerce les QUATRE contrôles de `jsonwebtoken` : signature, `exp`, `iss`,
    /// `aud`. Un jeton qui passe ici n'a plus rien à prouver — sauf son tenant.
    fn verifieur() -> OidcVerifier {
        OidcVerifier::from_pem(TEST_PUB, Some(ISS), Some(AUD)).expect("clé de test")
    }

    /// Un jeton valide sur les quatre contrôles. `org_id` est la claim d'organisation d'Auth0 :
    /// l'IdP DISTINGUE les deux organisations, mais dans une claim que la passerelle ne lit pas.
    fn jeton(sub: &str, org_id: &str, tenant: Option<&str>) -> String {
        let mut c = json!({
            "sub": sub, "org_id": org_id, "iss": ISS, "aud": AUD,
            "roles": ["operator"], "exp": EXP
        });
        if let Some(t) = tenant {
            c["tenant"] = json!(t);
        }
        sign(c)
    }

    /// Le refus attendu : un 401 qui NOMME ce qui manque — sans quoi l'exploitant chercherait
    /// dans ses rôles un défaut qui est dans la configuration de son IdP.
    fn refus_401(r: Result<Caller, GatewayError>) -> String {
        match r {
            Err(GatewayError::Unauthenticated(m)) => {
                assert!(m.contains("tenant") && m.contains("tid"), "le refus doit nommer les deux claims : {m}");
                m
            }
            autre => panic!("attendu un refus 401 (Unauthenticated), obtenu {autre:?}"),
        }
    }

    fn app() -> axum::Router {
        crate::router(SwarmGatewayState::in_memory().with_oidc(Arc::new(verifieur())))
    }

    /// Exécute un run sous `jeton` et rend `(statut, corps)`.
    async fn run_sous(app: &axum::Router, jeton: &str) -> (StatusCode, Value) {
        let req = Request::builder()
            .method("POST")
            .uri("/v1/swarm/run")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {jeton}"))
            .body(Body::from(json!({ "task": { "text": "x" }, "topology": { "kind": "auto" } }).to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let statut = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (statut, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// Re-prouve `run_id` sous `jeton` et rend le statut.
    async fn verifier_sous(app: &axum::Router, run_id: &str, jeton: &str) -> StatusCode {
        let req = Request::builder()
            .uri(format!("/v1/swarm/verify/{run_id}"))
            .header("authorization", format!("Bearer {jeton}"))
            .body(Body::empty())
            .unwrap();
        app.clone().oneshot(req).await.unwrap().status()
    }

    /// **LE DÉFAUT, FERMÉ.** Deux jetons valides sur les quatre contrôles, deux organisations pour
    /// l'IdP, ni `tenant` ni `tid` : ils recevaient le tenant système, donc la même clé. Refusés
    /// désormais — et par les DEUX vérifieurs. Essaim et frontière partagent `claims_to_caller` ;
    /// le second cas le MESURE plutôt que de le déduire de la lecture du code.
    #[test]
    fn un_jeton_valide_sans_tenant_ni_tid_est_refuse_401() {
        let v = verifieur();
        refus_401(v.verify(&jeton("u-a", "org_A", None)));
        refus_401(v.verify(&jeton("u-b", "org_B", None)));

        // Le chemin frontière (egress PAYANT) : audience épinglée `efe-llm`, même refus.
        let frontiere = sign(json!({ "sub": "u-a", "aud": "efe-llm", "roles": ["operator"], "exp": EXP }));
        refus_401(verifieur_llm().expect("vérifieur de test").verify(&frontiere));
    }

    /// **LE DÉFAUT, FERMÉ EN HTTP.** Le run est créé sous un jeton qui DÉCLARE le tenant système,
    /// dans la forme EXACTE que fabriquait l'ancien repli (`TenantId::system().to_string()`) : le
    /// magasin compare des chaînes (`store.rs`), si bien qu'une autre forme (`"system"`) aurait
    /// rendu 404 même sur le code fautif, et ce test n'aurait rien mesuré.
    ///
    /// Sur le code fautif, le jeton sans tenant exécutait (200) et relisait ce run (200). Désormais :
    /// 401 à la porte, dans les deux cas. Le déclarant, lui, re-prouve toujours son run.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn en_http_un_jeton_sans_tenant_est_refuse_401_et_ne_lit_plus_le_tenant_systeme() {
        let app = app();
        let declarant = jeton("u-op", "org_ops", Some(&TenantId::system().to_string()));
        let (statut, corps) = run_sous(&app, &declarant).await;
        assert_eq!(statut, StatusCode::OK, "le tenant système DÉCLARÉ reste accepté");
        let run_id = corps["run_id"].as_str().expect("run_id").to_string();

        let sans = jeton("u-b", "org_B", None);
        let (statut, corps) = run_sous(&app, &sans).await;
        assert_eq!(statut, StatusCode::UNAUTHORIZED, "exécuter sans tenant : 401");
        assert_eq!(corps["error"], json!("unauthenticated"));
        assert!(
            corps["message"].as_str().is_some_and(|m| m.contains("tenant")),
            "le corps dit ce qui manque : {corps}"
        );
        assert_eq!(verifier_sous(&app, &run_id, &sans).await, StatusCode::UNAUTHORIZED, "relire sans tenant : 401");
        assert_eq!(verifier_sous(&app, &run_id, &declarant).await, StatusCode::OK, "le déclarant re-prouve");
    }

    /// Une claim vide compte pour absente, et c'est AVANT le repli sur `tid` qu'on l'écarte.
    /// L'ancien code filtrait APRÈS `or` : `tenant: ""` masquait un `tid` valide, et le jeton
    /// tombait… dans le tenant système. Une claim blanche, seule, est refusée comme une absente.
    #[test]
    fn une_claim_tenant_vide_cede_a_tid_et_une_claim_blanche_seule_est_refusee() {
        const T: &str = "33333333-3333-3333-3333-333333333333";
        let v = verifieur();

        let vide_puis_tid = sign(json!({ "sub": "u", "iss": ISS, "aud": AUD, "exp": EXP, "tenant": "", "tid": T }));
        assert_eq!(v.verify(&vide_puis_tid).expect("le tid est valide").tenant, T, "`tid` prend le relais");

        let tid_seul = sign(json!({ "sub": "u", "iss": ISS, "aud": AUD, "exp": EXP, "tid": T }));
        assert_eq!(v.verify(&tid_seul).expect("le tid suffit").tenant, T, "`tid` seul (façon Entra ID)");

        let blanche = sign(json!({ "sub": "u", "iss": ISS, "aud": AUD, "exp": EXP, "tenant": "   " }));
        refus_401(v.verify(&blanche));
    }

    /// **LE TÉMOIN** — il tient avant ET après le correctif. Deux tenants DÉCLARÉS : deux clés,
    /// et B ne voit pas le run de A. Le cloisonnement fonctionne ; seule l'attribution faillit.
    /// Sans ce témoin, un test qui verrait « la même clé » ne prouverait rien — il pourrait
    /// aussi bien mesurer un instrument incapable de distinguer.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn temoin_deux_tenants_declares_sont_cloisonnes() {
        const TA: &str = "11111111-1111-1111-1111-111111111111";
        const TB: &str = "22222222-2222-2222-2222-222222222222";
        let (ja, jb) = (jeton("u-a", "org_A", Some(TA)), jeton("u-b", "org_B", Some(TB)));

        let v = verifieur();
        let s = SwarmGatewayState::in_memory();
        let (a, b) = (v.verify(&ja).unwrap(), v.verify(&jb).unwrap());
        assert_ne!(s.derive_key(&a.tenant).unwrap(), s.derive_key(&b.tenant).unwrap(), "deux clés");

        let app = app();
        let (statut, corps) = run_sous(&app, &ja).await;
        assert_eq!(statut, StatusCode::OK);
        let run_id = corps["run_id"].as_str().expect("run_id").to_string();
        assert_eq!(verifier_sous(&app, &run_id, &jb).await, StatusCode::NOT_FOUND, "B ne voit rien");
        assert_eq!(verifier_sous(&app, &run_id, &ja).await, StatusCode::OK, "A re-prouve son run");
    }
}

