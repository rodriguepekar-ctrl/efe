// SPDX-License-Identifier: UNLICENSED
//! Le trousseau des **générations**, rendu durable.
//!
//! ## Le trou que ce module comble — et que j'avais moi-même creusé
//!
//! [`crate::generations::TrousseauMaitre`] vit en mémoire. Sa méthode `poser_detruite` porte, écrite
//! noir sur blanc, la phrase *« un trousseau vit en mémoire, un redémarrage le reconstruit depuis un
//! fichier »*. Ce fichier n'existait pas. La porte de restauration était là, la maison non : au
//! premier redémarrage, une génération détruite serait revenue `Inconnue`, la destruction serait
//! restée effective — la clé n'est nulle part — mais sa **date** aurait disparu, et « détruite le 12
//! mars » serait redevenu « jamais vue ».
//!
//! C'est exactement le défaut que M6 avait laissé au niveau des clés de **cas**, et que
//! `audit-console::trousseau_fichier` a fermé. Je l'avais recréé un étage plus haut.
//!
//! ## Pourquoi ce module transforme des LIGNES et ne sérialise jamais un état
//!
//! [`provenance_dag::TenantKey`] garde ses octets en `pub(crate)`. Ce crate détient donc des clés et
//! est **incapable** de les écrire. Ce n'est pas une gêne à contourner, c'est la contrainte qui
//! dessine l'API — et c'est, comprend-on enfin, pourquoi `trousseau_fichier::broyer` prend un chemin
//! et des identifiants plutôt qu'un trousseau : on ne réécrit pas ce qu'on ne peut pas lire.
//!
//! Alors les gestes d'écriture ici sont des **transformations de lignes** : retirer change un mot,
//! détruire remplace une clé par `-`. La clé qu'on écrit est toujours celle qu'on vient de lire, ou
//! celle que l'exploitant vient de fournir en hexadécimal. Jamais une clé extraite d'un objet.
//!
//! Ce choix a un bénéfice qu'on ne voit qu'après : le fichier peut porter des colonnes que le type
//! en mémoire ne modélise pas — la date d'entrée en service d'une génération **détruite**, par
//! exemple — et un aller-retour ne les perd pas. Une réécriture depuis l'état les aurait effacées en
//! silence.
//!
//! ## Une destruction qui n'est pas durable est un MENSONGE sur une destruction
//!
//! La règle de `trousseau_fichier`, et elle vaut mot pour mot ici. Détruire en mémoire pendant que
//! la clé reste sur le disque, c'est déclarer détruit ce qui est intact, et le redémarrage suivant
//! la ressusciterait. Donc : **le fichier d'abord**. Si la réécriture échoue, le geste échoue — on
//! ne détruit pas en mémoire pour se taire ensuite.
//!
//! ## Le format
//!
//! Cinq colonnes, tabulées, arité **fixe**. `#` commente. Générable à la main :
//!
//! ```text
//! # numéro  état      clé (64 hex, ou -)  depuis      jusqu_a
//! 1         RETIREE   3f9a…c1             1735689600  1742000000
//! 2         VIVANTE   a71b…4e             1742000000  -
//! 3         DETRUITE  -                   1690000000  1750000000
//! ```
//!
//! Une destruction **remplace la clé par `-` et l'état par `DETRUITE`** au lieu de supprimer la
//! ligne : supprimer perdrait la date, qui est la seule chose qu'un régulateur demande.
//!
//! Le fichier **est un secret** — il porte les clés de toutes les générations lisibles. [`charger`]
//! le signale si ses permissions le rendent lisible par n'importe qui, mais ne **refuse** pas : un
//! montage de secret peut avoir sa propre discipline, et refuser de démarrer sur une permission
//! serait un déni de service décidé par une heuristique.

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use provenance_dag::TenantKey;

use crate::generations::{RefusRotation, TrousseauMaitre};

/// L'état écrit dans la deuxième colonne.
const VIVANTE: &str = "VIVANTE";
/// Idem, pour une génération qui ne scelle plus mais re-prouve encore.
const RETIREE: &str = "RETIREE";
/// Idem, pour une pierre tombale.
const DETRUITE: &str = "DETRUITE";
/// Ce qui occupe une colonne sans valeur. Un tiret, jamais une colonne vide : une colonne vide et
/// une colonne perdue se ressemblent trop dans un fichier tabulé.
const RIEN: &str = "-";

/// Ce qui peut clocher — et jamais un message vague.
#[derive(Debug)]
pub enum Erreur {
    /// Le fichier n'a pas pu être lu ou écrit.
    Io(std::io::Error),
    /// Une ligne n'a pas cinq colonnes.
    Arite { numero_ligne: usize, colonnes: usize },
    /// Un état inconnu de ce lecteur.
    EtatInconnu { numero_ligne: usize, trouve: String },
    /// Un champ qui devait être un nombre ne l'est pas.
    NombreInvalide { numero_ligne: usize, champ: &'static str, trouve: String },
    /// Une clé qui n'est pas 64 caractères hexadécimaux minuscules.
    CleInvalide { numero_ligne: usize },
    /// Une génération vivante ou retirée sans clé — elle serait illisible et le fichier le cache.
    CleManquante { numero_ligne: usize, generation: u32 },
    /// Deux générations vivantes : « laquelle scelle ? » n'aurait pas de réponse.
    DeuxVivantes { premiere: u32, seconde: u32 },
    /// Le fichier contredit les invariants du trousseau (numéros non croissants, doublon…).
    Incoherent { cause: RefusRotation },
    /// Le geste demandé porte sur une génération que le fichier ne contient pas.
    GenerationAbsente { numero: u32 },
    /// Le geste demandé n'a pas de sens depuis l'état écrit dans le fichier.
    EtatIncompatible { numero: u32, etat: String },
}

impl fmt::Display for Erreur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "trousseau maître illisible ou non écrit : {e}"),
            Self::Arite { numero_ligne, colonnes } => write!(
                f,
                "ligne {numero_ligne} : {colonnes} colonne(s) au lieu de 5 — l'arité est fixe pour \
                 qu'une colonne perdue se voie"
            ),
            Self::EtatInconnu { numero_ligne, trouve } => write!(
                f,
                "ligne {numero_ligne} : état « {trouve} » inconnu (attendu {VIVANTE}, {RETIREE} ou {DETRUITE})"
            ),
            Self::NombreInvalide { numero_ligne, champ, trouve } => {
                write!(f, "ligne {numero_ligne} : « {champ} » vaut « {trouve} », qui n'est pas un nombre")
            }
            Self::CleInvalide { numero_ligne } => write!(
                f,
                "ligne {numero_ligne} : la clé n'est pas 64 caractères hexadécimaux minuscules"
            ),
            Self::CleManquante { numero_ligne, generation } => write!(
                f,
                "ligne {numero_ligne} : la génération {generation} est déclarée lisible mais n'a pas de \
                 clé — elle rendrait tout illisible sans jamais le dire"
            ),
            Self::DeuxVivantes { premiere, seconde } => write!(
                f,
                "générations {premiere} ET {seconde} vivantes — laquelle scellerait ? On refuse plutôt \
                 que de choisir en silence."
            ),
            Self::Incoherent { cause } => write!(f, "fichier incohérent : {cause}"),
            Self::GenerationAbsente { numero } => {
                write!(f, "la génération {numero} n'est pas dans ce fichier")
            }
            Self::EtatIncompatible { numero, etat } => {
                write!(f, "la génération {numero} est {etat} — ce geste n'a pas de sens depuis cet état")
            }
        }
    }
}

impl std::error::Error for Erreur {}

impl From<std::io::Error> for Erreur {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Une ligne analysée. Conserve son texte d'origine : les réécritures transforment, elles ne
/// re-sérialisent pas (cf. la note du module).
#[derive(Debug, Clone)]
struct Ligne {
    numero: u32,
    etat: String,
    cle_hex: String,
    depuis: u64,
    jusqu_a: Option<u64>,
}

impl Ligne {
    fn rendre(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}",
            self.numero,
            self.etat,
            self.cle_hex,
            self.depuis,
            self.jusqu_a.map_or_else(|| RIEN.to_string(), |v| v.to_string())
        )
    }
}

/// Une clé bien formée : 64 caractères, hexadécimaux, minuscules.
fn hex_valide(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Convertit 64 hex en 32 octets. L'appelant a déjà validé la forme.
fn octets(hex: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    out
}

/// Analyse le fichier en lignes, sans rien interpréter des invariants du trousseau.
fn analyser(texte: &str) -> Result<Vec<Ligne>, Erreur> {
    let mut lignes = Vec::new();
    for (i, brute) in texte.lines().enumerate() {
        let n = i + 1;
        let l = brute.trim_end_matches('\r');
        if l.trim().is_empty() || l.trim_start().starts_with('#') {
            continue;
        }
        let champs: Vec<&str> = l.split('\t').map(str::trim).filter(|c| !c.is_empty()).collect();
        if champs.len() != 5 {
            return Err(Erreur::Arite { numero_ligne: n, colonnes: champs.len() });
        }
        let numero = champs[0]
            .parse::<u32>()
            .map_err(|_| Erreur::NombreInvalide { numero_ligne: n, champ: "numéro", trouve: champs[0].into() })?;
        let etat = champs[1].to_string();
        if etat != VIVANTE && etat != RETIREE && etat != DETRUITE {
            return Err(Erreur::EtatInconnu { numero_ligne: n, trouve: etat });
        }
        let cle_hex = champs[2].to_string();
        if etat == DETRUITE {
            // Une pierre tombale NE DOIT PAS porter de clé : en porter une signifierait que la
            // destruction n'a pas eu lieu, ce que le fichier prétend pourtant.
            if cle_hex != RIEN {
                return Err(Erreur::CleInvalide { numero_ligne: n });
            }
        } else if !hex_valide(&cle_hex) {
            if cle_hex == RIEN {
                return Err(Erreur::CleManquante { numero_ligne: n, generation: numero });
            }
            return Err(Erreur::CleInvalide { numero_ligne: n });
        }
        let depuis = champs[3]
            .parse::<u64>()
            .map_err(|_| Erreur::NombreInvalide { numero_ligne: n, champ: "depuis", trouve: champs[3].into() })?;
        let jusqu_a = if champs[4] == RIEN {
            None
        } else {
            Some(champs[4].parse::<u64>().map_err(|_| Erreur::NombreInvalide {
                numero_ligne: n,
                champ: "jusqu_a",
                trouve: champs[4].into(),
            })?)
        };
        lignes.push(Ligne { numero, etat, cle_hex, depuis, jusqu_a });
    }
    lignes.sort_by_key(|l| l.numero);
    Ok(lignes)
}

/// Les permissions de ce fichier le rendent-elles lisible par d'autres que son propriétaire ?
#[cfg(unix)]
fn trop_ouvert(chemin: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(chemin).map(|m| m.permissions().mode() & 0o077 != 0).unwrap_or(false)
}

#[cfg(not(unix))]
fn trop_ouvert(_chemin: &Path) -> bool {
    false
}

/// Charge le trousseau depuis son fichier.
///
/// Rend aussi un **avertissement** de permissions : le fichier porte des clés. On le dit, on ne
/// refuse pas — un montage de secret peut avoir sa propre discipline, et transformer une permission
/// en panne de démarrage serait un déni de service décidé par une heuristique.
pub fn charger(chemin: &Path) -> Result<(TrousseauMaitre, bool), Erreur> {
    let texte = std::fs::read_to_string(chemin)?;
    let lignes = analyser(&texte)?;

    // Deux vivantes se détectent AVANT de construire : le message doit nommer les deux numéros, ce
    // que le refus du trousseau (qui ne voit qu'un conflit) ne saurait pas faire.
    let vivantes: Vec<u32> = lignes.iter().filter(|l| l.etat == VIVANTE).map(|l| l.numero).collect();
    if vivantes.len() > 1 {
        return Err(Erreur::DeuxVivantes { premiere: vivantes[0], seconde: vivantes[1] });
    }

    // Reconstruction dans l'ordre croissant. On installe d'abord les états sans clé et les retirées,
    // la vivante en dernier : `poser` refuse une seconde vivante, et la garde de stricte croissance
    // impose de toute façon l'ordre.
    let mut t = TrousseauMaitre::new();
    for l in &lignes {
        let r = match l.etat.as_str() {
            DETRUITE => t.poser_detruite(l.numero, l.jusqu_a.unwrap_or(l.depuis)),
            RETIREE => t
                .poser(l.numero, TenantKey::from_bytes(octets(&l.cle_hex)), l.depuis)
                .and_then(|()| t.retirer(l.numero, l.jusqu_a.unwrap_or(l.depuis))),
            _ => t.poser(l.numero, TenantKey::from_bytes(octets(&l.cle_hex)), l.depuis),
        };
        if let Err(cause) = r {
            return Err(Erreur::Incoherent { cause });
        }
    }
    Ok((t, trop_ouvert(chemin)))
}

/// Écrit le fichier de façon atomique et durable : temporaire voisin, `fsync`, puis `rename`.
///
/// Une écriture en place interrompue par une panne perdrait des clés que personne n'a demandé de
/// détruire : un effacement de masse par accident, dans le module dont tout le propos est la
/// conservation.
fn ecrire_atomique(chemin: &Path, lignes: &[Ligne]) -> Result<(), Erreur> {
    let mut contenu = String::from("# numéro\tétat\tclé (64 hex, ou -)\tdepuis\tjusqu_a\n");
    for l in lignes {
        contenu.push_str(&l.rendre());
        contenu.push('\n');
    }
    if let Some(parent) = chemin.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut tmp_os = chemin.as_os_str().to_owned();
    tmp_os.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp_os);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contenu.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, chemin)?;
    Ok(())
}

/// Lit les lignes d'un fichier qui n'existe peut-être pas encore (un trousseau naît vide).
fn lignes_de(chemin: &Path) -> Result<Vec<Ligne>, Erreur> {
    match std::fs::read_to_string(chemin) {
        Ok(t) => analyser(&t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(Erreur::Io(e)),
    }
}

/// Vérifie qu'un ensemble de lignes satisfait les invariants du trousseau, en le construisant.
///
/// On ne duplique pas les règles : on les fait appliquer par le type qui les porte. Une garde
/// réécrite ici finirait par diverger de celle de `generations.rs`, et c'est toujours la copie
/// oubliée qui laisse passer le défaut.
fn valider(lignes: &[Ligne]) -> Result<(), Erreur> {
    let mut contenu = String::new();
    for l in lignes {
        contenu.push_str(&l.rendre());
        contenu.push('\n');
    }
    charger_depuis_texte(&contenu).map(|_| ())
}

/// La moitié « analyse + construction » de [`charger`], sans le disque. Sert à [`valider`].
fn charger_depuis_texte(texte: &str) -> Result<TrousseauMaitre, Erreur> {
    let lignes = analyser(texte)?;
    let vivantes: Vec<u32> = lignes.iter().filter(|l| l.etat == VIVANTE).map(|l| l.numero).collect();
    if vivantes.len() > 1 {
        return Err(Erreur::DeuxVivantes { premiere: vivantes[0], seconde: vivantes[1] });
    }
    let mut t = TrousseauMaitre::new();
    for l in &lignes {
        let r = match l.etat.as_str() {
            DETRUITE => t.poser_detruite(l.numero, l.jusqu_a.unwrap_or(l.depuis)),
            RETIREE => t
                .poser(l.numero, TenantKey::from_bytes(octets(&l.cle_hex)), l.depuis)
                .and_then(|()| t.retirer(l.numero, l.jusqu_a.unwrap_or(l.depuis))),
            _ => t.poser(l.numero, TenantKey::from_bytes(octets(&l.cle_hex)), l.depuis),
        };
        if let Err(cause) = r {
            return Err(Erreur::Incoherent { cause });
        }
    }
    Ok(t)
}

/// Installe une génération dans le fichier. La clé arrive en **hexadécimal**, de l'exploitant.
///
/// Ce module ne fabrique jamais d'entropie : une clé tenant se dérive d'un maître ou se tire au
/// hasard, et les deux gestes appartiennent à celui qui détient le secret, pas à celui qui tient le
/// registre des générations.
pub fn poser_generation(chemin: &Path, numero: u32, cle_hex: &str, depuis: u64) -> Result<(), Erreur> {
    if !hex_valide(cle_hex) {
        return Err(Erreur::CleInvalide { numero_ligne: 0 });
    }
    let mut lignes = lignes_de(chemin)?;
    lignes.push(Ligne {
        numero,
        etat: VIVANTE.into(),
        cle_hex: cle_hex.to_string(),
        depuis,
        jusqu_a: None,
    });
    lignes.sort_by_key(|l| l.numero);
    valider(&lignes)?;
    ecrire_atomique(chemin, &lignes)
}

/// Retire une génération : elle cesse de sceller, elle continue de re-prouver.
pub fn retirer_generation(chemin: &Path, numero: u32, le: u64) -> Result<(), Erreur> {
    let mut lignes = lignes_de(chemin)?;
    let l = lignes
        .iter_mut()
        .find(|l| l.numero == numero)
        .ok_or(Erreur::GenerationAbsente { numero })?;
    if l.etat != VIVANTE {
        return Err(Erreur::EtatIncompatible { numero, etat: l.etat.clone() });
    }
    l.etat = RETIREE.into();
    l.jusqu_a = Some(le);
    valider(&lignes)?;
    ecrire_atomique(chemin, &lignes)
}

/// **Détruit** une génération : la clé quitte le fichier, la date reste. Irréversible.
///
/// Le fichier d'abord, toujours : c'est la seule façon que la destruction soit vraie. Un appelant
/// qui détruirait d'abord en mémoire et échouerait ensuite à écrire aurait déclaré détruite une clé
/// intacte sur le disque.
pub fn detruire_generation(chemin: &Path, numero: u32, le: u64) -> Result<(), Erreur> {
    let mut lignes = lignes_de(chemin)?;
    let l = lignes
        .iter_mut()
        .find(|l| l.numero == numero)
        .ok_or(Erreur::GenerationAbsente { numero })?;
    if l.etat == DETRUITE {
        return Err(Erreur::EtatIncompatible { numero, etat: DETRUITE.into() });
    }
    l.etat = DETRUITE.into();
    l.cle_hex = RIEN.into(); // la clé QUITTE le fichier — c'est là que la destruction a lieu
    l.jusqu_a = Some(le);
    valider(&lignes)?;
    ecrire_atomique(chemin, &lignes)
}

/// **Le geste de rotation**, en UNE seule réécriture.
///
/// Retirer puis poser en deux écritures laisserait, entre les deux, un fichier sans aucune
/// génération vivante — et une panne à cet instant précis rendrait un service incapable de sceller
/// quoi que ce soit au redémarrage. Un seul `rename` : le fichier passe de l'ancien état au nouveau
/// sans jamais exposer l'état intermédiaire.
pub fn tourner(chemin: &Path, numero: u32, cle_hex: &str, quand: u64) -> Result<(), Erreur> {
    if !hex_valide(cle_hex) {
        return Err(Erreur::CleInvalide { numero_ligne: 0 });
    }
    let mut lignes = lignes_de(chemin)?;
    for l in lignes.iter_mut() {
        if l.etat == VIVANTE {
            l.etat = RETIREE.into();
            l.jusqu_a = Some(quand);
        }
    }
    lignes.push(Ligne {
        numero,
        etat: VIVANTE.into(),
        cle_hex: cle_hex.to_string(),
        depuis: quand,
        jusqu_a: None,
    });
    lignes.sort_by_key(|l| l.numero);
    valider(&lignes)?;
    ecrire_atomique(chemin, &lignes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generations::EtatGeneration;

    fn fichier(nom: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("scella-tm-{}-{}", nom, std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn hex(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    #[test]
    fn un_trousseau_absent_naît_vide_au_lieu_d_echouer() {
        let f = fichier("absent");
        poser_generation(&f, 1, &hex(1), 100).expect("un trousseau naît vide");
        let (t, _) = charger(&f).unwrap();
        assert_eq!(t.courante().map(|(n, _)| n), Some(1));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn la_date_de_destruction_survit_au_redemarrage() {
        // LE test de ce module : c'est exactement ce que l'absence de fichier faisait perdre.
        let f = fichier("date");
        poser_generation(&f, 1, &hex(1), 100).unwrap();
        detruire_generation(&f, 1, 777).unwrap();
        let (t, _) = charger(&f).unwrap();
        assert_eq!(
            t.etat(1),
            EtatGeneration::Detruite { le: 777 },
            "« détruite le 777 » ne doit pas redevenir « jamais vue »"
        );
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn la_cle_quitte_vraiment_le_fichier_a_la_destruction() {
        let f = fichier("cle-partie");
        poser_generation(&f, 1, &hex(0xAB), 100).unwrap();
        assert!(std::fs::read_to_string(&f).unwrap().contains(&hex(0xAB)));
        detruire_generation(&f, 1, 500).unwrap();
        let apres = std::fs::read_to_string(&f).unwrap();
        assert!(
            !apres.contains(&hex(0xAB)),
            "une destruction qui laisse la clé sur le disque est un mensonge : {apres}"
        );
        assert!(apres.contains("DETRUITE"), "et la pierre tombale reste : {apres}");
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn la_rotation_ne_passe_jamais_par_un_fichier_sans_vivante() {
        let f = fichier("rotation");
        poser_generation(&f, 1, &hex(1), 100).unwrap();
        tourner(&f, 2, &hex(2), 200).unwrap();
        let texte = std::fs::read_to_string(&f).unwrap();
        assert_eq!(texte.matches(VIVANTE).count(), 1, "exactement une vivante : {texte}");
        let (t, _) = charger(&f).unwrap();
        assert_eq!(t.courante().map(|(n, _)| n), Some(2));
        assert_eq!(t.etat(1), EtatGeneration::Retiree { le: 200 });
        assert!(t.cle_de_lecture(1).is_some(), "la retirée re-prouve encore");
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn un_aller_retour_ne_perd_pas_la_date_d_entree_d_une_detruite() {
        // Le bénéfice de la transformation de lignes : `depuis` n'est pas modélisé en mémoire pour
        // une détruite, et pourtant il survit — une réécriture depuis l'état l'aurait effacé.
        let f = fichier("depuis-survit");
        poser_generation(&f, 1, &hex(1), 12345).unwrap();
        detruire_generation(&f, 1, 999).unwrap();
        tourner(&f, 2, &hex(2), 1000).unwrap(); // une réécriture complète de plus
        let texte = std::fs::read_to_string(&f).unwrap();
        assert!(texte.contains("12345"), "la date d'entrée en service a survécu : {texte}");
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn deux_vivantes_dans_le_fichier_sont_refusees_en_les_nommant() {
        let f = fichier("deux-vivantes");
        std::fs::write(
            &f,
            format!("1\t{VIVANTE}\t{}\t100\t-\n2\t{VIVANTE}\t{}\t200\t-\n", hex(1), hex(2)),
        )
        .unwrap();
        match charger(&f) {
            Err(Erreur::DeuxVivantes { premiere: 1, seconde: 2 }) => {}
            autre => panic!("attendu DeuxVivantes(1,2), obtenu {autre:?}"),
        }
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn une_pierre_tombale_qui_porte_une_cle_est_refusee() {
        // Le fichier prétendrait avoir détruit une clé qu'il contient encore.
        let f = fichier("tombale-menteuse");
        std::fs::write(&f, format!("1\t{DETRUITE}\t{}\t100\t500\n", hex(1))).unwrap();
        assert!(matches!(charger(&f), Err(Erreur::CleInvalide { .. })));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn une_vivante_sans_cle_est_refusee_au_lieu_de_tout_rendre_illisible() {
        let f = fichier("vivante-sans-cle");
        std::fs::write(&f, format!("1\t{VIVANTE}\t-\t100\t-\n")).unwrap();
        assert!(matches!(charger(&f), Err(Erreur::CleManquante { generation: 1, .. })));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn un_numero_deja_detruit_ne_peut_pas_resservir() {
        let f = fichier("numero-mort");
        poser_generation(&f, 7, &hex(7), 100).unwrap();
        detruire_generation(&f, 7, 300).unwrap();
        let avant = std::fs::read_to_string(&f).unwrap();
        assert!(matches!(poser_generation(&f, 7, &hex(9), 400), Err(Erreur::Incoherent { .. })));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), avant, "un refus ne touche pas le fichier");
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn une_arite_fausse_se_voit_au_lieu_de_se_deviner() {
        let f = fichier("arite");
        std::fs::write(&f, format!("1\t{VIVANTE}\t{}\t100\n", hex(1))).unwrap();
        assert!(matches!(charger(&f), Err(Erreur::Arite { colonnes: 4, .. })));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn retirer_une_absente_le_dit_et_ne_touche_a_rien() {
        let f = fichier("retirer-absente");
        poser_generation(&f, 1, &hex(1), 100).unwrap();
        let avant = std::fs::read_to_string(&f).unwrap();
        assert!(matches!(retirer_generation(&f, 9, 200), Err(Erreur::GenerationAbsente { numero: 9 })));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), avant);
        let _ = std::fs::remove_file(&f);
    }
}
