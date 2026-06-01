interface Props {
  label: string;
  value: string | number;
  hint?: string;
}

export function KpiCard({ label, value, hint }: Props) {
  return (
    <div className="rounded-md border p-4 shadow-sm">
      <p className="text-xs uppercase tracking-wide text-muted-foreground">{label}</p>
      <p className="mt-2 text-2xl font-bold tabular-nums">{value}</p>
      {hint && <p className="mt-1 text-xs text-muted-foreground">{hint}</p>}
    </div>
  );
}
