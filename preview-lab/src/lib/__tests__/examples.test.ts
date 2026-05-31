// SPDX-License-Identifier: UNLICENSED
//
// P5.6 — Contrat des prompts d'exemple. Verrouille la propriété centrale : le
// prompt « long » dépasse le seuil heuristique de pré-mâchage (sinon la démo ne
// montre PAS la compression — toute la raison d'être de P5.6). Plus les
// invariants de base (ids uniques, textes non vides, défaut = court).

import { describe, expect, it } from "vitest";

import {
  DEFAULT_PROMPT,
  EXAMPLE_PROMPTS,
  PRECHEW_HINT_CHARS,
  type ExamplePrompt,
} from "../examples";

const byId = (id: string): ExamplePrompt => {
  const found = EXAMPLE_PROMPTS.find((e) => e.id === id);
  if (!found) throw new Error(`exemple introuvable: ${id}`);
  return found;
};

describe("prompts d'exemple (P5.6)", () => {
  it("expose au moins un exemple court et un long", () => {
    expect(EXAMPLE_PROMPTS.length).toBeGreaterThanOrEqual(2);
    expect(byId("short")).toBeDefined();
    expect(byId("long")).toBeDefined();
  });

  it("le prompt LONG dépasse le seuil de pré-mâchage (> 500 car.)", () => {
    // Cœur de P5.6 : sans ça, le SavingsGauge reste plat.
    expect(byId("long").text.length).toBeGreaterThan(PRECHEW_HINT_CHARS);
  });

  it("le prompt COURT reste court (< 200 car.)", () => {
    expect(byId("short").text.length).toBeLessThan(200);
  });

  it("tous les textes sont non vides et les libellés présents", () => {
    for (const ex of EXAMPLE_PROMPTS) {
      expect(ex.text.trim().length).toBeGreaterThan(0);
      expect(ex.label.trim().length).toBeGreaterThan(0);
    }
  });

  it("les ids sont uniques", () => {
    const ids = EXAMPLE_PROMPTS.map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("le défaut du composer est l'exemple court", () => {
    expect(DEFAULT_PROMPT).toBe(byId("short").text);
  });

  it("l'exemple fact-check pré-remplit des sources (≥ 1 ligne non vide)", () => {
    const fc = byId("factcheck");
    expect(fc.sources, "le fact-check doit porter des sources").toBeDefined();
    const lines = (fc.sources ?? "").split("\n").filter((l) => l.trim().length > 0);
    expect(lines.length).toBeGreaterThanOrEqual(1);
  });

  it("seul l'exemple fact-check porte des sources (court/long n'en ont pas)", () => {
    expect(byId("short").sources).toBeUndefined();
    expect(byId("long").sources).toBeUndefined();
  });
});
