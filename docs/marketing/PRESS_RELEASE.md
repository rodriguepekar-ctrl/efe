# Press Release — EFE v1.0.0 GA

**Embargo** : 2026-05-09, 12:00 UTC
**Distribution** : PRWeb, Business Wire, EU GovTech newsletter, AI / ML
trade press
**Boilerplate length** : ~600 words (full) / ~200 words (short)

---

## Headline

```
EFE-AI Releases v1.0.0 of Its Multi-Tenant Classification Platform —
Sigstore-Signed and SLSA-3 Attested Out of the Box
```

## Sub-head

```
First production release of the Ephemeral Fluid Swarm reaches General
Availability with cryptographically verifiable supply chain, multi-cloud
deployment, and per-tenant compliance evidence packages aligned with
SOC 2, GDPR, EU AI Act, and ISO 27001.
```

## Dateline

```
PARIS — May 9, 2026 —
```

## Body (full version, ~600 words)

> EFE-AI Inc., the company behind the Ephemeral Fluid Swarm
> classification platform, today announced the **General Availability of
> EFE v1.0.0**. The release marks the first stable, production-supported
> version of EFE following 24 months of engineering and a year-long beta
> with over 80 enterprise tenants across financial services, healthcare,
> public sector, and technology industries.
>
> EFE is a multi-tenant gRPC service for high-throughput semantic
> classification, used to triage support tickets, categorize documents,
> classify policy and compliance signals, and route work in agentic
> systems. Unlike general-purpose LLM APIs, EFE is purpose-built for
> deterministic, low-latency, audit-grade classification — every
> prediction is HMAC-chained for tamper-evidence and routed through
> human-oversight queues when configured by the customer.
>
> ### A first-of-its-kind verifiable supply chain
>
> EFE v1.0.0 is the first major AI infrastructure release shipping with
> **end-to-end Sigstore signatures and SLSA-3 provenance attestations
> on every artifact** — across 4 Docker images, 1 Helm umbrella chart,
> 6 platform CLI binaries, 1 npm SDK package, 3 Terraform modules, and
> 1 Cargo library crate.
>
> "We've operated for years under the belief that AI infrastructure
> deserves the same supply-chain rigor as the kernel and Kubernetes
> itself," said the EFE Engineering Lead. "v1.0.0 is the first release
> where we deliver on that belief end-to-end. Every artifact you pull
> from any of our six distribution channels can be cryptographically
> traced back to the exact source commit, the exact GitHub Actions
> runner that built it, and the exact identity claim of our release
> pipeline — all without us holding any long-lived signing keys."
>
> ### Compliance is now a build artifact
>
> EFE v1.0.0 introduces an automated **per-tenant compliance evidence
> pack** that regenerates nightly. Each pack is a cosign-signed ZIP
> containing :
>
> - SOC 2 Type II readiness evidence.
> - GDPR Article 30 register entry, tenant-personalized.
> - EU AI Act Article 11 technical documentation.
> - ISO 27001:2022 readiness self-assessment (currently 87.4 %).
> - Audit chain integrity report (HMAC tamper-evidence).
> - Live attestation status JSON.
>
> "Today, every tenant — from a 5-engineer startup to a 200,000-employee
> regulated enterprise — gets compliance evidence regenerated nightly
> and signed with a cryptographic identity that audit firms can verify
> independently," the EFE Compliance Officer added. "We believe
> compliance posture should be a public, verifiable artifact, not a
> PowerPoint."
>
> ### Multi-cloud, single chart
>
> The same `efe-stack` Helm umbrella chart deploys unchanged on AWS
> EKS, GCP GKE, and Azure AKS. Cloud-specific Terraform modules
> wrap IAM, VPC, KMS, and observability stacks while preserving
> identical operator semantics. EFE supports air-gapped tenants via
> mirrored OCI registries and a pinned TUF trusted root for offline
> Sigstore verification.
>
> ### Availability and pricing
>
> EFE v1.0.0 is available immediately :
>
> - Public Docker images at `ghcr.io/efe-ai/`.
> - Helm chart at `oci://ghcr.io/efe-ai/charts/efe-stack`.
> - Terraform modules at `registry.terraform.io/efe-ai/efe-stack/{aws,gcp,azure}`.
> - CLI binaries at `https://github.com/efe-ai/efe/releases`.
>
> Pricing is per-tenant subscription with usage-based metering. Contact
> sales for enterprise terms.
>
> ### About EFE-AI Inc.
>
> EFE-AI Inc. builds the Ephemeral Fluid Swarm classification platform
> — purpose-built infrastructure for high-throughput, audit-grade
> semantic classification in regulated industries. Headquartered in
> Paris with offices in San Francisco, the company is venture-backed
> and serves customers in financial services, healthcare, public
> sector, and SaaS technology.
>
> ### Media contact
>
> Press : `press@efe.example.com`
> Investor relations : `ir@efe.example.com`
> Engineering blog : `https://efe.example.com/blog`

## Body (short version, ~200 words)

> PARIS — May 9, 2026 — EFE-AI Inc. today announced the General
> Availability of **EFE v1.0.0**, the first stable release of its
> multi-tenant classification platform. EFE v1.0.0 ships with end-to-end
> **Sigstore signatures and SLSA-3 provenance** on every artifact across
> Docker, Helm, CLI, npm, Terraform, and Cargo distribution channels.
>
> A central new feature is the **per-tenant compliance evidence pack**,
> a cosign-signed ZIP regenerated nightly that aligns with SOC 2 Type
> II, GDPR Art. 30, EU AI Act Art. 11, and ISO 27001:2022. Every pack
> is verifiable by external auditors through public Rekor inclusion
> proofs.
>
> The release covers 4 production Docker images, an `efe-stack` Helm
> umbrella chart with 6 sub-charts, 3 Terraform modules (AWS, GCP,
> Azure), 6 CLI binaries, 4 SDKs (Python, Go, TypeScript, Java), and
> a per-tenant compliance evidence pack.
>
> EFE v1.0.0 is available immediately at the public Docker registry
> `ghcr.io/efe-ai/`, the Terraform Registry, GitHub Releases, and
> Homebrew. Verification instructions are at
> `https://github.com/efe-ai/efe/blob/v1.0.0/docs/release/VERIFY_RELEASE.md`.
>
> Press contact : `press@efe.example.com`.

## Boilerplate (always last paragraph)

```
About EFE-AI Inc. — EFE-AI Inc. builds the Ephemeral Fluid Swarm
classification platform, purpose-built infrastructure for
high-throughput, audit-grade semantic classification in regulated
industries. Headquartered in Paris with offices in San Francisco, the
company serves customers in financial services, healthcare, public
sector, and SaaS technology. https://efe.example.com
```

## Quoted spokespeople

| Name (placeholder) | Role                  | Topic                                       |
|--------------------|-----------------------|---------------------------------------------|
| TBD                | Engineering Lead      | Sigstore + SLSA-3 supply chain              |
| TBD                | Compliance Officer    | Per-tenant compliance evidence              |
| TBD                | Customer (large bank) | Production deployment outcome               |
| TBD                | Sigstore community    | Industry significance (optional, with consent) |

---

## Distribution checklist

- [ ] Final review by Comms + Legal (T-3 days)
- [ ] PR Newswire schedule (T-1 day, embargoed)
- [ ] Business Wire schedule (T-1 day, embargoed)
- [ ] EU GovTech newsletter (T-7 days direct outreach)
- [ ] AI/ML trade press direct outreach (T-3 days, embargoed)
- [ ] Public blog post live (T+0)
- [ ] Customer email send (T+2 h)
- [ ] Partner Slack notifications (T+2 h)

---

## Internal coordination

- Comms lead approves final copy.
- Legal review for forward-looking statements.
- Engineering review for technical accuracy.
- Customer success review for customer-quote logistics.
- Embargoed copies go to PR Newswire 24 h before T+0.

## References

- Blog post : `docs/marketing/BLOG_POST_v1_0_0_GA.md`
- Customer email : `docs/marketing/CUSTOMER_EMAIL_TEMPLATE.md`
- Partner notification : `docs/marketing/PARTNER_NOTIFICATION.md`
- Architecture : `docs/EFE_RELEASE_V1_GA_ARCHITECTURE.md`
- Release notes : `docs/release/RELEASE_NOTES_v1.0.0.md`
