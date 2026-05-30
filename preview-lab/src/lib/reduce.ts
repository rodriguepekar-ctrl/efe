// SPDX-License-Identifier: UNLICENSED
//
// Réducteur **pur** : `StreamEvent[]` → état dérivé pour le rendu de la scène
// Live Mission. Aucune dépendance React → testable en isolation (T8).

import type {
  ChewingMetrics,
  LlmFinished,
  MissionTag,
  PheromoneDeposited,
  SpeculationResolved,
  StreamEvent,
  TaskClass,
  VerdictReady,
  VoteResolved,
} from "./events";

export type RobotStatus = "idle" | "running" | "done" | "failed";

export interface RobotState {
  id: string;
  status: RobotStatus;
  latencyMs?: number;
  error?: string | null;
}

export type RunStatus = "idle" | "running" | "done" | "failed";

export type OverlayKind = "fallback" | "circuit" | "noise" | "bypass" | "llm_failed";

export interface Overlay {
  kind: OverlayKind;
  message: string;
  ts: string;
}

export interface MissionState {
  swarmId: string | null;
  mission: MissionTag | null;
  userText: string | null;
  taskClass: TaskClass | null;
  status: RunStatus;
  robots: Record<string, RobotState>;
  robotOrder: string[];
  llmTokens: string;
  llmProvider: string | null;
  draftTokens: string;
  metrics: ChewingMetrics | null;
  llmFinished: LlmFinished | null;
  verdicts: VerdictReady[];
  vote: VoteResolved | null;
  speculation: SpeculationResolved | null;
  pheromones: PheromoneDeposited[];
  overlays: Overlay[];
  finished: boolean;
  failed: { stage: string; error: string } | null;
  auditHash: string | null;
  latencyMs: number | null;
}

export function initialMissionState(): MissionState {
  return {
    swarmId: null,
    mission: null,
    userText: null,
    taskClass: null,
    status: "idle",
    robots: {},
    robotOrder: [],
    llmTokens: "",
    llmProvider: null,
    draftTokens: "",
    metrics: null,
    llmFinished: null,
    verdicts: [],
    vote: null,
    speculation: null,
    pheromones: [],
    overlays: [],
    finished: false,
    failed: null,
    auditHash: null,
    latencyMs: null,
  };
}

function touchRobot(s: MissionState, id: string): RobotState {
  if (!s.robots[id]) {
    s.robots[id] = { id, status: "idle" };
    s.robotOrder.push(id);
  }
  return s.robots[id];
}

/**
 * Plie une liste d'events en un `MissionState`. Pure : ne mute pas `events`.
 * (On mute un accumulateur local fraîchement créé — sûr.)
 */
export function reduceMissionState(events: readonly StreamEvent[]): MissionState {
  const s = initialMissionState();
  if (events.length > 0 && s.status === "idle") {
    s.status = "running";
  }

  for (const ev of events) {
    switch (ev.type) {
      case "SWARM_STARTED":
        s.swarmId = ev.swarm_id;
        s.mission = ev.mission;
        s.userText = ev.user_text;
        s.taskClass = ev.task_class;
        s.status = "running";
        break;
      case "ROBOT_STARTED": {
        const r = touchRobot(s, ev.robot_id);
        r.status = "running";
        break;
      }
      case "ROBOT_FINISHED": {
        const r = touchRobot(s, ev.robot_id);
        r.status = ev.success ? "done" : "failed";
        r.latencyMs = ev.latency_ms;
        r.error = ev.error;
        break;
      }
      case "PHEROMONE_DEPOSITED":
        s.pheromones.push(ev);
        break;
      case "LLM_TOKEN_STREAMED":
        s.llmTokens += ev.token;
        s.llmProvider = ev.provider;
        break;
      case "LLM_FINISHED":
        s.llmFinished = ev;
        s.llmProvider = ev.provider;
        break;
      case "VERDICT_READY":
        s.verdicts.push(ev);
        break;
      case "VOTE_RESOLVED":
        s.vote = ev;
        break;
      case "DRAFT_TOKEN":
        s.draftTokens += ev.token;
        break;
      case "SPECULATION_RESOLVED":
        s.speculation = ev;
        break;
      case "PRE_CHEWING_METRICS":
        s.metrics = ev.metrics;
        break;
      case "SWARM_FINISHED":
        s.finished = true;
        s.status = "done";
        s.auditHash = ev.audit_chain_hash;
        s.latencyMs = ev.latency_ms;
        break;
      case "SWARM_FAILED":
        s.failed = { stage: ev.stage, error: ev.error };
        s.status = "failed";
        break;
      case "LLM_FALLBACK_TRIGGERED":
        s.overlays.push({
          kind: "fallback",
          message: `fallback ${ev.from} → ${ev.to} (${ev.reason})`,
          ts: ev.ts,
        });
        break;
      case "LLM_FAILED":
        s.overlays.push({
          kind: "llm_failed",
          message: `${ev.provider} en échec : ${ev.reason}`,
          ts: ev.ts,
        });
        break;
      case "SLM_CIRCUIT_OPENED": {
        const r = touchRobot(s, ev.robot);
        r.status = "failed";
        s.overlays.push({
          kind: "circuit",
          message: `circuit ${ev.robot} ouvert (${ev.failures} échecs)`,
          ts: ev.ts,
        });
        break;
      }
      case "DRAFTER_BYPASSED":
        s.overlays.push({
          kind: "bypass",
          message: `drafter ${ev.drafter} sauté → ${ev.fallback_to_verifier}`,
          ts: ev.ts,
        });
        break;
      case "DRAFT_REJECTED_AS_NOISE":
        s.overlays.push({
          kind: "noise",
          message: `bruit rejeté (${ev.signal}) : « ${ev.draft_excerpt} »`,
          ts: ev.ts,
        });
        break;
      case "HEARTBEAT":
        // keep-alive — ignoré côté état.
        break;
    }
  }
  return s;
}

/** Coût µ¢ observé (priorité : LlmFinished cost_cents×1000 ; sinon métrique). */
export function observedCostMicrocents(s: MissionState): number | null {
  if (s.llmFinished) return s.llmFinished.cost_cents * 1000;
  if (s.metrics) return s.metrics.router_estimated_cost_microcents ?? null;
  return null;
}
