# Sprint D17 — Backup + Disaster Recovery (architecture)

> **Status** : in-progress, v0.1.18 (post-D13-bis OTel SDK real).
> **Versionnement BENCHMARKS.md** : `0.1.18` (sprint D17 = première
> livraison Backup + DR de bout-en-bout).
> **Pré-requis** : D14 (Postgres catalog backend) en place,
> D14-bis (HMAC-chained audit log) actif, D15 (OIDC) opérationnel,
> D16 (Embedder bundle Ed25519-signed) déployé.
> **Pattern référence** : audits D8 (8 bugs, 2 critiques),
> D12-bis (7 bugs, 2 critiques), D14 (7 bugs, 1 critique),
> D14-bis (9 bugs, 3 critiques), D15 (17 bugs, 8 critiques),
> D16 (17 bugs, 5 critiques), D13-bis (13 bugs, 2 critiques).

---

## 0. TL;DR

EFE persiste maintenant des données dont la perte est **catastrophique**
sur les axes business, légal et opérationnel :

* **Catalog Postgres** (D14) — toute la connaissance fonctionnelle
  d'un tenant (catalogues d'embeddings nommés, versions, mappings
  classes ↔ phrases). Sans, le tenant ne peut plus inférer.
* **Audit log HMAC-chained** (D14-bis) — exigence réglementaire
  (RGPD, audit interne, due diligence client). Une chaîne brisée
  = obligation de notification à l'autorité de protection des
  données dans les 72 h.
* **Embedder bundles** (D16) — artefacts ML signés Ed25519 ;
  re-télécharger HuggingFace post-incident est lent (~20 min/bundle)
  et casse les SLO d'inférence ; plus catastrophique encore si la
  source HF est inaccessible (offline cluster, embargo, déprécation
  du modèle upstream).
* **OIDC issuer registry** (D15) — sans, plus aucune
  authentification possible ; tous les tenants en panne.
* **Configuration Helm + secrets K8s** — le plan de redéploiement
  tout entier.

D17 livre :

1. Un crate **`efe-backup`** qui orchestre les snapshots des 5
   types de données critiques (Postgres + audit keys + bundles +
   OIDC config + secrets K8s) avec **vérification d'intégrité**
   post-snapshot ET post-restore.
2. Un **CronJob Helm** déclenchant des base backups Postgres
   toutes les 6 h + un push WAL continu (PITR fenêtre 7 j).
3. Un **CLI** `efe-backup` pour les opérations manuelles
   (`snapshot`, `restore`, `verify`, `drill`).
4. Un **runbook DR** documenté + scripté (un seul `./dr/restore.sh`
   pour reproduire un cluster from-scratch en moins de 30 min).
5. Une **DR drill automatique** hebdomadaire qui restore un cluster
   éphémère et vérifie l'intégrité, fail-closing si anomalie.
6. Des **objectifs SLO mesurables** (RTO 30 min, RPO 5 min)
   instrumentés en métriques.
7. Une **threat model** explicite couvrant ransomware, insider DBA,
   region-wide outage, audit chain divergence, supply-chain attack
   sur l'outil de backup lui-même.

**Non-objectifs** : multi-region active-active (D18+ candidate),
backup des logs Loki (transient), backup des métriques Prometheus
(transient — l'observabilité repart en boot).

---

## 1. Pourquoi ce sprint maintenant

### 1.1 Pain point opérationnel

Au tick du sprint D17, l'inventaire des données critiques EFE est :

| Source | Sprint | Volume attendu (1k tenants) | Perte = ? |
|--------|--------|----------------------------:|-----------|
| Postgres `catalog_*` tables | D14 | ~3 GB | **Inférence morte** |
| Postgres `audit_log` table (HMAC-chained) | D14-bis | ~50 GB / an / cluster | **RGPD breach** + audit trail brisée |
| HMAC keys (`/etc/efe/hmac_secrets.json`) | D14-bis | ~1 KB | **Audit chain unverifiable** |
| Embedder bundles (S3-compatible) | D16 | ~5 GB / bundle × N tenants | **Inférence dégradée** (re-pull HF lent) |
| Ed25519 trust store (`/etc/efe/embedder_trust.json`) | D16 | ~1 KB | **Plus de signature verifiable** |
| OIDC issuers file (`/etc/efe/oidc_issuers.json`) | D15 | ~10 KB | **Plus d'auth tenant** |
| K8s Helm values + Secrets | tous | ~100 KB | **Plus de redéploiement** |

Pré-D17 : **aucune** stratégie de backup formalisée. Un opérateur
qui faisait `kubectl delete namespace efe` perdait tout. Un DBA
malveillant avec accès Postgres pouvait `DROP DATABASE` sans
recourse. Un incident région-wide AWS chez le hosting cluster =
**données définitivement perdues**.

### 1.2 Pain point conformité

EFE est commercialisé sur des contrats B2B avec des clauses de :

* **DPA RGPD** : "Subprocessor garantit la disponibilité des données
  personnelles avec un RPO maximal de 1 h et un RTO maximal de
  4 h."
* **Audit interne** : "Le contrat de service exige que l'audit log
  soit conservé 5 ans avec preuve d'intégrité (HMAC chain)."
* **Due diligence client (Tier 1)** : "Le prestataire doit fournir
  un runbook DR documenté + résultats des derniers DR drills."

Pré-D17 : aucune réponse légalement défendable. D17 = condition
de signature de tout contrat Tier 1.

### 1.3 Pain point business

Le sprint D8 a livré le decoder MVP. Les sprints D9–D16 ont
construit la stack inférence + observability + auth + storage. Le
**produit fonctionne**, mais la **survivabilité** n'est pas
adressée. Premier incident = perte de tout l'historique commercial,
toutes les configurations tenant, tout le travail d'onboarding. Le
business serait **mort** en quelques heures.

D17 transforme EFE d'un **prototype démontrable** en un **système
de production**.

---

## 2. Threat model

12 menaces identifiées, classées par sévérité × probabilité.

### 2.1 Liste exhaustive

| ID | Menace | Vecteur | Sévérité | Probabilité | Mitigation D17 |
|----|--------|---------|:--------:|:-----------:|----------------|
| **T-DR-1** | Ransomware encrypt-in-place | Compromis cluster K8s + accès volume PVC | 🔴 CRIT | 🟠 MED | Backups offsite + immutable storage (object-lock S3) |
| **T-DR-2** | Insider DBA malveillant | DBA fait `DROP DATABASE` puis supprime backups | 🔴 CRIT | 🟡 LOW | Backup write-once + 4-eyes pour delete + air-gapped weekly archive |
| **T-DR-3** | Region-wide outage | AWS us-east-1 down 6 h+ | 🟠 HARD | 🟡 LOW | Cross-region backup replication |
| **T-DR-4** | Audit chain divergence post-restore | HMAC keys perdues ou chain start mismatch | 🟠 HARD | 🟠 MED | Backup HMAC keys séparément + verify post-restore + re-anchor procedure |
| **T-DR-5** | Bundle Ed25519 keys lost | Trust store + bundles backup décorrélés | 🟠 HARD | 🟠 MED | Trust store backup atomique avec bundles ; verify signatures post-restore |
| **T-DR-6** | Tenant data leak via cross-env restore | Restore prod backup vers staging visible aux devs | 🔴 CRIT | 🟠 MED | Backup tagged with `env=prod` ; restore CLI refuses cross-env without `--force-cross-env` flag |
| **T-DR-7** | OIDC issuers drift post-restore | Restore old issuers → new tenants can't auth | 🟡 MED | 🟠 MED | OIDC config snapshotté avec timestamp ; reconciliation post-restore |
| **T-DR-8** | PITR clock skew | Restore to "2026-04-28 14:00" diverges across replicas | 🟡 MED | 🟡 LOW | UTC-only ; NTP-sync verified ; PITR target = LSN preferred over wall-clock |
| **T-DR-9** | Silent backup corruption | pg_basebackup completes but file truncated | 🔴 CRIT | 🟠 MED | SHA-256 manifest + restore-and-verify drill weekly |
| **T-DR-10** | RGPD right-to-erasure not applied to backups | Tenant erase request doesn't propagate to historical backups | 🟠 HARD | 🟠 MED | Erasure-tracking table ; backup-aware retention policy ; documented "we cannot erase old backups but they expire in N days" |
| **T-DR-11** | Supply-chain attack on backup tool | pgBackRest binary compromised | 🟠 HARD | 🟡 LOW | Pin pgBackRest version + SHA + reproduce from source ; SBOM tracked |
| **T-DR-12** | Backup-storage credential compromise | S3 bucket creds leak → attacker reads backups | 🔴 CRIT | 🟠 MED | KMS encrypt-at-rest with customer-managed keys + IAM role assumption + access logs |

### 2.2 Matrix de mitigation

```
                         Sévérité
                  CRIT       HARD       MED        LOW
              ┌──────────┬──────────┬──────────┬──────────┐
   HIGH      │          │          │          │          │
   probab    │   T-DR-1 │          │          │          │
              ├──────────┼──────────┼──────────┼──────────┤
   MED       │  T-DR-6  │  T-DR-4  │  T-DR-7  │          │
   probab    │  T-DR-9  │  T-DR-5  │          │          │
              │ T-DR-12  │ T-DR-10  │          │          │
              ├──────────┼──────────┼──────────┼──────────┤
   LOW       │          │  T-DR-3  │  T-DR-8  │          │
   probab    │  T-DR-2  │ T-DR-11  │          │          │
              └──────────┴──────────┴──────────┴──────────┘
```

**Priorité de fix** : T-DR-1, T-DR-6, T-DR-9, T-DR-12 (CRIT × MED+).
T-DR-2 (CRIT × LOW) bénéficie d'une mitigation 4-eyes mais reste
un risque résiduel acceptable. T-DR-10 et T-DR-11 sont mitigés
par documentation + SBOM tracking.

---

## 3. Requirements

### 3.1 Fonctionnels

* **R-DR-1** — Tout backup doit être **chiffré at-rest** avec une
  clé customer-managed (KMS).
* **R-DR-2** — Tout backup doit être **chiffré in-transit** (TLS 1.3
  obligatoire vers le storage S3-compatible).
* **R-DR-3** — Tout backup doit produire un **manifeste signé** :
  liste de fichiers + SHA-256 par fichier + signature Ed25519 du
  manifeste (réutilise la trust store D16).
* **R-DR-4** — La **fréquence de backup** est configurable mais
  doit respecter le RPO contractuel (par défaut 5 min via WAL
  continuous).
* **R-DR-5** — Le **restore** doit être **scriptable et idempotent** :
  un seul `./dr/restore.sh <backup-id> --target <env>` pour
  reproduire un cluster fonctionnel.
* **R-DR-6** — Un **DR drill hebdomadaire** automatique restaure
  dans un namespace éphémère, exécute une suite de smoke tests,
  puis détruit le namespace.
* **R-DR-7** — Le **monitoring** doit alerter si :
  * dernier backup réussi > 24 h (CRIT)
  * dernier DR drill réussi > 14 j (CRIT)
  * différence entre RPO mesuré et RPO objectif > 10 % (WARN)
* **R-DR-8** — Le **CLI `efe-backup`** doit supporter au minimum :
  `snapshot`, `restore`, `verify`, `drill`, `list`, `delete`.
* **R-DR-9** — Le **logging** des opérations backup/restore doit
  être chaîné dans l'audit log HMAC (D14-bis).
* **R-DR-10** — Toute opération destructive (`delete`) doit
  exiger **two-person authorisation** via deux JWT distincts
  (4-eyes principle).

### 3.2 Non-fonctionnels

* **N-DR-1 (RTO)** — Recovery Time Objective : **30 minutes** end-to-end
  (depuis la décision "on bascule" jusqu'à `efe-server` répondant
  des `Execute` RPCs avec succès dans le nouveau cluster).
* **N-DR-2 (RPO)** — Recovery Point Objective : **5 minutes**
  (perte de données acceptable maximale).
* **N-DR-3** — Backup latency : un base-backup d'un cluster 100 GB
  doit completer en **< 30 min**.
* **N-DR-4** — Restore latency : un restore from-scratch d'un
  backup 100 GB doit completer en **< 20 min**.
* **N-DR-5** — Storage cost : retention overhead **< 3×** le size
  de la prod base (compression + dédup).
* **N-DR-6** — Backup process **must not impact production
  performance** : p99 `Execute` latency delta ≤ 2 % pendant le
  base-backup.

### 3.3 Compliance

* **C-DR-1** — RGPD Art. 32 : "Le responsable du traitement et le
  sous-traitant mettent en œuvre les mesures techniques et
  organisationnelles appropriées..." → backups chiffrés + RTO/RPO
  documentés.
* **C-DR-2** — RGPD Art. 17 : "right to erasure" → backups expirent
  au rythme de la retention policy, et l'opérateur peut documenter
  "votre donnée sera effacée au plus tard à la date X".
* **C-DR-3** — Loi française "obligation de continuité de service" :
  PRA/PCA documenté.
* **C-DR-4** — SOC 2 Type II (futur) : audit trail des opérations
  backup/restore pendant 12 mois minimum.

---

## 4. Inventaire des données critiques (par tier criticality)

### 4.1 Tier 0 — Catastrophique si perdu

| Donnée | Source | Backup strategy | Verify post-restore |
|--------|--------|-----------------|---------------------|
| Postgres `catalog_*` tables | D14 | pgBackRest base + WAL | row-count diff vs metric snapshot |
| Postgres `audit_log` table | D14-bis | pgBackRest (incluse base) | HMAC chain re-verify end-to-end |
| HMAC signing keys | D14-bis | secret-store snapshot (Vault / SOPS) | re-sign known canary + verify |

### 4.2 Tier 1 — Très grave (mais reproductible avec effort)

| Donnée | Source | Backup strategy | Verify post-restore |
|--------|--------|-----------------|---------------------|
| Embedder bundles (Ed25519-signed) | D16 | rclone S3→S3 sync + manifest | Ed25519 signature verify (existing D16 path) |
| Trust store Ed25519 public keys | D16 | secret-store snapshot | empty trust store = bundles unloadable, alert |
| OIDC issuers JSON | D15 | secret-store snapshot | parse + JWKS reachability check |

### 4.3 Tier 2 — Grave (reproductible avec runbook)

| Donnée | Source | Backup strategy | Verify post-restore |
|--------|--------|-----------------|---------------------|
| Helm values + ConfigMaps | tous | git-stored (déjà) + K8s snapshot | `helm template` clean render |
| K8s Secrets non sensitive | tous | sealed-secrets git-stored | `kubectl apply --dry-run` |

### 4.4 Tier 3 — Transient (re-derivable)

* Logs Loki — out of scope D17. Ré-émis à partir de la nouvelle prod.
* Métriques Prometheus — out of scope. Repart en boot.
* JWKS cache — out of scope. Refetch automatique.

### 4.5 Volume estimé par tenant

```
Tier 0 :
  catalog tables :          ~3 MB / tenant   (1 catalog × ~10 versions × ~100 sentences)
  audit_log :              ~50 MB / tenant / an  (10 events/h × 24h × 365j × 0.5KB)
  HMAC keys :                ~50 B / tenant

Tier 1 :
  bundle :                ~5 GB / model variant (shared across tenants)
  trust store :              ~1 KB cluster-wide
  OIDC issuers :             ~10 KB cluster-wide

Total / 1k tenants ≈ 50 GB Postgres + 5 × N_models GB S3 = ~70 GB
backup size, ~210 GB retention (3× via N-DR-5).
```

---

## 5. Stratégies de backup (par data type)

### 5.1 Postgres : pgBackRest + WAL streaming

**Choix de l'outil** : `pgBackRest` (évalué vs WAL-G + Barman ;
voir § 17 alternatives). pgBackRest supporte :

* **Base backup** parallel (4–8 worker threads) pour réduire
  la durée d'un dump 100 GB de 90 min (pg_dump) à ~15 min.
* **WAL archiving** vers S3 toutes les 5 min (RPO N-DR-2).
* **Full / diff / incremental** policy : 1× full hebdomadaire,
  1× diff quotidienne, 5× incremental quotidiennes.
* **Encryption at rest** AES-256-GCM avec passphrase via env var
  injectée par K8s Secret.
* **Verification** intégrée via `pgbackrest check` à chaque archive.
* **Restore** avec target time / target LSN / target name.

**Configuration type** (`/etc/pgbackrest/pgbackrest.conf`) :

```ini
[global]
repo1-type=s3
repo1-s3-bucket=efe-backups-prod
repo1-s3-region=eu-west-1
repo1-s3-endpoint=s3.eu-west-1.amazonaws.com
repo1-s3-key-type=auto                      # IAM role assumption
repo1-cipher-type=aes-256-cbc
repo1-cipher-pass-environment=PGBACKREST_REPO1_CIPHER_PASS
repo1-retention-full=4                      # 4 weeks of full backups
repo1-retention-diff=14                     # 14 days of diff
repo1-retention-archive=14                  # 14 days WAL retention

archive-async=y
archive-push-queue-max=4GB
archive-timeout=60                           # WAL push timeout
process-max=4

[main]
pg1-path=/var/lib/postgresql/data
pg1-port=5432
pg1-user=pgbackrest
pg1-database=postgres
```

### 5.2 Audit log : intégré au backup Postgres

L'audit log (`audit_log` table D14-bis) est dans la base Postgres.
Il est backupé **automatiquement** par le base backup pgBackRest.

**Spécificité D14-bis** : le HMAC chain implique un ordre strict
des rows (parent_hmac → child_hmac). Le backup row-by-row de
pg_basebackup respecte cet ordre. **Le restore re-vérifie la chaîne
de bout-en-bout** dans le post-restore hook (cf. § 12).

### 5.3 HMAC keys : Vault (ou SOPS-encrypted git)

**Choix recommandé** : HashiCorp Vault avec snapshot encryption
key auto-managed. Alternative pour clusters sans Vault : SOPS
(Mozilla) avec age-encrypted YAML committed dans un repo git
privé.

**Backup strategy** :

* Vault : daily snapshot via `vault operator raft snapshot save`
  poussé vers S3 distinct (compartmentalization).
* SOPS : git-managed, le repo lui-même est backupé par GitHub /
  GitLab.

**Recovery** : restaurer Vault (ou cloner le repo SOPS) AVANT
de restaurer Postgres. L'audit chain post-restore exige les keys
pour re-vérifier la chaîne. Ordre critique : keys → DB → verify.

### 5.4 Embedder bundles : rclone S3→S3 + manifest

Les bundles (D16) sont déjà dans un S3-compatible store. Le backup
consiste en :

1. **rclone sync** vers un bucket distinct (S3 cross-region ou
   compartiment air-gapped).
2. **Manifest** SHA-256 par bundle + signature Ed25519 (réutilise
   la trust store D16).
3. **Verification post-sync** : `rclone check --size-only --differ` ;
   un échec déclenche une alert PagerDuty.

**Important** : la trust store (clés publiques Ed25519) est backupée
**séparément** pour éviter qu'un compromis du backup-bucket invalide
les signatures (T-DR-5 mitigation).

### 5.5 OIDC issuers : Vault snapshot

Backupé avec les HMAC keys (même Vault, même cadence). Format
JSON simple, ~10 KB, négligeable.

### 5.6 Helm values + ConfigMaps

* **Helm values.yaml** : git-stored, le backup est implicite.
* **ConfigMaps générés** : `velero` snapshots du namespace EFE
  toutes les 24 h (pour récupérer en moins d'une minute la
  configuration cluster).
* **Sealed Secrets** : git-stored, backup implicite.

### 5.7 Diagramme global

```
┌─────────────────────────────────────────────────────────────────┐
│                        Production cluster                       │
│                                                                 │
│  ┌─────────────┐  ┌──────────────┐  ┌──────────────┐           │
│  │  efe-server │  │  Postgres    │  │   Vault      │           │
│  │   pods      │  │  (catalog +  │  │  (HMAC keys, │           │
│  │             │◄─┤   audit_log) │  │   OIDC)      │           │
│  └─────────────┘  └──────┬───────┘  └──────┬───────┘           │
│                          │                  │                   │
│                          │ WAL streaming    │ daily snapshot    │
│                          │ + base backup    │                   │
│                          ▼                  ▼                   │
│                    ┌──────────────────────────────┐             │
│                    │  pgBackRest sidecar          │             │
│                    │  + Vault snapshot CronJob    │             │
│                    └──────────┬───────────────────┘             │
│                               │ TLS 1.3                         │
└───────────────────────────────┼─────────────────────────────────┘
                                │
                                ▼
                  ┌─────────────────────────┐
                  │   S3-compatible (KMS    │
                  │   encrypted, immutable) │
                  │                         │
                  │   /pg-basebackup/       │
                  │   /pg-wal/              │
                  │   /vault-snapshots/     │
                  │   /bundle-manifests/    │
                  │   /helm-values/         │
                  │                         │
                  │  Object-lock 30 d       │
                  │  Cross-region replic.   │
                  └─────────────────────────┘
                                ▲
                                │ rclone sync
                  ┌─────────────┴───────────┐
                  │  Bundles bucket (D16)   │
                  └─────────────────────────┘
```

---

## 6. Pipeline backup détaillée

### 6.1 Phases d'un backup base Postgres

```
┌────────────────────────┐
│ T0 : kick CronJob      │
└────────┬───────────────┘
         │
         ▼
┌────────────────────────────────────┐
│ T1 : pré-checks                   │
│  - PG primary detected            │
│  - replication slots OK           │
│  - free disk space ≥ 2× DB size   │
│  - last backup status             │
└────────┬───────────────────────────┘
         │
         ▼
┌────────────────────────────────────┐
│ T2 : pgBackRest backup --type=full │
│   - parallel I/O (4 workers)      │
│   - encryption AES-256-GCM        │
│   - per-file SHA-256 manifest     │
└────────┬───────────────────────────┘
         │
         ▼
┌────────────────────────────────────┐
│ T3 : sign manifest                 │
│   - Ed25519 sign via trust store  │
│   - upload manifest.sig           │
└────────┬───────────────────────────┘
         │
         ▼
┌────────────────────────────────────┐
│ T4 : record metadata in DB         │
│   - INSERT INTO efe_backup_history │
│   - audit_log emit                │
└────────┬───────────────────────────┘
         │
         ▼
┌────────────────────────────────────┐
│ T5 : metric emit                   │
│   - efe_backup_duration_seconds   │
│   - efe_backup_size_bytes         │
│   - efe_backup_last_success_ts    │
└────────────────────────────────────┘
```

### 6.2 Schémas SQL ajoutés

Une nouvelle table `efe_backup_history` dans le schéma `public` :

```sql
CREATE TABLE IF NOT EXISTS efe_backup_history (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    backup_label    TEXT        NOT NULL,           -- "20260428T060000Z-full"
    backup_type     TEXT        NOT NULL CHECK (backup_type IN ('full','diff','incr')),
    started_at      TIMESTAMPTZ NOT NULL,
    finished_at     TIMESTAMPTZ NOT NULL,
    status          TEXT        NOT NULL CHECK (status IN ('ok','failed','partial')),
    size_bytes      BIGINT      NOT NULL,
    files_count     INTEGER     NOT NULL,
    sha256_manifest TEXT        NOT NULL,           -- hex SHA-256 of the manifest file itself
    ed25519_sig     TEXT        NOT NULL,           -- base64 detached signature
    repo_path       TEXT        NOT NULL,           -- "s3://efe-backups-prod/main/full/20260428T060000Z"
    pg_lsn_start    TEXT        NOT NULL,           -- "0/1A000028"
    pg_lsn_stop     TEXT        NOT NULL,
    error_message   TEXT        NULL,
    created_by      TEXT        NOT NULL DEFAULT 'cronjob',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_efe_backup_history_started_at
    ON efe_backup_history (started_at DESC);

CREATE INDEX idx_efe_backup_history_status
    ON efe_backup_history (status, started_at DESC);
```

### 6.3 Migration `0006_d17_backup_history.sql`

```sql
-- Sprint D17 — Backup + DR
--
-- Ajoute la table efe_backup_history pour tracer toutes les
-- opérations backup. Lue par le CLI `efe-backup list` et par
-- les alertes `D17BackupStaleAlert`.

BEGIN;

CREATE TABLE IF NOT EXISTS efe_backup_history ( /* ... cf. § 6.2 ... */ );
CREATE INDEX IF NOT EXISTS idx_efe_backup_history_started_at
    ON efe_backup_history (started_at DESC);
CREATE INDEX IF NOT EXISTS idx_efe_backup_history_status
    ON efe_backup_history (status, started_at DESC);

-- Marker row pour le test de migration (D14 pattern).
INSERT INTO efe_migration_marker (sprint, applied_at)
VALUES ('D17', NOW())
ON CONFLICT DO NOTHING;

COMMIT;
```

### 6.4 Cardinality des tables backup

`efe_backup_history` croît à raison de :

* 1 row × 6 h × 30 j = **120 rows/mois** pour les bases.
* 1 row par WAL push toutes les 5 min × 30 j = **8 640 rows/mois**.

Total annuel ≈ 100k rows. Négligeable. La table est compressée
post-3-mois via `pg_partman` (out of scope D17 mais documenté).

---

## 7. Crate `efe-backup` design

### 7.1 Position dans l'arbre du workspace

```
crates/
├── efe-backup/                    # NEW
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── config.rs              # BackupConfig (env + file)
│   │   ├── snapshot.rs            # Snapshotter trait + impls
│   │   ├── postgres.rs            # PgBackRest wrapper
│   │   ├── vault.rs               # Vault snapshot wrapper
│   │   ├── bundles.rs             # rclone wrapper for S3 sync
│   │   ├── manifest.rs            # Ed25519 manifest sign/verify
│   │   ├── restore.rs             # Restore orchestration
│   │   ├── verify.rs              # Restore-verify suite
│   │   ├── drill.rs               # DR drill orchestrator
│   │   ├── audit.rs               # audit_log emission
│   │   ├── metrics.rs             # 8 D17 instruments
│   │   └── error.rs               # BackupError enum
│   └── tests/
│       └── d17_backup_integration.rs
├── efe-server/                    # existing
└── ...

bin/
└── efe-backup/                    # NEW CLI
    ├── Cargo.toml
    └── src/
        └── main.rs                # clap subcommand router
```

### 7.2 API publique du crate

```rust
//! Sprint D17 — Backup + DR orchestration.

pub mod config;
pub mod manifest;
pub mod metrics;

use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BackupError {
    #[error("pgBackRest failed: {0}")]
    PgBackRest(String),
    #[error("Vault snapshot failed: {0}")]
    Vault(String),
    #[error("rclone sync failed: {0}")]
    Rclone(String),
    #[error("manifest signing failed: {0}")]
    SignManifest(String),
    #[error("manifest verification failed: {0}")]
    VerifyManifest(String),
    #[error("audit log emission failed: {0}")]
    AuditLog(String),
    #[error("postgres connection failed: {0}")]
    Postgres(#[from] sqlx::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("4-eyes authorization required: {0}")]
    FourEyesRequired(String),
    #[error("cross-environment restore refused: {0}")]
    CrossEnvRefused(String),
    #[error("RPO violation: last successful backup {age_secs}s ago, RPO target {target_secs}s")]
    RpoViolation { age_secs: u64, target_secs: u64 },
}

#[async_trait::async_trait]
pub trait Snapshotter: Send + Sync {
    /// Symbolic name (e.g. "postgres", "vault", "bundles").
    fn name(&self) -> &'static str;

    /// Take a snapshot, returning the repo path + size + hash.
    async fn snapshot(&self, label: &str) -> Result<SnapshotMetadata, BackupError>;

    /// Verify the snapshot integrity at rest (no restore).
    async fn verify(&self, snapshot_id: &str) -> Result<(), BackupError>;
}

#[derive(Debug, Clone)]
pub struct SnapshotMetadata {
    pub label: String,
    pub repo_path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub ed25519_sig: String,  // base64
    pub pg_lsn_start: Option<String>,
    pub pg_lsn_stop: Option<String>,
}
```

### 7.3 Snapshotter implementations

#### 7.3.1 `PgBackRestSnapshotter`

```rust
pub struct PgBackRestSnapshotter {
    config_path: PathBuf,
    stanza: String,
    backup_type: BackupType,  // Full | Diff | Incr
    process_max: u32,
}

#[async_trait::async_trait]
impl Snapshotter for PgBackRestSnapshotter {
    fn name(&self) -> &'static str { "postgres" }

    async fn snapshot(&self, label: &str) -> Result<SnapshotMetadata, BackupError> {
        // 1. Pre-checks : PG reachable, free disk, no concurrent backup.
        self.preflight().await?;

        // 2. Run pgbackrest backup --stanza=<stanza> --type=<type>
        //    --label=<label> --process-max=<n>
        let out = tokio::process::Command::new("pgbackrest")
            .args(&[
                "--config", self.config_path.to_str().unwrap(),
                "--stanza", &self.stanza,
                "--type", self.backup_type.as_str(),
                "--label", label,
                "--process-max", &self.process_max.to_string(),
                "backup",
            ])
            .output()
            .await
            .map_err(|e| BackupError::PgBackRest(e.to_string()))?;

        if !out.status.success() {
            return Err(BackupError::PgBackRest(
                String::from_utf8_lossy(&out.stderr).to_string(),
            ));
        }

        // 3. Parse pgbackrest info JSON for LSN start/stop + manifest path.
        let info = self.fetch_backup_info(label).await?;

        // 4. Compute SHA-256 of the manifest file.
        let sha = self.compute_manifest_sha(&info.manifest_path).await?;

        // 5. Sign the manifest hash with Ed25519 (D16 trust store).
        let sig = manifest::sign(&sha)?;

        Ok(SnapshotMetadata {
            label: label.into(),
            repo_path: info.repo_path,
            size_bytes: info.size,
            sha256: sha,
            ed25519_sig: sig,
            pg_lsn_start: Some(info.lsn_start),
            pg_lsn_stop: Some(info.lsn_stop),
        })
    }

    async fn verify(&self, snapshot_id: &str) -> Result<(), BackupError> {
        // Run pgbackrest check + verify manifest signature.
        // Detailed in § 6.3.
        // ...
        Ok(())
    }
}
```

#### 7.3.2 `VaultSnapshotter`

```rust
pub struct VaultSnapshotter {
    addr: String,        // VAULT_ADDR
    token_path: PathBuf, // file containing VAULT_TOKEN
    s3_dest: String,     // s3://efe-backups-prod/vault-snapshots
}

#[async_trait::async_trait]
impl Snapshotter for VaultSnapshotter {
    fn name(&self) -> &'static str { "vault" }

    async fn snapshot(&self, label: &str) -> Result<SnapshotMetadata, BackupError> {
        // 1. Run `vault operator raft snapshot save /tmp/snap.<label>`.
        // 2. Compute SHA-256 of the snapshot file.
        // 3. Upload to S3 via aws-sdk-s3.
        // 4. Sign manifest.
        // 5. Delete local file.
        // ...
    }
    /* ... */
}
```

#### 7.3.3 `BundleSnapshotter`

```rust
pub struct BundleSnapshotter {
    source_bucket: String,    // bundles bucket
    dest_bucket: String,      // backup bucket
    bundle_paths: Vec<String>,
}

#[async_trait::async_trait]
impl Snapshotter for BundleSnapshotter {
    fn name(&self) -> &'static str { "bundles" }

    async fn snapshot(&self, label: &str) -> Result<SnapshotMetadata, BackupError> {
        // 1. rclone sync source → dest --immutable.
        // 2. rclone check post-sync.
        // 3. Manifest = list of dest paths + per-file SHA-256.
        // 4. Sign manifest.
        // ...
    }
    /* ... */
}
```

---

## 8. CLI `efe-backup` UX

### 8.1 Subcommands

```text
$ efe-backup --help
EFE backup + disaster recovery toolkit (Sprint D17).

Usage: efe-backup [OPTIONS] <COMMAND>

Commands:
  snapshot   Take a snapshot of one or all data sources.
  restore    Restore from a snapshot.
  verify     Verify the integrity of a snapshot at rest.
  drill      Run a DR drill (restore to ephemeral namespace + smoke test).
  list       List recent backups.
  delete     Delete a snapshot (requires 4-eyes approval).
  rotate     Trigger retention pruning.

Options:
  --config <PATH>       Path to BackupConfig YAML (default: $EFE_BACKUP_CONFIG).
  --vault-token <PATH>  Vault token file (default: ~/.vault-token).
  --json                Output machine-readable JSON.
  -v, --verbose         Verbose logging.
  -h, --help            Print help.
```

### 8.2 Examples

```bash
# Take a full backup of every source.
efe-backup snapshot --type=full --label="ad-hoc-$(date -u +%Y%m%dT%H%M%SZ)"

# Take a Postgres-only diff backup.
efe-backup snapshot --source=postgres --type=diff

# Restore to a target time (PITR), production environment.
efe-backup restore \
    --backup-id=20260428T060000Z-full \
    --target-time="2026-04-28 06:30:00 UTC" \
    --target-env=prod

# DR drill (restore to ephemeral namespace, run smoke tests, destroy).
efe-backup drill --target-env=drill-$(date +%Y%m%d)

# List recent backups (JSON for grafana annotation).
efe-backup list --since=7d --json | jq '.[] | {label, status, size_bytes}'

# Delete a snapshot (requires 4-eyes approval).
efe-backup delete --backup-id=20260420T060000Z-full \
    --primary-token=$JWT_OPS_LEAD \
    --secondary-token=$JWT_DBA_LEAD \
    --reason="GDPR erasure for tenant T-XYZ"
```

### 8.3 4-eyes flow détaillé

`efe-backup delete` exige deux tokens JWT distincts :

1. Le token primaire (`--primary-token`) doit avoir le claim
   `permission: ["backup:delete:request"]`.
2. Le token secondaire (`--secondary-token`) doit avoir
   `permission: ["backup:delete:approve"]` ET un `sub` différent
   du primaire (le binaire compare les `sub` claims).
3. Une raison textuelle (`--reason`) obligatoire ; loggée dans
   l'audit log (D14-bis chained).
4. Si une condition échoue, `BackupError::FourEyesRequired` est
   retournée et la suppression refusée.

---

## 9. Schedule (CronJob Helm)

### 9.1 Cadences

| Job | Cron | Type | Retention | Note |
|-----|------|------|-----------|------|
| `efe-backup-postgres-full` | `0 6 * * 0` | full | 4 weekly | Dimanche 06:00 UTC |
| `efe-backup-postgres-diff` | `0 6 * * 1-6` | diff | 14 daily | Lundi-samedi 06:00 UTC |
| `efe-backup-postgres-incr` | `0 0,12,18 * * *` | incr | 14 daily | Toutes les 6 h sauf 06:00 |
| `efe-backup-postgres-wal` | continuous | WAL | 14 daily | archive-async via PG `archive_command` |
| `efe-backup-vault` | `30 6 * * *` | snapshot | 30 daily | Tous les jours 06:30 UTC |
| `efe-backup-bundles` | `0 7 * * 0` | rclone-sync | 4 weekly | Dimanche 07:00 UTC |
| `efe-backup-helm` | `0 7 * * *` | velero | 14 daily | Tous les jours 07:00 UTC |
| `efe-dr-drill` | `0 3 * * 0` | drill | (ephemeral) | Dimanche 03:00 UTC |

### 9.2 Helm values type

```yaml
backup:
  enabled: true
  storage:
    type: s3
    bucket: efe-backups-prod
    region: eu-west-1
    encryption:
      type: kms-aws
      kmsKeyId: arn:aws:kms:eu-west-1:111122223333:key/abcd-1234
    objectLock:
      enabled: true
      modeDays: 30
  postgres:
    schedule:
      full: "0 6 * * 0"
      diff: "0 6 * * 1-6"
      incr: "0 0,12,18 * * *"
    walArchive:
      enabled: true
      flushTimeoutS: 60
    retention:
      fullWeeks: 4
      diffDays: 14
      walDays: 14
    parallel: 4
    cipherPassSecretRef:
      name: pgbackrest-cipher
      key: passphrase
  vault:
    schedule: "30 6 * * *"
    retentionDays: 30
    addr: https://vault.efe.svc.cluster.local:8200
    tokenSecretRef:
      name: vault-snapshot-token
      key: token
  bundles:
    schedule: "0 7 * * 0"
    sourceBucket: efe-bundles-prod
    destBucket: efe-bundle-backups-prod
    crossRegion: true
  drill:
    enabled: true
    schedule: "0 3 * * 0"
    namespace: efe-dr-drill
    smokeTestImage: ghcr.io/efe/dr-smoke-test:v0.1.18
    timeout: 30m
  alerts:
    rpoTargetSeconds: 300       # 5 min RPO
    rtoTargetSeconds: 1800      # 30 min RTO
    backupStaleSeconds: 86400   # 24 h
    drillStaleSeconds: 1209600  # 14 days
  audit:
    chainEvents: true           # emit to D14-bis chain
```

### 9.3 ConfigMap rendering D17

```yaml
# templates/configmap.yaml — D17 block
data:
  # --- Sprint D17 — Backup + DR ---
  EFE_BACKUP_ENABLED: {{ .Values.backup.enabled | quote }}
  {{- if .Values.backup.enabled }}
  EFE_BACKUP_STORAGE_TYPE: {{ .Values.backup.storage.type | quote }}
  EFE_BACKUP_STORAGE_BUCKET: {{ .Values.backup.storage.bucket | quote }}
  EFE_BACKUP_STORAGE_REGION: {{ .Values.backup.storage.region | quote }}
  EFE_BACKUP_RPO_TARGET_S: {{ .Values.backup.alerts.rpoTargetSeconds | quote }}
  EFE_BACKUP_RTO_TARGET_S: {{ .Values.backup.alerts.rtoTargetSeconds | quote }}
  EFE_BACKUP_VAULT_ADDR: {{ .Values.backup.vault.addr | quote }}
  EFE_BACKUP_BUNDLES_SOURCE: {{ .Values.backup.bundles.sourceBucket | quote }}
  EFE_BACKUP_BUNDLES_DEST: {{ .Values.backup.bundles.destBucket | quote }}
  EFE_BACKUP_AUDIT_CHAIN_EVENTS: {{ .Values.backup.audit.chainEvents | quote }}
  {{- end }}
```

---

## 10. Restoration runbook

### 10.1 Scénarios couverts

* **S-1** : perte d'un node Postgres → failover replica (out of
  scope D17, géré par `cloudnative-pg`).
* **S-2** : perte du cluster Postgres → restore base + WAL replay.
* **S-3** : perte du namespace EFE → restore Velero + restore Postgres.
* **S-4** : perte du cluster K8s → re-déploiement from-scratch +
  restore complet.
* **S-5** : perte région-wide → bascule vers région secondaire +
  restore depuis backup cross-region.

### 10.2 Runbook scripté

`./dr/restore.sh` (livré avec le sprint) :

```bash
#!/usr/bin/env bash
# Sprint D17 — DR restore script.
#
# Usage : ./restore.sh <backup-id> --target <env> [--target-time <iso8601>]
#
# Restaure dans l'ordre :
#   1. Vault (HMAC keys, OIDC, trust store).
#   2. Postgres (catalog + audit_log).
#   3. Bundles S3 (sync depuis backup bucket).
#   4. Helm release (avec values reconstruites).
#   5. Verify post-restore (HMAC chain, bundle signatures, OIDC reachability).
#   6. Smoke test (1 RPC Execute + 1 RPC ListCatalogs).
#
# Critères de succès (RTO 30 min) :
#   - étape 1 ≤ 2 min
#   - étape 2 ≤ 15 min (incluant WAL replay)
#   - étape 3 ≤ 8 min (parallel rclone)
#   - étape 4 ≤ 3 min
#   - étapes 5–6 ≤ 2 min

set -euo pipefail

BACKUP_ID="${1:-}"
TARGET_ENV=""
TARGET_TIME=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target) TARGET_ENV="$2"; shift 2 ;;
    --target-time) TARGET_TIME="$2"; shift 2 ;;
    *) shift ;;
  esac
done

if [[ -z "$BACKUP_ID" || -z "$TARGET_ENV" ]]; then
  echo "Usage: $0 <backup-id> --target <env> [--target-time <iso8601>]" >&2
  exit 1
fi

# 0. Pré-checks
echo "[$(date -u +%FT%TZ)] DR restore starting"
echo "  backup_id=$BACKUP_ID target=$TARGET_ENV target_time=${TARGET_TIME:-latest}"

efe-backup verify --backup-id="$BACKUP_ID" || {
  echo "FATAL: backup verification failed" >&2
  exit 2
}

# 1. Restore Vault
echo "[$(date -u +%FT%TZ)] Step 1/6 — restoring Vault..."
efe-backup restore --backup-id="$BACKUP_ID" --source=vault --target-env="$TARGET_ENV"

# 2. Restore Postgres
echo "[$(date -u +%FT%TZ)] Step 2/6 — restoring Postgres..."
efe-backup restore --backup-id="$BACKUP_ID" --source=postgres --target-env="$TARGET_ENV" \
    ${TARGET_TIME:+--target-time "$TARGET_TIME"}

# 3. Restore bundles
echo "[$(date -u +%FT%TZ)] Step 3/6 — restoring bundles..."
efe-backup restore --backup-id="$BACKUP_ID" --source=bundles --target-env="$TARGET_ENV"

# 4. Re-deploy Helm release
echo "[$(date -u +%FT%TZ)] Step 4/6 — Helm deploy..."
helm upgrade --install efe ./deploy/helm/efe \
    -f ./deploy/helm/efe/values-${TARGET_ENV}.yaml

# 5. Verify post-restore
echo "[$(date -u +%FT%TZ)] Step 5/6 — verify integrity..."
efe-backup verify --post-restore --target-env="$TARGET_ENV"

# 6. Smoke test
echo "[$(date -u +%FT%TZ)] Step 6/6 — smoke test..."
./dr/smoke_test.sh "$TARGET_ENV"

echo "[$(date -u +%FT%TZ)] DR restore complete"
```

### 10.3 Verifications post-restore

`efe-backup verify --post-restore` enchaîne :

1. **HMAC chain integrity** : appel à
   `efe-server admin verify-audit-chain --full` (réutilise le code
   D14-bis `periodic_self_verify`). Si la chaîne est brisée,
   panique avec un exit code distinct (8) qui bloque le passage à
   smoke-test.

2. **Bundle signatures** : `efe-bundle verify --all` (D16). Toute
   signature invalide = exit code 9.

3. **OIDC reachability** : pour chaque issuer, fetch le JWKS et
   vérifier qu'au moins une clé est récupérable. Exit code 10 si
   un issuer est unreachable.

4. **Catalog smoke** : `efe-server catalog list --tenant=<canary>`
   doit retourner les catalogues attendus. Exit code 11.

5. **Execute smoke** : 1 RPC `Execute` avec un payload canary,
   doit retourner une `Response` succès. Exit code 12.

### 10.4 Rollback en cas de smoke-test failure

Si `./dr/restore.sh` échoue à l'étape 5 ou 6 :

* Le script **NE DELETE PAS** le namespace existant.
* Il logge l'échec dans l'audit log (D14-bis chained) avec le
  motif d'échec.
* Il déclenche une alert PagerDuty `D17DrRestoreFailed`.
* L'opérateur reprend la main : soit corrige l'origine du problème
  et relance, soit bascule sur un backup plus ancien
  (`--backup-id=<previous>`).

---

## 11. Encryption (at-rest + in-transit)

### 11.1 At-rest

* **pgBackRest** : AES-256-CBC avec `repo1-cipher-pass` injecté
  via env var. La passphrase est un secret K8s (Sealed Secrets ou
  Vault).
* **S3** : couche supplémentaire AWS KMS server-side encryption
  avec une CMK (Customer-Managed Key). Compromission KMS-CMK
  ≠ compromission cipher-pass : double encryption layered.
* **Vault snapshot** : Vault encrypte automatiquement avant
  d'écrire vers S3 (Raft snapshot encryption built-in).
* **Bundles** : déjà chiffrés au niveau objet (S3 SSE-KMS dans
  D16) ; le backup-bucket utilise une CMK distincte de la prod
  pour compartmentalization.

### 11.2 In-transit

* **TLS 1.3** obligatoire vers tout endpoint S3 (option `repo1-s3-uri-style=path`
  + endpoint HTTPS).
* **Tonic gRPC** : déjà TLS via D11.
* **Vault** : HTTPS obligatoire.
* **rclone** : `--ca-cert` pin sur le CA cluster.

### 11.3 Key rotation

* **pgBackRest cipher-pass** : rotation manuelle annuelle, exige
  un re-backup full (les anciens backups restent déchiffrables
  avec l'ancienne pass tant qu'on ne perd pas l'ancienne pass).
* **KMS CMK** : auto-rotation AWS yearly. Les anciens backups
  restent déchiffrables (KMS gère le versioning des keys).
* **Vault root token** : rotation via `vault operator generate-root`
  trimestrielle.
* **Ed25519 trust store** : rotation suivant le pattern D16
  (clé courante + 1 ancienne pour grace period).

---

## 12. Audit chain integrity post-restore (D14-bis interaction)

### 12.1 Contrainte critique

L'audit log (D14-bis) est chaîné par HMAC : chaque row contient
le HMAC du row précédent. Une chaîne rompue = preuve d'altération
ou de perte. Le restore Postgres restaure les rows tels quels —
mais si les **HMAC keys** ne sont pas restaurées avant, ou si elles
diffèrent, la chaîne ne peut plus être vérifiée et l'auditor signale
"chaîne brisée" alors qu'elle est en réalité intacte.

### 12.2 Procédure correcte

L'ordre du runbook (§ 10.2) est :

1. Vault (HMAC keys) **first**.
2. Postgres (audit_log) **second**.
3. Verify chain end-to-end **third** via `verify-audit-chain --full`.

Si l'étape 3 dit "chaîne brisée" :

* Cas 1 : keys correctes mais row corrompu → bug Postgres ou
  truncation backup. Action : rollback vers backup précédent.
* Cas 2 : keys incorrectes → bug restore Vault ou backup keys
  périmés. Action : restorer les keys du backup précédent et
  re-vérifier.

### 12.3 Re-anchor procedure (CONTINGENCY)

Si toutes les keys sont définitivement perdues mais la base est
restaurée :

1. **Document the loss** dans un audit-log immutable externe
   (papier, Time Machine, ou autre source régulièrement attestée).
2. **Re-anchor** : générer une nouvelle key, marquer un row
   `re_anchor=true` dans `audit_log` avec un message expliquant
   la rupture.
3. **À partir de ce row**, la chaîne reprend.
4. Tous les **rows antérieurs** restent dans la base mais sont
   **non-vérifiables** (l'auditor doit l'accepter en se référant au
   document externe).
5. Notifier l'auditor RGPD (autorité de protection des données)
   dans les 72 h conformément à l'Art. 33.

Cette procédure est **dernier recours**. La vraie mitigation est
**que le backup Vault soit toujours antérieur au backup Postgres**
(ce qui est trivialement vrai si le runbook restaure dans l'ordre).

### 12.4 Test automatisé

`tests/dr/test_audit_chain_post_restore.rs` :

* Setup : populate prod-DB avec 1k audit rows + HMAC chain valide.
* Backup full + Vault snapshot.
* Wipe DB + Vault.
* Restore (Vault first, then DB).
* Run `verify-audit-chain --full`.
* Assert : exit 0, "chain intact, 1000 rows verified".

---

## 13. Bundle re-verification post-restore

### 13.1 Procédure

Après `efe-backup restore --source=bundles` :

1. Pour chaque bundle restauré :
   * Compute le SHA-256 du fichier.
   * Compare avec le SHA-256 du manifeste (signé Ed25519 D16).
   * Vérifie la signature détachée du bundle (`*.sig`).
2. Toute différence = exit code 9, l'opérateur investigue avant de
   passer en prod.

### 13.2 Test automatisé

`tests/dr/test_bundle_post_restore.rs` :

* Setup : 5 bundles signés Ed25519 dans le S3 source.
* Snapshot + verify.
* Wipe S3.
* Restore.
* Run `efe-bundle verify --all` (D16 path).
* Assert : 5/5 bundles validés.

---

## 14. Tenant isolation pendant restore

### 14.1 Risque

T-DR-6 : un opérateur restaure un backup prod dans un namespace
**staging** (par accident ou pour tester). Les données prod
deviennent visibles aux développeurs staging — qui n'ont **PAS**
le droit GDPR de voir ces données. Fuite légale grave.

### 14.2 Mitigation

* Chaque backup contient un **tag d'environnement** dans le
  manifeste (`env=prod`, `env=staging`, …).
* `efe-backup restore --target-env=<env>` vérifie que
  `manifest.env == target-env`. Si différent, refuse avec
  `BackupError::CrossEnvRefused`.
* Override possible via `--force-cross-env` flag, mais :
  * loggé dans audit log (D14-bis chained) avec sévérité
    `RGPD_RISK`.
  * exige le 4-eyes principle (deux JWT distincts).
  * notifie le DPO automatiquement par email.

### 14.3 Tagging au snapshot

Le manifeste produit par `Snapshotter::snapshot` contient :

```json
{
  "label": "20260428T060000Z-full",
  "env": "prod",
  "cluster_id": "efe-prod-eu-west-1",
  "created_at": "2026-04-28T06:00:00Z",
  "components": [
    { "name": "postgres", "size_bytes": 51234567890, "sha256": "..." },
    { "name": "vault", "size_bytes": 4567, "sha256": "..." },
    { "name": "bundles", "size_bytes": 5234567890, "sha256": "..." }
  ],
  "ed25519_sig": "base64..."
}
```

Le champ `env` est **dans la signature** ; impossible de le
modifier sans invalider la signature.

---

## 15. RTO / RPO objectifs et mesure

### 15.1 Définitions strictes

* **RTO** (Recovery Time Objective) : temps maximal acceptable
  entre la décision "on bascule" et le moment où le nouveau cluster
  répond aux RPCs `Execute` avec succès.
* **RPO** (Recovery Point Objective) : perte de données maximale
  acceptable, mesurée en secondes entre la dernière donnée
  persistée par la prod et la dernière donnée présente dans le
  backup utilisé pour le restore.

### 15.2 Objectifs D17

* **RTO target** : 30 minutes (N-DR-1).
* **RPO target** : 5 minutes (N-DR-2).

### 15.3 Mesure RPO

À chaque WAL push :

```
efe_backup_rpo_seconds gauge
   = (now - timestamp_of_last_wal_pushed)
```

L'alerte `D17BackupRpoViolation` fire sur
`efe_backup_rpo_seconds > 300` (5 min) maintenue 5 min.

### 15.4 Mesure RTO (DR drill)

Le DR drill mesure le delta `T_decision → T_smoke_test_pass` :

```
efe_dr_drill_duration_seconds histogram
   buckets = [60, 300, 600, 900, 1200, 1500, 1800, 2400, 3000]
```

L'alerte `D17DrRtoViolation` fire si le dernier drill p95 >
1 800 (30 min).

---

## 16. DR drills (plan + métriques)

### 16.1 Cadence

Hebdomadaire, dimanche 03:00 UTC, dans un namespace éphémère
`efe-dr-drill-<YYYYMMDD>`.

### 16.2 Pipeline drill

```
T0 : kick CronJob `efe-dr-drill`
  │
  ▼
T1 : pick latest successful backup_id
  │
  ▼
T2 : create namespace `efe-dr-drill-<date>`
  │
  ▼
T3 : helm install efe-dr-drill ./deploy/helm/efe -f drill-values.yaml
  │
  ▼
T4 : run ./dr/restore.sh <backup_id> --target=drill
  │
  ▼
T5 : run ./dr/smoke_test.sh drill
  │
  ▼
T6 : measure RTO (T1 → T5)
  │
  ▼
T7 : if pass → emit metric, destroy namespace
     if fail → emit metric, KEEP namespace for forensics, page ops
```

### 16.3 Métriques drill

* `efe_dr_drill_runs_total{result}` — `success` / `failure`.
* `efe_dr_drill_duration_seconds` — histogram.
* `efe_dr_drill_last_success_timestamp` — gauge.
* `efe_dr_drill_smoke_test_failures_total{step}` —
  `verify_chain` / `verify_bundles` / `oidc` / `catalog` / `execute`.

### 16.4 Alerte SLO drill

`D17DrDrillStale` : fire si
`time() - efe_dr_drill_last_success_timestamp > 14d`.

`D17DrRtoViolation` : fire si le dernier drill p95 > 1 800 s.

---

## 17. Failover régional (multi-region)

### 17.1 Architecture cible (D17 ne livre que les fondations)

```
┌──────────────────────┐                  ┌──────────────────────┐
│ Région primaire      │   replicate      │ Région secondaire    │
│ (eu-west-1)          │ ──────────────►  │ (eu-west-3)          │
│                      │                  │                      │
│  efe-server (active) │                  │  efe-server (cold)   │
│  Postgres (primary)  │  WAL streaming   │  Postgres (replica)  │
│  Vault (active)      │  S3 cross-region │  Vault (cold)        │
└──────────────────────┘                  └──────────────────────┘
         │                                          │
         └─────────────► S3 ◄─────────────────────  │
              (cross-region buckets, KMS replicated)
```

### 17.2 D17 livre

* Backup S3 cross-region replicated (option `--cross-region` sur
  le bucket).
* WAL archiving accessible depuis les deux régions.
* Helm chart paramétrable par région (`--set region=eu-west-3`).
* Runbook DR exécutable depuis la région secondaire (variables
  `EFE_REGION`, `EFE_S3_BACKUP_BUCKET_REGION`).

### 17.3 D17 NE livre PAS

* **Active-active** : out of scope, complexité multi-master Postgres
  hors budget D17.
* **Auto-failover DNS** : géré par l'infra réseau (Route 53 health
  checks, out of scope).
* **Réplication continue Vault** : Vault Enterprise feature, hors
  scope OSS.

---

## 18. Integration avec sprints existants

### 18.1 D14 (Postgres catalog)

* `efe_backup_history` table créée dans le même schéma `public`
  que les tables D14.
* `efe-backup` réutilise le `PgPool` de D14 pour les opérations
  metadata.
* La migration `0006_d17_backup_history.sql` suit le pattern
  des migrations D14 (`migrations/` dir, sqlx-cli applied).

### 18.2 D14-bis (HMAC audit log)

* Toute opération `efe-backup snapshot/restore/delete` émet un
  event audit dans le HMAC chain :
  * `event_type = "backup.snapshot"` / `"backup.restore"` / `"backup.delete"`.
  * `payload = { backup_id, source, status, ... }`.
* Le restore re-vérifie la chaîne (§ 12).

### 18.3 D15 (OIDC)

* Le CLI `efe-backup` s'authentifie via JWT issued par les
  issuers OIDC enregistrés (réutilise le path D15 auth).
* 4-eyes pour `delete` exige deux JWT de `sub` distincts.

### 18.4 D16 (Embedder bundle)

* Les bundles sont backupés via `BundleSnapshotter`.
* Le manifest backup est signé Ed25519 avec la même trust store
  que les bundles eux-mêmes.

### 18.5 D13-bis (OTel)

* `efe-backup` émet des spans OTel (sampler error_boost forcé sur
  les erreurs).
* Trace context propagé : `backup.snapshot.start` →
  `backup.snapshot.complete` avec child spans
  `pg_backup_main`, `vault_snapshot`, `bundle_sync`,
  `manifest_sign`.

---

## 19. Métriques Prometheus

### 19.1 Inventaire (8 instruments D17)

| # | Métrique | Type | Labels | Cardinalité |
|---|----------|------|--------|-------------|
| 1 | `efe_backup_runs_total` | Counter | `source`, `type`, `result` | 3×3×3=27 |
| 2 | `efe_backup_duration_seconds` | Histogram | `source`, `type` | 3×3=9 buckets×9 |
| 3 | `efe_backup_size_bytes` | Histogram | `source`, `type` | 9 |
| 4 | `efe_backup_last_success_timestamp` | Gauge | `source`, `type` | 9 |
| 5 | `efe_backup_rpo_seconds` | Gauge | (no label) | 1 |
| 6 | `efe_dr_drill_runs_total` | Counter | `result` | 2 |
| 7 | `efe_dr_drill_duration_seconds` | Histogram | (no label) | 9 |
| 8 | `efe_dr_drill_last_success_timestamp` | Gauge | (no label) | 1 |

**Cardinalité total D17** ≈ 27 + 9 + 9 + 9 + 1 + 2 + 9 + 1 = **67 séries**.
Très en-dessous du budget D13 (50k).

### 19.2 Buckets histograms

* `efe_backup_duration_seconds` : `[60, 300, 900, 1800, 3600, 7200, 14400]` (1 min à 4 h).
* `efe_backup_size_bytes` : `[1e6, 1e7, 1e8, 1e9, 1e10, 5e10, 1e11]` (1 MB à 100 GB).
* `efe_dr_drill_duration_seconds` : `[60, 300, 600, 900, 1200, 1500, 1800, 2400, 3000]` (1 min à 50 min).

### 19.3 Recording rules Prometheus

```yaml
groups:
  - name: d17_backup
    interval: 30s
    rules:
      - record: efe:backup_failure_rate_5m
        expr: rate(efe_backup_runs_total{result="failed"}[5m])
              / rate(efe_backup_runs_total[5m])

      - record: efe:backup_age_seconds
        expr: time() - efe_backup_last_success_timestamp

      - record: efe:dr_drill_age_seconds
        expr: time() - efe_dr_drill_last_success_timestamp
```

---

## 20. Alertes

### 20.1 Liste exhaustive

| ID | Nom | Expression | Sévérité | Runbook |
|----|-----|------------|:--------:|---------|
| **A-D17-1** | `D17BackupStale` | `efe:backup_age_seconds{type="full"} > 86400 OR efe:backup_age_seconds{type="diff"} > 3600` | 🔴 CRIT | runbook/D17-1.md |
| **A-D17-2** | `D17BackupFailureRate` | `efe:backup_failure_rate_5m > 0.1` | 🟠 HARD | runbook/D17-2.md |
| **A-D17-3** | `D17BackupRpoViolation` | `efe_backup_rpo_seconds > 300` for 5m | 🔴 CRIT | runbook/D17-3.md |
| **A-D17-4** | `D17DrDrillStale` | `efe:dr_drill_age_seconds > 1209600` | 🔴 CRIT | runbook/D17-4.md |
| **A-D17-5** | `D17DrRtoViolation` | `histogram_quantile(0.95, efe_dr_drill_duration_seconds_bucket) > 1800` | 🟠 HARD | runbook/D17-5.md |
| **A-D17-6** | `D17BackupCipherPassMissing` | absent or empty `EFE_BACKUP_CIPHER_PASS` | 🔴 CRIT | runbook/D17-6.md |
| **A-D17-7** | `D17BackupS3Unreachable` | `up{job="efe-backup-s3-probe"} == 0` for 5m | 🔴 CRIT | runbook/D17-7.md |

### 20.2 Runbook A-D17-1 (BackupStale) extract

```markdown
# D17BackupStale runbook

## Symptôme
Le dernier backup `full` ou `diff` réussi date de plus de 24 h
(full) ou 1 h (diff).

## Impact
RPO target (5 min) violé. Selon la durée réelle, perte de données
si incident.

## Triage immédiat
1. `kubectl logs -n efe -l app=efe-backup-postgres --tail=200`
2. Vérifier que le PG primary est healthy.
3. Vérifier que le S3 bucket est accessible
   (`aws s3 ls s3://efe-backups-prod/`).

## Causes communes
- Cipher passphrase changée mais Secret pas updated.
- S3 bucket policy modifiée.
- Réseau cluster ↔ S3 cassé (NetworkPolicy + DNS).

## Mitigation
1. Identifier le root cause via les logs.
2. Une fois résolu, lancer manuellement
   `efe-backup snapshot --type=full` pour rattraper.
3. Vérifier que l'alert clear sous 1 h.

## Escalation
Si non résolu sous 4 h, page le DBA on-call.
```

---

## 21. Helm chart

### 21.1 Nouveaux templates D17

```
deploy/helm/efe/templates/
├── d17-backup-cronjobs.yaml       # NEW
├── d17-backup-rbac.yaml           # NEW (ServiceAccount + Role)
├── d17-backup-secret.yaml         # NEW (cipher pass, vault token refs)
├── d17-backup-pdb.yaml            # NEW (PodDisruptionBudget)
├── d17-dr-drill-cronjob.yaml      # NEW
├── d17-prometheus-rules.yaml      # NEW (recording + alerts)
├── networkpolicy-backup-egress.yaml  # NEW (egress S3 + Vault)
└── ... (existing templates)
```

### 21.2 RBAC minimum

```yaml
# d17-backup-rbac.yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: efe-backup
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: efe-backup
rules:
  - apiGroups: [""]
    resources: ["pods/exec"]   # for restoring into ephemeral drill ns
    verbs: ["create"]
  - apiGroups: [""]
    resources: ["secrets"]
    verbs: ["get"]             # read-only on cipher pass + vault token
    resourceNames:
      - pgbackrest-cipher
      - vault-snapshot-token
  - apiGroups: ["batch"]
    resources: ["cronjobs"]
    verbs: ["get","list","watch"]   # Drill discovers itself
```

### 21.3 NetworkPolicy egress

```yaml
# networkpolicy-backup-egress.yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: efe-backup-egress
spec:
  podSelector:
    matchLabels:
      app: efe-backup
  policyTypes: [Egress]
  egress:
    # S3 endpoint
    - to:
        - ipBlock:
            cidr: 52.219.0.0/22  # eu-west-1 S3 prefix list
      ports: [{ protocol: TCP, port: 443 }]
    # Vault
    - to:
        - podSelector:
            matchLabels:
              app: vault
      ports: [{ protocol: TCP, port: 8200 }]
    # PG primary (intra-cluster)
    - to:
        - podSelector:
            matchLabels:
              role: primary
              app: postgres
      ports: [{ protocol: TCP, port: 5432 }]
```

---

## 22. Test plan

### 22.1 Unit tests (Rust, ~40 tests visés)

* `efe-backup::config` — env parsing, defaults, error paths.
* `efe-backup::manifest` — Ed25519 sign + verify happy + corruption.
* `efe-backup::snapshot::PgBackRestSnapshotter` — happy path
  (mocked `tokio::process::Command`).
* `efe-backup::snapshot::VaultSnapshotter` — happy path.
* `efe-backup::snapshot::BundleSnapshotter` — happy path.
* `efe-backup::restore::CrossEnvGuard` — refuses cross-env without
  flag.
* `efe-backup::audit` — emits HMAC-chained event.
* `efe-backup::metrics` — 8 instruments registered + label-arity
  pin.

### 22.2 Integration tests (Rust, ~10 tests visés)

* `tests/d17_backup_integration.rs::end_to_end_pg_backup_restore` :
  use `testcontainers` to spin a Postgres + a MinIO ; run
  snapshot → wipe → restore → verify.
* `tests/d17_backup_integration.rs::audit_chain_post_restore` :
  D14-bis chain re-verify after restore.
* `tests/d17_backup_integration.rs::cross_env_restore_refused`.
* `tests/d17_backup_integration.rs::four_eyes_delete_workflow`.
* `tests/d17_backup_integration.rs::dr_drill_smoke`.

### 22.3 Python pin tests (~25 tests visés)

* Assert each Helm template renders cleanly.
* Assert ConfigMap renders all `EFE_BACKUP_*` env vars.
* Assert RBAC contains exactly the permissions required.
* Assert PrometheusRules contains all 7 alerts (A-D17-1 to 7).
* Assert Cargo.toml lists `efe-backup` workspace member.
* Assert `efe-backup` crate version matches workspace version.
* Assert `0006_d17_backup_history.sql` exists and matches schema.
* Assert `./dr/restore.sh` exists and is executable.

### 22.4 DR drill end-to-end (CI)

A new GitHub Actions job `dr-drill-ci` :

```yaml
name: DR Drill CI
on:
  schedule:
    - cron: "0 4 * * 0"  # Sunday 04:00 UTC
  workflow_dispatch: {}

jobs:
  drill:
    runs-on: ubuntu-latest
    timeout-minutes: 45
    steps:
      - uses: actions/checkout@v4
      - name: Spin Postgres + MinIO
        run: ./dr/setup-test-env.sh
      - name: Take backup
        run: ./bin/efe-backup snapshot --type=full --label=ci-drill
      - name: Wipe data
        run: ./dr/wipe.sh
      - name: Restore
        run: ./dr/restore.sh ci-drill --target=drill
      - name: Smoke test
        run: ./dr/smoke_test.sh drill
      - name: Assert RTO < 30 min
        run: |
          duration=$(jq -r .duration_seconds < ./dr/timing.json)
          [[ "$duration" -lt 1800 ]] || exit 1
```

---

## 23. SLOs (DR-1 à DR-5)

* **SLO-DR-1** : 99 % des backups réussissent (eligible = `ok`,
  ineligible = `failed` ou `partial`). Mesure rolling 30 jours.
* **SLO-DR-2** : 99.9 % des backups respectent le RPO target
  (5 min). Mesure rolling 7 jours.
* **SLO-DR-3** : 100 % des DR drills réussissent. Mesure rolling
  90 jours (12 drills attendus, 0 échec acceptable).
* **SLO-DR-4** : 95 % des DR drills respectent le RTO target
  (30 min). Mesure rolling 90 jours (au moins 11 sur 12 drills).
* **SLO-DR-5** : 100 % des restores prod sont `ok` au smoke test.
  Mesure : tous les restores prod (pas drill).

---

## 24. Migration plan

### 24.1 Phase 0 — Pre-deploy (J-7)

* Provision S3 bucket avec object-lock + KMS CMK.
* Provision Vault si pas déjà déployé.
* Configurer IAM role pour `efe-backup` ServiceAccount.

### 24.2 Phase 1 — Initial deploy (J-0)

* `helm upgrade --install efe ./deploy/helm/efe --set backup.enabled=true`
* La migration SQL applique `0006_d17_backup_history.sql`.
* Premier full backup déclenché manuellement :
  `efe-backup snapshot --type=full --label=baseline-D17`.

### 24.3 Phase 2 — Cron schedule activation (J+1)

* Activer les CronJobs (`spec.suspend: false`).
* Vérifier sur 24 h que les diff + WAL archiving tournent.

### 24.4 Phase 3 — DR drill premier run (J+7)

* Manuel : déclencher `efe-backup drill` une fois.
* Si pass, activer le CronJob hebdomadaire.
* Si fail, freeze le déploiement, investigate.

### 24.5 Phase 4 — Documentation client (J+14)

* Publier le runbook DR public dans le portail tenant.
* Annoncer aux clients Tier 1 le SLA RTO/RPO formel.

---

## 25. Cardinality envelope

| Source | Séries Prometheus | Note |
|--------|------------------:|------|
| Métriques D17 (§ 19) | 67 | Largement sous le budget |
| `efe_backup_history` rows / mois | ~9 000 | Compressé via partman post-3-mois |
| Audit log events / mois | ~1 000 | 30 j × ~30 ops/j |
| WAL archiving frequency | every 5 min | 8 640 / mois |
| Bundle snapshot frequency | weekly | 4 / mois |

Aucun risque de cardinality explosion.

---

## 26. Critères d'acceptation D17

D17 est livré quand **tous** les critères ci-dessous sont
satisfaits sur `main` :

- [ ] `cargo test -p efe-backup --lib` : ≥ 40 verts (cf. § 22.1).
- [ ] `cargo test -p efe-backup --test d17_backup_integration` : ≥ 5 verts.
- [ ] `pytest scripts/tests/test_d17_backup_dr.py` : ≥ 25 verts.
- [ ] Helm chart renders cleanly (`helm template -f values-prod.yaml`).
- [ ] DR drill premier run réussit avec RTO < 30 min, RPO < 5 min.
- [ ] No regression D13/D13-bis/D14/D14-bis/D15/D16 metrics.
- [ ] Performance bench : `Execute` p99 delta ≤ 2 % during a full
      base backup.
- [ ] CHANGELOG.md entry + audit interne livré
      `docs/audits/d17_backup_dr_audit_report.md`.
- [ ] Runbook `dr/restore.sh` testé manuellement par 2 ingénieurs
      différents.
- [ ] DPO signoff sur la procédure de re-anchor (§ 12.3).

---

## 27. Anti-corruption — versioning des formats backup

### 27.1 Manifest version field

Chaque manifest contient un champ `version: <semver>`. Lecteur
strict : refuse les versions inconnues.

```json
{
  "version": "1.0.0",
  "label": "...",
  "env": "prod",
  ...
}
```

### 27.2 pgBackRest version pinning

* Pin pgBackRest version dans le base-image Docker (`pgbackrest:2.50`).
* Renovate bot ouvre PR sur minor bumps ; review obligatoire.
* Major bump (`2.x → 3.x`) = audit pass requis + amendement du
  document architecture.

### 27.3 Backward compatibility window

Un restore depuis un backup de version N doit fonctionner avec
le binaire `efe-backup` de version N+1. Pour N+2 et au-delà,
l'opérateur doit utiliser le binaire historique compatible.

---

## 28. Non-objectifs explicites

D17 n'inclut PAS :

1. **Multi-region active-active** — sprint D18+ candidate.
2. **Backup des logs Loki** — transient, hors scope.
3. **Backup des métriques Prometheus** — transient, hors scope.
4. **Auto-scaling backup workers** — fixed `process-max=4`.
5. **Backup des images container** — gérées par le registry,
   hors scope EFE.
6. **DR test client-side** — chaque client gère son propre PRA
   de son côté de l'API.
7. **Encryption en quantum-safe** — out of scope ; D17 utilise
   AES-256-GCM + Ed25519.
8. **Hot-standby Postgres** — `cloudnative-pg` gère la haute
   disponibilité ; D17 gère uniquement la **continuité**, pas la
   **disponibilité**.
9. **Tenant-self-service backup** — les opérateurs gèrent les
   backups, pas les tenants individuels.
10. **GDPR right-to-erasure automation** — documenté dans
    `T-DR-10` mitigation, mais l'effacement effectif requiert
    une opération manuelle DPO (out of scope D17).

---

## 29. Inventaire ASCII complet (synthèse)

```
SPRINT D17 — Backup + DR
═════════════════════════════════════════════════════════════════

NEW FILES (estimated)
  crates/efe-backup/                       ~2 200 lignes Rust
    Cargo.toml                                  ~40
    src/lib.rs                                  ~80
    src/config.rs                              ~280
    src/snapshot.rs                            ~350
    src/postgres.rs                            ~330
    src/vault.rs                               ~180
    src/bundles.rs                             ~190
    src/manifest.rs                            ~220
    src/restore.rs                             ~310
    src/verify.rs                              ~230
    src/drill.rs                               ~190
    src/audit.rs                                ~80
    src/metrics.rs                             ~280
    src/error.rs                               ~120
    tests/d17_backup_integration.rs            ~520

  bin/efe-backup/                          ~600 lignes Rust
    src/main.rs                                ~600

  deploy/helm/efe/templates/               ~480 lignes YAML
    d17-backup-cronjobs.yaml                  ~140
    d17-backup-rbac.yaml                       ~50
    d17-backup-secret.yaml                     ~40
    d17-backup-pdb.yaml                        ~30
    d17-dr-drill-cronjob.yaml                  ~70
    d17-prometheus-rules.yaml                  ~80
    networkpolicy-backup-egress.yaml           ~70

  dr/                                       ~250 lignes shell
    restore.sh                                ~120
    smoke_test.sh                              ~80
    setup-test-env.sh                          ~50

  migrations/                                ~30 lignes SQL
    0006_d17_backup_history.sql                ~30

  scripts/tests/test_d17_backup_dr.py       ~430 lignes Python pin

  docs/bench/d17_backup_dr_architecture.md  (this file, ~1 800 lignes)

CHANGES TO EXISTING
  Cargo.toml workspace                     +1 ligne (efe-backup)
  deploy/helm/efe/values.yaml             +120 lignes (backup: {})
  deploy/helm/efe/templates/configmap.yaml +14 lignes (D17 block)
  CHANGELOG.md                              +50 lignes

TOTAL D17 net ajout : ~5 000 lignes.
```

---

## 30. Conclusion

D17 transforme EFE d'un système **fonctionnel** en un système
**survivable**. Les défauts pré-D17 (perte de tout en cas
d'incident) sont résolus par :

1. **Inventaire exhaustif** des données critiques, classées par
   tier de criticality avec stratégie backup adaptée.
2. **Pipeline backup orchestrée** par un crate dédié
   `efe-backup` qui réutilise les invariants existants
   (HMAC chain D14-bis, Ed25519 trust store D16, OIDC D15).
3. **Verification post-restore** systématique : HMAC chain
   re-vérifiée, signatures Ed25519 re-vérifiées, OIDC issuers
   reachables, smoke test fonctionnel.
4. **DR drill automatique hebdomadaire** garantissant que le
   runbook reste opérationnel et que les SLOs RTO/RPO sont
   atteints.
5. **Threat model documenté** avec mitigation pour 12 menaces
   couvrant ransomware, insider DBA, region outage, audit
   divergence, cross-env leak, supply-chain.
6. **Compliance** : RGPD Art. 32, Loi française PRA/PCA, future
   SOC 2 Type II.

Le sprint suivant (D18) pourra construire sur D17 pour livrer
multi-region active-active.

---

*Document architecture livré 2026-04-28. Auteur : architecture
interne (Claude). Pour questions ou amendements, contact
maintainer EFE.*
