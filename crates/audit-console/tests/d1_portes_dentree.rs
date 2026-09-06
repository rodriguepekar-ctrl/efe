// SPDX-License-Identifier: UNLICENSED
//! **D1 — les quatre portes d'entrée ne citent que des chemins qui existent.**
//!
//! ## Le grief, et pourquoi il vaut un garde
//!
//! D1 avait réparé le `README`, et rien d'autre. Le 5 septembre 2026, deux documents que le premier
//! lecteur ouvre juste après lui décrivaient encore un dépôt disparu :
//!
//! - `CONTRIBUTING.md` renvoyait à `docs/docs/adr/ADR-D93-001-…` (absent), à un `CHANGELOG.md`
//!   (absent), à `.github/CODEOWNERS` (archivé la veille), à une chaîne Go pour `cmd/vsock-relayer`
//!   (absent, et **zéro** fichier `.go` dans tout le dépôt) ;
//! - `SECURITY.md` citait **sept** documents de conformité dont **aucun** n'existait, et accordait
//!   une licence Apache-2.0 que `LICENSE` contredit mot pour mot.
//!
//! Aucun de ces manques n'était détectable autrement qu'en ouvrant les fichiers un par un. Un lien
//! mort ne casse pas la compilation, ne fait pas tomber un test, et ne se voit pas dans un diff : il
//! ne se voit que chez le lecteur, c'est-à-dire trop tard.
//!
//! ## Ce qu'il regarde, et ce qu'il refuse de regarder
//!
//! **Les liens markdown relatifs**, `[texte](chemin)`, dans les quatre documents de la racine. Un
//! suffixe `:123` est toléré et retiré avant vérification — c'est la convention de citation de ligne
//! du dépôt, pas une partie du chemin.
//!
//! Il **ne regarde pas** :
//!
//! - les URL (`http`, `https`, `mailto`) — leur validité ne se décide pas hors ligne, et un garde
//!   qui prétendrait la décider mentirait à la première coupure réseau ;
//! - les ancres (`#section`) — elles n'ont pas de fichier à trouver ;
//! - les chemins cités **entre accents graves** dans la prose. C'est délibéré et c'est la limite
//!   honnête de ce garde : `` `cmd/vsock-relayer` `` était écrit ainsi, et ce garde ne l'aurait pas
//!   attrapé. Étendre la règle aux accents graves ferait tomber le garde sur chaque identifiant de
//!   code cité — `routes/llm.rs` est un chemin, `Vec<u8>` n'en est pas un, et aucune règle
//!   syntaxique ne les sépare proprement. Un garde qui exige des exemptions à chaque phrase finit
//!   par n'être qu'une liste d'exemptions ;
//! - `docs/` — même raison que [`p3_aucune_cible_abandonnee`] : ce sont des archives **datées**, et
//!   réécrire une archive pour satisfaire un contrôle est une falsification de registre, ce qui est
//!   exactement le contraire de ce que ce dépôt vend.
//!
//! ## Le plancher
//!
//! Un garde qui n'observe rien rend `0` avec le même enthousiasme qu'un garde qui observe tout. Si
//! l'extracteur casse, ou si un fichier est renommé sans que ce garde le sache, la boucle
//! principale ne trouvera plus aucun lien et sera **verte**. [`PLANCHER`] refuse ce silence.

use std::path::{Path, PathBuf};

/// Les documents que le premier lecteur ouvre, dans l'ordre où il les ouvre.
const PORTES: &[&str] = &[
    "README.md",
    "LISEZ-MOI-C1-C3.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
];

/// Nombre minimal de liens relatifs à trouver, tous documents confondus.
///
/// Mesuré à **18** le 5 septembre 2026. Le plancher est posé sous cette valeur avec de la marge,
/// pour qu'une réécriture légitime ne le fasse pas tomber — mais assez haut pour qu'un extracteur
/// cassé, lui, tombe. Il n'y a pas de liste d'exemptions dans ce garde, et il ne faut pas en créer
/// une : un lien mort se répare en corrigeant le lien.
const PLANCHER: usize = 12;

/// Nombre minimal de sections `##` par porte d'entrée, mesuré le 6 septembre 2026 :
/// README 9 · LISEZ-MOI 13 · CONTRIBUTING 5 · SECURITY 5. Le plancher est posé sous chacune avec
/// de la marge — il n'interdit pas de réorganiser, il interdit qu'un document PERDE la moitié de
/// sa structure sans que rien ne le dise.
///
/// **Ce plancher existe parce que ce garde a laissé passer exactement cela.** Le 6 septembre 2026,
/// une édition par script a supprimé la section 5 entière de `SECURITY.md` — un repli
/// `j = len(s)` qui a effacé tout ce qui suivait le dernier point d'une liste. Les cinq tests de
/// ce fichier sont restés VERTS : ils comptent les **liens**, et la section perdue n'en portait
/// aucun. Un garde qui ne vérifie qu'une dimension laisse passer tout ce qui vit dans les autres.
const PLANCHER_SECTIONS: &[(&str, usize)] = &[
    ("README.md", 6),
    ("LISEZ-MOI-C1-C3.md", 8),
    ("CONTRIBUTING.md", 4),
    ("SECURITY.md", 4),
];

fn racine() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("racine du dépôt")
}

/// **L'extracteur, isolé et pur** — pour qu'il soit éprouvable sur une entrée synthétique.
///
/// Rend les cibles des liens markdown qui désignent un chemin du dépôt : ni URL, ni ancre, et le
/// suffixe `:123` retiré. L'ordre de sortie suit l'ordre du texte.
fn liens_relatifs(texte: &str) -> Vec<String> {
    let mut sortie = Vec::new();
    let octets = texte.as_bytes();
    let mut i = 0;
    while i + 1 < octets.len() {
        // On ne s'intéresse qu'à la séquence `](`, qui ouvre la cible d'un lien markdown.
        if octets[i] != b']' || octets[i + 1] != b'(' {
            i += 1;
            continue;
        }
        let debut = i + 2;
        let Some(longueur) = texte[debut..].find(')') else {
            break; // parenthèse jamais refermée : plus rien d'exploitable après
        };
        let cible = &texte[debut..debut + longueur];
        i = debut + longueur + 1;

        if cible.is_empty()
            || cible.starts_with('#')
            || cible.starts_with("http://")
            || cible.starts_with("https://")
            || cible.starts_with("mailto:")
        {
            continue;
        }
        // `chemin:123` — la citation de ligne du dépôt. Le `:` n'appartient pas au chemin.
        let chemin = match cible.rsplit_once(':') {
            Some((avant, apres)) if !apres.is_empty() && apres.bytes().all(|b| b.is_ascii_digit()) => avant,
            _ => cible,
        };
        sortie.push(chemin.to_string());
    }
    sortie
}

#[test]
fn les_portes_dentree_ne_citent_que_ce_qui_existe() {
    let racine = racine();
    let mut vus = 0usize;
    let mut morts: Vec<String> = Vec::new();

    for porte in PORTES {
        let chemin = racine.join(porte);
        let texte = std::fs::read_to_string(&chemin)
            .unwrap_or_else(|e| panic!("{porte} est une porte d'entrée et doit être lisible : {e}"));
        for lien in liens_relatifs(&texte) {
            vus += 1;
            if !racine.join(&lien).exists() {
                morts.push(format!("{porte} → {lien}"));
            }
        }
    }

    assert!(
        morts.is_empty(),
        "{} lien(s) mort(s) dans les portes d'entrée — un lecteur les rencontrerait avant tout le \
         reste :\n  {}",
        morts.len(),
        morts.join("\n  ")
    );
    assert!(
        vus >= PLANCHER,
        "seulement {vus} lien(s) relatif(s) trouvé(s) dans {} documents, plancher {PLANCHER}. \
         Ce n'est pas une réussite : soit l'extracteur est cassé, soit les portes d'entrée ont \
         cessé de renvoyer au dépôt. Un garde qui n'observe rien est vert pour rien.",
        PORTES.len()
    );
}

// ── Les contre-épreuves : un garde incapable de tomber ne garde rien ────────────────────────────

#[test]
fn contre_epreuve_un_chemin_absent_serait_vu() {
    let liens = liens_relatifs("voir [l'ADR](docs/docs/adr/ADR-D93-001-rustc-1.88-bump.md) pour le détail");
    assert_eq!(liens, vec!["docs/docs/adr/ADR-D93-001-rustc-1.88-bump.md"]);
    assert!(
        !racine().join(&liens[0]).exists(),
        "ce chemin est l'exemple réel du grief de D1 ; s'il venait à exister, cette contre-épreuve \
         perdrait son sens et il faudrait en choisir un autre — pas la supprimer"
    );
}

#[test]
fn contre_epreuve_url_et_ancres_sont_ecartees() {
    let liens = liens_relatifs(
        "[a](https://example.invalid/x) [b](http://x) [c](mailto:x@y.z) [d](#section) [e](LICENSE)",
    );
    assert_eq!(
        liens,
        vec!["LICENSE"],
        "seules les cibles vérifiables hors ligne doivent sortir de l'extracteur"
    );
}

#[test]
fn contre_epreuve_le_suffixe_de_ligne_est_retire() {
    assert_eq!(
        liens_relatifs("[x](crates/efe-swarm-gateway/src/main.rs:53) [y](a/b.rs:) [z](a:b)"),
        vec!["crates/efe-swarm-gateway/src/main.rs", "a/b.rs:", "a:b"],
        "`:53` est une citation de ligne ; `:` suivi d'autre chose qu'un nombre appartient au chemin"
    );
}

#[test]
fn contre_epreuve_lextracteur_trouve_vraiment_quelque_chose() {
    // Sans cette épreuve, un extracteur qui rendrait toujours `vec![]` passerait les trois
    // précédentes dès qu'on les aurait écrites à l'envers, et le plancher serait le seul filet.
    let texte = std::fs::read_to_string(racine().join("SECURITY.md")).expect("SECURITY.md");
    assert!(
        liens_relatifs(&texte).len() >= 5,
        "SECURITY.md renvoie au code en plusieurs endroits ; en trouver moins de cinq signale un \
         extracteur cassé, pas un document sobre"
    );
}

#[test]
fn aucune_porte_dentree_ne_perd_sa_structure() {
    let racine = racine();
    let mut maigres: Vec<String> = Vec::new();
    for (porte, plancher) in PLANCHER_SECTIONS {
        let texte = std::fs::read_to_string(racine.join(porte))
            .unwrap_or_else(|e| panic!("{porte} doit être lisible : {e}"));
        let n = texte.lines().filter(|l| l.starts_with("## ")).count();
        if n < *plancher {
            maigres.push(format!("{porte} : {n} section(s), plancher {plancher}"));
        }
    }
    assert!(
        maigres.is_empty(),
        "une porte d'entrée a perdu sa structure — c'est le défaut exact qu'une édition par \
         script a produit sur SECURITY.md sans que les cinq autres tests le voient :\n  {}",
        maigres.join("\n  ")
    );
}

/// Contre-épreuve : sans elle, un compteur qui rendrait toujours `usize::MAX` passerait.
#[test]
fn contre_epreuve_le_compteur_de_sections_compte_vraiment() {
    let texte = std::fs::read_to_string(racine().join("SECURITY.md")).expect("SECURITY.md");
    let n = texte.lines().filter(|l| l.starts_with("## ")).count();
    assert!((4..=40).contains(&n), "compte invraisemblable ({n}) — le compteur est cassé");
}

