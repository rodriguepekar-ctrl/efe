"use client";

interface Props {
  spentCents: number;
  capCents: number;
}

export default function CostMeter({ spentCents, capCents }: Props) {
  const pct = capCents > 0 ? Math.min(100, Math.round((spentCents / capCents) * 100)) : 100;
  const color = pct >= 95 ? "bg-red-500" : pct >= 80 ? "bg-orange-500" : "bg-emerald-500";

  return (
    <div className="space-y-1">
      <p className="font-mono text-sm">
        ${(spentCents / 100).toFixed(2)} / ${(capCents / 100).toFixed(2)}
      </p>
      <div className="h-2 bg-slate-100 rounded">
        <div className={`h-2 rounded ${color}`} style={{ width: `${pct}%` }} />
      </div>
      <p className="text-xs text-slate-500">{pct}%</p>
    </div>
  );
}
