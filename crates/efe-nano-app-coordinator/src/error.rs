// SPDX-License-Identifier: UNLICENSED
//! Taxonomie d'erreurs des nano-robots — 8 variantes typées + retry hints.
//!
//! Conception : chaque variante porte assez de contexte pour
//! 1. Logger lisiblement (via `tracing`).
//! 2. Décider d'un retry (`should_retry()`).
//! 3. Reconstituer un audit entry même en cas d'échec.

use std::time::Duration;

/// Résultat shorthand utilisé par tous les clients.
pub type RobotResult<T> = std::result::Result<T, RobotError>;

/// Erreurs canoniques retournées par les nano-robots.
#[derive(Debug, thiserror::Error)]
pub enum RobotError {
    /// Échec réseau (TCP refused, DNS, TLS, etc.).
    #[error("transport failure for {robot}: {source}")]
    Transport {
        /// Identifiant du robot (`analyzer`, `reformulator`, ...).
        robot: &'static str,
        /// Cause originelle de reqwest.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// `llama-server` a retourné un statut HTTP non-2xx.
    #[error("HTTP {status} from {robot}: {body}")]
    HttpStatus {
        /// Identifiant du robot.
        robot: &'static str,
        /// Code HTTP retourné.
        status: u16,
        /// Body partiel (premier 512 chars).
        body: String,
    },

    /// La sortie du robot ne valide pas le JSON Schema attendu.
    ///
    /// Cas extrême : la grammar GBNF est censée empêcher cela.
    /// Si on l'observe, c'est soit un bug de grammar, soit un bug de
    /// `llama-server`, soit un override grammar absent.
    #[error("schema violation in {robot}: {error}")]
    SchemaViolation {
        /// Identifiant du robot.
        robot: &'static str,
        /// Description de la violation.
        error: String,
        /// Sortie brute pour debug (truncatée à 1 KB).
        raw_output: String,
    },

    /// Le JSON valide mais viole une contrainte sémantique métier.
    ///
    /// Exemple : `confidence > 1.0`, `start_char > end_char`, `entity.text == ""`.
    #[error("semantic violation in {robot}: {issues}")]
    SemanticViolation {
        /// Identifiant du robot.
        robot: &'static str,
        /// Description des violations.
        issues: String,
    },

    /// Dépassement du timeout d'invocation.
    #[error("timeout after {ms}ms invoking {robot}")]
    Timeout {
        /// Identifiant du robot.
        robot: &'static str,
        /// Durée écoulée avant abandon.
        ms: u64,
    },

    /// Circuit breaker du robot est ouvert (trop d'erreurs récentes).
    #[error("circuit breaker open for {robot}")]
    CircuitOpen {
        /// Identifiant du robot.
        robot: &'static str,
    },

    /// L'écriture dans la chaîne d'audit HMAC a échoué.
    #[error("audit chain write failed for {robot}: {reason}")]
    AuditFailure {
        /// Identifiant du robot.
        robot: &'static str,
        /// Cause détaillée.
        reason: String,
    },

    /// Erreur de configuration au boot (fichier grammar absent, etc.).
    #[error("config error for {robot}: {reason}")]
    Config {
        /// Identifiant du robot.
        robot: &'static str,
        /// Cause détaillée.
        reason: String,
    },
}

impl RobotError {
    /// Le robot impliqué dans l'erreur.
    #[must_use]
    pub fn robot(&self) -> &'static str {
        match self {
            Self::Transport { robot, .. }
            | Self::HttpStatus { robot, .. }
            | Self::SchemaViolation { robot, .. }
            | Self::SemanticViolation { robot, .. }
            | Self::Timeout { robot, .. }
            | Self::CircuitOpen { robot }
            | Self::AuditFailure { robot, .. }
            | Self::Config { robot, .. } => robot,
        }
    }

    /// Le caller devrait-il retenter cette opération ?
    ///
    /// Suit la table de retry policy documentée dans
    /// `PHASE_1_JSON_GRAMMAR.md §12.2`.
    #[must_use]
    pub fn should_retry(&self) -> bool {
        match self {
            Self::Transport { .. } | Self::Timeout { .. } => true,
            Self::HttpStatus { status, .. } => *status >= 500,
            // Retry intelligent côté caller avec un prompt enrichi
            // (cf. invoke_with_repair).
            Self::SchemaViolation { .. } | Self::SemanticViolation { .. } => true,
            Self::CircuitOpen { .. } | Self::AuditFailure { .. } | Self::Config { .. } => false,
        }
    }

    /// Backoff suggéré avant le prochain retry.
    #[must_use]
    pub fn suggested_backoff(&self, attempt: u32) -> Duration {
        let base_ms = match self {
            Self::Transport { .. } | Self::Timeout { .. } => 200,
            Self::HttpStatus { .. } => 500,
            Self::SchemaViolation { .. } | Self::SemanticViolation { .. } => 100,
            _ => 1000,
        };
        // Exponential backoff capped at 10s.
        let ms = base_ms * 2u64.pow(attempt.min(6));
        Duration::from_millis(ms.min(10_000))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_should_retry() {
        let err = RobotError::Transport {
            robot: "analyzer",
            source: "dns failure".into(),
        };
        assert!(err.should_retry());
        assert_eq!(err.robot(), "analyzer");
    }

    #[test]
    fn http_4xx_should_not_retry() {
        let err = RobotError::HttpStatus {
            robot: "analyzer",
            status: 400,
            body: "bad request".into(),
        };
        assert!(!err.should_retry());
    }

    #[test]
    fn http_5xx_should_retry() {
        let err = RobotError::HttpStatus {
            robot: "verifier",
            status: 503,
            body: "overloaded".into(),
        };
        assert!(err.should_retry());
    }

    #[test]
    fn circuit_open_should_not_retry() {
        let err = RobotError::CircuitOpen {
            robot: "compressor",
        };
        assert!(!err.should_retry());
    }

    #[test]
    fn backoff_grows_exponentially_then_caps() {
        let err = RobotError::Transport {
            robot: "a",
            source: "x".into(),
        };
        let b0 = err.suggested_backoff(0).as_millis();
        let b1 = err.suggested_backoff(1).as_millis();
        let b2 = err.suggested_backoff(2).as_millis();
        let big = err.suggested_backoff(20).as_millis();
        assert_eq!(b0, 200);
        assert_eq!(b1, 400);
        assert_eq!(b2, 800);
        assert_eq!(big, 10_000); // capped
    }
}
