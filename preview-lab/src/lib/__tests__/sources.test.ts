// SPDX-License-Identifier: UNLICENSED
//
// P5 (suite) — Contrat de `parseSources` (saisie UI → CandidateSourceRef[]).

import { describe, expect, it } from "vitest";

import { parseSources } from "../sources";

describe("parseSources (saisie des sources candidates)", () => {
  it("entrée vide ou blanche → [] (verifier dormant)", () => {
    expect(parseSources("")).toEqual([]);
    expect(parseSources("   \n  \n\t")).toEqual([]);
  });

  it("1 ligne = 1 source, texte trimé, id auto src-N", () => {
    const out = parseSources("  Paris est la capitale.  \nDeuxième source.");
    expect(out).toHaveLength(2);
    expect(out[0]).toEqual({ id: "src-1", text: "Paris est la capitale." });
    expect(out[1]).toEqual({ id: "src-2", text: "Deuxième source." });
  });

  it("lignes vides intercalées ignorées, numérotation sur les survivantes", () => {
    const out = parseSources("a\n\n   \nb\n");
    expect(out).toHaveLength(2);
    expect(out[0]).toEqual({ id: "src-1", text: "a" });
    expect(out[1]).toEqual({ id: "src-2", text: "b" });
  });

  it("ids uniques et non vides", () => {
    const out = parseSources("un\ndeux\ntrois");
    const ids = out.map((s) => s.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(out.every((s) => s.id.length > 0 && s.text.length > 0)).toBe(true);
  });
});
