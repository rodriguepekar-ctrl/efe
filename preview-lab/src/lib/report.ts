// SPDX-License-Identifier: UNLICENSED
//
// Miroir TS des types `SavingsReport` (R3.S9) et `ABComparison` (R3.S8) servis
// par le bridge (`GET /v1/savings-report`, `POST /v1/ab/run`).

export type RunMode = "BASELINE" | "S4_CHEWING" | "S6_ROUTING" | "FULL" | "SPECULATIVE";

export const RUN_MODES: RunMode[] = [
  "BASELINE",
  "S4_CHEWING",
  "S6_ROUTING",
  "FULL",
  "SPECULATIVE",
];

export const RUN_MODE_LABEL: Record<RunMode, string> = {
  BASELINE: "Baseline (Claude direct)",
  S4_CHEWING: "S4 · pré-mâchage",
  S6_ROUTING: "S6 · routing",
  FULL: "Full (S4+S6)",
  SPECULATIVE: "Speculative",
};

export interface ModeSummary {
  mode: RunMode;
  n_runs: number;
  total_cost_cents: number;
  total_cost_microcents: number;
  avg_cost_cents: number;
  avg_cost_microcents: number;
  avg_latency_ms: number;
  avg_tokens_output: number;
  quality_pass_rate: number;
  degradations_count: number;
}

export interface SavingsReport {
  summaries: Partial<Record<RunMode, ModeSummary>>;
  baseline_total_cost_cents: number;
  full_total_cost_cents: number;
  baseline_total_cost_microcents: number;
  full_total_cost_microcents: number;
  savings_cents: number;
  savings_microcents: number;
  savings_ratio: number;
  savings_ratio_microcents: number;
  generated_at: string;
  version: string;
}

// ── A/B (R3.S8) ─────────────────────────────────────────────────────────────

export type ABArm = "CONTROL" | "EXPERIMENT";

export interface ArmStats {
  arm: ABArm;
  table_version: string;
  n_runs: number;
  avg_cost_microcents: number;
  quality_pass_rate: number;
  avg_latency_ms: number;
}

export interface ABComparison {
  control: ArmStats;
  experiment: ArmStats;
  cost_delta_microcents: number;
  quality_delta: number;
  latency_delta_ms: number;
}

export interface AbRunResult {
  n: number;
  comparison: ABComparison;
}

export interface Weights {
  quality: number;
  latency: number;
  cost: number;
}

export interface AbRunDto {
  user_text?: string;
  n?: number;
  control_weights?: Weights;
  experiment_weights?: Weights;
}

// ── Helpers de formatage ────────────────────────────────────────────────────

export function fmtMicrocents(mc: number | null | undefined): string {
  if (mc === null || mc === undefined) return "—";
  return `${(mc / 1000).toFixed(3)} ¢`;
}

export function fmtPct(ratio: number | null | undefined): string {
  if (ratio === null || ratio === undefined) return "—";
  return `${Math.round(ratio * 100)} %`;
}
