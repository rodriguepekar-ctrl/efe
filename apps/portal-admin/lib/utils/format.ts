/** Format helpers — pure functions, easily unit-tested. */

export function formatLatencyMs(ms: number | null | undefined): string {
  if (ms == null) return "—";
  if (ms < 1) return "<1ms";
  if (ms < 1000) return `${ms.toFixed(0)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

export function formatRps(rps: number | null | undefined): string {
  if (rps == null) return "—";
  if (rps < 1) return rps.toFixed(2);
  if (rps < 1000) return rps.toFixed(0);
  return `${(rps / 1000).toFixed(1)}k`;
}

export function formatCostUsd(cost: number | null | undefined): string {
  if (cost == null) return "—";
  if (cost < 0.01) return `$${cost.toFixed(5)}`;
  if (cost < 100) return `$${cost.toFixed(2)}`;
  return `$${cost.toFixed(0)}`;
}

export function formatPercent(value: number | null | undefined, digits = 2): string {
  if (value == null) return "—";
  return `${(value * 100).toFixed(digits)}%`;
}

export function formatTimestamp(iso: string | null | undefined): string {
  if (!iso) return "—";
  try {
    return new Date(iso).toISOString().replace("T", " ").substring(0, 19);
  } catch {
    return iso;
  }
}

export function relativeTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const ms = Date.now() - new Date(iso).getTime();
  if (ms < 60_000) return "just now";
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m ago`;
  if (ms < 86_400_000) return `${Math.floor(ms / 3_600_000)}h ago`;
  return `${Math.floor(ms / 86_400_000)}d ago`;
}
