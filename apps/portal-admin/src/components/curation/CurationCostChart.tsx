"use client";

import { useMemo } from "react";

type Bucket = {
  labels: Record<string, string>;
  call_count: number;
  input_tokens: number;
  output_tokens: number;
  cost_cents: number;
};

export function CurationCostChart({ buckets }: { buckets: Bucket[] }) {
  // Aggregate per provider (pie data) + per tool (bar data)
  const byProvider = useMemo(() => {
    const m = new Map<string, number>();
    for (const b of buckets) {
      const p = b.labels.provider ?? "(unknown)";
      m.set(p, (m.get(p) ?? 0) + b.cost_cents);
    }
    const total = Array.from(m.values()).reduce((s, v) => s + v, 0) || 1;
    return Array.from(m.entries()).map(([provider, cents]) => ({
      provider,
      cents,
      pct: cents / total * 100,
    })).sort((a, b) => b.cents - a.cents);
  }, [buckets]);

  const byTool = useMemo(() => {
    const m = new Map<string, number>();
    for (const b of buckets) {
      const t = b.labels.tool_name ?? "(unknown)";
      m.set(t, (m.get(t) ?? 0) + b.cost_cents);
    }
    return Array.from(m.entries())
      .map(([tool, cents]) => ({ tool, cents }))
      .sort((a, b) => b.cents - a.cents);
  }, [buckets]);

  const max = byTool[0]?.cents ?? 1;

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
      <div className="rounded-lg border border-slate-200 bg-white p-5">
        <h2 className="text-sm font-semibold text-slate-700 uppercase tracking-wide mb-3">
          By tool
        </h2>
        <ul className="space-y-2">
          {byTool.map((row) => (
            <li key={row.tool}>
              <div className="flex justify-between text-sm">
                <span className="text-slate-700"><code>{row.tool}</code></span>
                <span className="tabular-nums">${(row.cents / 100).toFixed(2)}</span>
              </div>
              <div className="h-2 bg-slate-100 rounded mt-1 overflow-hidden">
                <div
                  className="h-full bg-blue-500"
                  style={{ width: `${(row.cents / max) * 100}%` }}
                />
              </div>
            </li>
          ))}
        </ul>
      </div>

      <div className="rounded-lg border border-slate-200 bg-white p-5">
        <h2 className="text-sm font-semibold text-slate-700 uppercase tracking-wide mb-3">
          By provider
        </h2>
        <ul className="space-y-2">
          {byProvider.map((row) => (
            <li key={row.provider}>
              <div className="flex justify-between text-sm">
                <span className="text-slate-700">{row.provider}</span>
                <span className="tabular-nums">
                  ${(row.cents / 100).toFixed(2)} <span className="text-xs text-slate-500">({row.pct.toFixed(1)}%)</span>
                </span>
              </div>
              <div className="h-2 bg-slate-100 rounded mt-1 overflow-hidden">
                <div
                  className="h-full bg-emerald-500"
                  style={{ width: `${row.pct}%` }}
                />
              </div>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
