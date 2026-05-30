// SPDX-License-Identifier: UNLICENSED
import type { MissionState, RobotStatus } from "@/lib/reduce";
import { RobotNode } from "./RobotNode";

function Arrow() {
  return <span className="select-none px-1 text-efe-border">→</span>;
}

export function Pipeline({ state }: { state: MissionState }) {
  const robotStatus = (id: string): RobotStatus => state.robots[id]?.status ?? "idle";
  const robotLat = (id: string): number | undefined => state.robots[id]?.latencyMs;

  // Nœud Router (in-process, pas d'event dédié) : "done" dès qu'un LLM est choisi.
  const selected = state.metrics?.selected_llm || state.llmProvider || undefined;
  const routerStatus: RobotStatus = selected ? "done" : "idle";

  // Nœud LLM (le gros modèle) : dérivé du stream de tokens / LlmFinished.
  const llmStatus: RobotStatus = state.llmFinished
    ? "done"
    : state.llmTokens || state.llmProvider
      ? "running"
      : "idle";

  return (
    <div className="flex flex-wrap items-center gap-1 rounded-xl border border-efe-border bg-efe-bg p-3">
      <RobotNode label="Analyzer" status={robotStatus("analyzer")} latencyMs={robotLat("analyzer")} />
      <Arrow />
      <RobotNode
        label="Router"
        sublabel={selected ? `→ ${selected}` : "Pareto"}
        status={routerStatus}
        latencyMs={state.metrics?.router_latency_ms}
      />
      <Arrow />
      <RobotNode label="Compressor" status={robotStatus("compressor")} latencyMs={robotLat("compressor")} />
      <Arrow />
      <RobotNode
        label="LLM"
        sublabel={state.llmProvider ?? "gros modèle"}
        status={llmStatus}
        latencyMs={state.llmFinished?.latency_ms}
      />
      <Arrow />
      <RobotNode label="Verifier" status={robotStatus("verifier")} latencyMs={robotLat("verifier")} />
    </div>
  );
}
