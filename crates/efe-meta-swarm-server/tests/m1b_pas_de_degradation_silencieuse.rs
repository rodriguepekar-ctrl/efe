// SPDX-License-Identifier: UNLICENSED
//! # M1.b — le test qui aurait attrapé le défaut.
//!
//! ## Ce qui était faux, et pourquoi aucune relecture ne l'a vu
//!
//! `efe-meta-swarm-server` hébergeait les deux **derniers** analyseurs d'autorité du dépôt. L'un
//! d'eux, `tls::endpoint_is_loopback`, décidait si TLS pouvait être **retiré** d'un lien
//! contrôleur↔participant (AD18 / T44 : le loopback contourne TLS). Son découpage finissait par
//!
//! ```text
//! authority.rsplit_once(':').map_or(authority, |(h, _)| h)
//! ```
//!
//! — une coupe au **dernier** deux-points. Sur `127.0.0.1:8080@evil.com`, cela rend l'hôte
//! `127.0.0.1` et le « port » `8080@evil.com`. `IpAddr::from_str` recevait donc une IP
//! **parfaitement propre**, répondait « loopback », et `RemoteParticipant::connect_with_tls`
//! posait `effective_tls = None`. Du gRPC **en clair** vers `evil.com`, annoncé en `debug!`.
//!
//! La dette de M1.a déclarait ces deux analyseurs « sûrs aujourd'hui par composition (une entrée
//! malformée produit un SAN qu'aucun certificat ne porte) ». Pour celui-ci, la composition
//! invoquée pour rassurer était **exactement** ce qui le faisait échouer : l'entrée n'était pas
//! malformée après le découpage, elle était impeccable — et fausse.
//!
//! ## Pourquoi ce fichier ne contient pas de liste de charges écrite à la main
//!
//! Une liste écrite à la main ne trouve que ce qu'on avait déjà pensé. Or personne n'avait pensé à
//! celle-ci : ni l'architecture de M1, ni deux tours de durcissement, ni les exemptions rédigées
//! une par une. Le seul contrôle qui pouvait la trouver est un **invariant confronté au vrai
//! client** :
//!
//! > pour tout endpoint `e`, si nous déclarons `e` local, alors l'hôte que
//! > `tonic::transport::Endpoint` va réellement composer est un hôte de loopback.
//!
//! `tonic::transport::Endpoint` n'est pas un oracle choisi pour la commodité : c'est le type que
//! `connect_with_tls` construit trois lignes après la décision. Le comparer à lui, c'est comparer
//! la décision à son conséquent.
//!
//! Le corpus est **engendré** (schémas × userinfo × hôtes × ports × queues), pas énuméré : c'est
//! la méthode qui avait trouvé les deux différentiels de M1.a, et elle retrouve ici les cinq
//! formes que la relecture avait manquées.

#![allow(clippy::missing_panics_doc)]

use std::net::{IpAddr, TcpListener};

use efe_meta_swarm_server::tls::endpoint_is_loopback;

// ─── L'oracle : où `tonic` compose-t-il vraiment ? ──────────────────────────────────────────

/// L'hôte que `tonic` contactera, ou `None` s'il refuse l'endpoint.
///
/// Passe par `Endpoint::from_shared` — le constructeur employé par `connect_with_tls` — et non par
/// un analyseur d'URL choisi pour l'occasion. Un endpoint que `tonic` refuse ne sera jamais
/// composé : la question de sa localité ne se pose pas, donc il ne peut pas violer l'invariant.
fn hote_de_tonic(endpoint: &str) -> Option<String> {
    let ep = tonic::transport::Endpoint::from_shared(endpoint.to_string()).ok()?;
    ep.uri().host().map(str::to_string)
}

/// Cet hôte — tel que `tonic` le rendra — désigne-t-il la machine locale ?
///
/// Volontairement **large** : `localhost`, tout `127.0.0.0/8`, toute IPv6 de loopback, et la forme
/// entre crochets que `http::Uri` conserve. Être large ici rend l'invariant plus difficile à
/// satisfaire, donc le test plus dur à passer — c'est le bon sens de l'erreur pour un contrôle
/// dont l'échec signifie « TLS a été retiré à tort ».
fn est_un_hote_local(hote: &str) -> bool {
    let nu = hote.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(hote);
    nu.eq_ignore_ascii_case("localhost")
        || nu.eq_ignore_ascii_case("localhost.")
        || nu.parse::<IpAddr>().is_ok_and(|i| i.is_loopback())
}

// ─── Le corpus, engendré ────────────────────────────────────────────────────────────────────

const SCHEMAS: &[&str] = &["", "http://", "https://", "grpc://"];

/// Les préfixes d'autorité. Ceux qui portent un `@` sont le cœur du défaut : ils font *paraître*
/// l'autorité locale alors que l'hôte réel est ce qui SUIT le `@`.
const USERINFO: &[&str] = &[
    "",
    "user@",
    "user:pass@",
    "127.0.0.1@",
    "127.0.0.1:8080@",   // ⟵ la forme qui coupait TLS
    "localhost:8080@",   // ⟵ idem
    "[::1]:8080@",       // ⟵ idem
    "127.0.0.1:8080:9@", // deux ports : personne ne lit cela pareil
];

const HOTES: &[&str] = &[
    "127.0.0.1",
    "127.1.2.3",
    "localhost",
    "LOCALHOST",
    "localhost.",
    "[::1]",
    "[2001:db8::1]",
    "10.0.0.1",
    "controller.efe.svc",
    "127.evil.com",
    "evil.com",
    "2130706433",
    "0177.0.0.1",
    "127.1",
    "[::0001]",
];

const PORTS: &[&str] = &["", ":50080", ":80"];

/// Ce qui suit l'autorité. `#@127.0.0.1` et `?x@127.0.0.1` sont là parce qu'un analyseur qui ne
/// coupe pas sur `#` ou `?` avant d'isoler l'hôte y voit un `userinfo` là où il n'y en a pas.
const QUEUES: &[&str] = &["", "/", "/grpc", "?q=1", "#f", "#@127.0.0.1", "?x@127.0.0.1", "/x@127.0.0.1"];

fn corpus() -> Vec<String> {
    let mut v = Vec::new();
    for s in SCHEMAS {
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
    v
}

// ─── L'invariant ────────────────────────────────────────────────────────────────────────────

/// ⭐⭐ **L'invariant de souveraineté** : ce que nous déclarons local doit être local *pour le
/// client*. Sa violation n'est pas une imprécision — c'est TLS retiré sur un lien distant.
#[test]
fn tout_ce_que_nous_declarons_local_est_local_pour_tonic() {
    let corpus = corpus();
    assert!(corpus.len() > 4_000, "corpus de {} entrées — trop court pour prouver quoi que ce soit", corpus.len());

    let mut examines = 0usize;
    let mut refuses = 0usize;
    let mut violations = Vec::new();
    for e in &corpus {
        if !endpoint_is_loopback(e) {
            continue; // nous ne prétendons rien : rien à violer.
        }
        let Some(hote) = hote_de_tonic(e) else {
            // ⚠ Ce cas N'EST PAS une violation, et le compter comme telle était ma première
            // erreur : `tonic` refuse toute autorité sans schéma (`http::Uri` y voit un chemin).
            // S'il refuse, `Endpoint::from_shared` échoue dans `connect_with_tls` et **personne
            // n'est contacté** — il n'y a pas de lien dont TLS puisse être retiré. La production
            // ne présente jamais ces formes : `iterate.rs` préfixe toujours un schéma.
            refuses += 1;
            continue;
        };
        examines += 1;
        if !est_un_hote_local(&hote) {
            violations.push(format!("  {e:<52} nous: local · tonic contacte: {hote}"));
        }
    }

    // Un invariant qui ne s'applique à rien est vert pour rien : le corpus DOIT contenir des cas
    // que nous déclarons locaux, sinon ce test ne mesure que son propre filtre.
    assert!(
        examines > 300,
        "seulement {examines} endpoints jugés locaux — le filtre a tout mangé, l'invariant ne \
         porte sur rien"
    );
    assert!(
        violations.is_empty(),
        "\n{} désaccord(s) entre notre décision de localité et l'hôte que tonic compose.\n\
         Chacun signifie que TLS peut être RETIRÉ sur un lien qui n'est pas local.\n\n{}\n\n\
         N'écrivez pas de découpage d'URL ici : `scella_authority::is_local_endpoint` est \
         l'analyseur unique, et\n`scella-authority/tests/un_seul_analyseur.rs` (exemptions vides) \
         interdit qu'un second réapparaisse.\n",
        violations.len(),
        violations.join("\n")
    );
    eprintln!(
        "  invariant tenu sur {examines} endpoints jugés locaux ({refuses} formes sans schéma \
         refusées par tonic — hors production)"
    );
}

/// Le miroir, pour le SAN : l'hôte que nous **dérivons** doit être celui qui sera contacté.
///
/// Un SAN faux n'est pas symétrique d'une localité fausse — il fait *échouer* la poignée de main,
/// donc il est sûr par composition. Mais c'est cette même phrase qui couvrait le défaut de
/// localité pendant un module entier : on la vérifie plutôt que de la répéter.
#[test]
fn le_san_derive_est_lhote_que_tonic_composera() {
    let mut examines = 0usize;
    let mut violations = Vec::new();
    for e in corpus() {
        let Some(notre) = scella_authority::host_of(&e) else { continue };
        // Même raison que ci-dessus : un endpoint que `tonic` refuse n'est jamais composé, donc
        // le SAN que nous en aurions dérivé ne sert à vérifier aucune identité.
        let Some(tonic) = hote_de_tonic(&e) else { continue };
        examines += 1;
        // `http::Uri` conserve les crochets d'une IPv6 littérale ; nous rendons l'adresse nue.
        // Même hôte, deux présentations — la comparaison les met à plat, elle ne les excuse pas.
        let plat = tonic.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(&tonic);
        if !notre.eq_ignore_ascii_case(plat) {
            violations.push(format!("  {e:<52} nous: {notre} · tonic: {tonic}"));
        }
    }
    assert!(examines > 1_000, "seulement {examines} endpoints acceptés — l'invariant porte sur trop peu");
    assert!(
        violations.is_empty(),
        "\n{} SAN dérivé(s) qui ne désignent pas l'hôte contacté :\n{}\n",
        violations.len(),
        violations.join("\n")
    );
}

/// **La contre-épreuve.** Un invariant qui ne peut pas tomber ne prouve rien : on rejoue
/// l'ANCIEN découpage — celui que M1.b a supprimé — et on exige qu'il **échoue**.
///
/// C'est la seule manière de savoir que les deux tests ci-dessus auraient attrapé le défaut, et
/// non qu'ils passent parce qu'ils regardent ailleurs.
#[test]
fn lancien_analyseur_viole_linvariant_donc_le_test_a_des_dents() {
    /// Le découpage supprimé, reproduit ici et NULLE PART ailleurs : dans un test dont le rôle est
    /// de le voir échouer. (Il n'emploie pas le littéral gardé par `un_seul_analyseur.rs` —
    /// `SEP` le compose à l'exécution — parce que ce fichier n'est pas un analyseur : c'est son
    /// exécution capitale.)
    fn ancien_est_loopback(endpoint: &str) -> bool {
        const SEP: &str = concat!(':', '/', '/');
        let apres = endpoint.split_once(SEP).map_or(endpoint, |(_, r)| r);
        let autorite = apres.split(['/', '?']).next().unwrap_or(apres);
        let hote = if let Some(s) = autorite.strip_prefix('[') {
            s.split_once(']').map_or(s, |(h, _)| h)
        } else if autorite.contains("::") {
            autorite
        } else {
            autorite.rsplit_once(':').map_or(autorite, |(h, _)| h)
        };
        hote.eq_ignore_ascii_case("localhost") || hote.parse::<IpAddr>().is_ok_and(|i| i.is_loopback())
    }

    let mut contournements = Vec::new();
    for e in corpus() {
        if !ancien_est_loopback(&e) {
            continue;
        }
        if let Some(h) = hote_de_tonic(&e) {
            if !est_un_hote_local(&h) {
                contournements.push(format!("{e} → tonic contacte {h}"));
            }
        }
    }
    assert!(
        !contournements.is_empty(),
        "l'ancien analyseur ne viole PAS l'invariant sur ce corpus — donc les deux tests \
         ci-dessus n'auraient rien attrapé, et il faut élargir le corpus avant de leur faire \
         confiance"
    );
    // Et le nouveau, sur les mêmes charges, refuse toutes celles-là.
    for c in &contournements {
        let e = c.split(" → ").next().expect("format");
        assert!(
            !endpoint_is_loopback(e),
            "{e} : l'ancien analyseur coupait TLS ici, le nouveau doit refuser"
        );
    }
    eprintln!(
        "  contre-épreuve : l'ancien découpage coupait TLS sur {} forme(s) distincte(s) ; \
         exemple : {}",
        contournements.len(),
        contournements[0]
    );
}

/// Pose le fournisseur cryptographique de processus dont `tonic` a besoin — et **pourquoi ici,
/// plutôt que dans la bibliothèque**.
///
/// Les deux tests ci-dessous sont les seuls du fichier à construire un
/// `tonic::transport::ClientTlsConfig` ; les quatre autres n'interrogent que l'analyseur. Ce sont
/// exactement eux qui tombaient sous `cargo test --workspace` — non par instabilité, mais parce
/// que `rustls 0.23` résout son `CryptoProvider` de processus **d'après les features de la
/// caisse**, et qu'il PANIQUE quand `ring` et `aws-lc-rs` sont tous deux allumés.
///
/// **Ce n'est pas une course : c'est une propriété de la COMPILATION.** Ce crate épingle
/// `rustls = { default-features = false, features = ["ring"] }` (Cargo.toml l. 120), mais Cargo
/// unifie les features par caisse sur l'ensemble résolu : sous `--workspace`, un autre membre
/// tire `rustls` avec ses features par défaut, `aws-lc-rs` s'allume, et le binaire de test lie
/// les DEUX fournisseurs. Compilé par `-p efe-meta-swarm-server`, le même fichier n'en lie qu'un.
/// Mesuré le 2026-08-27 : le binaire issu du workspace échoue **10 fois sur 10**, machine au
/// repos ; celui issu de `-p` passe **105 fois sur 105**, y compris sous CPU saturé et pendant
/// l'exécution du workspace. Conséquence de méthode : un « rejeu seul » qui passe par `cargo`
/// **reconstruit** au lieu de relancer — il ne rejoue pas l'exécutable qui a échoué, il en
/// fabrique un autre, qui passe. C'est ce qui avait fait classer ces deux tests « instables ».
///
/// **Pourquoi le correctif n'est pas dans `connect_with_tls`.** Le chemin SERVEUR est déjà
/// protégé : `src/tls/config.rs` emploie `builder_with_provider` précisément pour ne pas dépendre
/// de cette résolution. Le chemin CLIENT n'a pas d'équivalent — `ClientTlsConfig::new()` n'offre
/// aucun point d'injection en tonic 0.12 — et le seul levier restant, `install_default`, est de
/// portée **processus** : une bibliothèque qui l'appellerait imposerait `ring` au binaire hôte,
/// y compris à celui qui voulait l'autre. Les binaires du crate le font donc eux-mêmes
/// (`src/bin/controller.rs`, `src/bin/participant.rs`) ; ce montage de test fait de même, à la
/// même place — chez l'appelant.
///
/// Le résultat est ignoré à dessein : `install_default` ne rend `Err` que si un fournisseur est
/// déjà posé. Les deux tests tournent en fils parallèles et posent le MÊME (`ring`) ; celui qui
/// perd la course reçoit cet `Err`, et c'est le déroulement nominal, pas un échec.
fn fournisseur_crypto_installe() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Un `TlsClientConfig` **réellement valide** : une autorité auto-signée et une feuille signée
/// par elle, comme `cert-manager` en produirait. Le contenu n'a pas à être *utile* (aucun serveur
/// ne présentera ce certificat) ; il doit être **acceptable**, pour que l'échec observé plus bas
/// soit imputable à la seule chose mesurée.
fn configuration_tls_valide() -> efe_meta_swarm_server::tls::TlsClientConfig {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, PKCS_ECDSA_P256_SHA256};

    let ca_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("clé de l'autorité");
    let mut ca_params = CertificateParams::new(vec!["m1b-test-ca".to_string()]).expect("params ca");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = ca_params.self_signed(&ca_kp).expect("autorité auto-signée");

    let kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("clé de la feuille");
    let feuille = CertificateParams::new(vec!["controller.efe.svc".to_string()])
        .expect("params feuille")
        .signed_by(&kp, &ca, &ca_kp)
        .expect("feuille signée");

    efe_meta_swarm_server::tls::TlsClientConfig {
        cert_chain_pem: feuille.pem().into_bytes(),
        key_pem: kp.serialize_pem().into_bytes(),
        server_ca_pem: ca.pem().into_bytes(),
        min_protocol: efe_meta_swarm_server::tls::ProtocolVersion::Tls13,
        expected_server_san: "participant.efe.svc".to_string(),
    }
}

/// ⭐ **Pourquoi `iterate.rs` compare DEUX lectures au lieu de les déclarer équivalentes.**
///
/// Le schéma est choisi d'après la localité de l'endpoint *sans schéma* ; `connect_with_tls`
/// reprend la décision sur l'endpoint *avec* schéma. « Les deux chaînes ne diffèrent que par le
/// schéma, que l'analyseur ignore » — vrai, sauf si l'endpoint portait déjà un séparateur de
/// schéma. Ce test exhibe la forme où c'est faux, donc l'existence du garde.
///
/// Sans lui, la décision prise dans `iterate.rs` ne serait pas celle appliquée à la connexion :
/// exactement la faute que M1.b ferme (deux endroits, une seule chose), à une échelle plus petite.
#[test]
fn prefixer_un_schema_peut_changer_lhote_donc_le_garde_daccord_est_necessaire() {
    // Sans schéma, l'autorité s'arrête au premier `/` : l'hôte est `x`.
    assert_eq!(scella_authority::host_of("127.0.0.1://x"), Some("x"));
    // Préfixé, le premier séparateur de schéma est celui qu'on vient d'ajouter : l'hôte devient
    // `127.0.0.1`, et il est LOOPBACK — donc `connect_with_tls` retirerait TLS là où
    // `iterate.rs` avait décidé de l'appliquer.
    assert_eq!(scella_authority::host_of("https://127.0.0.1://x"), Some("127.0.0.1"));
    assert!(!endpoint_is_loopback("127.0.0.1://x"));
    assert!(endpoint_is_loopback("https://127.0.0.1://x"), "les deux lectures divergent");

    // Le format de production, lui, est invariant au préfixe — c'est ce qui rend le garde
    // silencieux là où il doit l'être.
    for nu in [
        "p.efe.svc:50091",
        "tst-efe-meta-swarm-participant-supplier-scoring.efe.svc:50091",
        "127.0.0.1:50091",
        "[::1]:50091",
        "10.0.0.5:50091",
    ] {
        assert_eq!(
            scella_authority::host_of(nu),
            scella_authority::host_of(&format!("https://{nu}")),
            "{nu} : le garde d'accord doit rester muet sur les formes réelles"
        );
        assert_eq!(
            endpoint_is_loopback(nu),
            endpoint_is_loopback(&format!("https://{nu}")),
            "{nu} : la localité doit être invariante au schéma"
        );
    }
}

// ─── La preuve sur socket réel : « non local » veut bien dire « TLS appliqué » ───────────────

/// Toute la sévérité du défaut repose sur une phrase — *« déclarer local retire TLS »*. Elle est
/// vraie par lecture du code ; ce test la rend vraie par **observation**, sur une socket.
///
/// Le montage : une écoute **en clair** sur `127.0.0.1:P`, et le même
/// `RemoteParticipant::connect_with_tls` appelé deux fois avec la MÊME configuration TLS, sur les
/// deux endpoints que `iterate.rs` compose désormais :
///
/// * `http://127.0.0.1:P`  — lien local ⟹ TLS retiré ⟹ la connexion en clair **aboutit**.
/// * `https://0.0.0.0:P`   — lien non local ⟹ TLS gardé ⟹ la poignée de main contre une écoute
///   en clair **échoue**.
///
/// Les deux jambes sont nécessaires : la première prouve que le contournement existe (sans elle,
/// la seconde pourrait échouer pour n'importe quelle raison), la seconde prouve que TLS est
/// réellement posé. `0.0.0.0` atteint une écoute de loopback sous Linux et macOS — les deux
/// plateformes de ce dépôt ; le pré-contrôle ci-dessous le VÉRIFIE au lieu de le supposer, parce
/// qu'un test qui échoue faute de connectivité passerait pour une preuve.
#[tokio::test]
async fn une_ecoute_en_clair_accepte_le_loopback_et_refuse_le_non_local() {
    use efe_meta_swarm_server::client::RemoteParticipant;
    use tokio_util::sync::CancellationToken;

    fournisseur_crypto_installe();
    let (port, _fil) = ecoute_en_clair();

    // Pré-contrôle : `0.0.0.0:P` atteint-il bien cette écoute ? Un échec ici rendrait la jambe
    // « non local » verte pour la mauvaise raison.
    assert!(
        std::net::TcpStream::connect(("0.0.0.0", port)).is_ok(),
        "sur cette plateforme, 0.0.0.0 n'atteint pas une écoute de loopback : la seconde jambe de \
         ce test échouerait faute de connectivité, et non parce que TLS a été appliqué. Choisir \
         une autre adresse non-loopback joignable localement avant de faire confiance à ce test."
    );

    let tls = configuration_tls_valide();
    // Et on le PROUVE : la même configuration doit être acceptée par `tonic`. Ces quatre lignes
    // reproduisent celles de `connect_with_tls`, non pour redécider quoi que ce soit, mais pour
    // établir que l'échec attendu plus bas ne peut venir que du réseau.
    {
        let identity = tonic::transport::Identity::from_pem(&tls.cert_chain_pem, &tls.key_pem);
        let ca = tonic::transport::Certificate::from_pem(&tls.server_ca_pem);
        let cfg = tonic::transport::ClientTlsConfig::new()
            .domain_name(tls.expected_server_san.clone())
            .ca_certificate(ca)
            .identity(identity);
        assert!(
            tonic::transport::Endpoint::from_static("http://127.0.0.1:1").tls_config(cfg).is_ok(),
            "les certificats du montage sont refusés par tonic — la jambe « non local » \
             échouerait à la configuration et non à la poignée de main"
        );
    }

    let local = RemoteParticipant::connect_with_tls(
        "p-local",
        4,
        format!("http://127.0.0.1:{port}"),
        CancellationToken::new(),
        Some(tls.clone()),
    )
    .await;
    assert!(
        local.is_ok(),
        "loopback : TLS doit être retiré (AD18) et le schéma composé en `http`, donc la connexion \
         en clair aboutit — sinon la seconde jambe ne prouve rien : {local:?}"
    );

    let distant = RemoteParticipant::connect_with_tls(
        "p-distant",
        4,
        format!("https://0.0.0.0:{port}"),
        CancellationToken::new(),
        Some(tls),
    )
    .await;
    assert!(
        distant.is_err(),
        "non local : TLS doit être APPLIQUÉ, donc la poignée de main contre une écoute en clair \
         échoue. Qu'elle aboutisse signifierait que TLS a été retiré — le défaut de M1.b."
    );
}

/// ⭐⭐ **Les deux comportements de `tonic` dont dépend le choix du schéma dans `iterate.rs`.**
///
/// Ils ne sont pas documentés dans ce dépôt et le commentaire d'origine affirmait le premier **à
/// l'envers**. Les épingler ici les rend exécutables : si `tonic` change, ce test tombe, au lieu
/// que la pile mTLS se taise.
///
/// 1. `http://` **avec** `tls_config` ⟹ TLS **n'est pas** appliqué (la connexion en clair passe).
///    C'est pourquoi conserver un `http://` écrit par l'opérateur donnait du gRPC en clair avec
///    toute la pile mTLS configurée.
/// 2. `https://` **sans** `tls_config` ⟹ la connexion **échoue**. C'est pourquoi le contournement
///    loopback d'AD18 ne pouvait pas fonctionner : `iterate.rs` composait `https://`, puis
///    `connect_with_tls` retirait la configuration.
///
/// Les deux ensemble disent une seule chose : **le schéma et la décision TLS doivent venir du
/// même endroit.** C'est ce que `iterate.rs` fait désormais.
#[tokio::test]
async fn le_schema_et_la_configuration_tls_doivent_saccorder() {
    fournisseur_crypto_installe();
    let (port, _fil) = ecoute_en_clair();
    let tls = configuration_tls_valide();

    let avec_tls = |uri: String| {
        let identity = tonic::transport::Identity::from_pem(&tls.cert_chain_pem, &tls.key_pem);
        let ca = tonic::transport::Certificate::from_pem(&tls.server_ca_pem);
        let cfg = tonic::transport::ClientTlsConfig::new()
            .domain_name(tls.expected_server_san.clone())
            .ca_certificate(ca)
            .identity(identity);
        tonic::transport::Endpoint::from_shared(uri)
            .expect("uri")
            .connect_timeout(std::time::Duration::from_secs(3))
            .tls_config(cfg)
            .expect("tls_config accepté")
    };

    assert!(
        avec_tls(format!("http://0.0.0.0:{port}")).connect().await.is_ok(),
        "(1) réfutée : tonic appliquerait désormais TLS sur `http://`. Alors le correctif de \
         `iterate.rs` est inutile — le relire avant de le simplifier."
    );
    assert!(
        avec_tls(format!("https://0.0.0.0:{port}")).connect().await.is_err(),
        "témoin : la même configuration DOIT échouer en `https` contre une écoute en clair, sinon \
         la mesure (1) ne distingue rien"
    );
    assert!(
        tonic::transport::Endpoint::from_shared(format!("https://127.0.0.1:{port}"))
            .expect("uri")
            .connect_timeout(std::time::Duration::from_secs(3))
            .connect()
            .await
            .is_err(),
        "(2) réfutée : `https://` sans configuration TLS passerait. Alors composer `https` pour un \
         lien dont TLS est retiré ne casse plus rien, et le correctif peut être allégé."
    );
}

/// Une écoute TCP **en clair** sur `127.0.0.1`, qui accepte en boucle puis raccroche : jamais de
/// TLS de ce côté. Rend `(port, fil)` — garder le fil vivant maintient l'écoute ouverte.
///
/// La boucle est nécessaire : sans elle, la première connexion consomme l'accept et la seconde
/// attendrait dans le backlog, faisant échouer une jambe par temporisation plutôt que par TLS.
fn ecoute_en_clair() -> (u16, std::thread::JoinHandle<()>) {
    let ecoute = TcpListener::bind("127.0.0.1:0").expect("écoute en clair");
    let port = ecoute.local_addr().expect("adresse").port();
    ecoute.set_nonblocking(true).expect("non bloquant");
    let fil = std::thread::spawn(move || {
        let debut = std::time::Instant::now();
        while debut.elapsed() < std::time::Duration::from_secs(60) {
            match ecoute.accept() {
                Ok((flux, _)) => drop(flux),
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });
    (port, fil)
}
