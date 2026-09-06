// SPDX-License-Identifier: UNLICENSED
//! `ApiKey` — wrapper sécurisé pour clés API (invariant I14).

use std::fmt;

use crate::error::{CoordError, CoordResult};

/// Clé API en mémoire avec redaction automatique au Debug/Display.
///
/// La clé brute n'est exposée QUE via `expose()`, méthode explicite
/// utilisée uniquement au moment de construire la requête HTTP.
#[derive(Clone)]
pub struct ApiKey {
    inner: String,
}

impl ApiKey {
    /// Construit depuis une string (validation minimale).
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self { inner: key.into() }
    }

    /// Construit depuis une variable d'environnement.
    ///
    /// Deux erreurs, et la distinction est **portante** — cf. [`CoordError::ConfigAbsent`] :
    ///
    /// - [`CoordError::ConfigAbsent`] si la variable **n'existe pas**. Rien n'a été fourni ;
    ///   personne n'a agi. Le démarrage de la passerelle se tait sur ce cas.
    /// - [`CoordError::Config`] si la valeur est **vide** ou si le **préfixe** ne convient pas.
    ///   Dans les deux cas une valeur a été posée puis rejetée : l'opérateur a agi, et il doit
    ///   l'apprendre au boot.
    ///
    /// ⚠ Les trois situations étaient auparavant typées `Config`, ce qui faisait affirmer à la
    /// passerelle « ce n'est pas une clé manquante » sur une clé manquante.
    pub fn from_env(var_name: &str, expected_prefix: &str) -> CoordResult<Self> {
        let v = std::env::var(var_name).map_err(|_| {
            CoordError::ConfigAbsent(format!("env var {var_name} not set"))
        })?;
        if v.is_empty() {
            return Err(CoordError::Config(format!("env var {var_name} is empty")));
        }
        if !v.starts_with(expected_prefix) {
            return Err(CoordError::Config(format!(
                "env var {var_name} does not start with expected prefix '{expected_prefix}'"
            )));
        }
        Ok(Self::new(v))
    }

    /// Expose la clé brute — à n'appeler QUE pour insérer dans un header HTTP.
    ///
    /// **JAMAIS** logger / printer / inclure dans une Error.
    pub fn expose(&self) -> &str {
        &self.inner
    }

    /// Suffixe redacté pour observabilité (les 4 derniers chars).
    #[must_use]
    pub fn suffix_redacted(&self) -> String {
        let n = self.inner.len();
        if n <= 4 {
            "***".to_string()
        } else {
            format!("***{}", &self.inner[n - 4..])
        }
    }
}

// Invariant I14 — Debug et Display redactent toujours.
impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiKey({})", self.suffix_redacted())
    }
}

impl fmt::Display for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.suffix_redacted())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_full_key() {
        let k = ApiKey::new("sk-ant-abc1234567890xyz");
        let dbg = format!("{k:?}");
        assert!(!dbg.contains("abc1234567890"));
        assert!(dbg.contains("***"));
        assert!(dbg.contains("0xyz"), "should show last 4 chars: {dbg}");
    }

    #[test]
    fn display_redacts() {
        let k = ApiKey::new("AIzaSyABCD1234EFGH5678");
        let d = format!("{k}");
        assert!(!d.contains("ABCD1234"));
        assert!(d.contains("***"));
    }

    #[test]
    fn expose_returns_raw() {
        let k = ApiKey::new("sk-ant-secret");
        assert_eq!(k.expose(), "sk-ant-secret");
    }

    /// **La distinction qui porte une décision.** Ce test asserte les DEUX cas ensemble, et non
    /// chacun de son côté : ce qui compte n'est pas qu'ils échouent, c'est qu'ils échouent
    /// DIFFÉREMMENT. Assertés séparément, ils passaient tous les deux quand les deux rendaient
    /// `Config` — et la passerelle affirmait alors « ce n'est pas une clé manquante » sur une clé
    /// manquante. Un test par cas ne pouvait pas voir cela ; celui-ci ne peut pas le manquer.
    #[test]
    fn from_env_distingue_absence_et_rejet() {
        std::env::remove_var("EFE_TEST_NONEXISTENT_KEY_XYZ");
        let absente = ApiKey::from_env("EFE_TEST_NONEXISTENT_KEY_XYZ", "sk-");
        assert!(
            matches!(absente, Err(CoordError::ConfigAbsent(_))),
            "variable inexistante ⟹ ConfigAbsent (personne n'a agi), obtenu : {absente:?}"
        );

        std::env::set_var("EFE_TEST_BAD_PREFIX", "wrong-format-key");
        let mauvais_prefixe = ApiKey::from_env("EFE_TEST_BAD_PREFIX", "sk-ant-");
        std::env::remove_var("EFE_TEST_BAD_PREFIX");
        assert!(
            matches!(mauvais_prefixe, Err(CoordError::Config(_))),
            "préfixe non conforme ⟹ Config (une valeur a été posée puis rejetée), obtenu : {mauvais_prefixe:?}"
        );

        std::env::set_var("EFE_TEST_EMPTY_KEY", "");
        let vide = ApiKey::from_env("EFE_TEST_EMPTY_KEY", "sk-");
        std::env::remove_var("EFE_TEST_EMPTY_KEY");
        assert!(
            matches!(vide, Err(CoordError::Config(_))),
            "valeur vide ⟹ Config : poser une variable vide EST un acte, obtenu : {vide:?}"
        );

        // L'anti-vacuité : si les trois rendaient la même chose, les trois assertions ci-dessus
        // passeraient encore le jour où quelqu'un les ré-unifierait. On l'interdit explicitement.
        assert!(
            !matches!(absente, Err(CoordError::Config(_))),
            "une absence ne doit JAMAIS être typée Config — c'est le défaut que ce test ferme"
        );
    }

    #[test]
    fn from_env_valid_succeeds() {
        std::env::set_var("EFE_TEST_GOOD_KEY", "sk-ant-fake-but-valid-format");
        let r = ApiKey::from_env("EFE_TEST_GOOD_KEY", "sk-ant-");
        assert!(r.is_ok());
        assert_eq!(r.unwrap().expose(), "sk-ant-fake-but-valid-format");
        std::env::remove_var("EFE_TEST_GOOD_KEY");
    }

    #[test]
    fn suffix_redacted_short_key() {
        let k = ApiKey::new("ab");
        assert_eq!(k.suffix_redacted(), "***");
    }
}
