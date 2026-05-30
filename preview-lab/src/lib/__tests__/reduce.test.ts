// SPDX-License-Identifier: UNLICENSED
//
// T8 — Réducteur pur : events factices → état dérivé attendu.

import { describe, expect, it } from "vitest";

import type { StreamEvent } from "../events";
import { observedCostMicrocents, reduceMissionState } from "../reduce";

const TS = "2026-05-30T00:00:00Z";

describe("reduceMissionState (T8)", () => {
  it("état initial sur liste vide", () => {
    const s = reduceMissionState([]);
    expect(s.status).toBe("idle");
    expect(s.swarmId).toBeNull();
    expect(s.llmTokens).toBe("");
    expect(s.robotOrder).toEqual([]);
  });

  it("séquence Mission A nominale", () => {
    const events: StreamEvent[] = [
      { type: "SWARM_STARTED", swarm_id: "sw1", mission: "MISSION_A", user_text: "Capitale ?", task_class: "FACTUAL_QA", ts: TS },
      { type: "ROBOT_STARTED", swarm_id: "sw1", robot_id: "analyzer", ts: TS },
      { type: "ROBOT_FINISHED", swarm_id: "sw1", robot_id: "analyzer", success: true, latency_ms: 7, error: null, ts: TS },
      { type: "LLM_TOKEN_STREAMED", swarm_id: "sw1", provider: "mock-gemini", token: "Pa", index: 0, ts: TS },
      { type: "LLM_TOKEN_STREAMED", swarm_id: "sw1", provider: "mock-gemini", token: "ris", index: 1, ts: TS },
      { type: "LLM_FINISHED", swarm_id: "sw1", provider: "mock-gemini", tokens_generated: 2, cost_cents: 1, latency_ms: 30, ts: TS },
      {
        type: "PRE_CHEWING_METRICS",
        swarm_id: "sw1",
        ts: TS,
        metrics: {
          input_tokens_raw: 20,
          input_tokens_compressed: 5,
          llm_tokens_generated: 2,
          compression_ratio: 0.25,
          savings_tokens: 15,
          savings_ratio: 0.75,
          analyzer_latency_ms: 7,
          router_latency_ms: 1,
          compressor_latency_ms: 0,
          llm_latency_ms: 30,
          verifier_latency_ms: 0,
          total_slm_latency_ms: 7,
          total_pipeline_latency_ms: 40,
          selected_llm: "mock-gemini",
          router_reason: "cost-driven",
          router_estimated_cost_cents: 0,
          router_estimated_cost_microcents: 37,
        },
      },
      { type: "SWARM_FINISHED", swarm_id: "sw1", mission: "MISSION_A", latency_ms: 40, audit_chain_hash: "deadbeef", ts: TS },
    ];

    const s = reduceMissionState(events);
    expect(s.status).toBe("done");
    expect(s.swarmId).toBe("sw1");
    expect(s.mission).toBe("MISSION_A");
    expect(s.taskClass).toBe("FACTUAL_QA");
    expect(s.llmTokens).toBe("Paris");
    expect(s.llmProvider).toBe("mock-gemini");
    expect(s.robots["analyzer"].status).toBe("done");
    expect(s.robots["analyzer"].latencyMs).toBe(7);
    expect(s.robotOrder).toEqual(["analyzer"]);
    expect(s.metrics?.savings_ratio).toBe(0.75);
    expect(s.finished).toBe(true);
    expect(s.auditHash).toBe("deadbeef");
    expect(s.latencyMs).toBe(40);
    // coût : LlmFinished.cost_cents × 1000.
    expect(observedCostMicrocents(s)).toBe(1000);
  });

  it("Speculative : draft + hot path", () => {
    const events: StreamEvent[] = [
      { type: "SWARM_STARTED", swarm_id: "sp", mission: "SPECULATIVE", user_text: "x", task_class: "FACTUAL_QA", ts: TS },
      { type: "DRAFT_TOKEN", swarm_id: "sp", token: "Pa", index: 0, ts: TS },
      { type: "DRAFT_TOKEN", swarm_id: "sp", token: "ris", index: 1, ts: TS },
      { type: "SPECULATION_RESOLVED", swarm_id: "sp", tokens_accepted: 2, tokens_regenerated: 0, latency_saved_ms: 250, verifier_skipped: true, ts: TS },
      { type: "SWARM_FINISHED", swarm_id: "sp", mission: "SPECULATIVE", latency_ms: 18, audit_chain_hash: "h", ts: TS },
    ];
    const s = reduceMissionState(events);
    expect(s.draftTokens).toBe("Paris");
    expect(s.speculation?.verifier_skipped).toBe(true);
    expect(s.speculation?.latency_saved_ms).toBe(250);
    expect(s.mission).toBe("SPECULATIVE");
  });

  it("VERDICT_READY : collecté dans state.verdicts", () => {
    const events: StreamEvent[] = [
      { type: "SWARM_STARTED", swarm_id: "v", mission: "MISSION_A", user_text: "x", task_class: "FACTUAL_QA", ts: TS },
      { type: "VERDICT_READY", swarm_id: "v", claim: "La Terre est ronde", verdict: "SUPPORTED", severity: "INFO", best_source_id: "src-1", similarity: 0.92, ts: TS },
      { type: "VERDICT_READY", swarm_id: "v", claim: "Le ciel est vert", verdict: "CONTRADICTED", severity: "WARN", best_source_id: null, similarity: 0.12, ts: TS },
      { type: "SWARM_FINISHED", swarm_id: "v", mission: "MISSION_A", latency_ms: 10, audit_chain_hash: "h", ts: TS },
    ];
    const s = reduceMissionState(events);
    expect(s.verdicts).toHaveLength(2);
    expect(s.verdicts[0].verdict).toBe("SUPPORTED");
    expect(s.verdicts[1].verdict).toBe("CONTRADICTED");
    expect(s.verdicts[0].similarity).toBeCloseTo(0.92);
  });

  it("Mission B : vote", () => {
    const events: StreamEvent[] = [
      { type: "SWARM_STARTED", swarm_id: "b", mission: "MISSION_B", user_text: "x", task_class: "FACTUAL_QA", ts: TS },
      { type: "VOTE_RESOLVED", swarm_id: "b", winner: "mock-claude", winner_score: 2.5, per_llm_scores: { "mock-claude": 2.5, "mock-gpt": 1.0 }, ts: TS },
      { type: "SWARM_FINISHED", swarm_id: "b", mission: "MISSION_B", latency_ms: 22, audit_chain_hash: "h", ts: TS },
    ];
    const s = reduceMissionState(events);
    expect(s.vote?.winner).toBe("mock-claude");
    expect(s.vote?.winner_score).toBe(2.5);
  });

  it("dégradations → overlays + robot en échec", () => {
    const events: StreamEvent[] = [
      { type: "SWARM_STARTED", swarm_id: "d", mission: "MISSION_A", user_text: "x", task_class: "FACTUAL_QA", ts: TS },
      { type: "SLM_CIRCUIT_OPENED", swarm_id: "d", robot: "analyzer", failures: 3, ts: TS },
      { type: "LLM_FALLBACK_TRIGGERED", swarm_id: "d", from: "claude", to: "gemini", reason: "429", ts: TS },
      { type: "DRAFT_REJECTED_AS_NOISE", swarm_id: "d", signal: "ONLY_STOPWORDS", draft_excerpt: "La", ts: TS },
      { type: "SWARM_FAILED", swarm_id: "d", mission: "MISSION_A", stage: "analyze", error: "circuit", ts: TS },
    ];
    const s = reduceMissionState(events);
    expect(s.status).toBe("failed");
    expect(s.failed?.stage).toBe("analyze");
    expect(s.robots["analyzer"].status).toBe("failed");
    expect(s.overlays.map((o) => o.kind)).toEqual(["circuit", "fallback", "noise"]);
  });

  it("ne mute pas l'entrée", () => {
    const events: StreamEvent[] = [
      { type: "SWARM_STARTED", swarm_id: "x", mission: "MISSION_A", user_text: "x", task_class: "FACTUAL_QA", ts: TS },
    ];
    const snapshot = JSON.stringify(events);
    reduceMissionState(events);
    expect(JSON.stringify(events)).toBe(snapshot);
  });
});
