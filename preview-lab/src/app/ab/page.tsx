// SPDX-License-Identifier: UNLICENSED
"use client";

import { useState } from "react";

import { AbComparisonView } from "@/components/AbComparison";
import { runAbTest } from "@/lib/api";
import type { AbRunResult, Weights } from "@/lib/report";

export default function AbPage() {
  const [n, setN] = useState(16);
  const [control, setControl] = useState<Weights>({ quality: 5, latency: 0, cost: 0 });
  const [experiment, setExperiment] = useState<Weights>({ quality: 1, latency: 0, cost: 5 });
  const [result, setResult] = useState<AbRunResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run() {
    setBusy(true);
    setError(null);
    try {
      const r = await runAbTest({
        n,
        control_weights: control,
        experiment_weights: experiment,
      });
      setResult(r);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div>
        <h1 className="text-xl font-bold text-efe-text">A/B Lab</h1>
        <p className="text-sm text-efe-muted">
          Teste deux tables de scoring en prod sans risque : le trafic est routé
          50/50, et on compare empiriquement control vs experiment.
        </p>
      </div>

      <div className="flex flex-col gap-3 rounded-xl border border-efe-border bg-efe-panel p-4">
        <div className="flex flex-wrap items-end gap-4">
          <label className="flex flex-col gap-1">
            <span className="text-[11px] text-efe-muted">missions (n)</span>
            <input
              type="number"
              min={2}
              max={64}
              value={n}
              onChange={(e) => setN(Math.max(2, Math.min(64, Number(e.target.value) || 2)))}
              className="w-24 rounded-lg border border-efe-border bg-efe-bg px-2 py-1 font-mono text-sm text-efe-text"
            />
          </label>
          <button
            onClick={run}
            disabled={busy}
            className="rounded-lg bg-efe-accent px-4 py-1.5 text-sm font-semibold text-efe-bg disabled:opacity-40"
          >
            {busy ? "Run en cours…" : "Lancer l'A/B"}
          </button>
        </div>

        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <WeightCard title="Control (table v1)" w={control} onChange={setControl} />
          <WeightCard title="Experiment (table v2)" w={experiment} onChange={setExperiment} />
        </div>
      </div>

      {error && (
        <div className="rounded-lg border border-efe-danger bg-efe-panel px-3 py-2 text-sm text-efe-danger">
          Erreur : {error}
        </div>
      )}

      {result && (
        <div className="flex flex-col gap-2">
          <div className="font-mono text-[11px] text-efe-muted">{result.n} missions lancées</div>
          <AbComparisonView cmp={result.comparison} />
        </div>
      )}
    </div>
  );
}

function WeightCard({
  title,
  w,
  onChange,
}: {
  title: string;
  w: Weights;
  onChange: (w: Weights) => void;
}) {
  const set = (k: keyof Weights) => (v: number) => onChange({ ...w, [k]: v });
  return (
    <div className="flex flex-col gap-2 rounded-lg border border-efe-border bg-efe-bg p-3">
      <span className="text-sm font-semibold text-efe-text">{title}</span>
      <div className="flex gap-2">
        <Num label="quality" value={w.quality} onChange={set("quality")} />
        <Num label="latency" value={w.latency} onChange={set("latency")} />
        <Num label="cost" value={w.cost} onChange={set("cost")} />
      </div>
    </div>
  );
}

function Num({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (v: number) => void;
}) {
  return (
    <label className="flex flex-1 flex-col gap-1">
      <span className="text-[11px] text-efe-muted">{label}</span>
      <input
        type="number"
        step={0.5}
        min={0}
        value={value}
        onChange={(e) => onChange(Math.max(0, Number(e.target.value) || 0))}
        className="w-full rounded-lg border border-efe-border bg-efe-panel px-2 py-1 font-mono text-sm text-efe-text"
      />
    </label>
  );
}
