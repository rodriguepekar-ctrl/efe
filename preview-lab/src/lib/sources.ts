// SPDX-License-Identifier: UNLICENSED
//
// P5 (suite) — Parsing des sources candidates saisies dans le `MissionComposer`.
//
// Convention de saisie : UNE source par ligne. La ligne (trimée) devient le
// `text` ; l'`id` est auto-généré (`src-N`, N = rang 1-based parmi les lignes
// non vides). Les lignes vides sont ignorées. Le bridge re-valide et normalise
// ensuite (trim, drop des vides, plafonds 32 sources / 4000 car.) — ceci n'est
// qu'une commodité de saisie, PAS la garde de sécurité.

import type { CandidateSourceRef } from "./events";

/**
 * Convertit un texte multi-lignes en `CandidateSourceRef[]` (1 ligne = 1 source).
 * Entrée vide / uniquement blanche → `[]` (verifier dormant, pas d'erreur).
 */
export function parseSources(raw: string): CandidateSourceRef[] {
  return raw
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .map((text, i) => ({ id: `src-${i + 1}`, text }));
}
