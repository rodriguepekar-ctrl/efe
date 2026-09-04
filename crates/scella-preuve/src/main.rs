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
#[derive(Debug)]
struct Ancre {
    run_id: String,
    racine: String,
    noeuds: u64,
}

/// L'ancre **de ce run**, cherchée par son identifiant dans le journal append-only. La ligne
/// celle de l'export qu'on vient de déclencher.
fn ancre_du_run(chemin: &Path, run_id: &str) -> Result<Ancre, String> {
    let texte = std::fs::read_to_string(chemin)
        .map_err(|e| format!("journal d'ancres « {} » illisible : {e}", chemin.display()))?;
    // **La ligne de CE run, pas la dernière.** Cette fonction prenait `next_back()`, et
    // l'hypothèse tenait tant qu'un seul service écrivait dans le journal. Le profil `jonction`
    // en met DEUX — la passerelle du Seuil et celle de la jonction publient toutes deux dans
    // `/preuve/anchors.log` — et « la dernière » devient alors celle de l'autre, au hasard de
    // l'ordonnancement. Le champ `run_id` était déjà lu ; il ne servait simplement pas à choisir.
    //
    // Le défaut aurait été SILENCIEUX et intermittent : une racine qui ne concorde pas, un
    // comptage de nœuds faux, et une démonstration qui accuse le produit d'un défaut qu'il n'a pas.
    let ligne = texte
        .lines()
        .filter(|l| !l.trim().is_empty())
        .find(|l| l.split('\t').nth(1) == Some(run_id))
        .ok_or_else(|| {
            format!(
                "aucune ancre pour le run {run_id} dans « {} » — le journal contient {} ligne(s). \
                 L'export a-t-il publié son ancre ? (EFE_SWARM_ANCHOR_LOG)",
                chemin.display(),
                texte.lines().filter(|l| !l.trim().is_empty()).count()
            )
        })?;
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

/// Le répertoire des dossiers réglementaires — **le seul interrupteur de C1 dans ce pilote**.
///
/// Absent, la démonstration est celle du Seuil seul, et rien ne change. Présent, elle montre en
/// plus que le Registre a produit un dossier **du même run** : c'est la jonction des deux produits,
/// et c'est le critère de `É2`.
///
/// Il vaut `/preuve/reg` sous le profil `jonction` de `compose.yaml`, où `profiles/jonction.env`
/// pose `EFE_SWARM_REG_EVIDENCE_DIR` sur le même chemin. Les deux valeurs doivent coïncider — la
/// passerelle ÉCRIT là où ce pilote LIT — et c'est `compose.yaml` qui les tient ensemble.
fn reg_dir() -> Option<PathBuf> {
    std::env::var("SCELLA_REG").ok().filter(|v| !v.trim().is_empty()).map(PathBuf::from)
}

/// Les fichiers d'un répertoire, **récursivement**, en chemins relatifs et triés.
///
/// Douze lignes plutôt qu'une dépendance : l'image est distroless et ce binaire n'a que
/// `serde_json` et `verify-mini`. Récursif, et c'est le point — la première version de cette
/// vérification ne lisait qu'un niveau et annonçait « 1 fichier » là où il y en a trois, parce que
/// `write_reg_subtree` les range sous un `reg/`. Un compte qui s'arrête au premier étage ne
/// mesure pas le dossier, il mesure sa porte.
fn fichiers_recursifs(racine: &Path) -> std::io::Result<Vec<String>> {
    fn descendre(base: &Path, cur: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
        for e in std::fs::read_dir(cur)? {
            let e = e?;
            let c = e.path();
            if c.is_dir() {
                descendre(base, &c, out)?;
            } else {
                out.push(c.strip_prefix(base).unwrap_or(&c).display().to_string());
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    descendre(racine, racine, &mut out)?;
    out.sort();
    Ok(out)
}

/// Le nombre d'étapes de cette démonstration : **5** pour le Seuil seul, **6** quand le Registre
/// est branché.
///
/// Calculé, jamais écrit en dur. Un « 1/5 » suivi d'un « 6/6 » ferait douter le lecteur de ce
/// qu'il vient de lire — et cette démonstration n'existe que pour être crue sur pièces.
fn total_etapes() -> u8 {
    if reg_dir().is_some() { 6 } else { 5 }
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

    let t = total_etapes();
    println!("═══ 1/{t} · la passerelle répond ═══");
    attendre_la_passerelle(&hote, 120)?;
    println!("  {hote} — vivante.");
    println!("  Aucune clé n'a été fournie : le rapport de démarrage ci-dessus (journaux du");
    println!("  service « passerelle ») annonce donc « provenance DÉGRADÉE ». C'est voulu, et");
    println!("  c'est dit — on y revient au point 5.");

    println!("\n═══ 2/{t} · un run réel, et son reçu ═══");
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

    println!("\n═══ 3/{t} · l'export, et l'ancre publiée HORS du bundle ═══");
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

    println!("\n═══ 4/{t} · la troncature, démasquée SANS CLÉ ═══");
    // Aucun garde `a.run_id != run_id` ici, et son absence est la CONCLUSION du
    // correctif, pas un oubli : `ancre_du_run` sélectionne la ligne PAR ce champ et
    // recopie ce même champ dans l'`Ancre` qu'elle rend. La condition était donc
    // devenue structurellement infranchissable — un garde qui ne peut pas tomber ne
    // garde rien, et le sien répétait encore « la DERNIÈRE ancre », le vocabulaire
    // de la règle que ce correctif venait de réfuter. Deux fautes en une : mort, et
    // faux sur ce qu'il croyait surveiller.
    //
    // Ce qui tient l'invariant à sa place, c'est le test
    // `lancre_est_choisie_par_run_id_pas_par_position` : il exige qu'un run absent
    // soit une ERREUR et jamais un repli sur un voisin — précisément ce qu'un
    // retour à `next_back()` ferait échouer.
    let a = ancre_du_run(&ancre_chemin, &run_id)?;
    // Dire QUELLE ligne a été retenue, et pas seulement ce qu'elle annonce. Sous le
    // profil `jonction`, deux passerelles publient dans le même journal : un lecteur
    // qui ne verrait que « l'ancre annonce 3 » ne saurait pas de qui elle parle.
    println!("  ancre retenue              : celle du run {}", a.run_id);
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
    // Le geste enseigné ici sélectionne l'ancre PAR LE RUN, et pas par position. Il
    // disait `END{print $5}` — la DERNIÈRE ligne —, c'est-à-dire exactement la règle
    // que ce pilote a cessé d'appliquer à lui-même : sous le profil `jonction`, deux
    // passerelles publient dans le même journal et « la dernière » est celle du
    // hasard. Le produit enseignait un geste qu'il avait lui-même réfuté.
    println!("  ↑ détection faite avec un COMPTAGE. Aucune clé, aucune cryptographie, aucune");
    println!("    confiance accordée à qui que ce soit. Refaites-la vous-même :");
    println!("      grep -c '[^[:space:]]' {}", f_intact.display());
    println!("      grep -c '[^[:space:]]' {}", f_tronque.display());
    println!(
        "      awk -F'\\t' -v r={run_id} '$2==r {{print $5}}' {}",
        ancre_chemin.display()
    );
    println!("    (par le RUN, jamais par la position : sous le profil `jonction` deux");
    println!("     passerelles publient dans ce journal, et « la dernière » n'est pas la vôtre.)");

    // ── 5/6 · C1 — LA JONCTION, quand elle est branchée ────────────────────────────────────
    //
    // Cette étape n'existe que sous le profil `jonction`. Elle est le critère de `É2` : « la
    // démonstration dérive un dossier réglementaire DU MÊME RUN ».
    //
    // Pourquoi elle est ici et non dans un script : l'image est **distroless**, elle n'a ni shell
    // ni `ls`. Le geste que `compose.yaml` documentait — `docker compose run --entrypoint ls
    // jonction -R /preuve/reg` — ne pouvait donc PAS fonctionner. Un pilote Rust le peut.
    if let Some(reg) = reg_dir() {
        println!("\n═══ 5/{t} · C1 — le dossier réglementaire, dérivé DU MÊME RUN ═══");

        // (a) Le pont est-il câblé ? On ne le suppose pas : la passerelle le DIT, dans ses
        //     métriques, et c'est la seule source qui ne puisse pas se tromper sur elle-même.
        let m = requete(&hote, "GET", "/metrics", None, None)?;
        let cable = m
            .corps
            .lines()
            .find(|l| l.starts_with("efe_swarm_reg_pont "))
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("absent");
        println!("  efe_swarm_reg_pont : {cable}   (1 = un dossier est possible)");
        if cable != "1" {
            return Err(format!(
                "le pont vers le registre n'est PAS câblé (efe_swarm_reg_pont = {cable}). \
                 SCELLA_REG est posé, donc ce pilote attendait un dossier — or la passerelle \
                 annonce qu'elle ne peut pas en produire. Vérifiez EFE_SWARM_REG_EVIDENCE_DIR \
                 dans profiles/jonction.env, et que l'image est bien celle construite avec la \
                 feature `reg-evidence`."
            ));
        }

        // (b) Le dossier existe-t-il ? Et il porte le NOM DU RUN — ce n'est pas une commodité de
        //     rangement, c'est LE lien. Un dossier trouvé sous un autre nom ne prouverait pas
        //     qu'il vient de ce run-là, et « du même run » est le mot qui compte dans É2.
        let dossier = reg.join(&run_id);
        let noms = fichiers_recursifs(&dossier).map_err(|e| {
            format!(
                "aucun dossier réglementaire en {} — {e}.\n  \
                 La passerelle annonce pourtant le pont câblé (efe_swarm_reg_pont = 1). Deux \
                 causes possibles, et elles se distinguent : le volume partagé n'est pas monté \
                 au même endroit dans les deux services, ou la dérivation a échoué en silence \
                 (elle est conçue pour ne jamais faire tomber la requête du client).",
                dossier.display()
            )
        })?;
        println!("  dossier     : {}", dossier.display());
        println!("  ↑ il porte le NOM du run — c'est le lien, pas un rangement");
        println!("  il contient : {} fichier(s)", noms.len());
        for n in &noms {
            println!("      · {n}");
        }
        if noms.is_empty() {
            return Err(format!(
                "le dossier {} existe mais il ne contient AUCUN fichier. Un répertoire créé sans \
                 contenu est pire qu'un répertoire absent : il fait croire qu'un dossier a été \
                 produit. `write_reg_subtree` propage pourtant chaque écriture — si le répertoire \
                 est là et vide, c'est que la dérivation a été interrompue entre les deux.",
                dossier.display()
            ));
        }
        // Les trois pièces que `reg_subtree` produit toujours, et ce que chacune sert. On vérifie
        // leur PRÉSENCE par leur rôle, jamais par leur nom exact : celui du mapping porte sa
        // version (`mapping.2026-06.json`), et l'épingler ici ferait échouer la démonstration au
        // prochain millésime — un test qui tombe quand le produit évolue correctement ne garde rien.
        let a_vue = noms.iter().any(|n| n.ends_with(".md") && !n.ends_with("REDERIVE.md"));
        let a_mapping = noms.iter().any(|n| n.contains("mapping") && n.ends_with(".json"));
        let a_mode_emploi = noms.iter().any(|n| n.ends_with("REDERIVE.md"));
        println!(
            "  ⟹ la vue {} · le mapping versionné {} · le mode d'emploi de re-dérivation {}",
            if a_vue { "✓" } else { "MANQUE" },
            if a_mapping { "✓" } else { "MANQUE" },
            if a_mode_emploi { "✓" } else { "MANQUE" }
        );
        if !(a_vue && a_mapping && a_mode_emploi) {
            return Err(format!(
                "le dossier est incomplet : il faut la VUE (ce que le cadre couvre), le MAPPING \
                 versionné (pour que la vue soit re-dérivable) et REDERIVE.md (le mode d'emploi). \
                 Un dossier sans son mapping n'est pas opposable : personne ne peut refaire le \
                 chemin. Trouvé : {}",
                noms.join(", ")
            ));
        }

        // (c) Et il se RATTRAPE. C'est la propriété que M5 vend — « un dossier manqué se rattrape
        //     à l'identique » — et elle ne vaut que si on la montre : `POST /v1/swarm/reg/:id`
        //     re-dérive depuis le run PERSISTÉ, jamais depuis le dossier déjà écrit. Une
        //     re-dérivation qui relirait sa propre sortie ne prouverait rien.
        let re = requete(&hote, "POST", &format!("/v1/swarm/reg/{run_id}"), Some("auditor"), Some("{}"))?;
        if re.statut == 200 {
            println!("  rattrapage  : POST /v1/swarm/reg/{run_id} → 200, re-dérivé depuis le run persisté");
        } else {
            println!(
                "  ⚠ rattrapage : HTTP {} — le dossier est là, mais la voie de récupération de M5 \
                 ne répond pas ({}).",
                re.statut,
                re.corps.chars().take(120).collect::<String>()
            );
        }

        println!();
        // 8081 est le DÉFAUT, plus une certitude : `SCELLA_PORT_JONCTION` le déplace.
        // Ce pilote ne peut pas le savoir — il ne connaît que l'adresse INTERNE au
        // réseau de compose (`jonction:8080`), qui ne veut rien dire depuis l'hôte.
        // Le dire ainsi reste vrai quel que soit le port ; l'affirmer ne l'était plus.
        println!("  Refaites-le vous-même, sans ce pilote — sur le port que publie");
        println!("  SCELLA_PORT_JONCTION (8081 par défaut) :");
        println!("    curl -s localhost:8081/metrics | grep efe_swarm_reg_pont");
        println!("    curl -s -X POST localhost:8081/v1/swarm/reg/{run_id} -H 'x-efe-roles: auditor'");
    }

    println!("\n═══ {t}/{t} · ce qui a été prouvé, et ce qui ne l'a pas été ═══");
    println!("  SANS clé, à l'instant :");
    println!("    · un reçu a été servi par une passerelle qui tourne en local ;");
    println!("    · son bundle concorde avec une ancre publiée hors bande ;");
    println!("    · une troncature a été démasquée par un comptage.");
    if reg_dir().is_some() {
        println!("    · et le Registre a dérivé un dossier réglementaire DU MÊME run.");
    }
    println!();
    // La clôture doit couvrir ce qui vient d'être fait, sinon elle cesse de clore. Et elle doit
    // dire ce que le dossier NE prouve pas : le mapping est une revendication DATÉE, pas un
    // certificat de conformité. « Outillé n'est pas conforme » est écrit dans reg-evidence
    // lui-même ; le répéter ici évite qu'un lecteur pressé conclue l'inverse.
    if reg_dir().is_some() {
        println!("    Ce dossier est une VUE re-dérivable du run, sous un cadre daté (eu_ai_act");
        println!("    ici). Il montre ce qui est couvert, ce qui ne l'est qu'en partie, et ce qui");
        println!("    sort du périmètre. Il n'atteste PAS la conformité : outillé n'est pas conforme.");
        println!();
    }

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
        let a = ancre_du_run(&f, "fd32f69f-ba3c-407f-a991-38c2a86a0fad").expect("ancre lisible");
        assert_eq!(a.run_id, "fd32f69f-ba3c-407f-a991-38c2a86a0fad");
        assert_eq!(a.racine.len(), 64, "la racine est un blake3 hexadécimal");
        assert_eq!(a.noeuds, 2);
        std::fs::remove_file(&f).ok();
    }

    /// **Ce test affirmait le contraire, et il avait tort.** Il s'appelait
    /// `cest_la_derniere_ancre_qui_compte` et sa doc disait : « c'est la DERNIÈRE ligne qui décrit
    /// l'export qu'on vient de déclencher ».
    ///
    /// C'était vrai tant qu'un seul service écrivait dans le journal. Le profil `jonction` en met
    /// DEUX — la passerelle du Seuil et celle de la jonction publient toutes deux dans
    /// `/preuve/anchors.log` — et « la dernière » devient alors celle de l'autre, au hasard de
    /// l'ordonnancement. Le défaut aurait été silencieux et intermittent.
    ///
    /// La règle juste ne dépend pas du nombre d'écrivains : **l'ancre de ce run est celle qui porte
    /// son identifiant.** Le champ était déjà lu ; il ne servait pas à choisir.
    #[test]
    fn lancre_est_choisie_par_run_id_pas_par_position() {
        let f = std::env::temp_dir().join("scella-preuve-ancre-multi.log");
        std::fs::write(
            &f,
            "1\tAAA\tt\t".to_string() + &"0".repeat(64) + "\t7\t1\n1\tBBB\tt\t" + &"1".repeat(64) + "\t9\t2\n",
        )
        .unwrap();
        // La PREMIÈRE ligne, choisie par son identifiant — c'est précisément ce que l'ancienne
        // règle rendait impossible.
        let a = ancre_du_run(&f, "AAA").expect("ancre lisible");
        assert_eq!(a.run_id, "AAA");
        assert_eq!(a.noeuds, 7, "les 7 nœuds de AAA, pas les 9 de BBB");
        // Et la dernière se trouve aussi bien, sans que sa position n'y soit pour rien.
        assert_eq!(ancre_du_run(&f, "BBB").expect("ancre lisible").noeuds, 9);
        // Un run absent du journal est une ERREUR, jamais un repli silencieux sur un voisin.
        let e = ancre_du_run(&f, "CCC").expect_err("un run absent doit échouer");
        assert!(e.contains("CCC"), "le refus nomme le run cherché : {e}");
        assert!(e.contains('2'), "et il dit combien de lignes le journal contenait : {e}");
        std::fs::remove_file(&f).ok();
    }

    /// **Le test qui aurait attrapé mon défaut.** La première version de la vérification C1 lisait
    /// un seul niveau et annonçait « 1 fichier » là où `write_reg_subtree` en range trois sous un
    /// `reg/`. Elle était donc verte ET fausse : elle aurait laissé passer un `reg/` vide.
    #[test]
    // Le nom portait « PROFONDEUR » en capitales, et cette emphase coûtait un
    // `non_snake_case` — un AVERTISSEMENT, donc une erreur sous le `-D warnings`
    // que la porte de qualité (É3) doit rendre verte. É2 l'avait introduit ; il
    // ne figurait dans aucun des six défauts recensés. L'emphase descend dans le
    // message d'assertion, où elle sert le lecteur d'un échec au lieu du compilateur.
    fn les_fichiers_se_comptent_en_profondeur_pas_au_premier_etage() {
        let base = std::env::temp_dir().join(format!("scella-preuve-rec-{}", std::process::id()));
        let sous = base.join("reg");
        std::fs::create_dir_all(&sous).unwrap();
        std::fs::write(sous.join("eu_ai_act.md"), "vue").unwrap();
        std::fs::write(sous.join("mapping.2026-06.json"), "{}").unwrap();
        std::fs::write(sous.join("REDERIVE.md"), "mode d'emploi").unwrap();

        let noms = fichiers_recursifs(&base).expect("lecture");
        assert_eq!(noms.len(), 3, "trois fichiers, pas un répertoire : {noms:?}");
        assert_eq!(noms[0], "reg/REDERIVE.md", "chemins RELATIFS et triés");
        assert!(noms.iter().any(|n| n.contains("mapping")), "le mapping versionné est vu");

        // Un répertoire vide rend une liste vide — c'est ce qui permet au refus de tomber.
        let vide = base.join("rien");
        std::fs::create_dir_all(&vide).unwrap();
        assert!(fichiers_recursifs(&vide).expect("lecture").is_empty());

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn un_journal_vide_est_une_erreur_pas_un_zero() {
        // Rendre `0` ferait passer « aucune ancre publiée » pour « un bundle à zéro nœud », et la
        // démonstration conclurait à une concordance qui n'a jamais eu lieu.
        let f = std::env::temp_dir().join("scella-preuve-ancre-vide.log");
        std::fs::write(&f, "\n  \n").unwrap();
        assert!(ancre_du_run(&f, "peu-importe").is_err());
        std::fs::remove_file(&f).ok();
    }

    #[test]
    fn la_cle_hexa_est_refusee_si_elle_est_douteuse() {
        assert!(hex_vers_octets("00ff").is_ok());
        assert!(hex_vers_octets("0").is_err(), "longueur impaire");
        assert!(hex_vers_octets("zz").is_err(), "hors alphabet hexadécimal");
    }
}
