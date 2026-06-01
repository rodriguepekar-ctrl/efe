"use client";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { trpc } from "@/lib/trpc/client";
import { useRouter } from "next/navigation";
import { useState } from "react";

export function AttestationForm() {
  const router = useRouter();
  const [aiAct, setAiAct] = useState(false);
  const [gdpr, setGdpr] = useState(false);
  const [signedBy, setSignedBy] = useState("");
  const [dpoContact, setDpoContact] = useState("");
  const [error, setError] = useState<string | null>(null);

  const attest = trpc.compliance.attest.useMutation({
    onSuccess: () => router.refresh(),
    onError: (e) => setError(e.message),
  });

  const submit = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    setError(null);
    if (!aiAct && !gdpr) {
      setError("Select at least one attestation");
      return;
    }
    if (gdpr && !dpoContact) {
      setError("DPO contact is required when GDPR Art. 22 is acknowledged");
      return;
    }
    if (!/^.+@.+\..+$/.test(signedBy)) {
      setError("Valid signer email required");
      return;
    }
    attest.mutate({
      ai_act_art14: aiAct,
      gdpr_art22: gdpr,
      signed_by: signedBy,
      dpo_contact: dpoContact || undefined,
    });
  };

  return (
    <form onSubmit={submit} className="space-y-4 rounded-md border p-4">
      <fieldset className="space-y-3">
        <legend className="font-medium">Select attestations</legend>
        <label className="flex items-start gap-2">
          <input
            type="checkbox"
            checked={aiAct}
            onChange={(e) => setAiAct(e.target.checked)}
            className="mt-1"
          />
          <span>
            <strong>EU AI Act Art. 14</strong> — I acknowledge the high-risk human oversight
            obligations for this tenant.
          </span>
        </label>
        <label className="flex items-start gap-2">
          <input
            type="checkbox"
            checked={gdpr}
            onChange={(e) => setGdpr(e.target.checked)}
            className="mt-1"
          />
          <span>
            <strong>GDPR Art. 22</strong> — I acknowledge the right not to be subject to solely
            automated decisions.
          </span>
        </label>
      </fieldset>

      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <div>
          <Label htmlFor="signed-by">Signer email</Label>
          <Input
            id="signed-by"
            type="email"
            required
            value={signedBy}
            onChange={(e) => setSignedBy(e.target.value)}
            placeholder="dpo@acme.com"
          />
        </div>
        <div>
          <Label htmlFor="dpo-contact">
            DPO contact{" "}
            {gdpr ? <span className="text-destructive">*</span> : <span>(optional)</span>}
          </Label>
          <Input
            id="dpo-contact"
            type="email"
            value={dpoContact}
            onChange={(e) => setDpoContact(e.target.value)}
            placeholder="dpo@acme.com"
          />
        </div>
      </div>

      {error && <p className="text-sm text-destructive">{error}</p>}

      <div className="flex justify-end">
        <Button type="submit" disabled={attest.isPending}>
          {attest.isPending ? "Recording…" : "Record attestation"}
        </Button>
      </div>
    </form>
  );
}
