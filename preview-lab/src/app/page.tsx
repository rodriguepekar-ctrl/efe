// SPDX-License-Identifier: UNLICENSED
"use client";

import { useEffect, useState, type ReactNode } from "react";

import { BlackboardPanel } from "@/components/BlackboardPanel";
import { MissionComposer } from "@/components/MissionComposer";
import { OverlayToasts } from "@/components/OverlayToasts";
import { Pipeline } from "@/components/Pipeline";
import { SavingsGauge } from "@/components/SavingsGauge";
import { TokenStream } from "@/components/TokenStream";
import { Verdicts } from "@/components/Verdicts";
import { checkSchema, type SchemaCheck } from "@/lib/api";
import type { MissionState } from "@/lib/reduce";
import { useMissionStream, type StreamStatus } from "@/lib/useMissionStream";

export default function LiveMissionPage() {
  const { state, status, error, start, reset } = useMissionStream();
  const [schema, setSchema] = useState<SchemaCheck | null>(null);

  useEffect(() => {
    let cancelled = false;
    checkSchema().then((r) => {
      if (!cancelled) setSchema(r);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const busy = status === "connecting" || status === "streaming";

  return (
    <div className="flex flex-col gap-4">
      <div>
        <h1 className="text-xl font-bold text-efe-text">Live Mission</h1>
        <p className="text-sm text-efe-muted">
          Tape une requête, regarde le pré-mâchage se dérouler en direct (SSE).
        </p>
      </div>

      {schema && !schema.ok && <Banner tone="warn">Schéma : {schema.message}</Banner>}
      {error && <Banner tone="error">Erreur : {error}</Banner>}

      <MissionComposer disabled={busy} onRun={start} onReset={reset} />

      <StatusBar state={state} status={status} />

      <OverlayToasts state={state} />
      <Pipeline state={state} />

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <TokenStream state={state} status={status} />
        <SavingsGauge state={state} />
      </div>

      <Verdicts state={state} />
      <BlackboardPanel state={state} />
    </div>
  );
}

function StatusBar({ state, status }: { state: MissionState; status: StreamStatus }) {
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1 rounded-lg border border-efe-border bg-efe-bg px-3 py-2 font-mono text-[11px] text-efe-muted">
      <span>
        statut : <span className="text-efe-text">{status}</span>
      </span>
      {state.mission && (
        <span>
          mission : <span className="text-efe-text">{state.mission}</span>
        </span>
      )}
      {state.swarmId && <span>swarm : {state.swarmId.slice(0, 8)}…</span>}
      {state.latencyMs !== null && (
        <span>
          e2e : <span className="text-efe-latency">{state.latencyMs} ms</span>
        </span>
      )}
      {state.auditHash && <span>audit : {state.auditHash.slice(0, 10)}…</span>}
      {state.failed && (
        <span className="text-efe-danger">
          échec @ {state.failed.stage} : {state.failed.error}
        </span>
      )}
    </div>
  );
}

function Banner({ tone, children }: { tone: "warn" | "error"; children: ReactNode }) {
  const c =
    tone === "error"
      ? "border-efe-danger text-efe-danger"
      : "border-efe-cost text-efe-cost";
  return (
    <div className={`rounded-lg border bg-efe-panel px-3 py-2 text-sm ${c}`}>{children}</div>
  );
}
