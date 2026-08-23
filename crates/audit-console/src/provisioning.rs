// SPDX-License-Identifier: UNLICENSED
//! # `provisioning` — M7 : de la démonstration au **service**.
//!
//! ## Ce que M7 n'est pas
//!
//! Le critère de la feuille de route dit « `audit-api` répond en HTTP sur un port (aujourd'hui :
//! bibliothèque) », et sa preuve est `ls crates/audit-api/src/bin` → absent. Or **la propriété est
//! déjà vraie** : `audit-console` sert les six routes de `audit_api::router::route` sur
//! `/api/tenants/{t}/…`, en `Authorization: Bearer`, avec l'invariant `I-S3-VUE-PURE` (le corps et
//! le statut sont **ceux de `route()`**, la coquille n'ajoute aucune décision).
//!
//! La preuve mesurait donc **l'emplacement d'un binaire**, pas la propriété qu'elle énonce. Et lui
//! obéir aurait été nuisible : donner à `audit-api` sa propre coquille HTTP, c'est soit dupliquer
//! les 570 lignes de serveur écrites à la main dans S3, soit faire dépendre S2 de S3 — une
//! inversion de couche. C'est exactement le défaut que M1 a passé un module entier à supprimer.
//! La feuille de route l'autorise elle-même : *« si l'un de ces critères ne peut pas être exécuté,
//! c'est le critère qu'il faut réécrire, pas le module »*.
//!
//! ## Ce que M7 est
//!
//! Le binaire qui sert ce registre **ne peut tenir qu'un seul tenant** :
//!
//! ```text
//! console.provision(&tenant, key);                                  // UN tenant
//! console.add_session(&token, &tenant, Role::ComplianceOfficer, …);  // UNE session, rôle figé
//! ```
//!
//! `TenantRegistry` est multi-tenant ; `route()` refuse en **403** un `{t}` qui n'est pas celui du
//! porteur, *« sans même révéler l'existence d'autres tenants »*. Toute cette machinerie
//! d'isolation — la promesse même de M8 — **n'a aujourd'hui aucun moyen d'être exercée dans un
//! service vivant**, puisque le service vivant n'a qu'un tenant. C'est le trou que M7 comble, et
//! c'est pour cela que la feuille de route écrit `M7 → M8`.
//!
//! ## Le danger qui a dicté la forme de ce module
//!
//! `Sessions::insert` est un `HashMap::insert` : il **écrase en silence**. Deux sessions déclarées
//! avec le **même jeton** et des tenants différents, et le porteur de ce jeton atteint **l'autre
//! tenant** — « un tenant voit l'autre », introduit par une faute de frappe dans une configuration,
//! sans qu'une seule ligne ne proteste.
//!
//! Une déclaration est donc **validée avant d'être appliquée**, et un refus **arrête le démarrage**.
//! Ouvrir un port avec une isolation qu'on n'a pas vérifiée serait la pire des dégradations
//! silencieuses : tout aurait l'air de fonctionner.
//!
//! ## Pourquoi un fichier, et pas des variables
//!
//! Les jetons sont des **secrets**, et la règle du dépôt est qu'un profil n'en contient jamais. Le
//! profil ne porte donc que le **chemin** (`EFE_CONSOLE_DEPLOIEMENT`) ; les secrets vivent dans un
//! fichier monté, à la manière dont un déploiement les monte réellement. Et **aucun jeton n'est
//! jamais imprimé** : le résumé d'un démarrage compte, il ne cite pas.

use std::collections::BTreeMap;
use std::fmt;

use audit_api::Role;
use provenance_dag::{SealedDag, TenantKey};
use serde::Deserialize;

use crate::app::Console;
use crate::session::{empreinte, empreinte_de_hex, Empreinte};

/// Le vocabulaire des rôles, déclaré **une seule fois**.
///
/// `Role` n'a pas de `FromStr` dans S2, et c'est tant mieux : un rôle mal orthographié doit être
/// **refusé**, jamais retombé sur un défaut. Un `unwrap_or(Role::Viewer)` paraîtrait prudent et
/// serait faux — une session d'auditeur silencieusement rétrogradée casse un audit ; et le défaut
/// symétrique (retomber sur `ComplianceOfficer`) accorderait le droit d'**agir** à qui ne l'a pas.
pub const ROLES: &[(&str, Role)] = &[
    ("viewer", Role::Viewer),
    ("compliance_officer", Role::ComplianceOfficer),
    ("auditor", Role::Auditor),
    ("admin", Role::Admin),
];

/// Le rôle nommé par cette chaîne, ou rien. Sensible à la casse : un vocabulaire de configuration
/// qui accepte les variantes finit par accepter les fautes.
#[must_use]
pub fn role_de(s: &str) -> Option<Role> {
    ROLES.iter().find(|(n, _)| *n == s).map(|(_, r)| *r)
}

/// La déclaration d'un déploiement : quels tenants, quelles sessions.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deploiement {
    /// Une note libre, **le seul champ décoratif admis**.
    ///
    /// `deny_unknown_fields` refuse tout le reste — une clé mal orthographiée dans une configuration
    /// d'isolation doit échouer. Mais JSON n'a pas de commentaires, et un fichier de déploiement
    /// qu'on ne peut pas annoter finit recopié sans qu'on sache d'où il vient. Ce champ est déclaré
    /// **explicitement**, donc il n'affaiblit rien : « recods » échoue toujours.
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub tenants: Vec<TenantDecl>,
    #[serde(default)]
    pub sessions: Vec<SessionDecl>,
}

/// Un tenant à servir. `records` réhydrate un log scellé persisté ; absent, le tenant démarre vide
/// (la capture S1 du runtime l'alimente).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TenantDecl {
    pub id: String,
    #[serde(default)]
    pub records: Option<String>,
}

/// Une session : un jeton porteur, le tenant qu'il ouvre, le rôle qu'il porte, et le sujet.
///
/// `role` est une **chaîne**, pas un `Role`. Laisser serde désérialiser directement dans l'énum
/// produirait une erreur de désérialisation générique (« unknown variant ») au lieu d'un refus qui
/// **nomme le danger** — et le message d'un refus est la moitié de sa valeur.
///
/// `subject` n'est pas décoratif : c'est lui qui est **scellé** dans les nœuds `Human` d'une revue.
/// Un sujet erroné est une **fausse attestation dans une chaîne non répudiable**.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDecl {
    /// Le jeton **en clair**. Pratique, mais le fichier devient alors un magasin de secrets.
    #[serde(default)]
    pub token: Option<String>,
    /// **M8** — l'empreinte SHA-256 hexadécimale du jeton (64 caractères), à la place du jeton.
    ///
    /// Le fichier de déclaration cesse alors de contenir le moindre secret : le lire ne donne aucun
    /// identifiant présentable. C'est la forme recommandée ; `token` reste accepté pour ne pas
    /// casser une déclaration écrite avant M8.
    #[serde(default)]
    pub token_sha256: Option<String>,
    pub tenant: String,
    pub role: String,
    pub subject: String,
    /// **M8** — l'instant (époque, secondes) au-delà duquel la session cesse de résoudre.
    ///
    /// Absent = sans échéance, ce qui est **accepté mais compté** au démarrage : un identifiant qui
    /// ne meurt jamais est une décision, pas un défaut.
    #[serde(default)]
    pub expire_le: Option<u64>,
}

impl SessionDecl {
    /// L'empreinte du jeton de cette session — quelle que soit la forme déclarée.
    ///
    /// C'est **par elle** que le partage de jeton se détecte, et il faut voir pourquoi : accepter
    /// deux formes crée le piège où l'une déclare `"x"` et l'autre `sha256("x")`. Elles partagent
    /// un identifiant, et une comparaison de chaînes ne le verrait pas.
    #[must_use]
    pub fn empreinte(&self) -> Option<Empreinte> {
        match (&self.token, &self.token_sha256) {
            (Some(t), None) => Some(empreinte(t)),
            (None, Some(h)) => empreinte_de_hex(h.trim()),
            _ => None,
        }
    }
}

/// Pourquoi une déclaration est refusée. Chacun de ces cas est un danger réel, pas une pédanterie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refus {
    /// ⭐ **Le danger central.** Deux sessions partagent un jeton. `HashMap::insert` garderait la
    /// dernière **en silence** ; si les tenants diffèrent, le porteur atteint l'autre tenant.
    JetonPartage { tenants: (String, String), sujets: (String, String) },
    /// Une session ouvre un tenant qui n'est pas provisionné. `route()` rendrait « tenant inconnu »
    /// (404) — indistinguable d'un tenant qui n'existe pas, alors que c'est une erreur de câblage.
    SessionSansTenant { tenant: String, sujet: String },
    /// Un rôle hors vocabulaire. Ni rétrogradé ni promu : refusé.
    RoleInconnu { valeur: String, sujet: String },
    /// Un jeton vide authentifierait une requête sans en-tête, ou n'importe laquelle.
    JetonVide { sujet: String },
    /// **M8** — ni `token` ni `token_sha256` : cette session n'ouvre rien.
    NiJetonNiEmpreinte { sujet: String },
    /// **M8** — les deux à la fois. Lequel fait foi ? Deviner accorderait un accès sur une lecture
    /// arbitraire du fichier ; et si les deux divergent, l'un des deux est un secret oublié là.
    JetonEtEmpreinte { sujet: String },
    /// **M8** — une empreinte qui n'est pas 64 caractères hexadécimaux. Retomber sur « aucune
    /// session » laisserait un tenant illisible sans que rien ne le dise.
    EmpreinteInvalide { sujet: String },
    /// **M8** — une échéance déjà passée au démarrage : la session naît morte, et l'exploitant
    /// croira que son jeton fonctionne.
    EcheanceDejaPassee { sujet: String, expire_le: u64, maintenant: u64 },
    /// Le jeton de démonstration, publiquement connu, dans une déclaration explicite.
    JetonDeDemonstration { sujet: String },
    /// Deux tenants du même identifiant : le second écraserait le premier, clé comprise.
    TenantDuplique { id: String },
    /// Un identifiant de tenant vide ou blanc.
    TenantVide,
    /// Un log scellé est déclaré mais illisible. **Fatal, et non « on démarre vide »** : un tenant
    /// silencieusement vide ferait conclure à un auditeur que rien ne s'est passé, ce qui est la
    /// pire réponse possible d'un registre de preuve — fausse, et rassurante.
    RecordsIllisible { tenant: String, chemin: String, cause: String },
    /// Aucun tenant : le service ouvrirait un port pour ne rien pouvoir servir.
    AucunTenant,
    /// Des tenants mais aucune session : personne ne peut lire, et rien ne le dirait.
    AucuneSession,
}

impl fmt::Display for Refus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // On nomme les SUJETS et les TENANTS, jamais le jeton : un refus ne doit pas devenir la
            // fuite qu'il prétend empêcher.
            Refus::JetonPartage { tenants, sujets } if tenants.0 != tenants.1 => write!(
                f,
                "JETON PARTAGÉ ENTRE DEUX TENANTS — « {} » ({}) et « {} » ({}) déclarent le même \
                 jeton : le porteur atteindrait l'autre tenant, ce que l'isolation promet d'empêcher",
                sujets.0, tenants.0, sujets.1, tenants.1
            ),
            Refus::JetonPartage { sujets, tenants } => write!(
                f,
                "jeton partagé par « {} » et « {} » dans le tenant « {} » : le sujet retenu serait \
                 arbitraire, or c'est LUI qui est scellé dans les nœuds de revue",
                sujets.0, sujets.1, tenants.0
            ),
            Refus::SessionSansTenant { tenant, sujet } => write!(
                f,
                "la session « {sujet} » ouvre le tenant « {tenant} », qui n'est pas provisionné — \
                 les lectures rendraient « tenant inconnu », indistinguable d'une absence réelle"
            ),
            Refus::RoleInconnu { valeur, sujet } => {
                let connus: Vec<&str> = ROLES.iter().map(|(n, _)| *n).collect();
                write!(
                    f,
                    "rôle inconnu « {valeur} » pour « {sujet} » — attendus : {}. Ni rétrogradé ni \
                     promu : un rôle deviné accorde ou retire des droits en silence",
                    connus.join(", ")
                )
            }
            Refus::JetonVide { sujet } => {
                write!(f, "jeton vide pour « {sujet} » : il authentifierait une requête sans jeton")
            }
            Refus::NiJetonNiEmpreinte { sujet } => write!(
                f,
                "« {sujet} » ne déclare ni « token » ni « token_sha256 » : cette session n'ouvre rien"
            ),
            Refus::JetonEtEmpreinte { sujet } => write!(
                f,
                "« {sujet} » déclare À LA FOIS « token » et « token_sha256 » : lequel fait foi ? \
                 Deviner accorderait un accès sur une lecture arbitraire du fichier"
            ),
            Refus::EmpreinteInvalide { sujet } => write!(
                f,
                "« {sujet} » : « token_sha256 » doit faire 64 caractères hexadécimaux. Retomber sur \
                 « aucune session » laisserait ce tenant illisible sans rien dire"
            ),
            Refus::EcheanceDejaPassee { sujet, expire_le, maintenant } => write!(
                f,
                "la session « {sujet} » expire à {expire_le}, or il est {maintenant} : elle naîtrait \
                 morte, et son porteur croirait son jeton valide"
            ),
            Refus::JetonDeDemonstration { sujet } => write!(
                f,
                "« {sujet} » déclare le jeton de DÉMONSTRATION, publiquement connu : n'importe qui \
                 ouvrirait cette session"
            ),
            Refus::TenantDuplique { id } => {
                write!(f, "tenant « {id} » déclaré deux fois : le second écraserait le premier, clé comprise")
            }
            Refus::RecordsIllisible { tenant, chemin, cause } => write!(
                f,
                "le log scellé du tenant « {tenant} » ({chemin}) est illisible — {cause}. Démarrer \
                 vide ferait conclure qu'il ne s'est rien passé : c'est faux, et rassurant"
            ),
            Refus::TenantVide => write!(f, "identifiant de tenant vide"),
            Refus::AucunTenant => {
                write!(f, "aucun tenant : le service ouvrirait un port sans rien pouvoir servir")
            }
            Refus::AucuneSession => {
                write!(f, "aucune session : personne ne pourrait lire, et rien ne le dirait")
            }
        }
    }
}

/// Ce qu'un démarrage a réellement monté. **Aucun jeton**, par construction : ce résumé est destiné
/// à un journal d'exploitation, et un identifiant dans un journal est un identifiant divulgué.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resume {
    /// Les tenants montés, triés.
    pub tenants: Vec<String>,
    /// Combien ont été réhydratés depuis un log scellé (le reste démarre vide).
    pub rehydrates: usize,
    /// Combien de sessions, **par tenant** — l'exploitant a besoin de savoir qu'un tenant sans
    /// session est illisible, ce qu'un total masquerait.
    pub sessions_par_tenant: BTreeMap<String, usize>,
}

impl Resume {
    /// Le nombre total de sessions.
    #[must_use]
    pub fn sessions(&self) -> usize {
        self.sessions_par_tenant.values().sum()
    }
}

impl Deploiement {
    /// Lit une déclaration JSON. `deny_unknown_fields` est délibéré : une clé mal orthographiée
    /// dans une configuration d'isolation doit **échouer**, pas être ignorée.
    pub fn depuis_json(s: &str) -> Result<Deploiement, String> {
        serde_json::from_str(s).map_err(|e| format!("déclaration illisible : {e}"))
    }

    /// **Valide tout**, et rend **tous** les refus — pas seulement le premier.
    ///
    /// Un opérateur qui corrige sa configuration un démarrage à la fois y passe la journée ; et
    /// surtout, il croit avoir fini au premier succès alors qu'il a corrigé un problème sur trois.
    pub fn valider(&self, maintenant: u64) -> Result<(), Vec<Refus>> {
        let mut refus = Vec::new();

        // ── les tenants ──
        let mut vus: BTreeMap<&str, ()> = BTreeMap::new();
        for t in &self.tenants {
            if t.id.trim().is_empty() {
                refus.push(Refus::TenantVide);
                continue;
            }
            if vus.insert(t.id.as_str(), ()).is_some() {
                refus.push(Refus::TenantDuplique { id: t.id.clone() });
            }
        }
        if self.tenants.is_empty() {
            refus.push(Refus::AucunTenant);
        }
        if self.sessions.is_empty() {
            refus.push(Refus::AucuneSession);
        }

        // ── les sessions ──
        let demo = empreinte(crate::profile::switch("EFE_CONSOLE_TOKEN").map_or("", |s| s.default));
        // EMPREINTE → (tenant, sujet) du PREMIER déclarant. La clé est l'empreinte et non le jeton :
        // deux formes déclarées différemment (`token` ici, `token_sha256` là) peuvent désigner le
        // MÊME identifiant, et une comparaison de chaînes ne le verrait pas.
        let mut par_empreinte: BTreeMap<Empreinte, (&str, &str)> = BTreeMap::new();
        for s in &self.sessions {
            match (&s.token, &s.token_sha256) {
                (None, None) => refus.push(Refus::NiJetonNiEmpreinte { sujet: s.subject.clone() }),
                (Some(_), Some(_)) => {
                    refus.push(Refus::JetonEtEmpreinte { sujet: s.subject.clone() })
                }
                (Some(t), None) if t.is_empty() => {
                    refus.push(Refus::JetonVide { sujet: s.subject.clone() })
                }
                (None, Some(_)) if s.empreinte().is_none() => {
                    refus.push(Refus::EmpreinteInvalide { sujet: s.subject.clone() })
                }
                _ => {}
            }
            if let Some(e) = s.empreinte() {
                if e == demo {
                    refus.push(Refus::JetonDeDemonstration { sujet: s.subject.clone() });
                }
                if let Some((t0, sujet0)) = par_empreinte.insert(e, (&s.tenant, &s.subject)) {
                    refus.push(Refus::JetonPartage {
                        tenants: (t0.to_string(), s.tenant.clone()),
                        sujets: (sujet0.to_string(), s.subject.clone()),
                    });
                }
            }
            if let Some(t) = s.expire_le {
                if t <= maintenant {
                    refus.push(Refus::EcheanceDejaPassee {
                        sujet: s.subject.clone(),
                        expire_le: t,
                        maintenant,
                    });
                }
            }
            if role_de(&s.role).is_none() {
                refus.push(Refus::RoleInconnu { valeur: s.role.clone(), sujet: s.subject.clone() });
            }
            if !vus.contains_key(s.tenant.as_str()) {
                refus.push(Refus::SessionSansTenant {
                    tenant: s.tenant.clone(),
                    sujet: s.subject.clone(),
                });
            }
        }

        if refus.is_empty() {
            Ok(())
        } else {
            Err(refus)
        }
    }

    /// Les sessions d'un tenant donné — utilisé par le résumé, et par les tests d'isolation.
    #[must_use]
    pub fn sessions_de(&self, tenant: &str) -> Vec<&SessionDecl> {
        self.sessions.iter().filter(|s| s.tenant == tenant).collect()
    }
}

/// Applique une déclaration à une console : valide, provisionne, réhydrate, ouvre les sessions.
///
/// **Valide d'abord, et s'arrête là si quoi que ce soit cloche.** Ouvrir un port avec une isolation
/// qu'on n'a pas vérifiée serait la dégradation silencieuse la plus coûteuse de tout le dépôt :
/// tout aurait l'air de fonctionner.
///
/// La clé de chaque tenant est **dérivée** de `root` — c'est le modèle existant
/// (`TenantKey::derive`), et il tient à plusieurs tenants sans changement : un secret racine, N
/// clés, chacune liée à l'identifiant du tenant. Rien de ce que cette fonction rend ne contient de
/// clé ni de jeton.
pub fn appliquer(
    dep: &Deploiement,
    console: &mut Console,
    root: &[u8],
    maintenant: u64,
) -> Result<Resume, Vec<Refus>> {
    dep.valider(maintenant)?;

    let mut refus = Vec::new();
    let mut rehydrates = 0usize;

    for t in &dep.tenants {
        let cle = TenantKey::derive(root, &t.id);
        match &t.records {
            None => console.provision(&t.id, cle),
            Some(chemin) => match charger(chemin) {
                Ok(records) => {
                    console.rehydrate(&t.id, cle, records);
                    rehydrates += 1;
                }
                Err(cause) => refus.push(Refus::RecordsIllisible {
                    tenant: t.id.clone(),
                    chemin: chemin.clone(),
                    cause,
                }),
            },
        }
    }
    if !refus.is_empty() {
        return Err(refus);
    }

    let mut sessions_par_tenant: BTreeMap<String, usize> = BTreeMap::new();
    for t in &dep.tenants {
        sessions_par_tenant.insert(t.id.clone(), 0);
    }
    for s in &dep.sessions {
        let role = role_de(&s.role).expect("validé : le rôle appartient au vocabulaire");
        let e = s.empreinte().expect("validé : une forme de jeton exactement");
        // On pose l'EMPREINTE, jamais le jeton : même quand la déclaration en portait un en clair,
        // il ne survit pas à ce point.
        console.sessions_mut().inserer(e, &s.tenant, role, &s.subject, s.expire_le);
        *sessions_par_tenant.entry(s.tenant.clone()).or_insert(0) += 1;
    }

    let mut tenants: Vec<String> = dep.tenants.iter().map(|t| t.id.clone()).collect();
    tenants.sort();
    Ok(Resume { tenants, rehydrates, sessions_par_tenant })
}

/// Lit un log scellé. L'erreur est rendue en texte pour être **citée** dans le refus : « fichier
/// absent » et « JSONL corrompu » appellent des gestes différents, et un `Option` les confondrait.
fn charger(chemin: &str) -> Result<Vec<provenance_dag::Record>, String> {
    let jsonl = std::fs::read_to_string(chemin).map_err(|e| e.to_string())?;
    SealedDag::records_from_jsonl(&jsonl).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(json: &str) -> Deploiement {
        Deploiement::depuis_json(json).expect("JSON valide")
    }

    const SAIN: &str = r#"{
        "tenants": [{"id": "acme"}, {"id": "globex"}],
        "sessions": [
            {"token": "t-acme", "tenant": "acme", "role": "compliance_officer", "subject": "officer:hanna"},
            {"token": "t-globex", "tenant": "globex", "role": "auditor", "subject": "auditor:ext"}
        ]
    }"#;

    #[test]
    fn une_declaration_saine_passe() {
        let d = dep(SAIN);
        assert_eq!(d.valider(0), Ok(()));
        assert_eq!(d.tenants.len(), 2);
        assert_eq!(d.sessions_de("acme").len(), 1);
    }

    /// ⭐ **Le danger central de M7.** `Sessions::insert` est un `HashMap::insert` : sans ce refus,
    /// le porteur de « partagé » atteindrait le tenant retenu en dernier — l'isolation tomberait
    /// sur une faute de frappe, et rien ne le dirait.
    #[test]
    fn un_jeton_partage_entre_deux_tenants_est_refuse() {
        let d = dep(r#"{
            "tenants": [{"id": "acme"}, {"id": "globex"}],
            "sessions": [
                {"token": "partage", "tenant": "acme", "role": "auditor", "subject": "a"},
                {"token": "partage", "tenant": "globex", "role": "auditor", "subject": "b"}
            ]
        }"#);
        let e = d.valider(0).expect_err("doit refuser");
        assert!(
            e.iter().any(|r| matches!(r, Refus::JetonPartage { .. })),
            "le partage de jeton doit être nommé"
        );
        let msg = e.iter().map(|r| r.to_string()).collect::<String>();
        assert!(msg.contains("JETON PARTAGÉ ENTRE DEUX TENANTS"), "et crié : {msg}");
        assert!(!msg.contains("partage"), "mais le JETON lui-même ne doit jamais être imprimé");
    }

    #[test]
    fn un_jeton_partage_dans_un_meme_tenant_est_refuse_aussi() {
        // Moins spectaculaire, tout aussi faux : le SUJET retenu serait arbitraire — or c'est lui
        // qui est scellé dans les nœuds de revue. Un sujet faux est une fausse attestation.
        let d = dep(r#"{
            "tenants": [{"id": "acme"}],
            "sessions": [
                {"token": "x", "tenant": "acme", "role": "auditor", "subject": "a"},
                {"token": "x", "tenant": "acme", "role": "auditor", "subject": "b"}
            ]
        }"#);
        assert!(d.valider(0).is_err());
    }

    #[test]
    fn un_role_inconnu_nest_ni_retrograde_ni_promu() {
        let d = dep(r#"{
            "tenants": [{"id": "acme"}],
            "sessions": [{"token": "x", "tenant": "acme", "role": "ComplianceOfficer", "subject": "a"}]
        }"#);
        let e = d.valider(0).expect_err("la casse compte");
        assert!(e.iter().any(|r| matches!(r, Refus::RoleInconnu { .. })));
        assert!(e[0].to_string().contains("compliance_officer"), "le message donne le vocabulaire");
    }

    #[test]
    fn une_session_vers_un_tenant_non_provisionne_est_refusee() {
        let d = dep(r#"{
            "tenants": [{"id": "acme"}],
            "sessions": [{"token": "x", "tenant": "typo", "role": "auditor", "subject": "a"}]
        }"#);
        let e = d.valider(0).expect_err("doit refuser");
        assert!(e.iter().any(|r| matches!(r, Refus::SessionSansTenant { .. })));
    }

    #[test]
    fn le_jeton_de_demonstration_est_refuse_dans_une_declaration_explicite() {
        let demo = crate::profile::switch("EFE_CONSOLE_TOKEN").expect("déclaré en M0").default;
        let d = dep(&format!(
            r#"{{"tenants":[{{"id":"acme"}}],
                 "sessions":[{{"token":"{demo}","tenant":"acme","role":"auditor","subject":"a"}}]}}"#
        ));
        let e = d.valider(0).expect_err("doit refuser");
        assert!(e.iter().any(|r| matches!(r, Refus::JetonDeDemonstration { .. })));
    }

    #[test]
    fn un_service_sans_tenant_ou_sans_session_est_refuse() {
        assert!(dep(r#"{"tenants":[],"sessions":[]}"#).valider(0).is_err());
        let e = dep(r#"{"tenants":[{"id":"a"}],"sessions":[]}"#).valider(0).expect_err("");
        assert_eq!(e, vec![Refus::AucuneSession], "un port ouvert que personne ne peut lire");
    }

    /// Tous les refus d'un coup — un opérateur ne doit pas les découvrir un démarrage à la fois.
    #[test]
    fn la_validation_rend_tous_les_refus_pas_seulement_le_premier() {
        let d = dep(r#"{
            "tenants": [{"id": "acme"}, {"id": "acme"}],
            "sessions": [
                {"token": "", "tenant": "acme", "role": "nope", "subject": "a"},
                {"token": "y", "tenant": "absent", "role": "auditor", "subject": "b"}
            ]
        }"#);
        let e = d.valider(0).expect_err("doit refuser");
        assert!(e.len() >= 4, "quatre problèmes distincts, quatre refus : {e:?}");
        assert!(e.iter().any(|r| matches!(r, Refus::TenantDuplique { .. })));
        assert!(e.iter().any(|r| matches!(r, Refus::JetonVide { .. })));
        assert!(e.iter().any(|r| matches!(r, Refus::RoleInconnu { .. })));
        assert!(e.iter().any(|r| matches!(r, Refus::SessionSansTenant { .. })));
    }

    /// Une clé mal orthographiée dans une configuration d'ISOLATION doit échouer, pas être ignorée.
    #[test]
    fn une_cle_inconnue_fait_echouer_la_lecture() {
        let r = Deploiement::depuis_json(
            r#"{"tenants":[{"id":"a","recods":"/x"}],"sessions":[]}"#,
        );
        assert!(r.is_err(), "« recods » au lieu de « records » ne doit pas passer inaperçu");
    }

    #[test]
    fn le_vocabulaire_des_roles_couvre_les_quatre() {
        assert_eq!(ROLES.len(), 4, "les quatre rôles de S2");
        for (n, r) in ROLES {
            assert_eq!(role_de(n), Some(*r));
        }
        assert_eq!(role_de("Admin"), None, "sensible à la casse");
        assert_eq!(role_de(""), None);
    }
    // ── M8 : l'empreinte, l'échéance ──

    /// Une déclaration peut ne porter **aucun secret** : seule l'empreinte.
    #[test]
    fn une_declaration_peut_ne_contenir_aucun_secret() {
        let e: String = empreinte("le-vrai-jeton").iter().map(|b| format!("{b:02x}")).collect();
        let d = dep(&format!(
            r#"{{"tenants":[{{"id":"acme"}}],
                 "sessions":[{{"token_sha256":"{e}","tenant":"acme","role":"auditor","subject":"a"}}]}}"#
        ));
        assert_eq!(d.valider(0), Ok(()));
        let mut c = Console::new();
        appliquer(&d, &mut c, b"racine", 0).expect("saine");
        assert!(c.sessions().resolve("le-vrai-jeton", 0).is_some(), "l'empreinte suffit à ouvrir");
    }

    /// ⭐ Le piège que le durcissement crée, et que le même geste referme : deux sessions déclarées
    /// SOUS DES FORMES DIFFÉRENTES peuvent désigner le même identifiant. Une comparaison de chaînes
    /// ne le verrait pas ; la comparaison d'empreintes, si.
    #[test]
    fn un_jeton_partage_sous_deux_formes_differentes_est_quand_meme_vu() {
        let e: String = empreinte("partage").iter().map(|b| format!("{b:02x}")).collect();
        let d = dep(&format!(
            r#"{{"tenants":[{{"id":"acme"}},{{"id":"globex"}}],
                 "sessions":[
                   {{"token":"partage","tenant":"acme","role":"auditor","subject":"a"}},
                   {{"token_sha256":"{e}","tenant":"globex","role":"auditor","subject":"b"}}]}}"#
        ));
        let err = d.valider(0).expect_err("doit refuser");
        assert!(
            err.iter().any(|r| matches!(r, Refus::JetonPartage { .. })),
            "un jeton en clair et son empreinte sont LE MÊME identifiant : {err:?}"
        );
    }

    #[test]
    fn ni_jeton_ni_empreinte_ou_les_deux_sont_refuses() {
        let sans = dep(r#"{"tenants":[{"id":"a"}],"sessions":[{"tenant":"a","role":"auditor","subject":"s"}]}"#);
        assert!(sans.valider(0).expect_err("").iter().any(|r| matches!(r, Refus::NiJetonNiEmpreinte { .. })));

        let e: String = empreinte("x").iter().map(|b| format!("{b:02x}")).collect();
        let deux = dep(&format!(
            r#"{{"tenants":[{{"id":"a"}}],"sessions":[{{"token":"x","token_sha256":"{e}","tenant":"a","role":"auditor","subject":"s"}}]}}"#
        ));
        assert!(deux.valider(0).expect_err("").iter().any(|r| matches!(r, Refus::JetonEtEmpreinte { .. })),
                "même quand les deux CONCORDENT : deviner lequel fait foi n'est pas une option");
    }

    #[test]
    fn une_empreinte_malformee_est_refusee_et_non_ignoree() {
        let d = dep(r#"{"tenants":[{"id":"a"}],"sessions":[{"token_sha256":"pas-du-hex","tenant":"a","role":"auditor","subject":"s"}]}"#);
        assert!(d.valider(0).expect_err("").iter().any(|r| matches!(r, Refus::EmpreinteInvalide { .. })));
    }

    /// Une session qui naît morte : l'exploitant croirait son jeton valide et chercherait ailleurs.
    #[test]
    fn une_echeance_deja_passee_arrete_le_demarrage() {
        let d = dep(r#"{"tenants":[{"id":"a"}],"sessions":[{"token":"t","tenant":"a","role":"auditor","subject":"s","expire_le":100}]}"#);
        assert_eq!(d.valider(50), Ok(()), "à venir : c'est bon");
        let e = d.valider(500).expect_err("déjà passée");
        assert!(e.iter().any(|r| matches!(r, Refus::EcheanceDejaPassee { .. })));
        assert!(e[0].to_string().contains("naîtrait morte"));
    }

    #[test]
    fn lecheance_declaree_est_appliquee_a_la_session() {
        let d = dep(r#"{"tenants":[{"id":"a"}],"sessions":[{"token":"t","tenant":"a","role":"auditor","subject":"s","expire_le":100}]}"#);
        let mut c = Console::new();
        appliquer(&d, &mut c, b"racine", 0).expect("saine");
        assert!(c.sessions().resolve("t", 99).is_some());
        assert!(c.sessions().resolve("t", 100).is_none(), "échue");
        assert_eq!(c.sessions().sans_expiration(), 0);
    }

    /// Le jeton en clair d'une déclaration **ne survit pas** au provisionnement.
    #[test]
    fn le_jeton_en_clair_dune_declaration_ne_survit_pas() {
        let mut c = Console::new();
        appliquer(&dep(SAIN), &mut c, b"racine", 0).expect("saine");
        // On ne peut pas fouiller Sessions de l'extérieur — mais on peut vérifier le contrat :
        // le jeton résout, et rien dans le résumé ne le contient (cf. `un_resume_ne_contient_jamais_de_jeton`).
        assert!(c.sessions().resolve("t-acme", 0).is_some());
        assert_eq!(c.sessions().len(), 2);
    }

    // ── l'application ──

    #[test]
    fn appliquer_monte_deux_tenants_et_leurs_sessions() {
        let mut c = Console::new();
        let r = appliquer(&dep(SAIN), &mut c, b"racine-de-test", 0).expect("déclaration saine");
        assert_eq!(r.tenants, vec!["acme".to_string(), "globex".to_string()]);
        assert_eq!(r.rehydrates, 0, "aucun log déclaré ⟹ tenants vides, alimentés par la capture");
        assert_eq!(r.sessions(), 2);
        assert_eq!(r.sessions_par_tenant["acme"], 1);
        assert_eq!(r.sessions_par_tenant["globex"], 1);
        // les deux jetons résolvent, chacun vers SON tenant
        let a = c.sessions().resolve("t-acme", 0).expect("session acme");
        let g = c.sessions().resolve("t-globex", 0).expect("session globex");
        assert_eq!(a.tenant.as_str(), "acme");
        assert_eq!(g.tenant.as_str(), "globex");
        assert_ne!(a.tenant.as_str(), g.tenant.as_str(), "deux tenants VIVANTS dans un service");
    }

    #[test]
    fn un_resume_ne_contient_jamais_de_jeton() {
        let mut c = Console::new();
        let r = appliquer(&dep(SAIN), &mut c, b"racine", 0).expect("saine");
        let texte = format!("{r:?}");
        assert!(!texte.contains("t-acme"), "un identifiant dans un journal est un identifiant divulgué");
        assert!(!texte.contains("t-globex"));
    }

    /// Un tenant sans session est **compté à zéro**, pas absent du résumé : sinon l'exploitant ne
    /// verrait pas qu'il a monté un tenant que personne ne peut lire.
    #[test]
    fn un_tenant_sans_session_apparait_a_zero() {
        let mut c = Console::new();
        let d = dep(r#"{
            "tenants": [{"id": "acme"}, {"id": "muet"}],
            "sessions": [{"token": "t", "tenant": "acme", "role": "auditor", "subject": "a"}]
        }"#);
        let r = appliquer(&d, &mut c, b"racine", 0).expect("saine");
        assert_eq!(r.sessions_par_tenant["muet"], 0, "monté, et illisible — il faut le voir");
    }

    /// Un log déclaré mais illisible est FATAL. Démarrer vide ferait conclure à un auditeur qu'il
    /// ne s'est rien passé : faux, et rassurant.
    #[test]
    fn un_log_declare_mais_illisible_arrete_le_demarrage() {
        let mut c = Console::new();
        let d = dep(r#"{
            "tenants": [{"id": "acme", "records": "/chemin/qui/nexiste/pas.jsonl"}],
            "sessions": [{"token": "t", "tenant": "acme", "role": "auditor", "subject": "a"}]
        }"#);
        let e = appliquer(&d, &mut c, b"racine", 0).expect_err("doit refuser");
        assert!(e.iter().any(|r| matches!(r, Refus::RecordsIllisible { .. })));
        assert!(e[0].to_string().contains("faux, et rassurant"));
    }

    #[test]
    fn une_declaration_invalide_ne_monte_rien_du_tout() {
        // Le point : la validation précède TOUTE mutation. Une console à moitié montée servirait
        // un sous-ensemble arbitraire, ce qui est pire qu'un démarrage refusé.
        let mut c = Console::new();
        let d = dep(r#"{
            "tenants": [{"id": "acme"}, {"id": "globex"}],
            "sessions": [
                {"token": "p", "tenant": "acme", "role": "auditor", "subject": "a"},
                {"token": "p", "tenant": "globex", "role": "auditor", "subject": "b"}
            ]
        }"#);
        assert!(appliquer(&d, &mut c, b"racine", 0).is_err());
        assert!(c.sessions().is_empty(), "aucune session ouverte");
        assert!(c.key("acme").is_none(), "et aucun tenant provisionné");
    }
}
