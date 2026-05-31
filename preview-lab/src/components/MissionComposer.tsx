// SPDX-License-Identifier: UNLICENSED
"use client";

import { useState, type FormEvent } from "react";

import type { MissionDto, MissionType, TaskClass } from "@/lib/events";
import { DEFAULT_PROMPT, EXAMPLE_PROMPTS, PRECHEW_HINT_CHARS } from "@/lib/examples";
import { parseSources } from "@/lib/sources";

const TASK_CLASSES: TaskClass[] = [
  "FACTUAL_QA",
  "CODE",
  "CREATIVE",
  "ANALYSIS",
  "REASONING",
  "TRANSLATION",
];

function parseCsv(s: string): string[] {
  return s
    .split(",")
    .map((x) => x.trim())
    .filter((x) => x.length > 0);
}

export function MissionComposer({
  disabled,
  onRun,
  onReset,
}: {
  disabled: boolean;
  onRun: (type: MissionType, dto: MissionDto) => void;
  onReset: () => void;
}) {
  const [type, setType] = useState<MissionType>("a");
  const [userText, setUserText] = useState(DEFAULT_PROMPT);
  const [taskClass, setTaskClass] = useState<TaskClass>("FACTUAL_QA");
  const [llmsCsv, setLlmsCsv] = useState("");
  const [drafter, setDrafter] = useState("mock-gemini");
  const [verifier, setVerifier] = useState("mock-claude");
  const [sourcesText, setSourcesText] = useState("");

  const canRun = !disabled && userText.trim().length > 0;

  function buildDto(): MissionDto {
    const base = { user_text: userText.trim(), task_class: taskClass };
    if (type === "speculative") {
      return { ...base, drafter_llm: drafter.trim(), verifier_llm: verifier.trim() };
    }
    const llms = parseCsv(llmsCsv);
    // P5.5 — sources de fact-check (A et B) : présentes → réveillent le verifier.
    const sources = parseSources(sourcesText);
    const srcPart = sources.length > 0 ? { candidate_sources: sources } : {};
    if (type === "b") {
      return llms.length > 0 ? { ...base, ...srcPart, llms } : { ...base, ...srcPart };
    }
    return llms.length > 0
      ? { ...base, ...srcPart, available_llms: llms }
      : { ...base, ...srcPart };
  }

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!canRun) return;
    onRun(type, buildDto());
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-3 rounded-xl border border-efe-border bg-efe-panel p-4">
      <div className="flex flex-wrap items-center gap-2">
        <label className="text-sm font-semibold text-efe-text">Mission</label>
        <div className="flex overflow-hidden rounded-lg border border-efe-border">
          {(["a", "b", "speculative"] as MissionType[]).map((t) => (
            <button
              key={t}
              type="button"
              onClick={() => setType(t)}
              className={`px-3 py-1 text-sm ${
                type === t ? "bg-efe-accent/20 text-efe-accent" : "text-efe-muted hover:text-efe-text"
              }`}
            >
              {t === "a" ? "A · pré-mâchage" : t === "b" ? "B · vote" : "Speculative"}
            </button>
          ))}
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <span className="text-[11px] text-efe-muted">Exemples</span>
        {EXAMPLE_PROMPTS.map((ex) => (
          <button
            key={ex.id}
            type="button"
            onClick={() => {
              setUserText(ex.text);
              setSourcesText(ex.sources ?? "");
            }}
            disabled={disabled}
            className="rounded-lg border border-efe-border px-2 py-1 text-[12px] text-efe-muted hover:text-efe-text disabled:cursor-not-allowed disabled:opacity-40"
          >
            {ex.label}
          </button>
        ))}
      </div>

      <textarea
        value={userText}
        onChange={(e) => setUserText(e.target.value)}
        rows={3}
        className="w-full resize-y rounded-lg border border-efe-border bg-efe-bg px-3 py-2 font-mono text-sm text-efe-text outline-none focus:border-efe-accent"
        placeholder="Texte de la requête…"
      />

      <div className="flex justify-end">
        <span
          className={`text-[11px] ${
            userText.length >= PRECHEW_HINT_CHARS ? "text-efe-accent" : "text-efe-muted"
          }`}
        >
          {userText.length} caractères
          {userText.length >= PRECHEW_HINT_CHARS ? " · pré-mâchage visible" : ""}
        </span>
      </div>

      <div className="flex flex-wrap items-center gap-3">
        <label className="text-[12px] text-efe-muted">task_class</label>
        <select
          value={taskClass}
          onChange={(e) => setTaskClass(e.target.value as TaskClass)}
          className="rounded-lg border border-efe-border bg-efe-bg px-2 py-1 text-sm text-efe-text"
        >
          {TASK_CLASSES.map((tc) => (
            <option key={tc} value={tc}>
              {tc}
            </option>
          ))}
        </select>
      </div>

      {type === "speculative" ? (
        <div className="flex flex-wrap gap-2">
          <Field label="drafter_llm" value={drafter} onChange={setDrafter} />
          <Field label="verifier_llm" value={verifier} onChange={setVerifier} />
        </div>
      ) : (
        <Field
          label={type === "b" ? "llms (CSV, ≥2 ; vide = défauts)" : "available_llms (CSV ; vide = défauts)"}
          value={llmsCsv}
          onChange={setLlmsCsv}
          placeholder="mock-claude, mock-gemini, mock-gpt"
        />
      )}

      {type !== "speculative" && (
        <label className="flex flex-col gap-1">
          <span className="text-[11px] text-efe-muted">
            candidate_sources — une par ligne (active le fact-check / verdicts)
          </span>
          <textarea
            value={sourcesText}
            onChange={(e) => setSourcesText(e.target.value)}
            rows={2}
            placeholder={"Paris est la capitale de la France.\n…"}
            className="w-full resize-y rounded-lg border border-efe-border bg-efe-bg px-3 py-2 font-mono text-xs text-efe-text outline-none focus:border-efe-accent"
          />
        </label>
      )}

      <div className="flex gap-2">
        <button
          type="submit"
          disabled={!canRun}
          className="rounded-lg bg-efe-accent px-4 py-1.5 text-sm font-semibold text-efe-bg disabled:cursor-not-allowed disabled:opacity-40"
        >
          {disabled ? "En cours…" : "Lancer"}
        </button>
        <button
          type="button"
          onClick={onReset}
          className="rounded-lg border border-efe-border px-4 py-1.5 text-sm text-efe-muted hover:text-efe-text"
        >
          Réinitialiser
        </button>
      </div>
    </form>
  );
}

function Field({
  label,
  value,
  onChange,
  placeholder,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
}) {
  return (
    <label className="flex flex-1 flex-col gap-1">
      <span className="text-[11px] text-efe-muted">{label}</span>
      <input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="rounded-lg border border-efe-border bg-efe-bg px-2 py-1 font-mono text-sm text-efe-text outline-none focus:border-efe-accent"
      />
    </label>
  );
}
