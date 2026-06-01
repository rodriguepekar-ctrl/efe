"use client";

import { Button } from "@/components/ui/button";
import { trpc } from "@/lib/trpc/client";
import { formatTimestamp, relativeTime } from "@/lib/utils/format";
import { useRouter } from "next/navigation";
import { useState } from "react";

interface KeyRow {
  key_id: string;
  suffix: string;
  scope: "read_only" | "classify" | "admin";
  created_at: string;
  expires_at?: string | null;
  revoked_at?: string | null;
  last_used_at?: string | null;
  use_count: number;
}

export function KeyListTable({ keys, canManage }: { keys: KeyRow[]; canManage: boolean }) {
  const router = useRouter();
  const rotate = trpc.keys.rotate.useMutation({ onSuccess: () => router.refresh() });
  const revoke = trpc.keys.revoke.useMutation({ onSuccess: () => router.refresh() });
  const [pendingRevoke, setPendingRevoke] = useState<string | null>(null);
  const [revokeReason, setRevokeReason] = useState("");

  if (keys.length === 0) {
    return (
      <p className="rounded-md border bg-muted/30 p-8 text-center text-sm text-muted-foreground">
        No active API keys for this tenant.
      </p>
    );
  }

  return (
    <div className="overflow-x-auto rounded-md border">
      <table className="w-full text-sm">
        <thead className="bg-muted/50 text-xs uppercase">
          <tr>
            <th className="px-3 py-2 text-left">Key ID</th>
            <th className="px-3 py-2 text-left">Suffix</th>
            <th className="px-3 py-2 text-left">Scope</th>
            <th className="px-3 py-2 text-left">Created</th>
            <th className="px-3 py-2 text-left">Expires</th>
            <th className="px-3 py-2 text-left">Last used</th>
            <th className="px-3 py-2 text-right">Uses</th>
            {canManage && <th className="px-3 py-2 text-right">Actions</th>}
          </tr>
        </thead>
        <tbody>
          {keys.map((k) => (
            <tr key={k.key_id} className="border-t">
              <td className="px-3 py-2 font-mono text-xs">{k.key_id.slice(0, 8)}…</td>
              <td className="px-3 py-2 font-mono">…{k.suffix}</td>
              <td className="px-3 py-2">
                <span className="rounded bg-muted px-2 py-0.5 text-xs">{k.scope}</span>
              </td>
              <td className="px-3 py-2">{formatTimestamp(k.created_at)}</td>
              <td className="px-3 py-2">
                {k.expires_at ? formatTimestamp(k.expires_at) : "never"}
              </td>
              <td className="px-3 py-2">{relativeTime(k.last_used_at)}</td>
              <td className="px-3 py-2 text-right tabular-nums">{k.use_count}</td>
              {canManage && (
                <td className="px-3 py-2 text-right">
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => rotate.mutate({ keyId: k.key_id })}
                    disabled={rotate.isPending}
                  >
                    Rotate
                  </Button>
                  <Button
                    size="sm"
                    variant="destructive"
                    onClick={() => setPendingRevoke(k.key_id)}
                  >
                    Revoke
                  </Button>
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>

      {pendingRevoke && (
        <div className="border-t p-4">
          <p className="text-sm font-medium">Revoke key {pendingRevoke.slice(0, 8)}…</p>
          <textarea
            placeholder="Reason (≥ 10 chars)"
            className="mt-2 w-full rounded border bg-background px-2 py-1 text-sm"
            rows={2}
            value={revokeReason}
            onChange={(e) => setRevokeReason(e.target.value)}
          />
          <div className="mt-2 flex gap-2">
            <Button
              size="sm"
              variant="destructive"
              onClick={() => {
                if (revokeReason.length < 10) return;
                revoke.mutate({ keyId: pendingRevoke, reason: revokeReason });
                setPendingRevoke(null);
                setRevokeReason("");
              }}
            >
              Confirm revoke
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setPendingRevoke(null)}>
              Cancel
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
