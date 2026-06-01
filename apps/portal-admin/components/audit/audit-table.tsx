import { formatLatencyMs, formatTimestamp } from "@/lib/utils/format";
import Link from "next/link";

interface Row {
  audit_id: number;
  ts: string;
  tenant_id: string;
  mode: string;
  sentence_hash: string;
  label?: string | null;
  confidence?: number | null;
  duration_ms: number;
  converged?: boolean | null;
}

export function AuditTable({ rows }: { rows: Row[] }) {
  if (rows.length === 0) {
    return (
      <div className="rounded-md border bg-muted/30 p-8 text-center text-sm text-muted-foreground">
        No audit rows match the current filters.
      </div>
    );
  }

  return (
    <div className="overflow-x-auto rounded-md border">
      <table className="w-full text-sm">
        <thead className="bg-muted/50 text-xs uppercase tracking-wide">
          <tr>
            <th className="px-3 py-2 text-left">ID</th>
            <th className="px-3 py-2 text-left">Timestamp</th>
            <th className="px-3 py-2 text-left">Mode</th>
            <th className="px-3 py-2 text-left">Sentence</th>
            <th className="px-3 py-2 text-left">Label</th>
            <th className="px-3 py-2 text-left">Conf</th>
            <th className="px-3 py-2 text-right">Duration</th>
            <th className="px-3 py-2 text-center">Converged</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.audit_id} className="border-t hover:bg-accent/30">
              <td className="px-3 py-2 font-mono">
                <Link href={`/audit/${r.audit_id}`} className="text-blue-600 hover:underline">
                  #{r.audit_id}
                </Link>
              </td>
              <td className="px-3 py-2 font-mono text-xs">{formatTimestamp(r.ts)}</td>
              <td className="px-3 py-2">
                <span
                  className={
                    r.mode === "MODE_3"
                      ? "rounded bg-purple-100 px-2 py-0.5 text-xs text-purple-900 dark:bg-purple-950 dark:text-purple-200"
                      : "text-xs text-muted-foreground"
                  }
                >
                  {r.mode}
                </span>
              </td>
              <td className="px-3 py-2 font-mono text-xs">{r.sentence_hash.slice(0, 16)}…</td>
              <td className="px-3 py-2">{r.label ?? "—"}</td>
              <td className="px-3 py-2 tabular-nums">
                {r.confidence != null ? r.confidence.toFixed(3) : "—"}
              </td>
              <td className="px-3 py-2 text-right tabular-nums">
                {formatLatencyMs(r.duration_ms)}
              </td>
              <td className="px-3 py-2 text-center">
                {r.mode === "MODE_3" ? (r.converged ? "✓" : "✗") : "—"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
