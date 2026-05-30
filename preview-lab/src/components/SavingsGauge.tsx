// SPDX-License-Identifier: UNLICENSED
import { observedCostMicrocents, type MissionState } from "@/lib/reduce";

function fmtMicrocents(mc: number | null): string {
  if (mc === null) return "—";
  return `${(mc / 1000).toFixed(3)} ¢`;
}

function Tile({
  label,
  value,
  hint,
  tone,
}: {
  label: string;
  value: string;
  hint?: string;
  tone: "latency" | "cost" | "accent";
}) {
  const border = {
    latency: "border-efe-latency",
    cost: "border-efe-cost",
    accent: "border-efe-accent",
  }[tone];
  const text = {
    latency: "text-efe-latency",
    cost: "text-efe-cost",
    accent: "text-efe-accent",
  }[tone];
  return (
    <div className={`flex flex-1 flex-col gap-1 rounded-lg border ${border} bg-efe-bg px-3 py-2`}>
      <span className="text-[11px] uppercase tracking-wide text-efe-muted">{label}</span>
      <span className={`font-mono text-xl ${text}`}>{value}</span>
      <span className="min-h-[1rem] text-[11px] text-efe-muted">{hint ?? "\u00a0"}</span>
    </div>
  );
}

export function SavingsGauge({ state }: { state: MissionState }) {
  const m = state.metrics;
  const latency = state.latencyMs ?? m?.total_pipeline_latency_ms ?? null;
  const cost = observedCostMicrocents(state);
  const savingsRatio = m ? m.savings_ratio : null;

  return (
    <div className="flex flex-col gap-2 rounded-xl border border-efe-border bg-efe-panel p-4">
      <h3 className="text-sm font-semibold text-efe-text">Triplet économique</h3>
      <div className="flex flex-wrap gap-2">
        <Tile
          label="Latence"
          tone="latency"
          value={latency !== null ? `${latency} ms` : "—"}
          hint={m ? `SLM ${m.total_slm_latency_ms} ms · LLM ${m.llm_latency_ms} ms` : undefined}
        />
        <Tile
          label="Coût (µ¢)"
          tone="cost"
          value={fmtMicrocents(cost)}
          hint={m && typeof m.spec_savings_microcents === "number" && m.spec_savings_microcents > 0
            ? `spec −${(m.spec_savings_microcents / 1000).toFixed(3)} ¢`
            : undefined}
        />
        <Tile
          label="Tokens compressés"
          tone="accent"
          value={savingsRatio !== null ? `${Math.round(savingsRatio * 100)} %` : "—"}
          hint={m ? `${m.input_tokens_raw} → ${m.input_tokens_compressed} tokens` : undefined}
        />
      </div>
      {m && (
        <div className="mt-1 font-mono text-[11px] text-efe-muted">
          router : {m.router_reason || "—"}
          {m.ab_arm ? ` · A/B ${m.ab_arm} (${m.ab_table_version ?? "?"})` : ""}
        </div>
      )}
    </div>
  );
}
