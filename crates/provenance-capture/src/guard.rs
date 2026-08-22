// SPDX-License-Identifier: UNLICENSED
//! **Le garde de souveraineté** — egress refusé par défaut.
//!
//! La passerelle LLM applique ce garde au point d'entrée : les agents ne peuvent appeler qu'un
//! endpoint **local** (les modèles tournent DANS la frontière du client). Aucune donnée client ne
//! sort : c'est la condition d'achat de l'industriel européen.
//!
//! ## M1 — ce fichier n'analyse plus rien
//!
//! Il portait l'implémentation de référence, et un jumeau déclaré « miroir exact » vivait dans
//! `efe-swarm-sdk`. Les deux étaient tenus d'accord par une discipline écrite en commentaire :
//! *« les deux copies durcies ENSEMBLE »*. Elle a tenu deux fois, puis échoué trois — parce qu'une
//! discipline est une vigilance, et qu'une vigilance ne se compile pas.
//!
//! L'analyse vit désormais dans [`scella_authority`], une **feuille sans dépendance** que les deux
//! périmètres prennent sans se souder l'un à l'autre. Ce module n'en est plus que la porte
//! d'entrée historique : la réexportation ci-dessous garde intacte la chaîne
//! `provenance-capture` → `audit-api` → `audit-console`, et aucun appelant n'a eu à changer.
//!
//! ## Les tests, eux, restent ici — et c'est le point
//!
//! Ils décrivent le comportement de l'implémentation **historique**. Les laisser en place, pointés
//! sur la nouvelle, est la seule preuve directe que M1.a ne change aucun comportement : s'ils
//! passent, c'est que le remplacement est neutre sur chaque cas qu'ils couvrent.
//!
//! Un point mérite d'être dit franchement : l'analyseur partagé est **plus strict** que celui qu'il
//! remplace. Il refuse en plus les écritures numériques ambiguës (`2130706433`, `0177.0.0.1`), les
//! IPv6 non canoniques (`[::0001]`) et les IPv6 nues (`fe80::1:8080`) — trois familles que le test
//! de propriété contre `url` a fait apparaître. Sur ces entrées, `is_local_endpoint` répondait déjà
//! « non », et répond toujours « non » : la **décision de souveraineté est inchangée**. Seul
//! `host_of` passe de « une chaîne qui ne désignait pas l'hôte réel » à `None`, ce qui est un
//! durcissement pour ses deux usages — journaliser, et apparier une épingle TLS.

pub use scella_authority::{assert_local, authority_of, host_of, is_local_endpoint, Authority, SovereigntyError};

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
