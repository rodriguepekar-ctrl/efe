// SPDX-License-Identifier: UNLICENSED
//! Type d'une entrée de blackboard.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Catégorie d'une entrée — typée pour permettre des filtres efficaces.
#[allow(missing_docs)] // chaque variant porte sa propre doc inline
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntryKind {
    /// Entité nommée extraite par l'Analyseur.
    Entity,
    /// Intent classifié par l'Analyseur.
    Intent,
    /// Décision de routage produite par le Filtreur.
    RoutingDecision,
    /// LLM sélectionné par le Routeur.
    LlmSelection,
    /// Texte reformulé par le Reformulateur.
    ReformulatedText,
    /// Texte compressé par le Compresseur.
    CompressedText,
    /// Verdict produit par le Vérificateur.
    Verdict,
    /// Token streamé par un LLM gros (Claude/GPT/...).
    LlmToken,
    /// Bulletin de vote produit par un LLM gros (Mission B).
    Vote,
    /// Trace générique de debugging.
    Trace,
}

/// Une entrée de blackboard (immutable une fois posée).
///
/// L'`intensity` peut décroître par garbage collection — c'est la
/// seule mutation possible (cf. invariant I7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlackboardEntry {
    /// UUID v7 — fournit ordre temporel global.
    pub entry_id: Uuid,
    /// Robot ou LLM qui a posé l'entrée (`"analyzer"`, `"claude_opus_4"`, ...).
    pub author: String,
    /// Catégorie (typée).
    pub kind: EntryKind,
    /// Payload sérialisé (JSON arbitraire — la responsabilité du caller).
    pub payload: serde_json::Value,
    /// Intensité de la phéromone ∈ [0,1].  Décroît avec le temps.
    pub intensity: f32,
    /// Estampille de dépôt.
    pub deposited_at: DateTime<Utc>,
    /// Décroissance — demi-vie en secondes.  Default 60 s.
    pub half_life_secs: u64,
    /// Taille sérialisée approximative (octets — pour bornage I2).
    pub size_bytes: usize,
}

impl BlackboardEntry {
    /// Construit une nouvelle entrée avec intensité 1.0 et demi-vie par défaut.
    ///
    /// La taille est calculée par `serde_json::to_vec(&payload).len()`.
    #[must_use]
    pub fn new(author: impl Into<String>, kind: EntryKind, payload: serde_json::Value) -> Self {
        let bytes = serde_json::to_vec(&payload).map(|v| v.len()).unwrap_or(0);
        Self {
            entry_id: Uuid::now_v7(),
            author: author.into(),
            kind,
            payload,
            intensity: 1.0,
            deposited_at: Utc::now(),
            half_life_secs: super::DEFAULT_HALF_LIFE_SECS,
            size_bytes: bytes,
        }
    }

    /// Variante avec demi-vie custom.
    #[must_use]
    pub fn with_half_life(
        author: impl Into<String>,
        kind: EntryKind,
        payload: serde_json::Value,
        half_life_secs: u64,
    ) -> Self {
        let mut e = Self::new(author, kind, payload);
        e.half_life_secs = half_life_secs;
        e
    }

    /// Intensité actuelle compte tenu du temps écoulé depuis le dépôt.
    ///
    /// Formule : `intensity * exp(-ln(2) * elapsed_secs / half_life)`.
    #[must_use]
    pub fn current_intensity(&self, now: DateTime<Utc>) -> f32 {
        let elapsed_secs = (now - self.deposited_at).num_seconds().max(0) as f32;
        let half = self.half_life_secs.max(1) as f32;
        let decay = (-std::f32::consts::LN_2 * elapsed_secs / half).exp();
        (self.intensity * decay).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_kind_roundtrip() {
        let k = EntryKind::Verdict;
        let s = serde_json::to_string(&k).unwrap();
        assert_eq!(s, "\"VERDICT\"");
        let back: EntryKind = serde_json::from_str(&s).unwrap();
        assert_eq!(back, k);
    }

    #[test]
    fn new_entry_has_unit_intensity() {
        let e = BlackboardEntry::new("analyzer", EntryKind::Entity, serde_json::json!({"x":1}));
        assert!((e.intensity - 1.0).abs() < 1e-6);
        assert_eq!(e.half_life_secs, 60);
        assert!(e.size_bytes > 0);
    }

    #[test]
    fn decay_at_half_life_gives_half_intensity() {
        let e = BlackboardEntry::with_half_life(
            "x",
            EntryKind::Trace,
            serde_json::json!(null),
            10,
        );
        let later = e.deposited_at + chrono::Duration::seconds(10);
        let i = e.current_intensity(later);
        assert!((i - 0.5).abs() < 0.01, "expected ~0.5, got {i}");
    }

    #[test]
    fn decay_far_in_future_approaches_zero() {
        let e = BlackboardEntry::new("x", EntryKind::Trace, serde_json::json!(null));
        let later = e.deposited_at + chrono::Duration::seconds(3600);
        assert!(e.current_intensity(later) < 1e-10);
    }

    #[test]
    fn decay_at_origin_is_full_intensity() {
        let e = BlackboardEntry::new("x", EntryKind::Trace, serde_json::json!({"a":42}));
        let i = e.current_intensity(e.deposited_at);
        assert!((i - 1.0).abs() < 1e-6);
    }

    #[test]
    fn intensity_is_monotone_non_increasing() {
        // Invariant I7 : intensity ne ré-augmente jamais hors dépôt.
        let e = BlackboardEntry::new("x", EntryKind::Trace, serde_json::json!(null));
        let mut prev = 1.0_f32;
        for s in 0..600 {
            let i = e.current_intensity(e.deposited_at + chrono::Duration::seconds(s));
            assert!(i <= prev + 1e-6, "non-monotone at s={s}");
            prev = i;
        }
    }
}
