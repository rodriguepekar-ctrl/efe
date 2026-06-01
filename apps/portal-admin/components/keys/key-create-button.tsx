"use client";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { trpc } from "@/lib/trpc/client";
import { useRouter } from "next/navigation";
import { useState } from "react";

export function KeyCreateButton() {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [scope, setScope] = useState<"read_only" | "classify" | "admin">("classify");
  const [expiresInDays, setExpiresInDays] = useState<number | null>(30);
  const [plaintext, setPlaintext] = useState<string | null>(null);

  const create = trpc.keys.create.useMutation({
    onSuccess: (resp: any) => {
      setPlaintext(resp.plaintext ?? null);
      router.refresh();
    },
  });

  const reset = () => {
    setPlaintext(null);
    setScope("classify");
    setExpiresInDays(30);
    setOpen(false);
  };

  return (
    <>
      <Button onClick={() => setOpen(true)}>Create new key</Button>
      <Dialog open={open} onOpenChange={(v) => (v ? setOpen(true) : reset())}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create API key</DialogTitle>
            <DialogDescription>
              Plaintext is shown ONCE — copy and store immediately.
            </DialogDescription>
          </DialogHeader>

          {plaintext ? (
            <div className="space-y-2">
              <p className="text-sm">Save this token now :</p>
              <pre className="overflow-x-auto rounded bg-muted p-3 text-xs font-mono">
                {plaintext}
              </pre>
              <Button onClick={reset}>Done</Button>
            </div>
          ) : (
            <div className="space-y-3">
              <div>
                <Label>Scope</Label>
                <select
                  value={scope}
                  onChange={(e) => setScope(e.target.value as typeof scope)}
                  className="block w-full rounded border bg-background px-2 py-1 text-sm"
                >
                  <option value="read_only">read_only</option>
                  <option value="classify">classify</option>
                  <option value="admin">admin</option>
                </select>
              </div>
              <div>
                <Label>Expires in (days, blank = never)</Label>
                <input
                  type="number"
                  min={1}
                  max={3650}
                  value={expiresInDays ?? ""}
                  onChange={(e) =>
                    setExpiresInDays(e.target.value ? Number.parseInt(e.target.value, 10) : null)
                  }
                  className="block w-full rounded border bg-background px-2 py-1 text-sm"
                />
              </div>
              <DialogFooter>
                <Button variant="ghost" onClick={reset}>
                  Cancel
                </Button>
                <Button
                  onClick={() => create.mutate({ scope, expiresInDays })}
                  disabled={create.isPending}
                >
                  {create.isPending ? "Creating…" : "Create"}
                </Button>
              </DialogFooter>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </>
  );
}
