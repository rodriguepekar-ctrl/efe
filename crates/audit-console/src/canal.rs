// SPDX-License-Identifier: UNLICENSED
//! # `canal` — M8 : le barreau de confiance du canal, et le plancher qu'un acte humain exige.
//!
//! ## Le trou, énoncé par le dépôt lui-même
//!
//! `efe-nano-sdk` a déjà résolu ce problème une fois, pour la **computation**. Sa doc RC-1 dit :
//!
//! > *« Aujourd'hui la localité est **invisible** dans les octets scellés : un record produit à
//! > distance serait byte-identique à un local. »*
//!
//! C'est mot pour mot le trou de M8, transposé aux **actes humains**. Un nœud `Human` scellé dit
//! `human_id: "officer:hanna"` et `action: Approve`. Qu'il vienne d'un officier devant sa console
//! en boucle locale, de quelqu'un qui a lu le jeton dans un fichier, ou d'un réseau où le jeton a
//! traversé en clair — les octets scellés sont **identiques**. « Opposable » ne peut pas vouloir
//! dire autre chose que : ces conditions-là sont tenues, et tenues **avant** de sceller.
//!
//! ## Pourquoi ce module ENFORCE au lieu d'ENREGISTRER
//!
//! La réponse de RC-1 est d'**enregistrer** le barreau dans le record. Ici, c'est impossible sans
//! changer le format de `Node::Human` — donc sans invalider toutes les chaînes existantes, ce qui
//! est un événement de version, pas un module. (Le détour par un nœud `Evidence` marcherait
//! techniquement — `consumed_by_node` n'est pas une référence, cf. `Node::refs` — mais
//! `reg-evidence` mappe `NodeKind::Evidence` à l'art. 13 « reconstruit la chaîne de raisonnement » :
//! des nœuds d'authentification y seraient comptés comme des preuves de raisonnement. On ne pollue
//! pas un artefact réglementaire pour se simplifier la vie.)
//!
//! Alors on applique la règle de M3 : *dégrader produit-il quelque chose d'utile **et honnête** ?*
//!
//! * **Lire** sur un canal faible : on perd de la **confidentialité**. C'est grave, c'est réparable
//!   en changeant de canal, et cela n'inscrit rien nulle part.
//! * **Écrire** sur un canal faible : on inscrit une **fausse attestation dans une chaîne non
//!   répudiable**. Irréversible, et opposable à quelqu'un qui n'a rien fait. Sceller un acte
//!   faiblement authentifié produirait un record *utile* et **malhonnête**.
//!
//! ⟹ **La lecture dégrade, l'écriture COUPE.** Un acte humain n'est scellé que si le canal qui l'a
//! porté atteint le plancher déclaré ; sinon **403, et rien n'est scellé**.
//!
//! ## Ce que le processus PEUT observer, et ce qu'il ne peut que RÉPÉTER
//!
//! La distinction de RC-1 entre *attester la computation* (R2, témoin) et *attester la transaction*
//! (R4, canal) se transpose exactement :
//!
//! * [`Canal::A1LoopbackTemoin`] — le processus **observe** que la connexion vient de la boucle
//!   locale. Il en témoigne : aucun réseau n'a porté le jeton.
//! * [`Canal::A2ProtegeDeclare`] — le processus **ne peut rien vérifier**. Il répète ce que
//!   l'exploitant a déclaré (« un terminateur TLS est devant moi »). C'est une déclaration, et le
//!   nom du type le dit. Un exploitant qui ment scelle sous **sa** clé, pas sous la nôtre.
//! * [`Canal::A3NonProtege`] — distant, rien de déclaré. Le jeton a pu traverser en clair.
//! * [`Canal::NonAssertee`] — le défaut, **strictement plus faible que A3**, pour qu'un chemin qui
//!   aurait oublié de qualifier son canal ne puisse jamais passer pour fort. Même geste que
//!   `LegacyUnspecified` dans RC-1.
//!
//! ## Deux limites, à dire plutôt qu'à masquer
//!
//! **1. Un client qui CONTOURNE le terminateur est crédité « protégé ».** La déclaration vaut pour
//! le processus, pas pour la connexion : si l'exploitant expose le service directement en plus de
//! son ingress, un pair qui se connecte sans passer par lui reçoit quand même
//! [`Canal::A2ProtegeDeclare`]. Ce processus **ne peut pas** faire la différence — le seul témoin
//! serait un en-tête posé par le terminateur, et un en-tête est falsifiable par le client, donc
//! inutilisable ici (cf. l'invariant ci-dessous). La parade est un geste de déploiement : ne pas
//! exposer le service à côté de son terminateur.
//!
//! **2. [`Canal::A3NonProtege`] est INATTEIGNABLE sur le chemin servi**, et c'est cohérent : sans
//! déclaration, `serve` refuse un bind non-local, donc tout pair est en boucle ; avec déclaration,
//! tout pair distant est crédité A2. La garde de bind et le barreau sont **deux expressions de la
//! même règle**. Le rang existe pour nommer l'état que la garde empêche — et pour que les tests
//! puissent l'exercer.
//!
//! ## L'invariant qui rend tout cela vrai
//!
//! **Le barreau vient de la CONNEXION, jamais d'un en-tête.** Un client qui pourrait annoncer son
//! propre niveau de confiance rendrait l'échelle décorative. C'est pourquoi [`Canal`] est passé au
//! gestionnaire en **paramètre séparé** de la requête, et non porté par elle — et un test interdit
//! qu'aucun en-tête ne l'influence.

use std::fmt;
use std::net::{IpAddr, SocketAddr};

/// Le barreau de confiance du canal qui a porté une requête. **Échelle totale et décroissante** :
/// `A1 > A2 > A3 > NonAssertee`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Canal {
    /// **A1** — la connexion vient de la **boucle locale**, et le processus l'a **observé**. Le
    /// jeton n'a traversé aucun réseau. C'est le seul barreau que ce processus puisse *témoigner*.
    A1LoopbackTemoin,
    /// **A2** — distant, mais l'exploitant **déclare** un terminateur de canal devant le service.
    /// Le processus ne le vérifie pas et ne peut pas le vérifier : il répète une déclaration.
    A2ProtegeDeclare {
        /// Ce que l'exploitant a déclaré (« ingress-nginx », « envoy-mtls »…). Jamais interprété —
        /// c'est une **trace de responsabilité**, pas une configuration.
        terminateur: String,
    },
    /// **A3** — distant, rien de déclaré. Le jeton a pu traverser en clair.
    A3NonProtege,
    /// Le barreau n'a pas été qualifié. **Strictement plus faible que A3** : un chemin qui aurait
    /// oublié de le poser ne doit jamais passer pour fort.
    NonAssertee,
}

impl Canal {
    /// Rang de force (plus haut = plus fort). L'échelle est **totale**.
    #[must_use]
    pub fn rang(&self) -> u8 {
        match self {
            Canal::A1LoopbackTemoin => 3,
            Canal::A2ProtegeDeclare { .. } => 2,
            Canal::A3NonProtege => 1,
            Canal::NonAssertee => 0,
        }
    }

    /// Le nom court du barreau — pour un journal, une réponse, un message de refus.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Canal::A1LoopbackTemoin => "a1_loopback_temoin",
            Canal::A2ProtegeDeclare { .. } => "a2_protege_declare",
            Canal::A3NonProtege => "a3_non_protege",
            Canal::NonAssertee => "non_assertee",
        }
    }

    /// **Qualifie une connexion.** L'unique fabrique : le barreau vient du **pair** et de la
    /// déclaration d'exploitation, jamais de la requête.
    ///
    /// `pair` absent (`peer_addr` a échoué) ⟹ [`Canal::NonAssertee`] : ne rien savoir n'autorise pas
    /// à supposer le meilleur.
    #[must_use]
    pub fn observer(pair: Option<SocketAddr>, terminateur_declare: Option<&str>) -> Canal {
        match pair {
            None => Canal::NonAssertee,
            Some(a) if est_loopback(a.ip()) => Canal::A1LoopbackTemoin,
            Some(_) => match terminateur_declare {
                Some(t) if !t.trim().is_empty() => {
                    Canal::A2ProtegeDeclare { terminateur: t.trim().to_string() }
                }
                _ => Canal::A3NonProtege,
            },
        }
    }
}

/// Une adresse est-elle de boucle locale ?
///
/// `Ipv4Addr::is_loopback` couvre tout `127.0.0.0/8` et `Ipv6Addr::is_loopback` couvre `::1`. On
/// traite en plus les IPv4 **mappées en IPv6** (`::ffff:127.0.0.1`) — une pile double les produit
/// couramment, et les manquer classerait une connexion locale en « non protégée », c'est-à-dire
/// **refuserait** une écriture parfaitement légitime.
fn est_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().map(|m| m.is_loopback()).unwrap_or(false)
        }
    }
}

impl fmt::Display for Canal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Canal::A1LoopbackTemoin => f.write_str("A1 — boucle locale, observée par le processus"),
            Canal::A2ProtegeDeclare { terminateur } => {
                write!(f, "A2 — canal protégé DÉCLARÉ (« {terminateur} ») ; non vérifiable ici")
            }
            Canal::A3NonProtege => f.write_str("A3 — distant, aucun terminateur déclaré"),
            Canal::NonAssertee => f.write_str("barreau non asserti"),
        }
    }
}

/// Le **plancher** qu'un acte humain doit franchir pour être scellé.
///
/// Défaut : [`Canal::A1LoopbackTemoin`] — c'est-à-dire exactement le comportement d'avant M8, où
/// `assert_local` interdisait tout bind distant. Un exploitant qui veut des écritures à distance
/// **déclare** son terminateur, ce qui abaisse le plancher à A2. Personne ne peut l'abaisser à A3 :
/// sceller une identité sur un canal dont on sait qu'il n'est pas protégé n'est pas un compromis,
/// c'est une signature à la place de quelqu'un.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plancher(u8);

impl Default for Plancher {
    fn default() -> Self {
        Plancher(Canal::A1LoopbackTemoin.rang())
    }
}

impl Plancher {
    /// Le plancher le plus haut : seules les écritures en boucle locale sont scellées.
    #[must_use]
    pub fn loopback() -> Self {
        Plancher(Canal::A1LoopbackTemoin.rang())
    }

    /// Le plancher abaissé à A2 — pour un déploiement qui déclare un terminateur de canal.
    ///
    /// Il n'existe **délibérément aucun** constructeur pour A3 ou en dessous. Ce n'est pas un oubli :
    /// un plancher que l'on peut mettre à zéro n'est pas un plancher, et la seule raison de vouloir
    /// le faire serait de sceller des actes qu'on sait mal authentifiés.
    #[must_use]
    pub fn protege_declare() -> Self {
        Plancher(Canal::A2ProtegeDeclare { terminateur: String::new() }.rang())
    }

    /// Ce canal franchit-il le plancher ?
    #[must_use]
    pub fn franchi_par(&self, c: &Canal) -> bool {
        c.rang() >= self.0
    }

    /// Le refus, rédigé. Il dit **ce qui manque** et **quel geste le corrige** — un 403 nu enverrait
    /// l'exploitant chercher une permission alors que le problème est un canal.
    #[must_use]
    pub fn refus(&self, c: &Canal) -> String {
        format!(
            "canal insuffisant pour SCELLER un acte de supervision : {c}. Plancher exigé : {}. \
             Lire reste possible — mais sceller une identité sur ce canal inscrirait une attestation \
             non répudiable que rien ne soutient. Corriger : agir depuis la boucle locale, ou \
             déclarer le terminateur de canal du déploiement.",
            if self.0 >= Canal::A1LoopbackTemoin.rang() { "A1 (boucle locale)" } else { "A2 (canal déclaré)" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sock(s: &str) -> SocketAddr {
        s.parse().expect("adresse")
    }

    #[test]
    fn lechelle_est_totale_et_decroissante() {
        let a1 = Canal::A1LoopbackTemoin;
        let a2 = Canal::A2ProtegeDeclare { terminateur: "ingress".into() };
        let a3 = Canal::A3NonProtege;
        let n = Canal::NonAssertee;
        assert!(a1.rang() > a2.rang() && a2.rang() > a3.rang() && a3.rang() > n.rang());
        assert_eq!(n.rang(), 0, "le non-asserti est strictement le plus faible");
    }

    #[test]
    fn la_boucle_locale_est_temoignee_sous_toutes_ses_ecritures() {
        for a in ["127.0.0.1:5000", "127.9.9.9:5000", "[::1]:5000", "[::ffff:127.0.0.1]:5000"] {
            assert_eq!(
                Canal::observer(Some(sock(a)), None),
                Canal::A1LoopbackTemoin,
                "{a} est de la boucle locale — la manquer REFUSERAIT une écriture légitime"
            );
        }
    }

    #[test]
    fn un_pair_distant_sans_declaration_est_non_protege() {
        assert_eq!(Canal::observer(Some(sock("10.0.0.5:5000")), None), Canal::A3NonProtege);
        assert_eq!(Canal::observer(Some(sock("10.0.0.5:5000")), Some("   ")), Canal::A3NonProtege,
                   "une déclaration blanche n'est pas une déclaration");
    }

    #[test]
    fn un_pair_distant_avec_declaration_est_a2_et_la_repete() {
        let c = Canal::observer(Some(sock("10.0.0.5:5000")), Some(" ingress-nginx "));
        assert_eq!(c, Canal::A2ProtegeDeclare { terminateur: "ingress-nginx".into() });
        assert!(c.to_string().contains("non vérifiable ici"), "le type DIT qu'il répète : {c}");
    }

    /// Ne rien savoir n'autorise pas à supposer le meilleur.
    #[test]
    fn un_pair_inconnu_nest_pas_asserti() {
        assert_eq!(Canal::observer(None, Some("ingress")), Canal::NonAssertee);
        assert_eq!(Canal::observer(None, None), Canal::NonAssertee);
    }

    #[test]
    fn le_plancher_par_defaut_est_la_boucle_locale() {
        let p = Plancher::default();
        assert_eq!(p, Plancher::loopback(), "le défaut = le comportement d'avant M8");
        assert!(p.franchi_par(&Canal::A1LoopbackTemoin));
        assert!(!p.franchi_par(&Canal::A2ProtegeDeclare { terminateur: "x".into() }));
        assert!(!p.franchi_par(&Canal::A3NonProtege));
        assert!(!p.franchi_par(&Canal::NonAssertee));
    }

    #[test]
    fn le_plancher_declare_accepte_a2_mais_jamais_a3() {
        let p = Plancher::protege_declare();
        assert!(p.franchi_par(&Canal::A1LoopbackTemoin), "le plus fort passe toujours");
        assert!(p.franchi_par(&Canal::A2ProtegeDeclare { terminateur: "x".into() }));
        assert!(!p.franchi_par(&Canal::A3NonProtege), "un canal non protégé ne scelle JAMAIS");
        assert!(!p.franchi_par(&Canal::NonAssertee));
    }

    /// **Il n'existe aucun moyen de mettre le plancher à A3 ou en dessous.** Ce test fige une
    /// propriété de l'API : un plancher que l'on peut annuler n'est pas un plancher.
    #[test]
    fn aucun_constructeur_ne_descend_sous_a2() {
        let src = include_str!("canal.rs");
        let hors_tests = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
        let code: String =
            hors_tests.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        assert!(!code.contains("A3NonProtege.rang()"), "aucun plancher ne se construit sur A3");
        assert!(!code.contains("Plancher(0)"), "ni sur le non-asserti");
        assert_eq!(code.matches("pub fn loopback").count() + code.matches("pub fn protege_declare").count(), 2,
                   "exactement deux constructeurs de plancher, et pas un de plus");
    }

    #[test]
    fn le_refus_dit_le_geste_qui_corrige() {
        let r = Plancher::loopback().refus(&Canal::A3NonProtege);
        assert!(r.contains("Lire reste possible"), "il distingue lire d'écrire : {r}");
        assert!(r.contains("boucle locale") && r.contains("terminateur"), "il donne les deux gestes : {r}");
        assert!(r.contains("non répudiable"), "et pourquoi c'est irréversible : {r}");
    }
}
