// SPDX-License-Identifier: UNLICENSED
//! **L'authentification de session → `AuthContext`.** C'est la frontière que l'`auth.rs` de S2 décrit :
//! « l'authentification est le travail de la coquille de transport, qui construit un `AuthContext` **déjà
//! vérifié** ». S2 enforce ensuite l'**autorisation** (qui peut quoi) — jamais la console.
//!
//! Le mécanisme d'authentification (SSO, mTLS, OIDC…) est **orthogonal** à la confiance de provenance :
//! on fournit ici une résolution simple **jeton-porteur → session**, qu'un déploiement remplace par son
//! IdP. Le point d'architecture qui compte : la session porte le **tenant**, le **rôle** et le **sujet**,
//! et rien d'autre — la **clé tenant n'est jamais ici** (elle vit dans le backend, cf. [`crate::app`]).
//!
//! ## M8 — trois durcissements, et pourquoi chacun
//!
//! 1. **Le jeton n'est jamais gardé.** Seule son empreinte SHA-256 est en mémoire. Un vidage de
//!    processus, un fichier de cœur, un journal de débogueur ne rendent alors **aucun identifiant
//!    utilisable** : une empreinte ne se présente pas à la place d'un jeton. Le coût est nul, le
//!    format de déclaration n'y change rien — et une déclaration peut désormais ne porter QUE
//!    l'empreinte (`token_sha256`), auquel cas le fichier cesse d'être un magasin de secrets.
//! 2. **La comparaison est à temps constant.** On compare des *empreintes*, pas des jetons, et fuir
//!    une empreinte n'est pas fuir un jeton (préimage). Mais `ct_eq` est déjà là, le coût est nul, et
//!    l'alternative demande d'argumenter une non-fuite plutôt que de la rendre impossible.
//! 3. **Une session peut EXPIRER.** Une session sans échéance est un identifiant qui ne meurt
//!    jamais ; le champ est optionnel (le format de M7 ne casse pas), et le démarrage **compte** les
//!    sessions non bornées plutôt que de les accepter en silence.
//!
//! ## D'où vient le jeton, et pourquoi il faut le savoir
//!
//! [`extract_token`] rend aussi la **provenance** du jeton. Ce n'est pas une curiosité : un cookie
//! est envoyé **automatiquement** par un navigateur, y compris sur une requête déclenchée par un
//! autre site ; un en-tête `Authorization` ne peut pas l'être. La défense CSRF de [`crate::handler`]
//! repose entièrement sur cette distinction.

use crate::http::HttpRequest;
use audit_api::{AuthContext, Role};
use provenance_dag::{ct_eq, sha256};

/// L'empreinte d'un jeton — ce qu'on garde, à la place du jeton.
pub type Empreinte = [u8; 32];

/// L'empreinte SHA-256 d'un jeton présenté.
#[must_use]
pub fn empreinte(token: &str) -> Empreinte {
    sha256(token.as_bytes())
}

/// L'empreinte lue depuis une déclaration hexadécimale (64 caractères).
#[must_use]
pub fn empreinte_de_hex(hex: &str) -> Option<Empreinte> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Une session authentifiée (le triplet que S2 exige, plus son échéance).
#[derive(Debug, Clone)]
pub struct Session {
    pub tenant: String,
    pub role: Role,
    pub subject: String,
    /// L'instant (époque, secondes) au-delà duquel la session ne résout plus. `None` = sans
    /// échéance — accepté, mais **compté** au démarrage.
    pub expire_le: Option<u64>,
}

/// D'où venait le jeton présenté. La défense CSRF en dépend entièrement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// `Authorization: Bearer …` — un site tiers **ne peut pas** faire poser cet en-tête par un
    /// navigateur sur une requête simple. L'en-tête est donc lui-même une défense.
    EnTete,
    /// `Cookie: session=…` — un navigateur l'envoie **automatiquement**, y compris sur une requête
    /// déclenchée par un autre site. C'est le vecteur CSRF.
    Cookie,
}

/// Le registre des sessions actives — empreinte → session. (Un déploiement réel branche son IdP ici ;
/// la forme du contrat — `resolve(token, now) -> AuthContext` — ne change pas.)
///
/// Une **liste**, parcourue en entier : la comparaison ne s'arrête pas au premier succès, donc le
/// temps de réponse ne dépend pas de la position de l'entrée trouvée.
#[derive(Default)]
pub struct Sessions {
    par_empreinte: Vec<(Empreinte, Session)>,
}

impl Sessions {
    #[must_use]
    pub fn new() -> Self {
        Sessions { par_empreinte: Vec::new() }
    }

    /// Enregistre une session à partir du **jeton en clair** (il est haché ici et non conservé).
    pub fn insert(&mut self, token: &str, tenant: &str, role: Role, subject: &str) {
        self.inserer(empreinte(token), tenant, role, subject, None);
    }

    /// Enregistre une session à partir de l'**empreinte** — la voie par laquelle une déclaration
    /// peut ne contenir aucun secret.
    pub fn inserer(
        &mut self,
        e: Empreinte,
        tenant: &str,
        role: Role,
        subject: &str,
        expire_le: Option<u64>,
    ) {
        let s = Session {
            tenant: tenant.to_string(),
            role,
            subject: subject.to_string(),
            expire_le,
        };
        if let Some(slot) = self.par_empreinte.iter_mut().find(|(h, _)| *h == e) {
            slot.1 = s;
        } else {
            self.par_empreinte.push((e, s));
        }
    }

    /// Résout un jeton en `AuthContext` **déjà vérifié**, ou `None` (→ 401).
    ///
    /// Le balayage est **complet** : on ne sort pas au premier succès, pour que la durée ne dise
    /// rien de l'entrée trouvée. Une session échue ne résout pas — et son absence est
    /// indistinguable d'un jeton inconnu, ce qui est voulu : dire « votre session a expiré » à un
    /// porteur non authentifié lui apprendrait que ce jeton a existé.
    #[must_use]
    pub fn resolve(&self, token: &str, now: u64) -> Option<AuthContext> {
        let e = empreinte(token);
        let mut trouve: Option<&Session> = None;
        for (h, s) in &self.par_empreinte {
            if ct_eq(h, &e) {
                trouve = Some(s);
            }
        }
        let s = trouve?;
        match s.expire_le {
            Some(t) if now >= t => None,
            _ => Some(AuthContext::new(&s.tenant, s.role, &s.subject)),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.par_empreinte.is_empty()
    }

    /// Combien de sessions **sans échéance** — la grandeur qu'un démarrage doit annoncer plutôt que
    /// d'accepter en silence : un identifiant qui ne meurt jamais est une décision, pas un défaut.
    #[must_use]
    pub fn sans_expiration(&self) -> usize {
        self.par_empreinte.iter().filter(|(_, s)| s.expire_le.is_none()).count()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.par_empreinte.len()
    }
}

/// Extrait le jeton de session d'une requête, **et d'où il vient** : en-tête
/// `Authorization: Bearer <token>` **ou** cookie `session=<token>`. L'en-tête a la priorité.
/// Insensible à la casse pour le schéma porteur. `None` = pas de jeton (→ 401).
#[must_use]
pub fn extract_token(req: &HttpRequest) -> Option<(String, Provenance)> {
    if let Some(h) = req.header("authorization") {
        let h = h.trim();
        // « Bearer <token> » (schéma insensible à la casse).
        if let Some(rest) = h.get(..7) {
            if rest.eq_ignore_ascii_case("bearer ") {
                let tok = h[7..].trim();
                if !tok.is_empty() {
                    return Some((tok.to_string(), Provenance::EnTete));
                }
            }
        }
    }
    if let Some(cookie) = req.header("cookie") {
        for part in cookie.split(';') {
            let part = part.trim();
            if let Some(v) = part.strip_prefix("session=") {
                if !v.is_empty() {
                    return Some((v.to_string(), Provenance::Cookie));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(entetes: &str) -> HttpRequest {
        HttpRequest::parse(format!("GET / HTTP/1.1\r\n{entetes}\r\n").as_bytes()).expect("requête")
    }

    #[test]
    fn le_jeton_nest_jamais_garde() {
        let mut s = Sessions::new();
        s.insert("secret-en-clair", "acme", Role::Auditor, "a");
        // On fouille la structure entière : le jeton ne doit apparaître nulle part.
        let brut = format!("{:?}", s.par_empreinte);
        assert!(!brut.contains("secret-en-clair"), "un vidage de processus ne doit rendre aucun identifiant");
        assert!(s.resolve("secret-en-clair", 0).is_some(), "et pourtant il résout");
    }

    #[test]
    fn une_empreinte_seule_suffit_a_declarer_une_session() {
        let mut s = Sessions::new();
        s.inserer(empreinte("le-jeton"), "acme", Role::Viewer, "v", None);
        assert!(s.resolve("le-jeton", 0).is_some(), "déclarée par empreinte, résolue par jeton");
        assert!(s.resolve("autre", 0).is_none());
    }

    #[test]
    fn lempreinte_hexadecimale_se_relit() {
        let e = empreinte("x");
        let hex: String = e.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(empreinte_de_hex(&hex), Some(e));
        assert_eq!(empreinte_de_hex("trop-court"), None);
        assert_eq!(empreinte_de_hex(&"z".repeat(64)), None, "hex invalide");
    }

    #[test]
    fn une_session_echue_ne_resout_plus() {
        let mut s = Sessions::new();
        s.inserer(empreinte("t"), "acme", Role::Auditor, "a", Some(100));
        assert!(s.resolve("t", 99).is_some(), "avant l'échéance");
        assert!(s.resolve("t", 100).is_none(), "à l'échéance : échue");
        assert!(s.resolve("t", 1000).is_none());
    }

    /// Une session échue est **indistinguable** d'un jeton inconnu. Dire « votre session a expiré »
    /// à un porteur non authentifié lui apprendrait que ce jeton a existé.
    #[test]
    fn une_session_echue_est_indistinguable_dun_jeton_inconnu() {
        let mut s = Sessions::new();
        s.inserer(empreinte("echu"), "acme", Role::Auditor, "a", Some(10));
        assert_eq!(s.resolve("echu", 99).is_none(), s.resolve("jamais-vu", 99).is_none());
    }

    #[test]
    fn les_sessions_sans_echeance_sont_comptees() {
        let mut s = Sessions::new();
        s.inserer(empreinte("a"), "t", Role::Viewer, "a", None);
        s.inserer(empreinte("b"), "t", Role::Viewer, "b", Some(500));
        s.inserer(empreinte("c"), "t", Role::Viewer, "c", None);
        assert_eq!((s.len(), s.sans_expiration()), (3, 2), "2 identifiants qui ne meurent jamais");
    }

    #[test]
    fn reposer_la_meme_empreinte_remplace_au_lieu_de_dupliquer() {
        let mut s = Sessions::new();
        s.inserer(empreinte("t"), "acme", Role::Viewer, "v1", None);
        s.inserer(empreinte("t"), "acme", Role::Auditor, "v2", None);
        assert_eq!(s.len(), 1, "une empreinte, une session");
        assert_eq!(s.resolve("t", 0).expect("résout").subject, "v2");
    }

    // ── la provenance du jeton ──

    #[test]
    fn len_tete_a_la_priorite_et_la_provenance_est_dite() {
        let r = req("Authorization: Bearer abc\r\nCookie: session=def\r\n");
        assert_eq!(extract_token(&r), Some(("abc".to_string(), Provenance::EnTete)));

        let r = req("Cookie: session=def\r\n");
        assert_eq!(extract_token(&r), Some(("def".to_string(), Provenance::Cookie)));

        assert_eq!(extract_token(&req("")), None);
    }

    #[test]
    fn le_schema_porteur_est_insensible_a_la_casse() {
        for s in ["Bearer x", "bearer x", "BEARER x"] {
            assert_eq!(
                extract_token(&req(&format!("Authorization: {s}\r\n"))),
                Some(("x".to_string(), Provenance::EnTete))
            );
        }
    }
}
