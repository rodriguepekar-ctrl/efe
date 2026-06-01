# Runbook — Bundle Revocation Emergency

> **Sévérité** : critical (PagerDuty P0).
> **Trigger** : vulnérabilité ML / supply-chain / clé compromise.
> **Sprint d'origine** : Φ-C D27.
> **Référence architecture** : `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md`
> § 11.2.

---

## TL;DR

Une emergency revocation est déclenchée quand un bundle déployé en
production présente une vulnérabilité critique (= modèle empoisonné,
clé de signature volée, faille ONNX runtime, IP infringement, etc.).

L'objectif est de :
- **Bloquer** les nouveaux déploiements du bundle compromis en < 15 min.
- **Roll-forward** les replicas existants vers un bundle propre en < 4 h.
- **Communiquer** publiquement aux clients en < 30 min.

Ce runbook est exécuté par l'**équipe sécurité on-call** uniquement.
Une revocation par erreur a un coût opérationnel énorme (= reboot
fleet) ; lever le drapeau seulement après confirmation tier-2.

---

## 1. Triage (T = 0 → +5 min)

### 1.1 Confirmation de la menace

3 sources possibles déclenchent un triage :

1. **Vulnérabilité ML upstream** : HF rapporte un CVE sur le model
   source (= modèle empoisonné post-publication).
2. **Vulnérabilité ONNX Runtime** : Microsoft rapporte un CVE
   `ort` ≥ HIGH severity affectant la version pinnée.
3. **Compromise clé de signature** : OpsSec détecte un usage
   anormal de la clé Ed25519 (= sortie audit log,
   `cosign verify` qui ne devrait pas exister).

→ **Évaluation par tier-2 sécurité** (≤ 5 min) : la menace est-elle
**activement exploitable** en production ? Si non → ticket
P1/P2 normal.

### 1.2 Décision de revocation

Si menace confirmée :

```bash
# Sur la console sécurité, déclencher la commande revocation
$ efe-attest bundle revoke \
    --bundle-id paraphrase-multilingual-MiniLM-L12-v2 \
    --version 1.0.0 \
    --reason "CVE-2026-XXXX exploitable in CUDA kernels" \
    --approved-by alice@efe.ai \
    --approved-by bob@efe.ai

# Cela :
# 1. UPDATE bundle_releases SET revoked_at=NOW(), revoked_reason=... ;
# 2. push une entrée Rekor "revoke" pour la transparency ;
# 3. notify tous les replicas via webhook (= force re-load au prochain check) ;
# 4. log dans la chaîne audit D14-bis.
```

---

## 2. Containment (T = +5 min → +30 min)

### 2.1 Update du trust store

PR urgente sur main + auto-merge (= équipe sécurité a `bypass` perm) :

```toml
# /etc/efe/trust-store.toml
[[trusted_keys]]
fingerprint = "<fingerprint v1.0.0 key>"
description = "v1.0.x signing key"
revoked = true                                # ← NEW
revoked_at = "2026-07-15T14:00:00Z"           # ← NEW
revoked_reason = "Compromise — see SR-2026-07-15-001"  # ← NEW
```

### 2.2 Helm chart bump + rolling restart

```bash
# Push une nouvelle image avec le trust-store updated
$ kubectl rollout restart deployment/efe-server -n efe

# Pendant le rollout, les replicas font :
#   1. Boot → load bundle → verify against trust store.
#   2. trust_store says revoked=true → REFUSE boot.
#   3. POD CrashLoopBackOff → alertes PagerDuty.
#   4. Operator doit déployer un bundle propre.
```

### 2.3 Publication d'un bundle clean

```bash
# 1. Tag le bundle v1.0.0-revoked-replacement (=
#    rebuild from scratch avec patch applied).
$ git tag bundle/paraphrase-multilingual-MiniLM-L12-v2/1.0.0-r1

# 2. CI publie automatiquement v1.0.0-r1 via bundle-release.yml.
# 3. Operator update le Helm chart pour pointer sur v1.0.0-r1.
$ helm upgrade efe efe/efe-stack \
    --set bundle.version=1.0.0-r1
```

---

## 3. Communication (T = +5 min → +30 min)

### 3.1 Notification webhook automatique

À T+5 min, le système envoie un webhook à **tous** les tenants :

```json
{
  "event": "BUNDLE_REVOKED",
  "severity": "critical",
  "bundle_id": "paraphrase-multilingual-MiniLM-L12-v2",
  "version": "1.0.0",
  "revoked_at": "2026-07-15T14:00:00Z",
  "reason": "CVE-2026-XXXX",
  "replacement_version": "1.0.0-r1",
  "action_required": "auto-upgrade (= cas Pro/Enterprise)",
  "cve_url": "https://cve.mitre.org/cgi-bin/cvename.cgi?name=CVE-2026-XXXX",
  "status_page_url": "https://status.efe.ai/incidents/INC-2026-07-15-001"
}
```

### 3.2 Status page incident

À T+10 min, un incident est créé sur `status.efe.ai` :

```markdown
## INC-2026-07-15-001 — Bundle Revocation
Severity: critical
Started: 2026-07-15 14:00 UTC

### What happened
On 2026-07-15 14:00 UTC, the security team detected a critical
vulnerability in `paraphrase-multilingual-MiniLM-L12-v2` v1.0.0
(CVE-2026-XXXX). We have revoked the bundle and are deploying a
patched replacement (v1.0.0-r1).

### Impact
- POC and Pro tenants : automatic upgrade in progress.
- Enterprise tenants with custom deployments : manual upgrade
  required ; instructions sent via dedicated channels.

### Status
- T+0  Revocation issued ; webhooks sent.
- T+10 Status page published.
- T+25 Patched bundle published.
- T+60 90 % of fleet upgraded.
- T+120 100 % of fleet upgraded.

### Next update
T+30 minutes.
```

### 3.3 Comms internes

- **Slack** `#incident-emergency` : ping all-hands ; CEO informé.
- **Email** customer success : draft 2-pager pour les top accounts.
- **Phone call** : top 5 enterprise accounts (= contact pré-établi
  dans le DPA).

---

## 4. Roll-forward (T = +30 min → +4 h)

### 4.1 Monitoring des replicas

Dashboard `efe-bundle` surveille :
- `efe_bundle_loaded_at_boot{version="1.0.0-r1"}` ↗ vers 1
- `efe_bundle_loaded_at_boot{version="1.0.0"}` ↘ vers 0
- `efe_bundle_revoked_total` ne doit plus s'incrémenter
- `efe:p1_e2e_p99_seconds` reste < 200 ms pendant le rollout

### 4.2 Enterprise tenants

Pour les tenants Enterprise avec deploy custom :

1. Customer Success contact direct.
2. Provision d'un consultant pour migration assistée.
3. Cap d'un délai de 24 h pour migration ; au-delà, leur cluster
   est isolé via firewall (= safety net).

### 4.3 Air-gapped tenants

```bash
# Sur la machine en ligne du tenant :
$ skopeo copy --src-cert-dir=/certs/efe \
    docker://registry.efe.ai/bundles/paraphrase-multilingual-MiniLM-L12-v2:1.0.0-r1 \
    docker-archive:efe-bundle-r1.tar:efe-bundle:1.0.0-r1

# Transfer + push interne :
$ scp efe-bundle-r1.tar jump-host:/data/
$ ssh jump-host
$ skopeo copy docker-archive:/data/efe-bundle-r1.tar:efe-bundle:1.0.0-r1 \
    docker://internal-registry.customer.local/efe/bundles/...
```

---

## 5. Post-mortem (T = +24 h → +5 j)

### 5.1 RCA publique

Publication sur `status.efe.ai` :

- Root cause confirmée
- Timeline détaillée (= every minute)
- Action items pour éviter récidive

### 5.2 Post-mortem interne

Template `docs/runbook/p2_post_mortem_template.md` adapté pour
bundle revocation, focusé sur :
- Comment la vulnérabilité a échappé aux gates qualité ?
- Le temps de détection (= < ou > 1 h ?)
- Le temps de containment (= < ou > 30 min ?)
- Toutes les automations qui pourraient être ajoutées.

### 5.3 SOC 2 evidence

L'incident est ajouté au registre `bundle_release_attestations`
avec :
- `attestor_kind = "internal_security"`
- `attestation_payload = {"incident_id": "INC-...", "rca_url": "..."}`

---

## 6. Escalade

| Durée incident | Décision |
|----------------|----------|
| < 30 min | Security on-call + tech lead |
| 30 min - 4 h | Engager CEO + Customer Success Director |
| 4 - 24 h | Activer la cellule de crise (5 personnes) |
| > 24 h | Notification régulateur (= EU AI Act Art. 73 si "significant incident") |

---

## 7. Liens utiles

- Architecture : `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md` § 11.2
- `efe-attest` CLI : `docs/cli/efe_attest.md`
- D14-bis audit chain : `docs/bench/d14bis_hmac_audit_chain_architecture.md`
- Status page : https://status.efe.ai
- Rekor transparency log : https://rekor.sigstore.dev
