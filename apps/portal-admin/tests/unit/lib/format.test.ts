import {
  formatCostUsd,
  formatLatencyMs,
  formatPercent,
  formatRps,
  formatTimestamp,
  relativeTime,
} from "@/lib/utils/format";
import { describe, expect, it } from "vitest";

describe("formatLatencyMs", () => {
  it("returns dash for null", () => expect(formatLatencyMs(null)).toBe("—"));
  it("formats sub-ms", () => expect(formatLatencyMs(0.5)).toBe("<1ms"));
  it("formats ms", () => expect(formatLatencyMs(123)).toBe("123ms"));
  it("formats seconds", () => expect(formatLatencyMs(2500)).toBe("2.50s"));
});

describe("formatRps", () => {
  it("formats sub-1", () => expect(formatRps(0.45)).toBe("0.45"));
  it("formats integer rps", () => expect(formatRps(123)).toBe("123"));
  it("formats k", () => expect(formatRps(2400)).toBe("2.4k"));
  it("returns dash for null", () => expect(formatRps(null)).toBe("—"));
});

describe("formatCostUsd", () => {
  it("formats micro-cents", () => expect(formatCostUsd(0.00012)).toBe("$0.00012"));
  it("formats cents", () => expect(formatCostUsd(1.23)).toBe("$1.23"));
  it("formats dollars", () => expect(formatCostUsd(123)).toBe("$123"));
});

describe("formatPercent", () => {
  it("formats 0", () => expect(formatPercent(0)).toBe("0.00%"));
  it("formats fractional", () => expect(formatPercent(0.0123)).toBe("1.23%"));
  it("respects digits override", () => expect(formatPercent(0.01234, 4)).toBe("1.2340%"));
});

describe("formatTimestamp", () => {
  it("returns dash for null", () => expect(formatTimestamp(null)).toBe("—"));
  it("formats ISO", () =>
    expect(formatTimestamp("2026-05-09T10:23:45.000Z")).toBe("2026-05-09 10:23:45"));
});

describe("relativeTime", () => {
  it("returns dash for null", () => expect(relativeTime(null)).toBe("—"));
  it("returns just-now for recent", () => {
    const recent = new Date(Date.now() - 30_000).toISOString();
    expect(relativeTime(recent)).toBe("just now");
  });
});
