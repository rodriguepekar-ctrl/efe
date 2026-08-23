// SPDX-License-Identifier: UNLICENSED
//! # `scella-authority` — **un seul** analyseur d'autorité, pour tout le dépôt.
//!
//! ## La question à laquelle ce crate répond
//!
//! Elle n'est pas « que dit la RFC 3986 ? » mais : **quel hôte le client HTTP va-t-il réellement
//! contacter ?** Ce sont deux questions différentes, et tout l'écart entre elles est de la surface
//! d'attaque. `http://evil.com#@127.0.0.1` a pour hôte `evil.com` — n'importe quel client le dira —
//! mais un analyseur qui ne coupe pas le fragment y lit « 127.0.0.1 » et conclut au loopback. Le
//! code croit alors parler à la machine locale pendant que la connexion part chez l'attaquant.
//! Cette famille de bugs porte un nom : **différentiel d'analyseur**.
//!
//! ## Pourquoi elle doit avoir UNE seule réponse dans le dépôt
//!
//! Elle en avait cinq. Deux étaient durcies et se disaient « miroirs exacts » l'une de l'autre,
//! maintenues d'accord par une discipline écrite en commentaire : *« les deux copies durcies
//! ENSEMBLE »*. Les trois autres ne l'étaient pas, et personne ne les comptait — le `grep` de
//! référence cherchait le nom `is_local_endpoint`, que trois d'entre elles ne portaient même pas.
//!
//! Une discipline est une vigilance. Elle a tenu deux fois, puis a échoué trois, dont une **dans le
//! crate qui l'avait écrite**. Ce crate remplace la vigilance par une structure : il n'y a plus de
//! copie à tenir d'accord, et un test interdit qu'il en réapparaisse une.
//!
//! ## Trois devoirs, pas un — et le deuxième est le plus lourd
//!
//! * **Appliquer.** [`assert_local`] refuse. La console de conformité l'appelle sur son adresse
//!   d'écoute ; le SDK sur son gate d'egress. C'est le seul devoir que tout le monde reconnaît
//!   comme de la sécurité.
//! * **Attester.** Certains appelants ne gardent rien : ils *classent*, et la classe est ensuite
//!   **scellée dans un reçu**. Un garde qui se trompe laisse passer un appel — c'est grave et c'est
//!   réparable. Un attesteur qui se trompe écrit dans un document non répudiable la phrase « la
//!   computation a été témoignée localement » à propos d'un appel parti ailleurs. Le reçu se
//!   re-prouve alors parfaitement, avec sa fausse affirmation dedans. C'est le pire résultat
//!   possible pour une pile dont l'argument est l'intégrité, et c'est ce devoir-là — invisible,
//!   parce que le site d'appel ressemble à un `match` qui remplit un champ — qui justifie l'unicité.
//! * **Conseiller.** Un avertissement au démarrage. Le moins grave, mais c'est par lui que le
//!   défaut est entré : un « miroir minimal » y avait perdu le durcissement qu'il prétendait
//!   refléter.
//!
//! ## Ce que l'analyseur refuse, et pourquoi il refuse plutôt que de deviner
//!
//! [`authority_of`] rend `None` — c'est-à-dire **« je ne sais pas, donc non »** — dans quatre cas,
//! tous choisis parce qu'un client réel y ferait autre chose que nous :
//!
//! 1. **autorité vide** (`http://`) : il n'y a pas d'hôte ;
//! 2. **`\`, espace ou caractère de contrôle** : un client WHATWG normalise `\` en `/`, donc
//!    `http://evil.com\@127.0.0.1` a pour hôte réel `evil.com` là où une lecture RFC 3986 lirait
//!    `127.0.0.1`. Nous ne tranchons pas ce désaccord : nous refusons ;
//! 3. **non-ASCII** : un client applique l'IDNA et contacte la forme punycode
//!    (`xn--…`), que nous ne calculons pas. Prétendre connaître l'hôte serait mentir ;
//! 4. **hôte vide après retrait du port ou des crochets** ;
//! 5. **écriture numérique ambiguë d'IPv4** (`2130706433`, `0177.0.0.1`, `127.1`) : les clients
//!    normalisent ces formes vers un pointé-quadruple, et pas tous de la même façon. Ce cas-là n'a
//!    pas été trouvé par relecture mais par le test de propriété — voir [`ipv4_ambigue`] ;
//! 6. **IPv6 non canonique** (`[::0001]`) ou **IPv6 nue sans crochets** (`fe80::1:8080`). La
//!    seconde produisait l'hôte « fe80 » dans les deux implémentations historiques : sûr pour la
//!    localité, faux pour l'identité — or c'est cet hôte qui serait journalisé et apparié à une
//!    épingle TLS. Voir [`ipv6_non_canonique`].
//!
//! Le refus est toujours la direction sûre : `None` ⟹ « non local » ⟹ le garde interdit,
//! l'attestation classe en distant, l'avertissement part. Un faux négatif coûte une gêne ; un faux
//! positif coûte la souveraineté.
//!
//! ## Ce que ce crate NE fait pas, délibérément
//!
//! * **Aucune résolution DNS.** Le raisonnement porte sur le *littéral*. Résoudre au moment du
//!   contrôle ouvrirait un TOCTOU — la réponse DNS peut changer entre le contrôle et l'appel —
//!   c'est-à-dire un piège plus profond que celui qu'on referme.
//! * **Aucune IPv6 nue sans crochets.** `http://::1:8080` est refusé, et c'est correct : la RFC
//!   exige les crochets, et `Url::parse` rejette cette forme — un endpoint écrit ainsi n'était de
//!   toute façon pas composable. Un appelant qui en a besoin (le méta-essaim en accepte une
//!   aujourd'hui) devra obtenir une entrée **nommée pour ce qu'elle est**, pas un relâchement
//!   global de la règle : la localité sert aussi à décider si TLS peut être coupé.
//! * **Aucune tolérance à `::0001`** ni aux formes IPv6 alternatives du loopback. Refusées, donc
//!   « non locales », donc sûres. C'est le comportement exact de l'implémentation historique, et
//!   M1.a ne change aucun comportement.
//!
//! ## L'accord n'est pas espéré, il est prouvé
//!
//! `tests/accord_avec_url.rs` engendre un corpus et vérifie, sur chaque entrée que `url` sait
//! analyser, l'invariant : **`authority_of(x)` est `None`, ou son hôte est celui que `url`
//! désigne.** Jamais un troisième cas. `url` est ce sur quoi `reqwest` s'appuie : c'est donc
//! l'oracle de « qui sera réellement contacté ». Il vit en `dev-dependencies` — le crate livré
//! reste une feuille sans dépendance, et la liste de refus cesse d'être une liste noire écrite de
//! mémoire pour devenir une propriété vérifiée.

/// L'**autorité** d'un endpoint, réduite à son hôte — le seul type que ce crate rend.
///
/// C'est un type et non un `&str` pour une raison structurelle, pas esthétique. Toute la classe de
/// bugs se résume à : *quelqu'un a re-dérivé un hôte depuis une chaîne*. Une chaîne nue invite à la
/// re-dérivation — on la recoupe, on la compare à un autre hôte obtenu autrement. Un `Authority`
/// que seul [`authority_of`] peut fabriquer rend l'accident coûteux : pour obtenir un hôte, il faut
/// passer par l'analyseur. C'est exactement ce dont un appariement d'épingle TLS a besoin —
/// comparer deux hôtes exige qu'ils viennent du **même** analyseur, ce que le type impose et
/// qu'une fonction rendant `String` laisserait au hasard.
///
/// Il emprunte son entrée : aucune allocation, ni ici ni dans [`Authority::is_loopback`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Authority<'a>(&'a str);

impl<'a> Authority<'a> {
    /// L'hôte, tel qu'il apparaît dans l'entrée — sans `userinfo`, sans port, sans crochets.
    ///
    /// C'est la forme à **journaliser**. Imprimer l'URL brute recopierait son `userinfo` dans les
    /// logs : `http://user:pass@hôte/` mettrait un secret dans une sortie qui sera collectée,
    /// archivée et lue par des gens qui n'avaient pas à l'avoir. Le retrait du `userinfo` au
    /// **premier** `@` — le durcissement qui rend le garde correct — rend aussi le journal sûr.
    #[must_use]
    pub fn host(&self) -> &'a str {
        self.0
    }

    /// L'hôte désigne-t-il la machine locale ?
    ///
    /// `localhost` (casse indifférente, un point final de FQDN toléré), `::1`, ou une IPv4 du bloc
    /// `127.0.0.0/8` écrite en **quatre octets numériques**. Cette dernière exigence est le cœur du
    /// durcissement : un naïf `starts_with("127.")` accepterait `127.evil.com`, qui est un **nom**
    /// et résout où son propriétaire veut.
    #[must_use]
    pub fn is_loopback(&self) -> bool {
        // Un point final de FQDN (`localhost.`) désigne le même hôte ; le retirer corrige un FAUX
        // NÉGATIF sans ouvrir la moindre porte — `localhost.evil.com` garde ses deux étiquettes.
        let h = self.0.strip_suffix('.').unwrap_or(self.0);
        // `eq_ignore_ascii_case` plutôt qu'une mise en minuscules : même résultat, zéro allocation.
        h.eq_ignore_ascii_case("localhost") || h == "::1" || is_loopback_ipv4(h)
    }
}

/// Une IPv4 de **loopback** littérale : quatre octets numériques, le premier valant 127.
///
/// `parse::<u8>()` refuse l'octal, l'hexadécimal, les entiers décimaux compacts et tout ce qui
/// n'est pas un octet — donc `2130706433` (127.0.0.1 en décimal) est refusé, ce qui est un faux
/// négatif volontaire : un client le résoudrait vers le loopback, mais nous préférons refuser un
/// endpoint local exotique plutôt que risquer d'accepter une écriture que nous lisons mal.
fn is_loopback_ipv4(h: &str) -> bool {
    let octets: Vec<&str> = h.split('.').collect();
    octets.len() == 4 && octets[0] == "127" && octets.iter().all(|o| o.parse::<u8>().is_ok())
}

/// Une écriture **numérique ambiguë** d'IPv4 — c'est-à-dire une que les clients ne lisent pas tous
/// de la même façon.
///
/// Trouvé par le test de propriété, pas par relecture : `http://2130706433` a pour hôte
/// `127.0.0.1` selon `url` (donc selon `reqwest`), et `2130706433` selon une lecture naïve. Même
/// chose pour `0177.0.0.1` (zéro initial = octal pour un résolveur C) et pour les formes courtes
/// (`127.1`). Nous ne cherchons pas à imiter ces normalisations : les imiter reviendrait à parier
/// sur laquelle le client applique. Nous **refusons**, ce qui est la seule position tenable pour un
/// analyseur dont le contrat est « je désigne l'hôte réel, ou je ne dis rien ».
///
/// Un hôte contenant la moindre lettre n'est pas concerné : `127.evil.com` est un **nom**, il passe
/// ici sans être refusé, et c'est [`Authority::is_loopback`] qui le déclare non local — exigence de
/// quatre octets numériques.
///
/// Renvoie `true` lorsqu'il faut refuser.
fn ipv4_ambigue(h: &str) -> bool {
    // Pas une écriture purement numérique ⟹ rien à trancher ici (c'est un nom, ou une IPv6).
    if h.is_empty() || !h.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return false;
    }
    let parts: Vec<&str> = h.split('.').collect();
    if parts.len() != 4 {
        return true; // entier décimal compact, forme courte `127.1`, ou point en trop.
    }
    !parts.iter().all(|p| {
        !p.is_empty()
            && p.len() <= 3
            // Un zéro initial est la marque de l'octal pour un résolveur historique : `0177` vaut
            // 127 pour lui et 177 pour `u8::from_str`. Deux lectures, donc refus.
            && !(p.len() > 1 && p.starts_with('0'))
            && p.parse::<u8>().is_ok()
    })
}

/// Une IPv6 littérale écrite sous une forme **non canonique** — même adresse, autre texte.
///
/// Trouvé par le test de propriété, comme le cas IPv4 : `[::0001]` est l'adresse `::1`, et `url`
/// (donc `reqwest`) la sérialise ainsi ; une comparaison textuelle naïve, elle, y voit un hôte
/// différent. Le problème n'est pas la localité — `::0001` n'est reconnu par aucune de nos
/// comparaisons, donc « non local », donc sûr — mais l'**identité** : `host_of` sert aussi à
/// apparier une épingle TLS et à nommer un endpoint dans un journal.
///
/// La canonisation vient de `std::net` : aucune dépendance ajoutée. On ne s'y **fie** pas pour
/// décider, on l'utilise pour constater un désaccord et refuser. La comparaison ignore la casse,
/// car `FE80::1` et `fe80::1` sont le même texte à la présentation près.
fn ipv6_non_canonique(h: &str) -> bool {
    match h.parse::<std::net::Ipv6Addr>() {
        Ok(a) => !a.to_string().eq_ignore_ascii_case(h),
        Err(_) => false, // ce n'est pas une IPv6 : rien à trancher ici.
    }
}

/// **L'unique analyseur d'autorité du dépôt.** Rend l'hôte que le client contactera, ou `None`.
///
/// Voir la documentation du module pour les quatre cas de refus et pour ce que `None` engage.
/// Accepte les formes sans schéma (`127.0.0.1:8088`), employées couramment pour des adresses
/// d'écoute et des endpoints gRPC.
#[must_use]
pub fn authority_of(endpoint: &str) -> Option<Authority<'_>> {
    // Après le schéma. `split_once` — PAS `split("://").last()`, qui prendrait le DERNIER `://` :
    // un `://` glissé dans la requête ou le fragment renverrait le mauvais segment.
    let after_scheme = endpoint.split_once("://").map(|(_, rest)| rest).unwrap_or(endpoint);

    // L'autorité s'arrête au PREMIER de `/`, `?` ou `#`. Ne couper que sur `/` laissait
    // `http://evil.com#@127.0.0.1` exhiber « 127.0.0.1 » alors que le client appelle evil.com :
    // fragment et requête doivent être tranchés AVANT d'isoler l'hôte.
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or(after_scheme);
    if authority.is_empty() {
        return None;
    }

    // Non-ASCII ⟹ refus : un client applique l'IDNA et contacte la forme punycode, que nous ne
    // calculons pas. Rendre l'hôte tel quel affirmerait un hôte différent de celui appelé.
    // Le contrôle porte sur l'autorité ENTIÈRE, `userinfo` compris — plus conservateur, et la
    // direction du refus est toujours la sûre.
    if !authority.is_ascii() {
        return None;
    }

    // `\`, espace, contrôle : un client WHATWG normalise `\` en `/` et diverge de notre lecture
    // RFC 3986. Nous ne tranchons pas le désaccord — nous refusons, plutôt que de MENTIR sur la
    // localité (bypass SSRF latent).
    if authority.bytes().any(|b| b == b'\\' || b == b' ' || b.is_ascii_control()) {
        return None;
    }

    // `userinfo@` retiré au PREMIER `@` (RFC : `user:pass@host`), jamais au dernier :
    // `rsplit('@')` faisait passer `http://127.0.0.1@evil.com` pour du loopback.
    let hostport = authority.split_once('@').map(|(_, h)| h).unwrap_or(authority);

    let host = if let Some(rest) = hostport.strip_prefix('[') {
        // IPv6 littéral entre crochets : `[::1]:port`.
        let h = rest.split(']').next().filter(|h| !h.is_empty())?;
        if ipv6_non_canonique(h) {
            return None;
        }
        h
    } else {
        // Deux `:` ou plus sans crochets : c'est une IPv6 NUE, que la RFC interdit et que `url`
        // rejette. Sans ce refus, `http://fe80::1:8080` produirait l'hôte « fe80 » — une chaîne
        // qui ne désigne rien, et que les deux implémentations historiques rendaient bel et bien.
        // Sûr pour la localité (« fe80 » n'est pas du loopback), faux pour l'identité : c'est cet
        // hôte-là qui serait journalisé et comparé à une épingle.
        if hostport.bytes().filter(|b| *b == b':').count() >= 2 {
            return None;
        }
        // IPv4 ou nom : retirer `:port`.
        hostport.split(':').next().filter(|h| !h.is_empty())?
    };
    // Un point final de FQDN désigne le même hôte : le neutraliser AVANT le contrôle numérique,
    // sinon `127.0.0.1.` compterait cinq parties et serait refusé à tort.
    let noyau = host.strip_suffix('.').unwrap_or(host);
    if ipv4_ambigue(noyau) {
        return None;
    }
    Some(Authority(host))
}

/// L'hôte d'un endpoint, ou `None`. Raccourci de [`authority_of`] pour les appelants qui n'ont
/// besoin que de la chaîne — typiquement pour **journaliser** sans recopier un `userinfo`.
#[must_use]
pub fn host_of(endpoint: &str) -> Option<&str> {
    authority_of(endpoint).map(|a| a.host())
}

/// L'endpoint désigne-t-il la machine locale ? `None` de l'analyseur ⟹ **non** (fail-closed).
#[must_use]
pub fn is_local_endpoint(endpoint: &str) -> bool {
    authority_of(endpoint).is_some_and(|a| a.is_loopback())
}

/// Violation de souveraineté : un endpoint non local a été présenté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SovereigntyError {
    /// L'entrée refusée. Champ conservé tel quel pour compatibilité avec l'API historique ; un
    /// appelant qui JOURNALISE doit préférer [`host_of`], sans quoi un `userinfo` fuiterait.
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

/// Exige un endpoint local, sinon refuse — le point d'application du garde.
///
/// # Errors
/// [`SovereigntyError`] si l'endpoint n'est pas local, y compris lorsque l'autorité est
/// inanalysable : dans le doute, on refuse.
pub fn assert_local(endpoint: &str) -> Result<(), SovereigntyError> {
    if is_local_endpoint(endpoint) {
        Ok(())
    } else {
        Err(SovereigntyError { endpoint: endpoint.to_string() })
    }
}

/// **Le corpus de conformité** — chaque cas qui a durci cet analyseur, exporté.
///
/// Il est public pour une raison précise : si un jour un appelant doit légitimement garder son
/// propre analyseur (le méta-essaim en a deux, que M1.b traitera), il peut lui faire passer **le
/// même** corpus. C'est le filet pour les endroits où l'extraction est impossible — et c'est
/// exactement ce qui manquait aux trois copies que personne n'avait jamais confrontées à ces cas.
///
/// Format : `(entrée, hôte attendu)`. `None` = refus.
pub const CORPUS_HOTE: &[(&str, Option<&str>)] = &[
    // ── formes ordinaires ────────────────────────────────────────────────────────────────────
    ("http://127.0.0.1:11434/v1", Some("127.0.0.1")),
    ("127.0.0.1:11434", Some("127.0.0.1")),
    ("https://api.openai.com/v1", Some("api.openai.com")),
    ("http://[::1]:11434/v1", Some("::1")),
    ("http://user@127.0.0.1:80/x", Some("127.0.0.1")),
    // ── différentiels d'analyseur : l'hôte réel est evil.com dans TOUS ces cas ────────────────
    ("http://evil.com#@127.0.0.1", Some("evil.com")),
    ("http://evil.com?x@127.0.0.1", Some("evil.com")),
    ("http://evil.com/x@127.0.0.1", Some("evil.com")),
    ("http://evil.com#@localhost", Some("evil.com")),
    ("http://127.0.0.1@evil.com", Some("evil.com")),
    ("http://user:127.0.0.1@evil.com", Some("evil.com")),
    // ── refus : autorité inanalysable ou divergente d'un client WHATWG ────────────────────────
    ("http://", None),
    ("http://evil.com\\@127.0.0.1/v1", None),
    ("http://127.0.0.1 @evil.com", None),
    ("http://127.0.0.1\t/x", None),
    ("http://éxample.com/v1", None),
    ("http://::1:8080", None),
    // Écritures numériques ambiguës : `url` les normalise vers 127.0.0.1, nous refusons plutôt que
    // de parier sur la normalisation qu'appliquera le client.
    ("http://2130706433", None),
    ("http://0177.0.0.1", None),
    ("http://127.1", None),
    ("http://127.0.0.01", None),
    // …mais un NOM préfixé de chiffres n'est pas concerné : il est analysé, puis jugé non local.
    ("http://127.evil.com", Some("127.evil.com")),
    ("http://127.0.0.1.:11434", Some("127.0.0.1.")),
    // IPv6 : forme canonique acceptée, formes équivalentes refusées, forme nue refusée.
    ("http://[::1]:8080", Some("::1")),
    ("http://[FE80::1]:8080", Some("FE80::1")),
    ("http://[::0001]:8080", None),
    ("http://[0:0:0:0:0:0:0:1]", None),
    ("http://fe80::1:8080", None),
];

/// Le corpus de **localité** : `(entrée, est-local)`. Complète [`CORPUS_HOTE`], qui ne dit que
/// l'extraction ; celui-ci dit la décision.
pub const CORPUS_LOCALITE: &[(&str, bool)] = &[
    ("http://127.0.0.1:11434", true),
    ("http://127.255.0.1:9", true),
    ("http://localhost:8000", true),
    ("https://localhost", true),
    ("http://[::1]:11434/v1", true),
    ("http://LOCALHOST:8000", true),
    ("http://Localhost/v1", true),
    ("http://localhost.:8000", true),
    ("http://127.0.0.1.:11434", true),
    ("127.0.0.1:1", true),
    // Le loopback de façade : un NOM préfixé `127.`, qui résout vers l'externe.
    ("http://127.evil.com", false),
    ("http://127.0.0.1.evil.com", false),
    ("http://localhost.evil.com", false),
    // Réseaux privés et lien-local : proches, mais pas la machine locale.
    ("http://192.168.1.5:8000", false),
    ("http://10.0.0.3:11434", false),
    ("http://169.254.1.1", false),
    ("http://[fe80::1]:8080", false),
    ("https://api.openai.com/v1", false),
    // Écritures exotiques du loopback : refusées, donc sûres.
    ("http://2130706433", false),
    ("http://[::0001]:8080", false),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_corpus_dextraction_passe() {
        for (entree, attendu) in CORPUS_HOTE {
            assert_eq!(host_of(entree), *attendu, "extraction : {entree}");
        }
    }

    #[test]
    fn le_corpus_de_localite_passe() {
        for (entree, attendu) in CORPUS_LOCALITE {
            assert_eq!(is_local_endpoint(entree), *attendu, "localité : {entree}");
            assert_eq!(
                assert_local(entree).is_ok(),
                *attendu,
                "`assert_local` doit suivre `is_local_endpoint` sans exception : {entree}"
            );
        }
    }

    /// Le corpus ne vaut que s'il contient réellement les deux directions : un analyseur qui
    /// refuserait tout passerait un corpus fait uniquement de refus.
    #[test]
    fn le_corpus_couvre_les_deux_directions() {
        let acceptes = CORPUS_HOTE.iter().filter(|(_, a)| a.is_some()).count();
        let refuses = CORPUS_HOTE.len() - acceptes;
        assert!(acceptes >= 8, "un corpus sans acceptations ne prouverait rien");
        assert!(refuses >= 5, "un corpus sans refus ne prouverait rien non plus");
        assert!(CORPUS_LOCALITE.iter().any(|(_, l)| *l));
        assert!(CORPUS_LOCALITE.iter().any(|(_, l)| !*l));
    }

    /// Le type est la garantie : hors de ce crate, un `Authority` ne peut naître que de
    /// l'analyseur. Ce test fige le contrat que le compilateur applique déjà.
    #[test]
    fn un_hote_ne_sobtient_que_par_lanalyseur() {
        let a = authority_of("http://user:pw@127.0.0.1:8080/v1").expect("analysable");
        assert_eq!(a.host(), "127.0.0.1", "ni userinfo, ni port, ni chemin");
        assert!(a.is_loopback());
        // La cohérence des deux portes d'entrée, sur tout le corpus.
        for (entree, _) in CORPUS_HOTE {
            assert_eq!(
                authority_of(entree).map(|a| a.host()),
                host_of(entree),
                "`host_of` doit rester un simple raccourci : {entree}"
            );
        }
    }

    #[test]
    fn le_refus_est_toujours_la_direction_sure() {
        for (entree, attendu) in CORPUS_HOTE {
            if attendu.is_none() {
                assert!(
                    !is_local_endpoint(entree),
                    "autorité refusée ⟹ JAMAIS locale, sinon le refus deviendrait un laissez-passer : {entree}"
                );
            }
        }
    }
}
