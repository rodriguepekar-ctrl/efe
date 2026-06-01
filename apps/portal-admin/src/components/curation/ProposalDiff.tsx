"use client";

import { useMemo } from "react";

type ClassChange = {
  type: "Added" | "Removed" | "Renamed" | "Split" | "Merged" | "AnchorsChanged";
  class_id?: string;
  old_id?: string;
  new_id?: string;
  from?: string | string[];
  to?: string | string[];
  added?: string[];
  removed?: string[];
  cosine_centroid_shift?: number;
  anchors_count?: number;
};

type DiffJson = {
  from_semver?: string;
  to_semver?: string;
  class_changes?: ClassChange[];
  estimated_impact?: {
    affected_examples_count: number;
    estimated_accuracy_delta_pp: number;
    re_label_required: number;
    confusion_matrix_delta_norm: number;
  };
};

const CHANGE_COLORS: Record<ClassChange["type"], string> = {
  Added:           "bg-emerald-50 border-emerald-200 text-emerald-900",
  Removed:         "bg-red-50 border-red-200 text-red-900",
  Renamed:         "bg-blue-50 border-blue-200 text-blue-900",
  Split:           "bg-purple-50 border-purple-200 text-purple-900",
  Merged:          "bg-amber-50 border-amber-200 text-amber-900",
  AnchorsChanged:  "bg-slate-50 border-slate-200 text-slate-900",
};

export function ProposalDiff({ diffJson }: { diffJson: DiffJson }) {
  const changes = diffJson.class_changes ?? [];
  const grouped = useMemo(() => {
    return {
      added:   changes.filter((c) => c.type === "Added"),
      removed: changes.filter((c) => c.type === "Removed"),
      renamed: changes.filter((c) => c.type === "Renamed"),
      split:   changes.filter((c) => c.type === "Split"),
      merged:  changes.filter((c) => c.type === "Merged"),
      anchors: changes.filter((c) => c.type === "AnchorsChanged"),
    };
  }, [changes]);

  if (changes.length === 0) {
    return (
      <div className="text-sm text-slate-500 italic">
        No structural changes detected. (This may indicate a no-op proposal.)
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {diffJson.from_semver && diffJson.to_semver && (
        <div className="text-sm text-slate-600">
          <code className="text-slate-900">{diffJson.from_semver}</code>
          <span className="mx-2">→</span>
          <code className="text-slate-900">{diffJson.to_semver}</code>
        </div>
      )}

      <div className="grid grid-cols-2 md:grid-cols-3 gap-2 text-xs">
        <Stat label="Added" n={grouped.added.length} />
        <Stat label="Removed" n={grouped.removed.length} />
        <Stat label="Renamed" n={grouped.renamed.length} />
        <Stat label="Split" n={grouped.split.length} />
        <Stat label="Merged" n={grouped.merged.length} />
        <Stat label="Anchors changed" n={grouped.anchors.length} />
      </div>

      <ul className="space-y-2">
        {changes.map((c, i) => (
          <li key={i} className={`rounded border px-3 py-2 ${CHANGE_COLORS[c.type]}`}>
            <ChangeLine c={c} />
          </li>
        ))}
      </ul>

      {diffJson.estimated_impact && (
        <div className="mt-4 rounded bg-slate-50 border border-slate-200 p-3 text-sm">
          <div className="font-medium text-slate-700 mb-1">Estimated impact</div>
          <div className="grid grid-cols-2 gap-y-1 text-xs text-slate-700">
            <span>Affected examples</span>
            <span className="text-right tabular-nums">{diffJson.estimated_impact.affected_examples_count.toLocaleString()}</span>
            <span>Accuracy delta</span>
            <span className="text-right tabular-nums">
              {diffJson.estimated_impact.estimated_accuracy_delta_pp >= 0 ? "+" : ""}
              {diffJson.estimated_impact.estimated_accuracy_delta_pp.toFixed(2)} pp
            </span>
            <span>Re-label required</span>
            <span className="text-right tabular-nums">{diffJson.estimated_impact.re_label_required.toLocaleString()}</span>
            <span>Confusion delta norm</span>
            <span className="text-right tabular-nums">{diffJson.estimated_impact.confusion_matrix_delta_norm.toFixed(3)}</span>
          </div>
        </div>
      )}
    </div>
  );
}

function Stat({ label, n }: { label: string; n: number }) {
  return (
    <div className="rounded bg-white border border-slate-200 px-2 py-1.5">
      <div className="text-slate-500">{label}</div>
      <div className="text-slate-900 font-semibold tabular-nums">{n}</div>
    </div>
  );
}

function ChangeLine({ c }: { c: ClassChange }) {
  switch (c.type) {
    case "Added":          return <><strong>+ {c.class_id}</strong> ({c.anchors_count ?? "?"} anchors)</>;
    case "Removed":        return <><strong>− {c.class_id}</strong></>;
    case "Renamed":        return <><strong>{c.old_id}</strong> → <strong>{c.new_id}</strong></>;
    case "Split":          return <><strong>{Array.isArray(c.from) ? c.from.join(", ") : c.from}</strong> ⇒ {Array.isArray(c.to) ? c.to.join(" + ") : c.to}</>;
    case "Merged":         return <><strong>{Array.isArray(c.from) ? c.from.join(" + ") : c.from}</strong> ⇒ {Array.isArray(c.to) ? c.to.join(", ") : c.to}</>;
    case "AnchorsChanged": return (
      <>
        <strong>{c.class_id}</strong> :
        <span className="ml-2">{c.added?.length ?? 0} added</span>
        <span className="ml-2">{c.removed?.length ?? 0} removed</span>
        {typeof c.cosine_centroid_shift === "number" && (
          <span className="ml-2 text-xs">(centroid shift {c.cosine_centroid_shift.toFixed(3)})</span>
        )}
      </>
    );
    default: return <span>(unknown change)</span>;
  }
}
