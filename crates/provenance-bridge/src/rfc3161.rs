// SPDX-License-Identifier: UNLICENSED
//! # M9 — l'ancre **tierce** : un horodateur RFC 3161 comme « ailleurs ».
//!
//! ## Ce que M4 avait laissé ouvert, et que le message de démarrage disait déjà
//!
//! > *« l'ancre est la racine attendue, écrite **AILLEURS** — et c'est ce "ailleurs" qui fait tout
//! > le travail, pas la cryptographie »* · *« REMETTEZ ce journal à l'auditeur par un canal DISTINCT
//! > du bundle, sans quoi l'ancre ne prouve rien. »*
//!
//! Avec [`crate::FileAnchorSink`], cet « ailleurs » est **notre propre fichier**, remis par nous. Il
//! détecte une troncature — c'est déjà beaucoup — mais il ne dit rien à qui doute de **nous** : nous
//! aurions pu l'écrire hier, ou ce matin, ou après le litige.
//!
//! Un horodateur tiers change la nature de la chose. Il atteste que **ce condensé existait avant
//! l'instant T**, et il l'atteste avec sa clé, pas la nôtre. C'est le pas qui fait passer le reçu de
//! *tamper-evident pour un détenteur de clé* à **opposable au détenteur lui-même**.
//!
//! ## L'invariant : on ne déclare jamais une ancre qu'on n'a pas
//!
//! [`Rfc3161Sink::publish`] **lit le jeton reçu** et vérifie qu'il engage la racine soumise, avant
//! de rien écrire. Sans ce contrôle, un horodateur défaillant (ou un intermédiaire) pourrait rendre
//! un jeton portant un autre condensé, et l'on déposerait une preuve qui ne prouve rien — pire, une
//! preuve dont un auditeur croirait qu'elle prouve.
//!
//! ## Ce qui est délégué, et à qui
//!
//! Ce module **ne vérifie aucune signature**. La règle du dépôt est explicite — *« aucune crypto
//! maison, aucune 3ᵉ pile »*. Le jeton est déposé **tel quel** ; le tiers le vérifie avec l'outil de
//! son choix :
//!
//! ```text
//! openssl ts -verify -digest <manifest_root> -in <root>.tsr -CAfile <ca-de-lhorodateur>
//! ```
//!
//! La division est nette et aucun des deux côtés ne demande à l'autre d'être cru : **nous**
//! garantissons que le jeton déposé engage la racine déposée ; **le tiers** garantit que
//! l'horodateur l'a signé.
//!
//! ## Le prix, dit d'avance
//!
//! Ce puits **appelle le réseau pendant une requête**. Si l'horodateur est muet, la publication
//! échoue — et la route de bundle échoue avec elle, par le dessin existant qui refuse d'annoncer
//! `anchored: true` sans ancre. C'est un choix d'exploitant : `FileAnchorSink` ne tombe jamais et ne
//! prouve rien contre nous ; celui-ci prouve contre nous et dépend d'un tiers. On ne peut pas avoir
//! les deux, et prétendre le contraire serait la seule erreur vraiment coûteuse ici.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use forensic_export::anchor::{AnchorMethod, AnchorProof};
use provenance_dag::AnchorRecord;
use scella_rfc3161::{jeton, requete};

use crate::anchor::{AnchorSink, PublishError};

/// Le délai par défaut d'un aller-retour avec l'horodateur.
pub const DELAI_DEFAUT: Duration = Duration::from_secs(10);
/// Au-delà, la réponse n'est pas un horodatage.
const REPONSE_MAX: usize = 1 << 20;

/// Pourquoi une ancre tierce n'a pas pu être obtenue. Chaque cas est **nommé** : « échec
/// d'ancrage » n'aide pas un exploitant à savoir s'il doit appeler son horodateur ou relire sa
/// configuration.
#[derive(Debug)]
pub enum ErreurHorodateur {
    /// L'URL ne se décompose pas (analyseur unique du dépôt).
    Url(String),
    /// La racine de l'ancre n'est pas un condensé SHA-256 de 64 caractères hexadécimaux.
    Racine(String),
    /// Le réseau.
    Reseau(std::io::Error),
    /// Réponse HTTP inattendue.
    Http { statut: u16 },
    /// La réponse ne se lit pas comme un horodatage, ou l'horodateur a refusé.
    Jeton(String),
    /// ⭐ Le jeton engage un **autre** condensé que celui soumis.
    Liaison { engage: String, soumis: String },
    /// Le nonce rendu n'est pas celui envoyé : la réponse ne répond pas à **cette** demande.
    Nonce,
}

impl std::fmt::Display for ErreurHorodateur {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErreurHorodateur::Url(u) => write!(f, "URL d'horodateur illisible : « {u} »"),
            ErreurHorodateur::Racine(r) => {
                write!(f, "racine « {r} » : 64 caractères hexadécimaux attendus")
            }
            ErreurHorodateur::Reseau(e) => write!(f, "horodateur injoignable : {e}"),
            ErreurHorodateur::Http { statut } => write!(f, "l'horodateur a répondu HTTP {statut}"),
            ErreurHorodateur::Jeton(m) => write!(f, "{m}"),
            ErreurHorodateur::Liaison { engage, soumis } => write!(
                f,
                "LE JETON N'ENGAGE PAS CETTE RACINE : il engage « {engage} », on a soumis \
                 « {soumis} ». Rien n'est déposé — une preuve qui ne prouve rien est pire que pas \
                 de preuve, parce qu'un auditeur croirait qu'elle prouve"
            ),
            ErreurHorodateur::Nonce => write!(
                f,
                "le nonce rendu n'est pas celui envoyé : cette réponse ne répond pas à CETTE \
                 demande, et pourrait être un jeton ancien rejoué"
            ),
        }
    }
}

impl From<ErreurHorodateur> for PublishError {
    fn from(e: ErreurHorodateur) -> Self {
        PublishError::Io(std::io::Error::other(e.to_string()))
    }
}

/// Un puits d'ancrage qui fait horodater la racine par un **tiers**.
///
/// Il enveloppe un puits interne — typiquement [`crate::FileAnchorSink`] — et lui délègue la ligne
/// de journal. Ce n'est pas une commodité : c'est cette ligne qui détecte une **troncature**, et un
/// horodatage ne la remplace pas. Les deux gardes sont orthogonaux, et perdre le premier en gagnant
/// le second serait un échange, pas un progrès.
pub struct Rfc3161Sink {
    url: String,
    hote: String,
    port: u16,
    chemin: String,
    repertoire: PathBuf,
    journal: Box<dyn AnchorSink>,
    delai: Duration,
}

impl Rfc3161Sink {
    /// Construit le puits. `url` est décomposée par l'**analyseur unique** du dépôt.
    ///
    /// Le port par défaut est 80 : ce module parle en clair, et c'est délibéré — un jeton RFC 3161
    /// est **signé**, donc son intégrité ne dépend pas du canal. Ce que le canal expose est le
    /// **condensé** soumis, qui n'est ni un secret ni réversible. Un exploitant qui veut malgré tout
    /// du chiffrement pointe ce puits vers un horodateur interne ou un mandataire.
    pub fn nouveau(
        url: &str,
        repertoire: impl Into<PathBuf>,
        journal: Box<dyn AnchorSink>,
    ) -> Result<Self, ErreurHorodateur> {
        let cible = scella_authority::decomposer(url)
            .ok_or_else(|| ErreurHorodateur::Url(url.to_string()))?;
        Ok(Rfc3161Sink {
            url: url.to_string(),
            hote: cible.hote.to_string(),
            port: cible.port.unwrap_or(80),
            chemin: cible.chemin.to_string(),
            repertoire: repertoire.into(),
            journal,
            delai: DELAI_DEFAUT,
        })
    }

    /// Change le délai d'aller-retour.
    #[must_use]
    pub fn avec_delai(mut self, d: Duration) -> Self {
        self.delai = d;
        self
    }

    /// Le chemin du jeton déposé pour une racine.
    #[must_use]
    pub fn chemin_jeton(&self, racine: &str) -> PathBuf {
        self.repertoire.join(format!("{racine}.tsr"))
    }

    /// Le chemin de la preuve structurée déposée pour une racine.
    #[must_use]
    pub fn chemin_preuve(&self, racine: &str) -> PathBuf {
        self.repertoire.join(format!("{racine}.proof"))
    }

    /// Obtient un jeton pour une racine, **vérifie qu'il l'engage**, et le rend avec son instant.
    ///
    /// Rien n'est écrit ici : cette fonction obtient et contrôle, [`Self::publish`] dépose. Séparer
    /// les deux est ce qui garantit qu'aucun fichier n'apparaît pour une ancre non vérifiée.
    pub fn obtenir(&self, racine_hex: &str, nonce: &[u8]) -> Result<(Vec<u8>, u64), ErreurHorodateur> {
        let racine = racine_32(racine_hex)
            .ok_or_else(|| ErreurHorodateur::Racine(racine_hex.to_string()))?;
        let corps = requete::demande(&racine, nonce);
        let reponse = self.poster(&corps)?;

        let (jeton_der, info) =
            jeton::lire_reponse(&reponse).map_err(|e| ErreurHorodateur::Jeton(e.to_string()))?;

        // ⭐ Le contrôle qui empêche de déclarer une ancre qu'on n'a pas.
        if !info.engage(&racine) {
            return Err(ErreurHorodateur::Liaison {
                engage: hex(&info.empreinte),
                soumis: racine_hex.to_string(),
            });
        }
        // Le nonce, quand l'horodateur en rend un : il lie la réponse à CETTE demande. Un horodateur
        // qui n'en rend aucun n'est pas fautif (le champ est facultatif) — on ne refuse donc que la
        // DIVERGENCE, jamais l'absence.
        if !nonce.is_empty() && !info.nonce.is_empty() {
            let attendu: Vec<u8> = nonce.iter().copied().skip_while(|b| *b == 0).collect();
            if info.nonce != attendu {
                return Err(ErreurHorodateur::Nonce);
            }
        }
        Ok((jeton_der, info.instant))
    }

    /// Un POST HTTP/1.1 écrit à la main — même geste que le pilote de M4, et pour la même raison :
    /// une dépendance HTTP dans le pont pèserait plus que les quarante lignes qu'elle remplace.
    fn poster(&self, corps: &[u8]) -> Result<Vec<u8>, ErreurHorodateur> {
        let adresses: Vec<std::net::SocketAddr> =
            std::net::ToSocketAddrs::to_socket_addrs(&(self.hote.as_str(), self.port))
                .map_err(ErreurHorodateur::Reseau)?
                .collect();
        let premiere = adresses
            .first()
            .ok_or_else(|| ErreurHorodateur::Reseau(std::io::Error::other("aucune adresse")))?;
        let mut flux =
            TcpStream::connect_timeout(premiere, self.delai).map_err(ErreurHorodateur::Reseau)?;
        flux.set_read_timeout(Some(self.delai)).map_err(ErreurHorodateur::Reseau)?;
        flux.set_write_timeout(Some(self.delai)).map_err(ErreurHorodateur::Reseau)?;

        let entete = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: {}\r\nAccept: {}\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            self.chemin,
            self.hote,
            requete::TYPE_REQUETE,
            requete::TYPE_REPONSE,
            corps.len()
        );
        flux.write_all(entete.as_bytes()).map_err(ErreurHorodateur::Reseau)?;
        flux.write_all(corps).map_err(ErreurHorodateur::Reseau)?;
        flux.flush().map_err(ErreurHorodateur::Reseau)?;

        let mut brut = Vec::new();
        // Borné : une réponse sans fin ne doit pas remplir la mémoire du processus.
        std::io::Read::take(&mut flux, (REPONSE_MAX + 8192) as u64)
            .read_to_end(&mut brut)
            .map_err(ErreurHorodateur::Reseau)?;

        let sep = trouver(&brut, b"\r\n\r\n")
            .ok_or_else(|| ErreurHorodateur::Jeton("réponse HTTP sans en-tête complet".into()))?;
        let tete = String::from_utf8_lossy(&brut[..sep]).to_string();
        let statut: u16 = tete
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| ErreurHorodateur::Jeton("ligne de statut HTTP illisible".into()))?;
        if statut != 200 {
            return Err(ErreurHorodateur::Http { statut });
        }
        Ok(brut[sep + 4..].to_vec())
    }
}

impl AnchorSink for Rfc3161Sink {
    fn publish(&self, anchor: &AnchorRecord) -> Result<(), PublishError> {
        // Le nonce vient d'`uuid` — déjà dans l'arbre du pont, et une source d'aléa légitime. Ce
        // module n'en invente aucun : `scella-rfc3161` n'a pas d'entropie, par construction.
        let nonce = *uuid::Uuid::new_v4().as_bytes();
        let (jeton_der, instant) = self.obtenir(&anchor.manifest_root, &nonce)?;

        let preuve = AnchorProof {
            method: AnchorMethod::Rfc3161,
            issuer: self.hote.clone(),
            bound_root: anchor.manifest_root.clone(),
            timestamp: instant,
            receipt: jeton_der.clone(),
        };
        let encodee = preuve
            .encode()
            .map_err(|e| PublishError::Io(std::io::Error::other(format!("{e:?}"))))?;

        std::fs::create_dir_all(&self.repertoire).map_err(PublishError::Io)?;
        ecrire_atomique(&self.chemin_jeton(&anchor.manifest_root), &jeton_der)?;
        ecrire_atomique(&self.chemin_preuve(&anchor.manifest_root), &encodee)?;

        // La ligne de journal EN DERNIER : elle annonce qu'une ancre existe, et elle ne doit
        // l'annoncer qu'une fois le jeton réellement sur le disque.
        self.journal.publish(anchor)
    }

    fn describe(&self) -> String {
        format!(
            "horodateur RFC 3161 « {} » → {} · + {} — le jeton est déposé TEL QUEL : sa signature \
             se vérifie chez le tiers (`openssl ts -verify -digest <root> -in <root>.tsr`), pas ici",
            self.url,
            self.repertoire.display(),
            self.journal.describe()
        )
    }
}

/// Écrit un fichier par temporaire + `rename` : un dépôt interrompu ne laisse jamais un jeton
/// tronqué qu'un auditeur prendrait pour une preuve.
fn ecrire_atomique(chemin: &Path, octets: &[u8]) -> Result<(), PublishError> {
    let temporaire = chemin.with_extension("scella-tmp");
    {
        let mut f = std::fs::File::create(&temporaire).map_err(PublishError::Io)?;
        f.write_all(octets).map_err(PublishError::Io)?;
        f.sync_all().map_err(PublishError::Io)?;
    }
    std::fs::rename(&temporaire, chemin).map_err(|e| {
        let _ = std::fs::remove_file(&temporaire);
        PublishError::Io(e)
    })
}

fn racine_32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn trouver(foin: &[u8], aiguille: &[u8]) -> Option<usize> {
    foin.windows(aiguille.len()).position(|f| f == aiguille)
}
