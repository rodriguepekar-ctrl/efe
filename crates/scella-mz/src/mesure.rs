// SPDX-License-Identifier: UNLICENSED
//! # Le balayage, les paliers, et **le verdict**.
//!
//! ## Les seuils viennent du dépôt, pas de moi
//!
//! Un seuil inventé pour l'occasion serait un seuil qu'on peut défendre après coup. Celui qui
//! compte est écrit dans le type du compresseur lui-même :
//!
//! ```text
//! //! Compresse un texte long en préservant des entités spécifiées et en
//! //! produisant un ratio mesurable (compression typique 8:1 à 12:1).
//! ```
//!
//! La question de MZ n'est donc pas « est-ce assez ? » — question dont la réponse dépend de qui la
//! pose — mais **« tient-il sa propre promesse ? »**. Celle-là ne se déplace pas.
//!
//! ## Pourquoi ce module est PUR
//!
//! Le verdict se calcule sans modèle, donc il se **teste** sans modèle : chaque issue possible est
//! éprouvée par un test hors ligne. Un verdict qu'on ne peut éprouver qu'en lançant l'expérience
//! est un verdict que personne ne relit — et c'est celui-là qui finit par s'assouplir.

use serde::{Deserialize, Serialize};

use crate::corpus::Famille;
use crate::verifieur::Taux;

/// Le ratio que le compresseur **revendique** au bas de sa fourchette (`8:1 à 12:1`).
pub const RATIO_REVENDIQUE: f64 = 8.0;

/// Préservation vérifiée exigée. Dans un produit de preuve, une entité perdue sur cent n'est pas
/// une imprécision : la chaîne scellerait fidèlement un dossier amputé, et le reçu prouverait que
/// le fait n'a jamais existé.
pub const PRESERVATION_MIN: f64 = 0.99;

/// Dégénérescence tolérée. Au-delà, le chemin de repli serait le chemin normal — la campagne H1.1
/// avait mesuré ≈ 33 % à `temperature 0.2`, ce qui avait motivé le passage au greedy.
pub const DEGENERESCENCE_MAX: f64 = 0.01;

/// Une mesure : un document, une cible de compression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Point {
    pub document: String,
    pub famille: String,
    pub cible_mots: u32,
    pub mots_source: usize,
    pub mots_resume: usize,
    /// Le ratio **effectif** — `target_words` est une consigne à ±20 %, pas un résultat.
    pub ratio: f64,
    pub preservation_declaree: f64,
    pub preservation_verifiee: f64,
    /// Ce qui a été perdu, nommé : un taux ne dit pas *quoi* est tombé, et la nature de ce qui
    /// tombe est la moitié du diagnostic.
    pub perdues: Vec<String>,
    /// Ce que le compresseur a déclaré préserver sans que ce soit dans le résumé. **La quantité
    /// centrale** : elle mesure le mensonge, pas la perte.
    pub declarees_absentes: Vec<String>,
    pub degenere: bool,
    pub ms: u64,
    /// Deux passes identiques ont-elles produit le même résumé ? `None` si non éprouvé.
    pub stable: Option<bool>,
}

impl Point {
    /// Construit une mesure à partir des deux taux et du résumé.
    #[must_use]
    pub fn nouveau(
        document: &str,
        famille: Famille,
        cible_mots: u32,
        mots_source: usize,
        resume: &str,
        declare: &Taux,
        verifie: &Taux,
        ms: u64,
    ) -> Point {
        let mots_resume = resume.split_whitespace().count();
        // Un résumé vide donnerait un ratio infini : on le borne à 0, et la dégénérescence
        // l'expliquera. Un plafond calculé sur une division par zéro serait un plafond inventé.
        let ratio = if mots_resume == 0 { 0.0 } else { mots_source as f64 / mots_resume as f64 };
        // Déclarées mais absentes = (tout ce qui manque au résumé) moins (ce que le compresseur
        // reconnaît ne pas avoir gardé).
        let declarees_absentes: Vec<String> = verifie
            .manquantes
            .iter()
            .filter(|m| !declare.manquantes.contains(m))
            .cloned()
            .collect();
        Point {
            document: document.to_string(),
            famille: famille.code().to_string(),
            cible_mots,
            mots_source,
            mots_resume,
            ratio,
            preservation_declaree: declare.fraction(),
            preservation_verifiee: verifie.fraction(),
            perdues: verifie.manquantes.clone(),
            declarees_absentes,
            degenere: crate::verifieur::degenere(resume),
            ms,
            stable: None,
        }
    }

    /// Le compresseur a-t-il affirmé préserver ce qu'il n'a pas préservé ?
    #[must_use]
    pub fn ment(&self) -> bool {
        !self.declarees_absentes.is_empty()
    }
}

/// L'agrégat d'un palier de compression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Palier {
    pub cible_mots: u32,
    pub n: usize,
    pub ratio_median: f64,
    pub preservation_declaree: f64,
    pub preservation_verifiee: f64,
    /// `déclarée − vérifiée`. **Non nul ⟹ la garantie est décorative.**
    pub ecart: f64,
    pub degenerescence: f64,
    pub menteurs: usize,
}

/// Agrège les points par cible de compression.
#[must_use]
pub fn paliers(points: &[Point]) -> Vec<Palier> {
    let mut cibles: Vec<u32> = points.iter().map(|p| p.cible_mots).collect();
    cibles.sort_unstable();
    cibles.dedup();

    cibles
        .into_iter()
        .map(|c| {
            let lot: Vec<&Point> = points.iter().filter(|p| p.cible_mots == c).collect();
            let n = lot.len();
            let moy = |f: &dyn Fn(&Point) -> f64| -> f64 {
                if n == 0 {
                    0.0
                } else {
                    lot.iter().map(|p| f(p)).sum::<f64>() / n as f64
                }
            };
            let mut ratios: Vec<f64> = lot.iter().map(|p| p.ratio).collect();
            ratios.sort_by(|a, b| a.partial_cmp(b).expect("pas de NaN"));
            // MÉDIANE et non moyenne : une seule sortie dégénérée (ratio 0) tirerait la moyenne
            // vers le bas et ferait croire à un plafond plus dur qu'il n'est.
            let ratio_median = if ratios.is_empty() {
                0.0
            } else {
                ratios[ratios.len() / 2]
            };
            let d = moy(&|p| p.preservation_declaree);
            let v = moy(&|p| p.preservation_verifiee);
            Palier {
                cible_mots: c,
                n,
                ratio_median,
                preservation_declaree: d,
                preservation_verifiee: v,
                ecart: d - v,
                degenerescence: moy(&|p| if p.degenere { 1.0 } else { 0.0 }),
                menteurs: lot.iter().filter(|p| p.ment()).count(),
            }
        })
        .collect()
}

/// Un palier est-il **acceptable** ?
#[must_use]
pub fn acceptable(p: &Palier) -> bool {
    p.preservation_verifiee >= PRESERVATION_MIN && p.degenerescence <= DEGENERESCENCE_MAX
}

/// **Le plafond** : le plus grand ratio médian atteint par un palier acceptable.
///
/// Ce n'est pas un scalaire indépendant — c'est le point où la courbe d'arbitrage cesse d'être
/// tenable. `None` si aucun palier ne l'est.
#[must_use]
pub fn plafond(paliers: &[Palier]) -> Option<f64> {
    paliers
        .iter()
        .filter(|p| acceptable(p))
        .map(|p| p.ratio_median)
        .max_by(|a, b| a.partial_cmp(b).expect("pas de NaN"))
}

/// Le verdict — contre les seuils **déclarés avant l'expérience**.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Verdict {
    /// Le Cadrage ne peut pas être employé tel quel. Chaque raison est nommée.
    Tue { raisons: Vec<String> },
    /// Il fonctionne, mais pas à la hauteur de ce qu'il revendique.
    Declasse { plafond: f64, revendique: f64 },
    /// Le levier tient. **Le défaut est alors le CÂBLAGE** — `must_preserve` toujours vide.
    Sauve { plafond: f64 },
    /// Aucun palier n'a pu être mesuré.
    Indecidable { pourquoi: String },
}

/// Rend le verdict. **Pur** : testable sans modèle, donc relisible.
#[must_use]
pub fn verdict(paliers: &[Palier]) -> Verdict {
    if paliers.is_empty() {
        return Verdict::Indecidable { pourquoi: "aucun palier mesuré".into() };
    }

    let mut raisons = Vec::new();

    // ── (1) Le mensonge. Il tue à n'importe quel palier, et sans condition de ratio. ──
    //
    // Un compresseur qui affirme préserver ce qu'il n'a pas préservé rend `validate_preservation`
    // décorative — et le pipeline SCELLERAIT cette garantie. Aucune valeur de ratio ne rachète
    // cela : ce n'est pas une perte de qualité, c'est une fausse déclaration.
    let menteurs: usize = paliers.iter().map(|p| p.menteurs).sum();
    if menteurs > 0 {
        raisons.push(format!(
            "{menteurs} mesure(s) où le compresseur DÉCLARE préserver une entité absente du résumé \
             — `validate_preservation` compare à la liste déclarée, jamais au texte : elle ne peut \
             pas voir cela, et le pipeline scellerait la garantie"
        ));
    }

    // ── (2) et (3) : au palier du ratio REVENDIQUÉ. ──
    //
    // On juge le compresseur là où il prétend travailler. Le palier retenu est celui dont le ratio
    // médian s'approche le plus de 8:1 par le dessus — comprimer moins que promis n'est pas une
    // excuse pour être jugé plus doucement.
    let au_ratio = paliers
        .iter()
        .filter(|p| p.ratio_median >= RATIO_REVENDIQUE)
        .min_by(|a, b| a.ratio_median.partial_cmp(&b.ratio_median).expect("pas de NaN"));

    if let Some(p) = au_ratio {
        if p.preservation_verifiee < PRESERVATION_MIN {
            raisons.push(format!(
                "préservation vérifiée {:.1} % au ratio revendiqué {:.1}:1 (exigé {:.0} %) — dans \
                 un produit de preuve, une entité perdue est une falsification scellée",
                p.preservation_verifiee * 100.0,
                p.ratio_median,
                PRESERVATION_MIN * 100.0
            ));
        }
        if p.degenerescence > DEGENERESCENCE_MAX {
            raisons.push(format!(
                "dégénérescence {:.1} % au ratio revendiqué (toléré {:.0} %) — le chemin de repli \
                 serait le chemin normal",
                p.degenerescence * 100.0,
                DEGENERESCENCE_MAX * 100.0
            ));
        }
    }

    if !raisons.is_empty() {
        return Verdict::Tue { raisons };
    }

    match plafond(paliers) {
        None => Verdict::Tue {
            raisons: vec![format!(
                "aucun palier n'atteint {:.0} % de préservation vérifiée avec ≤ {:.0} % de \
                 dégénérescence — le compresseur n'a pas de zone utilisable",
                PRESERVATION_MIN * 100.0,
                DEGENERESCENCE_MAX * 100.0
            )],
        },
        Some(pf) if pf < RATIO_REVENDIQUE => {
            Verdict::Declasse { plafond: pf, revendique: RATIO_REVENDIQUE }
        }
        Some(pf) => Verdict::Sauve { plafond: pf },
    }
}

impl Verdict {
    /// Le mot, et ce qu'il implique — écrit ici pour que le rapport et le script ne le reformulent
    /// pas chacun à leur façon.
    #[must_use]
    pub fn rendu(&self) -> String {
        match self {
            Verdict::Tue { raisons } => format!(
                "TUÉ — le Cadrage ne peut pas être employé tel quel :\n  · {}",
                raisons.join("\n  · ")
            ),
            Verdict::Declasse { plafond, revendique } => format!(
                "DÉCLASSÉ — le compresseur plafonne à {plafond:.1}:1 alors qu'il revendique \
                 {revendique:.0}:1. Il fonctionne, mais pas à la hauteur de son propre contrat : \
                 c'est la DOCUMENTATION qu'il faut corriger, ou le composant."
            ),
            Verdict::Sauve { plafond } => format!(
                "SAUVÉ — plafond mesuré {plafond:.1}:1, préservation et dégénérescence tenues.\n  \
                 ⟹ Le défaut n'est PAS le levier, c'est le CÂBLAGE : les deux sites de production \
                 passent `must_preserve: vec![]`, donc la préservation n'a jamais été demandée. \
                 C'est un bogue nommé, plus une limite acceptée."
            ),
            Verdict::Indecidable { pourquoi } => format!("INDÉCIDABLE — {pourquoi}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palier(cible: u32, ratio: f64, decl: f64, verif: f64, degen: f64, menteurs: usize) -> Palier {
        Palier {
            cible_mots: cible,
            n: 6,
            ratio_median: ratio,
            preservation_declaree: decl,
            preservation_verifiee: verif,
            ecart: decl - verif,
            degenerescence: degen,
            menteurs,
        }
    }

    #[test]
    fn les_seuils_viennent_du_contrat_du_compresseur() {
        // Si cette assertion tombe, c'est que quelqu'un a bougé la cible : le ratio revendiqué est
        // celui que le TYPE du compresseur annonce, pas un chiffre choisi ici.
        assert_eq!(RATIO_REVENDIQUE, 8.0, "« compression typique 8:1 à 12:1 »");
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../efe-nano-app-coordinator/src/types/compressor.rs"
        ))
        .expect("le type du compresseur est lisible");
        assert!(
            src.contains("8:1") && src.contains("12:1"),
            "le contrat ne revendique plus 8:1 à 12:1 : le seuil de MZ doit suivre, ou être discuté"
        );
    }

    /// ⭐ **Le mensonge tue, quel que soit le ratio.** Même un palier parfait par ailleurs.
    #[test]
    fn un_seul_mensonge_tue_meme_avec_un_ratio_excellent() {
        let v = verdict(&[palier(30, 12.0, 1.0, 1.0, 0.0, 1)]);
        match v {
            Verdict::Tue { raisons } => {
                assert!(raisons[0].contains("DÉCLARE préserver une entité absente"));
                assert!(raisons[0].contains("scellerait"), "et pourquoi c'est fatal ICI");
            }
            autre => panic!("un mensonge doit tuer : {autre:?}"),
        }
    }

    #[test]
    fn une_preservation_insuffisante_au_ratio_revendique_tue() {
        let v = verdict(&[palier(30, 9.0, 0.95, 0.95, 0.0, 0)]);
        assert!(matches!(v, Verdict::Tue { .. }));
        assert!(v.rendu().contains("falsification scellée"));
    }

    #[test]
    fn une_degenerescence_excessive_au_ratio_revendique_tue() {
        let v = verdict(&[palier(30, 9.0, 1.0, 1.0, 0.05, 0)]);
        assert!(matches!(v, Verdict::Tue { .. }));
        assert!(v.rendu().contains("chemin de repli"));
    }

    /// Le compresseur marche, mais pas à 8:1 : ce n'est pas un meurtre, c'est un déclassement — et
    /// la conclusion vise la DOCUMENTATION autant que le composant.
    #[test]
    fn un_plafond_sous_le_ratio_revendique_declasse() {
        let v = verdict(&[palier(150, 3.0, 1.0, 1.0, 0.0, 0), palier(60, 6.0, 1.0, 0.90, 0.0, 0)]);
        match v {
            Verdict::Declasse { plafond, revendique } => {
                assert_eq!((plafond, revendique), (3.0, 8.0));
                assert!(v.rendu().contains("DOCUMENTATION"));
            }
            autre => panic!("attendu déclassé : {autre:?}"),
        }
    }

    /// ⭐ L'issue la plus probable, et la plus utile : **le levier tient, le câblage ne tient pas.**
    #[test]
    fn un_plafond_au_dessus_du_ratio_revendique_sauve_et_designe_le_cablage() {
        let v = verdict(&[palier(30, 10.0, 1.0, 1.0, 0.0, 0), palier(120, 4.0, 1.0, 1.0, 0.0, 0)]);
        assert_eq!(v, Verdict::Sauve { plafond: 10.0 });
        let r = v.rendu();
        assert!(r.contains("must_preserve: vec![]"), "il doit NOMMER le défaut : {r}");
        assert!(r.contains("CÂBLAGE"));
    }

    #[test]
    fn aucun_palier_acceptable_tue_en_le_disant() {
        let v = verdict(&[palier(30, 9.0, 1.0, 0.5, 0.0, 0)]);
        assert!(matches!(v, Verdict::Tue { .. }));
    }

    #[test]
    fn sans_mesure_le_verdict_est_indecidable_et_non_favorable() {
        // Ne JAMAIS rendre « sauvé » faute de données : l'absence de mesure n'est pas un succès.
        assert!(matches!(verdict(&[]), Verdict::Indecidable { .. }));
    }

    /// Le plafond emploie la MÉDIANE : une seule sortie dégénérée ne doit pas faire croire à un
    /// plafond plus dur qu'il n'est.
    #[test]
    fn le_ratio_dun_palier_est_median_pas_moyen() {
        let pts: Vec<Point> = [10.0, 10.0, 10.0, 0.0]
            .iter()
            .enumerate()
            .map(|(i, r)| Point {
                document: format!("d{i}"),
                famille: "dense".into(),
                cible_mots: 30,
                mots_source: 300,
                mots_resume: 30,
                ratio: *r,
                preservation_declaree: 1.0,
                preservation_verifiee: 1.0,
                perdues: vec![],
                declarees_absentes: vec![],
                degenere: *r == 0.0,
                ms: 1,
                stable: None,
            })
            .collect();
        let p = paliers(&pts);
        assert_eq!(p[0].ratio_median, 10.0, "la moyenne aurait donné 7,5");
        assert!((p[0].degenerescence - 0.25).abs() < 1e-9, "et la dégénérescence est comptée à part");
    }

    #[test]
    fn un_point_sait_dire_quand_le_compresseur_ment() {
        use crate::verifieur::{taux_declare, taux_verifie};
        let exigees = ["ACME", "2024-03-11"];
        let declarees: Vec<String> = exigees.iter().map(|s| (*s).to_string()).collect();
        let p = Point::nouveau(
            "d",
            Famille::Dense,
            30,
            300,
            "Un resume qui ne cite rien.",
            &taux_declare(&declarees, &exigees),
            &taux_verifie("Un resume qui ne cite rien.", &exigees),
            42,
        );
        assert!(p.ment());
        assert_eq!(p.declarees_absentes.len(), 2);
        assert_eq!(p.preservation_declaree, 1.0);
        assert_eq!(p.preservation_verifiee, 0.0);
    }
}
