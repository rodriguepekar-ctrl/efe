// SPDX-License-Identifier: UNLICENSED
//! # Le vérificateur de préservation — **indépendant du compresseur**.
//!
//! ## Le cercle qu'il existe pour casser
//!
//! `CompressorOutput::validate_preservation` compare `must_preserve` à **`self.preserved`** — la
//! liste que le compresseur *déclare* avoir préservée. Elle ne regarde **jamais** `self.summary`.
//! Un compresseur qui rend un résumé vide en déclarant `preserved: ["ACME"]` **passe la
//! validation**.
//!
//! C'est exactement le cercle de `bound_root` dans M9 : on compare une affirmation à une exigence,
//! jamais l'artefact à l'une ou l'autre. M9 l'a cassé en lisant le jeton ; MZ le casse en lisant le
//! **résumé**.
//!
//! Ce module ne prend donc qu'un texte et une entité. Il ne voit jamais `preserved`, et c'est
//! délibéré : un vérificateur qui aurait accès à la déclaration finirait par s'en servir.
//!
//! ## Pourquoi la règle est STRICTE, et déclarée
//!
//! Correspondance **littérale**, insensible à la casse, sur des **frontières de mots**. Aucune
//! lemmatisation, aucune correspondance approchée, aucun synonyme.
//!
//! Ce n'est pas de la sévérité pour la forme. Toute tolérance transformerait la mesure en
//! **jugement**, et un jugement qu'on règle soi-même donne toujours le résultat qu'on espère —
//! d'autant plus qu'on l'ajusterait après avoir vu les chiffres. Les formes voisines (« ACME
//! Metallurgie SA » / « ACME » / « la société ») sont traitées par le corpus **adverse**, pas par un
//! assouplissement du contrôle.
//!
//! Et pour ce que le Cadrage sert — un dossier qui doit citer un numéro, une date, un montant —
//! littéral est la bonne définition : « SUP-2024-0417 » reformulé n'est plus une référence.

/// L'entité apparaît-elle **littéralement** dans le texte ?
///
/// Insensible à la casse ; les deux extrémités doivent tomber sur une frontière de mot, faute de
/// quoi « ISO 14001 » serait trouvé dans « ISO 140012 » — une référence différente.
#[must_use]
pub fn presente(texte: &str, entite: &str) -> bool {
    let entite = entite.trim();
    if entite.is_empty() {
        return false;
    }
    let hay: Vec<char> = texte.to_lowercase().chars().collect();
    let aiguille: Vec<char> = entite.to_lowercase().chars().collect();
    if aiguille.len() > hay.len() {
        return false;
    }
    for d in 0..=(hay.len() - aiguille.len()) {
        if hay[d..d + aiguille.len()] != aiguille[..] {
            continue;
        }
        let avant_ok = d == 0 || !frontiere_interdite(hay[d - 1]);
        let f = d + aiguille.len();
        let apres_ok = f == hay.len() || !frontiere_interdite(hay[f]);
        if avant_ok && apres_ok {
            return true;
        }
    }
    false
}

/// Un caractère qui, collé à l'entité, en fait **autre chose**.
///
/// Alphanumérique uniquement : le tiret et le point appartiennent aux références elles-mêmes
/// (`SUP-2024-0417`, `2024-03-11`), et les interdire ferait échouer une entité correctement
/// préservée mais suivie d'une ponctuation.
fn frontiere_interdite(c: char) -> bool {
    c.is_alphanumeric()
}

/// Le compte d'entités retrouvées, et lesquelles manquent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taux {
    pub retrouvees: usize,
    pub exigees: usize,
    /// Ce qui manque — nommé, parce qu'un taux seul ne dit pas *quoi* a été perdu, et que la
    /// nature de ce qui tombe (une date ? un montant ? le nom du fournisseur ?) est la moitié du
    /// diagnostic.
    pub manquantes: Vec<String>,
}

impl Taux {
    /// La fraction préservée. Un ensemble d'exigences **vide** rend `1.0` — et il faut le dire :
    /// c'est exactement l'état du pipeline servi (`must_preserve: vec![]`), où la garantie est
    /// vraie parce qu'on ne demande rien.
    #[must_use]
    pub fn fraction(&self) -> f64 {
        if self.exigees == 0 {
            return 1.0;
        }
        self.retrouvees as f64 / self.exigees as f64
    }

    #[must_use]
    pub fn vacuite(&self) -> bool {
        self.exigees == 0
    }
}

/// Le taux **vérifié** : ce qui se trouve réellement dans le résumé.
#[must_use]
pub fn taux_verifie(resume: &str, exigees: &[&str]) -> Taux {
    let manquantes: Vec<String> =
        exigees.iter().filter(|e| !presente(resume, e)).map(|e| (*e).to_string()).collect();
    Taux { retrouvees: exigees.len() - manquantes.len(), exigees: exigees.len(), manquantes }
}

/// Le taux **déclaré** : ce que le compresseur affirme avoir préservé.
///
/// Reproduit la sémantique exacte de `CompressorOutput::validate_preservation` — comparaison
/// d'égalité stricte sur la liste déclarée. On la reproduit plutôt que de l'appeler pour pouvoir la
/// mesurer *par entité* là où l'originale rend un booléen global.
#[must_use]
pub fn taux_declare(declarees: &[String], exigees: &[&str]) -> Taux {
    let manquantes: Vec<String> = exigees
        .iter()
        .filter(|e| !declarees.iter().any(|d| d == *e))
        .map(|e| (*e).to_string())
        .collect();
    Taux { retrouvees: exigees.len() - manquantes.len(), exigees: exigees.len(), manquantes }
}

/// Une sortie **dégénérée** : vide, ou le gabarit recraché.
///
/// Miroir de `is_degenerate_summary` dans le client. On le reproduit ici pour que MZ puisse
/// mesurer le taux **sans** dépendre d'une fonction privée — et le test
/// `le_detecteur_reste_aligne_sur_celui_du_client` interdit aux deux de diverger.
#[must_use]
pub fn degenere(resume: &str) -> bool {
    let s = resume.trim();
    s.is_empty() || s.contains("<the compressed text>") || s.contains("<source text>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_entite_presente_est_trouvee_quelle_que_soit_la_casse() {
        assert!(presente("Le dossier SUP-2024-0417 est ouvert.", "SUP-2024-0417"));
        assert!(presente("le dossier sup-2024-0417 est ouvert.", "SUP-2024-0417"));
        assert!(presente("ACME METALLURGIE SA a repondu", "ACME Metallurgie SA"));
    }

    /// ⭐ La frontière de mot n'est pas une coquetterie : sans elle, « ISO 14001 » serait trouvé
    /// dans « ISO 140012 », qui est une **référence différente**.
    #[test]
    fn une_entite_noyee_dans_un_mot_plus_long_nest_pas_trouvee() {
        assert!(!presente("certification ISO 140012 obtenue", "ISO 14001"));
        assert!(!presente("dossier XSUP-2024-0417", "SUP-2024-0417"));
        assert!(!presente("montant 11 240 000 EUR", "1 240 000 EUR"));
    }

    /// Mais la ponctuation qui SUIT ne doit pas faire échouer une entité correctement préservée.
    #[test]
    fn la_ponctuation_adjacente_ne_masque_pas_une_entite() {
        for t in [
            "ouvert le 2024-03-11.",
            "ouvert le 2024-03-11,",
            "(2024-03-11)",
            "« 2024-03-11 »",
            "2024-03-11",
        ] {
            assert!(presente(t, "2024-03-11"), "doit trouver dans « {t} »");
        }
    }

    #[test]
    fn les_entites_a_espaces_se_trouvent_entieres() {
        assert!(presente("plafonne a 1 500 000 EUR par an", "1 500 000 EUR"));
        assert!(!presente("plafonne a 1 500 000 USD par an", "1 500 000 EUR"));
    }

    #[test]
    fn le_vide_et_les_cas_limites_ne_paniquent_pas() {
        assert!(!presente("", "ACME"));
        assert!(!presente("ACME", ""));
        assert!(!presente("", ""));
        assert!(!presente("court", "une entite bien plus longue que le texte"));
        assert!(presente("ACME", "ACME"), "le texte peut être exactement l'entité");
    }

    #[test]
    fn le_taux_verifie_nomme_ce_qui_manque() {
        let t = taux_verifie("Le dossier SUP-2024-0417 du 2024-03-11.", &["SUP-2024-0417", "ACME", "46 000 EUR"]);
        assert_eq!(t.retrouvees, 1);
        assert_eq!(t.exigees, 3);
        assert_eq!(t.manquantes, vec!["ACME".to_string(), "46 000 EUR".to_string()]);
        assert!((t.fraction() - 1.0 / 3.0).abs() < 1e-9);
    }

    /// ⭐ **Le cercle, démontré.** Le compresseur déclare tout préserver ; le résumé ne contient
    /// rien. La validation d'origine passerait ; MZ voit l'écart.
    #[test]
    fn un_resume_vide_qui_declare_tout_preserver_est_demasque() {
        let exigees = ["SUP-2024-0417", "ACME Metallurgie SA"];
        let declarees: Vec<String> = exigees.iter().map(|s| (*s).to_string()).collect();

        let declare = taux_declare(&declarees, &exigees);
        let verifie = taux_verifie("Resume sans aucune reference.", &exigees);

        assert_eq!(declare.fraction(), 1.0, "le compresseur affirme tout avoir gardé");
        assert_eq!(verifie.fraction(), 0.0, "et le résumé n'en contient rien");
        assert!(
            declare.fraction() > verifie.fraction(),
            "c'est CET écart que validate_preservation ne peut pas voir"
        );
    }

    /// Un ensemble d'exigences vide rend 1.0 — et c'est l'état du pipeline servi. La garantie y est
    /// vraie parce qu'on ne demande rien.
    #[test]
    fn une_exigence_vide_est_vacuement_satisfaite_et_le_dit() {
        let t = taux_verifie("n'importe quoi", &[]);
        assert_eq!(t.fraction(), 1.0);
        assert!(t.vacuite(), "il faut pouvoir distinguer « parfait » de « rien demandé »");
    }

    #[test]
    fn la_degenerescence_reconnait_le_gabarit_et_le_vide() {
        assert!(degenere(""));
        assert!(degenere("   \n  "));
        assert!(degenere("<the compressed text>"));
        assert!(degenere("Voici : <source text> fin"));
        assert!(!degenere("Un resume normal."));
    }

    /// Le détecteur de MZ doit rester **aligné** sur celui du client. S'ils divergeaient, MZ
    /// mesurerait une dégénérescence que le pipeline ne voit pas, ou l'inverse.
    #[test]
    fn le_detecteur_reste_aligne_sur_celui_du_client() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../efe-nano-app-coordinator/src/clients/compressor.rs"
        ))
        .expect("le client est lisible");
        let i = src.find("fn is_degenerate_summary").expect("le détecteur du client existe");
        let corps = &src[i..i + 300];
        for marqueur in ["<the compressed text>", "<source text>", "is_empty()"] {
            assert!(
                corps.contains(marqueur),
                "le client ne teste plus « {marqueur} » : le détecteur de MZ a divergé, et l'un des \
                 deux mesure autre chose que l'autre"
            );
        }
    }
}
