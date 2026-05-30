// SPDX-License-Identifier: UNLICENSED
import type { ReactNode } from "react";

import type { MissionState } from "@/lib/reduce";
import type { StreamStatus } from "@/lib/useMissionStream";

export function TokenStream({
  state,
  status,
}: {
  state: MissionState;
  status: StreamStatus;
}) {
  const streaming = status === "streaming" && !state.finished && !state.failed;
  const hasDraft = state.draftTokens.length > 0;

  return (
    <div className="flex flex-col gap-2 rounded-xl border border-efe-border bg-efe-panel p-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-efe-text">Sortie LLM</h3>
        <span className="font-mono text-[11px] text-efe-muted">
          {state.llmProvider ?? "—"}
        </span>
      </div>

      {hasDraft && (
        <div className="rounded-md border border-efe-border/60 bg-efe-bg/60 p-2">
          <div className="mb-1 text-[11px] uppercase tracking-wide text-efe-draft">
            draft spéculatif
          </div>
          <p className="whitespace-pre-wrap font-mono text-sm text-efe-draft">
            {state.draftTokens}
          </p>
        </div>
      )}

      <p className="min-h-[3rem] whitespace-pre-wrap font-mono text-sm text-efe-text">
        {state.llmTokens || (status === "idle" ? "Lance une mission pour voir la sortie." : "…")}
        {streaming && <span className="ml-0.5 inline-block animate-pulseNode">▋</span>}
      </p>

      {state.speculation && (
        <div className="mt-1 flex flex-wrap gap-2 text-[11px]">
          <Badge tone={state.speculation.verifier_skipped ? "accent" : "cost"}>
            {state.speculation.verifier_skipped ? "HOT PATH (verifier skippé)" : "SLOW PATH (verifier appelé)"}
          </Badge>
          <Badge tone="latency">
            latence gagnée {state.speculation.latency_saved_ms} ms
          </Badge>
          <Badge tone="muted">
            {state.speculation.tokens_accepted} acceptés / {state.speculation.tokens_regenerated} regénérés
          </Badge>
        </div>
      )}

      {state.vote && (
        <div className="mt-1 text-[11px] text-efe-muted">
          <span className="text-efe-accent">vote → {state.vote.winner}</span>{" "}
          (score {state.vote.winner_score.toFixed(2)})
        </div>
      )}
    </div>
  );
}

function Badge({
  children,
  tone,
}: {
  children: ReactNode;
  tone: "accent" | "cost" | "latency" | "muted";
}) {
  const c = {
    accent: "border-efe-accent text-efe-accent",
    cost: "border-efe-cost text-efe-cost",
    latency: "border-efe-latency text-efe-latency",
    muted: "border-efe-border text-efe-muted",
  }[tone];
  return (
    <span className={`rounded-full border px-2 py-0.5 font-mono ${c}`}>{children}</span>
  );
}
