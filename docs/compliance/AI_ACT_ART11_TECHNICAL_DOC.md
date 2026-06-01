# EU AI Act — Article 11 Technical Documentation

**System name** : EFE (Ephemeral Fluid Swarm) Classification Platform
**Provider** : EFE-AI Inc.
**Version** : v1.0.0 (GA, 2026-05-09)
**Risk class** : limited-risk (per EFE's AI Act self-classification ;
re-classification triggers documented in § 7).
**Conformity assessment** : self-assessment per Annex IV.
**Document version** : 1.0

> Article 11 of the EU AI Act (Regulation 2024/1689) requires providers
> of high-risk AI systems to draw up technical documentation before
> placing the system on the market. EFE is currently classified as
> **limited-risk** but maintains this documentation in case of
> re-classification or for tenants who deploy EFE in high-risk contexts
> (Annex III). The structure follows Annex IV.

---

## 1. General description (Annex IV § 1)

### 1.1 Intended purpose

EFE is a multi-tenant semantic classification gRPC service. It accepts
arbitrary text inputs and returns one or more labels with associated
confidences, drawn from a tenant-configured taxonomy. Typical use
cases :

- Customer support ticket triage.
- Document categorization (legal, finance, healthcare).
- Compliance / policy classification.

EFE does **not** make autonomous decisions. It produces classification
predictions ; downstream business logic (managed by the tenant) is
responsible for any decision based on those predictions.

### 1.2 Person(s) developing the system

EFE-AI Inc., engineering team headquartered in Paris and San Francisco.
Specific authorship per file is documented in `git log`.

### 1.3 Foreseeable misuse

Foreseeable misuse cases EFE actively guards against :

- **Use as a sole basis for legal decisions** (e.g. employment,
  benefits) without human oversight.
- **Use to classify protected categories** (race, religion, sexual
  orientation) without explicit tenant DPIA + Art. 22 GDPR opt-in.
- **Use in critical safety contexts** (medical diagnosis, autonomous
  vehicles) without tenant-side conformity assessment.

EFE's Acceptable Use Policy (AUP) prohibits these uses without
additional safeguards. Violations can trigger tenant suspension.

### 1.4 Hardware on which the system is intended to run

- Kubernetes 1.27+ on AWS EKS, GCP GKE, or Azure AKS.
- Minimum 16 vCPU + 64 GiB RAM per inference pod.
- GPU acceleration optional (NVIDIA L4 / L40 / H100 supported).
- Storage : Postgres 15+ for audit logs, S3-compatible blob for cold
  data.

### 1.5 Form in which the system is placed on the market

- Docker images on `ghcr.io/efe-ai/...`.
- Helm chart at `oci://ghcr.io/efe-ai/charts/efe-stack`.
- Terraform modules at `registry.terraform.io/efe-ai/efe-stack/{aws,gcp,azure}`.
- CLI binaries at `https://github.com/efe-ai/efe/releases`.

### 1.6 Description of the user interface

- gRPC API : binary contract, defined in `proto/efe/v1/*.proto`.
- HTTP/JSON gateway : REST equivalent for non-gRPC clients.
- Portal admin : web UI for configuration, audit, and observability.
- CLI (`efe-admin`) : management and operational tasks.

---

## 2. Detailed description (Annex IV § 2)

### 2.1 Methods used in development

- **Training data lineage** : every model version ships with a
  data-card listing data sources, dates, sizes, and license terms.
- **Training infrastructure** : isolated GitHub-hosted runners + EFE
  internal training cluster (no public-internet access to training
  data).
- **Validation** : 80/10/10 train/dev/test split with hold-out
  evaluation per tenant taxonomy.

### 2.2 Design specifications

- **Architecture** : transformer encoder + dense classifier head.
- **Quantization** : Q4_K_M and Q5_K_M GGUF variants for inference.
- **Multi-tenancy** : per-tenant KMS context + RBAC + audit chain.
- **Offline / online** : EFE supports both online inference and
  pre-computed batch classification.

### 2.3 Description of the system

See `docs/EFE_SERVER_ARCHITECTURE.md`, `docs/EFE_CORE_EXTENSION_ARCHITECTURE.md`,
and `docs/EFE_KERNELS_ARCHITECTURE.md` for detailed system architecture.

### 2.4 Information about training data

- Public corpora : Common Crawl, Wikipedia, OSCAR (filtered for license
  compatibility).
- Domain corpora : tenant-uploaded labeled examples (opt-in).
- Synthetic data : controlled generation with provenance tracking.
- Personal data : zero — training data is filtered to remove PII via
  named-entity recognition + manual review.

Detailed datasheet : `docs/compliance/datasheet_v1.0.md`.

### 2.5 Validation and testing procedures

- Unit tests : `pytest` and `cargo test` (>1000 tests, >85% coverage).
- Integration tests : E2E scenarios per tenant taxonomy.
- Eval pipeline : nightly evaluation on hold-out sets, drift detection.
- Bias evaluation : disaggregated metrics across protected attributes
  (where labeled).

### 2.6 Cybersecurity measures

See `docs/compliance/gdpr_art32.md` and `docs/compliance/iso27001.md`.

---

## 3. Information about monitoring (Annex IV § 3)

### 3.1 Capabilities and limitations

- **Capabilities** : multi-label, multi-class classification with
  configurable thresholds. Throughput ~10K req/s per pod.
- **Limitations** :
  - Performance varies with input length (best for ≤ 2048 tokens).
  - Predictions are language-dependent ; supported languages listed in
    `docs/EFE_KERNELS_ARCHITECTURE.md`.
  - Confidence calibration is approximate (ECE ≤ 0.05 on hold-out).

### 3.2 Expected accuracy

- Macro-F1 on tenant-aligned hold-out : > 0.85 (default models).
- Per-class accuracy : reported in the per-tenant evaluation dashboard.
- Drift alerts trigger when measured accuracy drops > 5 percentage
  points from baseline.

### 3.3 Risk management system

- Risk register : `docs/security/risk_register.md`.
- Threat model : `docs/security/threat_model.md`.
- Continuous compliance posture : `docs/compliance/continuous_compliance.md`.

### 3.4 Changes to the system through its lifecycle

- Every deployment is tagged and Sigstore-signed.
- Model retraining triggers a new minor version (`v1.X.0`).
- Architectural changes trigger a major version (`v2.0.0`) with 12+
  months deprecation window.
- Changelog : `CHANGELOG.md`.

### 3.5 Human oversight measures (Art. 14)

EFE provides first-class human oversight hooks :

- Per-prediction confidence threshold : below threshold triggers a
  human review queue.
- Manual override : portal-admin user with `reviewer` role can override
  any prediction.
- Audit chain : every override is HMAC-chained for tamper-evidence.
- Latency SLA : human review queue must be drained within tenant-set
  P99 (default 24h).
- Attestation : `efe-admin compliance verify <tenant>` reports the
  Art. 14 attestation status (`human_oversight_enabled`,
  `last_review_decision_at`).

---

## 4. Performance metrics

| Metric                 | Threshold      | Current        |
|------------------------|----------------|----------------|
| Macro-F1 (default model) | > 0.85       | 0.892          |
| ECE (calibration)      | < 0.05         | 0.038          |
| P99 latency (online)   | < 200 ms       | 142 ms         |
| Drift detection lag    | < 24 h         | < 6 h          |
| Human review SLA       | < 24 h (tenant default) | tenant-configurable |

---

## 5. Compliance with Article 13 (transparency)

EFE provides transparency information to tenants and (transitively) to
their end users :

- A **system card** at `https://efe.example.com/transparency` listing
  capabilities, limitations, training data, and known biases.
- A **per-prediction explanation** API : `EXPLAIN_PREDICTION` RPC
  returns top-K influencing tokens for the classification.
- A **public model registry** : `https://efe.example.com/models` lists
  every active model version with date and lineage.

---

## 6. Conformity assessment

- Self-assessment per Annex IV completed : 2026-05-08.
- External review (independent assessor) : planned 2027-Q1.
- CE marking (when EFE classifies as high-risk under Annex III) : not
  yet applicable, will be obtained before any high-risk deployment.

---

## 7. Re-classification triggers

EFE re-evaluates its risk class quarterly. A change to **high-risk**
under Annex III is triggered if EFE is sold for use in :

- Biometric identification.
- Critical infrastructure (energy, water, transport).
- Education or vocational training (decisions affecting access).
- Employment (HR decisions).
- Essential public services (welfare, credit scoring).
- Law enforcement.
- Migration, asylum, border control.
- Administration of justice.

A re-classification triggers :
1. Conformity assessment by an external notified body.
2. Registration in the EU database (Art. 60).
3. CE marking.
4. Post-market monitoring plan (Art. 72).

---

## References

- EU AI Act Annex IV (full text) : https://eur-lex.europa.eu/eli/reg/2024/1689
- EFE compliance documentation : `docs/compliance/`
- EFE security documentation : `docs/security/`
- Architecture : `docs/EFE_SERVER_ARCHITECTURE.md`, `docs/EFE_CORE_EXTENSION_ARCHITECTURE.md`
- Continuous compliance : `docs/compliance/continuous_compliance.md`
