"use client";

import { useRouter } from "next/navigation";

type SearchParams = {
  catalog?: string;
  kind?: string;
  severity?: string;
};

export function CurationFiltersBar({
  tenantSlug, initial,
}: { tenantSlug: string; initial: SearchParams }) {
  const router = useRouter();

  const updateFilter = (key: keyof SearchParams, value: string) => {
    const params = new URLSearchParams();
    Object.entries(initial).forEach(([k, v]) => v && params.set(k, v));
    if (value) params.set(key, value); else params.delete(key);
    router.push(`/${tenantSlug}/curation/queue?${params.toString()}`);
  };

  return (
    <div className="flex gap-3 items-center bg-slate-50 p-3 rounded border border-slate-200">
      <div className="text-sm font-medium text-slate-700">Filters :</div>
      <Select
        label="Kind"
        value={initial.kind ?? ""}
        onChange={(v) => updateFilter("kind", v)}
        options={[
          { value: "", label: "All kinds" },
          { value: "SPLIT", label: "Split" },
          { value: "MERGE", label: "Merge" },
          { value: "GENERATE_SYNTH", label: "Synth" },
          { value: "EXPLAIN", label: "Explain" },
          { value: "AUDIT_VS_GOLD", label: "Audit" },
        ]}
      />
      <Select
        label="Severity"
        value={initial.severity ?? ""}
        onChange={(v) => updateFilter("severity", v)}
        options={[
          { value: "", label: "Any severity" },
          { value: "HIGH", label: "High" },
          { value: "MEDIUM", label: "Medium" },
          { value: "LOW", label: "Low" },
        ]}
      />
    </div>
  );
}

function Select({ label, value, onChange, options }: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
}) {
  return (
    <label className="flex items-center gap-2 text-sm text-slate-700">
      <span className="sr-only">{label}</span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="rounded border border-slate-300 px-2 py-1 text-sm bg-white focus:outline-none focus:ring-2 focus:ring-blue-500"
      >
        {options.map((opt) => (
          <option key={opt.value} value={opt.value}>{opt.label}</option>
        ))}
      </select>
    </label>
  );
}
