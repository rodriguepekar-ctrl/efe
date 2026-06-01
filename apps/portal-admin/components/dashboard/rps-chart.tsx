"use client";

import { useEffect, useState } from "react";
import { Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";

interface Point {
  t: string;
  v: number;
}

export function RpsChart() {
  const [data, setData] = useState<Point[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    const url = `/api/metrics/promql?${new URLSearchParams({
      query: "sum(rate(efe_classify_total[1m]))",
      range_seconds: "3600",
      step: "60",
    })}`;
    fetch(url)
      .then((r) => (r.ok ? r.json() : Promise.reject(r.statusText)))
      .then((json: { values: [number, string][] }) => {
        if (!mounted) return;
        setData(
          json.values.map(([t, v]) => ({
            t: new Date(t * 1000).toLocaleTimeString(),
            v: Number.parseFloat(v),
          })),
        );
      })
      .catch((e) => mounted && setError(String(e)));
    return () => {
      mounted = false;
    };
  }, []);

  if (error) {
    return (
      <div className="rounded-md border bg-amber-50 p-4 text-sm text-amber-900 dark:bg-amber-950 dark:text-amber-200">
        Could not load RPS data : {error}
      </div>
    );
  }

  return (
    <div className="rounded-md border p-4">
      <h3 className="mb-2 text-sm font-medium">Classify RPS — last 1h</h3>
      <ResponsiveContainer width="100%" height={250}>
        <LineChart data={data}>
          <XAxis dataKey="t" tick={{ fontSize: 10 }} />
          <YAxis tick={{ fontSize: 10 }} />
          <Tooltip />
          <Line
            type="monotone"
            dataKey="v"
            stroke="hsl(var(--primary))"
            strokeWidth={2}
            dot={false}
          />
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
}
