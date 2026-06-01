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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { trpc } from "@/lib/trpc/client";
import { useRouter } from "next/navigation";
import { useState } from "react";

interface Props {
  token: string;
  proposedLabel: string;
}

export function ReviewActionPanel({ token, proposedLabel }: Props) {
  const router = useRouter();
  const [open, setOpen] = useState<"approve" | "reject" | null>(null);
  const [finalLabel, setFinalLabel] = useState("");
  const [reason, setReason] = useState("");
  const [error, setError] = useState<string | null>(null);

  const approve = trpc.reviews.approve.useMutation({
    onSuccess: () => {
      setOpen(null);
      router.push("/reviews");
      router.refresh();
    },
    onError: (e) => setError(e.message),
  });
  const reject = trpc.reviews.reject.useMutation({
    onSuccess: () => {
      setOpen(null);
      router.push("/reviews");
      router.refresh();
    },
    onError: (e) => setError(e.message),
  });

  const submit = () => {
    setError(null);
    if (open === "approve") {
      approve.mutate({ token, finalLabel: finalLabel || undefined });
    } else if (open === "reject") {
      if (reason.trim().length < 10) {
        setError("Rejection reason must be at least 10 characters");
        return;
      }
      reject.mutate({ token, reason, finalLabel: finalLabel || undefined });
    }
  };

  return (
    <div className="flex gap-2">
      <Button onClick={() => setOpen("approve")}>Approve</Button>
      <Button variant="destructive" onClick={() => setOpen("reject")}>
        Reject
      </Button>

      <Dialog open={open !== null} onOpenChange={(v) => !v && setOpen(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{open === "approve" ? "Approve" : "Reject"} review</DialogTitle>
            <DialogDescription>
              {open === "approve"
                ? "This action records the final label and closes the review. Audit-logged."
                : "This action records the rejection rationale. The reason is required (≥ 10 chars)."}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            <div>
              <Label htmlFor="final-label">Final label (optional override)</Label>
              <Input
                id="final-label"
                placeholder={proposedLabel}
                value={finalLabel}
                onChange={(e) => setFinalLabel(e.target.value)}
              />
            </div>
            {open === "reject" && (
              <div>
                <Label htmlFor="reason">Rejection reason</Label>
                <textarea
                  id="reason"
                  rows={3}
                  className="flex w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
                  value={reason}
                  onChange={(e) => setReason(e.target.value)}
                />
              </div>
            )}
            {error && <p className="text-sm text-destructive">{error}</p>}
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setOpen(null)}>
              Cancel
            </Button>
            <Button
              variant={open === "approve" ? "default" : "destructive"}
              onClick={submit}
              disabled={approve.isPending || reject.isPending}
            >
              Confirm {open}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
