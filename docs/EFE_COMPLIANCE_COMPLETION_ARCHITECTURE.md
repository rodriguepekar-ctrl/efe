# EFE — Architecture **Φ-D-COMPLIANCE** — Compliance opposable end-to-end

> **Sprint** : Φ-D-COMPLIANCE (D85 → D89)
>
> **Mission** : transformer la documentation compliance dispersée (M14
> threat model + ISO/SOC 2/GDPR mappings, M29 GDPR Art. 17/20, M24 AL
> GDPR Art. 22, Mode-3 GDPR Art. 22, breach detection alignment) en
> **enforcement runtime opposable** : chaque control documenté produit
> une evidence vérifiable, chaque event compliance-relevant trace via
> HMAC chain, chaque framework (GDPR, SOC 2, ISO 27001, EU AI Act,
> HIPAA-ready, CCPA) couvert avec SLO et anti-régression CI.
>
> **Doctrine cardinale** : *aucune compliance ne tient sans evidence
> runtime opposable*. Un control « documented only » est un faussement-
> PASS par défaut ; le sprint Φ-D-COMPLIANCE en éradique la possibilité.

---

## Table des matières

- **§ 0** — Méta : conventions, glossaire, doctrines D-COMP-*
- **§ 1** — Mission Φ-D-COMPLIANCE & contrat opposable
- **§ 2** — État AS-IS : ce qui existe (M14, M29, M24, Mode-3, breach)
- **§ 3** — Gap analysis : documentation ↔ enforcement
- **§ 4** — Surface 1 : Framework registry + control mapping
- **§ 5** — Surface 2 : Evidence runtime (= 1 script par control)
- **§ 6** — Surface 3 : Audit chain compliance-grade
- **§ 7** — Surface 4 : Breach detection + 72h notification
- **§ 8** — Surface 5 : Right to erasure + portability (Art. 17/20)
- **§ 9** — Surface 6 : Sub-processor lifecycle
- **§ 10** — Surface 7 : Data residency enforcement
- **§ 11** — Surface 8 : EU AI Act risk classification + oversight
- **§ 12** — Surface 9 : Consent granularity + transparency
- **§ 13** — Surface 10 : DPIA + Records of Processing automation
- **§ 14** — SLO opposables Φ-D-COMPLIANCE (= 18 SLOs)
- **§ 15** — Sécurité : tamper-evident audit + key rotation
- **§ 16** — Matrice complète des artefacts Φ-D-COMPLIANCE (= 40)
- **§ 17** — Plan d'implémentation D85 → D89
- **§ 18** — Risques + mitigations (24)
- **§ 19** — Décisions clés (22)
- **§ 20** — Anti-régression (= 36 pin tests + 8 evidence audits)
- **§ 21** — Critères d'acceptation (DoD à 100 %)
- **§ 22** — Hand-off & dépendances downstream
- **§ 23** — Annexes (A → I)
- **§ 24** — Defer Tickets `DT-COMP-*` (= 14 vrais SKIPs opposables)

---

## § 0. Méta, conventions, glossaire chirurgical

### 0.1 Doctrines D-COMP-* (= 10 contraintes opposables)

| Doctrine | Énoncé |
|---|---|
| **D-COMP-1 — Coverage opposable** | Chaque framework ciblé (GDPR, SOC 2, ISO 27001, EU AI Act, HIPAA-ready, CCPA) a un fichier `docs/compliance/<framework>.md` listant ses **controls** mappés vers les artefacts EFE. **Coverage = 100 %** des controls applicables (= « not applicable » est autorisé mais justifié). |
| **D-COMP-2 — Evidence runtime, pas documentation seule** | Chaque control mappé produit une **evidence script** : `eval/scripts/comp_<control_id>_evidence.py` ou pointeur vers un script existant. La doctrine **interdit** les controls « documented only » sans test d'exécution. Tout faussement-PASS pattern (= « le control existe parce qu'on l'a écrit ») est éradiqué. |
| **D-COMP-3 — Audit trail tamper-evident** | Chaque event compliance-relevant (consent change, erasure, breach, DPIA update, sub-processor change, AI Act risk reclassification) est enregistré dans `audit_events` avec HMAC chain (= D14-bis baseline). Le replay quotidien (= workflow `comp-audit-replay.yml`) vérifie l'intégrité. |
| **D-COMP-4 — Breach notification ≤ 72h** | Tout incident classé « personal data breach » (GDPR Art. 33) déclenche la chaîne automatisée : détection → triage → notification DPO → notification autorité de contrôle. **SLO** : DPO notifié ≤ 72h après détection. Containment ≤ 24h. |
| **D-COMP-5 — Right to erasure ≤ 30 days** | GDPR Art. 17 request → erasure cascade exécutée + audit logged ≤ 30 days. Pin via M29 + SLO-COMP-ERASURE-30D. |
| **D-COMP-6 — Sub-processor lifecycle** | Tout LLM provider ajouté ou retiré déclenche : (a) DPIA update auto-régénérée, (b) tenant notification 30 jours minimum, (c) Records of Processing rebuild, (d) audit chain entry. Aucun provider « silently added » possible. |
| **D-COMP-7 — Data residency strict** | Un tenant avec `data_region="EU"` ne peut pas voir ses données router via un LLM US-only ou stocker dans une région non-EU. **Enforcement runtime** via `tenant_config.data_region` + provider region match. Cross-region leak SLO = 0. |
| **D-COMP-8 — EU AI Act risk classification** | Chaque catalog porte un `ai_act_risk_level` ∈ {minimal, limited, high, unacceptable}. Les catalogs `high` exigent : (a) Mode-3 forcé pour les décisions critiques, (b) DPO sign-off, (c) transparence renforcée (Art. 13 RGPD étendu). Les `unacceptable` sont **interdits**. |
| **D-COMP-9 — Consent granularité 4 niveaux** | 4 axes indépendants : `{analytics, model_training, marketing, third_party_sharing}`. Default = all opt-out (Art. 7 RGPD principe de consentement explicite). Stockage versionné + retracé via audit chain. |
| **D-COMP-10 — Conformity assessment annuel** | Tous les artefacts compliance (mappings, evidences, DPIA, ROPA, threat model) re-générés ≥ 1× / an. HMAC keys rotées. STRIDE re-run. Sign-off DPO + Security Lead requis. |

### 0.2 Glossaire chirurgical

| Terme | Définition |
|---|---|
| **Control** | Mesure spécifique d'un framework (ex : ISO 27001 A.5.1 = politiques de sécurité). EFE mappe chaque control vers ≥ 1 artefact code/runbook. |
| **Evidence** | Script qui prouve à l'exécution qu'un control est appliqué. Ex : pour ISO A.9.4.3 = `comp_iso27001_a943_password_complexity_evidence.py`. |
| **Mapping** | Tableau `control → artefacts EFE` dans `docs/compliance/<framework>.md`. |
| **DPIA** | Data Protection Impact Assessment, GDPR Art. 35. Document généré automatiquement par catalog `high-risk`. |
| **ROPA** | Records of Processing Activities, GDPR Art. 30. Inventaire des traitements par tenant. |
| **DPO** | Data Protection Officer. Personne désignée par le client + EFE. |
| **Sub-processor** | Tiers traitant des données pour le compte du tenant (= LLM providers, embedders, telemetry). |
| **AI Act risk level** | Classification UE 2024 : minimal / limited / high / unacceptable. |
| **Data residency** | Région géographique de stockage + traitement. EU / US / APAC / GLOBAL. |
| **Consent granularité** | 4 axes : analytics, model_training, marketing, third_party_sharing. |
| **Threat model STRIDE** | Spoofing, Tampering, Repudiation, Information disclosure, DoS, Elevation. |
| **Conformity assessment** | Audit annuel des artefacts compliance + sign-off DPO. |

### 0.3 Conventions de référence

- Control IDs : `<FW>-<ID>` (e.g. `GDPR-Art17`, `ISO-A.9.4.3`, `SOC2-CC6.7`).
- SLO : `SLO-COMP-<KEY>`.
- Defer Ticket : `DT-COMP-<NAME>`.
- Sprint Φ-D-COMPLIANCE = 5 jours D85 → D89.
- Cross-references : `M14` (= compliance extension), `M29` (= data rights),
  `M24` (= active learning), `Mode-3`, `D14-bis` (= HMAC chain baseline).

### 0.4 Anti-mission (= ce que Φ-D-COMPLIANCE **ne fait pas**)

- ❌ Pas de **certification** (= SOC 2 Type II audit, ISO 27001 cert) — réservé aux auditeurs externes.
- ❌ Pas de **PCI-DSS** (= aucun paiement traité directement par EFE).
- ❌ Pas de **FedRAMP** (= US government, hors scope MVP).
- ❌ Pas de **HIPAA full** (= seulement "HIPAA-ready" = la baseline ; le mode HIPAA actif demande un sprint dédié).
- ❌ Pas de **réécriture** du threat model M14 — extension uniquement.

---

## § 1. Mission Φ-D-COMPLIANCE & contrat opposable

### 1.1 Mission

> Transformer la documentation compliance en enforcement runtime
> opposable :
>
> 1. **Framework registry** : 6 frameworks × controls × mapping artefact
>    EFE × evidence script.
> 2. **Evidence runtime** : chaque control a un script qui prouve à
>    l'exécution qu'il est appliqué (= pas de control « documented only »).
> 3. **Audit chain compliance** : HMAC chain replay quotidien sur les
>    events compliance-relevant.
> 4. **Breach detection + 72h notification** automatisée.
> 5. **Right to erasure ≤ 30 days** verified by SLO.
> 6. **Sub-processor lifecycle** automatisé.
> 7. **Data residency** enforced par config tenant + matching provider.
> 8. **EU AI Act risk classification** + DPO sign-off pour high-risk.
> 9. **Consent granularity** 4 niveaux + transparency notice freshness.
> 10. **DPIA + ROPA** automation : generated per-tenant + per-catalog.

### 1.2 Contrat opposable (= 18 SLOs synthétiques)

| Surface | SLO | Source bench | Trigger |
|---|---|---|---|
| GDPR controls | **SLO-COMP-GDPR-CONTROLS** = 47/47 mappés | mapping audit | PR-gate |
| SOC 2 controls | **SLO-COMP-SOC2-CONTROLS** = 64/64 mappés (TSC) | mapping audit | PR-gate |
| ISO 27001 controls | **SLO-COMP-ISO27001-CONTROLS** = 93/93 mappés (Annex A) | mapping audit | PR-gate |
| AI Act controls | **SLO-COMP-AI-ACT-CONTROLS** = 38/38 mappés | mapping audit | PR-gate |
| Evidence runtime | **SLO-COMP-EVIDENCE-COVERAGE** = 100 % controls ont evidence | evidence audit | PR-gate |
| Audit chain daily | **SLO-COMP-AUDIT-CHAIN-DAILY** = chain replay ≥ 1× / jour | replay tool | nightly |
| DPIA freshness | **SLO-COMP-DPIA-FRESHNESS** = ≤ 6 mois old per catalog | DPIA audit | nightly |
| ROPA freshness | **SLO-COMP-ROPA-FRESHNESS** = ≤ 1 mois old | ROPA audit | nightly |
| Sub-processor inventory | **SLO-COMP-SUBPROCESSOR-INVENTORY** = ≤ 7 jours old | inventory audit | nightly |
| Breach notify 72h | **SLO-COMP-BREACH-NOTIFY-72H** ≤ 72h DPO notif | injection test | PR-gate |
| Breach contain 24h | **SLO-COMP-BREACH-CONTAIN-24H** ≤ 24h containment | injection test | PR-gate |
| Erasure 30d | **SLO-COMP-ERASURE-30D** ≤ 30 jours | erasure audit | nightly |
| Portability 30d | **SLO-COMP-PORTABILITY-30D** ≤ 30 jours | portability audit | nightly |
| Residency leak | **SLO-COMP-RESIDENCY-LEAK** = 0 cross-region | adversarial scan | PR-gate |
| AI Act oversight | **SLO-COMP-AI-ACT-HIGH-OVERSIGHT** = 100 % high-risk Mode-3 | oversight audit | PR-gate |
| AI Act DPO sign-off | **SLO-COMP-AI-ACT-DPO-SIGNOFF** = 100 % high-risk signed | DPO registry | PR-gate |
| Consent version | **SLO-COMP-CONSENT-VERSION** = match policy | consent audit | nightly |
| Transparency notice | **SLO-COMP-TRANSPARENCY-NOTICE** ≤ 30 jours | notice audit | nightly |

**Total** : 18 SLOs symétriques aux modules précédents (AL, Mode-3, Streaming).

### 1.3 Anti-mission (= ce que Φ-D-COMPLIANCE **ne fait pas**)

- ❌ Ne fait pas la **certification externe** (= audit firm SOC 2 / ISO).
- ❌ Ne réécrit pas le threat model M14 (extension uniquement).
- ❌ Ne couvre pas PCI-DSS, FedRAMP (hors scope MVP).
- ❌ Ne fait pas HIPAA full mode (= baseline seulement, "HIPAA-ready").
- ❌ Ne modifie pas les bodies des RPCs production (= sortie de scope ;
   reste sur les artefacts compliance + evidence scripts + workflows).

---

## § 2. État AS-IS : ce qui existe

### 2.1 Sources de vérité documentaires (= 12 fichiers)

| Fichier | Lignes | Rôle |
|---|---|---|
| `docs/EFE_M29_DATA_RIGHTS_ARCHITECTURE.md` | 1173 | GDPR Art. 17 (erasure) + Art. 20 (portability) + BYOK + sub-processors + supply chain |
| `docs/EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md` | 2566 | Threat model STRIDE + ISO 27001 mapping + SOC 2 TSC mapping + GDPR mapping (M14) |
| `docs/EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md` | ~750 | GDPR Art. 22 (auto-decision) côté Active Learning |
| `docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md` | ~1400 | GDPR Art. 22 + AES-256-GCM at rest |
| `docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md` | ~1700 | HMAC chain audit replay + tenant isolation streaming |
| `docs/compliance/iso27001.md` | (référencé M14) | ISO 27001 Annex A 93 controls |
| `docs/compliance/soc2.md` | (référencé M14) | SOC 2 TSC 64 controls |
| `docs/compliance/gdpr.md` | (référencé M14) | GDPR 47 controls |
| `docs/compliance/threat_model.md` | (référencé M14) | STRIDE 63 threats |
| `docs/runbook/*.md` | ~40 fichiers | Runbooks opérationnels |
| `bench_baselines/v0.6.0/al_slos.yaml` | 18 SLOs | AL GDPR consent SLO inclus |
| `bench_baselines/v0.7.0/stream_slos.yaml` | 18 SLOs | Audit chain integrity inclus |

### 2.2 Primitives runtime existantes

| Primitive | Source | État |
|---|---|---|
| `audit_events` table + HMAC chain | D14-bis | ✓ production |
| `consent_hash` field | M24 AL | ✓ production |
| `tenant_id` strict filter | tous services | ✓ production |
| AES-256-GCM at rest | Mode-3 | ✓ production |
| GDPR Art. 17 erasure RPC | M29 (`TriggerGdprArt17Erasure`) | ✓ production |
| GDPR Art. 20 portability | M29 | ✓ production |
| Breach detection alignment | alignment crate | ✓ production (= alignment cone violations) |
| Sub-processor list endpoint | M29 (`/sub-processors`) | ✓ production |
| Audit replay daily | AL + Streaming | ✓ production |
| Tenant config `data_region` | (config layer) | ⚠ partial (= field exists but pas d'enforcement runtime) |
| AI Act risk classification | — | ✗ ABSENT |
| DPIA automation | — | ✗ ABSENT (= manuel) |
| ROPA automation | — | ✗ ABSENT (= manuel) |
| Consent granularity 4-axis | — | ⚠ partial (= binaire on/off, pas 4 axes) |
| Transparency notice freshness | — | ✗ ABSENT |
| Breach 72h notification automation | — | ⚠ partial (= alert mais pas SLO timer) |

### 2.3 Mapping AS-IS : controls existants vs documentation

| Framework | Mapped (doc) | Evidence runtime | Gap |
|---|---|---|---|
| GDPR | 47 controls | ~15 evidences | **32 controls « documented only »** |
| SOC 2 | 64 controls | ~10 evidences | **54 controls « documented only »** |
| ISO 27001 | 93 controls | ~8 evidences | **85 controls « documented only »** |
| EU AI Act | 0 controls | 0 evidences | **38 controls absents** |
| HIPAA-ready | 0 controls | 0 evidences | **18 controls absents** (baseline) |
| CCPA | 0 controls | 0 evidences | **8 controls absents** |

**Total gap : 235 controls** à mapper vers des evidences ou à créer.

---

## § 3. Gap analysis : documentation ↔ enforcement

### 3.1 Le pattern "documented only" comme faussement-PASS

Aujourd'hui, le statut compliance EFE est :
- ✅ Threat model STRIDE rédigé (M14)
- ✅ Mappings ISO/SOC 2/GDPR rédigés (M14)
- ✅ Runbooks erasure/portability rédigés (M29)
- ❌ Mais **0 mécanisme automatisé** ne prouve qu'un control est appliqué runtime.

**Conséquence** : un control comme « ISO 27001 A.9.4.3 — password complexity » est mappé à `crates/efe-server/src/auth/password.rs` mais **personne ne vérifie quotidiennement** que la complexity policy est toujours en vigueur. Un refactor pourrait silencieusement la désactiver et le contrôle resterait « documented only » = PASS apparent.

C'est le pattern **faussement-PASS compliance** : éradiqué par la doctrine D-COMP-2.

### 3.2 Décomposition des gaps en 6 catégories

| Gap | Compte | Sprint cible |
|---|---|---|
| Controls GDPR sans evidence | 32 | D87 |
| Controls SOC 2 sans evidence | 54 | D87 |
| Controls ISO 27001 sans evidence | 85 | D87 |
| Controls AI Act absents | 38 | D86 |
| Controls HIPAA-ready absents | 18 | D86 |
| Controls CCPA absents | 8 | D86 |
| **Total** | **235** | **D86-D87** |

### 3.3 Strategie : "Evidence pyramid"

Pour ne pas écrire 235 scripts d'evidence, on consolide via une **pyramide d'evidence** :

- **Tier 1 (= 1 script par primitive)** : chaque primitive runtime (HMAC chain, tenant isolation, consent, encryption at rest, audit logging) a **un evidence script** qui couvre **plusieurs controls** simultanément. Ex : `comp_hmac_chain_evidence.py` couvre 12 controls (ISO A.12.4.2, SOC 2 CC7.1, GDPR Art. 32, etc.).

- **Tier 2 (= 1 script par doctrine D-COMP-*)** : 10 evidence scripts (un par doctrine D-COMP-1..10).

- **Tier 3 (= rest)** : controls spécifiques sans primitive partagée (= ~15 scripts).

**Total** : ~30 evidence scripts couvrant 235 controls.

### 3.4 Compte de lignes à ajouter

| Composant | Estimation |
|---|---|
| 6 framework mapping files | 6 × ~600 lignes = 3 600 lignes |
| 30 evidence scripts | 30 × ~150 lignes = 4 500 lignes |
| 18 SLOs YAML | 1 × ~150 lignes |
| 36 pin tests | 1 × ~700 lignes |
| 3 workflows CI | 3 × ~150 lignes |
| 5 runbooks | 5 × ~80 lignes = 400 lignes |
| Archi (ce document) | 1 × ~1800 lignes |
| Rust scaffolding (compliance registry) | ~600 lignes |
| **Total** | **~12 000 lignes** |

---

## § 4. Surface 1 — Framework registry + control mapping

### 4.1 Structure cible `docs/compliance/`

```text
docs/compliance/
├── registry.yaml                    # ← NEW : registre central des frameworks
├── gdpr.md                          # 47 controls (existant, à enrichir avec evidence refs)
├── soc2.md                          # 64 controls (existant, idem)
├── iso27001.md                      # 93 controls (existant, idem)
├── eu_ai_act.md                     # ← NEW : 38 controls
├── hipaa_ready.md                   # ← NEW : 18 controls baseline
├── ccpa.md                          # ← NEW : 8 controls (consumer rights)
├── threat_model.md                  # 63 threats STRIDE (existant)
├── controls_index.yaml              # ← NEW : index inverse evidence → controls
└── README.md                        # navigation
```

### 4.2 Schéma `registry.yaml`

```yaml
version: v0.8.0
generated_at: "2026-04-01T00:00:00Z"
frameworks:
  GDPR:
    file: docs/compliance/gdpr.md
    n_controls: 47
    applicable: 47
    not_applicable: 0
    last_review: "2026-04-01"
    dpo_signoff: "alice.dupont@efe.ai"
  SOC2:
    file: docs/compliance/soc2.md
    n_controls: 64
    applicable: 58           # 6 not applicable (= no physical office)
    not_applicable: 6
  ISO27001:
    file: docs/compliance/iso27001.md
    n_controls: 93
    applicable: 87
    not_applicable: 6
  EU_AI_ACT:
    file: docs/compliance/eu_ai_act.md
    n_controls: 38
    applicable: 38
    not_applicable: 0
  HIPAA_READY:
    file: docs/compliance/hipaa_ready.md
    n_controls: 18
    applicable: 18           # baseline only
    not_applicable: 0
  CCPA:
    file: docs/compliance/ccpa.md
    n_controls: 8
    applicable: 8
    not_applicable: 0
total_controls: 268
total_applicable: 256
```

### 4.3 Schéma de chaque control dans le fichier framework

```markdown
### GDPR Art. 17 — Right to erasure

| Champ | Valeur |
|---|---|
| Control ID | GDPR-Art17 |
| Description | Le data subject a le droit d'obtenir l'effacement de ses données personnelles sans délai injustifié (max 30 jours). |
| Applicable | Yes |
| Risk if absent | HIGH (= amende 4 % CA mondial) |
| EFE artefacts | `crates/efe-server/src/services/admin.rs::trigger_gdpr_art17_erasure`, `M29 § 7` |
| Evidence script | `eval/scripts/comp_gdpr_art17_erasure_evidence.py` |
| SLO | SLO-COMP-ERASURE-30D ≤ 30 days |
| Runbook | `docs/runbook/comp_erasure_failure.md` |
| Last verified | 2026-04-01 |
```

### 4.4 Schéma `controls_index.yaml`

```yaml
# Inverse mapping : evidence script → controls couverts.
evidences:
  comp_hmac_chain_evidence:
    file: eval/scripts/comp_hmac_chain_evidence.py
    controls:
      - GDPR-Art32     # security of processing
      - ISO-A.12.4.2   # protection of log information
      - ISO-A.12.4.3   # admin/operator logs
      - SOC2-CC7.1     # detection of security events
      - SOC2-CC7.2     # monitoring of security events
      - AI-ACT-Art12   # record-keeping
      - HIPAA-164.312.b # audit controls
  comp_consent_granularity_evidence:
    file: eval/scripts/comp_consent_granularity_evidence.py
    controls:
      - GDPR-Art6     # lawfulness
      - GDPR-Art7     # conditions for consent
      - GDPR-Art13    # transparency
      - CCPA-1798.100 # right to know
  # ... 28 autres evidences
```

### 4.5 Pin test du registry

```python
def test_comp_registry_yaml_consistent():
    """Le registry.yaml doit déclarer exactement les fichiers existants."""
    reg = _read_yaml(_REPO / "docs" / "compliance" / "registry.yaml")
    for fw_name, fw in reg["frameworks"].items():
        fpath = _REPO / fw["file"]
        assert fpath.exists(), f"{fw_name} : file {fw['file']} missing"
    # Sum applicable + not_applicable = n_controls per framework.
    for fw_name, fw in reg["frameworks"].items():
        assert fw["applicable"] + fw["not_applicable"] == fw["n_controls"]
```

---

## § 5. Surface 2 — Evidence runtime (= 1 script par primitive)

### 5.1 Inventaire des 30 evidence scripts

| # | Evidence script | Controls couverts | Doctrine |
|---|---|---|---|
| 01 | `comp_hmac_chain_evidence` | 7 | D-COMP-3 |
| 02 | `comp_tenant_isolation_evidence` | 5 | D-COMP-7 |
| 03 | `comp_consent_granularity_evidence` | 4 | D-COMP-9 |
| 04 | `comp_encryption_at_rest_evidence` | 4 | (transverse) |
| 05 | `comp_encryption_in_transit_evidence` | 4 | (transverse) |
| 06 | `comp_access_control_rbac_evidence` | 6 | (transverse) |
| 07 | `comp_password_complexity_evidence` | 3 | (transverse) |
| 08 | `comp_session_timeout_evidence` | 2 | (transverse) |
| 09 | `comp_mfa_enforcement_evidence` | 3 | (transverse) |
| 10 | `comp_audit_log_retention_evidence` | 5 | D-COMP-3 |
| 11 | `comp_backup_restore_evidence` | 4 | (transverse) |
| 12 | `comp_disaster_recovery_rpo_rto_evidence` | 3 | (transverse) |
| 13 | `comp_data_residency_evidence` | 3 | D-COMP-7 |
| 14 | `comp_subprocessor_inventory_evidence` | 4 | D-COMP-6 |
| 15 | `comp_breach_72h_evidence` | 2 | D-COMP-4 |
| 16 | `comp_breach_24h_containment_evidence` | 2 | D-COMP-4 |
| 17 | `comp_erasure_30d_evidence` | 3 | D-COMP-5 |
| 18 | `comp_portability_30d_evidence` | 2 | D-COMP-5 |
| 19 | `comp_dpia_freshness_evidence` | 4 | D-COMP-10 |
| 20 | `comp_ropa_freshness_evidence` | 3 | D-COMP-10 |
| 21 | `comp_transparency_notice_evidence` | 3 | D-COMP-9 |
| 22 | `comp_ai_act_risk_classification_evidence` | 5 | D-COMP-8 |
| 23 | `comp_ai_act_human_oversight_evidence` | 4 | D-COMP-8 |
| 24 | `comp_ai_act_dpo_signoff_evidence` | 2 | D-COMP-8 |
| 25 | `comp_vulnerability_management_evidence` | 4 | (transverse) |
| 26 | `comp_incident_response_drill_evidence` | 3 | D-COMP-4 |
| 27 | `comp_change_management_evidence` | 3 | (transverse) |
| 28 | `comp_vendor_risk_assessment_evidence` | 2 | D-COMP-6 |
| 29 | `comp_data_minimization_evidence` | 4 | (transverse) |
| 30 | `comp_purpose_limitation_evidence` | 3 | (transverse) |

**Total controls couverts** : **108** par 30 evidences (= ratio 3.6 controls / evidence en moyenne).
Restant à couvrir : 256 - 108 = 148 controls → couverts par cross-references vers documents existants (= cas où l'evidence est dans un autre crate, ex : M17 backup tests prouvent BCP/DR).

### 5.2 Pattern standard d'un evidence script

```python
#!/usr/bin/env python3
"""Compliance evidence : <description du control>.

Sprint **Φ-D-COMPLIANCE (D87)** — evidence #NN.
Cf. ``docs/EFE_COMPLIANCE_COMPLETION_ARCHITECTURE.md`` § 5.

Controls couverts (cf. docs/compliance/controls_index.yaml) :
  - <FW>-<ID>
  - ...

Modes :
  * `--mode synth_pass`  : self-test démontrant le control appliqué.
  * `--mode synth_fail`  : régression injectée, doit détecter.
  * `--mode live`        : evidence sur la prod (= SKIP en CI sandbox).
"""

import argparse, json, sys
from pathlib import Path


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["live", "synth_pass", "synth_fail"],
                    default="synth_pass")
    ap.add_argument("--out", type=Path,
                    default=Path("target/compliance/evidence_NN.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    if args.mode == "live":
        args.out.write_text(json.dumps({
            "slo_compliance": {
                "SLO-COMP-EVIDENCE-COVERAGE": {
                    "skip_reason": "Live evidence (DT-COMP-LIVE-*)",
                }
            }
        }, indent=2) + "\n")
        return 0

    # Run the check.
    passed, reason = run_check(args.mode)
    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-COMP-EVIDENCE-COVERAGE": {
                "measured_pass_count": 1 if passed else 0,
                "measured_expected_count": 1,
                "control_ids": ["<FW>-<ID>", "..."],
                "reason": reason,
            }
        }
    }, indent=2) + "\n")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
```

### 5.3 Évidence agrégée

Le bench `comp_evidence_aggregator.py` :
1. Lit `docs/compliance/controls_index.yaml`.
2. Vérifie que chaque evidence script référencé existe + invokable.
3. Pour chaque framework, calcule le ratio `controls_with_evidence / applicable_controls`.
4. Emit `SLO-COMP-EVIDENCE-COVERAGE = pass si ratio = 100 %`.

---

## § 6. Surface 3 — Audit chain compliance-grade

### 6.1 Réutilisation de la chaîne D14-bis

L'`audit_events` table existe déjà (M29 § 6.3, Streaming § 8). Les events
compliance-relevant s'y joignent via le canonical schema :

```text
audit_events(
  event_id          UUID v7
  ts_unix_ms        INT8
  action            TEXT          -- 'compliance.consent_change', 'compliance.erasure', etc.
  tenant_id         TEXT
  principal         TEXT
  payload_json      JSONB
  signature_hex     CHAR(64)      -- HMAC-SHA-256
  prev_signature_hex CHAR(64)
  deleted           BOOL DEFAULT false
)
```

### 6.2 Nouveaux event types compliance

| Action | Émetteur | Trigger |
|---|---|---|
| `compliance.consent_change_v2` | Mode-2/3/AL | tenant consent update (4-axis) |
| `compliance.erasure_request` | M29 | client clicks "delete my data" |
| `compliance.erasure_complete` | M29 | erasure cascade done |
| `compliance.portability_request` | M29 | client clicks "export" |
| `compliance.portability_complete` | M29 | export delivered |
| `compliance.breach_detected` | breach detector | alignment cone violation OR > N rate limit OR... |
| `compliance.breach_dpo_notified` | breach handler | DPO contacted |
| `compliance.breach_authority_notified` | breach handler | CNIL/etc. contacted |
| `compliance.breach_resolved` | breach handler | post-mortem complete |
| `compliance.dpia_generated` | DPIA tool | new catalog OR retune |
| `compliance.dpia_signed_off` | DPO | manual sign-off |
| `compliance.ropa_rebuilt` | ROPA tool | sub-processor change OR cron |
| `compliance.subprocessor_added` | admin | new LLM provider |
| `compliance.subprocessor_removed` | admin | provider sunset |
| `compliance.ai_act_reclassified` | admin | catalog risk level change |
| `compliance.residency_violation` | residency enforcer | cross-region attempt |

### 6.3 Replay quotidien

Workflow `comp-audit-replay.yml` :
- Cron 19:00 UTC.
- Filter `audit_events` where `action LIKE 'compliance.%'`.
- Replay HMAC chain (= `replay_label_chain.py` pattern from M24).
- Emit `SLO-COMP-AUDIT-CHAIN-DAILY` PASS si 100 % integrity.

### 6.4 Retention compliance

| Catégorie | Retention |
|---|---|
| `compliance.consent_change_v2` | 7 ans (= GDPR retention recommandée) |
| `compliance.erasure_*` | 7 ans (= preuve d'effacement, paradoxe résolu via tombstone) |
| `compliance.breach_*` | 10 ans (= GDPR Art. 33 + SOC 2) |
| `compliance.dpia_*` | 5 ans après dernière modification |
| `compliance.ropa_*` | 5 ans après dernière modification |
| `compliance.ai_act_*` | 10 ans (= AI Act Art. 18) |

Stockage : `audit_events` table + cold storage S3 après 90 jours.

---

## § 7. Surface 4 — Breach detection + 72h notification

### 7.1 Définition GDPR du breach

> Art. 4(12) : "personal data breach" = a breach of security leading to
> the accidental or unlawful destruction, loss, alteration, unauthorised
> disclosure of, or access to, personal data.

**3 types** : confidentialité (= leak), intégrité (= tampering), disponibilité (= loss/destruction).

### 7.2 Sources de détection automatique

| Source | Type | Action |
|---|---|---|
| Alignment cone violations massives | Confidentialité | Trigger triage |
| HMAC chain break detection | Intégrité | Trigger triage |
| Multi-replica audit divergence | Intégrité | Trigger triage |
| Cross-region routing detected | Confidentialité | Trigger triage |
| Tenant isolation breach (= adversarial test fails) | Confidentialité | Trigger triage |
| LLM provider 5xx > 50 % pendant 30 min | Disponibilité | Trigger triage |
| Backup verify fail | Disponibilité | Trigger triage |
| Sub-processor unexpected data flow | Confidentialité | Trigger triage |

### 7.3 Triage automatisé

```
[Detection event] ──┐
                    ▼
             ┌──────────────┐
             │ Triage rule  │
             │ → severity   │
             │ → category   │
             │ → est. count │
             └──────┬───────┘
                    │
                    ▼
             ┌──────────────┐
             │ Severity ≥ 3 │  yes → DPO notification (= GDPR Art. 33 timer starts)
             │ OR PII leak  │
             └──────┬───────┘
                    │ no
                    ▼
              [Audit log + ops alert only]
```

### 7.4 Notification DPO ≤ 72h

```python
class BreachHandler:
    async fn on_detection(event: DetectionEvent) -> Result<()>:
        # 1. Audit log first (= avoid race with shutdown).
        audit_emit("compliance.breach_detected", payload=event)
        # 2. Triage.
        breach = triage(event)
        if breach.severity < 3 and not breach.pii_leak:
            return Ok(())  # log only
        # 3. Start 72h timer.
        timer = BreachTimer::new(deadline=now + 72h)
        # 4. Notify DPO immediately.
        DPO::notify(breach)
        audit_emit("compliance.breach_dpo_notified", payload=breach)
        # 5. Containment.
        ContainmentRunner::start(breach, deadline=now + 24h)
        # 6. Authority notif (= CNIL, ICO, etc.) within timer.
        if breach.affects_high_risk_subjects():
            AuthorityNotifier::send(breach, timer)
        Ok(())
```

### 7.5 SLO-COMP-BREACH-NOTIFY-72H

Injection test : `comp_breach_72h_evidence.py --mode synth_fail` injecte
un breach + vérifie que la notification DPO simulée arrive ≤ 72h
(= mode mock, le timer est compressé).

### 7.6 Containment 24h

`comp_breach_24h_containment_evidence.py` :
1. Simule un breach.
2. Mesure le temps entre détection et "containment action" (= block tenant access, revoke keys, etc.).
3. SLO = ≤ 24h (en mode mock = ≤ 24 simulated hours).

---

## § 8. Surface 5 — Right to erasure + portability (Art. 17/20)

### 8.1 Réutilisation M29

M29 livre déjà l'infrastructure (cf. M29 § 7-8). Φ-D-COMPLIANCE ajoute :

- **SLO timer** : `SLO-COMP-ERASURE-30D` = ≤ 30 days from request to
  cascade complete. Mesuré via `audit_events` diff `erasure_request →
  erasure_complete`.

- **SLO timer** : `SLO-COMP-PORTABILITY-30D` = ≤ 30 days from request
  to ZIP delivered.

### 8.2 Cascade erasure invariants

L'erasure cascade doit toucher :

```
                     [Erasure request]
                            │
                            ▼
                     [Audit log start]
                            │
              ┌─────────────┴───────────────┐
              ▼                             ▼
       ┌──────────────┐              ┌──────────────┐
       │ Postgres     │              │ Redis        │
       │ - sessions   │              │ - cache      │
       │ - audit_pii  │              │ - rate_limit │
       └──────┬───────┘              └──────┬───────┘
              │                             │
              ▼                             ▼
       ┌──────────────┐              ┌──────────────┐
       │ Object store │              │ Cold storage │
       │ S3 - backups │              │ Glacier      │
       │ - logs       │              │ - archives   │
       └──────┬───────┘              └──────┬───────┘
              │                             │
              └─────────────┬───────────────┘
                            ▼
                    [Audit log end]
                    [SLO timer stop]
```

### 8.3 Pin test cascade integrity

`comp_erasure_30d_evidence.py` synth_pass :
1. Génère un fake tenant + sessions + backups synthétiques.
2. Trigger erasure request (= timer t=0).
3. Wait for `erasure_complete` event.
4. Verify : `t_complete - t_request ≤ 30 days` (= mock = ≤ 30 simulated days).
5. Verify : 0 PII residue dans les 4 layers (PG/Redis/S3/Glacier).

---

## § 9. Surface 6 — Sub-processor lifecycle

### 9.1 Définition

Sub-processor = tiers traitant des données pour le compte du tenant. Liste actuelle EFE :

| Sub-processor | Service | Région | Data type |
|---|---|---|---|
| OpenAI | LLM (gpt-4o-mini) | US | Query + anchors |
| Anthropic | LLM (Claude haiku) | US | Query + anchors |
| Google Vertex | LLM (Gemini Flash) | EU/US | Query + anchors |
| Mistral | LLM | EU | Query + anchors |
| Cohere | LLM | US | Query + anchors |
| Llama-host | Self-hosted | client choice | Query + anchors |
| Azure OpenAI | LLM | EU/US | Query + anchors |
| AWS Bedrock | LLM | EU/US | Query + anchors |
| Postgres (Aiven/RDS) | Storage | tenant.data_region | Sessions, audit |
| Redis (Upstash) | Cache | tenant.data_region | Cache, rate limit |
| S3 (AWS) | Object store | tenant.data_region | Backups |
| Datadog | Observability | EU/US | Logs, metrics |
| Cloudflare | CDN/WAF | global | TLS, rate limit |

**Inventaire automatique** : `comp_subprocessor_inventory_evidence.py`
lit `crates/efe-server/config/subprocessors.yaml` + `M29` registry +
émet le diff vs. la dernière publication trust-center.

### 9.2 Triggers de lifecycle

| Trigger | Action automatique |
|---|---|
| Nouveau provider ajouté | (a) DPIA update auto-régénérée. (b) Tenant notification 30 jours. (c) ROPA rebuild. (d) Audit log. |
| Provider retiré | (a) Migration plan (tenants notifiés). (b) Erasure des données chez le provider. (c) Audit log. |
| Provider region change | (a) Tenant.data_region match check. (b) Re-routing si conflit. (c) Audit log. |
| Provider security incident | (a) Breach handler triggered. (b) Tenants notifiés ≤ 72h si data exfiltrée. |

### 9.3 Notification tenant 30 jours

Avant tout ajout/retrait substantif :

```python
async fn notify_tenants(change: SubprocessorChange):
    notice = render_change_notice(change)
    for tenant in active_tenants():
        Email::send(tenant.dpo_email, notice)
    audit_emit("compliance.subprocessor_change_notified", change)
    schedule_apply_at(now + 30 days, change)
```

### 9.4 SLO-COMP-SUBPROCESSOR-INVENTORY

`comp_subprocessor_inventory_evidence.py` :
- Read `subprocessors.yaml`.
- Compare against `audit_events` (last `compliance.subprocessor_added/removed`).
- Verify inventory.last_updated ≤ 7 jours.
- Pass si ≤ 7 jours, fail sinon.

---

## § 10. Surface 7 — Data residency enforcement

### 10.1 Tenant config

```yaml
# crates/efe-server/config/tenants/<tenant_id>.yaml
tenant_id: tenant-acme
data_region: EU                    # EU / US / APAC / GLOBAL
allowed_providers:
  - mistral                        # EU
  - vertex_eu                      # EU
  - azure_eu                       # EU
  # Explicitly NOT openai (= US-only)
gdpr_dpa_signed: true
ai_act_risk_overrides: {}
```

### 10.2 Provider routing enforcement

Mode-2 + Mode-3 router pre-flight check :

```rust
fn validate_provider_for_tenant(
    provider: &str,
    tenant: &TenantConfig,
) -> Result<(), Status> {
    let provider_regions = PROVIDER_REGIONS.get(provider)
        .ok_or_else(|| Status::internal("unknown provider"))?;
    let allowed = match tenant.data_region.as_str() {
        "EU"     => provider_regions.contains(&"EU"),
        "US"     => provider_regions.contains(&"US"),
        "APAC"   => provider_regions.contains(&"APAC"),
        "GLOBAL" => true,
        other    => return Err(Status::invalid_argument(format!("unknown region {other}"))),
    };
    if !allowed {
        // Audit + fail-closed.
        audit_emit("compliance.residency_violation", payload!({
            "tenant_id": tenant.tenant_id,
            "provider": provider,
            "tenant_region": tenant.data_region,
            "provider_regions": provider_regions,
        }));
        return Err(Status::permission_denied(format!(
            "provider {provider} not allowed in region {}", tenant.data_region
        )));
    }
    Ok(())
}
```

### 10.3 Storage residency

Postgres / Redis / S3 : tenant data shards sont co-localisés avec `data_region`. Migration entre régions = explicit consent + audit.

### 10.4 SLO-COMP-RESIDENCY-LEAK

`comp_data_residency_evidence.py` (synth_pass + adversarial) :
- 4 scenarios : EU tenant + EU provider (OK), EU tenant + US provider (DENIED), GLOBAL tenant + any (OK), US tenant + EU provider (OK, surclassement).
- Verify: 0 cross-region leaks across all scenarios.

---

## § 11. Surface 8 — EU AI Act risk classification + oversight

### 11.1 Risk levels (Art. 6 AI Act)

| Level | Définition | Exemples EFE |
|---|---|---|
| **minimal** | Pas de risque pour droits fondamentaux | Spam classifier, content tagger |
| **limited** | Risque modéré ; transparence requise (Art. 50) | General-purpose chatbot, recommendation engine |
| **high** | Risque sur droits fondamentaux ; Annex III | Credit scoring, hiring, biométrique, justice |
| **unacceptable** | Interdit (Art. 5) | Social scoring, manipulation, real-time biometric |

### 11.2 Per-catalog classification

Chaque catalog (= modèle déployé) porte :

```yaml
# bench_baselines/v0.8.0/catalogs/<catalog_id>.yaml
catalog_id: credit-decision-v1
ai_act_risk_level: high
ai_act_annex: Annex_III_4_a       # = creditworthiness assessment
human_oversight_mode: mode3        # forced (= no Mode-1/2)
dpo_signoff:
  signer: alice.dupont@efe.ai
  date: 2026-04-15
  expires: 2027-04-15
transparency_notice:
  url: https://efe.ai/notices/credit-decision-v1.html
  language: ["fr", "en"]
  last_updated: "2026-04-15"
```

### 11.3 Runtime enforcement

```rust
fn route_to_mode(catalog: &Catalog, requested_mode: Mode) -> Result<Mode> {
    if catalog.ai_act_risk_level == "high" && requested_mode != Mode::Mode3 {
        audit_emit("compliance.ai_act_oversight_enforced", payload!({
            "catalog_id": catalog.id,
            "requested": requested_mode,
            "forced": "mode3",
        }));
        return Ok(Mode::Mode3);  // force Mode-3 for high-risk
    }
    if catalog.ai_act_risk_level == "unacceptable" {
        return Err(Status::permission_denied("unacceptable risk catalog"));
    }
    Ok(requested_mode)
}
```

### 11.4 DPO sign-off freshness

`comp_ai_act_dpo_signoff_evidence.py` :
- Read every catalog with `risk_level=high`.
- Check `dpo_signoff.expires > now` (= signature ≤ 1 year old).
- SLO = 100 % high-risk catalogs have valid sign-off.

### 11.5 Human oversight enforcement

`comp_ai_act_human_oversight_evidence.py` :
- For each `risk_level=high` catalog, verify the recent traffic was routed through Mode-3 (= via audit_events filter).
- SLO = 100 % high-risk requests used Mode-3.

---

## § 12. Surface 9 — Consent granularité + transparency

### 12.1 4 axes indépendants (D-COMP-9)

```yaml
# Consent record per tenant.
tenant_id: tenant-acme
consent_version: "v3"
last_updated: 2026-04-15T10:00:00Z
axes:
  analytics:              true       # opt-in pour analytics produit
  model_training:         false      # opt-out (default) pour fine-tuning AL
  marketing:              false      # opt-out (default)
  third_party_sharing:    false      # opt-out (default ; aucun sub-processor extra)
```

### 12.2 Enforcement runtime

- `analytics=false` → telemetry traces stripped des IDs personnels.
- `model_training=false` → tenant data exclu des AL retunes.
- `marketing=false` → tenant désactivé des notifications produit.
- `third_party_sharing=false` → seuls les sub-processors essentiels (LLM provider de routage) sont utilisés.

### 12.3 SLO-COMP-CONSENT-VERSION

Le consent record doit matcher la dernière `consent_policy_version` publiée. Si la policy v4 sort, le tenant doit re-consenter v4 sinon ses axes opt-in deviennent opt-out (= fail-safe).

`comp_consent_granularity_evidence.py` :
- Read consent records.
- Compare against `docs/legal/consent_policy.md` version.
- SLO = 100 % consent records match.

### 12.4 Transparency notice freshness

Pour chaque catalog `limited` ou `high` :

`comp_transparency_notice_evidence.py` :
- For each catalog, verify `transparency_notice.last_updated ≤ 30 days OR matches policy version`.
- SLO = 100 % notices fresh.

---

## § 13. Surface 10 — DPIA + ROPA automation

### 13.1 DPIA (GDPR Art. 35)

Data Protection Impact Assessment requis pour tout traitement à "high risk". Notre target = automatique pour chaque catalog `high-risk` ou nouveau.

**Template DPIA** (= `docs/compliance/dpia_template.md`) :

```text
## DPIA — Catalog <ID>

1. Description systématique du traitement
2. Évaluation de la nécessité + proportionnalité
3. Évaluation des risques pour les droits + libertés
4. Mesures de mitigation
5. Sign-off DPO
```

**Génération automatique** : `scripts/compliance/generate_dpia.py` lit
le catalog YAML + audit events récents + emit un DPIA Markdown signé HMAC.

### 13.2 ROPA (GDPR Art. 30)

Records of Processing Activities = inventaire de tous les traitements. Per-tenant :

```yaml
tenant_id: tenant-acme
ropa:
  - activity: "Mode-2 query routing"
    purpose: "Intent classification"
    categories: ["query text"]
    recipients: ["openai-EU"]
    retention: "7 days raw, 90 days hashed"
    transfers: ["EU-only"]
  - activity: "Active Learning retune"
    purpose: "Model improvement"
    categories: ["confirmed labels"]
    recipients: ["internal"]
    retention: "5 years"
    transfers: ["EU-only"]
  # 8 autres activités
```

**Génération automatique** : `scripts/compliance/generate_ropa.py` lit
les catalogs + sub-processors + génère le ROPA Markdown par tenant.

### 13.3 SLOs freshness

- `SLO-COMP-DPIA-FRESHNESS` : chaque catalog `high-risk` a un DPIA ≤ 6 mois.
- `SLO-COMP-ROPA-FRESHNESS` : chaque tenant a un ROPA ≤ 1 mois.

---

## § 14. SLO opposables Φ-D-COMPLIANCE (= 18 SLOs)

### 14.1 YAML `bench_baselines/v0.8.0/comp_slos.yaml`

```yaml
version: v0.8.0
domain: compliance
host_fingerprint_baseline: "PLACEHOLDER_compute_at_freeze"

slos:
  # ── Coverage (4) ─────────────────────────────────────────────────
  - id: SLO-COMP-GDPR-CONTROLS
    description: "100 % GDPR controls applicables mappés vers artefacts"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: comp_framework_mapping_audit

  - id: SLO-COMP-SOC2-CONTROLS
    description: "100 % SOC 2 TSC controls applicables mappés"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: comp_framework_mapping_audit

  - id: SLO-COMP-ISO27001-CONTROLS
    description: "100 % ISO 27001 Annex A controls applicables mappés"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: comp_framework_mapping_audit

  - id: SLO-COMP-AI-ACT-CONTROLS
    description: "100 % EU AI Act controls applicables mappés"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: comp_framework_mapping_audit

  # ── Evidence runtime (5) ─────────────────────────────────────────
  - id: SLO-COMP-EVIDENCE-COVERAGE
    description: "100 % controls applicables ont une evidence script"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: comp_evidence_aggregator

  - id: SLO-COMP-AUDIT-CHAIN-DAILY
    description: "HMAC chain replay ≥ 1× / jour sans broken_links"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: comp_audit_chain_daily

  - id: SLO-COMP-DPIA-FRESHNESS
    description: "Chaque catalog high-risk a un DPIA ≤ 6 mois"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: comp_dpia_freshness_evidence

  - id: SLO-COMP-ROPA-FRESHNESS
    description: "Chaque tenant a un ROPA ≤ 1 mois"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: comp_ropa_freshness_evidence

  - id: SLO-COMP-SUBPROCESSOR-INVENTORY
    description: "Sub-processor inventory ≤ 7 jours"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: comp_subprocessor_inventory_evidence

  # ── Breach + erasure (4) ─────────────────────────────────────────
  - id: SLO-COMP-BREACH-NOTIFY-72H
    description: "DPO notification ≤ 72h après détection"
    comparator: lte
    threshold_seconds: 259200.0   # 72h
    source_bench: comp_breach_72h_evidence

  - id: SLO-COMP-BREACH-CONTAIN-24H
    description: "Containment action ≤ 24h après détection"
    comparator: lte
    threshold_seconds: 86400.0    # 24h
    source_bench: comp_breach_24h_containment_evidence

  - id: SLO-COMP-ERASURE-30D
    description: "Erasure cascade ≤ 30 jours"
    comparator: lte
    threshold_seconds: 2592000.0  # 30 jours
    source_bench: comp_erasure_30d_evidence

  - id: SLO-COMP-PORTABILITY-30D
    description: "Portability export ≤ 30 jours"
    comparator: lte
    threshold_seconds: 2592000.0  # 30 jours
    source_bench: comp_portability_30d_evidence

  # ── Residency + AI Act (3) ───────────────────────────────────────
  - id: SLO-COMP-RESIDENCY-LEAK
    description: "0 cross-region leaks détectés"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: comp_data_residency_evidence

  - id: SLO-COMP-AI-ACT-HIGH-OVERSIGHT
    description: "100 % high-risk catalogs forcent Mode-3"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: comp_ai_act_human_oversight_evidence

  - id: SLO-COMP-AI-ACT-DPO-SIGNOFF
    description: "100 % high-risk catalogs ont DPO sign-off ≤ 1 an"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: comp_ai_act_dpo_signoff_evidence

  # ── Consent + transparency (2) ───────────────────────────────────
  - id: SLO-COMP-CONSENT-VERSION
    description: "Consent records match policy version"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: comp_consent_granularity_evidence

  - id: SLO-COMP-TRANSPARENCY-NOTICE
    description: "Transparency notices ≤ 30 jours OR policy-matched"
    comparator: eq
    threshold_pass_count: 1
    expected_count: 1
    source_bench: comp_transparency_notice_evidence
```

---

## § 15. Sécurité : tamper-evident audit + key rotation

### 15.1 HMAC key rotation 90 jours

Hérité de D-AL-10. Φ-D-COMPLIANCE l'étend :

- Rotation manuelle via `efe-admin rotate-audit-key --tenant <id>`.
- Audit chain post-rotation : tombstone event `compliance.hmac_key_rotated`
  marquant la frontière. Replay tools gèrent les 2 keys.

### 15.2 Tombstone-based GDPR Art. 17 chain preservation

Quand un tenant exécute Art. 17 erasure, on remplace le `payload_json`
par un tombstone : `{"erased": true, "erased_at": ts, "erasure_id": "..."}`.

Le `signature_hex` est **recalculé** au-dessus du tombstone payload,
puis stocké. Le chain integrity reste valide post-erasure (= verify_chain
ne fail pas), mais le contenu personnel a disparu.

### 15.3 Anti-replay attack

Chaque event porte `nonce` (8 bytes) + `ts_unix_ms`. Un attaquant
re-jouant un event ancien serait détecté par le `prev_signature` chain
plus le `nonce` unicity (= UNIQUE index PG).

---

## § 16. Matrice complète des artefacts Φ-D-COMPLIANCE (= 40)

| # | Artefact | Type | Sprint |
|---|---|---|---|
| 01 | Archi (ce document) | doc | D85 |
| 02 | SLO YAML `comp_slos.yaml` | config | D85 |
| 03 | `docs/compliance/registry.yaml` | config | D85 |
| 04 | `docs/compliance/controls_index.yaml` | config | D85 |
| 05 | `docs/compliance/gdpr.md` enrichi | doc | D86 |
| 06 | `docs/compliance/soc2.md` enrichi | doc | D86 |
| 07 | `docs/compliance/iso27001.md` enrichi | doc | D86 |
| 08 | `docs/compliance/eu_ai_act.md` NEW | doc | D86 |
| 09 | `docs/compliance/hipaa_ready.md` NEW | doc | D86 |
| 10 | `docs/compliance/ccpa.md` NEW | doc | D86 |
| 11 | `comp_framework_mapping_audit.py` | bench | D87 |
| 12 | `comp_evidence_aggregator.py` | bench | D87 |
| 13 | `comp_audit_chain_daily.py` | bench | D87 |
| 14 | `comp_hmac_chain_evidence.py` | evidence | D87 |
| 15 | `comp_tenant_isolation_evidence.py` | evidence | D87 |
| 16 | `comp_consent_granularity_evidence.py` | evidence | D87 |
| 17 | `comp_encryption_at_rest_evidence.py` | evidence | D87 |
| 18 | `comp_data_residency_evidence.py` | evidence | D87 |
| 19 | `comp_subprocessor_inventory_evidence.py` | evidence | D87 |
| 20 | `comp_breach_72h_evidence.py` | evidence | D88 |
| 21 | `comp_breach_24h_containment_evidence.py` | evidence | D88 |
| 22 | `comp_erasure_30d_evidence.py` | evidence | D88 |
| 23 | `comp_portability_30d_evidence.py` | evidence | D88 |
| 24 | `comp_dpia_freshness_evidence.py` | evidence | D88 |
| 25 | `comp_ropa_freshness_evidence.py` | evidence | D88 |
| 26 | `comp_transparency_notice_evidence.py` | evidence | D88 |
| 27 | `comp_ai_act_risk_classification_evidence.py` | evidence | D88 |
| 28 | `comp_ai_act_human_oversight_evidence.py` | evidence | D88 |
| 29 | `comp_ai_act_dpo_signoff_evidence.py` | evidence | D88 |
| 30 | `generate_dpia.py` | tool | D88 |
| 31 | `generate_ropa.py` | tool | D88 |
| 32 | Pin tests Φ-D-COMPLIANCE | test | D89 |
| 33 | Workflow `comp-contract.yml` | ci | D89 |
| 34 | Workflow `comp-bench.yml` | ci | D89 |
| 35 | Workflow `comp-audit-replay.yml` | ci | D89 |
| 36 | Runbook `comp_breach_response.md` | doc | D89 |
| 37 | Runbook `comp_erasure_failure.md` | doc | D89 |
| 38 | Runbook `comp_subprocessor_change.md` | doc | D89 |
| 39 | Runbook `comp_residency_violation.md` | doc | D89 |
| 40 | Runbook `comp_ai_act_reclassification.md` | doc | D89 |

---

## § 17. Plan d'implémentation D85 → D89

| Jour | Tâche | Artefacts livrés |
|---|---|---|
| **D85** | Archi + SLO YAML + registry.yaml + controls_index.yaml | 01-04 |
| **D86** | 6 framework mappings (3 enrichis + 3 nouveaux) | 05-10 |
| **D87** | 6 evidence scripts core + 2 aggregators + audit replay | 11-19 |
| **D88** | 10 evidence scripts spécifiques + DPIA + ROPA generators | 20-31 |
| **D89** | Pin tests + 3 workflows + 5 runbooks | 32-40 |

### 17.1 Critical path

```
D85 (registry) → D86 (mappings) → D87 (evidences core) → D88 (specific evidences) → D89 (CI)
                                                ↘
                                                  D88 partial (= DPIA/ROPA peuvent commencer)
```

### 17.2 Parallelisable

- D85 + D86 : registry indépendant des mappings → 2 contributeurs.
- D87 + D88 : evidence scripts par doctrine → 6 contributeurs.

---

## § 18. Risques + mitigations (24)

| # | Risque | Sévérité | Mitigation |
|---|---|---|---|
| R1 | Control "documented only" non détecté | HIGH | SLO-COMP-EVIDENCE-COVERAGE pin |
| R2 | Audit chain break sur events compliance | CRIT | Daily replay + alert |
| R3 | DPO non joignable < 72h | HIGH | Multi-channel notif (email + SMS + Slack) |
| R4 | Erasure cascade incomplète (S3 résidu) | CRIT | Pin test cascade + scan S3 résiduel |
| R5 | Sub-processor "silently added" | HIGH | Inventory daily diff + audit chain |
| R6 | Cross-region leak Mode-2 | HIGH | Provider routing pre-flight check |
| R7 | AI Act high-risk catalog non oversight | HIGH | Forced Mode-3 routing |
| R8 | DPO sign-off expiré | MED | Daily check + 90j alert |
| R9 | Consent policy version drift | MED | Policy version tracker |
| R10 | DPIA outdated (> 6 mois) | MED | Daily freshness check |
| R11 | ROPA outdated (> 1 mois) | MED | Daily freshness check |
| R12 | HMAC key compromis non rotated | CRIT | 90j rotation policy enforcée |
| R13 | Tombstone race vs replay | MED | Atomic replace via PG transaction |
| R14 | Breach detection false positive flood | MED | Triage rule tunable |
| R15 | Breach detection false negative | CRIT | Multi-source detection + adversarial drill |
| R16 | Provider region change non détecté | MED | Sub-processor inventory daily |
| R17 | Tenant data_region missing | HIGH | Default = "GLOBAL" + admin alert |
| R18 | Transparency notice traduction missing | LOW | i18n check |
| R19 | DPIA template drift entre catalogs | MED | Template versionné |
| R20 | Audit chain too large to replay | MED | Daily incremental + weekly full |
| R21 | Compliance reporting client demand non honorée | MED | Per-tenant PDF export auto |
| R22 | Auditor demande non documenté | LOW | Sentinel pin + Q1 review |
| R23 | EU AI Act amendments | MED | Quarterly review of `eu_ai_act.md` |
| R24 | CCPA → CPRA → CPPA evolution | LOW | Annual review |

---

## § 19. Décisions clés (22)

| # | Décision | Rationale |
|---|---|---|
| D1 | 6 frameworks ciblés (GDPR, SOC 2, ISO 27001, AI Act, HIPAA-ready, CCPA) | Couverture MVP + extensible |
| D2 | Evidence pyramide 30 scripts → 235 controls | Compression 7.8× via primitives partagées |
| D3 | HMAC chain compliance-grade = D14-bis + nouveaux event types | Réutilisation maximale |
| D4 | Breach notify 72h hard SLO | GDPR Art. 33 |
| D5 | Containment 24h hard SLO | Best practice + ISO 27001 A.16.1.5 |
| D6 | Erasure 30 days hard SLO | GDPR Art. 17 |
| D7 | Sub-processor 30 jours notice | GDPR DPA standard |
| D8 | Data residency strict (fail-closed) | GDPR Art. 44-50 |
| D9 | AI Act high-risk = Mode-3 forced | Art. 14 human oversight |
| D10 | Consent 4 axes indépendants | GDPR Art. 7 granularité |
| D11 | DPIA auto pour high-risk | Art. 35 |
| D12 | ROPA per-tenant rebuild monthly | Art. 30 |
| D13 | Tombstone pattern post-erasure | Préserve chain intégrity |
| D14 | HMAC key rotation 90j | Industry standard |
| D15 | 18 SLOs (= symétrie AL/Mode-3/Streaming) | Pattern doctrine |
| D16 | 36 pin tests + 8 evidence audits | Anti-régression |
| D17 | 5 runbooks operations | Coverage breach/erasure/sub-processor/residency/AI Act |
| D18 | 3 CI workflows | contract / bench / audit-replay |
| D19 | DT-COMP-* explicites (= 14) | Anti-Φ-D.A1 |
| D20 | Per-tenant DPO email + SMS | Multi-channel notif |
| D21 | Trust center publish externe | Transparence client |
| D22 | Quarterly review du registry | Conformité continue |

---

## § 20. Anti-régression (= 36 pin tests + 8 evidence audits)

### 20.1 — Pin tests structurels (= 18)

| # | Test |
|---|---|
| 1 | `test_comp_registry_yaml_consistent` |
| 2 | `test_comp_controls_index_yaml_valid` |
| 3 | `test_comp_slo_yaml_has_18_entries` |
| 4 | `test_comp_six_frameworks_documented` |
| 5 | `test_comp_evidence_scripts_exist` (= 30) |
| 6 | `test_comp_evidence_scripts_invokable` |
| 7 | `test_comp_evidence_scripts_emit_skip_in_live_mode` |
| 8 | `test_comp_runbooks_exist` (= 5) |
| 9 | `test_comp_workflows_exist` (= 3) |
| 10 | `test_comp_dt_tickets_documented` (= 14) |
| 11 | `test_comp_slos_mentioned_in_archi` |
| 12 | `test_comp_dpia_template_present` |
| 13 | `test_comp_ropa_template_present` |
| 14 | `test_comp_subprocessors_yaml_present` |
| 15 | `test_comp_consent_policy_versioned` |
| 16 | `test_comp_transparency_notice_template_present` |
| 17 | `test_comp_breach_response_playbook_present` |
| 18 | `test_comp_check_slo_supports_thresholds` |

### 20.2 — Pin tests sémantiques (= 18)

| # | Test |
|---|---|
| 19 | `test_comp_framework_mapping_audit_full_coverage` |
| 20 | `test_comp_evidence_aggregator_no_orphan_control` |
| 21 | `test_comp_audit_chain_daily_replays_clean` |
| 22 | `test_comp_hmac_chain_evidence_self_test` |
| 23 | `test_comp_tenant_isolation_5_scenarios` |
| 24 | `test_comp_consent_4_axes_independence` |
| 25 | `test_comp_data_residency_4_scenarios` |
| 26 | `test_comp_subprocessor_inventory_freshness_7d` |
| 27 | `test_comp_breach_72h_synth_pass` |
| 28 | `test_comp_breach_24h_containment_synth_pass` |
| 29 | `test_comp_erasure_30d_synth_pass` |
| 30 | `test_comp_portability_30d_synth_pass` |
| 31 | `test_comp_dpia_freshness_6m` |
| 32 | `test_comp_ropa_freshness_1m` |
| 33 | `test_comp_ai_act_high_risk_forces_mode3` |
| 34 | `test_comp_ai_act_dpo_signoff_within_1y` |
| 35 | `test_comp_transparency_notice_freshness_30d` |
| 36 | `test_comp_tombstone_preserves_chain_integrity` |

### 20.3 — Evidence audits (= 8 = par doctrine D-COMP-*)

| # | Audit | Doctrine |
|---|---|---|
| A1 | `audit_d_comp_1_coverage` | D-COMP-1 |
| A2 | `audit_d_comp_2_evidence_runtime` | D-COMP-2 |
| A3 | `audit_d_comp_3_audit_chain` | D-COMP-3 |
| A4 | `audit_d_comp_4_breach_72h` | D-COMP-4 |
| A5 | `audit_d_comp_5_erasure_30d` | D-COMP-5 |
| A6 | `audit_d_comp_6_subprocessor` | D-COMP-6 |
| A7 | `audit_d_comp_7_residency` | D-COMP-7 |
| A8 | `audit_d_comp_8_ai_act` | D-COMP-8 |

---

## § 21. Critères d'acceptation (DoD à 100 %)

```text
Φ-D-COMPLIANCE DoD =
   (18/18 SLOs parseables ; ≥ 14 PASS en CI sandbox + ≤ 4 SKIP-with-DT-COMP-*)
 ∧ (40/40 artefacts livrés : doc + config + 30 evidences + 3 CI + 5 runbooks)
 ∧ (36/36 pin tests verts en < 5 s)
 ∧ (8/8 evidence audits par doctrine verts)
 ∧ (Registry registry.yaml référence exactement les 6 fichiers framework existants)
 ∧ (Chaque control applicable a au moins 1 evidence script référencé)
 ∧ (Audit chain compliance self-test 100 events PASS + corruption détectée)
 ∧ (Breach 72h synth + breach 24h synth PASS)
 ∧ (Erasure 30j synth PASS + 0 résidu PII détecté)
 ∧ (Data residency 4 scenarios PASS, 0 cross-region leak)
 ∧ (AI Act 100 % high-risk catalogs forced Mode-3 + DPO signed)
 ∧ (Consent 4 axes test PASS + policy version match)
 ∧ (DPIA freshness ≤ 6m + ROPA freshness ≤ 1m sur fixtures)
 ∧ (Sub-processor inventory ≤ 7j fresh sur fixtures)
 ∧ (3 CI workflows YAML valides + 18 jobs cumulés)
 ∧ (Aucune régression sur les sprints précédents : AL/Mode-3/Streaming/M29)
```

### 21.1 Artefacts livrés (= 40/40)

| Catégorie | # | Détail |
|---|---|---|
| Doc + config | 04 | archi + SLO YAML + registry.yaml + controls_index.yaml |
| Mappings frameworks | 06 | gdpr, soc2, iso27001, ai_act, hipaa_ready, ccpa |
| Aggregators | 03 | framework_mapping_audit, evidence_aggregator, audit_chain_daily |
| Evidence scripts | 16 | hmac, tenant_isolation, consent, residency, subproc, breach×2, erasure×2, dpia, ropa, transparency, ai_act×3 |
| Tools | 02 | generate_dpia, generate_ropa |
| Pin tests | 01 | 36 tests |
| CI workflows | 03 | contract, bench, audit-replay |
| Runbooks | 05 | breach, erasure, subproc, residency, ai_act |
| **Total** | **40/40** | **100 % livré** |

---

## § 22. Hand-off & dépendances downstream

- **M29 (Data Rights)** : Φ-D-COMPLIANCE consomme M29 erasure/portability endpoints + ajoute SLO timers.
- **M14 (Compliance Extension)** : Φ-D-COMPLIANCE consomme M14 mappings + ajoute evidence runtime.
- **Streaming RPC (Φ-D-STREAM)** : Φ-D-COMPLIANCE réutilise l'audit chain replay.
- **AL (Φ-D-AL)** : Φ-D-COMPLIANCE étend SLO-AL-GDPR-CONSENT-COVERAGE avec 4-axis consent.
- **Mode-3** : Φ-D-COMPLIANCE force Mode-3 pour AI Act high-risk catalogs.
- **M25 Agent Runtime** : Φ-D-COMPLIANCE évalue les agent tools contre AI Act.
- **Trust Center (= M27 commercial edge)** : Φ-D-COMPLIANCE publie les frameworks + DPIA + ROPA externes.

---

## § 23. Annexes (A → I)

### A — Mapping SLO ↔ test ↔ workflow (cf. § 14 + § 20)
### B — Instruments Prometheus compliance (= 6 nouveaux)
### C — Schéma `compliance.<event>` payload templates
### D — DPIA template `docs/compliance/dpia_template.md`
### E — ROPA template `docs/compliance/ropa_template.md`
### F — Sub-processor change notice email template
### G — Transparency notice i18n keys
### H — Breach severity rubric (1-5)
### I — Liens : `EFE_M29_DATA_RIGHTS`, `EFE_COMPLIANCE_EXTENSION` (M14), `EFE_ACTIVE_LEARNING_COMPLETION`, `EFE_MODE3_MULTI_TURN_COMPLETION`, `EFE_STREAMING_RPC_COMPLETION`

---

## § 24. Defer Tickets `DT-COMP-*` (= 14 vrais SKIPs opposables)

Anti-Φ-D.A1 : tout SLO non testable en CI sandbox doit pointer vers un
défer-ticket nommé avec un blocker explicite.

| ID | SLO bloqué | Blocker | Workflow | Sortie |
|---|---|---|---|---|
| `DT-COMP-LIVE-DPO-EMAIL` | SLO-COMP-BREACH-NOTIFY-72H (live) | Réception réelle des emails DPO | `comp-bench.yml` | bench synth_pass simule 72h compressé |
| `DT-COMP-LIVE-AUTHORITY` | breach authority notif (live) | Intégration CNIL/ICO formelle | `comp-bench.yml` | mock notif PASS |
| `DT-COMP-LIVE-S3-RESIDUE-SCAN` | SLO-COMP-ERASURE-30D (prod) | Scan S3 buckets multi-region | `comp-bench.yml` | synth scan PASS |
| `DT-COMP-LIVE-DPIA-DPO-SIGN` | DPIA auto-régénération signée DPO réel | Workflow humain DPO | N/A | template signed mock |
| `DT-COMP-LIVE-ROPA-CROSSPROD` | ROPA cross-tenant pour reporting auditeur | Auditor portal | N/A | per-tenant ROPA generated |
| `DT-COMP-LIVE-SUBPROC-DIFF` | Sub-processor diff vs trust center | Trust center API | `comp-bench.yml` | local diff PASS |
| `DT-COMP-LIVE-RESIDENCY-PROD` | Provider region routing prod | Multi-region prod infra | `comp-bench.yml` | mock scenarios PASS |
| `DT-COMP-LIVE-AI-ACT-AUDIT` | AI Act conformity assessment by 3rd party | EU notified body | N/A | self-attestation OK |
| `DT-COMP-LIVE-CONSENT-PIPELINE` | Consent collection pipeline UI | Frontend integration | N/A | API contract PASS |
| `DT-COMP-LIVE-TRANSPARENCY-CDN` | Transparency notices publiés CDN | CDN integration | N/A | template versioned |
| `DT-COMP-LIVE-HMAC-ROTATION` | HMAC rotation automatisée prod | Key vault integration | `comp-audit-replay.yml` | manual rotation procedure |
| `DT-COMP-SOC2-TYPE2-AUDIT` | SOC 2 Type II annual audit | External auditor | N/A | Type I self-attestation |
| `DT-COMP-ISO27001-CERT` | ISO 27001 certification | External cert body | N/A | self-assessment alignment |
| `DT-COMP-HIPAA-BAA-SIGNED` | HIPAA Business Associate Agreement | Legal review | N/A | template ready |

### 24.1 Mapping inverse (= SLO → mode CI / mode production)

| SLO | Mode CI | Mode production |
|---|---|---|
| SLO-COMP-GDPR-CONTROLS | PASS (mapping audit) | PASS sur prod mappings |
| SLO-COMP-SOC2-CONTROLS | PASS | PASS, DT-COMP-SOC2-TYPE2-AUDIT pour cert |
| SLO-COMP-ISO27001-CONTROLS | PASS | PASS, DT-COMP-ISO27001-CERT pour cert |
| SLO-COMP-AI-ACT-CONTROLS | PASS | PASS, DT-COMP-LIVE-AI-ACT-AUDIT pour 3rd party |
| SLO-COMP-EVIDENCE-COVERAGE | PASS | PASS sur prod evidence runs |
| SLO-COMP-AUDIT-CHAIN-DAILY | PASS (self-test) | PASS sur prod audit_events |
| SLO-COMP-DPIA-FRESHNESS | PASS (fixtures) | DT-COMP-LIVE-DPIA-DPO-SIGN |
| SLO-COMP-ROPA-FRESHNESS | PASS (fixtures) | DT-COMP-LIVE-ROPA-CROSSPROD |
| SLO-COMP-SUBPROCESSOR-INVENTORY | PASS (local) | DT-COMP-LIVE-SUBPROC-DIFF |
| SLO-COMP-BREACH-NOTIFY-72H | PASS (synth compressed) | DT-COMP-LIVE-DPO-EMAIL |
| SLO-COMP-BREACH-CONTAIN-24H | PASS (synth compressed) | PASS sur drill réel |
| SLO-COMP-ERASURE-30D | PASS (synth compressed) | DT-COMP-LIVE-S3-RESIDUE-SCAN |
| SLO-COMP-PORTABILITY-30D | PASS (synth) | PASS sur drill réel |
| SLO-COMP-RESIDENCY-LEAK | PASS (4 scenarios) | DT-COMP-LIVE-RESIDENCY-PROD |
| SLO-COMP-AI-ACT-HIGH-OVERSIGHT | PASS (fixture) | PASS sur prod catalogs |
| SLO-COMP-AI-ACT-DPO-SIGNOFF | PASS (fixture) | PASS sur prod registry |
| SLO-COMP-CONSENT-VERSION | PASS (fixture) | DT-COMP-LIVE-CONSENT-PIPELINE |
| SLO-COMP-TRANSPARENCY-NOTICE | PASS (fixture) | DT-COMP-LIVE-TRANSPARENCY-CDN |

**Bilan attendu en CI** : 14 PASS + 4 SKIP-with-DT (= 0 faussement-PASS).
**Bilan attendu en production** : 18 PASS (= tous DT-* résolus).

---

**Fin du document Φ-D-COMPLIANCE Compliance Completion.**
