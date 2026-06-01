# Runbook — Bundle Rotation (= rotation 6 mois cadence)

> **Cadence** : tous les 6 mois (= patch securité ORT + re-sign nouvelle clé Ed25519).
> **Sévérité** : info (= rotation programmée).
> **Sprint d'origine** : Φ-C D27.
> **Référence architecture** : `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md`
> § 11.1.

---

## TL;DR

Tous les 6 mois, on roule la version mineure du bundle production
(`v1.0.0` → `v1.0.1` → `v1.0.2` …) avec :
- Bump toolchain (= ORT 1.18 → 1.19, etc.).
- Re-sign avec **nouvelle clé Ed25519** (= cycle 6 mois).
- Notification aux clients 30 jours à l'avance.

La rotation **n'est PAS une emergency**. Elle suit un calendrier
public et passe par tous les gates qualité (recall, latence, SLO).
Pour les emergencies, voir `bundle_revocation_emergency.md`.

---

## 1. Pré-conditions (T = -30 j)

### 1.1 Décision Go/No-Go

L'équipe sécurité + ML lead se mettent d'accord sur :

- [ ] Quelle version d'ORT cibler ? (= changelog ORT, breaking changes ?)
- [ ] Mêmes inputs HF model ou bump version source ? (= rare)
- [ ] Le bundle restera-t-il compatible avec les catalogs en prod ?
  (= `manifest.dim` reste 384, `pooling=mean`, `normalize=true`)

Si l'un des "non" → escalade architecture review.

### 1.2 Génération de la nouvelle clé Ed25519

```bash
# Sur la machine de sécurité (HSM-backed) :
$ openssl genpkey -algorithm ed25519 -out efe-bundle-signing-key-v2.pem
$ openssl pkey -in efe-bundle-signing-key-v2.pem -pubout -out efe-bundle-signing-key-v2.pub
$ sha256sum efe-bundle-signing-key-v2.pub
<fingerprint>

# Backup la clé privée dans le KMS (= chiffrée at-rest)
$ aws kms encrypt --key-id alias/efe-bundle-keys \
    --plaintext fileb://efe-bundle-signing-key-v2.pem \
    --output text --query CiphertextBlob > efe-bundle-signing-key-v2.pem.enc
```

### 1.3 Mise à jour du trust store

```toml
# /etc/efe/trust-store.toml (= shipped dans image Docker)
[[trusted_keys]]
fingerprint = "<fingerprint v1>"
description = "v1.0.x signing key (= rotation cadence 6 mois)"
added_at = "2026-01-01T00:00:00Z"
revoked = false

[[trusted_keys]]
fingerprint = "<fingerprint v2>"   # ← NEW
description = "v2.0.x signing key — added 2026-07-01"
added_by = "security-team"
added_at = "2026-07-01T00:00:00Z"
revoked = false
```

PR `bump-trust-store-v2.0` mergée sur main avant T = 0.

### 1.4 Annonce aux clients (T = -30 j)

Webhook envoyé à tous les tenants Pro+ :

```json
{
  "event": "BUNDLE_ROTATION_SCHEDULED",
  "bundle_id": "paraphrase-multilingual-MiniLM-L12-v2",
  "current_version": "1.0.0",
  "target_version": "1.0.1",
  "scheduled_at": "2026-07-01T00:00:00Z",
  "action_required": "none (= auto-upgrade)",
  "rollback_window_days": 180,
  "breaking_changes": false,
  "details_url": "https://docs.efe.ai/bundles/rotations/2026-07-01"
}
```

---

## 2. Build (T = -7 j → -1 j)

### 2.1 Tag de la nouvelle version

```bash
$ git tag bundle/paraphrase-multilingual-MiniLM-L12-v2/1.0.1
$ git push origin bundle/paraphrase-multilingual-MiniLM-L12-v2/1.0.1
```

→ trigger automatique du workflow `bundle-release.yml`.

### 2.2 Validation des gates qualité

CI exécute :
1. `bundle-rust-tests` → instruments OK.
2. `bundle-python-tests` → benches OK.
3. `bundle-build-ci` → HF pull + quantize + sign avec **clé v2**.
4. `bundle-quality-gates` :
   - Recall ≥ 0.92 ?
   - Parité FR/EN ≤ 1.05× ?
   - Latence CPU p99 ≤ 180 ms ?
   - Reproductibility cross-runner ?
5. `bundle-publish` → cosign + Rekor + OCI push.

**Critère d'arrêt** : tous les gates PASS sinon hold + investigate.

### 2.3 Publication en `latest-beta`

Le bundle v1.0.1 est tagué `latest-beta` dans l'OCI registry :

```bash
$ oras tag registry.efe.ai/bundles/paraphrase-multilingual-MiniLM-L12-v2:1.0.1 latest-beta
```

Les clients qui opt-in pour `latest-beta` (= ~ 5 % du fleet)
commencent à le pull.

---

## 3. Canary deploy (T = 0 → +8 h)

```
T = 0      : 5 %    du trafic sur v1.0.1
T + 1 h    : 25 %   si SLO vert
T + 2 h    : 50 %   si SLO vert
T + 4 h    : 100 %  si SLO vert
T + 8 h    : full deploy ; ancien v1.0.0 reste disponible 180 j
```

À chaque pas, on observe sur le dashboard Grafana :
- `efe:p2_burn_rate:1h` reste < 1
- `efe:p1_e2e_p99_seconds` reste < 0.2
- `efe_bundle_inference_duration_seconds` p99 (= étage 5 P1)
  reste < 26 ms

Rollback automatique si un SLO breach.

---

## 4. Confirmation post-rotation (T = +24 h)

- [ ] `efe_bundle_loaded_at_boot = 1` sur 100 % des replicas
- [ ] `efe_bundle_fingerprint_info{version="1.0.1"} = 1` sur 100 %
- [ ] Aucun client n'a ouvert un support ticket avec category =
      "bundle_upgrade_issue"
- [ ] Rapport SOC2 trimestriel inclut la rotation dans
      `bundle_releases` ledger
- [ ] Status page note la rotation comme "successful maintenance"

---

## 5. Liens utiles

- Architecture : `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md` § 11.1
- Trust store doc : `docs/operations/trust_store.md`
- Compliance EU AI Act : `docs/compliance/eu_ai_act_bundle.md`
- Webhook spec : `docs/api/webhook_events.md#BUNDLE_ROTATION_SCHEDULED`
