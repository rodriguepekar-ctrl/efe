// SPDX-License-Identifier: UNLICENSED
//! Le **manifeste** d'une archive — ce qui accompagne les records, et rien de plus.
//!
//! ## Pourquoi ce format, et pas du JSON
//!
//! Le même raisonnement que le journal d'ancres : une archive doit pouvoir être **lue avec `cat`**
//! par un auditeur qui n'a rien installé, et **analysée en vingt lignes** par un vérifieur sans
//! dépendances. Une ligne, une clé, une tabulation, une valeur. `#` commente.
//!
//! Le format est **injectif par construction** : toute valeur contenant une tabulation ou un saut de
//! ligne est refusée **à l'écriture**. Ce n'est pas de la coquetterie — un format où deux manifestes
//! distincts peuvent produire les mêmes octets, ou l'inverse, rend le diagnostic de restauration
//! douteux au moment précis où l'on en a besoin.
//!
//! ## Chaque champ, et la question à laquelle il répond
//!
//! | champ | la question |
//! |---|---|
//! | `format` | ce lecteur sait-il lire ceci ? |
//! | `tenant` | de qui sont ces records ? |
//! | `racine` | qu'est-ce que la chaîne doit redonner ? |
//! | `noeuds` | combien y en avait-il ? — le second garde contre la troncature |
//! | `cree_a` | quand cette archive a-t-elle été prise ? |
//! | `generation` | **quelle clé** re-prouve ceci ? (cf. la note de `lib.rs`) |
//! | `empreinte_trousseau` | restaure-t-on avec le trousseau d'origine ? |
//! | `ancre` | où est l'engagement publié ailleurs, s'il existe ? |
//!
//! `noeuds` mérite un mot. Il fait doublon avec `racine` — les deux détectent une troncature — et
//! c'est **voulu**. Comparer deux chaînes de soixante-quatre caractères est un geste qu'un opérateur
//! pressé bâcle ; comparer deux nombres, non. Deux gardes valent mieux qu'un quand l'un des deux
//! repose sur l'attention humaine.

use std::collections::BTreeMap;
use std::fmt;

/// La version du format. Un lecteur refuse ce qu'il ne connaît pas plutôt que d'en deviner le sens.
pub const FORMAT: &str = "scella-conservation/1";

/// Le nom du fichier de manifeste dans une archive.
pub const NOM_MANIFESTE: &str = "MANIFESTE";

/// Le nom du fichier de records dans une archive.
pub const NOM_RECORDS: &str = "records.jsonl";

/// La valeur qui dit « ce champ facultatif est délibérément vide ».
///
/// Écrire `ABSENT` plutôt que d'omettre la ligne : une ligne manquante et une ligne vide se
/// confondent avec une ligne **perdue**. Le dire explicitement distingue « il n'y en avait pas » de
/// « il y en avait une et elle a disparu » — la même discipline que la pierre tombale `BROYE` du
/// trousseau, pour la même raison.
pub const ABSENT: &str = "ABSENT";

/// Ce qui accompagne les records dans une archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifeste {
    /// Le tenant dont ces records sont la propriété.
    pub tenant: String,
    /// Le manifest-root attendu (64 hex minuscules).
    pub racine: String,
    /// Le nombre de records au moment de l'archivage.
    pub noeuds: usize,
    /// L'instant de la prise (secondes d'époque). Fourni par l'appelant — ce crate n'a pas d'horloge.
    pub cree_a: u64,
    /// La génération de clé tenant qui a scellé ces records.
    pub generation: u32,
    /// L'empreinte du trousseau de cas en vigueur lors de la prise, si un trousseau existait.
    pub empreinte_trousseau: Option<String>,
    /// La référence de l'ancre publiée ailleurs, si elle existe.
    pub ancre: Option<String>,
}

/// Ce qui peut clocher dans un manifeste — et jamais un message vague.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifesteError {
    /// Une valeur contenait une tabulation ou un saut de ligne : le format cesserait d'être injectif.
    ValeurNonEcrivable { champ: &'static str, cause: &'static str },
    /// Une version de format que ce lecteur ne connaît pas.
    FormatInconnu { trouve: String },
    /// Un champ obligatoire manque.
    ChampManquant { champ: &'static str },
    /// Un champ apparaît deux fois — on refuse plutôt que de choisir en silence.
    ChampEnDouble { champ: String },
    /// Un champ numérique n'est pas un nombre.
    NombreInvalide { champ: &'static str, trouve: String },
    /// La racine n'est pas 64 caractères hexadécimaux minuscules.
    RacineInvalide { trouve: String },
    /// Une ligne ne porte pas de tabulation.
    LigneSansSeparateur { numero: usize },
}

impl fmt::Display for ManifesteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ValeurNonEcrivable { champ, cause } => write!(
                f,
                "le champ « {champ} » contient {cause} — le manifeste cesserait d'être relisible sans ambiguïté"
            ),
            Self::FormatInconnu { trouve } => write!(
                f,
                "format « {trouve} » inconnu de ce lecteur (attendu « {FORMAT} ») — refuser vaut mieux que deviner"
            ),
            Self::ChampManquant { champ } => write!(f, "le champ obligatoire « {champ} » manque"),
            Self::ChampEnDouble { champ } => {
                write!(f, "le champ « {champ} » apparaît deux fois — lequel serait le bon ?")
            }
            Self::NombreInvalide { champ, trouve } => {
                write!(f, "le champ « {champ} » vaut « {trouve} », qui n'est pas un nombre")
            }
            Self::RacineInvalide { trouve } => write!(
                f,
                "la racine « {trouve} » n'est pas 64 caractères hexadécimaux minuscules"
            ),
            Self::LigneSansSeparateur { numero } => {
                write!(f, "la ligne {numero} ne contient pas de tabulation")
            }
        }
    }
}

impl std::error::Error for ManifesteError {}

/// Une valeur est-elle écrivable dans ce format ?
fn ecrivable(champ: &'static str, v: &str) -> Result<(), ManifesteError> {
    if v.contains('\t') {
        return Err(ManifesteError::ValeurNonEcrivable { champ, cause: "une tabulation" });
    }
    if v.contains('\n') || v.contains('\r') {
        return Err(ManifesteError::ValeurNonEcrivable { champ, cause: "un saut de ligne" });
    }
    Ok(())
}

/// Une racine bien formée : 64 caractères, hexadécimaux, minuscules.
///
/// La minuscule est imposée et non normalisée : accepter les deux casses ferait de deux archives
/// identiques deux archives aux octets différents, ce qui rendrait toute comparaison bit-à-bit
/// inutile — précisément l'opération sur laquelle repose la restauration.
fn racine_valide(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Manifeste {
    /// Rend le manifeste sous sa forme de fichier. Échoue plutôt que d'écrire un fichier ambigu.
    pub fn rendre(&self) -> Result<String, ManifesteError> {
        ecrivable("tenant", &self.tenant)?;
        ecrivable("racine", &self.racine)?;
        if !racine_valide(&self.racine) {
            return Err(ManifesteError::RacineInvalide { trouve: self.racine.clone() });
        }
        let trousseau = self.empreinte_trousseau.clone().unwrap_or_else(|| ABSENT.to_string());
        let ancre = self.ancre.clone().unwrap_or_else(|| ABSENT.to_string());
        ecrivable("empreinte_trousseau", &trousseau)?;
        ecrivable("ancre", &ancre)?;

        let mut s = String::new();
        s.push_str("# archive scella-conservation — une clé, une tabulation, une valeur\n");
        s.push_str(&format!("format\t{FORMAT}\n"));
        s.push_str(&format!("tenant\t{}\n", self.tenant));
        s.push_str(&format!("racine\t{}\n", self.racine));
        s.push_str(&format!("noeuds\t{}\n", self.noeuds));
        s.push_str(&format!("cree_a\t{}\n", self.cree_a));
        s.push_str(&format!("generation\t{}\n", self.generation));
        s.push_str(&format!("empreinte_trousseau\t{trousseau}\n"));
        s.push_str(&format!("ancre\t{ancre}\n"));
        Ok(s)
    }

    /// Relit un manifeste. Refuse un champ en double, un champ manquant, une racine mal formée.
    pub fn lire(texte: &str) -> Result<Manifeste, ManifesteError> {
        let mut champs: BTreeMap<String, String> = BTreeMap::new();
        for (i, ligne) in texte.lines().enumerate() {
            let l = ligne.trim_end_matches('\r');
            if l.trim().is_empty() || l.trim_start().starts_with('#') {
                continue;
            }
            let (cle, val) = match l.split_once('\t') {
                Some(p) => p,
                None => return Err(ManifesteError::LigneSansSeparateur { numero: i + 1 }),
            };
            let cle = cle.trim().to_string();
            if champs.contains_key(&cle) {
                return Err(ManifesteError::ChampEnDouble { champ: cle });
            }
            champs.insert(cle, val.trim().to_string());
        }

        let obligatoire = |c: &'static str| -> Result<String, ManifesteError> {
            champs.get(c).cloned().ok_or(ManifesteError::ChampManquant { champ: c })
        };

        let format = obligatoire("format")?;
        if format != FORMAT {
            return Err(ManifesteError::FormatInconnu { trouve: format });
        }
        let racine = obligatoire("racine")?;
        if !racine_valide(&racine) {
            return Err(ManifesteError::RacineInvalide { trouve: racine });
        }
        let nombre = |c: &'static str, v: &str| -> Result<u64, ManifesteError> {
            v.parse::<u64>().map_err(|_| ManifesteError::NombreInvalide { champ: c, trouve: v.to_string() })
        };
        let noeuds = nombre("noeuds", &obligatoire("noeuds")?)? as usize;
        let cree_a = nombre("cree_a", &obligatoire("cree_a")?)?;
        let generation = nombre("generation", &obligatoire("generation")?)? as u32;

        let facultatif = |c: &str| -> Option<String> {
            match champs.get(c) {
                None => None,
                Some(v) if v == ABSENT => None,
                Some(v) => Some(v.clone()),
            }
        };

        Ok(Manifeste {
            tenant: obligatoire("tenant")?,
            racine,
            noeuds,
            cree_a,
            generation,
            empreinte_trousseau: facultatif("empreinte_trousseau"),
            ancre: facultatif("ancre"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temoin() -> Manifeste {
        Manifeste {
            tenant: "acme-corp".into(),
            racine: "a".repeat(64),
            noeuds: 9,
            cree_a: 1_756_166_400,
            generation: 3,
            empreinte_trousseau: Some("b".repeat(64)),
            ancre: Some("anchors://2026-08-26#run-1".into()),
        }
    }

    #[test]
    fn aller_retour_a_l_identique() {
        let m = temoin();
        let relu = Manifeste::lire(&m.rendre().unwrap()).unwrap();
        assert_eq!(m, relu, "un manifeste rendu puis relu doit être le même");
    }

    #[test]
    fn les_facultatifs_absents_reviennent_none_et_pas_vide() {
        let mut m = temoin();
        m.empreinte_trousseau = None;
        m.ancre = None;
        let texte = m.rendre().unwrap();
        assert!(texte.contains("ancre\tABSENT"), "l'absence est ÉCRITE, pas omise : {texte}");
        assert_eq!(Manifeste::lire(&texte).unwrap(), m);
    }

    #[test]
    fn une_tabulation_dans_une_valeur_est_refusee_a_l_ecriture() {
        let mut m = temoin();
        m.tenant = "acme\tcorp".into();
        assert!(matches!(
            m.rendre(),
            Err(ManifesteError::ValeurNonEcrivable { champ: "tenant", .. })
        ));
    }

    #[test]
    fn un_saut_de_ligne_aussi() {
        let mut m = temoin();
        m.ancre = Some("anchors://a\nfaux\tchamp\tinjecte".into());
        assert!(matches!(m.rendre(), Err(ManifesteError::ValeurNonEcrivable { .. })));
    }

    #[test]
    fn un_champ_en_double_est_refuse_au_lieu_d_etre_choisi() {
        let texte = format!("format\t{FORMAT}\ntenant\ta\ntenant\tb\nracine\t{}\nnoeuds\t1\ncree_a\t1\ngeneration\t0\n", "a".repeat(64));
        assert!(matches!(Manifeste::lire(&texte), Err(ManifesteError::ChampEnDouble { .. })));
    }

    #[test]
    fn un_format_inconnu_est_refuse_et_pas_devine() {
        let texte = format!("format\tscella-conservation/999\nracine\t{}\n", "a".repeat(64));
        assert!(matches!(Manifeste::lire(&texte), Err(ManifesteError::FormatInconnu { .. })));
    }

    #[test]
    fn une_racine_en_majuscules_est_refusee() {
        let mut m = temoin();
        m.racine = "A".repeat(64);
        assert!(matches!(m.rendre(), Err(ManifesteError::RacineInvalide { .. })));
    }

    #[test]
    fn les_commentaires_et_les_lignes_vides_sont_ignores() {
        let m = temoin();
        let texte = format!("# entête\n\n{}\n# fin\n", m.rendre().unwrap());
        assert_eq!(Manifeste::lire(&texte).unwrap(), m);
    }
}
