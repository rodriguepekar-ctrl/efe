// SPDX-License-Identifier: UNLICENSED
//! # L'accord avec le client HTTP réel n'est pas espéré : il est **prouvé**.
//!
//! ## Ce que ce test remplace
//!
//! L'analyseur refuse les autorités contenant `\`, un espace ou un caractère de contrôle. Cette
//! liste est une **liste noire**, et une liste noire n'est jamais complète : elle contient ce à
//! quoi quelqu'un a pensé. L'historique le montre — il a fallu deux tours de durcissement, et le
//! second a été trouvé par une revue, pas par un test.
//!
//! Ce test convertit la liste noire en **propriété**. `url` est la bibliothèque sur laquelle
//! `reqwest` s'appuie : c'est donc l'oracle de « quel hôte sera réellement contacté ». L'invariant
//! vérifié sur un corpus engendré est :
//!
//! > pour tout endpoint *x* que `url` sait analyser :
//! > `authority_of(x)` est **`None`**, **ou** son hôte est celui que `url` désigne.
//!
//! Jamais un troisième cas — jamais « nous désignons un autre hôte que celui appelé », qui est la
//! définition même du différentiel d'analyseur.
//!
//! ## Pourquoi la propriété est bornée aux entrées que `url` analyse
//!
//! Elle ne peut pas être totale, et le prétendre serait malhonnête. `url` refuse les formes **sans
//! schéma** (`127.0.0.1:8088`), que ce dépôt emploie couramment — adresses d'écoute, endpoints
//! gRPC. Sur celles-là il n'y a pas d'oracle : elles sont couvertes par le corpus écrit à la main
//! du crate, et par le contrôle de cohérence ci-dessous qui exige que `x` et `http://x` donnent le
//! même hôte.
//!
//! ## Pourquoi `url` reste en `dev-dependencies`
//!
//! Le crate livré doit rester une **feuille sans dépendance** : c'est ce qui lui permet d'être pris
//! par le périmètre preuve (une seule arête aujourd'hui) et par le périmètre essaim sans les
//! souder. Bâtir *sur* `url` donnerait l'accord par construction, mais ferait entrer `idna` et
//! `unicode-*` dans une pile dont la maigreur est l'argument. On garde donc l'analyseur à la main,
//! et on **prouve** son accord au lieu de l'espérer.

use scella_authority::{authority_of, host_of, is_local_endpoint};
use url::Url;

/// Deux hôtes désignent la même machine s'ils ne diffèrent que par la casse, les crochets d'une
/// IPv6 littérale, ou un point final de FQDN. `url` normalise ces trois choses, notre analyseur
/// rend l'entrée telle quelle : la comparaison doit donc les neutraliser des deux côtés, sans quoi
/// on mesurerait une différence de présentation là où il n'y a aucune différence de destination.
fn meme_hote(a: &str, b: &str) -> bool {
    let n = |s: &str| {
        let s = s.trim_start_matches('[').trim_end_matches(']');
        s.strip_suffix('.').unwrap_or(s).to_ascii_lowercase()
    };
    n(a) == n(b)
}

const SCHEMES: &[&str] = &["http://", "https://"];
const USERINFO: &[&str] = &["", "user@", "user:pass@", "127.0.0.1@", "user:127.0.0.1@", "@"];
const HOTES: &[&str] = &[
    "127.0.0.1", "localhost", "LOCALHOST", "evil.com", "127.evil.com", "localhost.evil.com",
    "10.0.0.3", "[::1]", "[fe80::1]", "127.0.0.1.", "192.168.1.5", "127.255.0.1",
];
const PORTS: &[&str] = &["", ":8080", ":1", ":11434"];
const QUEUES: &[&str] = &[
    "", "/", "/v1", "/x@127.0.0.1", "?q=1", "?x@127.0.0.1", "#f", "#@127.0.0.1",
    "#@localhost", "/v1?a=b#c", "/a://b",
];

/// **Les charges hostiles, sans lesquelles ce test ne prouverait rien d'intéressant.**
///
/// La première rédaction de ce fichier n'engendrait que des combinaisons saines — et ne produisait
/// donc **aucun refus**, ce qu'une assertion de couverture a heureusement signalé. Une propriété
/// « None ou égal » vérifiée uniquement sur des entrées bénignes ne dit rien du durcissement.
///
/// Ces entrées-ci sont exactement la région dangereuse : `url` (donc `reqwest`) les analyse — le
/// WHATWG normalise `\` en `/` — et y lit un hôte qui n'est PAS le leurre. Un analyseur naïf y
/// lirait `127.0.0.1` et conclurait au loopback. Le nôtre doit soit dire le même hôte que `url`,
/// soit refuser ; les deux sont sûrs, et le troisième cas est la faille.
const HOSTILES: &[&str] = &[
    "http://evil.com\\@127.0.0.1/v1",
    "http://evil.com\\@localhost",
    "http://evil.com\\@[::1]",
    "https://evil.com\\@127.0.0.1",
    "http://evil.com\\\\@127.0.0.1",
    "http://127.0.0.1\\@evil.com",
    "http://evil.com\t@127.0.0.1",
    "http://evil.com\n@127.0.0.1",
    "http://evil.com\r@127.0.0.1",
    "http://127.0.0.1 @evil.com",
    "http://evil.com#@127.0.0.1",
    "http://evil.com?x@127.0.0.1",
    "http://evil.com/x@127.0.0.1",
    "http://127.0.0.1@evil.com",
    "http://user:127.0.0.1@evil.com",
    "http://127.evil.com",
    "http://127.0.0.1.evil.com",
    "http://localhost.evil.com",
    "http://2130706433",
    "http://0177.0.0.1",
    "http://[::0001]:8080",
    "http://evil.com:80#@127.0.0.1:8080",
];

/// Le corpus complet : le produit combinatoire, plus les charges hostiles.
fn corpus() -> Vec<String> {
    let mut v: Vec<String> = Vec::with_capacity(8_000);
    for s in SCHEMES {
        for u in USERINFO {
            for h in HOTES {
                for p in PORTS {
                    for q in QUEUES {
                        v.push(format!("{s}{u}{h}{p}{q}"));
                    }
                }
            }
        }
    }
    v.extend(HOSTILES.iter().map(|s| (*s).to_string()));
    v
}

#[test]
fn jamais_un_hote_different_de_celui_qui_sera_appele() {
    let mut accords = 0_usize;
    let mut refus = 0_usize;
    let mut sans_oracle = 0_usize;

    for entree in corpus() {
        let Ok(parse) = Url::parse(&entree) else {
            sans_oracle += 1;
            continue;
        };
        let Some(oracle) = parse.host_str() else {
            sans_oracle += 1;
            continue;
        };
        match authority_of(&entree) {
            // Refuser est toujours permis : c'est la direction sûre, et le crate garantit par
            // ailleurs qu'un refus n'est jamais « local ».
            None => {
                refus += 1;
                assert!(
                    !is_local_endpoint(&entree),
                    "un refus ne doit JAMAIS devenir un laissez-passer : {entree}"
                );
            }
            Some(a) => {
                assert!(
                    meme_hote(a.host(), oracle),
                    "\nDIFFÉRENTIEL D'ANALYSEUR — c'est exactement le bug que ce crate existe \
                     pour rendre impossible.\n  \
                     entrée   : {entree:?}\n  \
                     nous     : {}\n  \
                     appelé   : {oracle}\n",
                    a.host()
                );
                accords += 1;
            }
        }
    }

    // Sans ces gardes, un analyseur qui rendrait TOUJOURS `None` passerait ce test avec les
    // honneurs : la propriété « None ou égal » serait vraie, et parfaitement vide de sens.
    let total = accords + refus;
    assert!(total > 2_000, "corpus trop petit pour prouver quoi que ce soit : {total}");
    assert!(
        accords * 2 > total,
        "la MAJORITÉ des entrées analysables doit produire un accord, pas un refus — sinon la \
         propriété est satisfaite par un analyseur inutile ({accords} accords / {total})"
    );
    // Et il DOIT rester des refus : ce sont les charges que `url` analyse et que nous écartons
    // faute de pouvoir garantir la même lecture. Zéro refus signifierait que le corpus ne contient
    // aucune entrée hostile — le défaut exact de la première rédaction de ce fichier.
    assert!(
        refus > 0,
        "aucun refus : le corpus ne contient donc aucune charge hostile, et le durcissement n'est \
         pas éprouvé"
    );
    eprintln!("  accord sur {accords} entrées · {refus} refus · {sans_oracle} sans oracle");
}

/// Les charges connues, une par une, avec leur oracle : ce sont elles qui ont motivé le
/// durcissement, et les voir passer par `url` prouve que la lecture « hôte réel » est la bonne.
#[test]
fn les_charges_de_differentiel_ont_bien_evil_com_pour_hote_reel() {
    for entree in [
        "http://evil.com#@127.0.0.1",
        "http://evil.com?x@127.0.0.1",
        "http://evil.com/x@127.0.0.1",
        "http://127.0.0.1@evil.com",
        "http://user:127.0.0.1@evil.com",
    ] {
        let oracle = Url::parse(entree).expect("analysable").host_str().unwrap().to_string();
        assert_eq!(oracle, "evil.com", "l'oracle confirme l'hôte réel : {entree}");
        assert_eq!(host_of(entree), Some("evil.com"), "et nous disons la même chose : {entree}");
        assert!(!is_local_endpoint(entree), "donc jamais local : {entree}");
    }
}

/// Les formes **sans schéma** n'ont pas d'oracle — `url` les refuse. La cohérence interne prend le
/// relais : ajouter un schéma ne doit pas changer l'hôte, sinon deux appelants du dépôt (adresse
/// d'écoute d'un côté, URL de l'autre) verraient deux hôtes pour la même machine.
#[test]
fn une_forme_sans_schema_designe_le_meme_hote_quavec_schema() {
    for nu in ["127.0.0.1:8088", "localhost:8088", "10.0.0.5:8088", "evil.com", "[::1]:9000"] {
        let avec = format!("http://{nu}");
        assert_eq!(
            host_of(nu),
            host_of(&avec),
            "l'ajout d'un schéma ne doit rien changer à l'hôte : {nu}"
        );
        assert_eq!(is_local_endpoint(nu), is_local_endpoint(&avec), "ni à la localité : {nu}");
    }
}

/// Le non-ASCII est refusé, et le test dit pourquoi : `url` applique l'IDNA et contacte la forme
/// punycode, que nous ne calculons pas. Rendre l'hôte tel quel affirmerait un hôte **différent**
/// de celui appelé — précisément ce que la propriété interdit.
#[test]
fn le_non_ascii_est_refuse_parce_que_le_client_contacte_le_punycode() {
    for entree in ["http://éxample.com/v1", "http://ex\u{e4}mple.com", "http://münchen.de"] {
        let oracle = Url::parse(entree).ok().and_then(|u| u.host_str().map(str::to_string));
        assert!(
            oracle.as_deref().is_some_and(|h| h.starts_with("xn--")),
            "l'oracle confirme que le client contacte du punycode : {entree} → {oracle:?}"
        );
        assert_eq!(host_of(entree), None, "nous refusons plutôt que d'affirmer autre chose");
        assert!(!is_local_endpoint(entree));
    }
}
