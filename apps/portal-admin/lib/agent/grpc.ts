// Mirror types of the M25 protobuf payloads, for typed access in
// React components. Kept in sync with `proto/efe/agent_runtime/v1/`.

export type AgentRunStatus =
  | "AGENT_RUN_STATUS_PENDING"
  | "AGENT_RUN_STATUS_PROVISIONING"
  | "AGENT_RUN_STATUS_RUNNING"
  | "AGENT_RUN_STATUS_AWAITING_APPROVAL"
  | "AGENT_RUN_STATUS_PAUSED"
  | "AGENT_RUN_STATUS_RESUMING"
  | "AGENT_RUN_STATUS_COMPLETED"
  | "AGENT_RUN_STATUS_FAILED"
  | "AGENT_RUN_STATUS_TERMINATED"
  | "AGENT_RUN_STATUS_TIMED_OUT"
  | "AGENT_RUN_STATUS_COST_CAPPED";

export type ApprovalStatus =
  | "APPROVAL_STATUS_PENDING"
  | "APPROVAL_STATUS_GRANTED"
  | "APPROVAL_STATUS_REJECTED"
  | "APPROVAL_STATUS_EXPIRED"
  | "APPROVAL_STATUS_CONSUMED";

export const STATUS_BADGE: Record<string, string> = {
  PENDING: "bg-slate-200 text-slate-700",
  PROVISIONING: "bg-amber-100 text-amber-700",
  RUNNING: "bg-emerald-100 text-emerald-700",
  AWAITING_APPROVAL: "bg-orange-100 text-orange-700",
  PAUSED: "bg-yellow-100 text-yellow-700",
  RESUMING: "bg-amber-100 text-amber-700",
  COMPLETED: "bg-blue-100 text-blue-700",
  FAILED: "bg-red-100 text-red-700",
  TERMINATED: "bg-rose-100 text-rose-700",
  TIMED_OUT: "bg-fuchsia-100 text-fuchsia-700",
  COST_CAPPED: "bg-pink-100 text-pink-700",
};

export function isTerminal(status: string): boolean {
  return ["COMPLETED", "FAILED", "TERMINATED", "TIMED_OUT", "COST_CAPPED"].includes(status);
}

export function isActive(status: string): boolean { return !isTerminal(status); }
