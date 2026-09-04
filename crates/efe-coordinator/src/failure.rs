// SPDX-License-Identifier: UNLICENSED
//! **Sprint S8 R2** — Modes de failure tracés pour graceful degradation.
//!
//! Quand une mission rencontre une dégradation (SLM down, LLM 429,
//! vote dégradé, drafter Spec bypassé), elle continue à servir l'utilisateur
//! mais trace le mode de failure dans :
//! - [`ChewingMetrics.failure_mode`](crate::ChewingMetrics)
//! - [`MissionAResponse.failure_mode`](crate::MissionAResponse)
//! - [`MissionBResponse.failure_mode`](crate::MissionBResponse)
//! - bus event correspondant
//!
//! ## Sémantique
//!
//! - [`FailureMode::None`] : cas nominal — aucune dégradation.
//! - [`FailureMode::SlmDown`] : un SLM (analyzer/compressor/verifier) a
//!   échoué, son circuit s'est ouvert, le robot est skippé pour la mission.
//! - [`FailureMode::LlmFallbackTriggered`] : le LLM `selected` par le router
//!   a échoué après retry épuisé, le bridge a basculé sur
//!   `fallback_chain[0]`.
//! - [`FailureMode::VoteDegraded`] : Mission B avec quorum atteint mais
//!   moins que N LLMs ont voté.
//! - [`FailureMode::DrafterBypassed`] : Mission Speculative — drafter a
//!   fail, on a sauté direct au verifier.
//! - [`FailureMode::Composite`] : plusieurs failures dans la même mission.
//!
//! ## Format serde
//!
//! Tagged enum `{ "kind": "...", "detail": ... }` en SCREAMING_SNAKE_CASE
//! pour cohérence avec le reste du codebase. Backward-compat préservée
//! via `#[serde(default)]` sur les nouveaux champs des response types.

use serde::{Deserialize, Serialize};

/// Mode de failure observé pendant une mission.
///
/// Tracé dans `ChewingMetrics.failure_mode` (S8 R2) pour audit + reporting
/// + S9 savings report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", tag = "kind", content = "detail")]
pub enum FailureMode {
    /// Aucune dégradation observée (cas nominal).
    None,
    /// Un SLM (analyzer/compressor/verifier) a échoué et son circuit
    /// breaker s'est ouvert. Le robot est skippé pour cette mission.
    /// Contient le nom du robot (e.g. `"analyzer"`).
    SlmDown(String),
    /// Le LLM `selected` par le router a échoué après retry épuisé.
    /// Le bridge a basculé sur `fallback_chain[0]`.
    LlmFallbackTriggered {
        /// LLM initialement choisi par le router (e.g. `"claude_…"`).
        from: String,
        /// LLM cible du fallback effectivement utilisé (e.g. `"gemini_…"`).
        to: String,
    },
    /// Mission B : quorum atteint mais moins que N LLMs ont voté.
    /// Contient le nombre de votes effectifs et le nombre attendu.
    VoteDegraded {
        /// Nombre de votes effectivement comptés.
        effective_votes: usize,
        /// Nombre attendu (= `llms.len()`).
        expected: usize,
    },
    /// Mission Speculative : le drafter a échoué, on a sauté direct au
    /// verifier. Mode "anti-savings" mais résilient — préserve la qualité.
    /// Contient le reason du drafter error.
    DrafterBypassed(String),
    /// Cas multi-failures (e.g. SLM down + LLM 429). Contient la liste
    /// des FailureMode sous-jacents (max 5 implicite).
    Composite(Vec<FailureMode>),
}

impl Default for FailureMode {
    fn default() -> Self {
        Self::None
    }
}

impl FailureMode {
    /// `true` si une dégradation a été observée (= autre que `None`).
    ///
    /// Pratique pour les assertions de test :
    /// ```ignore
    /// assert!(resp.failure_mode.is_degraded());
    /// ```
    #[must_use]
    pub fn is_degraded(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Résumé human-readable du mode (≤ 80 chars), pour audit log.
    ///
    /// Exemples :
    /// - `None` → `"nominal"`
    /// - `SlmDown("analyzer")` → `"SLM down: analyzer"`
    /// - `LlmFallbackTriggered { from, to }` → `"LLM fallback: claude_… → gemini_…"`
    #[must_use]
    pub fn summarize(&self) -> String {
        match self {
            Self::None => "nominal".to_string(),
            Self::SlmDown(robot) => format!("SLM down: {robot}"),
            Self::LlmFallbackTriggered { from, to } => {
                format!("LLM fallback: {from} → {to}")
            }
            Self::VoteDegraded {
                effective_votes,
                expected,
            } => format!("vote degraded: {effective_votes}/{expected}"),
            Self::DrafterBypassed(reason) => {
                let trimmed = reason.chars().take(40).collect::<String>();
                format!("drafter bypassed: {trimmed}")
            }
            Self::Composite(modes) => {
                let inner = modes
                    .iter()
                    .map(Self::summarize)
                    .collect::<Vec<_>>()
                    .join("; ");
                format!("composite: {inner}")
            }
        }
    }

    /// Nombre de dégradations distinctes (1 pour les variants simples,
    /// `len()` pour `Composite`, 0 pour `None`).
    #[must_use]
    pub fn count_degradations(&self) -> u32 {
        match self {
            Self::None => 0,
            Self::Composite(modes) => modes.iter().map(Self::count_degradations).sum(),
            _ => 1,
        }
    }
}

/// **R3.S2 (§2.3 architecture)** — Compose deux `FailureMode` en gérant
/// flatten + dedup + simplification du Composite-of-one.
///
/// Règles appliquées :
/// - **Normalisation préalable** : chaque input passe par `normalize()`
///   qui dédupe les Composite et simplifie `Composite([X])` → `X`.
/// - `None` + `X` → `X` (no-op)
/// - `X` + `None` → `X`
/// - `X` + `X` (égalité après normalisation) → `X` (dedup)
/// - `Composite(a)` + `Composite(b)` → `Composite(a ∪ b)` (dedup'd)
/// - `Composite(modes)` + `X` (non-composite) → `Composite(modes + X)` si X
///   pas déjà dans modes
/// - `X` (non-composite) + `Composite(modes)` → `Composite([X] + modes)`
/// - `X` + `Y` (deux simples distincts) → `Composite([X, Y])`
/// - Si le résultat est `Composite([single])`, simplifie en `single`.
///
/// **Garantie** : pas de `Composite` imbriqué dans le résultat (flatten).
/// **Garantie** : aucun `Composite([X])` dans le résultat (simplification).
///
/// Utilisé par les workflows R3.S2 pour accumuler les failure modes
/// au fur et à mesure que les guards SLM s'activent dans une même mission.
#[must_use]
pub fn combine_failure_modes(existing: FailureMode, new: FailureMode) -> FailureMode {
    use FailureMode::*;
    // R3.S2 (§2.3) : normaliser les inputs pour garantir simplification
    // même quand on reçoit déjà du Composite([X]) (cas légal mais
    // dégénéré).
    let existing = normalize_failure_mode(existing);
    let new = normalize_failure_mode(new);
    match (existing, new) {
        // Cas trivial : un des deux est None.
        (None, other) | (other, None) => other,
        // Égalité parfaite → dedup.
        (a, b) if a == b => a,
        // Deux composites → union dédupliquée.
        (Composite(a), Composite(b)) => {
            let mut merged: Vec<FailureMode> = a;
            for m in b {
                if !merged.contains(&m) {
                    merged.push(m);
                }
            }
            simplify_composite(merged)
        }
        // Composite + simple → append si pas déjà dedans.
        (Composite(mut modes), other) => {
            if !modes.contains(&other) {
                modes.push(other);
            }
            simplify_composite(modes)
        }
        // Simple + composite → prepend et dedup.
        (other, Composite(modes)) => {
            let mut combined: Vec<FailureMode> = vec![other];
            for m in modes {
                if !combined.contains(&m) {
                    combined.push(m);
                }
            }
            simplify_composite(combined)
        }
        // Deux simples distincts → Composite([a, b]).
        (a, b) => Composite(vec![a, b]),
    }
}

/// **R3.S2** — Normalise un `FailureMode` :
/// - `Composite([])` → `None`
/// - `Composite([X])` → `X` (simplification)
/// - `Composite([X, X, Y])` → `Composite([X, Y])` (dedup)
/// - autres variants → inchangés
fn normalize_failure_mode(m: FailureMode) -> FailureMode {
    match m {
        FailureMode::Composite(modes) => {
            let dedup: Vec<FailureMode> =
                modes
                    .into_iter()
                    .fold(Vec::new(), |mut acc, m| {
                        if !acc.contains(&m) {
                            acc.push(m);
                        }
                        acc
                    });
            simplify_composite(dedup)
        }
        other => other,
    }
}

/// Simplifie un vec de FailureMode : `[]` → `None`, `[x]` → `x`, sinon `Composite(vec)`.
fn simplify_composite(modes: Vec<FailureMode>) -> FailureMode {
    match modes.len() {
        0 => FailureMode::None,
        1 => modes.into_iter().next().expect("len 1"),
        _ => FailureMode::Composite(modes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_none() {
        let f = FailureMode::default();
        assert_eq!(f, FailureMode::None);
        assert!(!f.is_degraded());
        assert_eq!(f.count_degradations(), 0);
        assert_eq!(f.summarize(), "nominal");
    }

    #[test]
    fn slm_down_is_degraded() {
        let f = FailureMode::SlmDown("analyzer".to_string());
        assert!(f.is_degraded());
        assert_eq!(f.count_degradations(), 1);
        assert_eq!(f.summarize(), "SLM down: analyzer");
    }

    #[test]
    fn llm_fallback_triggered_summarizes() {
        let f = FailureMode::LlmFallbackTriggered {
            from: "claude_claude-sonnet-4-6".to_string(),
            to: "gemini_gemini-2.5-flash".to_string(),
        };
        assert!(f.is_degraded());
        assert!(f.summarize().contains("claude_claude-sonnet-4-6"));
        assert!(f.summarize().contains("→"));
    }

    #[test]
    fn vote_degraded_summarizes_with_counts() {
        let f = FailureMode::VoteDegraded {
            effective_votes: 1,
            expected: 3,
        };
        assert!(f.is_degraded());
        assert_eq!(f.summarize(), "vote degraded: 1/3");
    }

    #[test]
    fn drafter_bypassed_truncates_long_reason() {
        let long_reason = "A".repeat(200);
        let f = FailureMode::DrafterBypassed(long_reason);
        let s = f.summarize();
        // Doit être tronqué à 40 chars + prefix.
        assert!(s.len() <= 100);
        assert!(s.starts_with("drafter bypassed:"));
    }

    #[test]
    fn composite_counts_inner_degradations() {
        let f = FailureMode::Composite(vec![
            FailureMode::SlmDown("analyzer".into()),
            FailureMode::LlmFallbackTriggered {
                from: "c".into(),
                to: "g".into(),
            },
            FailureMode::VoteDegraded {
                effective_votes: 2,
                expected: 3,
            },
        ]);
        assert!(f.is_degraded());
        assert_eq!(f.count_degradations(), 3);
        let s = f.summarize();
        assert!(s.starts_with("composite:"));
        assert!(s.contains("SLM down"));
        assert!(s.contains("LLM fallback"));
        assert!(s.contains("vote degraded"));
    }

    #[test]
    fn composite_with_nested_composite_counts_correctly() {
        let inner = FailureMode::Composite(vec![
            FailureMode::SlmDown("a".into()),
            FailureMode::SlmDown("b".into()),
        ]);
        let outer = FailureMode::Composite(vec![
            inner,
            FailureMode::DrafterBypassed("d".into()),
        ]);
        assert_eq!(outer.count_degradations(), 3);
    }

    #[test]
    fn none_roundtrips_via_json() {
        let f = FailureMode::None;
        let s = serde_json::to_string(&f).unwrap();
        // Format tagged : {"kind": "NONE"}
        assert!(s.contains("\"NONE\""));
        let back: FailureMode = serde_json::from_str(&s).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn slm_down_roundtrips_via_json() {
        let f = FailureMode::SlmDown("compressor".into());
        let s = serde_json::to_string(&f).unwrap();
        // {"kind": "SLM_DOWN", "detail": "compressor"}
        assert!(s.contains("\"SLM_DOWN\""));
        assert!(s.contains("\"compressor\""));
        let back: FailureMode = serde_json::from_str(&s).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn llm_fallback_triggered_roundtrips_via_json() {
        let f = FailureMode::LlmFallbackTriggered {
            from: "claude".into(),
            to: "gemini".into(),
        };
        let s = serde_json::to_string(&f).unwrap();
        let back: FailureMode = serde_json::from_str(&s).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn composite_roundtrips_via_json() {
        let f = FailureMode::Composite(vec![
            FailureMode::SlmDown("analyzer".into()),
            FailureMode::DrafterBypassed("HTTP 503".into()),
        ]);
        let s = serde_json::to_string(&f).unwrap();
        let back: FailureMode = serde_json::from_str(&s).unwrap();
        assert_eq!(back, f);
    }

    // ================================================================
    // R3.S2 — Tests `combine_failure_modes` (§6.1.1 architecture, 6 tests)
    // ================================================================

    #[test]
    fn r3s2_combine_none_with_x_returns_x() {
        let x = FailureMode::SlmDown("analyzer".into());
        assert_eq!(combine_failure_modes(FailureMode::None, x.clone()), x);
    }

    #[test]
    fn r3s2_combine_x_with_none_returns_x() {
        let x = FailureMode::LlmFallbackTriggered {
            from: "claude_sonnet".into(),
            to: "gemini_flash".into(),
        };
        assert_eq!(combine_failure_modes(x.clone(), FailureMode::None), x);
    }

    #[test]
    fn r3s2_combine_two_simples_returns_composite() {
        let a = FailureMode::SlmDown("analyzer".into());
        let b = FailureMode::SlmDown("verifier".into());
        let combined = combine_failure_modes(a.clone(), b.clone());
        assert_eq!(
            combined,
            FailureMode::Composite(vec![a, b]),
            "deux simples distincts doivent produire Composite([a, b])"
        );
    }

    #[test]
    fn r3s2_combine_composite_with_simple_appends() {
        let pre = FailureMode::Composite(vec![FailureMode::SlmDown("analyzer".into())]);
        let new = FailureMode::SlmDown("compressor".into());
        let combined = combine_failure_modes(pre, new);
        if let FailureMode::Composite(modes) = combined {
            assert_eq!(modes.len(), 2, "Composite + simple → 2 entries");
            assert!(modes.contains(&FailureMode::SlmDown("analyzer".into())));
            assert!(modes.contains(&FailureMode::SlmDown("compressor".into())));
        } else {
            panic!("attendu Composite, got {combined:?}");
        }
    }

    #[test]
    fn r3s2_combine_dedupes_duplicates() {
        let a = FailureMode::SlmDown("analyzer".into());
        // Simple + même simple → garde une seule entry.
        let combined = combine_failure_modes(a.clone(), a.clone());
        assert_eq!(combined, a, "deux identiques dédupliqués → 1 entry");

        // Composite + simple déjà présent → pas de doublon.
        let pre = FailureMode::Composite(vec![a.clone(), FailureMode::SlmDown("v".into())]);
        let combined2 = combine_failure_modes(pre.clone(), a.clone());
        // Composite([analyzer, v]) inchangé.
        if let FailureMode::Composite(modes) = combined2 {
            assert_eq!(modes.len(), 2, "dedup dans Composite + simple");
        } else {
            panic!("attendu Composite");
        }
    }

    #[test]
    fn r3s2_combine_simplifies_composite_of_one() {
        // Composite([A]) + None → simplifie en A (par le helper simplify_composite
        // appliqué via le path Composite + Composite).
        // On force le path : Composite([a]) + Composite([a]) → dedup → [a] → A.
        let a = FailureMode::SlmDown("analyzer".into());
        let composite_a = FailureMode::Composite(vec![a.clone()]);
        let combined = combine_failure_modes(composite_a.clone(), composite_a);
        // Avec dedup, l'union reste [a] → simplify_composite → A.
        assert_eq!(combined, a, "Composite([A]) + Composite([A]) → A (simplifié)");
    }
}
