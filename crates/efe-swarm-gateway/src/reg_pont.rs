// SPDX-License-Identifier: UNLICENSED
//! # `reg_pont` — M5 : la jonction entre le coin et l'expansion.
//!
//! ## Ce que M5 relie, et pourquoi ce n'était pas relié
//!
//! Ce dépôt porte **deux produits séparables**. Le *coin* — la passerelle souveraine qui sert un
//! reçu — et l'*expansion* — le registre d'évidence réglementaire (`reg-evidence`), qui mappe une
//! chaîne de provenance sur les exigences de l'EU AI Act, du GDPR, de la CSDDD et de la LkSG.
//!
//! Les deux parlaient déjà la même langue sans que personne ne les fasse se parler :
//!
//! * `provenance-bridge::bundle_from_run` traduit un run d'essaim en nœuds de DAG scellés, et
//!   dérive au passage la **clé de pile B** ;
//! * `provenance_dag::SealedDag::records_from_jsonl` relit ces nœuds ;
//! * `reg_evidence::derive` les **re-prouve avec cette même clé**, puis en dérive l'artefact.
//!
//! Les trois pièces s'emboîtent exactement. M5 n'est donc pas une traduction à écrire : c'est le
//! dernier maillon à connecter — et la question de savoir **comment joindre sans souder**.
//!
//! ## Joindre n'est pas souder
//!
//! L'analyse « produit unique » avait explicitement conclu de **ne pas fusionner** les deux : le
//! coin exige de l'acheteur qu'il route son trafic par la passerelle — un engagement
//! d'architecture ; l'expansion ne demande que des points d'observation. Ce sont deux ventes, deux
//! rythmes d'adoption, deux acheteurs.
//!
//! La feature `reg-evidence` rend cette séparation **mécanique** plutôt que déclarative : sans
//! elle, le binaire du coin ne contient pas une ligne du registre, et
//! `tests/deux_produits_separables.rs` le vérifie sur la fermeture réelle. Avec elle, le coût est
//! d'**un** crate — `provenance-dag` est déjà là, et `reg-evidence` n'en dépend que.
//!
//! ## Pourquoi cette dérivation ne peut JAMAIS faire tomber le client
//!
//! M2 avait posé la règle : dégrader plutôt que couper, **quand dégrader produit quelque chose
//! d'utile et d'honnête**. Ici la réponse est plus forte encore que pour la persistance, et pour
//! une raison qu'il faut nommer :
//!
//! > un dossier manquant est **récupérable**, une persistance manquée ne l'est pas.
//!
//! Si le store échoue, le run est perdu pour toujours — d'où le journal des brèches. Si la
//! dérivation échoue, le run est **intact dans le store** : le dossier se re-dérive plus tard, par
//! `POST /v1/swarm/reg/:id`, à l'identique. Faire tomber la requête d'un client pour un artefact
//! qu'on saura reconstituer serait donc doublement injustifié.
//!
//! C'est pourquoi ce module n'a **aucun chemin d'erreur** : [`deriver`] rend toujours un rapport.
//! La politique de capture de M2 ne s'applique pas ici — elle gouverne l'irrécupérable.
//!
//! ## Ce que l'artefact est, et ce qu'il n'est pas
//!
//! `reg-evidence` interdit le glissement vers le verdict **par construction** : sa sémantique est
//! une énumération à **une seule valeur** (`EVIDENCE_MAPPING_ONLY`), et son artefact n'a aucun
//! champ « conforme » nulle part. Il ne prononce pas la conformité — il **mappe de l'évidence sur
//! des exigences**, et publie le compte de ce qui est couvert, partiel, ou hors périmètre.
//!
//! Ce rapport-ci reprend ces trois nombres tels quels. Un dossier vide se voit alors dans
//! `covered: 0` au lieu de passer pour une bonne nouvelle — ce qui serait le « vert malhonnête »
//! que toute cette série combat.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::store::StoredRun;

/// Le cadre réglementaire visé.
///
/// Réexporté sous un nom local pour que le reste de la passerelle n'ait pas à importer
/// `reg_evidence` — le jour où l'on voudra un second fournisseur d'artefacts, le point de contact
/// est ici et nulle part ailleurs.
#[cfg(feature = "reg-evidence")]
pub use reg_evidence::Framework;

/// Les valeurs acceptées par `EFE_SWARM_REG_FRAMEWORK`, déclarées une seule fois.
///
/// Cette liste est celle que le registre M0 publie dans `ENUMS` : une valeur hors énumération y est
/// **signalée** au démarrage au lieu de retomber en silence sur un défaut, ce qui ferait dériver un
/// dossier au titre d'un cadre que l'opérateur n'a pas choisi.
pub const CADRES: &[&str] = &["eu_ai_act", "gdpr", "csddd", "lksg"];

/// Le cadre, depuis la valeur d'environnement. Toute autre chaîne ⟹ `None` : on ne devine JAMAIS
/// un cadre réglementaire, parce que se tromper de cadre produit un dossier qui a l'air correct et
/// qui répond à la mauvaise question.
#[cfg(feature = "reg-evidence")]
#[must_use]
pub fn cadre_de(v: Option<&str>) -> Option<Framework> {
    match v.map(str::trim) {
        Some("eu_ai_act") => Some(Framework::EuAiAct),
        Some("gdpr") => Some(Framework::Gdpr),
        Some("csddd") => Some(Framework::Csddd),
        Some("lksg") => Some(Framework::Lksg),
        _ => None,
    }
}

/// Le nom court d'un cadre — celui qui apparaît dans le rapport et dans le chemin du dossier.
#[cfg(feature = "reg-evidence")]
#[must_use]
pub fn slug(f: Framework) -> &'static str {
    match f {
        Framework::EuAiAct => "eu_ai_act",
        Framework::Gdpr => "gdpr",
        Framework::Csddd => "csddd",
        Framework::Lksg => "lksg",
    }
}

/// La lecture versionnée à appliquer — le *mapping* exigence ↔ nature de nœud.
///
/// Elle voyage AVEC le dossier (`reg/mapping.<version>.json`) : c'est ce qui permet à un tiers de
/// re-dériver l'artefact **sans nous**, et donc de ne rien avoir à croire.
#[cfg(feature = "reg-evidence")]
fn mapping_pour(f: Framework) -> reg_evidence::MappingSet {
    // La lecture **courante**, décidée en un seul endroit (`reg_evidence::templates::courant`) — et non
    // recopiée ici. Les versions ne marchent plus au pas depuis que le GDPR est passé en `2026-08`
    // (l'art. 5(1)(e), que M6 outille) : un producteur qui choisirait son gabarit à la main resterait en
    // arrière à la montée suivante, et servirait un dossier neuf sous une lecture périmée.
    reg_evidence::templates::courant(f)
}

/// Le pont, tel que l'état le porte. `None` sur l'état ⟹ le pont n'est pas câblé.
#[cfg(feature = "reg-evidence")]
#[derive(Debug, Clone)]
pub struct RegPont {
    /// Où les dossiers sont écrits. Un sous-dossier par run.
    pub dir: PathBuf,
    pub framework: Framework,
}

/// Ce que la dérivation a fait de ce run — **dit au client dans sa réponse**.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegReport {
    /// Le dossier a-t-il été écrit ?
    pub produced: bool,
    /// Le cadre visé — pour qu'un lecteur sache à quelle question le dossier répond.
    ///
    /// Chaîne possédée, et non `&'static str` issu de `reg_evidence` : ce TYPE est compilé dans
    /// tous les builds, y compris ceux qui n'embarquent pas le registre. Sans quoi la forme de la
    /// réponse servie dépendrait de la feature — c'est-à-dire qu'un client verrait un champ
    /// apparaître ou disparaître selon la façon dont le binaire a été construit, ce que M0 existe
    /// pour interdire.
    pub framework: String,
    /// Où il a été écrit, relatif ou absolu selon la configuration. `None` en cas d'échec.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Combien d'exigences sont **couvertes**, **partielles**, **hors du périmètre du DAG**.
    ///
    /// Les trois nombres viennent tels quels de `reg-evidence`. Publier `covered` seul ferait
    /// passer un dossier vide pour une bonne nouvelle.
    pub covered: usize,
    pub partial: usize,
    pub out_of_dag_scope: usize,
    /// Combien de nœuds du DAG entraient dans le périmètre. Zéro ici avec `covered: 0` dit
    /// « rien à mapper » ; un nombre élevé avec `covered: 0` dit « rien ne correspond » — deux
    /// diagnostics opposés que le seul compte de couverture confondrait.
    pub records_in_scope: usize,
    /// Le **manifest-root** re-dérivé pendant la dérivation. C'est lui qui lie ce dossier à un état
    /// PRÉCIS du DAG : sans lui, un lecteur ne pourrait pas savoir de quoi le dossier parle, ni
    /// vérifier qu'il parle encore de la même chose demain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_root: Option<String>,
    /// La **version de la lecture** qui a produit l'artefact. Un auditeur doit pouvoir distinguer
    /// « le run a changé » de « la règle de lecture a changé » — sans cette version, les deux se
    /// confondent, et le dossier devient inopposable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degrade_reason: Option<String>,
    /// Ce que l'échec coûte — et, ici, ce qu'il ne coûte PAS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consequence: Option<&'static str>,
}

impl RegReport {
    /// Un échec, nommé. `framework` est le libellé configuré — on le rend tel quel, même quand la
    /// dérivation n'a pas eu lieu : savoir à quelle question le dossier AURAIT répondu fait partie
    /// du diagnostic.
    #[must_use]
    pub fn echec(framework: &str, raison: impl Into<String>) -> Self {
        Self {
            produced: false,
            framework: framework.to_string(),
            path: None,
            covered: 0,
            partial: 0,
            out_of_dag_scope: 0,
            records_in_scope: 0,
            manifest_root: None,
            mapping_version: None,
            degrade_reason: Some(raison.into()),
            consequence: Some(
                "le dossier n'a PAS été écrit — mais le run, lui, est persisté : il se re-dérive à \
                 l'identique par `POST /v1/swarm/reg/:id`. Rien n'est perdu, seulement différé.",
            ),
        }
    }
}

/// **La couture de M5.** Dérive le dossier d'un run et l'écrit. Ne peut pas échouer.
///
/// Voir la documentation du module : un dossier manquant est **récupérable**, donc aucune erreur
/// d'ici ne doit jamais atteindre le client. La fonction rend un rapport, toujours.
///
/// `a_key` est la clé de **pile A** (celle du scellement d'essaim) ; la clé de pile B — celle avec
/// laquelle les nœuds de DAG sont scellés, et donc celle que la re-preuve exige — en est dérivée
/// par `provenance-bridge`, source unique de cette dérivation.
#[cfg(feature = "reg-evidence")]
#[must_use]
pub fn deriver(pont: &RegPont, run_id: Uuid, tenant: &str, a_key: &[u8], stored: &StoredRun) -> RegReport {
    let vue = provenance_bridge::RunView::new(
        run_id,
        tenant,
        &stored.records,
        &stored.llm_records,
        stored.root.as_deref(),
    );
    let bundle = match provenance_bridge::bundle_from_run(&vue, a_key) {
        Ok(b) => b,
        Err(e) => return RegReport::echec(slug(pont.framework), format!("nœuds de DAG non produits : {e}")),
    };
    let records = match reg_evidence::SealedDag::records_from_jsonl(&bundle.jsonl) {
        Ok(r) => r,
        Err(e) => return RegReport::echec(slug(pont.framework), format!("nœuds illisibles : {e}")),
    };
    let cle_b = provenance_bridge::stack_b_key(a_key, tenant);
    let mapping = mapping_pour(pont.framework);

    // Le périmètre est `FullTenant`, et c'est exact : ce bundle ne contient QUE les nœuds de ce
    // run. `CaseScope::Case` supposerait la convention d'identifiants `{cas}::{étape}` de la pile
    // de capture, que les nœuds issus d'un run d'essaim ne suivent pas — le dossier serait alors
    // vide, et un dossier vide qui a l'air correct est exactement ce qu'on refuse de produire.
    let artefact = match reg_evidence::derive(
        &records,
        &cle_b,
        &mapping,
        pont.framework,
        &reg_evidence::CaseScope::FullTenant,
    ) {
        Ok(a) => a,
        Err(e) => {
            // `derive` re-prouve AVANT de dériver : jamais d'évidence sur du non-prouvé. Un échec
            // ici n'est donc pas un incident d'écriture, c'est une chaîne qui ne tient pas — et
            // c'est le genre de chose qu'un exploitant doit voir en toutes lettres.
            return RegReport::echec(slug(pont.framework), format!("dérivation refusée : {e:?}"));
        }
    };

    let dossier = pont.dir.join(run_id.to_string());
    if let Err(e) = std::fs::create_dir_all(&dossier) {
        return RegReport::echec(slug(pont.framework), format!("dossier « {} » non créé : {e}", dossier.display()));
    }
    let fichiers = reg_evidence::reg_subtree(&artefact, &mapping).len();
    if let Err(e) = reg_evidence::write_reg_subtree(&dossier, &artefact, &mapping) {
        return RegReport::echec(slug(pont.framework), format!("écriture refusée : {e}"));
    }

    RegReport {
        produced: true,
        framework: slug(pont.framework).to_string(),
        path: Some(dossier.display().to_string()),
        covered: artefact.coverage_summary.covered,
        partial: artefact.coverage_summary.partial,
        out_of_dag_scope: artefact.coverage_summary.out_of_dag_scope,
        records_in_scope: artefact.n_records_in_scope,
        manifest_root: Some(artefact.manifest_root.clone()),
        mapping_version: Some(artefact.mapping_version.clone()),
        degrade_reason: None,
        consequence: None,
    }
}

/// Le nombre de fichiers qu'un dossier contient, tel que [`deriver`] l'écrit. Utile aux tests et à
/// la route de re-dérivation, qui doivent pouvoir l'affirmer sans relire le disque.
#[cfg(feature = "reg-evidence")]
#[must_use]
pub fn fichiers_attendus(dossier: &Path) -> usize {
    std::fs::read_dir(dossier.join("reg")).map(|d| d.flatten().count()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "reg-evidence")]
    #[test]
    fn un_cadre_ne_se_devine_jamais() {
        // Se tromper de cadre produit un dossier qui a l'air correct et répond à la mauvaise
        // question. On refuse donc plutôt que de retomber sur un défaut.
        assert_eq!(cadre_de(Some("eu_ai_act")), Some(Framework::EuAiAct));
        assert_eq!(cadre_de(Some("gdpr")), Some(Framework::Gdpr));
        assert_eq!(cadre_de(Some("csddd")), Some(Framework::Csddd));
        assert_eq!(cadre_de(Some("lksg")), Some(Framework::Lksg));
        assert_eq!(cadre_de(None), None);
        assert_eq!(cadre_de(Some("EU_AI_ACT")), None, "la casse compte");
        assert_eq!(cadre_de(Some("ai_act")), None);
        assert_eq!(cadre_de(Some("")), None);
    }

    /// La liste publiée au registre M0 doit être exactement celle que le code accepte — sinon le
    /// rapport de démarrage signalerait comme refusée une valeur qui marche, ou l'inverse.
    #[cfg(feature = "reg-evidence")]
    #[test]
    fn les_cadres_publies_sont_exactement_ceux_acceptes() {
        for c in CADRES {
            assert!(cadre_de(Some(c)).is_some(), "{c} est publié mais refusé");
        }
        // …et le slug est l'inverse exact de la lecture : un aller-retour qui ne boucle pas ferait
        // écrire le dossier dans un chemin qui ne correspond pas au cadre annoncé.
        for c in CADRES {
            let f = cadre_de(Some(c)).unwrap();
            assert_eq!(slug(f), *c, "aller-retour rompu pour {c}");
        }
    }

    #[cfg(feature = "reg-evidence")]
    #[test]
    fn les_deux_devoirs_de_vigilance_partagent_une_lecture() {
        // La CSDDD européenne et la LkSG allemande posent les mêmes exigences de traçabilité
        // fournisseur ; leur mapping est le même, et ce test empêche qu'on les sépare par accident.
        assert_eq!(
            mapping_pour(Framework::Csddd).version,
            mapping_pour(Framework::Lksg).version
        );
    }

    #[test]
    fn un_echec_dit_la_raison_ET_quil_est_rattrapable() {
        let r = RegReport::echec("gdpr", "store en carton");
        assert!(!r.produced);
        assert_eq!(r.framework, "gdpr");
        assert!(r.degrade_reason.as_deref().is_some_and(|s| s.contains("carton")));
        assert!(
            r.consequence.is_some_and(|c| c.contains("re-dérive")),
            "la conséquence doit dire que RIEN n'est perdu — c'est ce qui justifie de ne pas couper"
        );
        // Les trois nombres restent à zéro : un échec ne doit pas ressembler à une couverture nulle
        // légitime, et c'est `produced` qui les distingue.
        assert_eq!((r.covered, r.partial, r.out_of_dag_scope, r.records_in_scope), (0, 0, 0, 0));
        assert!(r.manifest_root.is_none(), "aucun état de DAG à désigner quand rien n'a été dérivé");
        assert!(r.mapping_version.is_none());
    }
}
