"use client";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { trpc } from "@/lib/trpc/client";
import { useRouter } from "next/navigation";
import { useState } from "react";

interface Quotas {
  max_rps: number;
  max_tokens_per_day: number;
  monthly_cap_usd?: number | null;
}

export function QuotasForm({ initial, canEdit }: { initial: Quotas; canEdit: boolean }) {
  const router = useRouter();
  const [maxRps, setMaxRps] = useState<number>(initial.max_rps);
  const [maxTokens, setMaxTokens] = useState<number>(initial.max_tokens_per_day);
  const [monthlyCap, setMonthlyCap] = useState<number | null>(initial.monthly_cap_usd ?? null);

  const setMutation = trpc.quotas.set.useMutation({
    onSuccess: () => router.refresh(),
  });

  const submit = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    setMutation.mutate({
      maxRps,
      maxTokensPerDay: maxTokens,
      monthlyCapUsd: monthlyCap ?? undefined,
    });
  };

  return (
    <form onSubmit={submit} className="space-y-4 rounded-md border p-4">
      <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
        <div>
          <Label>Max RPS</Label>
          <Input
            type="number"
            min={0}
            max={100000}
            value={maxRps}
            disabled={!canEdit}
            onChange={(e) => setMaxRps(Number.parseInt(e.target.value, 10))}
          />
        </div>
        <div>
          <Label>Max tokens / day</Label>
          <Input
            type="number"
            min={0}
            value={maxTokens}
            disabled={!canEdit}
            onChange={(e) => setMaxTokens(Number.parseInt(e.target.value, 10))}
          />
        </div>
        <div>
          <Label>Monthly cap (USD)</Label>
          <Input
            type="number"
            min={0}
            step={0.01}
            value={monthlyCap ?? ""}
            disabled={!canEdit}
            onChange={(e) =>
              setMonthlyCap(e.target.value ? Number.parseFloat(e.target.value) : null)
            }
          />
        </div>
      </div>
      {canEdit && (
        <div className="flex justify-end">
          <Button type="submit" disabled={setMutation.isPending}>
            {setMutation.isPending ? "Saving…" : "Save"}
          </Button>
        </div>
      )}
      {!canEdit && (
        <p className="text-sm text-muted-foreground">
          Only users with role <code>admin</code> can edit quotas.
        </p>
      )}
    </form>
  );
}
