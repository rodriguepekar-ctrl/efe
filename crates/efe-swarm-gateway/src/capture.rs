// SPDX-License-Identifier: UNLICENSED
//! # `capture` — M2 : la capture ne peut plus renverser la réponse du client.
//!
//! ## Le défaut, en un caractère
//!
//! Le chemin synchrone finissait par `s.store.put(...).await?`. Le run a réussi, la réponse est
//! là — dans `outcome.run`, complète, scellée, avec sa racine. Et le `?` la jette, parce que la
//! **persistance** a échoué. Le client perd une réponse qui existe, pour une raison qui ne le
//! concerne pas.
//!
//! C'est l'objection qui tue l'adoption d'une passerelle de provenance, et elle est légitime :
//! *« si je fais passer mon trafic par votre passerelle et que votre store hoquette, mon
//! application tombe »*. Aucune équipe ne prend cet engagement d'architecture pour une fonction
//! de conformité.
//!
//! ## Ce n'est pas un renoncement au *fail-closed* — la preuve est dans le dépôt
//!
//! Ce dépôt refuse partout : `exit 78` au démarrage, egress fermé par défaut, registre vide plutôt
//! qu'ouvert. Introduire un chemin qui *ne refuse pas* demande donc une justification, pas une
//! préférence.
//!
//! Elle est écrite dans le code lui-même. Le mode JOB (`?stream=true`) traitait **déjà** l'échec de
//! persistance sans faire tomber le client : il l'émet sur le bus et le client garde sa réponse.
//! Son commentaire énonce la règle de l'auteur — *« jamais d'échec de persistance avalé, le moat
//! n'est pas négociable »*. La règle est **surfacer**, pas **faire tomber**. Les deux modes ne
//! divergeaient donc pas par décision : le mode JOB rend `202` avant la fin du run, il ne *peut*
//! pas échouer sur la persistance ; le mode SYNC le peut, et le faisait. L'asymétrie était un
//! accident de forme, et ce module en fait un choix.
//!
//! ## Ce qui reste non négociable
//!
//! La souveraineté. Un endpoint hors périmètre reste refusé, le garde de démarrage reste
//! fail-closed, l'allowlist d'egress reste vide par défaut. Les deux familles de panne n'ont pas
//! le même coût, et c'est tout l'argument :
//!
//! * ouvrir sur l'**egress**, c'est laisser sortir des données — dommage irréversible ;
//! * fermer sur la **capture**, c'est casser la production d'un client parce que le sous-système
//!   d'audit a hoqueté. Et l'audit n'y gagne rien : un auditeur préfère de loin *« le run 47 a eu
//!   lieu et n'a pas pu être persisté, voici pourquoi »* à *« le run 47 n'a jamais eu lieu parce
//!   que la persistance a échoué »*. Le second efface un fait ; le premier le documente.
//!
//! ## Ce qui échoue exactement, et pourquoi le mot compte
//!
//! `store.put` ne **scelle** pas : il **persiste** un `RunOutcome` déjà scellé. Une panne y laisse
//! donc un run parfaitement scellé, dont le client tient la racine — mais que `GET /verify/:id` ne
//! retrouvera **jamais**. La dégradation n'est pas « non scellée », elle est « non persistée », et
//! sa conséquence est précise : *plus personne ne pourra re-prouver ce run*. Le rapport de capture
//! le dit dans ces termes, parce qu'un client qui lit « dégradé » sans savoir quoi ne fera rien.
//!
//! ## Quatre endroits, parce qu'une brèche que personne ne voit est pire que pas de chaîne
//!
//! Le silence serait le vrai danger : une chaîne trouée que l'on croit complète vaut moins que pas
//! de chaîne du tout, puisque sa valeur *est* d'être complète. Toute brèche se dit donc :
//!
//! 1. **au client**, dans la réponse — il tient une racine que personne ne pourra recouper ;
//! 2. **à l'exploitant**, dans `/metrics` (deux séries : le décompte *et* la posture, sans quoi un
//!    zéro se lirait comme une bonne nouvelle alors qu'il peut être nul par construction) ;
//! 3. **à l'auditeur qui liste**, dans `GET /v1/swarm/gaps` ;
//! 4. **à quiconque redemande ce run**, par [`absence_expliquee`] : les huit routes qui lisent un
//!    run rendent `410` et non `404`. Ce quatrième canal n'était pas dans la première rédaction de
//!    ce module — l'audit de M2 a trouvé que `verify` avait été corrigé et que **sept autres
//!    routes** rendaient encore un `404` muet, c'est-à-dire affirmaient qu'un run servi n'avait
//!    jamais existé.
//!
//! ## Les deux chemins qui persistent passent par la même couture
//!
//! Le chemin d'**essaim** (`POST /v1/swarm/run`) et le chemin de **mission frontière**
//! (`/v1/llm/*`) persistent tous deux, et tous deux appellent [`decider`]. Le second est le plus
//! coûteux à perdre : les appels aux modèles ont déjà été **facturés** quand la persistance
//! intervient. La première rédaction de M2 ne traitait que le premier — et n'aurait de toute façon
//! rien changé au second, dont l'appelant **relisait le store** juste après pour bâtir sa réponse :
//! le client serait tombé dix lignes plus loin. Les deux défauts se corrigent ensemble ou pas du
//! tout, ce que seule une lecture du chemin complet fait apparaître.
//!
//! ## Ce que M2 ne fait délibérément pas
//!
//! **La capture reste synchrone.** La rendre concurrente sortirait aussi sa *latence* du chemin du
//! client, mais introduirait une course que le client ne peut pas distinguer : `verify/:id` appelé
//! juste après la réponse rendrait 404 pour un run qui *va* être persisté — indiscernable d'une
//! vraie brèche. La revendication du coin porte sur la **panne**, pas sur la latence. On tient la
//! même promesse sans inventer une ambiguïté.
//!
//! **Le défaut ne change pas** — mais disons exactement en quel sens. `Bloquante` reste la
//! politique par défaut, et sous elle **les codes de statut et le comportement sont ceux
//! d'avant M2** : un échec de persistance fait toujours échouer la requête.
//!
//! Ce n'est pas pour autant « byte-identique », et l'affirmer serait la sur-vente que ce dépôt
//! traque partout ailleurs : la réponse a gagné un champ `capture`, dans **toutes** les postures,
//! y compris la posture par défaut. C'est additif — un client JSON qui ignore les champs inconnus
//! ne voit rien — mais l'octet a changé, et le dire compte.
//!
//! Ce qui reste vrai sans réserve : changer en silence le **code de statut** d'un chemin servi est
//! ce que M0 existe pour interdire ; la posture d'observation est donc un **opt-in déclaré**,
//! gouverné par un interrupteur du registre, et annoncé au démarrage.
//!
//! ## Le vocabulaire, pour qui cherche par les noms usuels
//!
//! Ce module implémente ce que l'industrie appelle **`fail_open`** sur le chemin de capture, sous
//! une posture **`observe_only`** : la passerelle observe et enregistre, elle ne refuse jamais
//! *pour une raison de provenance*. Les noms de code sont français (`Observation`, `Bloquante`)
//! comme le reste du dépôt ; ces trois mots sont écrits ici pour qu'une recherche par le terme
//! usuel — celui de la feuille de route — tombe sur le module qui les réalise.
//!
//! Le troisième terme de cette liste, **`shadow`** (capture concurrente, hors du chemin de la
//! réponse), est délibérément **non implémenté** : voir la section suivante.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::GatewayError;

/// Nombre de brèches conservées en mémoire. Au-delà, la plus ancienne sort — mais le **compteur**,
/// lui, ne se perd pas : le journal est un échantillon récent, `/metrics` est le décompte total.
/// Confondre les deux ferait croire à une panne bornée alors qu'elle ne l'est pas.
pub const JOURNAL_CAP: usize = 256;

/// Ce que la passerelle fait quand la persistance d'un run échoue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CapturePolicy {
    /// **Défaut, et comportement d'avant M2.** L'échec de persistance devient l'échec de la
    /// requête : le client reçoit une erreur, et sa réponse — pourtant calculée — est perdue.
    ///
    /// Ce n'est pas absurde : c'est la position d'un déploiement où *aucun* run non re-prouvable
    /// n'est acceptable, et où l'on préfère qu'une requête échoue plutôt qu'elle réussisse sans
    /// trace. Un tel déploiement existe. Il n'est simplement pas celui d'un client qui découvre
    /// le produit.
    #[default]
    Bloquante,
    /// **La posture d'observation.** La passerelle observe et enregistre ; elle ne refuse jamais
    /// *pour une raison de provenance*. L'échec de persistance devient une **dégradation dite** :
    /// le client garde sa réponse, et la brèche est visible aux trois endroits.
    Observation,
}

impl CapturePolicy {
    /// La valeur d'environnement, réduite à la politique. Toute autre chaîne ⟹ défaut, et le
    /// registre M0 signale la valeur refusée : on ne devine jamais une intention.
    #[must_use]
    pub fn from_str_or_default(v: Option<&str>) -> Self {
        match v.map(str::trim) {
            Some("observation") => Self::Observation,
            _ => Self::Bloquante,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bloquante => "blocking",
            Self::Observation => "observation",
        }
    }

    /// `true` si cette politique peut produire des brèches — donc si le journal a un sens.
    #[must_use]
    pub fn peut_degrader(self) -> bool {
        matches!(self, Self::Observation)
    }
}

/// Ce que la capture a fait, **dit au client dans sa réponse**.
///
/// Toujours présent, y compris dans le cas nominal : un champ qui n'apparaît qu'en cas de problème
/// entraîne des clients qui ne le lisent jamais, et donc une dégradation que personne ne remarque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureReport {
    /// Le run est-il persisté, donc re-prouvable par `GET /v1/swarm/verify/:id` ?
    pub persisted: bool,
    /// La politique en vigueur — pour que le client sache s'il *peut* recevoir une réponse dégradée.
    pub policy: &'static str,
    /// Pourquoi la persistance a échoué. `None` dans le cas nominal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degrade_reason: Option<String>,
    /// Ce que la dégradation coûte, en clair. `None` dans le cas nominal.
    ///
    /// Séparé de `degrade_reason` à dessein : la raison est technique (« connexion refusée »), la
    /// conséquence est ce qui décide de l'action du client (« ce run ne sera jamais re-prouvable »).
    /// Un client qui lit la première sans la seconde ne saura pas s'il doit rejouer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consequence: Option<&'static str>,
}

impl CaptureReport {
    #[must_use]
    pub fn persistee(policy: CapturePolicy) -> Self {
        Self { persisted: true, policy: policy.as_str(), degrade_reason: None, consequence: None }
    }

    #[must_use]
    pub fn degradee(policy: CapturePolicy, raison: impl Into<String>) -> Self {
        Self {
            persisted: false,
            policy: policy.as_str(),
            degrade_reason: Some(raison.into()),
            consequence: Some(
                "le run a bien eu lieu et sa racine est dans cette réponse, mais il n'est PAS \
                 persisté : `GET /v1/swarm/verify/:id` ne le retrouvera jamais. Conserver cette \
                 racine hors bande si la re-preuve importe.",
            ),
        }
    }
}

/// Une **brèche** : un run qui a eu lieu et qui n'a pas pu être persisté.
#[derive(Debug, Clone, Serialize)]
pub struct Gap {
    pub run_id: Uuid,
    /// Le tenant, pour que le journal servi reste cloisonné. Jamais rendu à un autre tenant.
    #[serde(skip_serializing)]
    pub tenant: String,
    /// Millisecondes depuis l'époque, au moment de l'échec.
    pub at_ms: u64,
    pub reason: String,
}

/// Le journal borné des brèches, et leur décompte total.
///
/// Deux grandeurs distinctes, volontairement : `total` ne se perd jamais, `recentes` est un
/// échantillon. Servir uniquement l'échantillon laisserait croire à une panne bornée ; servir
/// uniquement le total priverait l'auditeur des identifiants qu'il doit aller chercher.
#[derive(Debug, Default)]
pub struct GapJournal {
    recentes: Mutex<VecDeque<Gap>>,
    total: AtomicU64,
}

impl GapJournal {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enregistre une brèche. Le verrou n'est jamais tenu à travers un `.await`.
    pub fn record(&self, gap: Gap) {
        self.total.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut q) = self.recentes.lock() {
            if q.len() == JOURNAL_CAP {
                q.pop_front();
            }
            q.push_back(gap);
        }
    }

    /// Le décompte total, qui ne se perd pas quand le journal tourne.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Une brèche précise, si elle est encore dans l'échantillon. Cloisonnée par tenant.
    ///
    /// `None` ne prouve PAS qu'il n'y a pas eu de brèche : le journal est borné, et une brèche
    /// ancienne en est sortie. C'est pourquoi la route qui l'utilise ne dit jamais « ce run n'a
    /// pas eu de brèche » — elle dit seulement ce qu'elle sait.
    #[must_use]
    pub fn trouve(&self, run_id: Uuid, tenant: &str) -> Option<Gap> {
        self.recentes
            .lock()
            .ok()?
            .iter()
            .find(|g| g.run_id == run_id && g.tenant == tenant)
            .cloned()
    }

    /// Les brèches récentes **d'un tenant**. Le cloisonnement est ici, pas dans la route : un
    /// filtre oublié dans un appelant serait une fuite inter-tenant.
    #[must_use]
    pub fn recentes_du_tenant(&self, tenant: &str) -> Vec<Gap> {
        self.recentes
            .lock()
            .map(|q| q.iter().filter(|g| g.tenant == tenant).cloned().collect())
            .unwrap_or_default()
    }
}

/// L'horloge murale en millisecondes, ou `0` si l'horloge est antérieure à l'époque.
fn maintenant_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// **La couture de M2.** Décide, à partir d'un échec de persistance, si la requête tombe.
///
/// Fonction **pure quant à la décision** : elle ne persiste rien elle-même, elle reçoit le résultat
/// de la persistance. Cela la rend testable sans store, et surtout : le point de décision est
/// UNIQUE. Un appelant ne peut pas décider autrement dans son coin — c'est la même discipline que
/// le registre de M0 pour l'environnement, ou l'analyseur unique de M1 pour les hôtes.
///
/// # Errors
/// Sous [`CapturePolicy::Bloquante`], propage l'erreur de persistance telle quelle — le
/// comportement d'avant M2, à l'octet près.
pub fn decider(
    policy: CapturePolicy,
    run_id: Uuid,
    tenant: &str,
    resultat: Result<(), GatewayError>,
    journal: &Arc<GapJournal>,
) -> Result<CaptureReport, GatewayError> {
    match resultat {
        Ok(()) => Ok(CaptureReport::persistee(policy)),
        Err(e) => match policy {
            // Le comportement historique, inchangé : l'échec de persistance est l'échec de la
            // requête. Aucune brèche n'est journalisée, parce qu'aucun run n'a été servi sans trace.
            CapturePolicy::Bloquante => Err(e),
            CapturePolicy::Observation => {
                let raison = e.to_string();
                journal.record(Gap {
                    run_id,
                    tenant: tenant.to_string(),
                    at_ms: maintenant_ms(),
                    reason: raison.clone(),
                });
                Ok(CaptureReport::degradee(policy, raison))
            }
        },
    }
}

/// **Pourquoi ce run n'est-il pas là ?** — le point de décision UNIQUE de l'absence.
///
/// Une route qui répond `404` à un run manquant affirme *« cet identifiant n'existe pas »*. Si le
/// run a eu lieu et n'a pas pu être persisté, c'est faux : cela **efface un fait** au lieu de le
/// documenter — exactement ce que la chaîne de provenance existe pour empêcher.
///
/// Cette fonction est séparée de la route pour la même raison que [`decider`] : sept routes lisent
/// un run persisté, et sept décisions indépendantes finiraient par diverger. Le premier audit de M2
/// l'a prouvé — `verify` avait été corrigé, les six autres avaient gardé leur `404` muet.
///
/// `None` du journal **ne prouve pas** l'absence de brèche : l'échantillon est borné et ne survit
/// pas au redémarrage. On ne dit donc jamais « ce run n'a jamais eu lieu » — seulement
/// « introuvable », qui est tout ce que l'on sait.
#[must_use]
pub fn absence_expliquee(journal: &GapJournal, id: Uuid, tenant: &str) -> GatewayError {
    match journal.trouve(id, tenant) {
        Some(g) => GatewayError::Gone(format!(
            "ce run a bien été SERVI puis n'a PAS pu être persisté : il ne sera jamais \
             re-prouvable. Raison enregistrée : {}",
            g.reason
        )),
        None => GatewayError::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panne() -> Result<(), GatewayError> {
        Err(GatewayError::Internal("connexion au store refusée".into()))
    }

    #[test]
    fn la_politique_par_defaut_est_celle_davant_m2() {
        assert_eq!(CapturePolicy::default(), CapturePolicy::Bloquante);
        assert_eq!(CapturePolicy::from_str_or_default(None), CapturePolicy::Bloquante);
        // Une valeur inconnue ne devine RIEN : elle retombe sur le défaut sûr. Le registre M0
        // signale par ailleurs la valeur refusée, pour que l'opérateur sache que son geste a raté.
        assert_eq!(CapturePolicy::from_str_or_default(Some("oui")), CapturePolicy::Bloquante);
        assert_eq!(CapturePolicy::from_str_or_default(Some("OBSERVATION")), CapturePolicy::Bloquante);
        assert_eq!(CapturePolicy::from_str_or_default(Some("observation")), CapturePolicy::Observation);
    }

    #[test]
    fn en_mode_bloquant_lechec_reste_lechec() {
        let j = Arc::new(GapJournal::new());
        let r = decider(CapturePolicy::Bloquante, Uuid::nil(), "acme", panne(), &j);
        assert!(r.is_err(), "sous la posture par défaut, l'échec reste l'échec — comme avant M2");
        assert_eq!(j.total(), 0, "aucun run n'a été servi sans trace : pas de brèche à journaliser");
    }

    #[test]
    fn en_observation_le_client_garde_sa_reponse_et_la_breche_est_dite() {
        let j = Arc::new(GapJournal::new());
        let id = Uuid::from_u128(7);
        let r = decider(CapturePolicy::Observation, id, "acme", panne(), &j)
            .expect("l'observation ne renverse jamais la réponse");
        assert!(!r.persisted);
        assert_eq!(r.policy, "observation");
        assert!(r.degrade_reason.as_deref().is_some_and(|s| s.contains("store")), "la RAISON est dite");
        assert!(
            r.consequence.is_some_and(|c| c.contains("ne le retrouvera jamais")),
            "et la CONSÉQUENCE aussi — sans elle, le client ne sait pas s'il doit agir"
        );
        assert_eq!(j.total(), 1);
        assert_eq!(j.recentes_du_tenant("acme").len(), 1);
        assert_eq!(j.recentes_du_tenant("acme")[0].run_id, id);
    }

    #[test]
    fn le_journal_est_cloisonne_par_tenant() {
        let j = Arc::new(GapJournal::new());
        decider(CapturePolicy::Observation, Uuid::from_u128(1), "acme", panne(), &j).unwrap();
        decider(CapturePolicy::Observation, Uuid::from_u128(2), "globex", panne(), &j).unwrap();
        assert_eq!(j.recentes_du_tenant("acme").len(), 1, "un tenant ne voit QUE ses brèches");
        assert_eq!(j.recentes_du_tenant("globex").len(), 1);
        assert_eq!(j.recentes_du_tenant("inconnu").len(), 0);
        assert_eq!(j.total(), 2, "le total, lui, compte tout");
    }

    /// Le journal tourne, le compteur non. Servir l'échantillon comme s'il était le total ferait
    /// croire à une panne bornée alors qu'elle ne l'est pas.
    #[test]
    fn le_journal_borne_ne_borne_pas_le_decompte() {
        let j = Arc::new(GapJournal::new());
        for i in 0..(JOURNAL_CAP as u128 + 20) {
            decider(CapturePolicy::Observation, Uuid::from_u128(i), "acme", panne(), &j).unwrap();
        }
        assert_eq!(j.recentes_du_tenant("acme").len(), JOURNAL_CAP, "l'échantillon est borné");
        assert_eq!(j.total(), JOURNAL_CAP as u64 + 20, "le décompte ne se perd pas");
    }

    #[test]
    fn une_absence_ne_se_confond_pas_avec_une_breche() {
        let j = Arc::new(GapJournal::new());
        let vu = Uuid::from_u128(3);
        decider(CapturePolicy::Observation, vu, "acme", panne(), &j).unwrap();

        // Le run a eu lieu : 410, avec la raison ET la conséquence.
        match absence_expliquee(&j, vu, "acme") {
            GatewayError::Gone(m) => {
                assert!(m.contains("SERVI"), "le fait doit être affirmé, pas effacé");
                assert!(m.contains("jamais re-prouvable"), "et sa conséquence dite");
            }
            autre => panic!("attendu Gone, obtenu {autre:?}"),
        }
        // Un identifiant inconnu reste un 404 : le journal borné ne permet pas d'affirmer plus.
        assert!(matches!(absence_expliquee(&j, Uuid::from_u128(99), "acme"), GatewayError::NotFound));
        // Et le cloisonnement tient jusque dans l'explication : un autre tenant voit un 404.
        assert!(matches!(absence_expliquee(&j, vu, "globex"), GatewayError::NotFound));
    }

    #[test]
    fn le_cas_nominal_dit_aussi_quelque_chose() {
        let j = Arc::new(GapJournal::new());
        let r = decider(CapturePolicy::Observation, Uuid::nil(), "acme", Ok(()), &j).unwrap();
        assert!(r.persisted);
        assert!(r.degrade_reason.is_none());
        // Le rapport est présent MÊME quand tout va bien : un champ qui n'apparaît qu'en cas de
        // problème produit des clients qui ne le lisent jamais.
        assert_eq!(r.policy, "observation");
        assert_eq!(j.total(), 0);
    }
}
