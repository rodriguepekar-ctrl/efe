// SPDX-License-Identifier: UNLICENSED
import type { MissionState } from "@/lib/reduce";

const VERDICT_TONE: Record<string, string> = {
  SUPPORTED: "border-efe-accent text-efe-accent",
  UNSUPPORTED: "border-efe-cost text-efe-cost",
  CONTRADICTED: "border-efe-danger text-efe-danger",
  INSUFFICIENT_EVIDENCE: "border-efe-muted text-efe-muted",
};

/**
 * Affiche les verdicts de fact-check (`VERDICT_READY`). Rendu seulement si la
 * mission a fourni des `candidate_sources` (sinon aucun verdict — cas mock).
 */
export function Verdicts({ state }: { state: MissionState }) {
  if (state.verdicts.length === 0) return null;
  return (
    <div className="flex flex-col gap-2 rounded-xl border border-efe-border bg-efe-panel p-4">
      <h3 className="text-sm font-semibold text-efe-text">
        Fact-check ({state.verdicts.length})
      </h3>
      <ul className="flex flex-col gap-1">
        {state.verdicts.map((v, i) => (
          <li
            key={`${v.claim}-${i}`}
            className="flex items-center justify-between gap-2 rounded-md border border-efe-border/60 bg-efe-bg/50 px-2 py-1"
          >
            <span className="truncate text-[12px] text-efe-muted" title={v.claim}>
              {v.claim}
            </span>
            <span className="flex shrink-0 items-center gap-2 font-mono text-[11px]">
              <span
                className={`rounded-full border px-2 py-0.5 ${VERDICT_TONE[v.verdict] ?? "border-efe-border text-efe-muted"}`}
              >
                {v.verdict}
              </span>
              <span className="text-efe-muted">{Math.round(v.similarity * 100)}%</span>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
