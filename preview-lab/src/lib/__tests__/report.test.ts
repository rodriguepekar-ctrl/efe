// SPDX-License-Identifier: UNLICENSED
//
// Formatters du rapport (P4.S5).

import { describe, expect, it } from "vitest";

import { fmtMicrocents, fmtPct, RUN_MODES } from "../report";

describe("report formatters", () => {
  it("fmtMicrocents", () => {
    expect(fmtMicrocents(37)).toBe("0.037 ¢");
    expect(fmtMicrocents(3000)).toBe("3.000 ¢");
    expect(fmtMicrocents(null)).toBe("—");
    expect(fmtMicrocents(undefined)).toBe("—");
  });

  it("fmtPct", () => {
    expect(fmtPct(0.75)).toBe("75 %");
    expect(fmtPct(1)).toBe("100 %");
    expect(fmtPct(null)).toBe("—");
  });

  it("RUN_MODES couvre les 5 modes", () => {
    expect(RUN_MODES).toEqual(["BASELINE", "S4_CHEWING", "S6_ROUTING", "FULL", "SPECULATIVE"]);
  });
});
