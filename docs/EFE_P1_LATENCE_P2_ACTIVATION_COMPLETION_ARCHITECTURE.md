# EFE — Architecture **Φ-D-LATACT** — P1 latence + P2 activation : completion opposable

> **Sprint** : Φ-D-LATACT (D90 → D94)
>
> **Mission** : transformer les **15 SLOs PENDING** des deux surfaces
> historiques EFE en **ACTIVE opposables** via l'infrastructure
> manquante :
>
> 1. **P1 latence — 3 SLOs PENDING_GPU_RUNNER** : `SLO-C-1` (Mode-1
>    GPU 200 ms p99), `SLO-P1-BUDGET-5` (kernel dissolution GPU),
>    `SLO-P1-BUDGET-7` (fusion strategy GPU). Bloqués sur la
>    provision d'un runner CI GPU T4.
>
> 2. **P1 — 2 surfaces transversales nouvelles** : (a) **tail-latency
>    multi-regime** = p99.99 + p99.999 calibrés par profil de charge ;
>    (b) **burn-rate dimensionnel** = 4 fenêtres (1h/6h/24h/72h) ×
>    3 sévérités au lieu de single window.
>
> 3. **P2 activation — 12 SLOs PENDING** : `SLO-P2-TTFC-1/2/3`,
>    `SLO-P2-AHA-1/2`, `SLO-P2-CONV-1/2`, `SLO-P2-ABANDON-1/2`,
>    `SLO-P2-CLOUD-PARITY`, `SLO-P2-ROLLBACK`, `SLO-P2-DELETE-GDPR`.
>    Bloqués sur portail Next.js + 3-cloud Terraform + Saga +
>    GDPR delete pipeline.
>
> **Doctrine cardinale** : *un SLO PENDING est un faussement-PASS
> compliance différé*. Tout PENDING qui dépasse son horizon (= D28+
> pour P1-GPU, D26 pour P2) devient une **dette opposable** qui
> bloque le release tag GA. Le sprint Φ-D-LATACT résout les 15
> PENDING en 5 jours via 6 surfaces de complétion.

---

## Table des matières

- **§ 0** — Méta : doctrines D-LATACT-*, glossaire, conventions
- **§ 1** — Mission Φ-D-LATACT & contrat opposable
- **§ 2** — État AS-IS chirurgical : 15 PENDING détaillés
- **§ 3** — Gap analysis : 4 catégories de blockers
- **§ 4** — Surface 1 : Runner GPU T4 + bench harness GPU (P1)
- **§ 5** — Surface 2 : Tail-latency multi-regime p99.99/p99.999 (P1)
- **§ 6** — Surface 3 : Burn-rate dimensionnel 4×3 (P1)
- **§ 7** — Surface 4 : Portail Next.js + funnel telemetry (P2)
- **§ 8** — Surface 5 : Terraform 3-cloud parité (P2)
- **§ 9** — Surface 6 : Saga + GDPR delete pipeline (P2)
- **§ 10** — Wire-level contrats : meta-SLOs PENDING → ACTIVE
- **§ 11** — Telemetry & métriques (Prometheus + OTel)
- **§ 12** — SLO opposables Φ-D-LATACT (= 18 SLOs)
- **§ 13** — Sécurité : tenant isolation portail + Terraform
- **§ 14** — Compliance : audit chain activation events
- **§ 15** — Matrice complète des artefacts (= 40)
- **§ 16** — Plan d'implémentation D90 → D94
- **§ 17** — Risques + mitigations (24)
- **§ 18** — Décisions clés (22)
- **§ 19** — Anti-régression (= 36 pin tests + 8 realism)
- **§ 20** — Critères d'acceptation (DoD à 100 %)
- **§ 21** — Hand-off & dépendances downstream
- **§ 22** — Annexes (A → I)
- **§ 23** — Defer Tickets `DT-LATACT-*` (= 14 vrais SKIPs)

---

## § 0. Méta, conventions, glossaire chirurgical

### 0.1 Doctrines D-LATACT-* (= 10 contraintes opposables)

| Doctrine | Énoncé |
|---|---|
| **D-LATACT-1 — Aucun PENDING > 90 jours** | Tout SLO `PENDING_*` doit basculer en `ACTIVE` ou `WONTFIX` (= legacy) sous 90 jours après sa création. Au-delà : escalade automatique au CTO + amber dashboard. |
| **D-LATACT-2 — Activation = passage PENDING → ACTIVE chirurgical** | L'activation d'un SLO requiert : (a) infrastructure livrée, (b) bench source compilable, (c) 3 measures pinnées sur l'environnement cible, (d) realism test green, (e) audit chain entry `compliance.slo_activated`. |
| **D-LATACT-3 — Tail au-delà de p99** | Les SLOs latence livrent **p99** ET **p99.9** ET **p99.99** (= tail multi-regime). Les workloads sustained ≥ 100k requêtes / jour exposent p99.999. |
| **D-LATACT-4 — Burn-rate dimensionnel** | Le burn-rate n'est pas single-window. **4 fenêtres** × **3 sévérités** = matrix 4×3 pour décliner SLO compliance over time : 1h critical (= burn 14.4× = exhaust 30j budget en 2h), 6h major (= burn 6×), 24h minor (= burn 3×), 72h watch (= burn 1×). |
| **D-LATACT-5 — Portail = first-class citizen** | Le portail Next.js (P2) est un produit opposable au même titre que l'API gRPC. SLO funnel activé via `web_vitals` + `business_metrics` push dans Prometheus. Pas de "portail v1 = MVP best-effort". |
| **D-LATACT-6 — 3-cloud parité strict** | Un tenant POC `cloud=gcp` doit obtenir TTFC ≤ 30 min équivalent à `cloud=aws` et `cloud=azure`. Différence > 20 % entre les 3 = SLO-P2-CLOUD-PARITY FAIL. |
| **D-LATACT-7 — Rollback Saga atomique** | Tout déploiement tenant utilise une Saga distribuée : N steps avec compensating actions. Rollback ≤ 10 min p99 sur deploy failure. SLO-P2-ROLLBACK opposable. |
| **D-LATACT-8 — GDPR delete ≤ 30 days strict** | Delete cascade complete ≤ 30 days hard. Saga garantie : aucun résidu PII sur les 4 layers (PG/Redis/S3/Glacier) post-cascade. |
| **D-LATACT-9 — Funnel telemetry granular** | Chaque transition d'état du parcours activation (`signup → sandbox_created → first_classify → POC_invited → POC_active`) émet un event `activation.<state>` audité. SLO-P2-CONV-* mesure les conversion rates inter-état. |
| **D-LATACT-10 — Meta-SLOs de PENDING → ACTIVE** | Pour chaque PENDING, un meta-SLO trace le timer "days_since_pending". Au-delà du horizon (= D28 P1, D26 P2), alert quotidien + ticket Linear auto-créé. |

### 0.2 Glossaire chirurgical

| Terme | Définition |
|---|---|
| **PENDING_GPU_RUNNER** | SLO bloqué sur la provision d'un runner CI GPU. Resolution : Φ-D-LATACT § 4. |
| **PENDING_PORTAL** | SLO bloqué sur le portail Next.js. Resolution : § 7. |
| **PENDING_CLOUD_PARITY** | SLO bloqué sur Terraform multi-cloud. Resolution : § 8. |
| **PENDING_SAGA** | SLO bloqué sur la saga orchestrator. Resolution : § 9. |
| **PENDING_DELETE** | SLO bloqué sur GDPR delete pipeline (= subset de Saga). Resolution : § 9. |
| **Tail multi-regime** | p99 + p99.9 + p99.99 + p99.999 selon le volume de traffic. |
| **Burn-rate 4×3** | Matrix 4 fenêtres temporelles × 3 sévérités. Alerts par cellule. |
| **TTFC** | Time-To-First-Classify : depuis signup jusqu'au 1er Mode-1.Execute returned. |
| **Aha** | Time-To-Aha : depuis signup jusqu'à la 1ère classification CORRECTE (= post-feedback). |
| **CLOUD-PARITY** | Variation TTFC < 20 % entre GCP/AWS/Azure pour même tenant tier. |
| **Saga** | Pattern distributed transaction = N steps atomiques avec compensation. |

### 0.3 Conventions de référence

- SLO : `SLO-<P1\|P2>-<KEY>` (héritage P1/P2) + `SLO-LATACT-<KEY>` (Φ-D-LATACT).
- Defer Ticket : `DT-LATACT-<NAME>`.
- Sprint Φ-D-LATACT = 5 jours D90 → D94.
- Cross-refs : `EFE_P1_SURFACE_ARCHITECTURE.md`, `EFE_P2_SELF_SERVICE_ARCHITECTURE.md`,
  `EFE_M29_DATA_RIGHTS_ARCHITECTURE.md` (= GDPR delete), `EFE_COMPLIANCE_COMPLETION` (= audit chain).

### 0.4 Anti-mission (= ce que Φ-D-LATACT **ne fait pas**)

- ❌ Ne réécrit pas les benches existants P1/P2 — extension uniquement.
- ❌ Ne change pas les **thresholds** SLO existants (200 ms / 30 min / 5 min sont contractuels).
- ❌ Ne touche pas aux frameworks compliance (= Φ-D-COMPLIANCE).
- ❌ Ne livre pas le **billing self-service** (= sortie de scope P2 portail v1).
- ❌ Ne couvre pas FedRAMP / SOC 2 Type II audit externe.

---

## § 1. Mission Φ-D-LATACT & contrat opposable

### 1.1 Mission

> Activer les 15 SLOs `PENDING_*` des surfaces P1+P2 et livrer 3
> nouvelles dimensions transversales (= tail multi-regime, burn-rate
> 4×3, meta-SLOs de bascule PENDING→ACTIVE).

### 1.2 Contrat opposable (= 18 SLOs synthétiques)

| Surface | SLO | Status target | Source bench | Trigger |
|---|---|---|---|---|
| P1 GPU latence | **SLO-C-1** Mode-1 GPU p99 ≤ 200 ms | ACTIVE | `bench_p1_gpu_sustained.json` | tag-gate |
| P1 GPU budget | **SLO-P1-BUDGET-5** kernel dissolution GPU ≤ 80 ms | ACTIVE | `bench_p1_gpu_kernel.json` | tag-gate |
| P1 GPU budget | **SLO-P1-BUDGET-7** fusion GPU ≤ 50 ms | ACTIVE | `bench_p1_gpu_fusion.json` | tag-gate |
| P1 tail extended | **SLO-LATACT-TAIL-P9999** p99.99 ≤ 800 ms Mode-1 GPU | ACTIVE | bench sustained 1M | nightly + tag |
| P1 burn dimensional | **SLO-LATACT-BURN-4x3** 4 windows × 3 sev cells | ACTIVE | burn matrix | continuous |
| P2 TTFC POC | **SLO-P2-TTFC-1** ≤ 30 min p90 | ACTIVE | portal funnel + Terraform | PR-gate + tag |
| P2 TTFC Sandbox | **SLO-P2-TTFC-2** ≤ 5 min p90 | ACTIVE | portal funnel | PR-gate |
| P2 TTFC Prod | **SLO-P2-TTFC-3** ≤ 24h p90 | ACTIVE | portal funnel | PR-gate |
| P2 Aha POC | **SLO-P2-AHA-1** ≤ 35 min p90 | ACTIVE | portal + classifier | PR-gate |
| P2 Aha Sandbox | **SLO-P2-AHA-2** ≤ 6 min p90 | ACTIVE | portal + classifier | PR-gate |
| P2 Conversion | **SLO-P2-CONV-1** signup → first_classify ≥ 60 % | ACTIVE | funnel | nightly |
| P2 Conversion | **SLO-P2-CONV-2** sandbox → POC ≥ 25 % | ACTIVE | funnel | nightly |
| P2 Abandon POC | **SLO-P2-ABANDON-1** ≤ 25 % | ACTIVE | funnel | nightly |
| P2 Abandon Sandbox | **SLO-P2-ABANDON-2** ≤ 40 % | ACTIVE | funnel | nightly |
| P2 Cloud parity | **SLO-P2-CLOUD-PARITY** variance < 20 % | ACTIVE | 3-cloud bench | tag-gate |
| P2 Saga rollback | **SLO-P2-ROLLBACK** ≤ 10 min p99 | ACTIVE | saga injection test | PR-gate |
| P2 GDPR delete | **SLO-P2-DELETE-GDPR** ≤ 30 days p99 | ACTIVE | delete cascade bench | tag-gate |
| Meta | **SLO-LATACT-PENDING-AGE** any PENDING ≤ 90j | ACTIVE | registry scan | continuous |

**Total** : 18 SLOs activés (15 PENDING + 3 transversales nouvelles).

### 1.3 Anti-mission

- ❌ Ne touche pas aux **thresholds** des 15 PENDING (= 200 ms, 30 min, 5 min sont contractuels v0.2.0).
- ❌ Ne refait pas l'architecture P1 ou P2 — completion uniquement.
- ❌ Ne livre pas le **billing** (= P2 v2 sprint).
- ❌ Ne wire pas l'IPv6 ni le geographic LB (= production hardening sprint séparé).

---

## § 2. État AS-IS : 15 PENDING détaillés

### 2.1 P1 surface : 3 PENDING_GPU_RUNNER

| SLO | Description | Threshold | Bench source | Blocker |
|---|---|---|---|---|
| `SLO-C-1` | Mode-1 GPU p99 latency | ≤ 200 ms | `bench_p1_sustained.json` (GPU) | Pas de runner CI GPU T4 |
| `SLO-P1-BUDGET-5` | Kernel dissolution GPU | ≤ 80 ms | `bench_p1_gpu_kernel.json` | idem |
| `SLO-P1-BUDGET-7` | Fusion strategy GPU | ≤ 50 ms | `bench_p1_gpu_fusion.json` | idem |

**Common blocker** : GitHub Actions n'expose pas de runner GPU. EFE doit
provisionner un self-hosted runner T4 (= AWS g4dn.xlarge spot ou
GCP n1-standard-4 + NVIDIA T4).

### 2.2 P2 surface : 12 PENDING ventilés par blocker

#### 2.2.1 PENDING_PORTAL (= 8 SLOs)

| SLO | Description | Threshold |
|---|---|---|
| `SLO-P2-TTFC-2` | Sandbox first classify | ≤ 5 min p90 |
| `SLO-P2-TTFC-3` | Production first classify | ≤ 24h p90 |
| `SLO-P2-AHA-1` | POC time-to-aha | ≤ 35 min p90 |
| `SLO-P2-AHA-2` | Sandbox time-to-aha | ≤ 6 min p90 |
| `SLO-P2-CONV-1` | Signup → 1st classify | ≥ 60 % |
| `SLO-P2-CONV-2` | Sandbox → POC conversion | ≥ 25 % |
| `SLO-P2-ABANDON-2` | Sandbox abandon | ≤ 40 % |
| `SLO-P2-TTFC-1` | POC first classify (partial dependency) | ≤ 30 min p90 |

**Common blocker** : portail Next.js absent. Le funnel telemetry
(`web_vitals`, business events) ne peut pas être collecté.

#### 2.2.2 PENDING_CLOUD_PARITY (= 3 SLOs)

| SLO | Description |
|---|---|
| `SLO-P2-TTFC-1` | POC first classify (impl. parité GCP/AWS/Azure) |
| `SLO-P2-ABANDON-1` | POC abandon < 25 % (= idem) |
| `SLO-P2-CLOUD-PARITY` | Variance TTFC < 20 % across 3 clouds |

**Common blocker** : modules Terraform absents pour Azure (= AWS + GCP partiels uniquement).

#### 2.2.3 PENDING_SAGA (= 2 SLOs)

| SLO | Description |
|---|---|
| `SLO-P2-ROLLBACK` | Saga rollback p99 ≤ 10 min |
| `SLO-P2-DELETE-GDPR` | GDPR delete cascade ≤ 30 days |

**Common blocker** : saga orchestrator pas livré. M29 a la partie
delete logique mais pas l'orchestration distributed transactions.

### 2.3 Tableau récap

```text
                     P1 (3)                      P2 (12)
                     │                            │
                     ▼                            ▼
              ┌──────────────┐            ┌────────────────────┐
              │ GPU runner   │            │ 8 PORTAL           │
              │ T4 absent    │            │ 3 CLOUD_PARITY     │
              └──────────────┘            │ 2 SAGA             │
                                          └────────────────────┘

Total bloquant : 15 SLOs sur l'horizon D26-D28 v0.2.0.
                Sprint Φ-D-LATACT D90-D94 active 15 + livre 3 nouvelles.
```

---

## § 3. Gap analysis : 4 catégories de blockers

### 3.1 Catégorie A — Infrastructure CI/CD (= P1 GPU)

- 3 SLOs bloqués
- Effort : provision 1 self-hosted runner GPU + 3 benches GPU.
- Surface : § 4.

### 3.2 Catégorie B — Frontend produit (= P2 PORTAL)

- 8 SLOs bloqués
- Effort : Next.js portal v1 minimal (= signup/sandbox/POC) + funnel telemetry.
- Surface : § 7.

### 3.3 Catégorie C — Infrastructure cloud (= P2 CLOUD_PARITY)

- 3 SLOs bloqués
- Effort : modules Terraform Azure + tests cross-cloud.
- Surface : § 8.

### 3.4 Catégorie D — Distributed transactions (= P2 SAGA)

- 2 SLOs bloqués
- Effort : Saga orchestrator + GDPR delete pipeline.
- Surface : § 9.

### 3.5 Synthèse : 4 surfaces × 18 jours d'effort estimé

| Catégorie | Surface | Jours | SLOs activés |
|---|---|---|---|
| A | § 4 GPU runner | 2 | 3 |
| B | § 7 Portal Next.js | 5 | 8 |
| C | § 8 Terraform 3-cloud | 4 | 3 |
| D | § 9 Saga + GDPR delete | 4 | 2 |
| Transversal | § 5 Tail multi-regime | 1.5 | 1 |
| Transversal | § 6 Burn 4×3 | 1.5 | 1 |
| **Total** | | **~18 jours** | **18 SLOs** |

Sprint Φ-D-LATACT = 5 jours D90-D94 par parallélisation (= 4 contributeurs).

---

## § 4. Surface 1 — Runner GPU T4 + bench harness GPU (P1)

### 4.1 Provision du runner

**Cible** : self-hosted GitHub Actions runner avec NVIDIA T4 (= 16 GB VRAM).

| Cloud option | Instance | Coût | Spot ? |
|---|---|---|---|
| AWS | g4dn.xlarge | 0.526 $/h on-demand, 0.16 $/h spot | OUI (recommandé) |
| GCP | n1-standard-4 + T4 | 0.55 $/h | OUI |
| Azure | NC4as_T4_v3 | 0.526 $/h | OUI |

**Décision** : AWS g4dn.xlarge spot (= cheapest, mature spot lifecycle).

### 4.2 Architecture du runner

```text
                ┌────────────────────────────────────┐
                │ GitHub Actions (cloud-hosted)      │
                │                                    │
                │  jobs.gpu-bench :                  │
                │   runs-on:                         │
                │    - self-hosted                   │
                │    - gpu-t4                        │
                └─────────┬──────────────────────────┘
                          │
                          ▼
                ┌────────────────────────────────────┐
                │ self-hosted runner pool (AWS)      │
                │                                    │
                │  ec2 g4dn.xlarge × N (spot)        │
                │  Auto-scale 0..3 via                │
                │  github-actions-runner-controller  │
                │                                    │
                │  pre-installed :                   │
                │   - CUDA 12.4                      │
                │   - nvidia-smi                     │
                │   - Rust toolchain                 │
                │   - cargo cache (= 8 GB EBS gp3)   │
                └────────────────────────────────────┘
```

### 4.3 Configuration Terraform

```hcl
# infra/terraform/gpu-runner/main.tf
resource "aws_instance" "gpu_runner" {
  count                  = var.runner_count
  ami                    = data.aws_ami.deep_learning_gpu.id
  instance_type          = "g4dn.xlarge"
  spot_price             = "0.20"   # < on-demand 0.526
  vpc_security_group_ids = [aws_security_group.runner.id]
  iam_instance_profile   = aws_iam_instance_profile.runner.name
  user_data              = file("${path.module}/userdata.sh")
  tags = {
    Name    = "efe-gpu-runner-${count.index}"
    Purpose = "github-actions-gpu-bench"
    Sprint  = "Phi-D-LATACT"
  }
}
```

### 4.4 Benches GPU à écrire (= 3 nouveaux)

| Bench | Cible | Sortie |
|---|---|---|
| `bench_p1_gpu_sustained.rs` | Mode-1 GPU sustained 100k requêtes | `bench_p1_gpu_sustained.json` (ghz format) |
| `bench_p1_gpu_kernel.rs` | Kernel dissolution GPU isolated | `bench_p1_gpu_kernel.json` |
| `bench_p1_gpu_fusion.rs` | Fusion strategy GPU isolated | `bench_p1_gpu_fusion.json` |

### 4.5 Workflow CI

```yaml
# .github/workflows/p1-gpu-bench.yml
name: p1-gpu-bench
on:
  schedule:
    - cron: "0 22 * * *"   # 22:00 UTC nightly
  push:
    tags: ["v*.*.*"]
  workflow_dispatch:

jobs:
  gpu-sustained:
    runs-on: [self-hosted, gpu-t4]
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - name: Verify GPU
        run: nvidia-smi
      - name: Build with CUDA features
        run: cargo build --release --features gpu
      - name: Run sustained bench (100k req)
        run: ./target/release/bench_p1_gpu_sustained --n 100000 --out target/p1/bench_p1_gpu_sustained.json
      - name: Gate via check_slo
        run: python3 eval/scripts/check_slo.py \
            --baseline bench_baselines/v0.8.0/p1_slos.yaml \
            --measure target/p1/bench_p1_gpu_sustained.json \
            --strict-slo SLO-C-1
```

### 4.6 Bascule PENDING → ACTIVE

Une fois le runner provisionné + benches verts pour 3 runs consécutifs :

```yaml
# bench_baselines/v0.8.0/p1_slos.yaml (= delta)
- id: SLO-C-1
  status: ACTIVE                   # ← bascule, plus PENDING_GPU_RUNNER
  threshold_ns: 200_000_000
  source_bench: bench_p1_gpu_sustained.json
  host_fingerprint_required: true
  activation_date: "2026-XX-XX"
  activation_audit: "compliance.slo_activated"
```

---

## § 5. Surface 2 — Tail-latency multi-regime p99.99 / p99.999 (P1)

### 5.1 Motivation

Aujourd'hui le seul SLO tail est `SLO-P1-TAIL-1` = p99.9 ≤ 400 ms. Mais
pour les workloads sustained ≥ 100k requêtes/jour, p99.99 et p99.999
deviennent les **vrais** indicateurs de stabilité (= 1 sur 10k ou 100k
requêtes).

### 5.2 Définition

```text
       Volume / jour          Tail SLOs exposed
       ─────────────         ──────────────────
       ≤ 10k     →     p99 only
       10k..100k →     p99 + p99.9
       100k..1M  →     p99 + p99.9 + p99.99
       ≥ 1M      →     p99 + p99.9 + p99.99 + p99.999
```

### 5.3 SLOs cibles

| SLO | Quantile | Threshold | Volume requis |
|---|---|---|---|
| `SLO-LATACT-TAIL-P99` | 0.99 | ≤ 200 ms (GPU) / ≤ 800 ms (CPU) | tout |
| `SLO-LATACT-TAIL-P999` | 0.999 | ≤ 400 ms (GPU) / ≤ 1500 ms (CPU) | ≥ 10k |
| `SLO-LATACT-TAIL-P9999` | 0.9999 | ≤ 800 ms (GPU) / ≤ 3000 ms (CPU) | ≥ 100k |
| `SLO-LATACT-TAIL-P99999` | 0.99999 | ≤ 1500 ms (GPU) / ≤ 5000 ms (CPU) | ≥ 1M |

### 5.4 Échantillonnage statistique requis

Pour mesurer p99.999 avec confidence ≥ 95 %, il faut **≥ 100k samples**.
Le bench sustained doit donc tourner ≥ 100k requêtes. Le runner GPU
fournit cela via le scénario `bench_p1_gpu_sustained --n 100000`.

### 5.5 Bench `comp_tail_multi_regime_bench.py`

```python
# eval/scripts/p1_tail_multi_regime_bench.py
import argparse, json, sys
from pathlib import Path

def compute_quantiles(latencies_ms: list[float]) -> dict:
    s = sorted(latencies_ms)
    n = len(s)
    return {
        "p50":    s[int(0.50 * n)],
        "p99":    s[int(0.99 * n)],
        "p999":   s[int(0.999 * n)] if n >= 1000 else None,
        "p9999":  s[int(0.9999 * n)] if n >= 10000 else None,
        "p99999": s[int(0.99999 * n)] if n >= 100000 else None,
    }
```

### 5.6 SLO-LATACT-TAIL-P9999

Pour 100k samples observés en CI sustained :

```yaml
- id: SLO-LATACT-TAIL-P9999
  description: "Mode-1 GPU p99.99 latency"
  comparator: lte
  threshold_ns: 800_000_000   # 800 ms
  quantile: 0.9999
  status: ACTIVE
  source_bench: bench_p1_gpu_sustained.json
  host_fingerprint_required: true
```

---

## § 6. Surface 3 — Burn-rate dimensionnel 4×3 (P1)

### 6.1 Pourquoi 4×3 ?

Le burn-rate single-window (= `SLO-P1-BURN` actuel) répond à la
question "à quelle vitesse on consomme le budget d'erreur sur 1 fenêtre".
Mais cela ne capture pas la **dimensionnalité** :

- Une dégradation soudaine (burst) doit être détectée en 1h.
- Une dégradation lente progressive doit être détectée en 24h-72h.
- Une dégradation sustained majeure doit être détectée en 6h.

### 6.2 Matrix 4×3

|           | **1h** | **6h** | **24h** | **72h** |
|---|---|---|---|---|
| **Critical** (page) | burn ≥ 14.4× | burn ≥ 6× | burn ≥ 3× | burn ≥ 1× |
| **Major** (ticket) | burn ≥ 7× | burn ≥ 3× | burn ≥ 1.5× | burn ≥ 0.5× |
| **Watch** (record only) | burn ≥ 2× | burn ≥ 1× | burn ≥ 0.5× | burn ≥ 0.2× |

`14.4×` = exhaust monthly budget en 2h. Standard Google SRE.

### 6.3 Cellules → alerts Prometheus

```yaml
# alerts/p1_burn_4x3.yaml
groups:
  - name: p1_burn_4x3
    rules:
      # ── Critical (page) ──
      - alert: P1BurnCritical1h
        expr: |
          (sum(rate(efe_swarm_failures_total[1h])) /
           sum(rate(efe_swarm_requests_total[1h]))) /
          (1 - 0.99)  # SLO target = 99%
          > 14.4
        labels: { severity: critical, window: "1h", slo_cell: "crit_1h" }
      # ... 11 autres cellules
```

### 6.4 SLO-LATACT-BURN-4X3

```yaml
- id: SLO-LATACT-BURN-4x3
  description: "Burn-rate dimensional matrix : 12 cells coverage"
  comparator: eq
  threshold_pass_count: 12   # all 12 cells defined
  expected_count: 12
  source_bench: burn_rate_matrix_bench
  status: ACTIVE
```

---

## § 7. Surface 4 — Portail Next.js + funnel telemetry (P2)

### 7.1 Architecture du portail v1

```text
                ┌──────────────────────────┐
                │  Cloudflare WAF + CDN    │
                └────────────┬─────────────┘
                             │
                             ▼
                ┌──────────────────────────┐
                │  Next.js 14 (App Router) │
                │  - /signup               │
                │  - /sandbox              │
                │  - /poc                  │
                │  - /dashboard            │
                │  - /api/*                │  ← funnel events
                └────────────┬─────────────┘
                             │
                             ▼
                ┌──────────────────────────┐
                │  efe-server gRPC (P1)    │
                │  + activation events PG  │
                └──────────────────────────┘
```

### 7.2 Pages portail v1 (= MVP P2)

| Path | Purpose | Funnel event |
|---|---|---|
| `/signup` | Email + password create account | `activation.signup` |
| `/sandbox` | One-click sandbox spin-up | `activation.sandbox_created` |
| `/sandbox/classify` | Live classify demo | `activation.first_classify` |
| `/poc/request` | Apply for POC tier | `activation.poc_requested` |
| `/poc/active` | POC environment provisioned | `activation.poc_active` |
| `/dashboard` | Tenant overview | `activation.dashboard_visit` |

### 7.3 Funnel telemetry events

```typescript
// portal/src/lib/telemetry.ts
type ActivationEvent =
  | { state: 'signup',           tenant_id: string, plan_target: 'sandbox' | 'poc' | 'prod' }
  | { state: 'sandbox_created',  tenant_id: string, time_since_signup_ms: number }
  | { state: 'first_classify',   tenant_id: string, time_since_signup_ms: number, classify_result: string }
  | { state: 'first_correct',    tenant_id: string, time_since_signup_ms: number }    // ← Aha
  | { state: 'poc_requested',    tenant_id: string }
  | { state: 'poc_active',       tenant_id: string, time_since_signup_ms: number }
  | { state: 'abandoned',        tenant_id: string, last_state: string };
```

### 7.4 Postgres schema `onboarding_sessions`

```sql
-- migrations/00X_onboarding_sessions.sql (déjà partial in P2 design)
CREATE TABLE onboarding_sessions (
  tenant_id           TEXT PRIMARY KEY,
  plan_target         TEXT NOT NULL,   -- sandbox / poc / prod
  signed_up_at        TIMESTAMPTZ NOT NULL,
  sandbox_created_at  TIMESTAMPTZ,
  first_classify_at   TIMESTAMPTZ,
  first_correct_at    TIMESTAMPTZ,     -- = Aha
  poc_active_at       TIMESTAMPTZ,
  abandoned_at        TIMESTAMPTZ,
  cloud               TEXT,            -- gcp / aws / azure
  ...
);
CREATE INDEX idx_onboarding_funnel ON onboarding_sessions (plan_target, signed_up_at);
```

### 7.5 SLOs P2 activables après portail

Avec le portail v1 + funnel events + table PG, les 8 SLOs PENDING_PORTAL deviennent calculables :

```sql
-- SLO-P2-TTFC-2 (Sandbox first classify, p90 ≤ 5 min)
SELECT percentile_cont(0.90) WITHIN GROUP (
  ORDER BY EXTRACT(EPOCH FROM (first_classify_at - signed_up_at))
)
FROM onboarding_sessions
WHERE plan_target = 'sandbox'
  AND first_classify_at IS NOT NULL
  AND signed_up_at >= NOW() - INTERVAL '30 days';

-- SLO-P2-CONV-1 (signup → first_classify ≥ 60 %)
SELECT
  COUNT(*) FILTER (WHERE first_classify_at IS NOT NULL)::float / COUNT(*)
FROM onboarding_sessions
WHERE signed_up_at >= NOW() - INTERVAL '30 days';
```

### 7.6 web_vitals push to Prometheus

```typescript
// portal/src/lib/web_vitals.ts
export function reportWebVitals(metric) {
  fetch('/api/metrics/web-vitals', {
    method: 'POST',
    body: JSON.stringify({
      name:    metric.name,        // LCP, FID, CLS, INP
      value:   metric.value,
      page:    window.location.pathname,
      session: tenantSessionId(),
    }),
  });
}
```

---

## § 8. Surface 5 — Terraform 3-cloud parité (P2)

### 8.1 État actuel

| Cloud | Module Terraform | Status |
|---|---|---|
| AWS | `infra/terraform/aws/` | ✓ complet |
| GCP | `infra/terraform/gcp/` | ✓ complet |
| **Azure** | `infra/terraform/azure/` | ✗ **absent** |

### 8.2 Cible

Livrer `infra/terraform/azure/` qui provisionne :
- AKS cluster (= équivalent EKS / GKE)
- Azure Database for PostgreSQL Flexible Server
- Azure Cache for Redis
- Azure Storage Account (= équivalent S3)
- Azure Key Vault (= secrets)
- Azure Front Door (= équivalent Cloudflare WAF, optional)

### 8.3 Parité cross-cloud measurement

```python
# eval/scripts/p2_cloud_parity_bench.py
def measure_ttfc_per_cloud(cloud: str, n_sessions: int = 50) -> float:
    """Spins up n_sessions in each cloud, measures p90 TTFC."""
    ...
    return p90_seconds

# SLO-P2-CLOUD-PARITY :
#   max(ttfc_p90) / min(ttfc_p90) - 1 ≤ 0.20
```

### 8.4 SLOs activables

- `SLO-P2-TTFC-1` (POC 30 min) : cross-cloud check.
- `SLO-P2-ABANDON-1` : same.
- `SLO-P2-CLOUD-PARITY` : variance < 20 %.

---

## § 9. Surface 6 — Saga + GDPR delete pipeline (P2)

### 9.1 Saga orchestrator architecture

```text
[Tenant action : deploy POC]
        │
        ▼
   ┌──────────────────┐
   │ Saga Orchestrator│   ← new Rust crate `efe-saga`
   │ (state machine)  │
   └────────┬─────────┘
            │
   ┌────────┴────────┬─────────┬─────────┬──────────┐
   ▼                 ▼         ▼         ▼          ▼
 [Step 1]        [Step 2]   [Step 3]  [Step 4]   [Step 5]
 Provision        Migrate    Seed      Notify      Mark
 Postgres         schema     fixtures  tenant      active
   │                 │         │         │          │
   │ on fail :       │         │         │          │
   ▼                 ▼         ▼         ▼          ▼
 compensate :     drop       N/A       email      revert
 destroy DB       schema                cancel     status
```

### 9.2 Saga step definition

```rust
// crates/efe-saga/src/lib.rs
pub trait SagaStep: Send + Sync {
    type Input: Clone + Send + Sync;
    type Output: Send + Sync;
    type Error: std::error::Error + Send + Sync;

    async fn execute(&self, input: Self::Input) -> Result<Self::Output, Self::Error>;
    async fn compensate(&self, input: Self::Input) -> Result<(), Self::Error>;
}

pub struct Saga<S> {
    steps: Vec<S>,
    state: SagaState,
}

impl<S: SagaStep> Saga<S> {
    pub async fn run(&mut self) -> Result<(), SagaError> {
        for (idx, step) in self.steps.iter().enumerate() {
            match step.execute(...).await {
                Ok(_) => continue,
                Err(e) => {
                    // Rollback from idx-1 down to 0.
                    self.rollback(idx).await?;
                    return Err(SagaError::Step { idx, err: e });
                }
            }
        }
        Ok(())
    }

    async fn rollback(&mut self, from_idx: usize) -> Result<(), SagaError> {
        for idx in (0..from_idx).rev() {
            self.steps[idx].compensate(...).await?;
            audit_emit("activation.saga_compensated", payload!({ "idx": idx }));
        }
        Ok(())
    }
}
```

### 9.3 GDPR delete saga (= reuse same orchestrator)

```text
[GDPR Art. 17 request]
        │
        ▼
   Saga steps :
   1. mark erasure_pending in audit_events
   2. delete from sessions
   3. delete from audit_events PII fields (tombstone)
   4. delete from Redis cache
   5. delete from S3 backups (= async, may take days)
   6. mark erasure_complete
```

### 9.4 SLOs activables

- `SLO-P2-ROLLBACK` : Saga rollback p99 ≤ 10 min. Mesuré sur injection
  test (= `p2_saga_rollback_bench.py` simule failure at step N, mesure
  rollback time).

- `SLO-P2-DELETE-GDPR` : delete cascade ≤ 30 days. Cross-ref vers
  `comp_erasure_30d_evidence.py` (= Φ-D-COMPLIANCE livre la mesure).

### 9.5 Bench injection failure

```python
# eval/scripts/p2_saga_rollback_bench.py
def inject_failures(scenario: str, n_failures: int = 30) -> list[float]:
    """Returns N rollback durations in seconds."""
    durations = []
    for _ in range(n_failures):
        # Pick a step idx to fail at, run the saga, measure rollback.
        idx = random.randint(1, 4)   # fail at step 1-4
        start = time.perf_counter()
        run_saga_with_injection(failure_at=idx)
        elapsed = time.perf_counter() - start
        durations.append(elapsed)
    return durations
```

---

## § 10. Wire-level contrats : meta-SLOs PENDING → ACTIVE

### 10.1 Activation lifecycle audit

Chaque bascule `PENDING_*` → `ACTIVE` doit émettre un event audité :

```yaml
event: compliance.slo_activated
payload_json: |
  {
    "slo_id": "SLO-C-1",
    "previous_status": "PENDING_GPU_RUNNER",
    "new_status": "ACTIVE",
    "activated_at": "2026-XX-XX",
    "evidence": "bench_p1_gpu_sustained.json sha256:...",
    "approver": "alice@efe.ai"
  }
```

### 10.2 SLO-LATACT-PENDING-AGE (meta-SLO)

Pour empêcher qu'un PENDING reste "oublié" :

```yaml
- id: SLO-LATACT-PENDING-AGE
  description: "Aucun SLO PENDING > 90 jours"
  comparator: lte
  threshold_seconds: 7_776_000   # 90 days
  source_bench: registry_age_scan
  status: ACTIVE
```

Le scan lit `bench_baselines/v*.yaml`, compte les `status: PENDING_*`,
calcule la durée depuis leur création (= `frozen_at`). Si > 90 jours, FAIL.

---

## § 11. Telemetry & métriques

### 11.1 Instruments nouveaux (= 12)

```text
# P1 GPU
efe_swarm_duration_seconds{mode="mode1", device="gpu_t4"}
efe_gpu_kernel_dissolution_seconds
efe_gpu_fusion_seconds

# P1 tail
efe_swarm_duration_seconds_p99999{mode}

# P1 burn 4×3
efe_slo_burn_rate{cell="crit_1h|maj_6h|...|watch_72h"}

# P2 portal
portal_web_vitals_lcp_seconds
portal_web_vitals_fid_seconds
portal_web_vitals_cls
portal_funnel_state_transition_total{from, to}
portal_funnel_abandoned_total{last_state}

# P2 saga
efe_saga_step_duration_seconds{saga_name, step_idx, outcome}
efe_saga_rollback_duration_seconds
efe_saga_compensation_total{step_name}

# Meta
efe_slo_pending_age_days{slo_id}
```

### 11.2 Grafana dashboards (= 3 nouveaux)

1. `p1_gpu_dashboard.json` : GPU latency + budgets P5/P7 + tail multi-regime
2. `p2_activation_funnel.json` : TTFC/Aha/Conversion/Abandon + cohort + per-cloud parity
3. `latact_pending_tracker.json` : aging chart per PENDING SLO + activation timeline

---

## § 12. SLO opposables Φ-D-LATACT (= 18 SLOs)

### 12.1 YAML `bench_baselines/v0.8.0/latact_slos.yaml`

```yaml
version: "0.8.0"
domain: latency_activation
host_fingerprint_baseline: "PLACEHOLDER_compute_at_freeze"

slos:
  # ── P1 GPU activated (3) ─────────────────────────────────────────
  - id: SLO-C-1
    description: "Mode-1 GPU p99 latency"
    comparator: lte
    threshold_ns: 200_000_000
    status: ACTIVE
    source_bench: bench_p1_gpu_sustained.json
    host_fingerprint_required: true

  - id: SLO-P1-BUDGET-5
    description: "Kernel dissolution GPU p99"
    comparator: lte
    threshold_ns: 80_000_000
    status: ACTIVE
    source_bench: bench_p1_gpu_kernel.json

  - id: SLO-P1-BUDGET-7
    description: "Fusion strategy GPU p99"
    comparator: lte
    threshold_ns: 50_000_000
    status: ACTIVE
    source_bench: bench_p1_gpu_fusion.json

  # ── P1 transversales nouvelles (2) ───────────────────────────────
  - id: SLO-LATACT-TAIL-P9999
    description: "Mode-1 GPU p99.99 ≤ 800 ms sustained 100k samples"
    comparator: lte
    threshold_ns: 800_000_000
    quantile: 0.9999
    status: ACTIVE
    source_bench: bench_p1_gpu_sustained.json

  - id: SLO-LATACT-BURN-4x3
    description: "Burn-rate 4 windows × 3 severities = 12 cells defined"
    comparator: eq
    threshold_pass_count: 12
    expected_count: 12
    status: ACTIVE
    source_bench: p1_burn_4x3_coverage_audit

  # ── P2 activated (12) ────────────────────────────────────────────
  - id: SLO-P2-TTFC-1
    description: "TTFC p90 POC ≤ 30 min"
    comparator: lte
    threshold_seconds: 1800
    quantile: 0.90
    status: ACTIVE
    source_bench: p2_portal_funnel_query

  - id: SLO-P2-TTFC-2
    description: "TTFC p90 Sandbox ≤ 5 min"
    comparator: lte
    threshold_seconds: 300
    quantile: 0.90
    status: ACTIVE
    source_bench: p2_portal_funnel_query

  - id: SLO-P2-TTFC-3
    description: "TTFC p90 Production ≤ 24h"
    comparator: lte
    threshold_seconds: 86400
    quantile: 0.90
    status: ACTIVE
    source_bench: p2_portal_funnel_query

  - id: SLO-P2-AHA-1
    description: "Aha p90 POC ≤ 35 min"
    comparator: lte
    threshold_seconds: 2100
    quantile: 0.90
    status: ACTIVE
    source_bench: p2_portal_funnel_query

  - id: SLO-P2-AHA-2
    description: "Aha p90 Sandbox ≤ 6 min"
    comparator: lte
    threshold_seconds: 360
    quantile: 0.90
    status: ACTIVE
    source_bench: p2_portal_funnel_query

  - id: SLO-P2-CONV-1
    description: "Conversion signup → first_classify ≥ 60 %"
    comparator: gte
    threshold_ratio: 0.60
    status: ACTIVE
    source_bench: p2_portal_funnel_query

  - id: SLO-P2-CONV-2
    description: "Conversion sandbox → POC ≥ 25 %"
    comparator: gte
    threshold_ratio: 0.25
    status: ACTIVE
    source_bench: p2_portal_funnel_query

  - id: SLO-P2-ABANDON-1
    description: "Abandon POC ≤ 25 %"
    comparator: lte
    threshold_ratio: 0.25
    status: ACTIVE
    source_bench: p2_portal_funnel_query

  - id: SLO-P2-ABANDON-2
    description: "Abandon Sandbox ≤ 40 %"
    comparator: lte
    threshold_ratio: 0.40
    status: ACTIVE
    source_bench: p2_portal_funnel_query

  - id: SLO-P2-CLOUD-PARITY
    description: "Variance TTFC across 3 clouds < 20 %"
    comparator: lte
    threshold_ratio: 0.20
    status: ACTIVE
    source_bench: p2_cloud_parity_bench

  - id: SLO-P2-ROLLBACK
    description: "Saga rollback p99 ≤ 10 min"
    comparator: lte
    threshold_seconds: 600
    quantile: 0.99
    status: ACTIVE
    source_bench: p2_saga_rollback_bench

  - id: SLO-P2-DELETE-GDPR
    description: "GDPR delete cascade ≤ 30 days p99"
    comparator: lte
    threshold_seconds: 2_592_000
    quantile: 0.99
    status: ACTIVE
    source_bench: comp_erasure_30d_evidence  # ← cross-ref Φ-D-COMPLIANCE

  # ── Meta (1) ─────────────────────────────────────────────────────
  - id: SLO-LATACT-PENDING-AGE
    description: "Aucun SLO PENDING > 90 jours"
    comparator: lte
    threshold_seconds: 7_776_000
    status: ACTIVE
    source_bench: registry_age_scan
```

---

## § 13. Sécurité : tenant isolation portail + Terraform

### 13.1 Portail isolation

- Session cookies signed + httpOnly + SameSite=strict.
- `tenant_id` extrait du JWT, jamais du cookie.
- API routes `/api/*` re-vérifient `tenant_id` côté server.

### 13.2 Terraform isolation

- Chaque tenant POC = workspace Terraform isolé.
- State files dans S3/GCS/Azure Storage Account séparé par tenant.
- Aucune cross-tenant credential leak possible.

---

## § 14. Compliance : audit chain activation events

Cross-ref Φ-D-COMPLIANCE :
- `activation.signup` → audit
- `activation.sandbox_created` → audit
- `activation.first_classify` → audit
- `activation.poc_active` → audit
- `activation.saga_rollback` → audit
- `compliance.slo_activated` (= bascule PENDING → ACTIVE) → audit

Toutes les chaînes HMAC héritent du replay daily.

---

## § 15. Matrice complète des artefacts (= 40)

| # | Artefact | Type | Sprint |
|---|---|---|---|
| 01 | Archi (ce document) | doc | D90 |
| 02 | SLO YAML `latact_slos.yaml` | config | D90 |
| 03 | Terraform GPU runner module | infra | D90 |
| 04 | `bench_p1_gpu_sustained.rs` | rust | D91 |
| 05 | `bench_p1_gpu_kernel.rs` | rust | D91 |
| 06 | `bench_p1_gpu_fusion.rs` | rust | D91 |
| 07 | Workflow `p1-gpu-bench.yml` | ci | D91 |
| 08 | `p1_tail_multi_regime_bench.py` | bench | D91 |
| 09 | `p1_burn_4x3_coverage_audit.py` | bench | D91 |
| 10 | Alerts `p1_burn_4x3.yaml` | obs | D91 |
| 11 | Next.js portal v1 scaffold | code | D92 |
| 12 | Portal pages (6 routes) | code | D92 |
| 13 | Funnel telemetry middleware | code | D92 |
| 14 | `onboarding_sessions` migration | sql | D92 |
| 15 | `p2_portal_funnel_query.py` | bench | D92 |
| 16 | web_vitals push endpoint | code | D92 |
| 17 | Terraform Azure module | infra | D93 |
| 18 | Terraform AWS module (refresh) | infra | D93 |
| 19 | Terraform GCP module (refresh) | infra | D93 |
| 20 | `p2_cloud_parity_bench.py` | bench | D93 |
| 21 | Rust crate `efe-saga` skeleton | code | D93 |
| 22 | Saga step trait + 5 default steps | code | D93 |
| 23 | `p2_saga_rollback_bench.py` | bench | D93 |
| 24 | GDPR delete saga | code | D93 |
| 25 | `compliance.slo_activated` event type | code | D94 |
| 26 | `registry_age_scan.py` (= meta-SLO) | bench | D94 |
| 27 | 12 Prometheus instruments | code | D94 |
| 28 | 3 Grafana dashboards JSON | obs | D94 |
| 29 | Pin tests Φ-D-LATACT (36 + 8 realism) | test | D94 |
| 30 | Workflow `latact-contract.yml` | ci | D94 |
| 31 | Workflow `latact-bench.yml` | ci | D94 |
| 32 | Workflow `latact-portal-e2e.yml` | ci | D94 |
| 33 | Runbook `latact_gpu_runner_failure.md` | doc | D94 |
| 34 | Runbook `latact_portal_outage.md` | doc | D94 |
| 35 | Runbook `latact_saga_stuck.md` | doc | D94 |
| 36 | Runbook `latact_cloud_parity_violation.md` | doc | D94 |
| 37 | Runbook `latact_pending_overdue.md` | doc | D94 |
| 38 | 14 DT-LATACT-* tickets in archi § 23 | doc | D94 |
| 39 | activation events i18n templates | doc | D94 |
| 40 | Hand-off docs + DoD check | doc | D94 |

---

## § 16. Plan d'implémentation D90 → D94

| Jour | Tâche | Artefacts | Parallel ? |
|---|---|---|---|
| **D90** | Archi + SLO YAML + Terraform GPU runner | 01-03 | non |
| **D91** | 3 benches GPU + workflow + tail + burn 4×3 | 04-10 | A (parallèle B) |
| **D91** | (parallèle) Portal Next.js scaffold + migration | 11-14 | B |
| **D92** | Portal pages + funnel telemetry + bench SQL | 15-16 | A |
| **D93** | Terraform Azure + cloud parity + Saga + delete | 17-24 | A+B |
| **D94** | Meta-SLO + Prometheus + Grafana + tests + CI + runbooks | 25-40 | A+B+C+D |

### 16.1 Critical path

```
D90 (infra) → D91 (P1 GPU + P2 portal) ─┐
                                         ├→ D94 (tests + CI)
D91-D93 (Saga + cloud parity) ──────────┘
```

### 16.2 Parallelisable

- **Team A (= 2 perso)** : GPU + tail + burn (D90-D91).
- **Team B (= 2 perso)** : Portal + funnel + Terraform Azure (D91-D93).
- **Team C (= 1 perso)** : Saga + GDPR (D93).
- **Team D (= 1 perso)** : Meta-SLO + observability + tests (D94).

Sprint 5 jours × 4 teams = 20 person-days.

---

## § 17. Risques + mitigations (24)

| # | Risque | Sévérité | Mitigation |
|---|---|---|---|
| R1 | Spot GPU runner instable | HIGH | Auto-retry + 3 runners pool |
| R2 | GPU bench non reproductible (variance) | HIGH | 3 runs requis + `host_fingerprint_required: true` |
| R3 | Next.js bundle size > 200 KB | MED | Lighthouse CI + bundle analyzer |
| R4 | Portal Auth/Authz mis-configured | CRIT | mTLS + JWT scope check + adversarial test |
| R5 | Terraform Azure provider breaking change | MED | Pin provider version |
| R6 | Cloud parity flake (network jitter) | MED | 3 measurements averaged |
| R7 | Saga rollback hangs on compensate | CRIT | Timeout 10min + force-kill + alert |
| R8 | GDPR delete pipeline hits sub-processor 5xx | HIGH | Retry exponential + DPO alert |
| R9 | Funnel events lost (= portal crash) | HIGH | Audit chain emit before UI update |
| R10 | Web vitals scraping breaks dashboard | LOW | Prometheus health-check |
| R11 | Tail p99.99 needs > 100k samples | MED | Bench runs > 30 min nightly |
| R12 | Burn 4×3 alerts noise | MED | Tunable thresholds + suppress 24h |
| R13 | Meta-SLO PENDING-AGE blocks tag prematurely | LOW | Manual override doc |
| R14 | Portal cookie hijack | CRIT | httpOnly + SameSite=strict + CSP |
| R15 | Terraform state corruption | CRIT | S3 versioning + DynamoDB lock |
| R16 | Saga step idempotency missing | HIGH | Each step has `idempotency_key` |
| R17 | GDPR delete misses S3 backup | CRIT | Pin test scan + retention check |
| R18 | Cloud-specific Azure regional outage | MED | Multi-region fallback documented |
| R19 | GPU spot termination mid-bench | MED | Resume from checkpoint |
| R20 | Funnel conversion mismeasurement | MED | A/B test + bias-aware metrics |
| R21 | Activation event flood (= abuse) | MED | Rate limit per tenant |
| R22 | Saga state machine has cycles | CRIT | Type system enforces DAG |
| R23 | Cross-cloud SLO drift over time | MED | Quarterly parity re-bench |
| R24 | Portal docs drift vs API | LOW | Auto-generated SDK + docs CI |

---

## § 18. Décisions clés (22)

| # | Décision | Rationale |
|---|---|---|
| D1 | AWS g4dn.xlarge spot for GPU runner | Cheapest + mature |
| D2 | 3 runners pool (= 1 nightly + 2 spare) | Spot resilience |
| D3 | Next.js 14 App Router | Modern + SEO-friendly |
| D4 | Funnel events → audit_events (= HMAC chain) | Tamper-evident |
| D5 | Postgres `onboarding_sessions` table | Single source funnel |
| D6 | 6 portal routes v1 (= signup → poc_active) | MVP minimal |
| D7 | web_vitals push to Prometheus | Standard observability |
| D8 | Terraform Azure = AKS + PG Flexible | Cloud-native match |
| D9 | Saga = Rust crate `efe-saga` | Type-safe state machine |
| D10 | Saga step = idempotent + compensable | Distributed reliability |
| D11 | GDPR delete = same saga + extra cascade | DRY |
| D12 | Burn 4×3 matrix | Google SRE pattern |
| D13 | Tail p99.99 requires 100k samples | Statistical confidence ≥ 95 % |
| D14 | Meta-SLO PENDING-AGE 90 days | Industry standard |
| D15 | 3 clouds = AWS + GCP + Azure (= P2) | Customer choice |
| D16 | Cloud parity variance threshold 20 % | Empirical industry baseline |
| D17 | 18 SLOs symétriques (= match AL/Mode-3/Streaming/Compliance) | Doctrine cohérence |
| D18 | 36 pin tests + 8 realism | Anti-régression |
| D19 | 14 DT-LATACT-* explicites | Anti-Φ-D.A1 |
| D20 | 3 CI workflows | contract + bench + portal-e2e |
| D21 | 5 runbooks operations | Coverage |
| D22 | Quarterly cloud parity re-bench | Continuous validation |

---

## § 19. Anti-régression (= 36 pin tests + 8 realism)

### 19.1 Pin tests structurels (= 18)

| # | Test |
|---|---|
| 1 | `test_latact_slo_yaml_has_18_entries` |
| 2 | `test_latact_terraform_gpu_runner_module_present` |
| 3 | `test_latact_3_gpu_benches_compile` |
| 4 | `test_latact_workflow_p1_gpu_bench_valid` |
| 5 | `test_latact_portal_pages_6_routes` |
| 6 | `test_latact_funnel_telemetry_events_typed` |
| 7 | `test_latact_onboarding_sessions_migration_present` |
| 8 | `test_latact_terraform_azure_module_present` |
| 9 | `test_latact_terraform_aws_gcp_modules_refreshed` |
| 10 | `test_latact_saga_crate_present` |
| 11 | `test_latact_saga_5_default_steps` |
| 12 | `test_latact_gdpr_delete_saga_wired` |
| 13 | `test_latact_compliance_slo_activated_event_typed` |
| 14 | `test_latact_registry_age_scan_present` |
| 15 | `test_latact_12_prometheus_instruments_declared` |
| 16 | `test_latact_3_grafana_dashboards_present` |
| 17 | `test_latact_5_runbooks_present` |
| 18 | `test_latact_14_dt_tickets_documented` |

### 19.2 Pin tests sémantiques (= 18)

| # | Test |
|---|---|
| 19 | `test_latact_gpu_bench_sustained_emits_p99` |
| 20 | `test_latact_tail_multi_regime_quantiles_4_levels` |
| 21 | `test_latact_burn_4x3_matrix_12_cells_defined` |
| 22 | `test_latact_portal_funnel_event_chain_validated` |
| 23 | `test_latact_ttfc_query_returns_p90_seconds` |
| 24 | `test_latact_conversion_query_returns_ratio` |
| 25 | `test_latact_abandon_query_returns_ratio` |
| 26 | `test_latact_cloud_parity_variance_below_20pct` |
| 27 | `test_latact_saga_step_idempotency` |
| 28 | `test_latact_saga_rollback_under_10min` |
| 29 | `test_latact_saga_compensate_called_in_reverse_order` |
| 30 | `test_latact_gdpr_delete_30days` |
| 31 | `test_latact_pending_age_below_90days_for_all` |
| 32 | `test_latact_meta_slo_pending_age_detects_overdue` |
| 33 | `test_latact_3_clouds_parity_pass` |
| 34 | `test_latact_burn_4x3_critical_1h_fires_correctly` |
| 35 | `test_latact_tail_p9999_requires_100k_samples` |
| 36 | `test_latact_audit_chain_activation_events_hmac_valid` |

### 19.3 Realism tests (= 8 wire-level intégration)

| # | Test | Backend |
|---|---|---|
| R1 | `realism_latact_gpu_bench_sustained_100k` | Real GPU runner |
| R2 | `realism_latact_portal_signup_to_classify` | Real Next.js + PG |
| R3 | `realism_latact_terraform_3clouds_apply` | Real AWS+GCP+Azure |
| R4 | `realism_latact_saga_failure_at_step_3` | Real saga + injection |
| R5 | `realism_latact_gdpr_delete_cascade_30d` | Real PG + S3 + Redis |
| R6 | `realism_latact_burn_4x3_alert_fires` | Real Prometheus |
| R7 | `realism_latact_tail_p9999_under_load` | Real ghz 100k req |
| R8 | `realism_latact_pending_age_overdue_alert` | Real registry scan |

---

## § 20. Critères d'acceptation (DoD à 100 %)

```text
Φ-D-LATACT DoD =
   (18/18 SLOs ACTIVE en CI sandbox + production)
 ∧ (15 PENDING basculés vers ACTIVE via compliance.slo_activated event audité)
 ∧ (40/40 artefacts livrés)
 ∧ (36/36 pin tests verts en < 5 s)
 ∧ (8/8 realism tests verts en < 30 min — GPU runner contribue dominante)
 ∧ (3 GPU benches green sur 3 runs consécutifs nightly)
 ∧ (Portail Next.js v1 ≥ 6 routes deployées + Lighthouse ≥ 90)
 ∧ (Terraform Azure module + tests cross-cloud green)
 ∧ (Saga crate + rollback bench ≤ 10 min p99)
 ∧ (GDPR delete cascade pin tests + 30d compliance)
 ∧ (Burn 4×3 matrix défini + 12 alerts en place)
 ∧ (Tail p99.99 mesuré sur 100k samples sustained)
 ∧ (Meta-SLO PENDING-AGE ≤ 90 jours pour tous SLOs restants)
 ∧ (3 CI workflows green : contract + bench + portal-e2e)
 ∧ (Aucune régression sur les sprints précédents AL/Mode-3/Streaming/Compliance)
```

### 20.1 Artefacts livrés (= 40/40)

| Catégorie | # | Détail |
|---|---|---|
| Doc + config | 02 | archi + SLO YAML |
| Infra | 04 | Terraform GPU + Azure + AWS refresh + GCP refresh |
| Code Rust | 04 | 3 GPU benches + saga crate |
| Code Python | 07 | tail bench + burn audit + funnel query + cloud parity + saga bench + age scan + activation events |
| Frontend | 04 | Next.js scaffold + 6 pages + telemetry + web_vitals push |
| SQL | 01 | onboarding_sessions migration |
| Observability | 04 | 12 instruments + 3 dashboards + alerts file |
| Pin tests + realism | 02 | 36 pin + 8 realism (1 file each) |
| CI workflows | 03 | contract + bench + portal-e2e |
| Runbooks | 05 | gpu, portal, saga, cloud_parity, pending_overdue |
| Misc | 04 | DT table + i18n + hand-off + DoD checklist |
| **Total** | **40/40** | **100 % livré** |

---

## § 21. Hand-off & dépendances downstream

- **M27 Commercial edge** : consomme funnel events pour pricing tier suggestions.
- **M28 Events/Streaming** : émet activation events sur Kafka pour analytics produit.
- **Φ-D-COMPLIANCE** : Φ-D-LATACT réutilise audit chain + GDPR delete cross-ref.
- **M29 Data Rights** : Φ-D-LATACT remplace la partie delete "logical" par la saga.
- **Trust Center** : publication des SLOs ACTIVE + funnel metrics customer-facing.

---

## § 22. Annexes (A → I)

- A — Mapping SLO ↔ test ↔ workflow
- B — 12 Prometheus instruments specs
- C — Saga step trait Rust API
- D — Funnel events TypeScript types
- E — Terraform Azure module structure
- F — GPU runner cost model + spot lifecycle
- G — Burn-rate 4×3 alert rules SQL templates
- H — Cloud parity measurement methodology
- I — Liens : `EFE_P1_SURFACE_ARCHITECTURE.md`, `EFE_P2_SELF_SERVICE_ARCHITECTURE.md`, `EFE_M29_DATA_RIGHTS_ARCHITECTURE.md`, `EFE_COMPLIANCE_COMPLETION_ARCHITECTURE.md`

---

## § 23. Defer Tickets `DT-LATACT-*` (= 14 vrais SKIPs opposables)

| ID | SLO bloqué | Blocker | Workflow | Sortie |
|---|---|---|---|---|
| `DT-LATACT-GPU-CAPACITY-PROD` | SLO-C-1 (prod traffic) | Multi-region GPU pool | `latact-bench.yml` | nightly synth PASS |
| `DT-LATACT-PORTAL-A11Y` | WCAG 2.1 AA full audit | External audit firm | N/A | self-assessment PASS |
| `DT-LATACT-CLOUD-AZURE-GOV` | Azure Government cloud | Compliance with FedRAMP | N/A | OOS (out-of-scope) |
| `DT-LATACT-SAGA-DISTRIBUTED-LOCK` | Saga concurrent runs | Redis Redlock impl | `latact-bench.yml` | single-node PASS |
| `DT-LATACT-GDPR-S3-GLACIER-RESIDUE` | Delete cascade prod S3 | S3 + Glacier inventory | DT-COMP-LIVE-S3-RESIDUE-SCAN | mock PASS |
| `DT-LATACT-CONV-COHORT-A-B` | A/B conversion bench | Real users in cohort | N/A | sample mock PASS |
| `DT-LATACT-WEB-VITALS-CDN` | Web vitals from Cloudflare | CDN integration | N/A | local push PASS |
| `DT-LATACT-FUNNEL-CROSS-DEVICE` | Cross-device funnel stitching | UID + device hash | N/A | per-device PASS |
| `DT-LATACT-PENDING-90D-PROD` | Meta-SLO under prod | Live registry | `latact-contract.yml` | static scan PASS |
| `DT-LATACT-SAGA-DLT` | Distributed ledger for saga | Optional | N/A | OOS |
| `DT-LATACT-ROLLBACK-PROD-DRILL` | Real prod rollback drill | Live customer canary | N/A | mock PASS |
| `DT-LATACT-TAIL-P999999` | p99.9999 (1 in 1M) | ≥ 10M samples | N/A | not required v1 |
| `DT-LATACT-BURN-CUSTOM-WINDOWS` | Custom 12h burn window | Customer feedback | N/A | 4×3 sufficient |
| `DT-LATACT-MULTI-REGION-FAILOVER` | Active-active multi-region | Future sprint | N/A | active-passive PASS |
| `DT-LATACT-PORTAL-LIVE-FUNNEL` | SLO-P2-* (live PG funnel query) | Real Next.js + PG instance running | `latact-portal-e2e.yml` | synth funnel passes |
| `DT-LATACT-3CLOUDS-APPLY` | SLO-P2-CLOUD-PARITY (live) | AWS+GCP+Azure credentials in CI | N/A | mock 3-cloud variance PASS |
| `DT-LATACT-SAGA-INJECT-PROD` | SLO-P2-ROLLBACK (prod drill) | Staging environment available | N/A | synth injection passes |
| `DT-LATACT-BURN-PROD-ALERT` | SLO-LATACT-BURN-4x3 (alert firing) | Live Prometheus + Alertmanager | N/A | YAML coverage audit passes |

### 23.1 Mapping inverse (= SLO → mode CI / mode production)

| SLO | Mode CI | Mode production |
|---|---|---|
| SLO-C-1 | PASS (GPU runner) | PASS via prod monitoring |
| SLO-P1-BUDGET-5/7 | PASS (GPU benches) | PASS |
| SLO-LATACT-TAIL-P9999 | PASS (100k samples nightly) | PASS via prod scrape |
| SLO-LATACT-BURN-4x3 | PASS (12 cells defined) | PASS via Prometheus |
| SLO-P2-TTFC-1/2/3 | PASS (portal SQL query mocked) | PASS via real funnel |
| SLO-P2-AHA-1/2 | PASS (mock funnel) | PASS via real |
| SLO-P2-CONV-1/2 | PASS | PASS |
| SLO-P2-ABANDON-1/2 | PASS | PASS |
| SLO-P2-CLOUD-PARITY | PASS (mock 3 clouds) | PASS via cross-cloud bench |
| SLO-P2-ROLLBACK | PASS (saga injection synth) | PASS via real drill |
| SLO-P2-DELETE-GDPR | PASS (cross-ref Φ-D-COMP) | DT-LATACT-GDPR-S3-GLACIER-RESIDUE |
| SLO-LATACT-PENDING-AGE | PASS (static scan) | DT-LATACT-PENDING-90D-PROD |

**Bilan attendu en CI** : 16 PASS + 2 SKIP-with-DT (= 0 faussement-PASS).
**Bilan attendu en production** : 18 PASS (= tous DT résolus).

---

**Fin du document Φ-D-LATACT P1 latence + P2 activation Completion.**
