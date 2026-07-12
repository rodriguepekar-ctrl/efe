// SPDX-License-Identifier: UNLICENSED
//! **Le garde de souveraineté** — egress refusé par défaut.
//!
//! La passerelle LLM applique ce garde au point d'entrée : les agents ne peuvent appeler qu'un endpoint
//! **local** (les modèles tournent DANS la frontière du client). Pendant code du garde EFE
//! `is_local_endpoint` — la même règle, ré-implémentée ici pour garder la couche de capture autonome.
//! Aucune donnée client ne sort : c'est la condition d'achat de l'industriel européen.

/// Extrait l'hôte d'un endpoint : `http://127.0.0.1:11434/v1` → `127.0.0.1` ; `http://[::1]:8080` → `::1`.
/// Minimal, sans dépendance (pas de crate `url`).
fn host_of(endpoint: &str) -> Option<&str> {
    // Après le schéma `://` (ou tout si absent).
    let after_scheme = endpoint.split_once("://").map(|(_, rest)| rest).unwrap_or(endpoint);
    // L'autorité s'arrête au PREMIER de `/`, `?` ou `#` (chemin, requête, fragment) — RFC 3986. Sans
    // terminer sur `?`/`#`, `http://evil.com#@127.0.0.1` laisserait « 127.0.0.1 » passer la frontière
    // alors que tout vrai client HTTP (curl/reqwest) appellerait evil.com : un différentiel d'analyseur
    // = bypass SSRF/egress. On coupe donc le fragment et la requête AVANT d'isoler l'hôte.
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or(after_scheme);
    if authority.is_empty() {
        return None;
    }
    // §3.2 (F3) : fail-close les autorités contenant `\`, un espace ou un caractère de contrôle.
    // Un client WHATWG (`url`/`reqwest`) normalise `\` → `/` ⟹ l'hôte RÉEL diffère de notre analyse
    // RFC-3986 (`http://evil.com\@127.0.0.1` : host reqwest = evil.com, host RFC = 127.0.0.1). Refuser
    // (None) aligne le garde sur le pire cas plutôt que de MENTIR sur la localité (bypass SSRF latent).
    if authority.bytes().any(|b| b == b'\\' || b == b' ' || b.is_ascii_control()) {
        return None;
    }
    // Retire un éventuel `userinfo@` sur le PREMIER `@` (RFC : `user:pass@host`), pas le dernier.
    // `rsplit('@')` (dernier `@`) était dangereux combiné à l'absence de coupe `#`/`?`.
    let hostport = authority.split_once('@').map(|(_, h)| h).unwrap_or(authority);
    // IPv6 littéral entre crochets : `[host]:port`.
    if let Some(rest) = hostport.strip_prefix('[') {
        return rest.split(']').next().filter(|h| !h.is_empty());
    }
    // IPv4 / nom : retire `:port`.
    hostport.split(':').next().filter(|h| !h.is_empty())
}

/// Un hôte est-il une IPv4 de **loopback** (`127.0.0.0/8`) — 4 octets NUMÉRIQUES dont le premier 127 ?
/// Durci vs un naïf `starts_with("127.")` : `127.evil.com` (un NOM, qui pourrait résoudre vers l'externe)
/// ne doit PAS passer la frontière de souveraineté. Seul un littéral IPv4 127.x.x.x compte.
fn is_loopback_ipv4(h: &str) -> bool {
    let octets: Vec<&str> = h.split('.').collect();
    octets.len() == 4 && octets[0] == "127" && octets.iter().all(|o| o.parse::<u8>().is_ok())
}

/// L'endpoint est-il **local** (loopback) ? `localhost`, `::1`, ou une IPv4 `127.0.0.0/8` littérale.
/// L'hôte est normalisé (minuscule + retrait d'**un** point final de FQDN) pour que `LOCALHOST`,
/// `Localhost`, `localhost.` et `127.0.0.1.` soient acceptés (sinon refus *fail-closed* mais gênant) ;
/// le parseur d'octets reste strict (octal/hex/décimal-entier toujours refusés).
pub fn is_local_endpoint(endpoint: &str) -> bool {
    match host_of(endpoint) {
        Some(h) => {
            let n = h.strip_suffix('.').unwrap_or(h).to_ascii_lowercase();
            n == "localhost" || n == "::1" || is_loopback_ipv4(n.as_str())
        }
        None => false,
    }
}

/// Violation de souveraineté : un endpoint non-local a été présenté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SovereigntyError {
    pub endpoint: String,
}
impl std::fmt::Display for SovereigntyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "endpoint non-local refusé (souveraineté/egress) : « {} » — seuls 127.*/localhost/::1 sont permis",
            self.endpoint
        )
    }
}
impl std::error::Error for SovereigntyError {}

/// Exige un endpoint local, sinon refuse (le point d'application du garde à la passerelle).
pub fn assert_local(endpoint: &str) -> Result<(), SovereigntyError> {
    if is_local_endpoint(endpoint) {
        Ok(())
    } else {
        Err(SovereigntyError { endpoint: endpoint.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_loopback_in_all_forms() {
        for e in [
            "http://127.0.0.1:11434",
            "http://127.0.0.1:8080/v1/chat",
            "http://localhost:8000",
            "https://localhost",
            "http://[::1]:11434/v1",
            "http://127.255.0.1:9",
        ] {
            assert!(is_local_endpoint(e), "devrait être local : {e}");
            assert!(assert_local(e).is_ok());
        }
    }

    #[test]
    fn refuses_non_local() {
        for e in [
            "http://192.168.1.5:8000",
            "http://10.0.0.3:11434",
            "https://api.openai.com/v1",
            "http://example.com",
            "http://169.254.1.1", // link-local, non loopback
            "http://[fe80::1]:8080",
        ] {
            assert!(!is_local_endpoint(e), "ne devrait PAS être local : {e}");
            assert!(matches!(assert_local(e), Err(SovereigntyError { .. })));
        }
    }

    #[test]
    fn host_extraction_edge_cases() {
        assert_eq!(host_of("http://user@127.0.0.1:80/x"), Some("127.0.0.1"), "userinfo retiré");
        assert_eq!(host_of("127.0.0.1:11434"), Some("127.0.0.1"), "sans schéma");
        assert_eq!(host_of("http://"), None, "autorité vide");
        // « 127.evil.com » ne doit PAS passer comme loopback : c'est un nom, pas 127.x.x.x numérique.
        assert!(!is_local_endpoint("http://127.evil.com"), "un NOM préfixé 127. n'est pas du loopback IPv4");
    }

    #[test]
    fn ssrf_fragment_and_userinfo_differential_is_closed() {
        // Différentiel d'analyseur : le fragment/la requête NE doivent PAS faire croire au loopback.
        // Un vrai client appellerait evil.com — donc le garde DOIT refuser.
        for e in [
            "http://evil.com#@127.0.0.1",
            "http://evil.com?x@127.0.0.1",
            "http://evil.com/x@127.0.0.1",
            "http://evil.com#@localhost",
            "http://127.0.0.1@evil.com", // userinfo=127.0.0.1, hôte=evil.com → refus (déjà correct)
            "http://user:127.0.0.1@evil.com",
        ] {
            assert_eq!(host_of(e), Some("evil.com"), "l'hôte réel est evil.com : {e}");
            assert!(!is_local_endpoint(e), "doit être refusé (bypass SSRF) : {e}");
            assert!(matches!(assert_local(e), Err(SovereigntyError { .. })));
        }
    }

    #[test]
    fn ssrf_backslash_space_control_fail_closed() {
        // §3.2 (F3) : une autorité contenant `\`, espace ou contrôle diverge d'un client WHATWG
        // (reqwest normalise `\` → `/`) ⟹ host_of=None ⟹ refusé, jamais faussement « local ».
        for e in [
            "http://evil.com\\@127.0.0.1/v1", // reqwest appellerait evil.com
            "http://127.0.0.1 @evil.com",
            "http://127.0.0.1\t/x",
        ] {
            assert_eq!(host_of(e), None, "autorité dangereuse ⟹ None : {e}");
            assert!(!is_local_endpoint(e), "fail-closed : {e}");
            assert!(matches!(assert_local(e), Err(SovereigntyError { .. })));
        }
    }

    #[test]
    fn localhost_is_normalized_case_and_trailing_dot() {
        // FAIL-OPEN serait grave ; ici on corrige seulement des FAUX NÉGATIFS (hôtes légitimement locaux).
        for e in [
            "http://LOCALHOST:8000",
            "http://Localhost/v1",
            "http://localhost.:8000", // point final de FQDN
            "http://127.0.0.1.:11434",
        ] {
            assert!(is_local_endpoint(e), "hôte local légitime, devrait passer : {e}");
        }
        // …mais la normalisation n'ouvre AUCUNE porte : un nom externe reste refusé.
        assert!(!is_local_endpoint("http://localhost.evil.com"));
        assert!(!is_local_endpoint("http://127.0.0.1.evil.com"));
    }
}
