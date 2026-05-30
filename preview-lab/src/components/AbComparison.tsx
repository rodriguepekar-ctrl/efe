// SPDX-License-Identifier: UNLICENSED
import { fmtMicrocents, fmtPct, type ABComparison, type ArmStats } from "@/lib/report";

export function AbComparisonView({ cmp }: { cmp: ABComparison }) {
  return (
    <div className="flex flex-col gap-3">
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <ArmCard stats={cmp.control} tone="muted" title="Control" />
        <ArmCard stats={cmp.experiment} tone="accent" title="Experiment" />
      </div>

      <div className="flex flex-wrap gap-2 rounded-xl border border-efe-border bg-efe-bg p-3 font-mono text-[12px]">
        <Delta
          label="Δ coût"
          value={`${signed(cmp.cost_delta_microcents / 1000, 3)} ¢`}
          good={cmp.cost_delta_microcents < 0}
        />
        <Delta
          label="Δ qualité"
          value={`${signed(cmp.quality_delta * 100, 1)} pts`}
          good={cmp.quality_delta >= 0}
        />
        <Delta
          label="Δ latence"
          value={`${signed(cmp.latency_delta_ms, 0)} ms`}
          good={cmp.latency_delta_ms <= 0}
        />
      </div>
      <p className="text-[11px] text-efe-muted">
        Δ = experiment − control. Vert = experiment avantageux (moins cher / plus
        rapide / meilleure qualité).
      </p>
    </div>
  );
}

function ArmCard({
  stats,
  tone,
  title,
}: {
  stats: ArmStats;
  tone: "accent" | "muted";
  title: string;
}) {
  const c = tone === "accent" ? "border-efe-accent" : "border-efe-border";
  return (
    <div className={`flex flex-col gap-2 rounded-xl border bg-efe-panel p-4 ${c}`}>
      <div className="flex items-center justify-between">
        <span className="text-sm font-semibold text-efe-text">{title}</span>
        <span className="font-mono text-[11px] text-efe-muted">{stats.table_version || "—"}</span>
      </div>
      <Row label="runs" value={String(stats.n_runs)} />
      <Row label="coût moy" value={fmtMicrocents(stats.avg_cost_microcents)} />
      <Row label="qualité" value={fmtPct(stats.quality_pass_rate)} />
      <Row label="latence moy" value={`${Math.round(stats.avg_latency_ms)} ms`} />
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between font-mono text-[12px]">
      <span className="text-efe-muted">{label}</span>
      <span className="text-efe-text">{value}</span>
    </div>
  );
}

function Delta({ label, value, good }: { label: string; value: string; good: boolean }) {
  return (
    <span className={`rounded-full border px-2 py-0.5 ${good ? "border-efe-accent text-efe-accent" : "border-efe-cost text-efe-cost"}`}>
      {label} {value}
    </span>
  );
}

function signed(n: number, digits: number): string {
  const s = n.toFixed(digits);
  return n > 0 ? `+${s}` : s;
}
