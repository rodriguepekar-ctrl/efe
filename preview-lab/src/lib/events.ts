// SPDX-License-Identifier: UNLICENSED
//
// Miroir TypeScript du protocole `StreamEvent` du coordinator EFE
// (`crates/efe-coordinator/src/events/stream.rs`).
//
// Le Rust sérialise avec `#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]`
// → chaque event JSON a un champ discriminant `type` en SCREAMING_SNAKE_CASE, et
// ses champs internes en snake_case. `mission` et `task_class` sont eux aussi en
// SCREAMING_SNAKE_CASE.
//
// **Non-dérive** : `ALL_EVENT_TAGS` + `EVENT_SCHEMA_VERSION` sont comparés à
// `GET /v1/schema` au boot (cf. `checkSchema` dans api.ts). Si le serveur ajoute
// un variant sans qu'on mette à jour ce fichier → bannière de drift.

export const EVENT_SCHEMA_VERSION = "1.0.0" as const;

export type MissionTag = "MISSION_A" | "MISSION_B" | "SPECULATIVE";

export type TaskClass =
  | "FACTUAL_QA"
  | "CODE"
  | "CREATIVE"
  | "ANALYSIS"
  | "REASONING"
  | "TRANSLATION";

/** Métriques de pré-mâchage (miroir de `ChewingMetrics`). */
export interface ChewingMetrics {
  input_tokens_raw: number;
  input_tokens_compressed: number;
  llm_tokens_generated: number;
  compression_ratio: number;
  savings_tokens: number;
  savings_ratio: number;
  analyzer_latency_ms: number;
  router_latency_ms: number;
  compressor_latency_ms: number;
  llm_latency_ms: number;
  verifier_latency_ms: number;
  total_slm_latency_ms: number;
  total_pipeline_latency_ms: number;
  selected_llm: string;
  router_reason: string;
  router_estimated_cost_cents: number;
  router_estimated_cost_microcents?: number;
  verifier_skipped?: boolean | null;
  spec_drafter_latency_ms?: number;
  spec_verifier_latency_ms?: number;
  spec_savings_cents?: number;
  spec_savings_microcents?: number;
  spec_noise_rejected?: boolean | null;
  spec_drafter_is_local?: boolean | null;
  degradations_count?: number;
  failed_components?: string[];
  ab_arm?: "CONTROL" | "EXPERIMENT" | null;
  ab_table_version?: string | null;
}

// ── Les 19 variants ─────────────────────────────────────────────────────────

interface Base {
  ts: string; // RFC3339
}

export interface SwarmStarted extends Base {
  type: "SWARM_STARTED";
  swarm_id: string;
  mission: MissionTag;
  user_text: string;
  task_class: TaskClass;
}
export interface RobotStarted extends Base {
  type: "ROBOT_STARTED";
  swarm_id: string;
  robot_id: string;
}
export interface RobotFinished extends Base {
  type: "ROBOT_FINISHED";
  swarm_id: string;
  robot_id: string;
  success: boolean;
  latency_ms: number;
  error: string | null;
}
export interface PheromoneDeposited extends Base {
  type: "PHEROMONE_DEPOSITED";
  swarm_id: string;
  author: string;
  kind: string;
  entry_id: string;
}
export interface LlmTokenStreamed extends Base {
  type: "LLM_TOKEN_STREAMED";
  swarm_id: string;
  provider: string;
  token: string;
  index: number;
}
export interface LlmFinished extends Base {
  type: "LLM_FINISHED";
  swarm_id: string;
  provider: string;
  tokens_generated: number;
  cost_cents: number;
  latency_ms: number;
}
export interface VerdictReady extends Base {
  type: "VERDICT_READY";
  swarm_id: string;
  claim: string;
  verdict: string;
  severity: string;
  best_source_id: string | null;
  similarity: number;
}
export interface VoteResolved extends Base {
  type: "VOTE_RESOLVED";
  swarm_id: string;
  winner: string;
  winner_score: number;
  per_llm_scores: Record<string, number>;
}
export interface DraftToken extends Base {
  type: "DRAFT_TOKEN";
  swarm_id: string;
  token: string;
  index: number;
}
export interface SpeculationResolved extends Base {
  type: "SPECULATION_RESOLVED";
  swarm_id: string;
  tokens_accepted: number;
  tokens_regenerated: number;
  latency_saved_ms: number;
  verifier_skipped: boolean;
}
export interface PreChewingMetrics extends Base {
  type: "PRE_CHEWING_METRICS";
  swarm_id: string;
  metrics: ChewingMetrics;
}
export interface SwarmFinished extends Base {
  type: "SWARM_FINISHED";
  swarm_id: string;
  mission: MissionTag;
  latency_ms: number;
  audit_chain_hash: string;
}
export interface SwarmFailed extends Base {
  type: "SWARM_FAILED";
  swarm_id: string;
  mission: MissionTag;
  stage: string;
  error: string;
}
export interface Heartbeat extends Base {
  type: "HEARTBEAT";
}
export interface LlmFallbackTriggered extends Base {
  type: "LLM_FALLBACK_TRIGGERED";
  swarm_id: string;
  from: string;
  to: string;
  reason: string;
}
export interface LlmFailed extends Base {
  type: "LLM_FAILED";
  swarm_id: string;
  provider: string;
  reason: string;
}
export interface SlmCircuitOpened extends Base {
  type: "SLM_CIRCUIT_OPENED";
  swarm_id: string;
  robot: string;
  failures: number;
}
export interface DrafterBypassed extends Base {
  type: "DRAFTER_BYPASSED";
  swarm_id: string;
  drafter: string;
  fallback_to_verifier: string;
  reason: string;
}
export interface DraftRejectedAsNoise extends Base {
  type: "DRAFT_REJECTED_AS_NOISE";
  swarm_id: string;
  signal: string;
  draft_excerpt: string;
}

export type StreamEvent =
  | SwarmStarted
  | RobotStarted
  | RobotFinished
  | PheromoneDeposited
  | LlmTokenStreamed
  | LlmFinished
  | VerdictReady
  | VoteResolved
  | DraftToken
  | SpeculationResolved
  | PreChewingMetrics
  | SwarmFinished
  | SwarmFailed
  | Heartbeat
  | LlmFallbackTriggered
  | LlmFailed
  | SlmCircuitOpened
  | DrafterBypassed
  | DraftRejectedAsNoise;

export type EventTag = StreamEvent["type"];

/** Liste canonique des 19 tags (doit matcher `efe_coordinator::ALL_EVENT_TAGS`). */
export const ALL_EVENT_TAGS: readonly EventTag[] = [
  "SWARM_STARTED",
  "ROBOT_STARTED",
  "ROBOT_FINISHED",
  "PHEROMONE_DEPOSITED",
  "LLM_TOKEN_STREAMED",
  "LLM_FINISHED",
  "VERDICT_READY",
  "VOTE_RESOLVED",
  "DRAFT_TOKEN",
  "SPECULATION_RESOLVED",
  "PRE_CHEWING_METRICS",
  "SWARM_FINISHED",
  "SWARM_FAILED",
  "HEARTBEAT",
  "LLM_FALLBACK_TRIGGERED",
  "LLM_FAILED",
  "SLM_CIRCUIT_OPENED",
  "DRAFTER_BYPASSED",
  "DRAFT_REJECTED_AS_NOISE",
] as const;

/** Events qui terminent un swarm (le stream finit après l'un d'eux). */
export const TERMINAL_TAGS: readonly EventTag[] = ["SWARM_FINISHED", "SWARM_FAILED"];

export function isTerminal(ev: StreamEvent): boolean {
  return TERMINAL_TAGS.includes(ev.type);
}

/** `swarm_id` de l'event (null pour Heartbeat). */
export function swarmIdOf(ev: StreamEvent): string | null {
  return ev.type === "HEARTBEAT" ? null : ev.swarm_id;
}

// ── DTOs de requête (miroir des DTOs du bridge `efe-preview-server`) ─────────

export type MissionType = "a" | "b" | "speculative";

export interface MissionADto {
  user_text: string;
  task_class?: TaskClass;
  available_llms?: string[];
}
export interface MissionBDto {
  user_text: string;
  task_class?: TaskClass;
  llms?: string[];
  vote_weights?: Record<string, number>;
  quorum?: number;
}
export interface SpecDto {
  user_text: string;
  task_class?: TaskClass;
  drafter_llm: string;
  verifier_llm: string;
}

export type MissionDto = MissionADto | MissionBDto | SpecDto;

/** Scénarios de la Resilience Theater (P4.S6). */
export type ResilienceScenario = "fallback" | "drafter_bypass" | "noise_rejection";

/** Réponse de `GET /v1/schema`. */
export interface SchemaResponse {
  event_schema_version: string;
  variants: string[];
}
