// SPDX-License-Identifier: UNLICENSED
//! # `scella-preuve` — M4 : la démonstration en **une** commande.
//!
//! ## Ce que ce binaire doit prouver, et à qui
//!
//! Le critère de M4 tient en une phrase : *« machine vierge, `docker compose up` → un reçu
//! re-prouvé, **sans clé** »*. Chaque mot y compte.
//!
//! * **machine vierge** — rien d'installé. Pas de Rust, pas de `cargo`, pas d'assets. Docker seul.
//! * **une commande** — pas un script en cinq étapes qu'il faut lire avant de l'exécuter.
//! * **un reçu re-prouvé** — pas « la passerelle a démarré », pas « les tests passent ». Un run
//!   réel, un reçu réel, et une vérification qui pourrait échouer.
//! * **sans clé** — et c'est là que se trouve la propriété qui vaut quelque chose.
//!
//! ## Pourquoi « sans clé » n'est pas une commodité mais la démonstration entière
//!
//! Le sceau d'une chaîne HMAC a une faiblesse que rien, à l'intérieur de la chaîne, ne trahit :
//! **le préfixe d'une chaîne valide est lui-même une chaîne valide**. Un bundle tronqué se
//! re-prouve donc *parfaitement*. Celui qui détient la clé ne voit rien ; celui qui ne l'a pas non
//! plus. La coupure est invisible de l'intérieur.
//!
//! Le seul témoin possible est **hors** du bundle : une ancre publiée au moment de l'export, qui
//! annonce la racine et le **nombre de nœuds**. Confronter le bundle à son ancre est alors un
//! **comptage** — pas une opération cryptographique, pas un acte de confiance. Un auditeur le fait
//! avec `grep -c` et `awk`, sans qu'on lui confie quoi que ce soit.
//!
//! C'est cela que ce binaire met en scène, et c'est la seule partie de la démonstration qui ne
//! demande à personne d'accorder sa confiance à qui que ce soit.
//!
//! ## Ce que ce binaire n'est pas
//!
//! Il n'est **pas** le vérifieur. Le vérifieur, c'est [`verify_mini`] — arbre de dépendances
//! **vide**, lisible en entier par un auditeur avant qu'il ne le croie. Ce pilote-ci, lui, prend
//! `serde_json` sans état d'âme : personne n'a à le croire. Tout ce qu'il affirme se recontrôle à
//! la main, sur les fichiers qu'il laisse derrière lui — le bundle, sa copie tronquée, et l'ancre.
//! Les rendre tous deux zéro-dépendance aurait été du théâtre : cela aurait obligé à élargir le
//! parseur volontairement étroit de `verify-mini`, dont l'étroitesse **est** la propriété.
//!
//! ## Ce que la démonstration ne prouve PAS, et qu'elle doit dire
//!
//! Sans `EFE_NANO_AUDIT_HMAC_MASTER`, la passerelle démarre sur la **clé de test publique** — et
//! M0 l'annonce au démarrage, en toutes lettres, dans les journaux que `docker compose up` fait
//! défiler. Le reçu produit ici démontre donc le **mécanisme**, jamais la **substance** : sa clé
//! est connue de quiconque a lu le dépôt, et il ne prouve rien contre personne.
//!
//! Ce binaire le répète à la fin plutôt que de laisser un « ✓ » vert donner le change. Une
//! démonstration qui a l'air meilleure qu'elle n'est, c'est exactement le défaut que M0 existe
//! pour supprimer — il serait absurde de le réintroduire dans la vitrine.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────────────────────────────────────
//  Un client HTTP/1.1 minuscule, sur `std::net`
// ─────────────────────────────────────────────────────────────────────────────
//
//  Écrit à la main plutôt que tiré d'une caisse : ce binaire parle à UN serveur connu, sur deux
//  routes, en HTTP/1.1 sans TLS et sans redirection. Faire entrer un client asynchrone complet —
//  et son exécuteur — dans une image de démonstration coûterait plus que ces quatre-vingts lignes.

struct Reponse {
    statut: u16,
    corps: String,
}

/// Une requête HTTP/1.1 complète. `roles` alimente l'en-tête de rôles du chemin dev-headers.
fn requete(
    hote: &str,
    methode: &str,
    chemin: &str,
    roles: Option<&str>,
    corps: Option<&str>,
) -> Result<Reponse, String> {
    let mut flux = TcpStream::connect(hote).map_err(|e| format!("connexion à {hote} : {e}"))?;

    let mut tete = format!(
        "{methode} {chemin} HTTP/1.1\r\nHost: {hote}\r\nConnection: close\r\nAccept: application/json\r\n"
    );
    if let Some(r) = roles {
        tete.push_str(&format!("x-efe-roles: {r}\r\n"));
    }
    match corps {
        Some(c) => {
            tete.push_str("content-type: application/json\r\n");
            tete.push_str(&format!("Content-Length: {}\r\n\r\n", c.len()));
            tete.push_str(c);
        }
        None => tete.push_str("\r\n"),
    }
    flux.write_all(tete.as_bytes()).map_err(|e| format!("envoi : {e}"))?;
    flux.flush().ok();

    // `Connection: close` ⟹ le serveur ferme, donc lire jusqu'à EOF suffit et évite d'avoir à
    // gérer `Content-Length` ou le découpage en morceaux. C'est le seul luxe que s'autorise ce
    // client, et il est payé par une connexion par requête — sans conséquence pour une démo.
    let mut brut = Vec::new();
    flux.read_to_end(&mut brut).map_err(|e| format!("lecture : {e}"))?;
    let texte = String::from_utf8_lossy(&brut).into_owned();

    let (entetes, corps) = texte
        .split_once("\r\n\r\n")
        .ok_or_else(|| "réponse HTTP sans séparateur d'en-têtes".to_string())?;
    let statut: u16 = entetes
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| "ligne de statut illisible".to_string())?;

    Ok(Reponse { statut, corps: corps.to_string() })
}

/// Attend que la passerelle réponde, ou renonce. Un `docker compose up` démarre les deux services
/// en même temps : le pilote arrive presque toujours avant que le serveur n'écoute.
fn attendre_la_passerelle(hote: &str, essais: u32) -> Result<(), String> {
    for i in 0..essais {
        if let Ok(r) = requete(hote, "GET", "/healthz", None, None) {
            if r.statut == 200 {
                return Ok(());
            }
        }
        if i % 10 == 0 && i > 0 {
            eprintln!("  … la passerelle ne répond pas encore ({i} tentatives)");
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err(format!("la passerelle n'a jamais répondu sur {hote} après {essais} tentatives"))
}

// ─────────────────────────────────────────────────────────────────────────────
//  L'ancre : le témoin publié HORS du bundle
// ─────────────────────────────────────────────────────────────────────────────

/// Ce qu'une ligne d'ancre annonce. Format tabulé, écrit en append-only par la passerelle :
/// `version | run_id | tenant_id | racine | nombre_de_nœuds | horodatage_ms`.
struct Ancre {
    run_id: String,
    racine: String,
    noeuds: u64,
}

/// La **dernière** ancre du journal. Append-only : la dernière ligne est la plus récente, donc
/// celle de l'export qu'on vient de déclencher.
fn derniere_ancre(chemin: &Path) -> Result<Ancre, String> {
    let texte = std::fs::read_to_string(chemin)
        .map_err(|e| format!("journal d'ancres « {} » illisible : {e}", chemin.display()))?;
    let ligne = texte
        .lines()
        .filter(|l| !l.trim().is_empty())
        .next_back()
        .ok_or_else(|| "journal d'ancres vide — aucun export n'a publié d'ancre".to_string())?;
    let champs: Vec<&str> = ligne.split('\t').collect();
    if champs.len() < 6 {
        return Err(format!("ligne d'ancre à {} champs, 6 attendus", champs.len()));
    }
    Ok(Ancre {
        run_id: champs[1].to_string(),
        racine: champs[3].to_string(),
        noeuds: champs[4].parse().map_err(|_| format!("nombre de nœuds illisible : {}", champs[4]))?,
    })
}

/// Le nombre de lignes non vides — la mesure que fait un auditeur avec `grep -c '[^[:space:]]'`.
fn noeuds_du_bundle(jsonl: &str) -> u64 {
    jsonl.lines().filter(|l| !l.trim().is_empty()).count() as u64
}

// ─────────────────────────────────────────────────────────────────────────────
//  La démonstration
// ─────────────────────────────────────────────────────────────────────────────

fn env_ou(nom: &str, defaut: &str) -> String {
    std::env::var(nom).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| defaut.to_string())
}

fn main() {
    match demonstration() {
        Ok(()) => {}
        Err(e) => {
            eprintln!("\n✗ ÉCHEC : {e}");
            std::process::exit(1);
        }
    }
}

fn demonstration() -> Result<(), String> {
    let hote = env_ou("SCELLA_PASSERELLE", "127.0.0.1:8080");
    let sortie = PathBuf::from(env_ou("SCELLA_SORTIE", "/preuve"));
    let ancre_chemin = PathBuf::from(env_ou("SCELLA_ANCRE", "/preuve/anchors.log"));
    std::fs::create_dir_all(&sortie).map_err(|e| format!("dossier de sortie : {e}"))?;

    println!("═══ 1/5 · la passerelle répond ═══");
    attendre_la_passerelle(&hote, 120)?;
    println!("  {hote} — vivante.");
    println!("  Aucune clé n'a été fournie : le rapport de démarrage ci-dessus (journaux du");
    println!("  service « passerelle ») annonce donc « provenance DÉGRADÉE ». C'est voulu, et");
    println!("  c'est dit — on y revient au point 5.");

    println!("\n═══ 2/5 · un run réel, et son reçu ═══");
    let corps = r#"{"task":{"text":"Résumer la clause de résiliation du contrat fournisseur."},"topology":{"kind":"auto"}}"#;
    let r = requete(&hote, "POST", "/v1/swarm/run", Some("operator"), Some(corps))?;
    if r.statut != 200 {
        return Err(format!("le run a été refusé (HTTP {}) : {}", r.statut, r.corps));
    }
    let v: serde_json::Value =
        serde_json::from_str(&r.corps).map_err(|e| format!("réponse de run illisible : {e}"))?;
    let run_id = v["run_id"].as_str().ok_or("réponse de run sans `run_id`")?.to_string();
    let racine_servie = v["run"]["provenance"]["root"].as_str().unwrap_or("").to_string();
    println!("  run servi   : {run_id}");
    // **Deux racines vont apparaître dans cette démonstration, et elles ne mesurent pas le même
    // objet.** Celle-ci est la provenance du RUN, telle que la passerelle la sert. Celle du point 5
    // est celle du BUNDLE, dérivé du run par `GET /v1/swarm/bundle/:id` — c'est elle que l'ancre
    // engage et que `verify-mini` doit recevoir. Les afficher sans le dire ferait soupçonner une
    // incohérence à qui les compare, et cette démonstration existe précisément pour être comparée.
    println!(
        "  racine      : {}  (provenance du RUN — pas celle du bundle, cf. point 5)",
        if racine_servie.is_empty() { "(non exposée dans la réponse)" } else { &racine_servie }
    );
    // M2 : la réponse dit toujours ce que la capture a fait de ce run.
    if v["capture"]["persisted"] == serde_json::Value::Bool(false) {
        println!("  ⚠ capture   : NON PERSISTÉ — {}", v["capture"]["degrade_reason"].as_str().unwrap_or("?"));
    }

    println!("\n═══ 3/5 · l'export, et l'ancre publiée HORS du bundle ═══");
    let b = requete(&hote, "GET", &format!("/v1/swarm/bundle/{run_id}"), Some("auditor"), None)?;
    if b.statut != 200 {
        return Err(format!("export du bundle refusé (HTTP {}) : {}", b.statut, b.corps));
    }
    let bv: serde_json::Value =
        serde_json::from_str(&b.corps).map_err(|e| format!("réponse de bundle illisible : {e}"))?;
    let jsonl = bv["bundle_jsonl"].as_str().ok_or("bundle sans `bundle_jsonl`")?.to_string();
    let annonce_bundle = bv["node_count"].as_u64().ok_or("bundle sans `node_count`")?;
    let ancre_publiee = bv["anchored"].as_bool().unwrap_or(false);
    if !ancre_publiee {
        return Err(
            "le bundle n'est PAS ancré : sans ancre, la troncature reste indétectable et la \
             démonstration n'a plus d'objet. Vérifier EFE_SWARM_ANCHOR_LOG."
                .into(),
        );
    }
    let f_intact = sortie.join("bundle.sealed.jsonl");
    std::fs::write(&f_intact, &jsonl).map_err(|e| format!("écriture du bundle : {e}"))?;
    println!("  bundle écrit : {}", f_intact.display());
    println!("  il annonce   : {annonce_bundle} nœud(s)");
    println!("  ancré        : oui — l'ancre est publiée hors du bundle, à l'export.");

    println!("\n═══ 4/5 · la troncature, démasquée SANS CLÉ ═══");
    let a = derniere_ancre(&ancre_chemin)?;
    if a.run_id != run_id {
        return Err(format!(
            "la dernière ancre porte le run {} et non {run_id} — le journal est partagé avec un \
             autre export, la démonstration ne peut pas conclure",
            a.run_id
        ));
    }
    let intact = noeuds_du_bundle(&jsonl);

    // La coupure : on retire la dernière ligne. Le bundle tronqué se re-prouve PARFAITEMENT —
    // c'est tout le problème, et c'est pourquoi le comptage est la seule parade.
    let lignes: Vec<&str> = jsonl.lines().filter(|l| !l.trim().is_empty()).collect();
    if lignes.len() < 2 {
        return Err("le bundle n'a qu'un nœud : il n'y a rien à tronquer".into());
    }
    let tronque = lignes[..lignes.len() - 1].join("\n");
    let f_tronque = sortie.join("bundle.TRONQUE.jsonl");
    std::fs::write(&f_tronque, &tronque).map_err(|e| format!("écriture du bundle tronqué : {e}"))?;
    let coupe = noeuds_du_bundle(&tronque);

    println!("  l'ancre annonce            : {} nœud(s)", a.noeuds);
    println!("  le bundle intact contient  : {intact}   → concorde");
    println!("  le bundle tronqué contient : {coupe}   → CONTREDIT l'ancre");

    if intact != a.noeuds {
        return Err(format!(
            "le bundle intact ({intact}) ne concorde pas avec son ancre ({}) — le témoin est faux",
            a.noeuds
        ));
    }
    if annonce_bundle != a.noeuds {
        return Err(format!(
            "le bundle s'annonce à {annonce_bundle} nœuds et l'ancre à {} — les deux gardes se \
             contredisent",
            a.noeuds
        ));
    }
    if coupe == a.noeuds {
        return Err("la troncature n'a pas été détectée — l'ancre ne protège rien".into());
    }
    println!("  ↑ détection faite avec un COMPTAGE. Aucune clé, aucune cryptographie, aucune");
    println!("    confiance accordée à qui que ce soit. Refaites-la : `grep -c` sur les deux");
    println!("    fichiers, `awk -F'\\t' 'END{{print $5}}'` sur {}.", ancre_chemin.display());

    println!("\n═══ 5/5 · ce qui a été prouvé, et ce qui ne l'a pas été ═══");
    println!("  SANS clé, à l'instant :");
    println!("    · un reçu a été servi par une passerelle qui tourne en local ;");
    println!("    · son bundle concorde avec une ancre publiée hors bande ;");
    println!("    · une troncature a été démasquée par un comptage.");
    println!();

    match std::env::var("SCELLA_CLE_HEX").ok().filter(|k| !k.trim().is_empty()) {
        Some(cle_hex) => {
            let cle = hex_vers_octets(&cle_hex)?;
            match verify_mini::run(&jsonl, &cle, Some(&a.racine)) {
                Ok(_) => println!("  AVEC la clé fournie : la chaîne se re-prouve, racine {} ✓", a.racine),
                Err(e) => return Err(format!("re-preuve AVEC clé échouée : {e}")),
            }
        }
        None => {
            println!("  AVEC une clé — ce que cette démonstration ne fait PAS :");
            println!("    la re-preuve complète de la chaîne. Elle appartient au responsable de");
            println!("    traitement, chez lui, avec SA clé :");
            println!();
            println!("      verify-mini {} \\", f_intact.display());
            println!("        --key-hex <clé tenant> --root {}", a.racine);
            println!(
                "      (cette racine est celle du BUNDLE, engagée par l'ancre — pas celle du run\n\
                 \x20      affichée au point 2, qui porte sur un autre objet.)"
            );
        }
    }

    println!();
    println!("  ⚠ CE QUE CE REÇU NE PROUVE PAS. Aucune clé n'a été injectée : la passerelle a");
    println!("    tourné sur la clé de TEST PUBLIQUE, connue de quiconque a lu le dépôt. Ce reçu");
    println!("    démontre le MÉCANISME, jamais la SUBSTANCE — il ne prouve rien contre personne.");
    println!("    Pour qu'il oppose quelque chose à quelqu'un, il faut une clé que vous seul");
    println!("    détenez : EFE_NANO_AUDIT_HMAC_MASTER, injectée au déploiement.");
    println!();
    println!("    Et ce qu'un reçu atteste, même avec une vraie clé, reste l'INTÉGRITÉ de ce qui");
    println!("    a été enregistré — jamais la CORRECTION de la décision.");
    println!();
    println!("═══ DÉMONSTRATION OK ═══");
    Ok(())
}

/// Hexadécimal → octets. Refuse tout ce qui n'est pas une paire hexa exacte.
fn hex_vers_octets(h: &str) -> Result<Vec<u8>, String> {
    let h = h.trim();
    if h.len() % 2 != 0 {
        return Err("clé hexadécimale de longueur impaire".into());
    }
    (0..h.len() / 2)
        .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).map_err(|_| "clé hexadécimale invalide".to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_comptage_ignore_les_lignes_vides_comme_grep_c() {
        // La mesure du pilote doit être EXACTEMENT celle qu'un auditeur refait à la main avec
        // `grep -c '[^[:space:]]'` — sans quoi les deux chiffres divergeraient et la
        // démonstration s'effondrerait sur un détail de comptage.
        assert_eq!(noeuds_du_bundle("a\nb\nc"), 3);
        assert_eq!(noeuds_du_bundle("a\nb\nc\n"), 3, "la nouvelle ligne finale ne compte pas");
        assert_eq!(noeuds_du_bundle("a\n\n  \nb"), 2, "les lignes vides ou blanches non plus");
        assert_eq!(noeuds_du_bundle(""), 0);
    }

    #[test]
    fn lancre_se_lit_sur_le_format_tabule_reel() {
        // Un exemplaire réel, produit par la passerelle : version, run, tenant, racine, nœuds, ms.
        let ligne = "1\tfd32f69f-ba3c-407f-a991-38c2a86a0fad\t00000000-0000-0000-0000-000000000001\tcc96e1774b150bed13080bc61709265c0cc32f143f17279a021ec8c4ee51cc6a\t2\t1787393243989";
        let f = std::env::temp_dir().join("scella-preuve-ancre-test.log");
        std::fs::write(&f, format!("{ligne}\n")).unwrap();
        let a = derniere_ancre(&f).expect("ancre lisible");
        assert_eq!(a.run_id, "fd32f69f-ba3c-407f-a991-38c2a86a0fad");
        assert_eq!(a.racine.len(), 64, "la racine est un blake3 hexadécimal");
        assert_eq!(a.noeuds, 2);
        std::fs::remove_file(&f).ok();
    }

    /// Le journal est **append-only** : c'est la DERNIÈRE ligne qui décrit l'export qu'on vient de
    /// déclencher. Lire la première ferait conclure la démonstration sur un run étranger.
    #[test]
    fn cest_la_derniere_ancre_qui_compte() {
        let f = std::env::temp_dir().join("scella-preuve-ancre-multi.log");
        std::fs::write(
            &f,
            "1\tAAA\tt\t".to_string() + &"0".repeat(64) + "\t7\t1\n1\tBBB\tt\t" + &"1".repeat(64) + "\t9\t2\n",
        )
        .unwrap();
        let a = derniere_ancre(&f).expect("ancre lisible");
        assert_eq!(a.run_id, "BBB");
        assert_eq!(a.noeuds, 9);
        std::fs::remove_file(&f).ok();
    }

    #[test]
    fn un_journal_vide_est_une_erreur_pas_un_zero() {
        // Rendre `0` ferait passer « aucune ancre publiée » pour « un bundle à zéro nœud », et la
        // démonstration conclurait à une concordance qui n'a jamais eu lieu.
        let f = std::env::temp_dir().join("scella-preuve-ancre-vide.log");
        std::fs::write(&f, "\n  \n").unwrap();
        assert!(derniere_ancre(&f).is_err());
        std::fs::remove_file(&f).ok();
    }

    #[test]
    fn la_cle_hexa_est_refusee_si_elle_est_douteuse() {
        assert!(hex_vers_octets("00ff").is_ok());
        assert!(hex_vers_octets("0").is_err(), "longueur impaire");
        assert!(hex_vers_octets("zz").is_err(), "hors alphabet hexadécimal");
    }
}
