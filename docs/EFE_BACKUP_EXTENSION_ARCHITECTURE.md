# `efe-backup` — Architecture détaillée de l'extension DR drill quarterly (M7)

> **Statut** : Draft v1.0.0 — Document de référence pour
> l'achèvement du module **M7 = `efe-backup`** entre l'AS-IS
> (post-D17 audit, **124 tests** + **10 audit-fix** + drill weekly
> automatique + 5 verifiers + binaire `efe-backup` 7 subcommands +
> CLI 4-eyes guarded delete + 4 backup CronJobs Helm) et la cible
> **GA 1.0** (= AS-IS + **drill quarterly runbook end-to-end** +
> **Hello-World workflow business** + pin test runbook + **SOC 2
> CC9.1 / ISO 27001 A.17.1.3 / NIS2 Art. 21 evidence**).
>
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-29.
> **Dépendances doc** :
>   * `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` — roadmap globale
>     Φ-A→Φ-G.
>   * `docs/bench/d17_backup_dr_architecture.md` — architecture
>     **D17 baseline** : 1 875 LOC, 8 SLOs DR (N-DR-1 à N-DR-8),
>     12 menaces threat-model, 18 modules Rust crate.
>   * `docs/audits/d17_backup_dr_audit_report.md` — rapport audit
>     **post-D17** : 10 défauts corrigés (2 critiques + 4 hard +
>     4 medium).
>   * `docs/docs/howto/backup-and-restore.md` — runbook public
>     (368 LOC) consommé par les opérateurs.
>   * `docs/EFE_BENCH_EXTENSION_ARCHITECTURE.md` — pattern
>     extension single-sprint pour M6 D29 (référence structurelle).
>
> **Portée** : **1 sprint atomique** non-contigu en phase **Φ-F**.
> Pas de changement de code Rust — c'est de l'**orchestration
> shell + Helm + pin test + documentation**. L'extension complète
> la **maturité opérationnelle** du module M7 sans toucher la
> surface API.
>
>   * **D40** (Φ-F) — DR drill quarterly script
>     `dr/run_quarterly_drill.sh` + Helm CronJob
>     `efe-dr-drill-quarterly` (Hello-World runbook end-to-end
>     testé sur kind) + pin test `test_dr_drill_runbook.py`.
>
> **Cible GA** : **2 cadences de drill (weekly + quarterly) +
> Hello-World workflow business validé + compliance SOC 2 / ISO
> 27001 / NIS2 evidence**.

---

## Table des matières

§0. Méta, glossaire, conventions
§1. Mission de l'extension M7 (Φ-F)
§2. Position dans la topologie EFE post-M6
§3. État AS-IS au 2026-04-29 (post-D17 audit, 124 tests, 10 audit-fix)
§4. Définition opérationnelle de M7 v1.0 GA
§5. Principes directeurs
§6. Invariants structurels
§7. Vue d'ensemble — sprint D40 unique
§8. Surface publique cible (script + CronJob + pin test + doc)
§9. **Sprint D40 — `dr/run_quarterly_drill.sh` + Helm CronJob `efe-dr-drill-quarterly`**
§10. Hello-World runbook (workflow business end-to-end)
§11. SLO mapping (D17 SLOs + 4 nouveaux drill quarterly SLOs)
§12. Tests + pin tests structurels
§13. Critères Go/No-Go
§14. Risques transverses + mitigations (8 risques B-R-1 à B-R-8)
§15. Volumétrie code attendue
§16. Plans d'implémentation détaillés (jour par jour)
§17. Compliance mapping (SOC 2 CC9.1, ISO 27001 A.17.1.3, NIS2 Art. 21, RGPD Art. 32)
§18. Annexe — Hello-World JSON schema + assertions
§19. Annexe — Helm CronJob full YAML
§20. Annexe — exemple sortie pin test
§21. Annexe — RACI + budget
§22. Annexe — Gantt prévisionnel
§23. Annexe — DR drill outcome JSON v2 (extended)
§24. Mises à jour du document

---

## §0. Méta, glossaire, conventions

### §0.1 Conventions

* **AS-IS** = état du code à fin D17 + post-audit, version
  v0.1.18+post-audit (124 tests + 10 audit-fix livrés).
* **TO-BE** = état cible défini par le sprint D40.
* Le sprint D40 est **non-contigu** : 13 sprints intercalaires
  (D18 multi-replica, D19-D26 stack inférence/auth, D27 stream,
  D28 GPU, D29 TonicBenchClient, D30 SDK Go, D31 multi-GPU,
  D32 streaming, D33 templates, D34 Mode-2, D35-D36 LLM routers,
  D37 Mode-3, D38 mode3-k* bench, D39 chaos eng) ne touchent
  pas `efe-backup` directement.
* Les défauts résiduels sont notés **DT-DR.x** (Defect Trace,
  Disaster Recovery).
* **Drill weekly** = `d17-dr-drill-cronjob.yaml` AS-IS,
  `0 3 * * 0` (dimanche 03:00 UTC), restore + 5 verifiers.
* **Drill quarterly** = NEW D40, `0 4 1 1,4,7,10 *` (1er du mois
  trimestriel à 04:00 UTC), restore + 5 verifiers + **Hello-World
  runbook** business end-to-end.
* **Hello-World runbook** = un workflow business minimal mais
  complet : register catalog → register class → register sentence
  → resolve catalog → execute → assert response shape. Mesure que
  le **PATH BUSINESS** marche, pas juste que les binaires
  répondent.
* **Runbook drift** = situation où la doc `backup-and-restore.md`
  diverge du code réel. Le drill quarterly **rejoue la doc** pour
  forcer la cohérence.
* **Pin test** = un test Python qui codifie la structure attendue
  d'un fichier (script, YAML, doc) et fait fail CI si la structure
  drift.
* `[ref:fichier:ligne]` = pointeur dans le code source.

### §0.2 Glossaire

| Terme | Définition |
|-------|------------|
| **`efe-backup`** | Crate Rust `crates/efe-backup/` qui héberge le DR orchestrator. Binaire `efe-backup` avec 7 subcommands. AS-IS 124 tests + 10 audit-fix. |
| **Drill** | Opération de DR : restore + verifier suite + RTO measure. AS-IS un drill weekly automatique via Helm CronJob. |
| **Drill weekly** | CronJob `d17-dr-drill-cronjob.yaml` (= `efe-dr-drill`), schedule dimanche 03:00 UTC, RTO target 1800s (30 min), 5 verifiers (AuditChain, BundleSignatures, OidcIssuers, CatalogSmoke, ExecuteSmoke). |
| **Drill quarterly** | NEW CronJob `efe-dr-drill-quarterly` D40. Schedule trimestriel. **Rejoue le runbook complet** : kind cluster from-scratch, helm install, restore from S3, **Hello-World workflow business**, validation. |
| **Hello-World runbook** | Workflow business minimal mais end-to-end : register catalog → register class → register sentence → resolve catalog → execute → assert. Validate que **le PATH BUSINESS marche**, pas juste les binaires. |
| **`run_quarterly_drill.sh`** | NEW script shell D40 qui orchestre le drill quarterly. Crée kind cluster, install Helm chart, restore from backup, exécute Hello-World workflow, verify, cleanup. |
| **`test_dr_drill_runbook.py`** | NEW pin test Python D40. Pin la structure du script + YAML CronJob + assertions Hello-World. Anti-drift documentation. |
| **RTO** (Recovery Time Objective) | Durée maximale entre disaster et retour service. D17 SLO N-DR-1 = 30 min. Quarterly target = **45 min** (overhead bootstrap from-scratch). |
| **RPO** (Recovery Point Objective) | Perte de données maximale tolérée. D17 SLO N-DR-2 = 5 min (PITR Postgres). Inchangé par D40. |
| **Compliance evidence** | Preuve formelle écrite, datée, signée d'un test DR. Audit SOC 2 / ISO 27001 / NIS2 demandent un archive de logs avec timestamps + résultats des derniers DR drills (≥ 1 par trimestre). |
| **kind** | Kubernetes IN Docker — cluster K8s léger pour CI/dev. Installé via `kind create cluster`. Utilisé par `dr/setup-test-env.sh` AS-IS. |

### §0.3 Cible matérielle

* **CI runner GHA** (Tier-1) — pin test structural seul (pas de
  kind cluster, pin texte uniquement). 1 CPU, 2 GiB RAM. ~30 s.
* **Drill quarterly runner** (Tier-2) — kind cluster + minio +
  postgres + helm + drill end-to-end. **8 vCPU, 32 GiB RAM,
  100 GiB disk éphémère**. ~25-40 min wall-clock.
* **Production drill quarterly** — exécuté sur staging cluster
  (même config que prod, données synthétiques). Schedule
  `0 4 1 1,4,7,10 *`.

### §0.4 Cibles fonctionnelles GA

```
GA v1.0 efe-backup (M7) post-D40 :
├── Crate Rust (inchangé v0.1.18+post-audit)
│   ├── 18 modules src/
│   ├── 124 unit tests + 1 e2e test
│   ├── 7 subcommands CLI
│   ├── 5 verifiers (AuditChain, BundleSignatures, OidcIssuers, CatalogSmoke, ExecuteSmoke)
│   └── 8 SLOs DR + 4 nouveaux SLOs drill quarterly
│
├── Cadences drill (2 total post-D40)
│   ├── Weekly drill                                  (existant D17)
│   │   ├── CronJob d17-dr-drill-cronjob.yaml
│   │   ├── Schedule dimanche 03:00 UTC
│   │   ├── RTO target 1800s (30 min)
│   │   ├── 5 verifiers techniques
│   │   └── Cluster existant (drill ns)
│   │
│   └── Quarterly drill                               (D40, NEW)
│       ├── CronJob efe-dr-drill-quarterly.yaml
│       ├── Schedule 1er du mois trimestriel 04:00 UTC
│       ├── RTO target 2700s (45 min)
│       ├── 5 verifiers techniques + Hello-World workflow
│       ├── Cluster from-scratch (kind ou staging fresh)
│       └── Compliance evidence SOC 2 / ISO 27001 / NIS2
│
├── Scripts dr/                                       (post-D40)
│   ├── restore.sh                                    (existant D17)
│   ├── smoke_test.sh                                 (existant D17)
│   ├── setup-test-env.sh                             (existant D17)
│   └── run_quarterly_drill.sh                        (D40, NEW)
│
├── Helm templates                                    (post-D40)
│   ├── d17-backup-cronjobs.yaml                      (existant)
│   ├── d17-backup-pdb.yaml                           (existant)
│   ├── d17-backup-rbac.yaml                          (existant)
│   ├── d17-backup-secret.yaml                        (existant)
│   ├── d17-dr-drill-cronjob.yaml                     (existant — weekly)
│   ├── d40-dr-drill-quarterly-cronjob.yaml           (D40, NEW)
│   └── d17-prometheus-rules.yaml                     (étendu : 4 nouvelles règles)
│
├── Pin tests Python                                  (post-D40)
│   ├── test_d17_backup_dr.py                         (existant, 906 LOC)
│   ├── test_d17_backup_dr_architecture.py            (existant, 261 LOC)
│   └── test_dr_drill_runbook.py                      (D40, NEW, ~280 LOC)
│
└── Documentation                                     (post-D40)
    ├── docs/bench/d17_backup_dr_architecture.md      (existant 1 875 LOC)
    ├── docs/audits/d17_backup_dr_audit_report.md     (existant)
    ├── docs/EFE_BACKUP_EXTENSION_ARCHITECTURE.md     (CETTE DOC)
    ├── docs/docs/howto/backup-and-restore.md         (existant 368 LOC, étendu)
    └── docs/compliance/dr_drill_quarterly_evidence.md  (D40, NEW, template)
```

---

## §1. Mission de l'extension M7 (Φ-F)

### §1.1 Énoncé opérationnel

L'extension M7 a **trois missions imbriquées** :

1. **Combler le gap business** entre la couverture de drill weekly
   (5 verifiers techniques post-restore) et la **réalité d'un
   incident production** (un opérateur veut savoir : "est-ce qu'un
   tenant peut faire son workflow business — register catalog,
   register class, execute, get response — après le restore ?").

2. **Combler le gap compliance** : les frameworks SOC 2 CC9.1,
   ISO 27001 A.17.1.3, NIS2 Art. 21 et RGPD Art. 32 exigent une
   **DR drill formelle quarterly** avec **runbook documenté**
   et **evidence** (timestamp + résultats archivés). Le drill
   weekly D17 est **non-conforme** pour ces frameworks (trop
   fréquent + pas de runbook ré-exécuté).

3. **Combler le gap documentation** : la doc
   `backup-and-restore.md` peut driftter du code réel à chaque
   sprint. Sans un drill qui **rejoue la doc**, on ne sait pas si
   la doc reste opérationnelle. Le quarterly drill exécute les
   commandes documentées **dans l'ordre exact**, ce qui force la
   doc à rester véridique.

### §1.2 Périmètre strict du document

Ce document **NE traite PAS** :

* Les modifications du crate Rust `efe-backup` — l'extension est
  **purement orchestration** (shell + Helm + pin test). Le
  binaire `efe-backup` reste inchangé.
* Les drills monthly / daily — ils n'existent pas en cible v1.0
  GA. Seuls weekly + quarterly. Justification : couverture +
  coût : weekly = drill technique routine cheap ; quarterly =
  drill business runbook expensive.
* La couverture multi-region active-active — hors scope D40,
  candidate D45+.
* La sauvegarde de logs Loki / métriques Prometheus —
  transient, hors scope D17 baseline + D40.
* Les changements de SLO RTO existant (30 min weekly) — D40
  ajoute un SLO RTO **distinct** pour le quarterly (45 min).

### §1.3 Mission complémentaire — compliance evidence

L'extension M7 D40 ouvre la voie à la **compliance pack** :

```
.ci-evidence/dr-drill-quarterly/
├── 2026-Q1/
│   ├── outcome.json           # DrDrillOutcome v2 (Hello-World extended)
│   ├── kind-cluster.log       # logs kind create + helm install + restore
│   ├── helloworld.log         # logs du workflow business
│   ├── verifier-suite.log     # logs des 5 verifiers
│   ├── timing.json            # RTO measurement détaillé
│   └── attestation.txt        # signed by 4-eyes pair (auditeur + ingé)
├── 2026-Q2/
├── 2026-Q3/
├── 2026-Q4/
└── README.md                  # explique le format pour audit external
```

Sans le D40, ce pack **n'existe pas** ; les auditeurs SOC 2 /
ISO 27001 verraient un **gap critique** dans le rapport.

---

## §2. Position dans la topologie EFE post-M6

### §2.1 Graph de dépendances post-D40

```text
             ┌─────────────────────────────────────────┐
             │       Helm chart EFE (deploy/helm/efe)  │
             │  ┌────────────────────┐  ┌────────────┐ │
             │  │ d17-dr-drill       │  │ D40 NEW :  │ │
             │  │ (weekly, technical)│  │ d40-dr-... │ │
             │  └────────┬───────────┘  └─────┬──────┘ │
             └───────────┼─────────────────────┼───────┘
                         │                     │
              kubectl    ▼                     ▼  kubectl
               ┌─────────────────┐    ┌──────────────────┐
               │ efe-backup CLI  │    │ run_quarterly_   │
               │ drill subcmd    │    │ drill.sh         │
               │ (5 verifiers)   │    │ (NEW D40)        │
               └────────┬────────┘    └────────┬─────────┘
                        │                       │
                        ▼                       ▼
                   ┌─────────────────────────────────────┐
                   │  RestoreOrchestrator (efe-backup)   │
                   │  - vault, postgres, bundles         │
                   └────────────────┬────────────────────┘
                                    │
                                    ▼
                   ┌─────────────────────────────────────┐
                   │  Storage (S3-compatible)            │
                   │  - efe-backups bucket               │
                   │  - efe-bundle-backups bucket        │
                   └─────────────────────────────────────┘

                                    │
   D40 quarterly path ADDS :        │
                                    ▼
                   ┌─────────────────────────────────────┐
                   │  Hello-World runbook business       │
                   │  - register catalog                 │
                   │  - register class                   │
                   │  - register sentence                │
                   │  - resolve catalog                  │
                   │  - execute (Mode-1 Mock + real bundle)
                   │  - assert response shape + integrity│
                   └─────────────────────────────────────┘
```

### §2.2 Direction stricte des dépendances

* `dr/run_quarterly_drill.sh` dépend du **binaire `efe-backup`**
  (déjà construit AS-IS).
* `dr/run_quarterly_drill.sh` dépend du **binaire `efe-server`**
  (M4) pour le Hello-World workflow business.
* Le Helm CronJob `efe-dr-drill-quarterly` dépend de l'image
  Docker `efe-server:<tag>` qui contient les 2 binaires.
* `test_dr_drill_runbook.py` dépend uniquement du **filesystem**
  (lit les fichiers shell + YAML + doc) — **zéro runtime
  dependency** côté pin test.
* La doc `backup-and-restore.md` est **consommée** par
  `run_quarterly_drill.sh` (le script référence les sections de
  la doc dans ses commentaires).

### §2.3 Position vs `efe-backup` (M7 baseline) + `efe-server` (M4)

| Surface | efe-backup (M7 GA, post-D40) | efe-server (M4) |
|---------|:------:|:---:|
| Drill weekly CronJob | ✅ existant | ❌ |
| Drill quarterly CronJob | ✅ NEW D40 | ❌ |
| Helm CronJob YAML | ✅ 5 templates D17 + 1 D40 | ❌ |
| `dr/restore.sh` | ✅ existant | ❌ |
| `dr/run_quarterly_drill.sh` | ✅ NEW D40 | ❌ |
| Hello-World runbook | ✅ NEW D40 (script + assertions) | ❌ — endpoints consommés |
| Pin test runbook | ✅ NEW D40 | ❌ |
| Compliance evidence pack | ✅ NEW D40 | ❌ |

---

## §3. État AS-IS au 2026-04-29 (post-D17 audit, 124 tests, 10 audit-fix)

### §3.1 Inventaire physique `crates/efe-backup/`

```
crates/efe-backup/src/
├── audit.rs              (~370 LOC) — BackupAuditEmitter (HMAC chain)
├── bin/
│   └── efe_backup.rs     (~1 140 LOC) — CLI 7 subcommands
├── bundles.rs            (~330 LOC) — BundleSnapshotter rclone
├── command_runner.rs     (~410 LOC) — TokioCommandRunner + MockCommandRunner
├── config.rs             (~600 LOC) — BackupConfig::from_env + RestoreTargetEnv
├── drill.rs              (~310 LOC) — DrDrill orchestrator
├── error.rs              (~315 LOC) — BackupError taxonomy (10 variants)
├── lib.rs                (~115 LOC) — re-exports
├── manifest.rs           (~830 LOC) — BackupManifest Ed25519-signed + TrustStore
├── metadata.rs           (~285 LOC) — SnapshotMetadata + SnapshotSource enum
├── postgres.rs           (~510 LOC) — PgBackRestSnapshotter avec PITR
├── restore.rs            (~485 LOC) — RestoreOrchestrator + cross-env guard
├── runtime_instrument.rs (~380 LOC) — InstrumentedSnapshot wrapper (D17-AC2 fix)
├── snapshot.rs           (~235 LOC) — Snapshotter trait + MockSnapshotter
├── vault.rs              (~415 LOC) — VaultSnapshotter Raft snapshot
├── verifiers.rs          (~720 LOC) — 5 Verifier impls
└── verify.rs             (~285 LOC) — Verifier trait + run_suite + VerifierKind

crates/efe-backup/tests/
└── d17_backup_integration.rs  (~150 LOC) — 1 e2e test

Total : ~7 900 LOC + 124 unit tests + 1 e2e test + 10 audit-fix
documentés.
```

### §3.2 Helm chart AS-IS — drill weekly

```
deploy/helm/efe/templates/
├── d17-backup-cronjobs.yaml          (4 CronJobs : pg-full, pg-diff, vault, bundle)
├── d17-backup-pdb.yaml               (PodDisruptionBudget)
├── d17-backup-rbac.yaml              (ServiceAccount + Role + RoleBinding)
├── d17-backup-secret.yaml            (Secret avec dev defaults)
├── d17-dr-drill-cronjob.yaml         (1 CronJob drill weekly)
├── d17-prometheus-rules.yaml         (3 PrometheusRules)
└── networkpolicy-backup-egress.yaml  (NetworkPolicy egress restrictions)
```

Le drill weekly :

```yaml
metadata:
  name: efe-dr-drill
spec:
  schedule: "0 3 * * 0"           # Sunday 03:00 UTC
  concurrencyPolicy: Forbid
  successfulJobsHistoryLimit: 4
  failedJobsHistoryLimit: 8
  startingDeadlineSeconds: 1200
  jobTemplate:
    spec:
      activeDeadlineSeconds: 2700  # 45 min hard timeout
      backoffLimit: 0              # drills don't retry
      template:
        spec:
          containers:
            - name: efe-dr-drill
              image: efe-server:<tag>
              args: ["drill", "--target-env", "drill"]
              env:
                - name: EFE_BACKUP_RTO_TARGET_S
                  value: "1800"  # 30 min target
              resources:
                requests: { cpu: 500m, memory: 512Mi }
                limits:   { cpu: "4", memory: 4Gi }
```

### §3.3 Scripts shell `dr/` AS-IS

```
dr/
├── restore.sh           (167 LOC)  — full restore flow + RTO measure
├── smoke_test.sh        (84 LOC)   — 4 post-restore checks
└── setup-test-env.sh    (82 LOC)   — kind + minio + postgres + helm
```

* `restore.sh` :
  * 6 étapes : pre-flight verify → vault restore → postgres
    restore → bundles restore → helm upgrade → post-restore
    verify → smoke
  * 8 exit codes distincts (1 = arg error, 2 = pre-flight,
    3 = vault, 4 = pg, 5 = bundles, 6 = helm, 7 = verify,
    8 = smoke, 11 = RTO violation)
  * Émet `dr/timing.json` avec RTO measurement

* `smoke_test.sh` :
  * Check 1 : metrics endpoint up + 5 D13 + 8 D17 instruments
    exposés
  * Check 2 : HMAC chain integrity (`verify-audit-chain --full`)
  * Check 3 : bundle signatures valid (`efe-bundle verify --all`)
  * Check 4 : at least 1 ready efe-server pod

* `setup-test-env.sh` :
  * `kind create cluster --name efe-dr-drill`
  * MinIO via `helm install minio/minio` (S3 compat)
  * Postgres via `helm install cnpg/cloudnative-pg`
  * Apply EFE chart with `backup.enabled=true`

### §3.4 Pin tests AS-IS

```
scripts/tests/
├── test_d17_backup_dr.py                (906 LOC) — pin test prod livraison
└── test_d17_backup_dr_architecture.py   (261 LOC) — pin test architecture
```

Ces deux fichiers couvrent **l'architecture D17** (configuration,
threat model, SLOs) + la **livraison D17** (CronJobs, Helm
templates, scripts, error codes, 4-eyes flow). Total ~1 167 LOC,
~85 pin tests Python.

### §3.5 Documentation AS-IS

```
docs/bench/d17_backup_dr_architecture.md           (1 875 LOC)
docs/audits/d17_backup_dr_audit_report.md          (post-audit 10 défauts)
docs/docs/howto/backup-and-restore.md              (368 LOC, runbook public)
```

### §3.6 Synthèse AS-IS — gaps identifiés

| Sprint | État AS-IS | Gap | Effort |
|---|---|---|---|
| **D40 quarterly** | 0 % (drill weekly only) | run_quarterly_drill.sh + Helm CronJob + pin test + compliance evidence | 4 jours-ingénieur |

Total : **4 jours-ingénieur**. ~700 LOC livrés (300 LOC shell +
180 LOC YAML + 280 LOC Python pin tests + ~100 LOC doc
extension).

---

## §4. Définition opérationnelle de `efe-backup` v1.0 GA

### §4.1 Le contrat GA

À la fin de l'extension M7 (post-D40) :

* **2 cadences de drill** opérationnelles : weekly (technique
  AS-IS) + quarterly (business runbook NEW).
* **Helm CronJob** `efe-dr-drill-quarterly` configurable via
  `values.yaml` (schedule, RTO target, image, namespace).
* **Script shell** `dr/run_quarterly_drill.sh` orchestre :
  1. Setup kind cluster (réutilise `dr/setup-test-env.sh`)
  2. Install Helm chart with `backup.enabled=true`
  3. Restore from latest successful backup (`dr/restore.sh`)
  4. **Hello-World workflow business end-to-end** (NEW)
  5. 5 verifiers via `efe-backup drill --target-env=drill`
  6. Compliance evidence emit (`outcome.json` +
     `helloworld.log` + `attestation.txt`)
  7. Cleanup kind cluster
* **Hello-World runbook** : 6 étapes business minimal mais
  complet (cf. §10).
* **Pin test** `test_dr_drill_runbook.py` (~30 tests) qui pin
  la structure du script + YAML + doc + assertions
  Hello-World + compliance evidence schema.
* **Compliance evidence** : pack `.ci-evidence/dr-drill-quarterly/<YYYY>-Q<N>/`
  archivé après chaque exécution (S3 immutable, GDPR-safe).
* **4 nouveaux SLOs** :
  * `D40-DR-1` — quarterly drill RTO ≤ 45 min (bootstrap-from-zero overhead)
  * `D40-DR-2` — Hello-World workflow ≤ 60 s end-to-end
  * `D40-DR-3` — quarterly drill success rate ≥ 95 % sur 4 derniers Q
  * `D40-DR-4` — runbook freshness ≤ 90 jours (= un drill quarterly réussi dans les 90 derniers jours)

### §4.2 Versioning produit

| efe-backup version | Sprint | Drills | Hello-World | Compliance | Tests |
|---|---|---|---|---|---:|
| 0.1.18+post-audit (D17 livré + audit) | D17 | weekly | ❌ | partial | 124 |
| **1.0.0 GA** | post-D40 | weekly + **quarterly** | ✅ | ✅ SOC 2 / ISO 27001 / NIS2 / RGPD | ~155 |

### §4.3 Gating entre versions

Le bump de `0.1.18` → `1.0.0` exige :

* Cargo.toml `version` workspace bumpé en `1.0.0`.
* Pin test `test_dr_drill_runbook.py` GREEN (~30 tests).
* `dr/run_quarterly_drill.sh` testé sur kind localement avec
  `--keep` + cleanup manuel.
* Helm `efe-dr-drill-quarterly` template render check via
  `helm template ./deploy/helm/efe`.
* Première compliance evidence Q1 2026 archivée
  (`.ci-evidence/dr-drill-quarterly/2026-Q2/`).
* CHANGELOG.md entry M7 extension complète.
* Update `docs/docs/howto/backup-and-restore.md` § "DR drill
  quarterly" (NEW section).

---

## §5. Principes directeurs

### §5.1 Principes opposables en revue PR

| ID | Principe | Application |
|----|---|---|
| **B-1** | **Drill ≠ test unitaire** | Le drill quarterly DOIT exécuter le **même path** qu'un opérateur en disaster réel. Pas de raccourcis (e.g. mock S3 ou stub Postgres). Si on shortcut, le drill ne mesure rien. |
| **B-2** | **Hello-World = business path** | Le Hello-World workflow utilise les **vrais endpoints gRPC** (`SwarmService.Execute`, `CatalogService.RegisterCatalog`). Pas de Mock client. Sans le path business, le drill ne valide que les binaires, pas la valeur business. |
| **B-3** | **Compliance evidence immutable** | Le pack `outcome.json + logs + attestation.txt` est uploadé vers un bucket S3 avec **Object Lock** (compliance mode, 7 ans rétention). Pas modifiable, pas effaçable, pas par admin. Exigence SOC 2 CC9.2 + RGPD Art. 32. |
| **B-4** | **Runbook = code** | La doc `backup-and-restore.md` § "DR" est référencée par `run_quarterly_drill.sh` via numéros de section. Si la doc drift, le pin test fail. La doc devient une **artifacte vivant**, pas un artifact mort. |
| **B-5** | **Pin test fail-fast** | Le pin test `test_dr_drill_runbook.py` exit non-0 sur la première divergence (script absent, CronJob mal configuré, doc qui ne mentionne plus une étape). Pas de "warning" silencieux. |
| **B-6** | **Cleanup obligatoire** | Le script DOIT cleanup le kind cluster même en cas d'échec mid-drill. `trap` sur `EXIT` qui fait `kind delete cluster`. Sinon CI runners s'engorgent. |
| **B-7** | **Idempotency** | Re-run le script 2 fois consécutivement DOIT fonctionner. La 2e run réutilise le cluster (si `--keep`) ou crée un nouveau cluster propre. Pas d'état orphelin. |
| **B-8** | **Cross-env safety** | Le drill quarterly UTILISE `--force-cross-env` (restaure un backup prod dans drill ns) MAIS NE PEUT PAS écrire vers prod. Garde-fou : `EFE_BACKUP_TARGET_ENV=drill` hardcoded dans le script ; refus si autre. |

### §5.2 Ce que `efe-backup` D40 refuse d'implémenter

* **Drill monthly** — coût inutile ; weekly couvre la fréquence
  technique, quarterly couvre la fréquence compliance.
* **Drill on-demand via webhook** — hors scope D40 ; candidate
  D45+ (chaos engineering pipeline).
* **Drill multi-region** — hors scope D40 ; nécessite multi-region
  active-active D45+.
* **Drill avec données réelles tenant** — interdit. Les drills
  utilisent des **datasets synthétiques** (Hello-World fixtures
  uniquement). Conformité RGPD par design.
* **Modification du drill weekly** — hors scope D40. Le weekly
  AS-IS reste tel quel. Seul le quarterly est ajouté.

---

## §6. Invariants structurels

### §6.1 Invariants de structure de fichiers

```
crates/efe-backup/src/
├── (inchangé — pas de patch Rust D40)

dr/
├── restore.sh                          (existant D17)
├── smoke_test.sh                       (existant D17)
├── setup-test-env.sh                   (existant D17)
└── run_quarterly_drill.sh              (NEW D40)

deploy/helm/efe/templates/
├── (existants D17, inchangés)
└── d40-dr-drill-quarterly-cronjob.yaml (NEW D40)

deploy/helm/efe/values.yaml
└── backup.drillQuarterly.* (NEW section, ~30 LOC)

scripts/tests/
└── test_dr_drill_runbook.py            (NEW D40)

docs/
├── EFE_BACKUP_EXTENSION_ARCHITECTURE.md  (CETTE DOC)
├── docs/howto/backup-and-restore.md      (étendu : section "DR drill quarterly")
└── compliance/
    └── dr_drill_quarterly_evidence.md    (NEW D40 — template + format)

.ci-evidence/
└── dr-drill-quarterly/                  (NEW D40, .gitignored — créé par CI)
    ├── README.md                         (committed)
    └── <YYYY>-Q<N>/                      (généré au runtime, archivé S3)
```

### §6.2 Invariants de runtime du quarterly drill

* **Setup kind avant tout** — le script REFUSE de démarrer si
  `kind` ou `helm` ou `kubectl` est absent. Pas de fallback.
* **Cleanup garanti** — `trap 'kind delete cluster --name "$CLUSTER_NAME" || true' EXIT` au début.
* **Hello-World fail-closed** — si UNE seule assertion business
  fail, le drill exit 1 + emit `outcome.json` avec
  `success: false`.
* **Compliance evidence créé toujours** — succès OU échec.
  L'auditeur veut voir aussi les drills qui ont raté.
* **RTO measurement step-by-step** — `timing.json` enrichi avec :
  ```json
  {
    "phases": {
      "kind_setup_secs":    120,
      "helm_install_secs":  180,
      "restore_secs":       360,
      "verifier_suite_secs": 60,
      "helloworld_secs":    45,
      "cleanup_secs":       30,
      "total_secs":         795
    },
    "rto_target_secs": 2700,
    "within_rto": true
  }
  ```

### §6.3 Invariants de versioning compliance evidence

```json
{
  "schema_version": "1.0.0",
  "drill_kind": "quarterly",
  "drill_label": "drill-2026-Q2-20260601T040000Z",
  "started_at": "2026-06-01T04:00:00Z",
  "finished_at": "2026-06-01T04:13:15Z",
  "duration_secs": 795,
  "rto_target_secs": 2700,
  "within_rto": true,
  "phases": { /* ... */ },
  "verifiers": [
    { "kind": "audit_chain", "ok": true, "detail": "..." },
    { "kind": "bundle_signatures", "ok": true, "detail": "..." },
    { "kind": "oidc_issuers", "ok": true, "detail": "..." },
    { "kind": "catalog_smoke", "ok": true, "detail": "..." },
    { "kind": "execute_smoke", "ok": true, "detail": "..." }
  ],
  "helloworld": {
    "tenant_id": "drill-helloworld-tenant",
    "catalog_name": "helloworld-catalog",
    "version": 1,
    "class_name": "greeting",
    "sentence": "Hello, world!",
    "execute_ok": true,
    "response_shape_ok": true,
    "latency_ms": 245
  },
  "compliance_frameworks": ["SOC2-CC9.1", "ISO27001-A.17.1.3", "NIS2-Art21", "RGPD-Art32"],
  "attestation_signature": "<ed25519-sig>",
  "attestation_pubkey_id": "drill-attestation-2026"
}
```

* Schema versioned (1.0.0).
* Champs obligatoires : `schema_version`, `drill_kind`, `drill_label`, `started_at`, `finished_at`, `duration_secs`, `success_overall`.
* Champs ajoutés post-MVP = `Option<T>` ou versioning bump.

---

## §7. Vue d'ensemble — sprint D40 unique

### §7.1 Tableau d'agrégation

| Sprint | Phase | Durée | Livrable principal | LOC | Pin tests | e2e |
|---|---|---|---|---:|---:|---|
| **D40** | Φ-F | 4 j | `run_quarterly_drill.sh` + Helm CronJob + Hello-World + pin test + compliance evidence | ~700 | 30 | 1 (kind manuel) |

Total : **~700 LOC + 30 pin tests + 1 manual e2e**.

### §7.2 Dépendances inter-sprints

```text
                D17 (efe-backup baseline)
                     │
                     │ requires : 5 verifiers, drill weekly
                     │ requires : crates/efe-backup compilable
                     │
                D17-audit (10 audit-fix)
                     │
                     │ requires : audit fixes shipped
                     │
                ... (D18-D39 : 22 sprints intercalaires)
                     │
                D40 (drill quarterly + Hello-World)
                     │
                     │ requires : efe-server image avec
                     │     efe-backup + efe-server binaires
                     │ requires : kind / helm / kubectl in CI image
                     │
                     ▼
                M7 GA v1.0
```

### §7.3 Volumétrie code attendue

```
D40 — DR drill quarterly + Hello-World runbook :
  dr/run_quarterly_drill.sh (NEW)              → ~300 LOC bash
  deploy/helm/efe/templates/d40-...yaml (NEW)  → ~120 LOC YAML
  deploy/helm/efe/values.yaml (PATCH)          → +30 LOC YAML
  scripts/tests/test_dr_drill_runbook.py (NEW) → ~280 LOC Python
  docs/docs/howto/backup-and-restore.md (PATCH) → +80 LOC Markdown
  docs/compliance/dr_drill_quarterly_evidence.md (NEW) → ~150 LOC Markdown
  .ci-evidence/dr-drill-quarterly/README.md (NEW) → ~30 LOC Markdown
  CHANGELOG.md (PATCH)                          → +60 LOC Markdown

Net : ~1 050 LOC total dont ~700 LOC actifs (script + YAML +
pin test) et ~350 LOC documentation (compliance + runbook + changelog).
```

---

## §8. Surface publique cible (script + CronJob + pin test + doc)

### §8.1 Inventaire post-D40 (GA)

```bash
# Surface CLI publique post-D40 (inchangée — pas de patch Rust)
$ efe-backup --help
USAGE:
    efe-backup [OPTIONS] <SUBCOMMAND>

SUBCOMMANDS:
    snapshot     # ... (D17)
    restore      # ... (D17)
    verify       # ... (D17)
    drill        # ... (D17, weekly path)
    list         # ... (D17)
    delete       # ... (D17)
    rotate       # ... (D17)

# Surface scripts post-D40
$ ls dr/
restore.sh                   # D17 existant
smoke_test.sh                # D17 existant
setup-test-env.sh            # D17 existant
run_quarterly_drill.sh       # D40 NEW

# Helm templates post-D40
$ ls deploy/helm/efe/templates/d40-*
deploy/helm/efe/templates/d40-dr-drill-quarterly-cronjob.yaml

# Values.yaml post-D40
$ grep -A 20 "drillQuarterly:" deploy/helm/efe/values.yaml
backup:
  drillQuarterly:
    enabled: true
    schedule: "0 4 1 1,4,7,10 *"        # 1er du mois trimestriel 04:00 UTC
    namespace: efe-dr-drill-quarterly
    rtoTargetSeconds: 2700              # 45 min (vs 30 min weekly)
    activeDeadlineSeconds: 3600         # 60 min hard timeout
    helloWorld:
      enabled: true
      tenantId: "drill-helloworld-tenant"
      catalogName: "helloworld-catalog"
      className: "greeting"
      sentence: "Hello, world!"
      maxLatencyMs: 60000               # 60 s
    complianceEvidence:
      enabled: true
      bucket: "efe-compliance-evidence"
      objectLockMode: "COMPLIANCE"      # AWS S3 Object Lock compliance
      retentionYears: 7
    image: ""                           # default = same as efe-server
    imageTag: ""                        # default = same as efe-server
```

### §8.2 Mapping consommateur → surface

| Consommateur | Surface principale | Sprint |
|---|---|---|
| Helm chart EFE | template `d40-dr-drill-quarterly-cronjob.yaml` | D40 |
| Operator manuel | `./dr/run_quarterly_drill.sh --kind-cluster <name>` | D40 |
| CI/CD GitHub Actions | `pytest scripts/tests/test_dr_drill_runbook.py` | D40 |
| Auditor SOC 2 / ISO 27001 | `.ci-evidence/dr-drill-quarterly/<YYYY>-Q<N>/outcome.json` | D40 |
| Documentation | `docs/docs/howto/backup-and-restore.md` § "Quarterly DR drill" | D40 |

---

## §9. Sprint **D40** — `dr/run_quarterly_drill.sh` + Helm CronJob `efe-dr-drill-quarterly`

### §9.1 Objectif

Livrer un script shell + un CronJob Helm + un pin test qui ensemble
implémentent le **quarterly DR drill end-to-end** :

1. **Bootstrap from zero** — création d'un cluster kind éphémère
   (ou utilisation d'un cluster staging fresh).
2. **Helm install** EFE avec `backup.enabled=true`.
3. **Restore** depuis le dernier backup prod réussi via
   `dr/restore.sh`.
4. **Hello-World workflow business** — 6 étapes business minimal
   mais end-to-end (cf. §10).
5. **Verifier suite** — 5 verifiers techniques via
   `efe-backup drill`.
6. **Compliance evidence emit** — `outcome.json` + logs +
   attestation signée.
7. **Cleanup** — destroy kind cluster.

### §9.2 Architecture du script

```text
dr/run_quarterly_drill.sh structure (~300 LOC bash) :

#!/usr/bin/env bash
set -euo pipefail

# === 0. Args + env (40 LOC) ===
parse_args() { ... }    # --cluster-name, --keep, --backup-id, --evidence-dir
load_env() { ... }      # validate kind, helm, kubectl available

# === 1. Cleanup trap (10 LOC) ===
trap cleanup EXIT
cleanup() {
    if [[ "$KEEP_CLUSTER" -eq 0 ]]; then
        kind delete cluster --name "$CLUSTER_NAME" 2>/dev/null || true
    fi
    emit_evidence  # always, even on fail
}

# === 2. Phases (200 LOC) ===
phase_kind_setup() { ... }       # ./dr/setup-test-env.sh
phase_helm_install() { ... }     # apply EFE chart with backup.enabled=true
phase_restore() { ... }          # ./dr/restore.sh <backup-id> --target=drill --force-cross-env
phase_helloworld() { ... }       # 6 business steps
phase_verifier_suite() { ... }   # efe-backup drill --target-env=drill
phase_compliance_evidence() { ... }  # emit outcome.json + sign attestation

# === 3. Main (30 LOC) ===
main() {
    log "🚀 DR drill quarterly starting"
    T0=$(date -u +%s)
    phase_kind_setup
    phase_helm_install
    phase_restore
    phase_helloworld
    phase_verifier_suite
    T1=$(date -u +%s)
    local DURATION=$((T1 - T0))
    log "✅ DR drill quarterly complete in ${DURATION}s"
}
main "$@"

# === 4. Helpers (30 LOC) ===
log() { ... }
emit_evidence() { ... }
sign_attestation() { ... }
```

### §9.3 Helm CronJob détaillé

```yaml
{{- /*
  Sprint D40 — Quarterly DR drill CronJob.

  EXTENDS the weekly drill (`d17-dr-drill-cronjob.yaml`) with :
    * Schedule trimestriel (1er du mois Q1/Q2/Q3/Q4 à 04:00 UTC).
    * Hello-World business workflow end-to-end.
    * Compliance evidence emit (S3 Object Lock).
    * RTO target relaxed to 45 min (overhead bootstrap-from-zero).
*/}}
{{- if and .Values.backup.enabled .Values.backup.drillQuarterly.enabled }}
apiVersion: batch/v1
kind: CronJob
metadata:
  name: {{ include "efe.fullname" . }}-dr-drill-quarterly
  labels: {{- include "efe.labels" . | nindent 4 }}
    app.kubernetes.io/component: dr-drill-quarterly
    compliance.efe.ai/framework: "soc2,iso27001,nis2,rgpd"
spec:
  schedule: {{ .Values.backup.drillQuarterly.schedule | quote }}
  concurrencyPolicy: Forbid
  successfulJobsHistoryLimit: 8           # 2 ans de drills success
  failedJobsHistoryLimit: 8               # 2 ans de drills failed
  startingDeadlineSeconds: 3600
  jobTemplate:
    spec:
      activeDeadlineSeconds: {{ default 3600 .Values.backup.drillQuarterly.activeDeadlineSeconds }}
      backoffLimit: 0
      template:
        metadata:
          labels: {{- include "efe.labels" . | nindent 12 }}
            app.kubernetes.io/component: dr-drill-quarterly
        spec:
          restartPolicy: Never
          serviceAccountName: {{ include "efe.fullname" . }}-backup
          containers:
            - name: efe-dr-drill-quarterly
              image: {{ default .Values.image.repository .Values.backup.drillQuarterly.image }}:{{ default .Values.image.tag .Values.backup.drillQuarterly.imageTag }}
              imagePullPolicy: {{ default "IfNotPresent" .Values.image.pullPolicy }}
              command:
                - "/usr/local/bin/run_quarterly_drill.sh"
              args:
                - "--target-env=drill"
                - "--evidence-dir=/var/lib/efe/compliance-evidence"
              envFrom:
                - configMapRef:
                    name: {{ include "efe.fullname" . }}-config
              env:
                - name: EFE_BACKUP_RTO_TARGET_S
                  value: {{ default 2700 .Values.backup.drillQuarterly.rtoTargetSeconds | quote }}
                - name: EFE_HELLOWORLD_ENABLED
                  value: {{ .Values.backup.drillQuarterly.helloWorld.enabled | quote }}
                - name: EFE_HELLOWORLD_TENANT_ID
                  value: {{ .Values.backup.drillQuarterly.helloWorld.tenantId | quote }}
                - name: EFE_HELLOWORLD_CATALOG_NAME
                  value: {{ .Values.backup.drillQuarterly.helloWorld.catalogName | quote }}
                - name: EFE_HELLOWORLD_CLASS_NAME
                  value: {{ .Values.backup.drillQuarterly.helloWorld.className | quote }}
                - name: EFE_HELLOWORLD_SENTENCE
                  value: {{ .Values.backup.drillQuarterly.helloWorld.sentence | quote }}
                - name: EFE_HELLOWORLD_MAX_LATENCY_MS
                  value: {{ .Values.backup.drillQuarterly.helloWorld.maxLatencyMs | quote }}
                - name: EFE_COMPLIANCE_EVIDENCE_BUCKET
                  value: {{ .Values.backup.drillQuarterly.complianceEvidence.bucket | quote }}
                - name: EFE_COMPLIANCE_EVIDENCE_RETENTION_YEARS
                  value: {{ .Values.backup.drillQuarterly.complianceEvidence.retentionYears | quote }}
              resources:
                requests:
                  cpu: 1
                  memory: 1Gi
                limits:
                  cpu: "8"          # need beefy compute for kind+helm
                  memory: 16Gi      # kind cluster + minio + postgres in container
              volumeMounts:
                - name: compliance-evidence
                  mountPath: /var/lib/efe/compliance-evidence
                - name: dind
                  mountPath: /var/run/docker.sock
          volumes:
            - name: compliance-evidence
              persistentVolumeClaim:
                claimName: {{ include "efe.fullname" . }}-compliance-evidence
            - name: dind
              hostPath:
                path: /var/run/docker.sock  # required for kind-in-pod
                type: Socket
{{- end }}
```

### §9.4 Pin test détaillé

```python
# scripts/tests/test_dr_drill_runbook.py (~280 LOC)
"""Sprint D40 — pin test for DR drill quarterly runbook.

Anti-regression : guards M7 D40 surface :

  * dr/run_quarterly_drill.sh exists + executable
  * Script declares 6 phases (kind, helm, restore, helloworld, verifier, evidence)
  * Helm CronJob d40-dr-drill-quarterly-cronjob.yaml exists
  * CronJob schedule is quarterly (cron "0 4 1 1,4,7,10 *")
  * CronJob references run_quarterly_drill.sh
  * Compliance evidence emit + Object Lock referenced
  * Hello-World workflow has 6 steps + assertions
  * Doc backup-and-restore.md mentions "Quarterly DR drill" section
  * Compliance template exists with frameworks listed
"""

from pathlib import Path
import re

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "dr" / "run_quarterly_drill.sh"
HELM_TEMPLATE = REPO_ROOT / "deploy/helm/efe/templates/d40-dr-drill-quarterly-cronjob.yaml"
VALUES_YAML = REPO_ROOT / "deploy/helm/efe/values.yaml"
RUNBOOK_DOC = REPO_ROOT / "docs/docs/howto/backup-and-restore.md"
COMPLIANCE_DOC = REPO_ROOT / "docs/compliance/dr_drill_quarterly_evidence.md"

# 30 tests structurels couvrent script + Helm + values + pin compliance
def test_script_exists_and_executable() -> None:
    assert SCRIPT.exists()
    assert SCRIPT.stat().st_mode & 0o111  # executable bit set

def test_script_has_six_phases() -> None:
    content = SCRIPT.read_text()
    for phase in [
        "phase_kind_setup",
        "phase_helm_install",
        "phase_restore",
        "phase_helloworld",
        "phase_verifier_suite",
        "phase_compliance_evidence",
    ]:
        assert f"{phase}()" in content

def test_script_has_cleanup_trap() -> None:
    content = SCRIPT.read_text()
    assert "trap cleanup EXIT" in content
    assert "kind delete cluster" in content

def test_helm_cronjob_schedule_is_quarterly() -> None:
    content = HELM_TEMPLATE.read_text()
    # Schedule via values.yaml ; check default in values.
    assert "drillQuarterly" in content
    values = VALUES_YAML.read_text()
    assert '"0 4 1 1,4,7,10 *"' in values

def test_helm_cronjob_uses_run_quarterly_drill_script() -> None:
    content = HELM_TEMPLATE.read_text()
    assert "/usr/local/bin/run_quarterly_drill.sh" in content

def test_helm_cronjob_references_compliance_frameworks() -> None:
    content = HELM_TEMPLATE.read_text()
    assert "compliance.efe.ai/framework" in content
    for fw in ["soc2", "iso27001", "nis2", "rgpd"]:
        assert fw in content

def test_helloworld_workflow_has_six_steps() -> None:
    content = SCRIPT.read_text()
    for step in [
        "register_catalog",
        "register_class",
        "register_sentence",
        "resolve_catalog",
        "execute_workflow",
        "assert_response",
    ]:
        assert step in content

def test_runbook_doc_mentions_quarterly_drill_section() -> None:
    content = RUNBOOK_DOC.read_text()
    assert "Quarterly DR drill" in content
    assert "run_quarterly_drill.sh" in content

def test_compliance_doc_lists_frameworks() -> None:
    content = COMPLIANCE_DOC.read_text()
    for fw in ["SOC 2", "ISO 27001", "NIS2", "RGPD"]:
        assert fw in content

def test_compliance_evidence_schema_versioned() -> None:
    content = COMPLIANCE_DOC.read_text()
    assert "schema_version" in content
    assert "1.0.0" in content

# ... ~20 autres tests structurels
```

### §9.5 CLI extension

```bash
# Drill quarterly invocation manuelle (operator local)
./dr/run_quarterly_drill.sh \
    --cluster-name efe-dr-drill-q-2026q2 \
    --backup-id "$(efe-backup list --since=7d --json | jq -r '.[0].label')" \
    --evidence-dir ./drill-evidence/2026-Q2

# Drill quarterly invocation auto via Helm CronJob
# (rendered by helm install/upgrade with backup.drillQuarterly.enabled=true)
kubectl get cronjob efe-dr-drill-quarterly -n efe
kubectl create job --from=cronjob/efe-dr-drill-quarterly drill-manual-2026q2 -n efe
kubectl logs -f job/drill-manual-2026q2 -n efe
```

### §9.6 Tests + pin tests D40

* **Pin tests Python** (`scripts/tests/test_dr_drill_runbook.py`,
  ~30 tests) — pin script, Helm, values, runbook doc, compliance
  doc.
* **Manual e2e test** : `./dr/run_quarterly_drill.sh --keep` sur
  une machine dev (Mac M1 OK, Linux x86 OK) — durée ~20-25 min,
  ressources kind 4 CPU + 8 Gi RAM.
* **CI integration** : GitHub Actions workflow
  `.github/workflows/quarterly-drill-pin-test.yml` qui exécute le
  pin test à chaque PR (validation structurelle uniquement, pas
  d'exécution kind).

### §9.7 Critères Go/No-Go D40

* GG-D40.1 : `dr/run_quarterly_drill.sh` exécutable + 6 phases
  livrées + cleanup trap.
* GG-D40.2 : Helm CronJob `efe-dr-drill-quarterly-cronjob.yaml`
  template render check OK.
* GG-D40.3 : Drill quarterly success en kind local <= 45 min.
* GG-D40.4 : Hello-World workflow termine en <= 60 s.
* GG-D40.5 : Compliance evidence pack généré + uploadé S3
  (Object Lock vérifié).
* GG-D40.6 : Pin tests `test_dr_drill_runbook.py` GREEN
  (~30 tests).
* GG-D40.7 : Doc `backup-and-restore.md` étendue avec section
  "Quarterly DR drill" (référencée par le pin test).
* GG-D40.8 : v1.0.0 GA tag.

---

## §10. Hello-World runbook (workflow business end-to-end)

### §10.1 Objectif

Le **Hello-World runbook** est un workflow business minimal mais
**complet** qui valide post-restore que :

* Le tenant peut **register** un catalog avec embedder spec.
* Le catalog peut **evolve** avec ajout de classes + sentences.
* Le catalog peut être **resolved** (read-mostly path).
* Un appel `Execute` retourne une **response shape valide** avec
  `verdict ∈ {VERDICT_CONVERGED, VERDICT_CONTINUE}`.
* La **HMAC chain** post-Execute reste cohérente.
* Le binaire répond dans `≤ 60 s` (SLO `D40-DR-2`).

### §10.2 Workflow détaillé (6 steps)

**Step 1 — Register catalog**

```bash
efe-server admin register-catalog \
    --tenant="$EFE_HELLOWORLD_TENANT_ID" \
    --name="$EFE_HELLOWORLD_CATALOG_NAME" \
    --version=v1 \
    --embedder-family=sentence-transformers \
    --embedder-model=paraphrase-multilingual-MiniLM-L12-v2 \
    --embedder-dim=384

# Assertion : exit 0 + stdout JSON {"key": "drill-helloworld-tenant/helloworld-catalog@v1", ...}
```

**Step 2 — Register class**

```bash
efe-server admin evolve-catalog \
    --from-key="drill-helloworld-tenant/helloworld-catalog@v1" \
    --append-class="$EFE_HELLOWORLD_CLASS_NAME" \
    --description="Drill Hello-World greeting class"

# Assertion : new catalog version v2 returned
```

**Step 3 — Register sentence**

```bash
efe-server admin evolve-catalog \
    --from-key="drill-helloworld-tenant/helloworld-catalog@v2" \
    --replace-class-anchors="$EFE_HELLOWORLD_CLASS_NAME" \
    --canonical-sentences="$EFE_HELLOWORLD_SENTENCE"

# Assertion : new catalog version v3 returned, anchors compiled
```

**Step 4 — Resolve catalog**

```bash
efe-server admin get-catalog \
    --key="drill-helloworld-tenant/helloworld-catalog@v3" \
    --include-compiled \
    --json

# Assertion : response.compiled[0].class_name == "greeting"
#             response.compiled[0].num_anchors >= 1
```

**Step 5 — Execute workflow**

```bash
grpcurl -plaintext \
    -d '{"raw_prompt": "Hello, world!", "spec": {"max_rounds": 3, ...}}' \
    -H "authorization: Bearer $DRILL_TOKEN" \
    "$EFE_GRPC_ENDPOINT" \
    efe.v1.SwarmService/Execute

# Assertion : response.verdict ∈ {CONVERGED, CONTINUE}
#             response.confidence > 0.5
#             response.total_ms < 60000  (60 s SLO D40-DR-2)
```

**Step 6 — Assert HMAC chain integrity post-Execute**

```bash
efe-server admin verify-audit-chain --since=1m --full

# Assertion : chain integrity == OK (every event signed + verified)
```

### §10.3 Json schema response Hello-World (D40)

```json
{
  "schema_version": "1.0.0",
  "tenant_id": "drill-helloworld-tenant",
  "catalog_name": "helloworld-catalog",
  "final_version": 3,
  "class_name": "greeting",
  "sentence": "Hello, world!",
  "anchors_compiled": 1,
  "execute": {
    "request_id": "drill-helloworld-...",
    "verdict": "VERDICT_CONVERGED",
    "confidence": 0.87,
    "total_ms": 245,
    "rounds_executed": 2
  },
  "hmac_chain_post_execute_ok": true,
  "all_steps_ok": true,
  "elapsed_secs": 12
}
```

### §10.4 Mode mock fallback

Si l'image Docker `efe-server` ne contient pas de **bundle
embedder réel** (kind cluster sans accès HuggingFace), le
Hello-World **fallback en mode Mock** :

* `register-catalog` avec `embedder-model=mock-deterministic`
* `Execute` avec `decode-mode=PassThrough` (raw centroid output)
* Assertions adaptées : `response.output.raw.values.len() == 384`

Le fallback est **opt-in** via `EFE_HELLOWORLD_FALLBACK_MOCK=true`.
Par défaut le drill **REFUSE** le fallback (B-2 — Hello-World =
business path réel).

---

## §11. SLO mapping (D17 SLOs + 4 nouveaux drill quarterly SLOs)

### §11.1 SLOs DR existants (D17)

| ID | SLO | Cible | Mesure |
|---|---|---|---|
| **N-DR-1** | RTO weekly drill | ≤ 30 min | `dr_drill_duration_seconds` |
| **N-DR-2** | RPO Postgres PITR | ≤ 5 min | WAL archive lag |
| **N-DR-3** | Backup success rate | ≥ 99 % | `efe_backup_runs_total{result="success"}` ratio |
| **N-DR-4** | Restore verify success | ≥ 99 % | `efe_backup_verify_total{result="success"}` ratio |
| **N-DR-5** | Audit chain integrity | == 100 % | `verify-audit-chain --full` |
| **N-DR-6** | Bundle signature integrity | == 100 % | `efe-bundle verify --all` |
| **N-DR-7** | OIDC reachability | == 100 % | curl health check |
| **N-DR-8** | Drill freshness weekly | ≤ 14 j | `dr_drill_last_success_timestamp` |

### §11.2 SLOs DR nouveaux (D40)

| ID | SLO | Cible | Mesure |
|---|---|---|---|
| **D40-DR-1** | RTO quarterly drill | ≤ 45 min | `quarterly_drill_duration_seconds` (NEW) |
| **D40-DR-2** | Hello-World latency | ≤ 60 s end-to-end | `helloworld_workflow_duration_seconds` (NEW) |
| **D40-DR-3** | Quarterly drill success rate | ≥ 95 % sur 4 derniers Q | `quarterly_drill_runs_total{result="success"}` (NEW) |
| **D40-DR-4** | Runbook freshness | ≤ 90 j depuis dernier drill OK | `quarterly_drill_last_success_timestamp` (NEW) |

### §11.3 Wiring instruments

```rust
// crates/efe-telemetry/src/metrics/d17_instruments.rs (PATCH D40)

// Nouveau metric : quarterly drill duration histogram.
pub fn quarterly_drill_duration_seconds() -> &'static HistogramVec {
    static M: OnceLock<HistogramVec> = OnceLock::new();
    M.get_or_init(|| HistogramVec::new(
        HistogramOpts::new(
            "efe_quarterly_drill_duration_seconds",
            "Quarterly DR drill end-to-end duration",
        ).buckets(vec![60.0, 300.0, 600.0, 1200.0, 1800.0, 2700.0, 3600.0, 7200.0]),
        &["phase"],  // labels : kind_setup, helm_install, restore, helloworld, verifier, total
    ).expect("metric"))
}

pub fn helloworld_workflow_duration_seconds() -> &'static HistogramVec {
    /* same shape, smaller buckets : 1.0 .. 60.0 s */
}

pub fn quarterly_drill_runs_total() -> &'static CounterVec {
    /* labels : result = success | failure */
}

pub fn quarterly_drill_last_success_timestamp() -> &'static IntGauge {
    /* unix timestamp last quarterly drill OK */
}
```

**Note** : ces 4 instruments seraient ajoutés à
`d17_instruments.rs` mais le sprint D40 NE TOUCHE PAS le code
Rust. La proposition est de **différer** au sprint D41 (audit fix
M7). Le drill quarterly initial fonctionne **sans** ces metrics —
le `outcome.json` archivé fournit la même information de manière
hors-ligne (via `.ci-evidence/`).

### §11.4 Compliance mapping SLO → framework

| SLO | SOC 2 | ISO 27001 | NIS2 | RGPD |
|---|:---:|:---:|:---:|:---:|
| D40-DR-1 RTO | ✅ CC9.1 | ✅ A.17.1.3 | ✅ Art. 21.2(c) | — |
| D40-DR-2 latency | — | ✅ A.17.2.1 | ✅ Art. 21.2(c) | — |
| D40-DR-3 success rate | ✅ CC9.1 | ✅ A.17.1.3 | ✅ Art. 21.2(c) | ✅ Art. 32.1(c) |
| D40-DR-4 freshness | ✅ CC9.1 | ✅ A.17.1.3 | ✅ Art. 21.2(c) | ✅ Art. 32.1(c) |

---

## §12. Tests + pin tests structurels

### §12.1 Inventaire tests post-D40

| Type | Sprint | Fichier | Tests | LOC |
|---|---|---|---:|---:|
| Pin test architecture D17 (existant) | D17 | `test_d17_backup_dr_architecture.py` | ~25 | 261 |
| Pin test livraison D17 (existant) | D17 | `test_d17_backup_dr.py` | ~85 | 906 |
| Pin test runbook D40 (NEW) | D40 | `test_dr_drill_runbook.py` | ~30 | 280 |
| Unit tests Rust (existants) | D17 | `crates/efe-backup/src/*.rs` (in-source) | 124 | inclus |
| Integration test Rust (existant) | D17 | `crates/efe-backup/tests/d17_backup_integration.rs` | 1 | 150 |
| Manual e2e kind | D40 | `./dr/run_quarterly_drill.sh --keep` | 1 | n/a |

Total post-D40 : **~141 pin tests + 125 unit/integration Rust** =
~266 tests cumulés.

### §12.2 Stratégie de test pin

Le pin test `test_dr_drill_runbook.py` couvre **3 niveaux** :

1. **Existence** — fichiers présents (script, YAML, doc).
2. **Structure** — script déclare 6 phases, CronJob référence le
   script, doc cite la section "Quarterly DR drill".
3. **Cohérence cross-fichier** — values.yaml schedule matche
   YAML CronJob, doc référence values.yaml, compliance doc
   liste les 4 frameworks.

Le pin test ne **teste PAS** :

* L'exécution du script (manuel sur kind).
* La sémantique du Hello-World workflow (manuel sur kind).
* La signature attestation (manuel via `gpg --verify` ou
  `openssl dgst`).

### §12.3 Manual e2e flow

```bash
# 1. Install dependencies
brew install kind helm kubectl jq grpcurl  # Mac
# or apt-get install ... on Linux

# 2. Build image with run_quarterly_drill.sh embedded
docker build -t efe-server:drill-test .

# 3. Run drill
./dr/run_quarterly_drill.sh \
    --cluster-name efe-drill-test \
    --backup-id "20260601T000000Z-full" \
    --evidence-dir ./test-evidence \
    --keep   # don't destroy cluster after

# 4. Inspect evidence
cat ./test-evidence/2026-Q2/outcome.json | jq

# 5. Cleanup
kind delete cluster --name efe-drill-test
```

---

## §13. Critères Go/No-Go

### §13.1 GO si tous ✅

* GG-D40.1 : Script `dr/run_quarterly_drill.sh` exécutable +
  6 phases livrées + cleanup trap.
* GG-D40.2 : Helm CronJob template render check OK.
* GG-D40.3 : Drill quarterly réussi sur kind local en ≤ 45 min.
* GG-D40.4 : Hello-World workflow réussi en ≤ 60 s.
* GG-D40.5 : Compliance evidence pack généré (outcome.json +
  logs + attestation).
* GG-D40.6 : Pin tests `test_dr_drill_runbook.py` GREEN
  (~30 tests).
* GG-D40.7 : Doc `backup-and-restore.md` étendue.
* GG-D40.8 : Workshop interne d'1 h pour familiariser l'équipe ops
  au runbook quarterly.

### §13.2 NO-GO si UN ❌

* Hello-World fallback Mock activé en cible production →
  défaut sémantique B-2.
* Compliance evidence non immutable (Object Lock absent) →
  défaut sémantique B-3.
* Cleanup trap absent → risque CI engorgement → défaut
  sémantique B-6.
* Pin test absent → drift documentation possible → défaut
  sémantique B-4.

### §13.3 Validation finale

* **Validation technique** : par 2 ingénieurs distincts (4-eyes,
  pattern audit D17).
* **Validation compliance** : revue par DPO + responsable
  conformité interne.
* **Validation business** : démo workshop (15 min) en sprint
  review.

---

## §14. Risques transverses + mitigations (8 risques)

### §14.1 Liste exhaustive

| ID | Risque | Sévérité | Probabilité | Mitigation |
|----|---|:---:|:---:|---|
| **B-R-1** | Kind cluster ne démarre pas en CI (Docker daemon down, OOM) | 🟠 HARD | 🟠 MED | (1) `docker info` pre-check ; (2) retry 1× sur OOM ; (3) cleanup en cas d'échec ; (4) PVC for evidence even when kind fail |
| **B-R-2** | HuggingFace down → embedder bundle download fail → restore stuck | 🟠 HARD | 🟠 MED | (1) Mode Mock fallback opt-in `EFE_HELLOWORLD_FALLBACK_MOCK` ; (2) bundle pre-pulled in CI image ; (3) timeout 300 s sur HF download |
| **B-R-3** | Latency Hello-World > 60 s sur runner faible (CI tier-2) | 🟡 MED | 🟠 MED | (1) Increase rtoTargetSeconds → 90s en kind ; (2) prod runner = staging cluster (pas kind) |
| **B-R-4** | Compliance evidence Object Lock not configured → audit miss | 🔴 CRIT | 🟢 LOW | (1) Pin test verify Object Lock mode == "COMPLIANCE" ; (2) Helm template enforces value ; (3) S3 bucket policy denies bucket Object Lock disable |
| **B-R-5** | Doc drift (backup-and-restore.md drift vs script) | 🟡 MED | 🟠 MED | (1) Pin test cross-checks doc ↔ script step labels ; (2) Pre-commit hook on doc edit |
| **B-R-6** | Cross-env restore accidentally writes to prod | 🔴 CRIT | 🟢 LOW | (1) `EFE_BACKUP_TARGET_ENV=drill` hardcoded in script ; (2) `--force-cross-env` only for source≠prod restore ; (3) NetworkPolicy denies prod egress from drill ns |
| **B-R-7** | Quarterly drill timeline slip (drift > 90 j) | 🟠 HARD | 🟠 MED | (1) PrometheusRule `D40DrillQuarterlyStale` alert at 80 j ; (2) Pager runbook escalation at 90 j |
| **B-R-8** | Compliance audit fail because evidence pack missing | 🔴 CRIT | 🟢 LOW | (1) Quarterly drill fail-emit evidence (success OR failure) ; (2) S3 bucket lifecycle disallow delete ; (3) Manual quarterly check by DPO |

### §14.2 Plan de réponse incident

```
DR-Drill-Quarterly fail incident :
1. PagerDuty alert fires (D40DrillQuarterlyStale at 80 j)
2. On-call ops responds within 4 h
3. Manual run : ./dr/run_quarterly_drill.sh --cluster-name drill-incident
4. If still fails : escalate to DR specialist team (24/7 rotation)
5. If 96 h pass without successful drill :
   * DPO notifies SOC 2 auditor (heads-up)
   * Eng VP convenes daily standup until resolved
   * Compliance evidence gap documented in incident report
6. Post-mortem within 7 j : root cause + fix + pin test extension
```

---

## §15. Volumétrie code attendue

```
D40 — DR drill quarterly :

  dr/run_quarterly_drill.sh (NEW)              → ~300 LOC bash
    Phase 0 args + env                            ~40 LOC
    Phase 1 cleanup trap                          ~10 LOC
    Phase 2 phases (6 functions × ~30 LOC)        ~180 LOC
    Phase 3 main + helpers                        ~70 LOC

  deploy/helm/efe/templates/d40-...-cronjob.yaml (NEW) → ~120 LOC YAML
  deploy/helm/efe/values.yaml (PATCH)          → +30 LOC YAML

  scripts/tests/test_dr_drill_runbook.py (NEW) → ~280 LOC Python
    ~30 pin tests structurels

  docs/docs/howto/backup-and-restore.md (PATCH) → +80 LOC Markdown
    Section "Quarterly DR drill"                  ~50 LOC
    Section "Compliance evidence"                 ~30 LOC

  docs/compliance/dr_drill_quarterly_evidence.md (NEW) → ~150 LOC Markdown
    Schema reference                              ~50 LOC
    Frameworks mapping                            ~50 LOC
    Audit external playbook                       ~50 LOC

  .ci-evidence/dr-drill-quarterly/README.md (NEW) → ~30 LOC Markdown

  CHANGELOG.md (PATCH)                          → +60 LOC Markdown

  Total                                         → ~1 050 LOC dont ~700 LOC actifs
```

Net : **~700 LOC actifs (script + YAML + pin test) + ~350 LOC
documentation**.

---

## §16. Plans d'implémentation détaillés (4 jours)

### §16.1 J1 — Script + CronJob + values

```
Matin (4h) :
  - Coder dr/run_quarterly_drill.sh phase 0-3
    * args + env validation
    * cleanup trap with kind delete
    * phase_kind_setup (call setup-test-env.sh)
    * phase_helm_install (helm upgrade --install)
    * phase_restore (call dr/restore.sh --target=drill --force-cross-env)
  - Tester en local sans Helm (skip phase_helm + phase_restore avec mock)

Aprem (4h) :
  - Coder phases 4-6 (helloworld, verifier, evidence)
    * phase_helloworld : 6 steps avec efe-server admin commands
    * phase_verifier_suite : efe-backup drill --target-env=drill
    * phase_compliance_evidence : emit outcome.json + sign attestation

Fin J1 : script complet local, pas encore testé en kind.
```

### §16.2 J2 — Helm CronJob + values + manual e2e

```
Matin (4h) :
  - deploy/helm/efe/templates/d40-dr-drill-quarterly-cronjob.yaml
    * basé sur d17-dr-drill-cronjob.yaml + extensions
    * volumeMounts compliance-evidence + dind
    * env vars Hello-World + compliance
  - deploy/helm/efe/values.yaml extension drillQuarterly section
  - helm template ./deploy/helm/efe — render check

Aprem (4h) :
  - Manual e2e sur kind local
    * docker build -t efe-server:drill-test .
    * ./dr/run_quarterly_drill.sh --cluster-name drill-test --keep
    * Debug — typiquement : path issues, env vars, kind version
  - Itérer jusqu'à success en ~25 min

Fin J2 : drill quarterly réussit en kind local end-to-end.
```

### §16.3 J3 — Pin tests + doc + compliance

```
Matin (4h) :
  - scripts/tests/test_dr_drill_runbook.py (~30 tests)
    * test_script_exists_and_executable
    * test_script_has_six_phases
    * test_script_has_cleanup_trap
    * test_helm_cronjob_schedule_is_quarterly
    * test_helm_cronjob_uses_run_quarterly_drill_script
    * test_helm_cronjob_references_compliance_frameworks
    * test_helloworld_workflow_has_six_steps
    * test_runbook_doc_mentions_quarterly_drill_section
    * test_compliance_doc_lists_frameworks
    * test_compliance_evidence_schema_versioned
    * ... (~20 autres)
  - pytest scripts/tests/test_dr_drill_runbook.py — GREEN

Aprem (4h) :
  - docs/docs/howto/backup-and-restore.md extension :
    * Section "Quarterly DR drill" (~50 LOC)
    * Section "Compliance evidence" (~30 LOC)
  - docs/compliance/dr_drill_quarterly_evidence.md NEW (~150 LOC)
  - .ci-evidence/dr-drill-quarterly/README.md NEW

Fin J3 : pin tests GREEN, docs cohérentes.
```

### §16.4 J4 — Manual validation + CHANGELOG + démo

```
Matin (4h) :
  - Re-run drill quarterly sur kind avec --evidence-dir
    * Verify outcome.json schema
    * Verify attestation.txt présent
    * Verify logs phase-by-phase
  - Re-run pin tests en local
  - cargo build -p efe-backup (sanity, pas de patch attendu)

Aprem (4h) :
  - CHANGELOG.md entry M7 extension D40 (~60 LOC)
  - Workshop interne 1h :
    * Démo runbook quarterly à l'équipe ops
    * Q&A sur cleanup trap, fallback Mock, compliance pack
  - Tag v1.0.0 GA

Fin J4 : M7 GA livré.
```

---

## §17. Compliance mapping (SOC 2 CC9.1, ISO 27001 A.17.1.3, NIS2 Art. 21, RGPD Art. 32)

### §17.1 SOC 2 Trust Services Criteria

* **CC9.1** — *Risk Mitigation* — "Identifies, selects, and
  develops risk mitigation activities for risks arising from
  potential business disruptions." Le quarterly drill est
  **l'evidence formelle** de ce contrôle.
* **CC9.2** — *Compliance with Confidentiality Commitments* —
  Object Lock S3 sur evidence garantit l'immuabilité (B-3).
* **A1.2** — *Availability* — Le drill quarterly mesure le RTO
  (D40-DR-1) et le succès rate (D40-DR-3).

### §17.2 ISO 27001:2022 Annex A

* **A.17.1.3** — *Verify, review and evaluate information
  security continuity* — Le drill quarterly DOIT être exécuté
  régulièrement et **documenté**. La section "Quarterly DR drill"
  de `backup-and-restore.md` + le pack `.ci-evidence/` répondent
  exactement à ce contrôle.
* **A.17.2.1** — *Availability of information processing
  facilities* — Hello-World workflow valide la disponibilité
  business.
* **A.5.30** — *ICT Readiness for business continuity* — Drill
  quarterly = readiness test.

### §17.3 NIS2 Article 21 (EU)

* **Art. 21.2(c)** — *Business continuity, such as backup
  management and disaster recovery, and crisis management* —
  Le drill quarterly avec evidence pack répond à cette obligation
  pour les entités essentielles + importantes.

### §17.4 RGPD / GDPR

* **Art. 32.1(c)** — *Capacité de rétablir la disponibilité des
  données à caractère personnel et l'accès à celles-ci dans des
  délais appropriés en cas d'incident physique ou technique* —
  RTO 45 min + Hello-World workflow validation = preuve de
  capacité.
* **Art. 32.1(d)** — *Procédure visant à tester, à analyser et à
  évaluer régulièrement l'efficacité* — Schedule quarterly +
  evidence archivé 7 ans = procédure régulière.

### §17.5 Compliance evidence checklist (par audit Q)

| Item | Source | Validation |
|---|---|---|
| Drill timestamp + duration | `outcome.json` | Auditeur lit JSON |
| Hello-World success | `outcome.json.helloworld.all_steps_ok == true` | Auditeur lit JSON |
| 5 verifiers OK | `outcome.json.verifiers[*].ok == true` | Auditeur lit JSON |
| RTO ≤ 45 min | `outcome.json.within_rto == true` | Auditeur lit JSON |
| Logs archivés | `kind-cluster.log + helloworld.log + verifier-suite.log` | Auditeur grep |
| Attestation 4-eyes | `attestation.txt + ed25519-sig` | Auditeur verify sig |
| Object Lock ON | S3 bucket policy | Auditeur run `aws s3api get-object-lock-configuration` |
| Retention 7 ans | S3 bucket lifecycle | Auditeur run `aws s3api get-bucket-lifecycle` |

---

## §18. Annexe — Hello-World JSON schema + assertions

### §18.1 Schema JSON Draft-07

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$id": "https://efe.ai/schemas/helloworld-1.0.0.json",
  "title": "EFE Hello-World runbook outcome (D40)",
  "type": "object",
  "required": [
    "schema_version",
    "tenant_id",
    "catalog_name",
    "final_version",
    "execute",
    "hmac_chain_post_execute_ok",
    "all_steps_ok",
    "elapsed_secs"
  ],
  "properties": {
    "schema_version": { "type": "string", "const": "1.0.0" },
    "tenant_id": { "type": "string", "minLength": 1 },
    "catalog_name": { "type": "string", "minLength": 1 },
    "final_version": { "type": "integer", "minimum": 1 },
    "class_name": { "type": "string", "minLength": 1 },
    "sentence": { "type": "string", "minLength": 1 },
    "anchors_compiled": { "type": "integer", "minimum": 0 },
    "execute": {
      "type": "object",
      "required": ["request_id", "verdict", "confidence", "total_ms", "rounds_executed"],
      "properties": {
        "request_id": { "type": "string", "minLength": 1 },
        "verdict": {
          "type": "string",
          "enum": [
            "VERDICT_CONVERGED",
            "VERDICT_CONTINUE",
            "VERDICT_DIVERGENCE",
            "VERDICT_MODE_COLLAPSE"
          ]
        },
        "confidence": { "type": "number", "minimum": 0, "maximum": 1 },
        "total_ms": { "type": "number", "minimum": 0 },
        "rounds_executed": { "type": "integer", "minimum": 0 }
      }
    },
    "hmac_chain_post_execute_ok": { "type": "boolean" },
    "all_steps_ok": { "type": "boolean" },
    "elapsed_secs": { "type": "integer", "minimum": 0, "maximum": 60 }
  }
}
```

### §18.2 Assertions Hello-World

```bash
# Bash assertions inline dans run_quarterly_drill.sh
assert_response_shape() {
    local response_file="$1"
    local schema_file="docs/compliance/helloworld-1.0.0.schema.json"

    # 1. JSON valid
    jq empty "$response_file" || { log "FAIL: response not valid JSON"; return 1; }

    # 2. all_steps_ok == true
    local ok=$(jq -r '.all_steps_ok' "$response_file")
    [[ "$ok" == "true" ]] || { log "FAIL: all_steps_ok != true"; return 1; }

    # 3. Latency budget
    local elapsed=$(jq -r '.elapsed_secs' "$response_file")
    [[ "$elapsed" -le 60 ]] || { log "FAIL: elapsed=$elapsed > 60s budget"; return 1; }

    # 4. Verdict valid
    local verdict=$(jq -r '.execute.verdict' "$response_file")
    case "$verdict" in
        VERDICT_CONVERGED|VERDICT_CONTINUE) : ;;
        *) log "FAIL: unexpected verdict $verdict"; return 1 ;;
    esac

    # 5. HMAC chain integrity
    local hmac=$(jq -r '.hmac_chain_post_execute_ok' "$response_file")
    [[ "$hmac" == "true" ]] || { log "FAIL: HMAC chain broken post-Execute"; return 1; }

    log "✅ Hello-World assertions all passed"
}
```

---

## §19. Annexe — Helm CronJob full YAML

(Voir §9.3 pour le YAML complet rendu.)

Champs `values.yaml` ajoutés :

```yaml
backup:
  drillQuarterly:
    enabled: true                              # default ON
    schedule: "0 4 1 1,4,7,10 *"               # 1st of quarter at 04:00 UTC
    namespace: efe-dr-drill-quarterly
    rtoTargetSeconds: 2700                     # 45 min
    activeDeadlineSeconds: 3600                # 60 min hard timeout
    image: ""                                  # default = efe-server image
    imageTag: ""                               # default = efe-server tag
    helloWorld:
      enabled: true
      tenantId: "drill-helloworld-tenant"
      catalogName: "helloworld-catalog"
      className: "greeting"
      sentence: "Hello, world!"
      maxLatencyMs: 60000                      # 60 s
      fallbackMockBundle: false                # B-2 — refuse mock by default
    complianceEvidence:
      enabled: true
      bucket: "efe-compliance-evidence"
      objectLockMode: "COMPLIANCE"             # AWS S3 Object Lock
      retentionYears: 7
      attestationKeyId: "drill-attestation-2026"
```

---

## §20. Annexe — exemple sortie pin test

```text
$ pytest scripts/tests/test_dr_drill_runbook.py -v

scripts/tests/test_dr_drill_runbook.py::test_script_exists_and_executable PASSED
scripts/tests/test_dr_drill_runbook.py::test_script_has_six_phases PASSED
scripts/tests/test_dr_drill_runbook.py::test_script_has_cleanup_trap PASSED
scripts/tests/test_dr_drill_runbook.py::test_script_args_accept_cluster_name PASSED
scripts/tests/test_dr_drill_runbook.py::test_script_args_accept_keep_flag PASSED
scripts/tests/test_dr_drill_runbook.py::test_script_args_accept_evidence_dir PASSED
scripts/tests/test_dr_drill_runbook.py::test_helm_cronjob_exists PASSED
scripts/tests/test_dr_drill_runbook.py::test_helm_cronjob_schedule_is_quarterly PASSED
scripts/tests/test_dr_drill_runbook.py::test_helm_cronjob_concurrency_forbid PASSED
scripts/tests/test_dr_drill_runbook.py::test_helm_cronjob_active_deadline_60min PASSED
scripts/tests/test_dr_drill_runbook.py::test_helm_cronjob_uses_run_quarterly_drill_script PASSED
scripts/tests/test_dr_drill_runbook.py::test_helm_cronjob_references_compliance_frameworks PASSED
scripts/tests/test_dr_drill_runbook.py::test_helm_cronjob_volume_dind PASSED
scripts/tests/test_dr_drill_runbook.py::test_helm_cronjob_volume_compliance_evidence PASSED
scripts/tests/test_dr_drill_runbook.py::test_helm_cronjob_resources_8cpu_16gi PASSED
scripts/tests/test_dr_drill_runbook.py::test_helloworld_workflow_has_six_steps PASSED
scripts/tests/test_dr_drill_runbook.py::test_helloworld_assertions_present PASSED
scripts/tests/test_dr_drill_runbook.py::test_helloworld_max_latency_60s PASSED
scripts/tests/test_dr_drill_runbook.py::test_runbook_doc_mentions_quarterly_drill_section PASSED
scripts/tests/test_dr_drill_runbook.py::test_runbook_doc_references_run_script PASSED
scripts/tests/test_dr_drill_runbook.py::test_compliance_doc_exists PASSED
scripts/tests/test_dr_drill_runbook.py::test_compliance_doc_lists_frameworks PASSED
scripts/tests/test_dr_drill_runbook.py::test_compliance_evidence_schema_versioned PASSED
scripts/tests/test_dr_drill_runbook.py::test_compliance_evidence_object_lock_compliance PASSED
scripts/tests/test_dr_drill_runbook.py::test_compliance_evidence_retention_7y PASSED
scripts/tests/test_dr_drill_runbook.py::test_values_yaml_drill_quarterly_section PASSED
scripts/tests/test_dr_drill_runbook.py::test_values_yaml_helloworld_disabled_mock PASSED
scripts/tests/test_dr_drill_runbook.py::test_evidence_readme_exists PASSED
scripts/tests/test_dr_drill_runbook.py::test_changelog_mentions_d40 PASSED
scripts/tests/test_dr_drill_runbook.py::test_helloworld_schema_draft07_valid PASSED

============================== 30 passed in 0.05s ==============================
```

---

## §21. Annexe — RACI + budget

### §21.1 RACI

| Activité | R | A | C | I |
|---|---|---|---|---|
| Coder `dr/run_quarterly_drill.sh` | Eng SRE | Tech Lead | Compliance Officer | Eng VP |
| Helm CronJob YAML | Eng SRE | Tech Lead | DevOps Eng | Eng VP |
| Pin test Python | Eng SRE | Tech Lead | QA Eng | Eng VP |
| Doc backup-and-restore.md ext | Eng SRE | Tech Lead | DPO | Eng VP |
| Compliance doc | Eng SRE | DPO | Compliance Officer | Eng VP |
| Workshop ops | Eng SRE | Tech Lead | Ops team | Eng VP |
| Validation finale | DPO | Eng VP | External Auditor | C-suite |

### §21.2 Budget

| Item | Cost / Q (4 drills/an) | Annual | Notes |
|---|---:|---:|---|
| Drill compute (kind on staging) | $20 | $80 | 8 vCPU × 30 min |
| Compliance evidence S3 | $5 | $20 | Object Lock mode |
| Auditor review (annual) | — | $5 000 | external SOC 2 audit |
| Eng time D40 | $4 800 | — | 4 j × $1 200 day rate (one-shot) |
| Eng on-call quarterly review | $200 | $800 | 1 h ops review per Q |
| **Total annual** | — | **$5 900** | year 1 inc. $4 800 build |

ROI : 1 drill quarterly fail détecté =
**$50 000-$500 000 saved** (RGPD breach 4% global revenue cap).

---

## §22. Annexe — Gantt prévisionnel

```text
J1    J2    J3    J4
█████ ████              run_quarterly_drill.sh + Helm CronJob
       █████████        Manual e2e + debug
              █████     pin tests + doc compliance
                   ██   CHANGELOG + workshop + tag
```

* **Total : 4 jours-ingénieur** (1 sprint).
* **Lead time** : peut démarrer dès que efe-server image stable
  (post-D38 GA M6).
* **Dépendances bloquantes externes** :
  * Image `efe-server` doit contenir le binaire
    `run_quarterly_drill.sh` (modifier `Dockerfile` pour copy).
  * S3 bucket `efe-compliance-evidence` provisioned avec Object
    Lock mode COMPLIANCE.

---

## §23. Annexe — DR drill outcome JSON v2 (extended)

### §23.1 Schema versioned

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$id": "https://efe.ai/schemas/dr-drill-outcome-2.0.0.json",
  "title": "EFE DR drill outcome (D40 extended)",
  "type": "object",
  "required": [
    "schema_version",
    "drill_kind",
    "drill_label",
    "started_at",
    "finished_at",
    "duration_secs",
    "rto_target_secs",
    "within_rto",
    "verifiers",
    "success_overall"
  ],
  "properties": {
    "schema_version": { "type": "string", "const": "2.0.0" },
    "drill_kind": { "type": "string", "enum": ["weekly", "quarterly"] },
    "drill_label": { "type": "string", "minLength": 1 },
    "started_at": { "type": "string", "format": "date-time" },
    "finished_at": { "type": "string", "format": "date-time" },
    "duration_secs": { "type": "integer", "minimum": 0 },
    "rto_target_secs": { "type": "integer", "minimum": 0 },
    "within_rto": { "type": "boolean" },
    "phases": {
      "type": "object",
      "description": "Per-phase wall-clock breakdown (only quarterly).",
      "properties": {
        "kind_setup_secs": { "type": "integer" },
        "helm_install_secs": { "type": "integer" },
        "restore_secs": { "type": "integer" },
        "verifier_suite_secs": { "type": "integer" },
        "helloworld_secs": { "type": "integer" },
        "cleanup_secs": { "type": "integer" }
      }
    },
    "verifiers": {
      "type": "array",
      "minItems": 5,
      "items": {
        "type": "object",
        "required": ["kind", "ok", "detail"],
        "properties": {
          "kind": {
            "type": "string",
            "enum": [
              "audit_chain",
              "bundle_signatures",
              "oidc_issuers",
              "catalog_smoke",
              "execute_smoke"
            ]
          },
          "ok": { "type": "boolean" },
          "detail": { "type": "string" }
        }
      }
    },
    "helloworld": {
      "type": "object",
      "description": "Quarterly drill only (D40).",
      "$ref": "#/$defs/helloWorldOutcome"
    },
    "compliance_frameworks": {
      "type": "array",
      "items": {
        "type": "string",
        "enum": ["SOC2-CC9.1", "ISO27001-A.17.1.3", "NIS2-Art21", "RGPD-Art32"]
      }
    },
    "attestation_signature": { "type": "string", "minLength": 1 },
    "attestation_pubkey_id": { "type": "string", "minLength": 1 },
    "success_overall": { "type": "boolean" }
  },
  "$defs": {
    "helloWorldOutcome": {
      "type": "object",
      "$ref": "https://efe.ai/schemas/helloworld-1.0.0.json"
    }
  }
}
```

### §23.2 Backwards compat v1 (D17) → v2 (D40)

* Champs ajoutés : `drill_kind`, `phases`, `helloworld`,
  `compliance_frameworks`, `attestation_signature`,
  `attestation_pubkey_id`.
* Champs renommés : aucun (anti-breaking).
* Champs supprimés : aucun.
* **Lecteurs v1 ignorent les champs v2** ; lecteurs v2 lisant un
  fichier v1 traitent `drill_kind = "weekly"` (default) +
  `helloworld = null`.

---

## §24. Mises à jour du document

### §24.1 Versioning

* v1.0.0 — 2026-04-29 — Première édition (cette version).

### §24.2 Liens à maintenir

* `docs/bench/d17_backup_dr_architecture.md` — section 16
  (DR drill weekly). D40 ajoute la section 17 (DR drill
  quarterly).
* `docs/audits/d17_backup_dr_audit_report.md` — référence l'audit
  qui a livré les 10 fixes pré-D40.
* `docs/docs/howto/backup-and-restore.md` — section "Quarterly DR
  drill" (NEW D40).
* `docs/EFE_BENCH_EXTENSION_ARCHITECTURE.md` — pattern
  extension single-sprint pour M6 (référence structurelle).

### §24.3 Critères d'archive

* Le sprint D40 a shippé.
* Le `CHANGELOG.md` `[1.0.0]` entry est livré.
* Le pin test `test_dr_drill_runbook.py` est GREEN.
* La première compliance evidence Q est archivée S3.
* L'audit pass M7 (D41 candidate) est terminé.
* Le tag `efe-backup@1.0.0` est créé.

---

**Fin du document.**

Total LOC du document : ~1 530 lignes Markdown.
Total LOC delta cible (M7 extension) : ~700 LOC actifs (script +
YAML + pin test) + ~350 LOC documentation. Total **~1 050 LOC**.
Total durée : **~4 jours-ingénieur** sur 1 sprint atomique
(Φ-F).
