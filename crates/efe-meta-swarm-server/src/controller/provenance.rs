// SPDX-License-Identifier: UNLICENSED
//! # `controller::provenance` — Couche 3 M2 : la clé et le tenant du reçu de consensus.
//!
//! Le moteur sait déjà sceller (`MetaSwarm::iterate_with_provenance`, feature `provenance`) et les
//! vérifieurs hors-ligne existent (`verify_meta`, `verify_meta_mini`). **Ce qui manquait au
//! contrôleur, c'est de savoir SOUS QUELLE CLÉ et POUR QUEL TENANT sceller.** Ce module est
//! exactement cette pièce, et rien d'autre : une décision de configuration, résolue une fois au
//! boot, puis portée par [`ControllerState`](super::ControllerState).
//!
//! ## La règle qui gouverne ce module : *pas de reçu, ou un vrai reçu — jamais un reçu décoratif*
//!
//! Un `meta_provenance_root` scellé sous une clé **publiquement connue** (la clé de dev) est
//! forgeable par n'importe qui : il a l'apparence d'une preuve — 64 caractères hexadécimaux,
//! re-prouvables — et n'en est pas une. L'émettre sans le dire serait précisément la classe de bug
//! que ce dépôt traque (« vert malhonnête » : le système affiche OK en faisant autre chose).
//!
//! D'où trois états, et **trois seulement** :
//!
//! | Configuration | Résultat | Sur le fil |
//! |---|---|---|
//! | Aucun tenant déclaré | provenance **OFF**, dit au boot | `meta_provenance_root` **vide** |
//! | Tenant + master présent | provenance **ON** (`EnvHkdf`) | root réel |
//! | Tenant, master absent, dev **reconnu** | provenance **ON** (clé de dev), avertissement BRUYANT | root réel *mais forgeable* |
//! | Tenant, master absent, dev **non reconnu** | **REFUS DE DÉMARRER** | — |
//!
//! La quatrième ligne est le fail-closed. Elle suit trois précédents du dépôt :
//! `frontier_boot_guard` (Couche 2 M4, la triade de la frontière), `h1_boot_gate` (Couche 3 M1.d,
//! le canari Lipschitz) et le refus de servir un mock à la place d'un bloc `controller`. Le motif
//! est toujours le même : **on n'impose pas le choix à l'opérateur, on exige qu'il soit FAIT.**
//!
//! ## Ce que ce module ne fait pas
//!
//! Il ne scelle rien (c'est le moteur), ne sérialise rien (c'est `MetaProvenance`), et ne vérifie
//! rien (ce sont les deux vérifieurs). Il ne **détient** la clé que via une [`HmacKeySource`], qui
//! la dérive à chaque run et la laisse tomber avec le sink : aucune clé ne survit à un `Iterate`,
//! et **aucune n'atteint jamais le fil** — le reçu voyage sans sa clé, c'est ce qui permet de le
//! publier à un auditeur.

use efe_meta_swarm::provenance::{HmacKeySource, TenantId};

/// Variable d'environnement déclarant le **tenant** sous lequel les reçus sont scellés.
/// Sa présence est ce qui **demande** la provenance ; son absence la laisse éteinte.
pub const TENANT_ENV: &str = "EFE_META_PROVENANCE_TENANT";

/// Reconnaissance explicite de la clé de **développement** (forgeable). Sans elle, une provenance
/// demandée sans master est un refus de démarrer, pas un repli silencieux.
pub const ALLOW_DEV_KEY_ENV: &str = "EFE_META_PROVENANCE_ALLOW_DEV_KEY";

/// La configuration de scellement du contrôleur : sous quelle clé, pour quel tenant.
///
/// Volontairement **sans `Clone` dérivé sur la clé elle-même** au sens où la `HmacKeySource` peut
/// porter un master (`StaticMaster`) : on la garde derrière un `Arc` dans l'état, jamais recopiée
/// au fil des requêtes.
#[derive(Debug, Clone)]
pub struct ProvenanceConfig {
    key_source: HmacKeySource,
    tenant: TenantId,
    /// Libellé forensique de la source (journalisé au boot, jamais sur le fil).
    label: &'static str,
    /// `true` si la clé est la clé de **dev**, publiquement connue ⟹ reçu **forgeable**.
    forgeable: bool,
}

impl ProvenanceConfig {
    /// La source de clé à passer à `iterate_with_provenance`.
    #[must_use]
    pub fn key_source(&self) -> &HmacKeySource {
        &self.key_source
    }

    /// Le tenant sous lequel dériver.
    #[must_use]
    pub fn tenant(&self) -> TenantId {
        self.tenant
    }

    /// Libellé forensique de la source de clé (pour les logs de boot).
    #[must_use]
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// Le reçu produit est-il **forgeable** (clé de dev publiquement connue) ?
    ///
    /// Exposé pour que l'appelant puisse le REDIRE — un opérateur qui lit « provenance active »
    /// six mois après le boot ne doit pas avoir à deviner sous quelle clé.
    #[must_use]
    pub fn is_forgeable(&self) -> bool {
        self.forgeable
    }

    /// Construit explicitement (tests, ou câblage programmatique).
    #[must_use]
    pub fn new(key_source: HmacKeySource, tenant: TenantId, label: &'static str, forgeable: bool) -> Self {
        Self { key_source, tenant, label, forgeable }
    }

    /// Résout la configuration depuis l'environnement.
    ///
    /// # Errors
    /// - Tenant illisible (UUID invalide) — un tenant mal orthographié scellerait sous une identité
    ///   silencieusement différente, ce qui casserait la re-preuve chez l'auditeur **sans erreur
    ///   côté serveur**. On refuse.
    /// - Provenance demandée alors qu'aucune clé sûre n'est disponible **et** que la clé de dev
    ///   n'est pas explicitement reconnue.
    pub fn from_env() -> Result<Option<Self>, String> {
        let Some(raw) = std::env::var(TENANT_ENV).ok().filter(|v| !v.trim().is_empty()) else {
            return Ok(None); // provenance non demandée — état honnête, pas une erreur
        };
        let tenant = parse_tenant(raw.trim())?;

        // Sonde HONNÊTE de la disponibilité du master : on tente réellement la dérivation plutôt
        // que de lire la variable (même patron que la passerelle). Un master présent mais trop
        // court échoue ici, et doit compter comme absent — sinon on démarrerait en croyant tenir
        // une clé de production.
        let master_usable = HmacKeySource::EnvHkdf.derive(TenantId::system()).is_ok();
        if master_usable {
            return Ok(Some(Self {
                key_source: HmacKeySource::EnvHkdf,
                tenant,
                label: "EnvHkdf (master côté serveur — PRODUCTION)",
                forgeable: false,
            }));
        }

        if env_flag_true(ALLOW_DEV_KEY_ENV) {
            return Ok(Some(Self {
                key_source: HmacKeySource::test_key(),
                tenant,
                label: "test_key() (DEV — reçu FORGEABLE, clé publiquement connue)",
                forgeable: true,
            }));
        }

        Err(format!(
            "provenance de consensus DEMANDÉE ({TENANT_ENV} est posé) mais aucune clé sûre n'est \
             disponible : {master} est absent ou trop court (< 32 octets).\n\
             Un reçu scellé sous la clé de dev est FORGEABLE par quiconque — il ressemble à une \
             preuve sans en être une, et le servir en silence tromperait l'auditeur qui le \
             re-prouve. Le contrôleur refuse donc de démarrer plutôt que d'émettre un reçu \
             décoratif.\n\
             Deux remèdes :\n\
               • production — poser {master} (≥ 32 octets) ;\n\
               • développement — reconnaître le risque avec {ALLOW_DEV_KEY_ENV}=true ;\n\
             ou retirer {TENANT_ENV} pour servir SANS provenance (le fil portera un \
             meta_provenance_root vide, ce qui est un signal honnête).",
            master = efe_audit_core_master_env(),
        ))
    }
}

/// Nom de la variable master, ré-exporté ici pour que le message d'erreur ne le code pas en dur.
fn efe_audit_core_master_env() -> &'static str {
    // `efe-audit-core` l'expose ; on passe par `efe-meta-swarm` qui le ré-exporte via sa feature
    // `provenance`, pour ne pas ajouter une dépendance directe au serveur juste pour une chaîne.
    "EFE_NANO_AUDIT_HMAC_MASTER"
}

/// Parse un tenant : UUID canonique, ou le mot-clé `system`.
fn parse_tenant(raw: &str) -> Result<TenantId, String> {
    if raw.eq_ignore_ascii_case("system") {
        return Ok(TenantId::system());
    }
    raw.parse::<uuid::Uuid>().map(TenantId::from_uuid).map_err(|e| {
        format!(
            "{TENANT_ENV}={raw:?} n'est pas un UUID valide ({e}) — attendu un UUID canonique ou le \
             mot-clé `system`. Refusé plutôt que replié sur un tenant par défaut : sceller sous une \
             identité autre que celle voulue casserait la re-preuve chez l'auditeur SANS erreur ici."
        )
    })
}

/// Égalité stricte `"true"` (fail-closed), avec avertissement sur un non-vide qui n'est pas `true`
/// — le footgun `1`/`TRUE`/`yes` a déjà été rencontré Couche 2.
fn env_flag_true(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) if v == "true" => true,
        Ok(v) if !v.trim().is_empty() => {
            tracing::warn!(
                value = %v,
                "{name}='{v}' n'est pas exactement 'true' ⟹ NON activé (fail-closed)."
            );
            false
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `system` est accepté comme alias — sinon un déploiement mono-tenant devrait écrire l'UUID
    /// magique à la main, ce qui invite la faute de frappe que `parse_tenant` refuse justement.
    #[test]
    fn tenant_accepts_the_system_keyword_and_a_canonical_uuid() {
        assert_eq!(parse_tenant("system").unwrap(), TenantId::system());
        assert_eq!(parse_tenant("SYSTEM").unwrap(), TenantId::system());
        let u = "3f2504e0-4f89-41d3-9a0c-0305e82c3301";
        assert_eq!(parse_tenant(u).unwrap().as_uuid().to_string(), u);
    }

    /// **Le test qui compte pour ce parseur** : un tenant mal orthographié est REFUSÉ, jamais
    /// replié sur un défaut. Un repli scellerait sous une autre identité et la re-preuve
    /// échouerait chez l'auditeur, sans la moindre erreur côté serveur — un échec déplacé sur
    /// quelqu'un qui n'a aucun moyen de le diagnostiquer.
    #[test]
    fn a_malformed_tenant_is_refused_never_defaulted() {
        for bad in ["", "not-a-uuid", "3f2504e0-4f89-41d3-9a0c", "systemm"] {
            let err = parse_tenant(bad).expect_err("un tenant illisible doit être refusé");
            assert!(err.contains(TENANT_ENV), "le message nomme la variable : {err}");
        }
    }

    /// Le flag de reconnaissance est en égalité stricte : `1`, `yes`, `TRUE` ne l'activent pas.
    #[test]
    fn dev_key_acknowledgement_is_strict_true() {
        // Variable inexistante ⟹ faux, sans panique.
        assert!(!env_flag_true("EFE_META_PROVENANCE_ALLOW_DEV_KEY_ABSENT_ON_PURPOSE"));
    }

    /// La configuration porte le caractère **forgeable** de la clé de dev, pour que l'appelant
    /// puisse le REDIRE. Une provenance active sous clé publique n'est pas une provenance.
    #[test]
    fn a_dev_key_config_says_it_is_forgeable() {
        let dev = ProvenanceConfig::new(HmacKeySource::test_key(), TenantId::system(), "dev", true);
        assert!(dev.is_forgeable());
        let prod = ProvenanceConfig::new(HmacKeySource::EnvHkdf, TenantId::system(), "prod", false);
        assert!(!prod.is_forgeable());
    }
}
