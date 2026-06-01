// Active learning REST client (server-side). Mirrors the Rust
// active_learning_client.rs surface.

const API_BASE = process.env.EFE_API_BASE ?? "http://localhost:8081";

export class ActiveLearningGrpcClient {
  constructor(private readonly bearer: string) {}

  private async post<T>(path: string, body: unknown): Promise<T> {
    const r = await fetch(`${API_BASE}/v1/learning${path}`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${this.bearer}`,
        "content-type": "application/json",
      },
      body: JSON.stringify(body),
      cache: "no-store",
    });
    if (!r.ok) throw new Error(`POST ${path} : ${r.status} ${await r.text().catch(() => "")}`);
    return r.json() as Promise<T>;
  }

  async confirmGoldLabel(b: { proposalId: string; rationale: string }) {
    return this.post(
      `/proposals/${encodeURIComponent(b.proposalId)}/confirm`,
      { rationale: b.rationale },
    );
  }

  async editAndConfirm(b: { proposalId: string; finalClassId: string; rationale: string }) {
    return this.post(
      `/proposals/${encodeURIComponent(b.proposalId)}/edit-and-confirm`,
      { final_class_id: b.finalClassId, rationale: b.rationale },
    );
  }

  async rejectGoldLabel(b: { proposalId: string; rationale: string }) {
    return this.post(
      `/proposals/${encodeURIComponent(b.proposalId)}/reject`,
      { rationale: b.rationale },
    );
  }

  async triggerRetune(b: { catalogId: string; forceBelowThreshold: boolean }) {
    return this.post("/retune/trigger", {
      catalog_id: b.catalogId, force_below_threshold: b.forceBelowThreshold,
    });
  }

  async promote(b: { runId: string; skipCanary: boolean }) {
    return this.post(`/retune/${encodeURIComponent(b.runId)}/promote`, {
      skip_canary: b.skipCanary,
    });
  }

  async rollback(b: { runId: string; reason: string }) {
    return this.post(`/retune/${encodeURIComponent(b.runId)}/rollback`, {
      reason: b.reason,
    });
  }

  async submitGdprErasure(b: {
    catalogId: string; urgency: string;
    contentHashes: string[]; subjectId?: string; reason: string;
  }) {
    return this.post("/gdpr/erasure", {
      catalog_id: b.catalogId, urgency: b.urgency,
      content_hashes: b.contentHashes,
      requester_subject_id: b.subjectId,
      reason: b.reason,
    });
  }
}
