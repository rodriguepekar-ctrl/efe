// SPDX-License-Identifier: UNLICENSED
//! **R3.S3 (fix L4/D6)** — Rejet de bruit pour le hot path Speculative.
//!
//! ## Le problème (L4/D6)
//!
//! Le hot path Speculative (S7) acceptait tout draft de `]0, max_words]`
//! mots. Un drafter cheap (Gemini Flash) renvoyant `"La"` (1 mot,
//! fragment) voyait son draft accepté **sans arbitrage verifier** → la
//! réponse fausse était livrée. Observé S9 fixture 03 :
//! `final_text="La"` → quality FAIL, pass rate Speculative à 50 %.
//!
//! ## L'insight chirurgical
//!
//! > Le bruit n'est pas une question de **longueur**, c'est une question
//! > de **contenu**.
//!
//! - `"La"` = 1 mot, **0 mot de contenu** (article/stopword) → bruit.
//! - `"Paris"` = 1 mot, **1 mot de contenu** → réponse valide.
//!
//! Un naïf `min_words >= 2` rejetterait `"Paris"` (réponse FactualQa
//! canonique en 1 mot) ET `"La"` de la même façon → destruction du
//! savings hot path. La bonne discrimination est par **mot de contenu**.
//!
//! ## L'asymétrie des coûts
//!
//! - **Faux négatif** (rater du bruit) → réponse fausse livrée = SÉVÈRE.
//! - **Faux positif** (sur-rejeter) → verifier appelé inutilement = coût.
//!
//! Quand un draft est rejeté comme bruit, il tombe sur le **slow path** :
//! le verifier produit la vraie réponse. Le rejet de bruit ne produit
//! donc **jamais** de sortie fausse. La policy maximise le recall sur le
//! bruit (zéro faux négatif) en gardant les faux positifs bas.
//!
//! ## Contrainte de design
//!
//! Tous les checks sont **locaux, déterministes, O(n)** sur la longueur du
//! draft, avec court-circuit. **Zéro appel LLM/SLM** — sinon on défait le
//! hot path (dont l'existence même est d'éviter un appel modèle).

use std::collections::HashSet;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

/// **R3.S3** — Signal de bruit détecté dans un draft.
///
/// Si [`NoiseRejectionPolicy::detect`] retourne `Some`, le hot path est
/// bloqué et le draft tombe sur le slow path (verifier).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NoiseSignal {
    /// Draft vide ou whitespace pur.
    EmptyOrWhitespace,
    /// Draft sous le seuil minimum de mots total.
    BelowMinWords,
    /// Draft sans aucun mot de contenu (= 100 % stopwords/ponctuation).
    /// LE fix D6 : `"La"`, `"Le"`, `"The"`, `"A"`.
    OnlyStopwords,
    /// Draft = refus ou non-réponse connue (`"I don't know"`, `"N/A"`).
    RefusalPattern,
    /// Répétition dégénérée (`"go go go"`, unique/total < seuil).
    DegenerateRepetition,
}

impl NoiseSignal {
    /// Tag string stable pour bus event + metrics + audit.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EmptyOrWhitespace => "EMPTY_OR_WHITESPACE",
            Self::BelowMinWords => "BELOW_MIN_WORDS",
            Self::OnlyStopwords => "ONLY_STOPWORDS",
            Self::RefusalPattern => "REFUSAL_PATTERN",
            Self::DegenerateRepetition => "DEGENERATE_REPETITION",
        }
    }
}

/// **R3.S3** — Politique de rejet de bruit pour le hot path Speculative.
///
/// Calibrée pour recall max sur le bruit, faux positifs min sur les
/// réponses terses correctes (`"Paris"`). Tous les checks sont locaux et
/// déterministes (zéro appel LLM/SLM).
#[derive(Debug, Clone)]
pub struct NoiseRejectionPolicy {
    /// Mots minimum (total). Défaut 1 → rejette uniquement le vide.
    /// NB : on NE met PAS 2 (pénaliserait `"Paris"`, cf. doc module).
    pub min_words: usize,
    /// Mots de contenu (non-stopword) minimum. Défaut 1 → LE fix D6.
    pub min_content_words: usize,
    /// Active la détection de refus. Défaut true.
    pub reject_refusals: bool,
    /// Active la détection de répétition dégénérée. Défaut true.
    pub reject_repetition: bool,
    /// Seuil unique/total sous lequel un draft est « répétitif ».
    /// Défaut 0.34 (`"go go go"` = 1/3 ≈ 0.33 < 0.34 → rejeté ;
    /// `"Paris is capital"` = 3/3 = 1.0 → ok).
    pub repetition_min_unique_ratio: f32,
}

impl Default for NoiseRejectionPolicy {
    fn default() -> Self {
        Self {
            min_words: 1,
            min_content_words: 1,
            reject_refusals: true,
            reject_repetition: true,
            repetition_min_unique_ratio: 0.34,
        }
    }
}

impl NoiseRejectionPolicy {
    /// **R3.S3** — Policy permissive S7 (ne rejette QUE le vide).
    ///
    /// Utilisée comme opt-out backward-compat : désactive effectivement
    /// le rejet de bruit R3.S3 et restaure le comportement S7 (tout draft
    /// non-vide ≤ `max_words` prend le hot path).
    #[must_use]
    pub fn permissive_s7() -> Self {
        Self {
            min_words: 1,
            min_content_words: 0, // pas de check content
            reject_refusals: false,
            reject_repetition: false,
            repetition_min_unique_ratio: 0.0,
        }
    }

    /// **R3.S3** — Détecte le premier signal de bruit (court-circuit, du
    /// moins cher au plus cher). `None` = draft propre, hot path autorisé.
    #[must_use]
    pub fn detect(&self, draft: &str, _query: &str) -> Option<NoiseSignal> {
        // 1. Empty / whitespace (le moins cher).
        let trimmed = draft.trim();
        if trimmed.is_empty() {
            return Some(NoiseSignal::EmptyOrWhitespace);
        }

        // Tokenisation unique (réutilisée par les checks suivants).
        let words: Vec<&str> = trimmed.split_whitespace().collect();

        // 2. Below min words.
        if words.len() < self.min_words {
            return Some(NoiseSignal::BelowMinWords);
        }

        // 3. Refusal patterns (avant content-word : un refus PEUT contenir
        //    des mots de contenu — « I cannot answer that »).
        if self.reject_refusals && is_refusal(trimmed) {
            return Some(NoiseSignal::RefusalPattern);
        }

        // 4. Only stopwords (LE fix D6).
        if content_word_count(&words) < self.min_content_words {
            return Some(NoiseSignal::OnlyStopwords);
        }

        // 5. Degenerate repetition (mots de contenu répétés).
        if self.reject_repetition
            && is_degenerate_repetition(&words, self.repetition_min_unique_ratio)
        {
            return Some(NoiseSignal::DegenerateRepetition);
        }

        None // draft propre
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────

/// Normalise un token : lowercase + retire la ponctuation de bordure.
/// `"Paris."` → `"paris"` ; `"La,"` → `"la"` ; `"N/A"` → `"n/a"`.
fn normalize_token(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

/// Compte les mots de CONTENU (non-stopword, après normalisation).
fn content_word_count(words: &[&str]) -> usize {
    words
        .iter()
        .filter(|w| {
            let norm = normalize_token(w);
            !norm.is_empty() && !STOPWORDS.contains(norm.as_str())
        })
        .count()
}

/// Détecte les refus / non-réponses connus. Match sur le draft normalisé
/// lowercase. Volontairement CONSERVATEUR (recall sur le refus franc).
fn is_refusal(draft: &str) -> bool {
    let lo = draft.to_lowercase();
    let lo = lo.trim();
    REFUSALS.iter().any(|r| lo == *r || lo.starts_with(r))
}

/// Répétition dégénérée : ratio mots-uniques / mots-total < seuil.
/// Sur < 3 mots, ne déclenche jamais (trop peu de signal).
fn is_degenerate_repetition(words: &[&str], min_unique_ratio: f32) -> bool {
    if words.len() < 3 {
        return false;
    }
    let unique: HashSet<String> = words.iter().map(|w| normalize_token(w)).collect();
    let ratio = unique.len() as f32 / words.len() as f32;
    ratio < min_unique_ratio
}

/// Stopwords FR + EN — focalisés sur les tokens à HAUT RISQUE de bruit :
/// articles, déterminants, pronoms, conjonctions, prépositions courantes,
/// auxiliaires. PAS une liste IR exhaustive (on veut le recall sur le
/// bruit, pas un filtre de pertinence).
static STOPWORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        // ── FR articles / déterminants ──
        "le", "la", "les", "un", "une", "des", "du", "de", "l", "d", "ce",
        "cet", "cette", "ces", "mon", "ma", "mes", "ton", "ta", "tes", "son",
        "sa", "ses", "notre", "votre", "leur", "leurs",
        // ── FR pronoms / conjonctions / prépositions / auxiliaires ──
        "je", "tu", "il", "elle", "on", "nous", "vous", "ils", "elles", "se",
        "et", "ou", "mais", "donc", "or", "ni", "car", "que", "qui", "quoi",
        "à", "au", "aux", "en", "dans", "sur", "sous", "par", "pour", "avec",
        "est", "sont", "être", "a", "ont", "avoir", "ce", "cela", "ça",
        // ── EN articles / determiners ──
        "the", "a", "an", "this", "that", "these", "those", "my", "your",
        "his", "her", "its", "our", "their",
        // ── EN pronouns / conjunctions / prepositions / auxiliaries ──
        "i", "you", "he", "she", "it", "we", "they", "me", "him", "them",
        "and", "or", "but", "so", "nor", "for", "yet",
        "to", "of", "in", "on", "at", "by", "with", "from", "as",
        "is", "are", "be", "was", "were", "been", "has", "have", "had", "do",
        "does", "did",
    ]
    .into_iter()
    .collect()
});

/// Patterns de refus / non-réponse (lowercase). Match exact OU prefix.
const REFUSALS: &[&str] = &[
    // ── EN ──
    "i don't know",
    "i do not know",
    "i cannot",
    "i can't",
    "i'm not sure",
    "i am not sure",
    "no answer",
    "not available",
    "n/a",
    "none",
    "null",
    "undefined",
    "error",
    "unknown",
    // ── FR ──
    "je ne sais pas",
    "je ne peux pas",
    "aucune réponse",
    "pas disponible",
    "inconnu",
    "erreur",
];

// ═════════════════════════════════════════════════════════════════════════
// Tests
// ═════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn default_policy() -> NoiseRejectionPolicy {
        NoiseRejectionPolicy::default()
    }

    // ── EmptyOrWhitespace ──

    #[test]
    fn r3s3_empty_is_noise() {
        assert_eq!(
            default_policy().detect("", "q"),
            Some(NoiseSignal::EmptyOrWhitespace)
        );
    }

    #[test]
    fn r3s3_whitespace_only_is_noise() {
        assert_eq!(
            default_policy().detect("   \t\n ", "q"),
            Some(NoiseSignal::EmptyOrWhitespace)
        );
    }

    // ── OnlyStopwords (LE fix D6) ──

    #[test]
    fn r3s3_la_is_only_stopwords() {
        // Le cas D6 exact.
        assert_eq!(
            default_policy().detect("La", "Capitale de la France ?"),
            Some(NoiseSignal::OnlyStopwords)
        );
    }

    #[test]
    fn r3s3_the_is_only_stopwords() {
        assert_eq!(
            default_policy().detect("The", "Capital of France?"),
            Some(NoiseSignal::OnlyStopwords)
        );
    }

    #[test]
    fn r3s3_le_with_punctuation_is_only_stopwords() {
        // « Le. » → normalize → « le » → stopword.
        assert_eq!(
            default_policy().detect("Le.", "q"),
            Some(NoiseSignal::OnlyStopwords)
        );
    }

    // ── Réponses terses CORRECTES (pas de régression savings) ──

    #[test]
    fn r3s3_paris_passes() {
        // LE test anti-régression : « Paris » (1 content word) DOIT passer.
        assert_eq!(
            default_policy().detect("Paris", "Capitale de la France ?"),
            None
        );
    }

    #[test]
    fn r3s3_twilight_passes() {
        assert_eq!(default_policy().detect("twilight", "translate"), None);
    }

    #[test]
    fn r3s3_the_capital_passes() {
        // « The capital » = 1 stopword + 1 content → 1 content word → pass.
        assert_eq!(default_policy().detect("The capital", "q"), None);
    }

    #[test]
    fn r3s3_paris_with_punctuation_passes() {
        // « Paris. » → normalize → « paris » → content word → pass.
        assert_eq!(default_policy().detect("Paris.", "q"), None);
    }

    // ── RefusalPattern ──

    #[test]
    fn r3s3_refusal_en_rejected() {
        assert_eq!(
            default_policy().detect("I don't know", "q"),
            Some(NoiseSignal::RefusalPattern)
        );
    }

    #[test]
    fn r3s3_refusal_fr_rejected() {
        assert_eq!(
            default_policy().detect("Je ne sais pas", "q"),
            Some(NoiseSignal::RefusalPattern)
        );
    }

    #[test]
    fn r3s3_na_rejected() {
        assert_eq!(
            default_policy().detect("N/A", "q"),
            Some(NoiseSignal::RefusalPattern)
        );
    }

    #[test]
    fn r3s3_refusal_prefix_rejected() {
        // « I cannot answer that » commence par « i cannot » → refus.
        assert_eq!(
            default_policy().detect("I cannot answer that", "q"),
            Some(NoiseSignal::RefusalPattern)
        );
    }

    // ── DegenerateRepetition ──

    #[test]
    fn r3s3_la_la_la_is_only_stopwords_not_repetition() {
        // « La La La » → 0 content word → OnlyStopwords AVANT le check
        // répétition (ordre §4.1). On documente ce chemin.
        assert_eq!(
            default_policy().detect("La La La", "q"),
            Some(NoiseSignal::OnlyStopwords)
        );
    }

    #[test]
    fn r3s3_content_repetition_rejected() {
        // « go go go go » = mots de contenu répétés → DegenerateRepetition
        // (unique=1, total=4 → 0.25 < 0.34).
        assert_eq!(
            default_policy().detect("go go go go", "q"),
            Some(NoiseSignal::DegenerateRepetition)
        );
    }

    #[test]
    fn r3s3_paris_is_capital_passes() {
        // 4 mots, 2 content, unique ratio 1.0 → pass.
        assert_eq!(default_policy().detect("Paris is the capital", "q"), None);
    }

    // ── permissive_s7 (opt-out backward-compat) ──

    #[test]
    fn r3s3_permissive_s7_only_rejects_empty() {
        let p = NoiseRejectionPolicy::permissive_s7();
        // « La » accepté en mode permissif (= comportement S7).
        assert_eq!(p.detect("La", "q"), None);
        assert_eq!(p.detect("I don't know", "q"), None);
        assert_eq!(p.detect("go go go", "q"), None);
        // Mais le vide reste rejeté.
        assert_eq!(p.detect("", "q"), Some(NoiseSignal::EmptyOrWhitespace));
    }

    // ── BelowMinWords (policy custom) ──

    #[test]
    fn r3s3_below_min_words_custom_policy() {
        let p = NoiseRejectionPolicy {
            min_words: 3,
            ..NoiseRejectionPolicy::default()
        };
        // « Paris is » = 2 mots < 3 → BelowMinWords.
        assert_eq!(p.detect("Paris is", "q"), Some(NoiseSignal::BelowMinWords));
    }

    // ── NoiseSignal::as_str ──

    #[test]
    fn r3s3_signal_as_str_stable() {
        assert_eq!(NoiseSignal::OnlyStopwords.as_str(), "ONLY_STOPWORDS");
        assert_eq!(NoiseSignal::RefusalPattern.as_str(), "REFUSAL_PATTERN");
        assert_eq!(
            NoiseSignal::DegenerateRepetition.as_str(),
            "DEGENERATE_REPETITION"
        );
    }

    #[test]
    fn r3s3_signal_serde_roundtrip() {
        let s = NoiseSignal::OnlyStopwords;
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"ONLY_STOPWORDS\"");
        let back: NoiseSignal = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}
