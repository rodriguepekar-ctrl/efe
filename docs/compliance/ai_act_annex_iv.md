# EU AI Act 2024/1689 — Annex IV Technical Documentation — EFE 1.0 GA

> **Status** : Sprint M14 D41 (Φ-B) — AI Act Annex IV technical doc.
> **Audience** : EU AI Office, customer compliance officers, regulators.
> **Regulation** : Regulation (EU) 2024/1689 (AI Act). Effective 2026-08-02.
> **EFE classification** : **Limited-risk AI system** per Annex III (NOT high-risk).
> **Last updated** : 2026-04-30.

---

## 1. Scope of this document

EU AI Act Art. 11 + Annex IV require providers of **high-risk** AI
systems to maintain technical documentation. EFE is **limited-risk**
(does NOT match any Annex III high-risk category), so this document
is provided **voluntarily** for transparency + customer reuse.

Customers deploying EFE for high-risk applications must produce their
own Annex IV documentation.

---

## §1 General description

### 1.1 System name and version
**EFE — Ephemeral Fluid Swarm v1.0.0**

### 1.2 Provider
EFE Project (release@efe.ai).

### 1.3 Intended purpose

EFE is a **semantic classification system** that assigns text inputs
to operator-defined classes via embedding-based nearest-cluster matching.

**Intended use cases** :
- Customer support ticket routing
- Content moderation triage
- Product feature classification
- Multi-tenant intent recognition
- E-commerce category assignment

### 1.4 NOT intended for (use limitations per Art. 5)
- Biometric identification (Art. 5(1)(d))
- Emotion recognition in workplaces or schools (Art. 5(1)(g))
- Predictive policing (Art. 5(1)(h))
- Social scoring (Art. 5(1)(c))
- Real-time remote biometric ID in public spaces (Art. 5(1)(h))
- Critical infrastructure safety functions
- Manipulation / exploitation of vulnerabilities (Art. 5(1)(b))

Customers attempting these uses violate the EFE ToS + DPA + AI Act.

### 1.5 AI Act risk classification

Per Annex III enumeration, EFE is **NOT high-risk** :
- ❌ Biometric (no face/fingerprint inputs)
- ❌ Critical infrastructure (no SCADA / power / transport)
- ❌ Employment / education (no candidate scoring)
- ❌ Access to essential services (no welfare eligibility)
- ❌ Law enforcement
- ❌ Migration / asylum
- ❌ Justice administration

**Classification** : **Limited-risk transparency obligations apply
(Art. 52)** — users interacting with EFE outputs must be informed
they're interacting with an AI system.

### 1.6 Interaction methods
- gRPC (Tonic protocol) over HTTP/2
- mTLS optional (M12 D22)
- REST gateway (optional, customer-deployed)
- SDKs : Python (M8), Go (M9), TypeScript (M10), Java (M11)
- No direct user-facing UI ; consumed via customer applications

### 1.7 Hardware requirements
- CPU : x86_64 / ARM64
- Memory : 8 GiB per pod (default ; configurable)
- GPU : optional (D28+ K-A3 path)
- Storage : 100 MiB - 500 MiB per bundle

### 1.8 Operational environment
- Kubernetes 1.27+ cluster
- Helm 3 chart M12
- OIDC-compatible identity provider (optional)
- Postgres backend (D14, optional)

---

## §2 Detailed description elements

### 2.1 Methods used

EFE uses **sentence-transformer embeddings** + **cosine similarity
nearest-class lookup** + **softmax thresholding** for confidence.

Flow :
```
Input text → Tokenizer (HuggingFace fast-tokenizer / sentencepiece)
           → Transformer encoder (ONNX Runtime CPU/GPU)
           → Mean pooling (token-level → sentence-level embedding)
           → L2 normalization
           → Cosine similarity vs operator-fitted class centroids
           → Softmax (temperature-tunable)
           → Top-K classes + confidence scores
```

### 2.2 Training datasets

**EFE does NOT train models.** EFE consumes pre-trained models shipped
via M13 bundles. Training data provenance documented per bundle :

| Bundle | Training data source | Data documentation URL |
|---|---|---|
| `all-MiniLM-L6-v2` | MS MARCO + SNLI + Reddit Webis (~1B sentence pairs) | https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2 |
| `paraphrase-multilingual-MiniLM-L12-v2` | ParallelCorpora 50+ languages (~10M parallel pairs) | https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2 |

### 2.3 Validation methods

- **Cross-lingual eval F1 ≥ 0.80** (D26 gate, fixture
  `tests/fixtures/multilingual_eval.jsonl` — 50 sentence pairs across
  5 languages + 5 contre-exemples)
- **Cosine similarity post-quantization ≥ 0.99** vs FP32 baseline
- **Operator-defined catalog validation** (D12) — accuracy threshold
  per customer

### 2.4 Optimization

- INT8 dynamic quantization (D20 sample) — 4× compression, ~99.3 % similarity
- FP16 (D26 production multilingual) — 2× compression, ~99.9 % similarity
- Trade-off documented in `docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md` §5.3

### 2.5 Computational resources at runtime

- CPU : 1-2 cores per pod
- Memory : 8 GiB per pod (incl. bundle in RAM)
- GPU : optional (D28+ K-A3 path)
- Latency : P95 < 50 ms / P99 < 200 ms (M11-RPC-3 SLO)
- Throughput : ~5 000 req/sec/pod (CPU), ~50 000 req/sec/pod (GPU)

### 2.6 Limitations

- **Inherited training data biases** (model card disclosures)
- **English-centric** : multilingual quality varies by language
  (highest : EN, FR, DE, ES, IT, PT ; medium : NL, RU, ZH, JA ;
  lower : less-resourced languages)
- **Out-of-distribution texts** produce low-confidence outputs
  (handled via confidence-threshold customization D12)
- **Adversarial inputs** : no formal adversarial robustness training ;
  mitigated via input length cap + rate limit
- **Long-context limitations** : `max_seq_len=256` (D20) or `128` (D26)
  truncates beyond ; tokens past this silently dropped

---

## §3 Risk management system

### 3.1 Risk identification methodology

EFE follows hybrid STRIDE + ML-specific threat modeling :
- **STRIDE-per-asset** : 63 threats consolidated in
  `docs/security/threat_model.md`
- **ML-specific risks** :
  - Embedding drift (mitigated via D6 cone fitting + manual approval D12)
  - Adversarial inputs (mitigated via input length cap + rate limit M11)
  - Training data bias (inherited from upstream model card disclosure)
  - Model inversion / extraction (out-of-scope — embeddings, no
    generative output ; semantic class output only)

### 3.2 Risk assessment cadence

- Quarterly threat model review (per `continuous_compliance.md`)
- Per-sprint audit pass (M11+M12+M13 pattern)
- External pen-test annual (post-1.0 GA plan)

### 3.3 Risk acceptance criteria

- **Accept** : Low residual risk + cost of mitigation > expected loss
- **Mitigate** : High residual risk + technically feasible
- **Transfer** : Insurance + customer DPA
- **Avoid** : Refuse to ship feature

### 3.4 Acceptance authority

- Low risk : Engineering lead
- Medium risk : CISO
- High risk : CTO + CISO + CEO
- Critical risk : Board approval required

---

## §4 Data governance + quality

### 4.1 Data sources

**Training data** : EFE does NOT train. Bundles come from upstream
HF Hub with pinned revisions (M13 D26).

**Operational data** : customer-provided sentences for catalog
fitting (D12). Encrypted in transit (TLS 1.3) + at rest (Postgres
TDE). Tenant-isolated per D11.

### 4.2 Data quality assurance

- Cosine similarity gate post-quantization (≥ 0.99 vs FP32)
- Cross-lingual F1 ≥ 0.80 (D26 fixture)
- Confidence threshold per-tenant (operator-tunable)
- Drift detection via D13 dashboard (`d13_classify_drift` runbook)

### 4.3 Bias mitigation

- Inherited model card disclosures shipped with bundle (`README.md`)
- Customer responsibility :
  - Test against their data
  - Adjust `confidence_threshold` per class
  - Add adversarial training examples if needed
  - Implement human review for high-stakes decisions
- Operator may freeze bundle version if drift detected

### 4.4 Data retention

| Data class | Retention | Storage |
|---|---|---|
| Audit log | 7 years | Object Lock COMPLIANCE (D17) |
| Operational data (catalog fitting sentences) | Tenant-defined, default 30 days | Postgres TDE |
| PII | Hashed pre-storage (D13 `EFE_PII_HASH=true`) | N/A (hashed) |
| Backup | Per D17 retention policy | S3 versioned |
| Bundles | Immutable, OCI registry | OCI tags immutable |

### 4.5 Data minimization

- Embedding outputs : 384-dim float — no PII reconstruction possible
- Catalog metadata : tenant-provided, no inference
- Audit log : action+actor only, no payload content (only references)

---

## §5 Human oversight measures (mandatory per Art. 14)

### 5.1 Built-in oversight features

- **Admin RPC for model rollback** (M12 D27 bundle reload) — operator
  can revert to a previous bundle version without redeploy.
- **Manual catalog approval** (D12) — new catalog versions require
  operator `Register` RPC call ; not auto-promoted.
- **Audit log per inference** (D13) — all `Encode` and `Classify`
  calls logged with tenant_id + timestamp + class_predicted.
  Operator can query audit log to spot drift.
- **Confidence threshold** — outputs below `confidence_min` flagged
  as uncertain ; operator decides downstream action.
- **Rate limit per tenant** (M11) — bounds resource consumption.

### 5.2 Operator training

EFE 1.0 GA ships :
- `docs/operator_guide.md` — day-1 orientation
- `docs/runbooks/` — 30 scenarios
- `docs/HANDOFF_OPS.md` — escalation tree + ownership matrix

### 5.3 Disengagement procedure

Helm chart M12 supports rollback via `helm rollback efe <REVISION>`
in < 60 seconds. Documented in `docs/runbooks/m12_helm_upgrade_rollback.md`.

### 5.4 Override capability

Operator can :
- Force-rollback to previous bundle version
- Freeze tenant data (read-only mode)
- Disable specific classes from catalog
- Adjust confidence thresholds at runtime

---

## §6 Predetermined changes for which the system was designed

### 6.1 Bundle versioning (M13)
- Semver `MAJOR.MINOR.PATCH` per bundle slug
- Backward-compat within MAJOR
- Performance metrics tracked per release
- Regression > 5 % blocks release (CI gate)

### 6.2 Catalog versioning (D12)
- Per-tenant immutable catalog versions
- `Register` creates new version, never mutates existing
- `Evolve` re-fits on additional sentences
- Operator manual approval gate

### 6.3 Configuration changes
- Helm values controlled via Git + 2-reviewer rule
- `helm upgrade` rolls forward ; `helm rollback` reverts
- All values changes audited in D14-bis chain

### 6.4 Performance change tracking

| Version | Bundle | F1 (cross-lingual) | P95 latency | Cosine vs FP32 |
|---|---|---|---|---|
| 1.0.0 D20 | all-MiniLM-L6-v2 | N/A (English-only) | < 5 ms | 0.993 |
| 1.0.0 D26 | paraphrase-multilingual-MiniLM-L12-v2 | 0.85 (5 lang × 9 sentences) | < 12 ms | 0.999 |

---

## §7 Post-market monitoring system

### 7.1 Continuous monitoring

- Prometheus M11 metrics :
  - `efe_classify_total{tenant,outcome}`
  - `efe_classify_confidence_histogram{tenant}`
  - `efe_classify_drift_score{tenant}` (D13)
- Grafana D13 dashboards (6 dashboards)
- 10 alerts D13 + runbooks

### 7.2 Drift detection

- `D13ObsClassifyDrift` alert fires when distribution divergence
  > threshold (KL-divergence > 0.3 over 1h window)
- Operator runbook `d13_classify_drift.md` triggers re-fit

### 7.3 Customer feedback channels

- Support : support@efe.ai
- Bug bounty (post-1.0 GA) : HackerOne private
- Status page : status.efe.ai
- Vulnerability disclosure : `docs/security/vulnerability_disclosure.md`

### 7.4 Incident reporting (Art. 73 of AI Act)

- Serious incidents reported to EU AI Office within 15 days
  (per upcoming EU implementing acts)
- Customer notification via SLA matrix
- Public statement for material incidents

---

## §8 Cybersecurity measures (Art. 15)

### 8.1 Data integrity

- HMAC chain audit log (D14-bis) — tamper-evident
- Bundle Ed25519 signature (M13) — supply chain integrity
- Cosign image signature — image integrity
- SBOM endpoint (M14 D40) — public verifiable

### 8.2 Access control

- OIDC JWT (D15) + JWKS rotation
- RBAC per-tenant (D11)
- K8s SecurityContext + Polaris GREEN (M12)
- NetworkPolicy strict (M12 D21)

### 8.3 Resilience

- HPA D9 (3-30 replicas)
- PodDisruptionBudget D18
- Multi-replica + leader election D18
- Backup D17 + DR drill D40
- SLO M11-RPC-3 P99 < 200 ms

### 8.4 Adversarial robustness

- Input length cap (`max_seq_len` per bundle, default 256)
- Rate limit per tenant (M11)
- Cardinality bomb mitigation (D13 alert + label allowlist)
- Confidence threshold rejection (operator-tunable)

### 8.5 Supply chain security

- SBOM CycloneDX 1.5 per bundle (M13 + M14 endpoint)
- Bundle Ed25519 sig + Rekor transparency log
- License gate (`license_gate.py`) refuses non-redistributable
- Image cosign + transparency log
- Reproducibility check (CI builds 2× and compares SHA256)

---

## §9 Article 52 Transparency obligations

EFE is a limited-risk AI system. Customers deploying EFE must :
1. Inform end-users they're interacting with an AI system
2. Display this information in a clear and distinguishable manner
3. Comply with Art. 52(1) on AI-generated content (if applicable —
   not applicable to embeddings/classifications which don't generate
   text/audio/video for users)

EFE provides the AI-system metadata via gRPC response headers :
- `efe-ai-system: true`
- `efe-ai-version: 1.0.0`
- `efe-ai-confidence: <float>`

---

## §10 Conformity assessment

Limited-risk AI systems do NOT require conformity assessment under
the AI Act. However, EFE voluntarily maintains :
- This Annex IV documentation (this file)
- ISO 27001 certification (post-1.0 GA)
- SOC 2 Type II report (post-1.0 GA)

---

## References

- Regulation (EU) 2024/1689 (AI Act)
- ISO/IEC 42001:2023 (AI management systems — voluntary)
- NIST AI Risk Management Framework (AI RMF 1.0)
- OECD AI Principles
- EU AI Office implementing acts (forthcoming)
