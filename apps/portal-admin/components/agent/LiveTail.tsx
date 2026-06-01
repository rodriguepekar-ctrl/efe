"use client";

import { useEffect, useRef, useState } from "react";

import type { AgentStep } from "@/lib/agent/api";

interface Props {
  runId: string;
  initialSteps: AgentStep[];
}

const POLL_INTERVAL_MS = 2000;
const SSE_RETRY_BACKOFF_MS = 3000;

/**
 * M26 LiveTail — primary transport is SSE
 * (`GET /api/agent/runs/[id]/steps`). On `error` the component falls
 * back to polling `tail/route.ts` until the run reaches a terminal
 * state.
 */
export default function LiveTail({ runId, initialSteps }: Props) {
  const [steps, setSteps] = useState<AgentStep[]>(initialSteps);
  const [connected, setConnected] = useState(false);
  const [terminal, setTerminal] = useState<string | null>(null);
  const [paused, setPaused] = useState(false);
  const seenIds = useRef(new Set(initialSteps.map((s) => s.id)));
  const lastIndex = useRef(
    initialSteps.length > 0
      ? Math.max(...initialSteps.map((s) => s.step_index))
      : -1,
  );

  const append = (incoming: AgentStep[]) => {
    setSteps((prev) => {
      const fresh = incoming.filter((s) => !seenIds.current.has(s.id));
      fresh.forEach((s) => {
        seenIds.current.add(s.id);
        if (s.step_index > lastIndex.current) {
          lastIndex.current = s.step_index;
        }
      });
      return [...prev, ...fresh];
    });
  };

  useEffect(() => {
    if (terminal || paused) return;

    let es: EventSource | null = null;
    let pollTimer: ReturnType<typeof setInterval> | null = null;
    let cancelled = false;

    const tryPoll = async () => {
      try {
        const r = await fetch(
          `/api/agent/runs/${runId}/tail?since=${lastIndex.current}`,
          { cache: "no-store" },
        );
        if (!r.ok) return;
        const data = await r.json();
        if (Array.isArray(data.steps)) append(data.steps);
        if (data.terminal) {
          setTerminal(data.terminal_status ?? "TERMINAL");
        }
      } catch {
        /* swallow ; retry next tick */
      }
    };

    const startPolling = () => {
      if (pollTimer) return;
      pollTimer = setInterval(tryPoll, POLL_INTERVAL_MS);
      void tryPoll();
    };

    const startSse = () => {
      es = new EventSource(`/api/agent/runs/${runId}/steps`);
      es.onopen = () => setConnected(true);
      es.addEventListener("agent_step", (ev) => {
        try {
          const step = JSON.parse((ev as MessageEvent).data) as AgentStep;
          append([step]);
        } catch {
          /* malformed event ; ignore */
        }
      });
      es.addEventListener("run_terminal", (ev) => {
        try {
          const data = JSON.parse((ev as MessageEvent).data) as { status: string };
          setTerminal(data.status);
        } catch {
          setTerminal("TERMINAL");
        }
        es?.close();
      });
      es.onerror = () => {
        setConnected(false);
        es?.close();
        if (!cancelled && !terminal) {
          setTimeout(() => {
            if (!cancelled && !terminal) startPolling();
          }, SSE_RETRY_BACKOFF_MS);
        }
      };
    };

    startSse();

    return () => {
      cancelled = true;
      es?.close();
      if (pollTimer) clearInterval(pollTimer);
    };
  }, [runId, terminal, paused]);

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-3 text-xs">
        <span
          className={
            connected
              ? "text-emerald-700"
              : terminal
                ? "text-slate-500"
                : "text-amber-700"
          }
        >
          {connected ? "● Live (SSE)" : terminal ? "● Closed" : "● Reconnecting…"}
        </span>
        {terminal && (
          <span className="text-slate-700">
            → Run reached <strong>{terminal}</strong>
          </span>
        )}
        {!terminal && (
          <button
            onClick={() => setPaused((p) => !p)}
            className="px-2 py-0.5 rounded border text-xs hover:bg-slate-50"
          >
            {paused ? "Resume" : "Pause"}
          </button>
        )}
        <span className="text-slate-500 ml-auto">{steps.length} step(s)</span>
      </div>
      <ul className="font-mono text-xs space-y-1 max-h-[60vh] overflow-y-auto border rounded p-2">
        {steps.map((s) => (
          <li key={s.id} className="border-b py-1 last:border-0">
            <span className="text-slate-400">
              [{String(s.step_index).padStart(3, "0")}]
            </span>{" "}
            <span className="font-semibold">{s.kind}</span>{" "}
            <span className="text-slate-500">cost={s.cost_cents}c</span>{" "}
            <span className="text-slate-400">
              {new Date(s.created_at).toLocaleTimeString()}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
