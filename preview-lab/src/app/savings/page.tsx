// SPDX-License-Identifier: UNLICENSED
"use client";

import { useCallback, useEffect, useState } from "react";

import { SavingsReportView } from "@/components/SavingsReportView";
import { fetchSavingsReport } from "@/lib/api";
import type { SavingsReport } from "@/lib/report";

type LoadState = "loading" | "ready" | "missing" | "error";

export default function SavingsPage() {
  const [report, setReport] = useState<SavingsReport | null>(null);
  const [state, setState] = useState<LoadState>("loading");
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    setState("loading");
    setError(null);
    fetchSavingsReport()
      .then((r) => {
        if (r === null) {
          setState("missing");
        } else {
          setReport(r);
          setState("ready");
        }
      })
      .catch((e) => {
        setState("error");
        setError(String(e));
      });
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-bold text-efe-text">Savings Report</h1>
          <p className="text-sm text-efe-muted">
            Le livrable R3.S9 (microcents) — savings cost enfin visibles.
          </p>
        </div>
        <button
          onClick={load}
          className="rounded-lg border border-efe-border px-3 py-1 text-sm text-efe-muted hover:text-efe-text"
        >
          Recharger
        </button>
      </div>

      {state === "loading" && <p className="text-sm text-efe-muted">Chargement…</p>}

      {state === "missing" && (
        <div className="rounded-lg border border-efe-cost bg-efe-panel px-3 py-2 text-sm text-efe-cost">
          Rapport non généré. Lance le live test R3.S9 :
          <code className="ml-1 font-mono text-efe-text">
            cargo test -p efe-coordinator --features all-providers --test live_chewing_savings_report_v2
          </code>
        </div>
      )}

      {state === "error" && (
        <div className="rounded-lg border border-efe-danger bg-efe-panel px-3 py-2 text-sm text-efe-danger">
          Erreur : {error}
        </div>
      )}

      {state === "ready" && report && <SavingsReportView report={report} />}
    </div>
  );
}
