import { KeyCreateButton } from "@/components/keys/key-create-button";
import { KeyListTable } from "@/components/keys/key-list-table";
import { hasAnyRole } from "@/lib/auth/rbac";
import { getRbacUser } from "@/lib/auth/session";
import { fetchKeysList } from "@/lib/trpc/server-rsc";

export default async function KeysPage() {
  const result = await fetchKeysList();
  const user = await getRbacUser();
  const canManage = hasAnyRole(user, ["admin"]);

  return (
    <div className="space-y-6 p-6">
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">API keys</h1>
          <p className="text-sm text-muted-foreground">
            Argon2id-hashed bearer tokens for the EFE gRPC API. Plaintext is displayed once at
            creation — store immediately.
          </p>
        </div>
        {canManage && <KeyCreateButton />}
      </header>

      <KeyListTable keys={result.keys ?? []} canManage={canManage} />
    </div>
  );
}
