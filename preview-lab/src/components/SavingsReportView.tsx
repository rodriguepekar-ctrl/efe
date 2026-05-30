// SPDX-License-Identifier: UNLICENSED
import type { ReactNode } from "react";

import {
  fmtMicrocents,
  fmtPct,
  RUN_MODES,
  RUN_MODE_LABEL,
  type SavingsReport,
} from "@/lib/report";

export function SavingsReportView({ report }: { report: SavingsReport }) {
  const pctMicro = Math.round(report.savings_ratio_microcents * 100);
  const pctCents = Math.round(report.savings_ratio * 100);

  return (
    <div className="flex flex-col gap-4">
      {/* Hero : le « 0 % » cents réparé par la résolution µ¢ */}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <HeroTile
          label="Savings cost — résolution cents (R2)"
          value={`${pctCents >= 0 ? "+" : ""}${-pctCents} %`}
          tone="muted"
          hint={`${report.baseline_total_cost_cents} ¢ → ${report.full_total_cost_cents} ¢`}
        />
        <HeroTile
          label="Savings cost — résolution µ¢ (R3.S9)"
          value={`${-pctMicro} %`}
          tone="accent"
          hint={`${fmtMicrocents(report.baseline_total_cost_microcents)} → ${fmtMicrocents(report.full_total_cost_microcents)}`}
        />
      </div>

      <p className="text-sm text-efe-muted">
        Le même pipeline, deux résolutions : en cents tout s'arrondit (savings
        invisibles) ; en microcents les vrais gains apparaissent.
      </p>

      {/* Détail par mode */}
      <div className="overflow-x-auto rounded-xl border border-efe-border bg-efe-panel">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-efe-border text-left text-efe-muted">
              <Th>Mode</Th>
              <Th>runs</Th>
              <Th>total µ¢</Th>
              <Th>moy µ¢/run</Th>
              <Th>latence moy</Th>
              <Th>qualité</Th>
            </tr>
          </thead>
          <tbody>
            {RUN_MODES.map((mode) => {
              const s = report.summaries[mode];
              if (!s || s.n_runs === 0) return null;
              return (
                <tr key={mode} className="border-b border-efe-border/40">
                  <Td className="text-efe-text">{RUN_MODE_LABEL[mode]}</Td>
                  <Td>{s.n_runs}</Td>
                  <Td>{fmtMicrocents(s.total_cost_microcents)}</Td>
                  <Td>{`${(s.avg_cost_microcents / 1000).toFixed(3)} ¢`}</Td>
                  <Td>{`${Math.round(s.avg_latency_ms)} ms`}</Td>
                  <Td>{fmtPct(s.quality_pass_rate)}</Td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      <div className="font-mono text-[11px] text-efe-muted">
        version {report.version} · généré {report.generated_at}
      </div>
    </div>
  );
}

function HeroTile({
  label,
  value,
  hint,
  tone,
}: {
  label: string;
  value: string;
  hint: string;
  tone: "accent" | "muted";
}) {
  const c = tone === "accent" ? "border-efe-accent text-efe-accent" : "border-efe-border text-efe-muted";
  return (
    <div className={`flex flex-col gap-1 rounded-xl border bg-efe-panel px-4 py-3 ${c}`}>
      <span className="text-[11px] uppercase tracking-wide text-efe-muted">{label}</span>
      <span className="font-mono text-3xl">{value}</span>
      <span className="font-mono text-[11px] text-efe-muted">{hint}</span>
    </div>
  );
}

function Th({ children }: { children: ReactNode }) {
  return <th className="px-3 py-2 font-medium">{children}</th>;
}
function Td({ children, className }: { children: ReactNode; className?: string }) {
  return <td className={`px-3 py-2 font-mono text-efe-muted ${className ?? ""}`}>{children}</td>;
}
