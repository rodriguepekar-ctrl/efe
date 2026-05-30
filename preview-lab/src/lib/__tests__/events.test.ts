// SPDX-License-Identifier: UNLICENSED
//
// T7 — Contrat des types/tags. Garantit que la liste TS `ALL_EVENT_TAGS` est
// cohérente (19 tags uniques, version 1.0.0) et que l'union discriminée couvre
// exactement ces tags. (Le garde-fou *runtime* vs le serveur est `checkSchema`,
// exercé au boot de la page ; ici on valide la cohérence interne du client.)

import { describe, expect, it } from "vitest";

import {
  ALL_EVENT_TAGS,
  EVENT_SCHEMA_VERSION,
  isTerminal,
  swarmIdOf,
  type EventTag,
  type StreamEvent,
} from "../events";

describe("event tag contract (T7)", () => {
  it("liste 19 tags, version 1.0.0", () => {
    expect(EVENT_SCHEMA_VERSION).toBe("1.0.0");
    expect(ALL_EVENT_TAGS).toHaveLength(19);
  });

  it("aucun tag dupliqué", () => {
    const set = new Set(ALL_EVENT_TAGS);
    expect(set.size).toBe(ALL_EVENT_TAGS.length);
  });

  it("contient les tags clés du pipeline", () => {
    for (const t of [
      "SWARM_STARTED",
      "ROBOT_FINISHED",
      "LLM_TOKEN_STREAMED",
      "PRE_CHEWING_METRICS",
      "SWARM_FINISHED",
      "DRAFT_REJECTED_AS_NOISE",
    ] as EventTag[]) {
      expect(ALL_EVENT_TAGS).toContain(t);
    }
  });

  it("un échantillon de chaque tag est typable et a un `type` listé", () => {
    // Force la cohérence union ↔ ALL_EVENT_TAGS : chaque membre construit doit
    // avoir un `type` présent dans la liste canonique.
    const samples: StreamEvent[] = [
      { type: "SWARM_STARTED", swarm_id: "s", mission: "MISSION_A", user_text: "x", task_class: "FACTUAL_QA", ts: "t" },
      { type: "ROBOT_STARTED", swarm_id: "s", robot_id: "analyzer", ts: "t" },
      { type: "ROBOT_FINISHED", swarm_id: "s", robot_id: "analyzer", success: true, latency_ms: 5, error: null, ts: "t" },
      { type: "PHEROMONE_DEPOSITED", swarm_id: "s", author: "a", kind: "ENTITY", entry_id: "e", ts: "t" },
      { type: "LLM_TOKEN_STREAMED", swarm_id: "s", provider: "p", token: "Pa", index: 0, ts: "t" },
      { type: "LLM_FINISHED", swarm_id: "s", provider: "p", tokens_generated: 3, cost_cents: 0, latency_ms: 10, ts: "t" },
      { type: "VERDICT_READY", swarm_id: "s", claim: "c", verdict: "SUPPORTED", severity: "INFO", best_source_id: null, similarity: 0.9, ts: "t" },
      { type: "VOTE_RESOLVED", swarm_id: "s", winner: "p", winner_score: 1, per_llm_scores: { p: 1 }, ts: "t" },
      { type: "DRAFT_TOKEN", swarm_id: "s", token: "Pa", index: 0, ts: "t" },
      { type: "SPECULATION_RESOLVED", swarm_id: "s", tokens_accepted: 3, tokens_regenerated: 0, latency_saved_ms: 100, verifier_skipped: true, ts: "t" },
      { type: "PRE_CHEWING_METRICS", swarm_id: "s", metrics: metricsStub(), ts: "t" },
      { type: "SWARM_FINISHED", swarm_id: "s", mission: "MISSION_A", latency_ms: 12, audit_chain_hash: "h", ts: "t" },
      { type: "SWARM_FAILED", swarm_id: "s", mission: "MISSION_A", stage: "vote", error: "boom", ts: "t" },
      { type: "HEARTBEAT", ts: "t" },
      { type: "LLM_FALLBACK_TRIGGERED", swarm_id: "s", from: "a", to: "b", reason: "429", ts: "t" },
      { type: "LLM_FAILED", swarm_id: "s", provider: "p", reason: "429", ts: "t" },
      { type: "SLM_CIRCUIT_OPENED", swarm_id: "s", robot: "analyzer", failures: 3, ts: "t" },
      { type: "DRAFTER_BYPASSED", swarm_id: "s", drafter: "d", fallback_to_verifier: "v", reason: "fail", ts: "t" },
      { type: "DRAFT_REJECTED_AS_NOISE", swarm_id: "s", signal: "ONLY_STOPWORDS", draft_excerpt: "La", ts: "t" },
    ];
    expect(samples).toHaveLength(19);
    const tags = new Set(samples.map((e) => e.type));
    expect([...tags].sort()).toEqual([...ALL_EVENT_TAGS].sort());
  });

  it("isTerminal + swarmIdOf", () => {
    const fin: StreamEvent = { type: "SWARM_FINISHED", swarm_id: "s", mission: "MISSION_A", latency_ms: 1, audit_chain_hash: "h", ts: "t" };
    const hb: StreamEvent = { type: "HEARTBEAT", ts: "t" };
    expect(isTerminal(fin)).toBe(true);
    expect(isTerminal(hb)).toBe(false);
    expect(swarmIdOf(fin)).toBe("s");
    expect(swarmIdOf(hb)).toBeNull();
  });
});

function metricsStub() {
  return {
    input_tokens_raw: 20,
    input_tokens_compressed: 5,
    llm_tokens_generated: 3,
    compression_ratio: 0.25,
    savings_tokens: 15,
    savings_ratio: 0.75,
    analyzer_latency_ms: 1,
    router_latency_ms: 1,
    compressor_latency_ms: 1,
    llm_latency_ms: 10,
    verifier_latency_ms: 0,
    total_slm_latency_ms: 2,
    total_pipeline_latency_ms: 13,
    selected_llm: "mock-gemini",
    router_reason: "cost-driven",
    router_estimated_cost_cents: 0,
    router_estimated_cost_microcents: 37,
  };
}
