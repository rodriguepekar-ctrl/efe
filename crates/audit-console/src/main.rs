// SPDX-License-Identifier: UNLICENSED
//! **Le binaire du service de lecture (S3)** — câble l'état et sert (bind **local** uniquement). Le
//! backend détient les clés tenant (souveraineté) ; il provisionne le registre S2, réhydrate
//! éventuellement des logs scellés persistés, ouvre les sessions, et lance le serveur. Il **ne capte
//! pas** de données — la capture (S1) est alimentée par le runtime ; ici on **sert** ce qui est scellé.
//!
//! ## M7 — deux postures, et une seule est un service
//!
//! * **Déclarée** (`EFE_CONSOLE_DEPLOIEMENT` = le chemin d'un JSON) : N tenants, N sessions, chacune
//!   avec son rôle et son sujet. C'est la posture d'un service. La déclaration est **validée avant
//!   d'être appliquée**, et un refus **arrête le démarrage** : ouvrir un port avec une isolation
//!   qu'on n'a pas vérifiée serait la pire des dégradations silencieuses.
//! * **Démonstration** (variable absente) : un tenant, une session, rôle d'agent de conformité —
//!   le comportement **inchangé** d'avant M7. Changer en silence ce que fait un chemin servi est
//!   précisément ce que le registre de profil interdit.
//!
//! Variables d'environnement : **toutes** déclarées dans [`audit_console::profile::REGISTRY`], et
//! lues par lui seul (`--profile-report` en rend la forme machine).

use audit_console::canal::Plancher;
use audit_console::profile::{read, Report};
use audit_console::provisioning::{appliquer, balayer, Deploiement, Resume};
use audit_console::server::{journaliser_balayage, Balayeur};
use audit_console::{serve, Console, Role};
use provenance_dag::{SealedDag, TenantKey};

fn main() -> std::io::Result<()> {
    // ── M0 : `--profile-report` — la forme MACHINE, par drapeau et jamais par route.
    if std::env::args().any(|a| a == "--profile-report") {
        let json = Report::from_env().to_json();
        println!("{}", serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string()));
        return Ok(());
    }

    let addr = read::value("EFE_CONSOLE_ADDR");
    let root = read::value("EFE_CONSOLE_ROOT");

    // ── M0 : le profil servi, AVANT d'écouter. Et surtout l'avertissement que la console
    // n'émettait pas : un secret racine laissé au défaut est PUBLIC, donc la clé tenant l'est
    // aussi, donc la re-preuve ne prouve rien. La passerelle criait déjà dans le cas symétrique.
    eprint!("{}", Report::from_env().render_human());

    let mut console = Console::new();
    let declaration = read::value("EFE_CONSOLE_DEPLOIEMENT");
    let (resume, deploiement) = if declaration.trim().is_empty() {
        (monter_demonstration(&mut console, root.as_bytes())?, None)
    } else {
        let (r, d) = monter_declare(&declaration, &mut console, root.as_bytes())?;
        (r, Some(d))
    };

    // ── M8 : la déclaration du terminateur de canal ──
    //
    // Elle achète deux choses INDISSOCIABLES, et c'est voulu : le droit de binder hors de la boucle
    // locale, ET le plancher de scellement abaissé à A2. Les séparer permettrait de servir à
    // distance sans jamais pouvoir sceller (inutile) ou de sceller sans avoir rien déclaré
    // (malhonnête).
    let terminateur = read::chemin("EFE_CONSOLE_TERMINATEUR_CANAL");
    if terminateur.is_some() {
        console.avec_plancher(Plancher::protege_declare());
    }

    // Les origines du terminateur — facultatives, et c'est délibéré : les exiger casserait tout
    // déploiement A2 existant. Les poser transforme une déclaration crue sur parole en une
    // déclaration CORROBORÉE, et fait retomber le contournement sous le plancher.
    let origines = read::origines("EFE_CONSOLE_TERMINATEUR_ORIGINE");

    annoncer(
        &addr,
        &resume,
        declaration.trim().is_empty(),
        terminateur.as_deref(),
        &origines,
        &console,
    );

    // ── M6, câblé : un balayage D'AMORÇAGE, puis le fil ──
    //
    // L'amorçage compte : sans lui, un cas expiré pendant l'arrêt du service attendrait un
    // intervalle entier avant d'être détruit — et le redémarrage est précisément le moment où
    // l'horizon a le plus de chances d'avoir été franchi.
    let balayeur = deploiement.map(|dep| {
        let maintenant = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        journaliser_balayage(&balayer(&mut console, &dep, maintenant));
        // `read::entier` et non un `parse().unwrap_or()` local : la décision appartient au
        // registre, qui est aussi ce que le rapport imprime. Une analyse faite ici serait un
        // SECOND point de décision, invisible du rapport — et c'est exactement l'écart que M0 ferme.
        let intervalle_s = read::entier("EFE_CONSOLE_BALAYAGE_S");
        Balayeur { intervalle_s, deploiement: dep }
    });

    serve(&addr, console, terminateur, origines, balayeur)
}

/// La posture de **service** : la déclaration décide, et elle est validée avant toute mutation.
fn monter_declare(
    chemin: &str,
    console: &mut Console,
    root: &[u8],
) -> std::io::Result<(Resume, Deploiement)> {
    let brut = std::fs::read_to_string(chemin).map_err(|e| {
        // Un chemin déclaré mais illisible est fatal : retomber sur la démonstration donnerait un
        // service mono-tenant à qui croit en avoir déployé cinq.
        std::io::Error::new(
            e.kind(),
            format!("déclaration de déploiement « {chemin} » illisible : {e} — refus de retomber \
                     sur la posture de démonstration, qui donnerait UN tenant à qui en attend N"),
        )
    })?;
    let dep = Deploiement::depuis_json(&brut)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    // L'heure réelle : une échéance déjà passée doit être refusée AU DÉMARRAGE, pas découverte par
    // un porteur dont le jeton « ne marche pas ».
    let maintenant = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match appliquer(&dep, console, root, maintenant) {
        Ok(r) => Ok((r, dep)),
        Err(refus) => {
            eprintln!("\n╔═ DÉMARRAGE REFUSÉ ═══════════════════════════════════════════════════");
            eprintln!("║ La déclaration « {chemin} » ne peut pas être servie :");
            for r in &refus {
                eprintln!("║  • {r}");
            }
            eprintln!("╚═ le port n'a pas été ouvert ═════════════════════════════════════════\n");
            // Code 2 : distinct d'une panne d'exécution (1) — c'est une configuration à corriger.
            std::process::exit(2);
        }
    }
}

/// La posture de **démonstration** : le comportement d'avant M7, à l'identique.
fn monter_demonstration(console: &mut Console, root: &[u8]) -> std::io::Result<Resume> {
    let tenant = read::value("EFE_CONSOLE_TENANT");
    let token = read::value("EFE_CONSOLE_TOKEN");
    let subject = read::value("EFE_CONSOLE_SUBJECT");
    let key = TenantKey::derive(root, &tenant);

    let mut rehydrates = 0;
    match read::chemin("EFE_CONSOLE_RECORDS") {
        Some(path) => {
            let jsonl = std::fs::read_to_string(&path)?;
            let records = SealedDag::records_from_jsonl(&jsonl)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            console.rehydrate(&tenant, key, records);
            rehydrates = 1;
            eprintln!("réhydraté le tenant « {tenant} » depuis {path}");
        }
        None => {
            console.provision(&tenant, key);
            eprintln!("tenant « {tenant} » provisionné (vide — alimenté par la capture S1 du runtime)");
        }
    }
    console.add_session(&token, &tenant, Role::ComplianceOfficer, &subject);

    Ok(Resume {
        tenants: vec![tenant.clone()],
        rehydrates,
        sessions_par_tenant: std::iter::once((tenant, 1)).collect(),
        // La posture de démonstration ne déclare rien : pas de conservation, donc rien à broyer et
        // rien à taire.
        conservation_par_tenant: std::collections::BTreeMap::new(),
        cas_sans_cle: std::collections::BTreeMap::new(),
        trousseaux_larges: Vec::new(),
    })
}

/// Ce qu'on imprime au démarrage — et surtout ce qu'on n'imprime pas.
fn annoncer(
    addr: &str,
    resume: &Resume,
    demonstration: bool,
    terminateur: Option<&str>,
    origines: &[std::net::IpAddr],
    console: &Console,
) {
    eprintln!("service de lecture (S3) — http://{addr}");
    eprintln!(
        "  {} tenant(s) monté(s) · {} réhydraté(s) · {} session(s)",
        resume.tenants.len(),
        resume.rehydrates,
        resume.sessions()
    );
    for t in &resume.tenants {
        let n = resume.sessions_par_tenant.get(t).copied().unwrap_or(0);
        // Un tenant à ZÉRO session est monté et illisible : le dire, plutôt que de le laisser
        // disparaître dans un total.
        let note = if n == 0 { "  ⟵ AUCUNE SESSION : ce tenant est illisible" } else { "" };
        eprintln!("    « {t} » : {n} session(s){note}");
    }

    // ── le jeton : imprimé SEULEMENT s'il est celui, public, du registre ──
    //
    // Un jeton dans un journal est un jeton divulgué — c'est le défaut que P1 a corrigé sur l'URL
    // du relais SLM, qui finissait entière (identifiants compris) dans le log de chaque démarrage
    // RÉUSSI. Mais le jeton par défaut vaut `demo-token`, il est déclaré dans le registre M0 et
    // publiquement connu : l'imprimer ne divulgue rien et garde la démonstration utilisable.
    // Le critère est celui de M0, et pas un autre : « l'opérateur l'a-t-il posé ? »
    if demonstration && !read::is_set("EFE_CONSOLE_TOKEN") {
        let token = read::value("EFE_CONSOLE_TOKEN");
        eprintln!("  jeton de DÉMONSTRATION (public, du registre) : Authorization: Bearer {token}");
    } else {
        eprintln!("  jeton(s) : posé(s) par l'opérateur — non imprimé(s).");
    }

    // ── M8 : le canal, et ce qu'il autorise ──
    match terminateur {
        None => eprintln!(
            "  canal : bind LOCAL uniquement · scellement d'actes humains exigé en boucle locale (A1)"
        ),
        Some(t) => {
            eprintln!("  canal : terminateur DÉCLARÉ « {t} » — non vérifiable par ce processus.");
            eprintln!("          bind hors boucle locale autorisé ; scellement admis jusqu'à A2.");
            eprintln!("          ce processus ne termine PAS TLS : la déclaration engage l'exploitant,");
            eprintln!("          pas le binaire.");
            if origines.is_empty() {
                // On ne se résigne plus : on nomme le geste. Un défaut qu'on peut corriger et
                // qu'on ne dit pas corrigeable est une limite qu'on entretient.
                eprintln!("          origine NON déclarée : un client qui CONTOURNE le terminateur est");
                eprintln!("          crédité « protégé » et peut sceller. Posez");
                eprintln!("          EFE_CONSOLE_TERMINATEUR_ORIGINE=<ip[,ip…]> pour que ce processus");
                eprintln!("          fasse la différence — l'adresse source vient du noyau, pas du client.");
            } else {
                let liste =
                    origines.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
                eprintln!("          origine(s) déclarée(s) : {liste} — le pair y est confronté.");
                eprintln!("          une connexion qui CONTOURNE le terminateur retombe en A3 : elle");
                eprintln!("          lit encore, mais ne scelle plus.");
            }
        }
    }
    // ── M6, câblé : ce que la conservation fait, et ce qu'elle ne peut PAS lire ──
    if resume.conservation_par_tenant.is_empty() {
        eprintln!("  conservation : AUCUNE — rien n'expire, et un contenu enrobé resterait illisible");
    } else {
        for (t, n) in &resume.conservation_par_tenant {
            eprintln!("  conservation « {t} » : {n} clé(s) de cas chargée(s) · balayage toutes les {} s",
                      read::value("EFE_CONSOLE_BALAYAGE_S"));
        }
    }
    for (t, n) in &resume.cas_sans_cle {
        eprintln!(
            "  ⚠ « {t} » : {n} cas présent(s) dans les records SANS clé — leur contenu enrobé se \n\
             \x20  lira « clé absente », jamais « broyé ». On n'affirme pas une destruction qu'on \n\
             \x20  n'a pas faite ; mais un trousseau incomplet est une perte, pas un effacement."
        );
    }
    for f in &resume.trousseaux_larges {
        eprintln!(
            "  ⚠ le trousseau « {f} » est lisible au-delà de son propriétaire : ses clés sont \n\
             \x20  des SECRETS, et un effacement dont la clé traîne n'efface rien. chmod 600."
        );
    }

    let nb = console.sessions().sans_expiration();
    if nb > 0 {
        eprintln!("  ⚠ {nb} session(s) SANS ÉCHÉANCE — un identifiant qui ne meurt jamais est");
        eprintln!("    une décision, pas un défaut. Posez « expire_le » dans la déclaration.");
    }
    eprintln!("  sondes : GET /healthz (le processus vit) · GET /readyz (le service est provisionné)");
    eprintln!("  connexion : GET /login (cookie HttpOnly + SameSite=Strict) · POST /logout");
    eprintln!("  souveraineté : les clés tenant ne quittent pas le backend.");
    eprintln!("  « montré vérifié ≠ prouvé sans confiance » — pour une preuve opposable, exportez.");
}
