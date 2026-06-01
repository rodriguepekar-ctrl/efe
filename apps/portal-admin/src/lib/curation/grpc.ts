// Curation write-side client (server-side, used by server actions).
//
// Despite the name, this is a REST client — it talks to the EFE backend's
// `/v1/curation/*` REST gateway. Naming retained to mirror the Rust
// `efe-admin/src/grpc/curation_client.rs` client. Future versions may
// switch to a tonic-web gRPC bridge.

const API_BASE = process.env.EFE_API_BASE ?? "http://localhost:8081";

type ApproveBody = {
  proposalId: string;
  rationale: string;
  oidcIdTokenHash?: string;
  oidcTokenIat?: string;
};

type PromoteBody = { proposalId: string; skipCanary: boolean };

type RollbackBody = { targetVersionId: string; catalogId: string; reason: string };

export class CurationGrpcClient {
  constructor(private readonly bearer: string) {}

  private async post<T>(path: string, body: unknown): Promise<T> {
    const r = await fetch(`${API_BASE}/v1/curation${path}`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${this.bearer}`,
        "content-type": "application/json",
      },
      body: JSON.stringify(body),
      cache: "no-store",
    });
    if (!r.ok) {
      const text = await r.text().catch(() => "");
      throw new Error(`POST ${path} : ${r.status} ${text}`);
    }
    return r.json() as Promise<T>;
  }

  async approveProposal(b: ApproveBody): Promise<{ new_status: string }> {
    return this.post(`/proposals/${encodeURIComponent(b.proposalId)}/approve`, {
      rationale: b.rationale,
      oidc_id_token_hash: b.oidcIdTokenHash,
      oidc_token_iat: b.oidcTokenIat,
    });
  }

  async rejectProposal(b: { proposalId: string; rationale: string }): Promise<{ new_status: string }> {
    return this.post(`/proposals/${encodeURIComponent(b.proposalId)}/reject`, {
      rationale: b.rationale,
    });
  }

  async promoteProposal(b: PromoteBody): Promise<{
    candidate_version_id: string;
    canary_started_at: string;
  }> {
    return this.post(`/proposals/${encodeURIComponent(b.proposalId)}/promote`, {
      skip_canary: b.skipCanary,
    });
  }

  async rollbackTaxonomyVersion(b: RollbackBody): Promise<{ new_active_version_id: string }> {
    return this.post(`/versions/${encodeURIComponent(b.targetVersionId)}/rollback`, {
      catalog_id: b.catalogId,
      reason: b.reason,
    });
  }

  async acknowledgeDriftEvent(b: { eventId: string; note?: string }): Promise<void> {
    await this.post(`/drift/${encodeURIComponent(b.eventId)}/acknowledge`, {
      note: b.note,
    });
  }
}
