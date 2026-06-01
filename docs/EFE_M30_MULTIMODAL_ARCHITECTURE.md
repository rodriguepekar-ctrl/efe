# EFE M30 — Multi-modal + GPU Serving + Multi-region Architecture

**Status** : ARCHITECTURE — design phase, pre-execution.
**Predecessors** :
- `EFE_M29_DATA_RIGHTS_ARCHITECTURE.md` (PII redaction → GPU multi-modal must respect).
- `EFE_M28_STREAMING_EVENTS_ARCHITECTURE.md` (NATS replication design).
- `EFE_M27_COMMERCIAL_EDGE_ARCHITECTURE.md` (Redis replication design).
- `EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md` (LLMRouter multi-provider).
**Target** : extend EFE from **text-only classification** to **multi-modal**
(vision + audio + long-context) on **GPU-accelerated** inference, deployed
in **multi-region active-passive** topology with **quarterly DR drill**.
**Codename** : "Constellation" (multiple perception modes orbiting a
single semantic core, with regional satellites for resilience).
**EFE version** : `2.5.0` — semver MINOR. New capabilities, zero
breaking surface on text classify.
**Sprint window** : 5-7 working weeks (the largest milestone since M26).
**Hard deps** : M27 (Envoy edge + Redis), M28 (NATS), M29 (PII redaction
must apply to image OCR + audio transcripts), cert-manager, NVIDIA GPU
nodes (g4dn / g5 / a10g class), Triton Inference Server 24.x, ≥ 2
geographic regions in the same cloud (e.g. AWS eu-west-1 + eu-west-3).

---

## Table of Contents

1. [Executive summary](#1-executive-summary)
2. [Scope + non-goals](#2-scope--non-goals)
3. [What M27-M29 left behind](#3-what-m27-m29-left-behind)
4. [Threat model](#4-threat-model)
5. [Topology overview](#5-topology-overview)
6. [Component A — Triton Inference Server (GPU pool)](#6-component-a--triton-inference-server-gpu-pool)
7. [Component B — Vision embedder (CLIP / SigLIP)](#7-component-b--vision-embedder-clip--siglip)
8. [Component C — Audio embedder (Whisper)](#8-component-c--audio-embedder-whisper)
9. [Component D — Long-context hierarchical chunking](#9-component-d--long-context-hierarchical-chunking)
10. [Component E — Multi-modal fusion API](#10-component-e--multi-modal-fusion-api)
11. [Component F — Multi-region active-passive](#11-component-f--multi-region-active-passive)
12. [Component G — KMS keyring cross-region replication](#12-component-g--kms-keyring-cross-region-replication)
13. [Component H — Region-aware DNS + routing](#13-component-h--region-aware-dns--routing)
14. [Component I — DR drill harness (quarterly)](#14-component-i--dr-drill-harness-quarterly)
15. [SQL migrations introduced](#15-sql-migrations-introduced)
16. [New crates : `efe-multimodal` + `efe-region`](#16-new-crates--efe-multimodal--efe-region)
17. [Helm chart additions](#17-helm-chart-additions)
18. [Observability matrix](#18-observability-matrix)
19. [SLO definitions + alerts](#19-slo-definitions--alerts)
20. [Performance budget + load test plan](#20-performance-budget--load-test-plan)
21. [Failure modes + degradation paths](#21-failure-modes--degradation-paths)
22. [Migration plan M29 → M30](#22-migration-plan-m29--m30)
23. [Testing strategy](#23-testing-strategy)
24. [Runbooks (5 new)](#24-runbooks-5-new)
25. [ADRs (6)](#25-adrs-6)
26. [Risk register](#26-risk-register)
27. [Sprint plan (5-7 weeks)](#27-sprint-plan-5-7-weeks)
28. [Done definition](#28-done-definition)
29. [Annexes](#29-annexes)

---

## 1. Executive summary

After M29, EFE is a production-grade text classification + agent
runtime with edge protection, event bus, data-rights self-service, and
audited supply chain. But three structural gaps remain :

1. **Text-only** : 60-80% of enterprise classification use cases need
   images (document classification, content moderation, defect
   detection) or audio (call triage, voice intent). Vector search
   (M31) and adjacent products need vision + audio embeddings too.
2. **CPU-only** : Mode-1 fusion runs on CPU. At 1000 RPS sustained per
   pod, classify p99 ~80ms. With GPU + Triton + batched inference,
   we can hit 10k+ RPS per GPU at p99 < 20ms — and unlock vision
   models that are CPU-prohibitive.
3. **Single-region** : the entire platform runs in one AZ-redundant
   region. A regional outage (AWS eu-west-1 incident 2024-12,
   GCP us-central1 incident 2023-06) takes the platform down.
   Enterprise customers with HA SLAs ≥ 99.95% require multi-region.

M30 — **Constellation** — closes these gaps with **9 components** :

1. **Triton Inference Server pod** with GPU node pool (NVIDIA T4 / A10g),
   shared by every embedding model.
2. **Vision embedder** (CLIP-ViT-B/32 or SigLIP-base) hosted in Triton ;
   exposed via `POST /v1/classify/multimodal` (image + text + optional
   audio).
3. **Audio embedder** (Whisper-tiny → text transcript → existing text
   embedder, OR direct audio embedding via a contrastive audio model).
4. **Long-context hierarchical chunking** : 512-token chunks with mean
   + max pool + cross-encoder rerank for >8K-token documents.
5. **Multi-modal fusion API** : late fusion (concat embeddings + learned
   per-catalog weights), or early fusion via CLIP-like joint embedder.
6. **Multi-region active-passive** : PG primary in `eu-west-1` with
   streaming replica in `eu-west-3` ; Redis Cluster cross-AZ + cross-
   region read replica ; NATS JetStream **mirror** stream in passive
   region.
7. **KMS keyring cross-region replication** : AWS KMS multi-region keys
   (M29 customer BYOK retains its 2-region presence ; EFE-managed
   keyring also replicates).
8. **Region-aware DNS** : Cloudflare Geosteering routes EU customers
   to `eu-west-1`, NA customers to `us-east-1` (future), with
   `<X-EFE-Region>` response header for client diagnostics.
9. **DR drill harness** : quarterly automated test that fails over to
   the passive region, runs synthetic classify, verifies audit chain
   integrity end-to-end, reports RPO/RTO.

Zero breaking API surface. Text-only callers continue to work
unchanged. Multi-modal is an additive endpoint.

---

## 2. Scope + non-goals

### 2.1 In scope (EFE 2.5.0)

- New crate `efe-multimodal` (`crates/efe-multimodal/`) — fusion API +
  chunking + Triton client.
- New crate `efe-region` (`crates/efe-region/`) — region detection +
  routing + cross-region health probes.
- New REST endpoints :
  - `POST /v1/classify/multimodal` (image + text + audio).
  - `POST /v1/embed/vision` (image → embedding ; M31 forward-compat).
  - `POST /v1/embed/audio` (audio → transcript + embedding).
- Vision model : SigLIP-base-patch16-224 (chosen over CLIP — see
  ADR-M30-001). Audio : Whisper-tiny.
- Triton Inference Server pod with NVIDIA k8s-device-plugin.
- Multi-region Helm overlay (`charts/efe-stack/values-eu-west-1.yaml`,
  `values-eu-west-3.yaml`).
- PG streaming replication setup (Patroni or managed RDS Cross-Region
  Read Replica).
- Redis Cluster cross-AZ + cross-region read replicas.
- NATS JetStream stream mirroring active → passive.
- AWS KMS multi-region keys.
- Cloudflare Geosteering rules.
- DR drill harness (`scripts/m30/dr-drill/` ; runs in chaos cluster
  every quarter, alerts on missed runs).
- SQL migration 0024 (region_health, multimodal_catalog_config,
  dr_drill_run + 4 enum values).
- 5 new runbooks + 6 ADRs.

### 2.2 Out of scope (explicitly deferred)

- **Multi-modal RAG** (image-as-query → top-K similar images). M31.
- **Vision generation** (image synthesis). Never — out of EFE's mission.
- **Real-time video classification** (frame-by-frame). M31.5 if demand.
- **Active-active multi-region** (writes to both regions). Out — too
  much consistency complexity. Active-passive only.
- **3+ regions** (e.g. EU + US + APAC). M30 ships exactly 2 (primary +
  DR). M33 if customer demand.
- **Model fine-tuning** (custom CLIP on customer data). Customer can
  bring their own ONNX model and we host it in Triton ; full fine-tune
  pipeline is M32+.
- **Confidential compute GPU** (Intel TDX + GPU). Niche, M33+.
- **Custom audio encoders** (CLAP, AudioMAE). M31.5 if demand.

### 2.3 Adjacent (parallel sprint candidates)

- **Multi-region SOC 2** evidence collection (P5 parallel).
- **Multi-modal eval harness** with bias datasets per protected
  attribute (parallel ML sprint).
- **Customer-managed GPU dedicated pool** (enterprise upgrade tier).

---

## 3. What M27-M29 left behind

| Gap | Symptom | M30 fix |
|---|---|---|
| **CPU classify ceiling** | 1000 RPS/pod, p99 80ms. Above 5000 RPS cluster-wide cost explodes. | Triton + GPU : 10k RPS/GPU, p99 < 20ms. (§ 6) |
| **No vision** | Customer wants document type classification (invoice / receipt / contract) — impossible. | SigLIP-base in Triton + `/v1/classify/multimodal`. (§ 7) |
| **No audio** | Voice triage use case unsupported. | Whisper-tiny → transcript → existing text path. (§ 8) |
| **8K-token cap** | Long PDFs / contracts > 8K tokens hit the embedder context limit. | Hierarchical chunking : 512-token chunks + cross-encoder rerank. (§ 9) |
| **Single-region SPOF** | AWS eu-west-1 outage = total platform down. | Active-passive in 2 regions + Cloudflare Geosteering failover. (§ 11, 13) |
| **No DR drill** | Recovery procedures untested. Customer asks "show me your last DR" : silence. | Quarterly automated DR drill harness ; runs in chaos cluster. (§ 14) |
| **KMS single-region** | AWS KMS key in `eu-west-1` ; if region down, no decrypt of any column. | Multi-region KMS keys ; replicas in passive region. (§ 12) |
| **No region observability** | `efe_region` label not tracked. Multi-region debugging blind. | Every metric tagged with region label. (§ 18) |

---

## 4. Threat model

| ID | Threat | Vector | Impact | Control (§) |
|---|---|---|---|---|
| T1 | **GPU OOM cascade** | Bad model load / customer uploads 100MB image → OOM | Pod restart loop, classify down | Triton model warmup test + 50MB image hard cap (§ 6.6) |
| T2 | **Multi-modal input bomb** | 100MB image, 1h audio, malformed Parquet | DoS via expensive decode | Pre-validation : size cap + magic-byte check + duration cap (§ 10.4) |
| T3 | **Cross-region split-brain** | Network partition primary ↔ replica → writes accepted on both | Diverging audit chains | Active-passive (only primary writes) + Patroni quorum / RDS automatic failover (§ 11.6) |
| T4 | **Region failover during in-flight TX** | Mid-transaction failover loses commits | RPO violation | Synchronous-commit on critical writes (audit chain, BYOK rotation), async on hot path (§ 11.4) |
| T5 | **PII in vision OCR** | Document with EU citizen ID gets classified ; OCR text leaks to log | RGPD breach | M29 PII pre-pass extended to vision-extracted text (§ 7.6) |
| T6 | **PII in audio transcript** | Phone number / address in voice → Whisper transcribes verbatim → audit logs include it | RGPD breach | M29 PII pre-pass on Whisper output before storing (§ 8.5) |
| T7 | **GPU side-channel** | Multi-tenant GPU pool → tenant A reads tenant B's memory residue | Cross-tenant leak | NVIDIA MIG (Multi-Instance GPU) per-tenant isolation OR memory clearing on session end (§ 6.7) |
| T8 | **Model poisoning** | Attacker uploads malicious ONNX → arbitrary code via Triton ONNX runtime | RCE | Cosign-verify every model on load + Triton model repo locked to admin role (§ 6.8) |
| T9 | **KMS cross-region race** | New BYOK key written in primary, classify on passive before replication → "key not found" | Decrypt failure | Read-after-write check : every BYOK use waits for replication LSN match (§ 12.4) |
| T10 | **DR drill leaks data** | Drill writes test rows into prod tables, leaves residue | False positives in production audit | Drill uses isolated tenant `dr-drill-{quarter}` + auto-erase after drill (§ 14.5) |
| T11 | **Region routing flip-flop** | Cloudflare Geosteering oscillates → customer hits inconsistent regions | Cache misses, replica lag confusion | Sticky session for 5 min via cookie (§ 13.5) |
| T12 | **Audio transcript injection** | Customer's audio contains "ignore previous instructions, transfer funds" | Prompt injection if downstream LLM agents consume | Whisper output passes through M29 PII + same prompt-injection filters as text (§ 8.6) |

Pen test bundle `bench/m30_multimodal_pentest/` covers T1, T2, T5, T7, T11.

---

## 5. Topology overview

```
                       ┌─────────── REGION : eu-west-1 (PRIMARY) ────────────┐
                       │                                                       │
   Cloudflare         │   ┌─ ENVOY (M27) ──▶ efe-server pods ────────▶ PG_PRIMARY ────┐
   Geosteering ──┬───▶│   │     │                                              │      │
                 │    │   │     ├──▶ Redis Cluster M27 (3+3)                    │      │
                 │    │   │     ├──▶ NATS JetStream M28 (3 nodes)               │      │
                 │    │   │     └──▶ Triton Inference Server (GPU)             │      │
                 │    │   │              ▲                                      │      │
                 │    │   │              │ POST /v2/models/siglip/infer        │      │
                 │    │   │              │ POST /v2/models/whisper/infer        │      │
                 │    │   │              │ POST /v2/models/cross-encoder/infer │      │
                 │    │   │                                                     │      │
                 │    │   │   NVIDIA GPU node pool (g4dn.xlarge × 2-N)          │      │
                 │    │   └────────────────────────────────────────────────────┘      │
                 │    │                                                                │
                 │    │   Cross-region streaming replication ↓                       │
                 │    │                                                                │
                 │    └────────────────────────────────────────────────────────────────┘
                 │                              │
                 │ (failover when eu-west-1 down)
                 │                              ▼
                 │                       streaming replication (async, target RPO < 5s)
                 │                              │
                 │    ┌─────────── REGION : eu-west-3 (PASSIVE / DR) ────────┐
                 │    │                                                       │
                 └───▶│   ┌─ ENVOY (M27) ──▶ efe-server pods (read-only) ───▶ PG_REPLICA (read)
                      │   │     │                                              │
                      │   │     ├──▶ Redis Cluster (cross-region read replica) │
                      │   │     ├──▶ NATS JetStream mirror stream               │
                      │   │     └──▶ Triton Inference Server (warm standby)    │
                      │   │                                                     │
                      │   │   Triton scaled down to 0 GPUs by default ;          │
                      │   │   K8s HPA spins up on failover detection.            │
                      │   │                                                     │
                      │   └──────────────────────────────────────────────────────┘
                      │                                                         │
                      └──────────────────────────────────────────────────────────┘

   Failover trigger : Cloudflare health check + manual SRE confirmation.
   Recovery objective : RPO < 5s, RTO < 15min (auto), < 5min (chaos drill).
```

---

## 6. Component A — Triton Inference Server (GPU pool)

### 6.1 Why Triton over alternatives

| Criterion | Triton | TorchServe | BentoML | Hugging Face TGI |
|---|:--:|:--:|:--:|:--:|
| NVIDIA-native | ✅ | ❌ | partial | ❌ |
| Multi-framework (ONNX, TensorRT, PyTorch, TF) | ✅ | torch | python | text only |
| Dynamic batching | ✅ | basic | basic | ✅ |
| Model versioning + warm reload | ✅ | basic | basic | basic |
| Prometheus metrics native | ✅ | partial | partial | basic |
| K8s helm chart official | ✅ | community | community | community |
| Rust client | ⚠️ via gRPC | ❌ | ❌ | ⚠️ |

**Decision** : Triton 24.x via gRPC. ADR-M30-002.

### 6.2 Deployment topology

- 2 replicas in primary region, anti-affinity per GPU node.
- StatefulSet (model repository is on shared PVC, not in image).
- GPU node pool : NVIDIA T4 (g4dn.xlarge) for dev/staging,
  NVIDIA A10g (g5.xlarge) for production.
- HPA on `nv_gpu_utilization` from DCGM metrics : scale up at 70%,
  scale down at 20% with 10min cooldown.

### 6.3 Model repository layout

```
/models/
├── siglip-base-patch16-224/
│   ├── config.pbtxt
│   └── 1/                          # version 1
│       └── model.onnx
├── whisper-tiny/
│   ├── config.pbtxt
│   └── 1/
│       └── model.onnx
├── cross-encoder-ms-marco/         # M30 long-context rerank
│   ├── config.pbtxt
│   └── 1/
│       └── model.onnx
└── README.md
```

The model repo lives on a ReadWriteMany PVC (NFS or EFS) ; both
replicas mount it. Model uploads go through a controlled CLI
(`efe-admin triton model upload`) that cosign-verifies the ONNX bytes.

### 6.4 `config.pbtxt` example (siglip)

```
name: "siglip-base-patch16-224"
platform: "onnxruntime_onnx"
max_batch_size: 32
input [
  { name: "pixel_values" data_type: TYPE_FP32 dims: [ 3, 224, 224 ] }
]
output [
  { name: "image_embeds" data_type: TYPE_FP32 dims: [ 768 ] }
]
dynamic_batching {
  preferred_batch_size: [ 8, 16, 32 ]
  max_queue_delay_microseconds: 5000
}
instance_group [
  { count: 1 kind: KIND_GPU }
]
```

### 6.5 GPU node pool config

```yaml
# Terraform — AWS managed node group
node_groups:
  efe-gpu:
    instance_types: ["g5.xlarge"]
    ami_type: "AL2_x86_64_GPU"
    capacity_type: "ON_DEMAND"
    min_size: 0     # scaled to 0 in DR region
    max_size: 5
    labels:
      "nvidia.com/gpu": "true"
      "efe.ai/role": "inference"
    taints:
      - key: "nvidia.com/gpu"
        value: "present"
        effect: "NO_SCHEDULE"
```

Only Triton pods tolerate the taint ; CPU workloads stay on the
regular node pool.

### 6.6 Input safety

- Image : max 10 MB (configurable), magic bytes verified
  (`image/jpeg`, `image/png`, `image/webp` only — no SVG, no TIFF
  with embedded exec), max 4096×4096 px.
- Audio : max 30 MB, max 5 min duration, formats : wav, mp3, ogg, m4a.
- Pre-validation in `efe-multimodal::input_guard` before reaching
  Triton.

### 6.7 GPU isolation (T7 mitigation)

Two strategies, picked per tier :
- **Shared pool** (free / starter tier) : MIG disabled, GPU memory
  cleared on every request via `cudaMemset` invoked by a small Triton
  pre-processor.
- **Dedicated MIG slice** (enterprise tier) : NVIDIA MIG with per-
  tenant 1g.5gb slice. Stricter isolation, fixed memory ceiling.
  Tenant-pinned via node label + tolerations.

### 6.8 Model integrity (T8 mitigation)

Every ONNX file in the repository is :
1. Signed by cosign (release key) at build time.
2. Verified on Triton pod startup via init container.
3. SHA256 published in the trust center page.
4. New uploads (via `efe-admin triton model upload`) require
   cosign-verify + admin role + double-confirm.

---

## 7. Component B — Vision embedder

### 7.1 Model choice : SigLIP over CLIP

| Property | CLIP-ViT-B/32 | SigLIP-base-patch16-224 |
|---|---|---|
| Released | OpenAI 2021 | Google 2023 |
| Embedding dim | 512 | 768 |
| Loss function | softmax contrastive | **sigmoid contrastive** |
| Multilingual support | English-leaning | better (mSigLIP variant for FR) |
| License | MIT (model card OK) | Apache 2.0 |
| Throughput (T4) | 200 img/s | 180 img/s |

**Decision** : SigLIP-base-patch16-224. Slightly slower but better
EU-language support + Apache license (CLIP's official model is MIT
but the training data has commercial ambiguity).

ADR-M30-001 covers the rationale.

### 7.2 Pipeline

```
POST /v1/classify/multimodal
  { tenant_id, image_url | image_base64, text?, audio_url?,
    catalog_id, mode }
        │
        ▼
  input_guard : magic bytes + size + dim check
        │
        ▼
  fetch image (from URL via presigned S3, or decode base64)
        │
        ▼
  preprocess : resize to 224×224, normalize per SigLIP stats
        │
        ▼
  Triton call : POST /v2/models/siglip-base-patch16-224/infer
        │
        ▼
  embedding 768-dim
        │
        ▼
  optional : ocr_text = pytesseract.image_to_string(image)
              → M29 PII pre-pass on ocr_text
        │
        ▼
  fusion (Component E)
        │
        ▼
  classify via Mode-1 / Mode-2 / Mode-3
        │
        ▼
  response : { class_id, confidence, ocr_text_redacted? }
```

### 7.3 OCR optional pass (vision → text)

For `mode=2` (LLM-routed) on document classification, we extract OCR
text via Tesseract (FR + EN languages installed in efe-server image)
and feed it to the LLM router alongside the vision embedding. The
OCR text passes through M29's PII redactor before any logging.

### 7.4 Per-tenant model selection

Some customers may want CLIP for compatibility with their existing
catalogs trained against CLIP embeddings. Per-tenant override :

```sql
ALTER TABLE tenant_feature
    ADD COLUMN vision_model VARCHAR(64) NOT NULL DEFAULT 'siglip-base-patch16-224';
```

Allowed values : `siglip-base-patch16-224`, `clip-vit-b-32`,
`clip-vit-l-14` (the last only on dedicated MIG slice).

### 7.5 Catalog compat

A catalog trained against text-only embeddings (384 dims, sentence-
transformers) **cannot directly** consume vision 768-dim embeddings.
Three options :
1. **Catalog is text-only** : reject multimodal request (HTTP 422,
   `catalog_not_multimodal`).
2. **Catalog is vision-only** : accept image, reject text part if
   `mode=1` (no fusion).
3. **Catalog is multi-modal** (new in M30) : accept image + text,
   fuse via Component E.

The `catalog_modality` SQL column gates which is which.

### 7.6 PII in OCR text

Critical : OCR can extract names, IDs, addresses from documents. The
M29 PII pre-pass MUST run on OCR text before :
- Logging to `classify_call_log`.
- Sending to LLM router (Mode-2).
- Surfacing in classify response.

Enforced in `efe-multimodal::vision::classify_one`.

---

## 8. Component C — Audio embedder

### 8.1 Approach : Whisper-tiny + text path

Rather than a dedicated audio embedder (CLAP, AudioMAE), M30 ships
Whisper-tiny for transcript extraction, then routes the text through
the existing Mode-1/2/3 path. Rationale :
- Most enterprise audio use cases (call triage, voice intent) are
  effectively "what is being said" — text is the canonical
  representation.
- A dedicated audio embedder (CLAP) opens a new modality dimension we
  don't yet have catalogs for. Out of scope.
- Whisper-tiny is small (75M params), fast on GPU (10× real-time on T4).

### 8.2 Pipeline

```
POST /v1/embed/audio
  { tenant_id, audio_url | audio_base64, language? }
        │
        ▼
  input_guard : magic bytes (RIFF for wav, ID3 for mp3, OggS) +
                size cap 30MB + duration cap 5min (mediainfo probe)
        │
        ▼
  Triton call : POST /v2/models/whisper-tiny/infer
                input : raw audio (resampled to 16kHz mono Float32)
        │
        ▼
  transcript : str
        │
        ▼
  M29 PII pre-pass on transcript
        │
        ▼
  existing text embedder (Mode-1 backend) → 384-dim
        │
        ▼
  response : { transcript_redacted, embedding, language_detected }
```

### 8.3 Why not direct contrastive audio model

CLAP (Contrastive Language-Audio Pretraining) would give us a 1024-dim
audio embedding directly, no transcription step. But :
- Catalogs would need separate training.
- Most use cases are transcript-based (no point losing fidelity to text).
- Customers can always classify the transcript via existing text path.

Future M31.5 : add CLAP for audio-similarity search (find similar
sounding calls).

### 8.4 Resampling

Whisper expects 16 kHz mono Float32. Customer uploads at any sample
rate / channel count. We resample in `efe-multimodal::audio::resample`
using `rubato` Rust crate (no Python in the hot path).

### 8.5 PII in transcripts

Same as vision OCR : M29 redaction before logging + LLM router.
Phone numbers, names, addresses spoken in audio are common.

### 8.6 Prompt-injection defense

Audio + text agent runs (M25/M26) : if a customer's audio contains
"ignore previous instructions" or similar, the agent step approval
chain (M25 Art. 14) catches it before any tool execution. M30 adds a
**transcript filter** that detects injection patterns via a small
regex set + heuristic LLM call (cached).

---

## 9. Component D — Long-context hierarchical chunking

### 9.1 Problem

Text embedders have context limits :
- sentence-transformers/all-MiniLM-L6-v2 (current) : 256 tokens.
- OpenAI text-embedding-3-small : 8K tokens.
- BGE-large : 512 tokens.

Customer uploads a 50K-token contract → today we truncate. M30 ships
hierarchical chunking.

### 9.2 Algorithm

```
1. Tokenize input.
2. Split into 512-token chunks with 64-token overlap.
3. Embed each chunk → list of 384-dim vectors.
4. Aggregate :
     - mean_pool = mean(chunks)
     - max_pool = elementwise max(chunks)
     - top-k chunks by self-attention to a query (if query provided)
5. For classify with mode=1 :
     - Use mean_pool as the document embedding.
6. For classify with mode=2 (LLM-routed) :
     - Run cross-encoder rerank between query + each chunk → keep top 5.
     - LLM router sees the 5 chunks (truncated to context).
7. Return classify decision + chunks_used.
```

### 9.3 Cross-encoder model

`cross-encoder/ms-marco-MiniLM-L-6-v2` hosted in Triton (same
inference server as siglip). 22M params, fast on CPU but we put it on
GPU for batched throughput.

### 9.4 When to chunk

```
input_length_tokens > 512 → chunk
input_length_tokens ≤ 512 → embed directly
```

Detected via the embedder's tokenizer (loaded in efe-server once).

### 9.5 Storage

Each chunk's embedding is NOT individually persisted in M30 (would
multiply storage 10-100×). Only the aggregate is stored alongside
`classify_call_log`. The full chunk embeddings are exposed via
`POST /v1/embed/text?return_chunks=true` for customers who want them
for downstream vector search (M31 forward-compat).

---

## 10. Component E — Multi-modal fusion API

### 10.1 Fusion strategies

| Strategy | Dimension | When |
|---|---|---|
| **Concat + linear** | dim_text + dim_vision (e.g. 384+768=1152) → catalog learned projection to 384 | Default for new multimodal catalogs |
| **Weighted mean** | max(dim_text, dim_vision) — embed both, project both to 384, weighted average | Quick fusion without retraining catalog |
| **CLIP joint** | 512 (CLIP) or 768 (SigLIP) | Catalog trained on SigLIP joint space ; image OR text query both work |
| **Early fusion via LLM-routed (mode 2)** | LLM router sees vision_embed + text → outputs anchor selection | Document understanding tasks |

ADR-M30-003 covers the trade-offs.

### 10.2 Per-catalog config

```sql
CREATE TABLE multimodal_catalog_config (
    catalog_id      TEXT NOT NULL,
    fusion_strategy VARCHAR(32) NOT NULL DEFAULT 'concat_linear',
    vision_weight   NUMERIC(4,3) NOT NULL DEFAULT 0.500
                    CHECK (vision_weight BETWEEN 0.0 AND 1.0),
    text_weight     NUMERIC(4,3) NOT NULL DEFAULT 0.500
                    CHECK (text_weight BETWEEN 0.0 AND 1.0),
    audio_weight    NUMERIC(4,3) NOT NULL DEFAULT 0.000,
    PRIMARY KEY (catalog_id),
    CONSTRAINT weights_sum CHECK (
        abs(vision_weight + text_weight + audio_weight - 1.0) < 0.001
    )
);
```

### 10.3 API

```
POST /v1/classify/multimodal
  Headers : Authorization: Bearer <jwt>, Content-Type: application/json
  Body :
    {
      "tenant_id": "acme",
      "catalog_id": "contracts-v3",
      "text": "Long contract excerpt ...",   // optional
      "image_url": "s3://acme/doc1.png",      // optional
      "audio_url": "s3://acme/voicemail.mp3", // optional
      "mode": 1
    }
  At least ONE of text/image/audio required.

Response :
    {
      "class_id": "contract.nda",
      "confidence": 0.87,
      "fusion": {
        "strategy": "concat_linear",
        "components": ["text", "image"]
      },
      "ocr_text_redacted": "[REDACTED:PERSON] signed on [REDACTED:DATE]",
      "audio_transcript_redacted": null,
      "embeddings_dim": 1152
    }
```

### 10.4 Input pre-validation

Done in `efe-multimodal::input_guard` :
- Image URL must be in tenant's S3 allow-list (M28 reuse) OR a
  base64 inline image ≤ 10 MB.
- Audio URL : same allow-list, base64 cap 30 MB.
- Text : ≤ 100K chars (chunking handles ; cap is sanity).
- At least one modality present.

### 10.5 Per-modality rate limits

`tenant_feature` extensions :
```sql
ALTER TABLE tenant_feature
    ADD COLUMN classify_vision_quota_per_min INTEGER NOT NULL DEFAULT 100,
    ADD COLUMN classify_audio_quota_per_min INTEGER NOT NULL DEFAULT 100;
```

Vision + audio are GPU-expensive ; per-minute caps prevent runaway
cost.

---

## 11. Component F — Multi-region active-passive

### 11.1 Topology

```
                Cloudflare Geosteering (DNS-level routing)
                            │
            ┌───────────────┴───────────────┐
            ▼                                ▼
    eu-west-1 (PRIMARY)          eu-west-3 (PASSIVE)
       │                              │
       │ active writes                │ read-only (failover targets)
       ├──▶ efe-server                ├──▶ efe-server (limited replicas)
       ├──▶ Triton GPU pool           ├──▶ Triton GPU pool (scaled to 0)
       ├──▶ Redis Cluster M27         ├──▶ Redis read replica (cross-region)
       ├──▶ NATS JetStream M28        ├──▶ NATS mirror stream
       ├──▶ PG primary                ├──▶ PG streaming replica
       └──▶ KMS keyring (multi-region)└──▶ KMS keyring (replica)
```

### 11.2 PG replication

Two options :
- **Managed (RDS / Cloud SQL)** : enable cross-region read replica
  feature. AWS RDS handles WAL shipping, conflict-free. Production
  default.
- **Self-managed (Patroni)** : Patroni cluster with synchronous
  streaming replication across regions. More flexible but heavier ops.

Decision : start managed (RDS Cross-Region Read Replica). Patroni
deferred to M33 if self-hosted requirement materialises.

### 11.3 Replication lag SLO

- Async, target p99 < 5 s.
- Alert at p99 > 10 s for 5 min → P1 PagerDuty.
- At p99 > 60 s for 30 min → auto-failback the `EFE_FALLBACK_RO_TO_RW`
  flag (M27 mechanism extended cross-region).

### 11.4 Writes during failover

- **Synchronous writes** (audit chain HMAC, BYOK rotation, deletion
  cascade) : require ACK from at least 1 replica → at most 1s lag.
- **Async writes** (classify_call_log, hot path) : eventual ; up to
  5s lag acceptable.

`synchronous_standby_names = 'ANY 1 (eu-west-3-replica)'` for the
synchronous tier.

### 11.5 Redis cross-region

NOT replicated synchronously — that would add inter-region latency to
every cache GET. Instead :
- Primary region Redis Cluster (M27 unchanged).
- Passive region runs its own Redis Cluster.
- After failover, the passive cache is **cold** ; classify falls back
  to recompute. SLO degrades but service stays up.

### 11.6 NATS replication via JetStream Mirror

```
# Passive region : mirror configuration
streams:
  - name: EVENTS_TENANT_acme_mirror
    mirror:
      name: EVENTS_TENANT_acme
      external:
        api: $JS.api-from-eu-west-1
```

This is a **read-only mirror** in passive region. After failover,
the mirror becomes the new primary stream (manual promotion) and
event publishers redirect to it.

### 11.7 Split-brain prevention

Active-passive (not active-active) avoids the split-brain problem by
construction : only one region writes. The failover is intentional
(SRE-triggered or auto-via-Cloudflare-health) and includes a
**fencing token** check in PG (advisory lock that survives
restarts) before the passive promotes to primary.

---

## 12. Component G — KMS keyring cross-region replication

### 12.1 AWS KMS multi-region keys

AWS KMS supports **multi-region keys** since 2021. A single key
identifier (`mrk-<id>`) is replicated to N regions ; same key material,
same ciphertext compatibility. Decrypts work in any region.

We use multi-region keys for :
- Audit chain HMAC keys (already shipped by M29).
- BYOK customer keyrings (M29 architecture supports ; M30 adds the
  multi-region replication setup script).
- TLS certificates (already handled by cert-manager).

### 12.2 Replication lag

AWS KMS multi-region keys replicate via AWS's internal infrastructure
in ~minutes. For BYOK rotation, this can cause a race :
- T+0 : new key written in primary region.
- T+0 : classify happens in primary, uses new key. OK.
- T+0+30s : failover to passive.
- T+0+30s : classify in passive ; key not yet replicated. **DECRYPT FAILS.**

### 12.3 Mitigation : read-after-write LSN check

After a BYOK rotation, the rotation worker (M29) calls AWS KMS
`DescribeKey` on the passive region with retry loop until the key is
visible. Only then is the rotation marked `done` in PG.

For audit chain HMAC keys (rotated quarterly), the same pattern applies.

### 12.4 Customer BYOK in multi-region

Customers can either :
- **Provide one key per region** (most common ; their key management).
- **Trust EFE to replicate** (via STS AssumeRole into customer's
  account that creates a multi-region key).

ADR-M30-005 covers the policy.

---

## 13. Component H — Region-aware DNS + routing

### 13.1 Cloudflare Geosteering

Cloudflare Enterprise plan includes Geo Steering : map continent /
country → origin pool. We define :

```
geo_pools:
  - name: "eu-pool"
    origins:
      - "https://api-eu-west-1.efe.ai"   # primary
      - "https://api-eu-west-3.efe.ai"   # failover (lower priority)
    health_check: enabled
    routing: failover  # not load-balance, fail to next
  - name: "na-pool"   # future M33
    origins:
      - "https://api-us-east-1.efe.ai"

geo_rules:
  - if: "country in ['FR', 'DE', 'IT', 'ES', ...]"
    pool: "eu-pool"
  - else:
    pool: "eu-pool"  # default for M30 (NA pool added M33)
```

### 13.2 Health checks

Cloudflare pings `https://api-<region>.efe.ai/healthz` every 30s.
After 3 consecutive failures, the origin is marked unhealthy and
traffic shifts to the next priority.

### 13.3 Region response header

Every response includes `X-EFE-Region: eu-west-1` (added by Envoy).
Customers can verify routing visually.

### 13.4 Client-side fallback

For SDK clients (Python, Go, Java, TS), the SDK auto-detects the
region header and pins subsequent requests for 5 minutes (sticky
region cookie). Prevents flip-flop during partial outages.

### 13.5 Sticky session

When a customer is mid-flow (uploaded image, polling classify result),
they should not flip regions. Cloudflare offers session affinity via
cookie ; we set `EFE_REGION=eu-west-1` for 5 min.

### 13.6 DNS records

```
api.efe.ai           → CF proxy → geosteer → pool selection
api-eu-west-1.efe.ai → direct → Argo tunnel eu-west-1
api-eu-west-3.efe.ai → direct → Argo tunnel eu-west-3
```

Argo tunnel per region (M27 pattern extended).

---

## 14. Component I — DR drill harness (quarterly)

### 14.1 What it does

Every quarter (cron Q1/Q2/Q3/Q4), the DR drill harness runs in the
**chaos cluster** (separate from production) and exercises the full
failover flow :

1. **Setup** : provision a fresh primary in `eu-west-1` + replica in
   `eu-west-3`. Seed 1000 synthetic classify rows.
2. **Trigger** : simulate region-1 outage by NetworkPolicy denial of
   the primary PG.
3. **Detection** : Cloudflare health check fails 3× → failover.
4. **Promotion** : RDS replica promoted to primary (CLI script).
5. **Reconfiguration** : efe-server pods in eu-west-3 detect new
   write-pool DSN, start accepting writes.
6. **Sanity** : 100 synthetic classify calls succeed in passive region.
7. **Audit chain verify** : run scripts/m29/verify_erasure_complete
   equivalent (chain integrity check end-to-end).
8. **Failback** : after 1h, restore primary, re-replicate, demote
   passive.
9. **Report** : RPO observed, RTO observed, errors encountered, time
   to first successful classify in passive. Persist in `dr_drill_run`
   table + push to NATS `efe.system.dr.drill.completed`.

### 14.2 Schedule

```
GH Actions cron : 0 4 1 1,4,7,10 *  # 04:00 UTC on day 1 of Jan/Apr/Jul/Oct
```

Or manually via `efe-admin dr drill run --region eu-west-3`.

### 14.3 Isolation

The drill uses a dedicated chaos cluster + tenant ID
`dr-drill-{quarter}-{year}`. The fake data is auto-erased after the
drill via M29 erasure cascade. Production NEVER touched.

### 14.4 SLO

- **RPO** (max data loss) : ≤ 5 s. Verified by counting writes
  accepted by the primary in the last 5s before fence vs. writes
  visible in passive after promotion.
- **RTO** (max time to recovery) : ≤ 15 min auto, ≤ 5 min with
  warm passive Triton + pre-scaled passive efe-server.

### 14.5 Cleanup

After each drill, the harness :
1. Drops the synthetic tenant.
2. Verifies via `scripts/m29/verify_erasure_complete.sql` that all
   rows are gone.
3. Pushes a final `dr.drill.completed` event with the metrics.

---

## 15. SQL migrations introduced

### 15.1 Migration 0024 — `m30_multimodal_and_region.sql`

```sql
BEGIN;

-- ── 1. Catalog modality + multi-modal config ─────────────────────
ALTER TABLE catalog_version
    ADD COLUMN IF NOT EXISTS modality VARCHAR(16) NOT NULL DEFAULT 'text'
    CHECK (modality IN ('text','vision','audio','multimodal'));

CREATE TABLE IF NOT EXISTS multimodal_catalog_config (
    catalog_id          TEXT PRIMARY KEY,
    fusion_strategy     VARCHAR(32) NOT NULL DEFAULT 'concat_linear'
                        CHECK (fusion_strategy IN ('concat_linear','weighted_mean','clip_joint')),
    vision_weight       NUMERIC(4,3) NOT NULL DEFAULT 0.500,
    text_weight         NUMERIC(4,3) NOT NULL DEFAULT 0.500,
    audio_weight        NUMERIC(4,3) NOT NULL DEFAULT 0.000,
    cross_encoder_model VARCHAR(128) NOT NULL DEFAULT 'cross-encoder-ms-marco',
    CONSTRAINT weights_sum CHECK (
        abs(vision_weight + text_weight + audio_weight - 1.0) < 0.001
    ),
    CONSTRAINT weights_nonneg CHECK (
        vision_weight >= 0 AND text_weight >= 0 AND audio_weight >= 0
    )
);

-- ── 2. Per-tenant model selection ────────────────────────────────
ALTER TABLE tenant_feature
    ADD COLUMN IF NOT EXISTS vision_model VARCHAR(64) NOT NULL DEFAULT 'siglip-base-patch16-224',
    ADD COLUMN IF NOT EXISTS audio_model VARCHAR(64) NOT NULL DEFAULT 'whisper-tiny',
    ADD COLUMN IF NOT EXISTS classify_vision_quota_per_min INTEGER NOT NULL DEFAULT 100,
    ADD COLUMN IF NOT EXISTS classify_audio_quota_per_min INTEGER NOT NULL DEFAULT 100,
    ADD COLUMN IF NOT EXISTS gpu_isolation VARCHAR(16) NOT NULL DEFAULT 'shared'
        CHECK (gpu_isolation IN ('shared','mig_1g5gb','mig_2g10gb','dedicated'));

COMMENT ON COLUMN tenant_feature.gpu_isolation IS
    'M30 — GPU isolation tier. shared = pooled. mig_* = NVIDIA MIG slice. '
    'dedicated = whole GPU pinned. Enterprise upgrade.';

-- ── 3. region_health (per-region health probe results) ───────────
CREATE TABLE IF NOT EXISTS region_health (
    region              VARCHAR(32) PRIMARY KEY,
    is_primary          BOOLEAN NOT NULL DEFAULT FALSE,
    last_probe_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_probe_status   VARCHAR(16) NOT NULL DEFAULT 'unknown'
                        CHECK (last_probe_status IN ('healthy','degraded','down','unknown')),
    pg_replica_lag_ms   BIGINT,
    redis_node_count    SMALLINT,
    nats_mirror_lag_ms  BIGINT,
    gpu_pool_size       SMALLINT,
    notes               TEXT
);

INSERT INTO region_health (region, is_primary) VALUES
    ('eu-west-1', TRUE), ('eu-west-3', FALSE)
ON CONFLICT (region) DO NOTHING;

-- ── 4. dr_drill_run ───────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS dr_drill_run (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    quarter             VARCHAR(10) NOT NULL,        -- e.g. "2026-Q2"
    started_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at        TIMESTAMPTZ,
    failover_from       VARCHAR(32) NOT NULL,
    failover_to         VARCHAR(32) NOT NULL,
    rpo_observed_ms     BIGINT,
    rto_observed_ms     BIGINT,
    sanity_calls_ok     INTEGER,
    sanity_calls_total  INTEGER,
    audit_chain_ok      BOOLEAN,
    errors_log          JSONB DEFAULT '[]'::jsonb,
    triggered_by        TEXT NOT NULL,
    status              VARCHAR(16) NOT NULL DEFAULT 'running'
                        CHECK (status IN ('running','passed','failed','cancelled'))
);

CREATE INDEX IF NOT EXISTS idx_dr_drill_quarter
    ON dr_drill_run (quarter, started_at DESC);

-- ── 5. multimodal_input_audit (M30 PII trace for vision OCR + audio) ──
CREATE TABLE IF NOT EXISTS multimodal_input_audit (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    input_kind          VARCHAR(16) NOT NULL CHECK (input_kind IN ('image_ocr','audio_transcript')),
    input_sha256        CHAR(64) NOT NULL,
    pii_detection_count INTEGER NOT NULL DEFAULT 0,
    pii_redacted        BOOLEAN NOT NULL DEFAULT FALSE,
    processed_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_mm_input_audit_tenant_time
    ON multimodal_input_audit (tenant_id, processed_at DESC);

-- ── 6. Audit enum extensions ──────────────────────────────────────
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'MultimodalClassify') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'MultimodalClassify';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'RegionFailoverInitiated') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'RegionFailoverInitiated';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'DrDrillCompleted') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'DrDrillCompleted';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'GpuPoolScaled') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'GpuPoolScaled';
    END IF;
END $$;

COMMIT;
```

---

## 16. New crates : `efe-multimodal` + `efe-region`

### 16.1 `efe-multimodal` crate

```
crates/efe-multimodal/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── error.rs
│   ├── input_guard.rs        // size + magic-byte + duration checks
│   ├── vision.rs              // image preprocess + Triton SigLIP call
│   ├── audio.rs               // audio resample + Whisper call
│   ├── chunking.rs            // long-context hierarchical chunker
│   ├── fusion.rs              // concat_linear + weighted_mean + clip_joint
│   ├── triton_client.rs       // gRPC client to Triton
│   ├── ocr.rs                 // tesseract bindings (optional feature)
│   ├── policy.rs              // multi-modal policy state machine
│   └── metrics.rs
└── tests/integration.rs
```

Key trait :

```rust
#[async_trait]
pub trait MultimodalClassifier: Send + Sync {
    async fn classify(
        &self,
        tenant: &TenantId,
        catalog: &CatalogId,
        input: MultimodalInput,
    ) -> Result<MultimodalOutput, MultimodalError>;
}

pub struct MultimodalInput {
    pub text: Option<String>,
    pub image_bytes: Option<Vec<u8>>,
    pub audio_bytes: Option<Vec<u8>>,
    pub mode: ClassifyMode,
}

pub struct MultimodalOutput {
    pub class_id: String,
    pub confidence: f32,
    pub fusion_applied: FusionStrategy,
    pub ocr_text_redacted: Option<String>,
    pub audio_transcript_redacted: Option<String>,
}
```

### 16.2 `efe-region` crate

```
crates/efe-region/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── current.rs             // detect current region from env
│   ├── health.rs              // probe other regions + lag observability
│   ├── failover.rs            // failover orchestrator (admin CLI)
│   ├── dr_drill.rs            // drill harness
│   └── metrics.rs
```

Public API :

```rust
pub fn current_region() -> Region;
pub fn is_primary() -> bool;
pub async fn probe_passive(&self) -> RegionHealth;
pub async fn initiate_failover(reason: &str) -> FailoverResult;
```

---

## 17. Helm chart additions

### 17.1 New sub-chart : `charts/efe-stack/charts/efe-triton/`

```
charts/efe-stack/charts/efe-triton/
├── Chart.yaml
├── values.yaml
└── templates/
    ├── _helpers.tpl
    ├── triton-statefulset.yaml       (2 replicas, GPU toleration, NFS PVC mount)
    ├── triton-service.yaml            (gRPC + HTTP + metrics ports)
    ├── triton-configmap.yaml          (model repository init script)
    ├── triton-pvc.yaml                (RWX NFS for model repo)
    ├── triton-hpa.yaml                (scale on nv_gpu_utilization)
    ├── triton-networkpolicy.yaml
    └── triton-podmonitor.yaml         (ServiceMonitor for Prometheus)
```

### 17.2 Multi-region values overlays

```
charts/efe-stack/values-eu-west-1.yaml   # primary
charts/efe-stack/values-eu-west-3.yaml   # passive
```

Differences :
- `region: eu-west-1` vs `eu-west-3`.
- `efe-triton.gpuNodeMinSize: 2` vs `0` (passive scales from 0).
- `database.role: primary` vs `replica`.
- `efe-nats.mirror.upstream: eu-west-1` (passive only).

### 17.3 Umbrella integration

`charts/efe-stack/Chart.yaml` adds `efe-triton` sub-chart. New value
`efe-triton.enabled` (default false). Production : on via region
overlay.

---

## 18. Observability matrix

### 18.1 New metrics

| Metric | Type | Labels |
|---|---|---|
| `efe_multimodal_classify_total{tenant,modality_set,catalog}` | counter | |
| `efe_multimodal_classify_duration_seconds{stage}` | histogram | preprocess, triton, fusion |
| `efe_multimodal_input_size_bytes{kind}` | histogram | image, audio |
| `efe_triton_inference_total{model,outcome}` | counter | |
| `efe_triton_inference_duration_seconds{model}` | histogram | |
| `efe_triton_dynamic_batch_size{model}` | histogram | |
| `efe_gpu_utilization_percent{gpu_id,node}` | gauge | from DCGM exporter |
| `efe_gpu_memory_used_bytes{gpu_id,node}` | gauge | DCGM |
| `efe_region_health_status{region}` | gauge (0=down, 1=healthy) | |
| `efe_region_replica_lag_seconds{region}` | gauge | |
| `efe_region_failover_total{from,to,reason}` | counter | |
| `efe_dr_drill_rpo_observed_ms` | gauge | |
| `efe_dr_drill_rto_observed_ms` | gauge | |
| `efe_chunking_chunks_emitted{tenant}` | histogram | |
| `efe_pii_redaction_total{source}` | counter | source = ocr | transcript |

### 18.2 Region label propagation

Every metric in M27, M28, M29 gets an additional label `region` set
from env var `EFE_REGION` (injected by Helm overlay). Existing metrics
acquire `{region=eu-west-1}` automatically.

### 18.3 Dashboard

`ops/grafana/dashboards/m30-multimodal/` :
- Multi-modal overview (modality distribution, latencies, errors).
- Triton GPU panel (utilization, memory, batch size, queue depth).
- Multi-region panel (per-region SLOs side-by-side, replica lag).
- DR drill timeline (last 4 quarters).

---

## 19. SLO definitions + alerts

| ID | SLO | Window | Target |
|---|---|---|---|
| M30-1 | Multi-modal classify p99 latency | 30d | < 200ms |
| M30-2 | Triton inference p99 (GPU path) | 30d | < 50ms |
| M30-3 | GPU utilization (cost efficiency) | 30d | > 50% sustained |
| M30-4 | PG cross-region replica lag p99 | 30d | < 5s |
| M30-5 | Region availability (per region) | 30d | > 99.95% |
| M30-6 | DR drill RPO observed | per-drill | < 5s |
| M30-7 | DR drill RTO observed | per-drill | < 15min |
| M30-8 | Multi-modal classify success rate | 30d | > 99.5% |

PrometheusRule alerts cover each.

---

## 20. Performance budget + load test plan

### 20.1 Targets

- 5000 multi-modal classify req/s cluster-wide (vision-heavy).
- 10000 text classify req/s (unchanged from M27).
- p99 vision-only classify < 100ms (GPU cache hit).
- p99 vision-fusion classify < 200ms (GPU + text + cross-encoder).
- 1 hour audio file end-to-end transcription + classify in < 60s.

### 20.2 k6 scripts

`scripts/load/m30/`:
- `multimodal_vision_5krps.js` — sustained 5k req/s with mixed
  vision-only / vision+text inputs.
- `audio_classify_throughput.js` — 100 audio files (30s avg) → measure
  e2e latency.
- `region_failover_under_load.js` — sustain 1000 RPS while triggering
  Cloudflare failover ; measure dropped requests + recovery time.

### 20.3 Chaos tests

`scripts/chaos/m30/`:
- `kill_triton_pod.sh` — kill one Triton replica ; expect failover < 5s
  via the other replica.
- `gpu_oom_simulation.sh` — submit 100×10MB images simultaneously ;
  verify backpressure + 429.
- `nats_mirror_partition.sh` — partition NATS primary ↔ mirror ;
  verify reconciliation on heal.
- `pg_failover_drill.sh` — invoke the DR drill harness in chaos cluster.

---

## 21. Failure modes + degradation paths

| Failure | Detection | Degraded mode | Recovery |
|---|---|---|---|
| Triton pod crashes | Liveness fail | other replica absorbs ; HPA scales | restart pod |
| Triton OOM on big input | OOM kernel + restart | per-tenant rate limit kicks in, others unaffected | input_guard validates upfront ; problem doesn't recur |
| GPU node terminated (spot interruption) | DaemonSet detection | HPA spawns new node ; ~5min downtime in burst | new node provisions |
| Cross-region PG replica lag spike | PG lag exporter | classify continues from primary ; passive degrades | wait or manual sync |
| Region 1 down | Cloudflare health check | Geosteering routes to region 2 ; passive promoted | wait for region 1 to recover, failback manually |
| Cloudflare global outage | external monitor | direct DNS fallback to one region (no geo) | wait for CF recovery |
| NATS mirror behind | mirror lag metric | webhook delivery from passive replays mirror ; some events delayed | reconcile on heal |
| KMS key not replicated yet on failover | decrypt fails 403 | retry with backoff ; or fail-open if non-critical | wait for replication (minutes) |
| DR drill kicks in production | environment check + tenant gate | impossible : harness checks `EFE_CLUSTER_ROLE=chaos` | n/a |

---

## 22. Migration plan M29 → M30

### Phase 1 (Week 1-2) : Triton + GPU pool
- Install efe-triton Helm chart in staging.
- Upload SigLIP + Whisper + cross-encoder ONNX models.
- Triton smoke tests : 1 image, 1 audio classify.

### Phase 2 (Week 3) : Multi-modal API
- Ship `POST /v1/classify/multimodal` to staging.
- SQL migration 0024.
- Tenant test : classify 100 mixed-modality samples.

### Phase 3 (Week 4) : Long-context + chunking
- Hierarchical chunker live.
- Test with 50k-token contracts.

### Phase 4 (Week 5-6) : Multi-region setup
- Provision PG replica in eu-west-3 (managed RDS).
- Deploy efe-stack in eu-west-3 with values-eu-west-3.yaml.
- Cloudflare Geosteering configured.
- Smoke test : single classify from eu-west-3 (force via direct
  hostname).

### Phase 5 (Week 7) : DR drill + rollout
- First DR drill in chaos cluster.
- 1 design partner pilots multi-modal in production.
- Cloudflare Geosteering enabled cluster-wide.

---

## 23. Testing strategy

### Unit tests
- `efe-multimodal::input_guard` : magic bytes for image/audio,
  size caps, dim caps.
- `efe-multimodal::chunking` : 512-token + overlap edge cases.
- `efe-multimodal::fusion` : weights sum constraint + dim consistency.
- `efe-region::current` : env detection.
- `efe-region::failover` : state machine + fencing.

### Integration tests
- `tests/m30_multimodal_e2e.rs` : Triton mock backend ; classify image,
  classify audio, classify mixed.
- `tests/m30_chunking_50k_tokens.rs` : long-context contract.

### Load tests (§ 20.2).

### Chaos tests (§ 20.3).

### Pen test
- `bench/m30_multimodal_pentest/` :
  - `test_01_image_size_bomb.py` (T1, T2)
  - `test_02_pii_in_ocr_redacted.py` (T5)
  - `test_03_gpu_cross_tenant_leak.py` (T7)
  - `test_04_model_poisoning_rejected.py` (T8)
  - `test_05_region_routing_stable.py` (T11)

---

## 24. Runbooks (5 new)

- `RB_M30_TRITON_DOWN.md` — Triton pod recovery.
- `RB_M30_GPU_OOM.md` — GPU memory exhaustion mitigation.
- `RB_M30_REGION_FAILOVER.md` — manual failover procedure.
- `RB_M30_REPLICA_LAG_HIGH.md` — cross-region lag investigation.
- `RB_M30_DR_DRILL_FAILED.md` — drill failure diagnosis + remediation.

---

## 25. ADRs (6)

- **ADR-M30-001** : SigLIP-base over CLIP-ViT-B/32.
- **ADR-M30-002** : Triton Inference Server over TorchServe / BentoML.
- **ADR-M30-003** : Concat + linear fusion as default ; weighted_mean
  + clip_joint opt-in.
- **ADR-M30-004** : Active-passive 2-region (no active-active in v1).
- **ADR-M30-005** : Customer BYOK in multi-region : customer
  provides one key per region by default.
- **ADR-M30-006** : Quarterly DR drill cadence (vs monthly / annual).

---

## 26. Risk register

| ID | Risk | L | I | Mitigation |
|---|---|:--:|:--:|---|
| R1 | GPU cost overrun (vision-heavy traffic) | MED | HIGH | Per-tenant `classify_vision_quota_per_min` cap + spot instances + scale-to-0 in passive |
| R2 | Triton model upgrade introduces regression | MED | MED | Canary version (e.g. `siglip-base-patch16-224/2`), traffic-shifted via Triton's version routing, rollback if accuracy < baseline |
| R3 | Cross-region replication lag spikes during M24 active learning batch | MED | MED | Active learning runs off-peak ; alert at 10s lag |
| R4 | Cloudflare Geosteering misroutes after failover | LOW | HIGH | Health check 3-fail threshold + manual SRE override flag |
| R5 | DR drill leaves data residue in production | LOW | HIGH | Hard environment gate `EFE_CLUSTER_ROLE=chaos` ; auto-erase after drill |
| R6 | Whisper transcript contains hallucinated content | MED | MED | Mark transcripts as "model output, may be inaccurate" in classify response ; documented in customer SDK |
| R7 | SigLIP false-positive on protected attributes | MED | HIGH | Fairness eval against representative dataset ; M32 extends bias eval framework |
| R8 | Sprint window overruns | MED | MED | Scope-cut : defer chunking (Component D) to M30.1 if needed |

---

## 27. Sprint plan (5-7 weeks)

### Week 1 — Foundations
- Day 1-2 : Triton Helm sub-chart + GPU node pool config (Terraform).
- Day 3-4 : SigLIP + Whisper + cross-encoder model upload + cosign-sign.
- Day 5 : SQL migration 0024.

### Week 2 — efe-multimodal crate
- Day 1-2 : `input_guard` + Triton client.
- Day 3 : `vision` + `audio` pipelines.
- Day 4-5 : `fusion` + `chunking`.

### Week 3 — REST handlers + integration
- Day 1-2 : `POST /v1/classify/multimodal` handler.
- Day 3 : `/v1/embed/vision`, `/v1/embed/audio`.
- Day 4-5 : per-tenant quota enforcement + audit events.

### Week 4 — Multi-region setup
- Day 1-2 : PG replica in eu-west-3.
- Day 3 : NATS mirror stream.
- Day 4-5 : Cloudflare Geosteering + sticky session.

### Week 5 — efe-region crate + DR
- Day 1 : `current_region` + health probe.
- Day 2-3 : Failover orchestrator.
- Day 4-5 : DR drill harness + first chaos run.

### Week 6 — Validation
- Day 1-2 : Pen test bundle.
- Day 3 : Load test 5k RPS multimodal.
- Day 4 : Chaos tests.
- Day 5 : Documentation.

### Week 7 — Rollout
- Day 1 : Staging full deploy.
- Day 2-3 : Design partner pilot.
- Day 4-5 : Production cluster-wide enable + retro.

---

## 28. Done definition

- [ ] `cargo build --workspace --exclude efe-ffi` green.
- [ ] All 5 new REST endpoints documented + OpenAPI spec.
- [ ] Triton pod stable on GPU node for 24h ; > 50% utilization
      under load.
- [ ] First DR drill completes : RPO ≤ 5s, RTO ≤ 15min, audit chain
      verified.
- [ ] 1 design partner classifies images + audio in production for
      48h with 0 incidents.
- [ ] SBOM updated with SigLIP, Whisper, cross-encoder model hashes.
- [ ] All 5 pen test scenarios pass.
- [ ] 5 runbooks + 6 ADRs committed.
- [ ] CHANGELOG `## EFE 2.5.0 — Multi-modal + GPU + Multi-region`.

### Explicit non-criteria

- Multi-modal RAG (out, M31).
- 3+ regions (out, M33).
- Active-active writes (out forever per ADR-M30-004).
- Custom audio encoders (out, M31.5 if demand).
- Confidential compute (out, M33+).

---

## 29. Annexes

### Annex A — Final file list (shipped)

```
crates/efe-multimodal/                              (NEW crate, ~10 modules)
crates/efe-region/                                  (NEW crate, ~5 modules)
crates/efe-server/src/v1/multimodal/                (NEW REST module)
crates/efe-server/migrations/0024_m30_multimodal_region.sql  (NEW + rollback)
charts/efe-stack/charts/efe-triton/                 (NEW sub-chart, ~7 templates)
charts/efe-stack/values-eu-west-1.yaml              (NEW overlay)
charts/efe-stack/values-eu-west-3.yaml              (NEW overlay)
charts/efe-stack/Chart.yaml + values.yaml           (+ efe-triton, region var)
terraform/aws/gpu-node-pool/                        (NEW Terraform module)
terraform/aws/kms-multi-region/                     (NEW Terraform module)
terraform/cloudflare/geosteer.tf                    (NEW)
ops/prometheus/rules/m30-multimodal-slos.yaml       (NEW)
ops/grafana/dashboards/m30-multimodal/              (NEW, 4 dashboards)
bench/m30_multimodal_pentest/                       (NEW, 5 scenarios)
scripts/load/m30/                                   (NEW, 3 k6 scripts)
scripts/chaos/m30/                                  (NEW, 4 scripts)
scripts/m30/dr-drill/                               (NEW DR harness)
scripts/m30-assembly-verify.sh                      (NEW driver)
docs/EFE_M30_MULTIMODAL_ARCHITECTURE.md             (this doc)
docs/docs/adr/ADR-M30-{001..006}.md                  (6 ADRs)
docs/runbooks/RB_M30_{TRITON_DOWN,GPU_OOM,REGION_FAILOVER,REPLICA_LAG_HIGH,DR_DRILL_FAILED}.md
CHANGELOG.md                                        (## EFE 2.5.0 entry)
```

Total ~50 files.

### Annex B — Wire diff M29 → M30

| Wire | Status |
|---|---|
| `/v1/classify/multimodal` | NEW |
| `/v1/embed/vision` | NEW |
| `/v1/embed/audio` | NEW |
| `X-EFE-Region` response header | NEW |
| existing `/v1/classify` (text) | unchanged |
| existing M27-M29 surface | unchanged |

Backward-compatible.

### Annex C — Compliance touchpoints

| Article | Impact |
|---|---|
| GDPR Art. 5(1)(c) data minimisation | Vision OCR redacts PII before storage |
| GDPR Art. 17 erasure | Audit table `multimodal_input_audit` participates in M29 cascade |
| AI Act Art. 9 risk mgmt | Multi-modal risk = higher (bias surface ↑) ; documented per catalog |
| AI Act Art. 26 post-market | Multi-modal classifications surface in quarterly report |
| ISO 27001 A.5.10 backup | DR drill quarterly = evidence |
| SOC 2 CC9.1 risk identification | DR drill = control evidence |

### Annex D — Operational complexity delta

| Pods added (per region) | Description |
|:--:|---|
| 2 | Triton Inference Server |
| 0-2 | DCGM exporter DaemonSet (GPU nodes only) |
| 1 | NATS mirror (passive region) |
| 1 | pg-replica-lag-exporter (per region) |
| → ~5 new pods primary, ~4 passive |

Plus +2 GPU nodes primary (auto-scale 0-5 passive).

---

## End of document

**Audit-passable** per M27/M28/M29 pattern : architecture +
chirurgical audit + code drop + 5-tier verification + ADRs + runbooks
+ CHANGELOG.
