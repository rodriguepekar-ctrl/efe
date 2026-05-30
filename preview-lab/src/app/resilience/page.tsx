// SPDX-License-Identifier: UNLICENSED
"use client";

import { OverlayToasts } from "@/components/OverlayToasts";
import { Pipeline } from "@/components/Pipeline";
import { TokenStream } from "@/components/TokenStream";
import type { ResilienceScenario } from "@/lib/events";
import { useMissionStream } from "@/lib/useMissionStream";

const SCENARIOS: {
  id: ResilienceScenario;
  label: string;
  desc: string;
  expect: string;
}[] = [
  {
    id: "fallback",
    label: "LLM fallback (429)",
    desc: "Le LLM sélectionné renvoie 429 → la fallback chain bascule sur un LLM sain.",
    expect: "LLM_FALLBACK_TRIGGERED",
  },
  {
    id: "drafter_bypass",
    label: "Drafter bypass",
    desc: "Le drafter spéculatif échoue → bypass direct vers le verifier.",
    expect: "DRAFTER_BYPASSED",
  },
  {
    id: "noise_rejection",
    label: "Noise rejection",
    desc: "Le drafter produit du bruit (« La ») → rejeté du hot path → slow path.",
    expect: "DRAFT_REJECTED_AS_NOISE",
  },
];

export default function ResiliencePage() {
  const { state, status, error, startResilience, reset } = useMissionStream();
  const busy = status === "connecting" || status === "streaming";

  return (
    <div className="flex flex-col gap-4">
      <div>
        <h1 className="text-xl font-bold text-efe-text">Resilience Theater</h1>
        <p className="text-sm text-efe-muted">
          Déclenche de <em>vraies</em> dégradations et observe la récupération
          gracieuse — la mission se termine malgré l&apos;incident.
        </p>
      </div>

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        {SCENARIOS.map((s) => (
          <button
            key={s.id}
            disabled={busy}
            onClick={() => startResilience(s.id)}
            className="flex flex-col gap-2 rounded-xl border border-efe-border bg-efe-panel p-4 text-left hover:border-efe-accent disabled:cursor-not-allowed disabled:opacity-50"
          >
            <span className="text-sm font-semibold text-efe-text">{s.label}</span>
            <span className="text-[12px] text-efe-muted">{s.desc}</span>
            <span className="font-mono text-[11px] text-efe-cost">→ {s.expect}</span>
          </button>
        ))}
      </div>

      <div className="flex items-center justify-between rounded-lg border border-efe-border bg-efe-bg px-3 py-2 font-mono text-[11px] text-efe-muted">
        <span>
          statut : <span className="text-efe-text">{status}</span>
          {state.swarmId ? ` · swarm ${state.swarmId.slice(0, 8)}…` : ""}
        </span>
        <button onClick={reset} className="text-efe-muted hover:text-efe-text">
          réinitialiser
        </button>
      </div>

      {error && (
        <div className="rounded-lg border border-efe-danger bg-efe-panel px-3 py-2 text-sm text-efe-danger">
          Erreur : {error}
        </div>
      )}

      <OverlayToasts state={state} />
      <Pipeline state={state} />
      <TokenStream state={state} status={status} />

      {state.finished && (
        <p className="rounded-lg border border-efe-accent bg-efe-panel px-3 py-2 text-sm text-efe-accent">
          ✓ Mission terminée malgré la dégradation — récupération gracieuse.
        </p>
      )}
      {state.failed && (
        <p className="rounded-lg border border-efe-danger bg-efe-panel px-3 py-2 text-sm text-efe-danger">
          ✕ Mission en échec @ {state.failed.stage} : {state.failed.error}
        </p>
      )}
    </div>
  );
}
