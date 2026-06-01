# Sprint M14 — Compliance + docs : architecture du freeze 1.0 GA

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit M13 (Bundles modèles : D20 sample + D26
> production multilingual + D27 CI auto-bundling, livré + audit pass 1
> — 7 défauts corrigés, 49 pin tests M13 + 92 aggregate GREEN).
> **M14 est le DERNIER sprint avant le freeze 1.0 GA.** Aucun nouveau
> code applicatif ne sera livré post-M14 ; le repo passe en mode
> *operate-only* (patches sécurité + bug-fix critique uniquement).
>
> **Pré-requis** : avoir lu :
> - [`PRODUCT_COMPLETION_ARCHITECTURE.md`](PRODUCT_COMPLETION_ARCHITECTURE.md)
>   (1 952 lignes, vue d'ensemble post-D40 GA)
> - [`EFE_BACKUP_EXTENSION_ARCHITECTURE.md`](EFE_BACKUP_EXTENSION_ARCHITECTURE.md)
>   §17 Compliance mapping (SOC 2 CC9.1, ISO 27001 A.17.1.3, NIS2
>   Art. 21, RGPD Art. 32) + `docs/compliance/dr_drill_quarterly_evidence.md`
>   (premier doc compliance livré, scope DR drill uniquement)
> - [`EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md`](EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md)
>   (M12, post-audit, Helm chart 1.0 GA-ready avec compliance labels
>   `compliance.efe.ai/framework="soc2,iso27001,nis2,rgpd"`)
> - [`EFE_BUNDLES_EXTENSION_ARCHITECTURE.md`](EFE_BUNDLES_EXTENSION_ARCHITECTURE.md)
>   (M13, post-audit, bundles signed Ed25519 + SBOM CycloneDX 1.5 +
>   OCI Artifacts publication via `bundle-release.yml`)
> - [`d13_observability_dashboards_architecture.md`](bench/d13_observability_dashboards_architecture.md)
>   (10 runbooks D13 déjà livrés sous `docs/runbooks/d13_*.md`)
> - [`d14bis_hmac_audit_chain_architecture.md`](bench/d14bis_hmac_audit_chain_architecture.md)
>   (HMAC tamper-evident audit log — base de l'attestation d'intégrité
>   pour SOC 2 CC4 + ISO A.12.4)
> - [`d11_per_tenant_isolation_architecture.md`](bench/d11_per_tenant_isolation_architecture.md)
>   (zero-trust multi-tenant — base SOC 2 CC6 + ISO A.5.15)
>
> **Mission M14** : transformer EFE d'un produit *fonctionnellement
> complet* (M13 livré) en un produit *audit-ready, freeze-ready, et
> handoff-ready*. Trois sprints :
>
> | Sprint | Phase | Jour | Livrable |
> |---|:---:|:---:|---|
> | **D40** | Φ-A | J155 | `docs/compliance/iso27001.md` (mapping Annex A → modules EFE) ; `docs/security/threat_model.md` (formalisé, consolidant les threat models scattered) ; SBOM endpoint `https://efe.ai/sbom/<image-digest>.json` (online-vérifiable) |
> | **D41** | Φ-B | J165 | `docs/compliance/soc2_evidence.md` (SOC 2 CC1-CC9 Trust Services Criteria coverage) ; `docs/compliance/gdpr_art32.md` (RGPD Art. 32 mesures techniques + organisationnelles) ; `docs/compliance/ai_act_annex_iv.md` (EU AI Act 2024/1689 Annex IV : technical documentation) ; `docs/compliance/nist_800_53.md` (NIST SP 800-53 r5 control mapping pour les déploiements US-fed) |
> | **D42** | Φ-C | J175 | `docs/RELEASE_NOTES_1.0.md` (full GA release notes, breaking changes, migration guide) ; **handoff doc équipe ops** (transition build→operate) ; **runbook 15+ scénarios** (extension des 10 D13 runbooks existants) ; 🔒 **FREEZE** signed + announced (mode operate-only post-J175) |
>
> **Pain points opérationnels résolus** :
>
> 1. **Compliance gap critique** : aujourd'hui les seules preuves
>    compliance écrites sont (a) le DR drill quarterly evidence
>    (D40 livré, scope étroit : continuité business uniquement), et
>    (b) des labels `compliance.efe.ai/*` sur le CronJob Helm M12.
>    Aucun document formel mappant les controls EFE → SOC 2 / ISO
>    27001 / GDPR / AI Act / NIS2 / NIST 800-53. Lors d'un audit
>    SOC 2 Type II, un auditeur pose 100+ questions structurées par
>    Trust Services Criteria ; sans `soc2_evidence.md`, l'équipe
>    devrait improviser → audit fail garanti.
> 2. **Threat model éparpillé** : threat models scattered dans M12 §8
>    (Helm chart), M13 §2 (bundles supply-chain), D11 §X (zero-trust
>    multi-tenant), D14-bis §Y (tamper-evidence), D16 §2 (supply-chain
>    ML). Aucun document unique consolidé. Auditeur SOC 2 demandant
>    "show me your threat model" → 5 docs partiels à corréler à la
>    main.
> 3. **AI Act 2024/1689 non-couvert** : règlement EU sur l'IA en
>    application 2026. EFE = système d'IA classified "limited risk"
>    (classification semantic) qui DOIT shipper l'Annex IV technical
>    documentation. Aujourd'hui : 0 % couverture.
> 4. **Pen-test manquant** : aucune attestation tier-3 d'un
>    pentester externe. Conditions d'achat enterprise (banques,
>    assurance, gov) exigent un rapport pen-test ≤ 12 mois.
> 5. **SBOM endpoint absent** : M13 produit SBOM CycloneDX 1.5 par
>    bundle (D26) + le helm chart M12 ship un SBOM image. Mais
>    aucun ENDPOINT public consultable où un consumer peut faire
>    `GET https://efe.ai/sbom/sha256:abc...json` pour vérifier le
>    SBOM associé à l'image qu'il pull. ROADMAP SLSA Level 3 +
>    Cyber Resilience Act EU 2024 exigent un SBOM endpoint en ligne.
> 6. **Release notes 1.0 incomplets** : aucun document `RELEASE_NOTES_1.0.md`
>    GA-ready. Customers vont demander "qu'est-ce qui change vs
>    0.x", "quelle est la migration path", "qu'est-ce qui est
>    breaking", "quelles sont les SLAs". Sans ce doc : adoption
>    customer freinée.
> 7. **Runbook coverage insuffisant** : 10 runbooks D13 (observability)
>    livrés ; mais 0 runbook pour D14 (Postgres backend), D15 (OIDC),
>    D16 (bundles), D17 (backup/DR), D18 (multi-replica), D21-22-27-40
>    (M12 Helm), D20-26-27 (M13 bundles). Quand une alerte fire
>    pour un de ces sprints, l'équipe ops ne sait pas par où commencer.
> 8. **Handoff team formalisé manquant** : transition build → operate
>    requiert un doc qui dit qui possède quoi, qui appelle qui en
>    incident, où sont les credentials, comment on rotate les
>    secrets, etc. Aujourd'hui = tribal knowledge des founding
>    engineers.
> 9. **Customer DDQ (Due Diligence Questionnaire) non-prêt** :
>    enterprise procurement envoie typiquement 200-500 questions
>    sécurité/compliance. Sans pré-réponses formalisées, sales cycle
>    rallongé +30j minimum.
> 10. **Freeze procedure inexistante** : qui peut merger post-1.0 ?
>     Quels types de PR sont acceptés (security, critical bug-fix,
>     compliance) ? Comment on bump une version de patch ?
>
> **Note importante sur le périmètre** : M14 ne livre **AUCUN nouveau
> code applicatif**. Toutes les livraisons sont *documentaires* +
> *opérationnelles* (CI workflows pour SBOM endpoint, scripts pour
> compliance evidence collection). Le repo entre en freeze post-J175.

---

## Table des matières

1. [Pain points + objectifs M14](#1-pain-points--objectifs-m14)
2. [Compliance landscape EFE 1.0 GA](#2-compliance-landscape-efe-10-ga)
3. [Threat model formalisé (D40)](#3-threat-model-formalisé-d40)
4. [ISO 27001:2022 Annex A mapping (D40)](#4-iso-270012022-annex-a-mapping-d40)
5. [SBOM endpoint architecture (D40)](#5-sbom-endpoint-architecture-d40)
6. [Pen-test program (D40 plan)](#6-pen-test-program-d40-plan)
7. [SOC 2 Trust Services Criteria evidence (D41)](#7-soc-2-trust-services-criteria-evidence-d41)
8. [RGPD / GDPR Art. 32 evidence (D41)](#8-rgpd--gdpr-art-32-evidence-d41)
9. [AI Act 2024/1689 Annex IV (D41)](#9-ai-act-20241689-annex-iv-d41)
10. [NIST SP 800-53 r5 mapping (D41)](#10-nist-sp-800-53-r5-mapping-d41)
11. [Customer DDQ pre-fill template (D41)](#11-customer-ddq-pre-fill-template-d41)
12. [Release Notes 1.0 (D42)](#12-release-notes-10-d42)
13. [Handoff doc équipe ops (D42)](#13-handoff-doc-équipe-ops-d42)
14. [Runbook inventory (D42)](#14-runbook-inventory-d42)
15. [Freeze procedure (D42)](#15-freeze-procedure-d42)
16. [Continuous compliance post-GA](#16-continuous-compliance-post-ga)
17. [Audit-readiness checklist](#17-audit-readiness-checklist)
18. [Test plan M14](#18-test-plan-m14)
19. [SLOs M14-COMP-*](#19-slos-m14-comp)
20. [Risk register & open questions](#20-risk-register--open-questions)
21. [Appendix A — control matrix (5 frameworks × 30 controls)](#21-appendix-a--control-matrix-5-frameworks--30-controls)
22. [Appendix B — runbook template](#22-appendix-b--runbook-template)
23. [Appendix C — DDQ skeleton (200+ questions)](#23-appendix-c--ddq-skeleton-200-questions)
24. [Appendix D — release notes 1.0 skeleton](#24-appendix-d--release-notes-10-skeleton)
25. [Appendix E — pin tests inventory M14](#25-appendix-e--pin-tests-inventory-m14)
26. [Appendix F — glossaire](#26-appendix-f--glossaire)

---

## 1. Pain points + objectifs M14

### 1.1 Statut quo `docs/compliance/` post-M13

Inventory des fichiers compliance existants :

```
docs/compliance/
├── dr_drill_quarterly_evidence.md          # D40 livré (DR scope only, ~110 lignes)
├── dr-drill-outcome-2.0.0.schema.json      # JSON Schema for outcome.json
└── helloworld-1.0.0.schema.json            # Hello-World workflow schema
```

**Diagnostic** :
- 1 doc compliance, scope DR drill uniquement.
- 0 mapping ISO 27001 Annex A (114 controls).
- 0 mapping SOC 2 CC1-CC9 (60+ Common Criteria points).
- 0 mapping GDPR Art. 32 (4 mesures techniques + organisationnelles).
- 0 mapping AI Act Annex IV (8 sections technical doc obligatoires).
- 0 mapping NIST 800-53 r5 (300+ controls).
- 0 threat model formalisé.
- 0 SBOM endpoint.

### 1.2 Statut quo `docs/runbooks/`

```
docs/runbooks/
├── d13_audit_unknown_principal.md
├── d13_auth_failure_spike.md
├── d13_catalog_version_explosion.md
├── d13_classify_drift.md
├── d13_high_error_rate.md
├── d13_high_latency_p99.md
├── d13_log_pipeline_lag.md
├── d13_obs_cardinality_bomb.md
├── d13_runbook_missing.md
└── d13_trace_sampling.md
```

**Diagnostic** : 10 runbooks, tous D13 (observability). Total 580
lignes. Aucun runbook pour D14 / D15 / D16 / D17 / D18 / D21-22-27-40
(M12) / D20-26-27 (M13). M14 D42 doit livrer **15+ NEW runbooks**
couvrant les sprints non-couverts.

### 1.3 Pourquoi M14 maintenant et pas plus tôt ?

Les compliance docs ne peuvent pas être écrits avant que les
features qu'ils couvrent existent. Sequence logique :

```
D11-D18  → features sécurité de base (auth, RBAC, audit, multi-tenant)
D40       → DR drill compliance (premier doc compliance, scope étroit)
M11       → security architecture (DESIGN, pas evidence)
M12       → Helm hardening (compliance labels appliqués)
M13       → bundles signed (supply chain hardened)
M14       → docs compliance (CONSOLIDATION + audit-ready)
```

M14 est le sprint où on **écrit la doc qui prouve** que les features
livrées en M11-M13 satisfont les controls externes.

### 1.4 Objectifs D40 (Sprint Φ-A, ~10 jours d'effort)

| ID | Objectif | Métrique mesurable |
|---|---|---|
| **O-D40-1** | ISO 27001:2022 Annex A coverage ≥ 80 % (controls "Applicable" ; "Not Applicable" justifiés) | `docs/compliance/iso27001.md` map les 114 controls Annex A → "Applicable + Implemented" / "Applicable + Compensating" / "Not Applicable + Reason" |
| **O-D40-2** | Threat model formalisé consolidant 5 sources | `docs/security/threat_model.md` consolide M12 §8 + M13 §2 + D11 + D14-bis + D16 §2 dans un seul doc avec STRIDE classification |
| **O-D40-3** | SBOM endpoint live + queryable | `https://efe.ai/sbom/<image-digest>.json` retourne CycloneDX 1.5 SBOM en HTTP/200 < 100 ms p99 |
| **O-D40-4** | Pen-test program lancé (RFP issued) | RFP envoyé ≥ 3 vendors (NCC Group / Cure53 / Trail of Bits-class), schedule pen-test post-D42 freeze |

### 1.5 Objectifs D41 (Sprint Φ-B, ~10 jours d'effort)

| ID | Objectif | Métrique mesurable |
|---|---|---|
| **O-D41-1** | SOC 2 evidence pack 8 CC | `docs/compliance/soc2_evidence.md` documente CC1-CC9 (60+ criteria points) + Q&A pre-fill pour audit |
| **O-D41-2** | GDPR Art. 32 4 mesures | `docs/compliance/gdpr_art32.md` : pseudonymisation, chiffrement, intégrité+confidentialité, restoration capability + DPIA pour traitements à risque |
| **O-D41-3** | AI Act Annex IV 8 sections | `docs/compliance/ai_act_annex_iv.md` : description système IA, intended purpose, performance metrics, human oversight, post-market monitoring, etc. |
| **O-D41-4** | NIST 800-53 r5 mapping | `docs/compliance/nist_800_53.md` : 17 control families × ~20 controls/family = 300+ controls mappés (Applicable/Compensating/N/A) |
| **O-D41-5** | Customer DDQ pre-fill | Template `docs/compliance/ddq_prefill.md` avec 200+ questions/réponses pré-écrites |

### 1.6 Objectifs D42 (Sprint Φ-C, ~10 jours d'effort)

| ID | Objectif | Métrique mesurable |
|---|---|---|
| **O-D42-1** | Release Notes 1.0 GA-ready | `docs/RELEASE_NOTES_1.0.md` : breaking changes, migration guide, SLAs, support matrix, EOL policy |
| **O-D42-2** | Handoff doc | `docs/HANDOFF_OPS.md` : ownership matrix, escalation tree, secrets rotation calendar, on-call rotation template |
| **O-D42-3** | 15+ NEW runbooks | `docs/runbooks/` couvre D14 / D15 / D16 / D17 / D18 / D21 / D22 / D27 / D40 / D20 / D26 (M13) (≥ 15 fichiers nouveaux) |
| **O-D42-4** | Freeze announce + protected branch | GitHub branch protection `main` : require 2 reviewers + CODEOWNERS + status checks ; FREEZE notice in CONTRIBUTING.md ; tag `v1.0.0` signed |

---

## 2. Compliance landscape EFE 1.0 GA

### 2.1 Frameworks ciblés

| Framework | Scope géographique | Mandatory ? | Coverage M14 |
|---|---|---|---|
| **SOC 2 Type II** (AICPA) | US enterprise, global B2B | Mandatory pour vendre à enterprise US | CC1-CC9 evidence pack (D41) |
| **ISO 27001:2022** (ISO/IEC) | Global | Mandatory pour vendre EU/APAC enterprise | Annex A 114 controls (D40) |
| **GDPR / RGPD** (EU 2016/679) | EU + transferts données | Mandatory si data EU | Art. 32 mesures (D41) + DPIA |
| **NIS2** (EU 2022/2555) | EU critical entities | Conditional (clients critiques) | Art. 21.2 covered via D40 backup |
| **EU AI Act** (EU 2024/1689) | EU + AI systems | Mandatory 2026 | Annex IV technical documentation (D41) |
| **NIST 800-53 r5** | US federal contractors | Conditional (FedRAMP path) | 17 control families (D41) |
| **HIPAA** (US health) | US health data | NOT TARGETED — scope health-data |
| **FedRAMP** | US federal cloud | NOT TARGETED — phase 2.0 |
| **CCPA / CPRA** (California) | US California | Conditional (covered via GDPR-compatible posture) |
| **PCI DSS 4.0** | Card payments | NOT TARGETED — EFE n'handle pas de PAN |
| **CRA** (EU 2024 Cyber Resilience Act) | EU products with digital elements | Mandatory 2027 | SBOM endpoint (D40) + vulnerability handling |

### 2.2 Pourquoi pas HIPAA / FedRAMP / PCI ?

- **HIPAA** : EFE ne traite pas explicitement de données de santé.
  Customers santé doivent BAA (Business Associate Agreement) +
  configurer leur tenant en mode `EFE_PII_HASH=true` (déjà livré
  D13). Pas de scope EFE-as-vendor HIPAA-certified en 1.0.
- **FedRAMP** : exige une infrastructure dédiée (GovCloud) +
  audit DoD-class. Out-of-scope 1.0 ; positioned for 2.0 GA.
- **PCI DSS** : EFE n'est jamais sur le path d'un PAN (Primary
  Account Number). Customer responsability via tokenization
  pre-EFE. Documenté dans `compliance/pci_scope_exclusion.md`.

### 2.3 Mapping framework → docs M14

```
docs/
├── security/
│   ├── threat_model.md                  # D40 — STRIDE consolidé
│   ├── pentest_2026q3_report.md         # post-D42 — placeholder
│   └── vulnerability_disclosure.md      # D40 — coordinated VDP
├── compliance/
│   ├── iso27001.md                      # D40 — Annex A 114 controls
│   ├── soc2_evidence.md                 # D41 — CC1-CC9
│   ├── gdpr_art32.md                    # D41 — Art. 32 mesures
│   ├── gdpr_dpia.md                     # D41 — DPIA template
│   ├── ai_act_annex_iv.md               # D41 — 8 sections
│   ├── nist_800_53.md                   # D41 — 17 families
│   ├── nis2_art21.md                    # D40 — already partial via D40 backup
│   ├── cra_sbom_endpoint.md             # D40 — Cyber Resilience Act EU
│   ├── ddq_prefill.md                   # D41 — 200+ Q/A
│   ├── pci_scope_exclusion.md           # D40 — why no PCI
│   ├── continuous_compliance.md         # D42 — quarterly reviews
│   ├── dr_drill_quarterly_evidence.md   # ALREADY LIVRÉ (D40 backup)
│   └── *.schema.json                    # JSON schemas existants
├── runbooks/
│   ├── d13_*.md                         # 10 runbooks ALREADY LIVRÉS
│   ├── d14_postgres_failover.md         # NEW D42
│   ├── d14_pg_pool_exhausted.md         # NEW D42
│   ├── d15_oidc_jwks_stale.md           # NEW D42
│   ├── d15_tenant_onboarding_failed.md  # NEW D42
│   ├── d16_bundle_signature_invalid.md  # NEW D42
│   ├── d16_bundle_load_timeout.md       # NEW D42
│   ├── d17_backup_failed.md             # NEW D42
│   ├── d17_dr_drill_rto_breach.md       # NEW D42
│   ├── d18_pod_unable_to_acquire_lease.md  # NEW D42
│   ├── d18_replicas_below_minimum.md    # NEW D42
│   ├── d21_networkpolicy_strict_blocks.md  # NEW D42
│   ├── d22_mtls_cert_renewal_failed.md  # NEW D42
│   ├── d27_bundle_reload_hook_failed.md # NEW D42
│   ├── d40_dr_drill_quarterly_failed.md # NEW D42
│   ├── d26_bundle_pull_oci_failed.md    # NEW D42
│   ├── m12_helm_upgrade_rollback.md     # NEW D42
│   ├── m13_signing_key_rotation.md      # NEW D42
│   └── general_incident_playbook.md     # NEW D42 — IR top-level
├── RELEASE_NOTES_1.0.md                  # D42
├── HANDOFF_OPS.md                        # D42
└── PRODUCT_COMPLETION_ARCHITECTURE.md    # ALREADY LIVRÉ (post-D40 vue)
```

Nombre total NEW docs M14 : **~35 fichiers** (~2 500 lignes M14 + 6 000+ lignes
de compliance/runbooks).

---

## 3. Threat model formalisé (D40)

### 3.1 Mission

Consolider 5 threat models scattered into ONE authoritative document
`docs/security/threat_model.md`, en utilisant la framework **STRIDE**
(Spoofing / Tampering / Repudiation / Information Disclosure / Denial
of Service / Elevation of Privilege).

### 3.2 Sources existantes à consolider

| Source | Scope | # threats | Categories |
|---|---|---|---|
| M12 §8 (Helm chart) | K8s manifests | 12 | Spoofing K8s identity, RBAC escalation, NP bypass |
| M13 §2 (Bundles) | Supply chain ML | 10 (T-16-*) + 10 (T-M13-*) | Tampering weights, Repudiation publish |
| D11 (multi-tenant) | Cross-tenant isolation | 8 | Information Disclosure cross-tenant |
| D14-bis (HMAC audit) | Audit log integrity | 6 | Tampering audit, Repudiation actions |
| D15 (OIDC) | Authn/Authz | 7 | Spoofing tokens, Elevation via aud claim |
| D16 §2 (ML supply chain) | Model load path | 10 (T-16-*) | Tampering, Information Disclosure |
| **TOTAL consolidé** | | **63 threats** | All 6 STRIDE categories |

### 3.3 Schéma cible `threat_model.md`

```markdown
# EFE 1.0 GA — Formal Threat Model

## 1. Scope + assumptions
## 2. System decomposition (boundary diagram)
## 3. Trust boundaries
## 4. Asset inventory
## 5. Threats by STRIDE category
   ### 5.1 Spoofing (S)
   ### 5.2 Tampering (T)
   ### 5.3 Repudiation (R)
   ### 5.4 Information Disclosure (I)
   ### 5.5 Denial of Service (D)
   ### 5.6 Elevation of Privilege (E)
## 6. Threat × Mitigation matrix (63 threats × controls)
## 7. Residual risks (accepted + reasoned)
## 8. Re-evaluation schedule (quarterly)
```

### 3.4 STRIDE categorization (extrait — 63 threats)

| ID | Source sprint | STRIDE | Threat | Mitigation |
|---|:---:|:---:|---|---|
| **TM-S-01** | D11 | S | Tenant A spoofs JWT claim `tenant_id=B` | OIDC issuer validation D15 + per-tenant key D11 |
| **TM-S-02** | D15 | S | Stolen JWT replayed | `jti` claim + replay cache (5 min TTL) |
| **TM-S-03** | D14 | S | Postgres credentials exfiltrated | K8s Secret + external-secrets operator (Tier 3) |
| **TM-T-01** | D14-bis | T | Audit log row deleted by admin | HMAC chain + Object Lock S3 (7-yr retention) |
| **TM-T-02** | D16 | T | Bundle weights swapped on disk | Per-file SHA-256 in manifest + Ed25519 sig |
| **TM-T-03** | M13 D27 | T | OCI artifact tag rewritten by registry | Cosign sign-blob + Rekor transparency log |
| **TM-T-04** | M12 D27 | T | Helm hook Job replaced mid-upgrade | `hook-delete-policy` + RBAC scoped (M12-A8 fix) |
| **TM-R-01** | D11 | R | Cross-tenant action attributed wrongly | `tenant_id` propagated via grpc context + audit row |
| **TM-R-02** | D14-bis | R | Auditor claims log was tampered | HMAC chain proves byte-exact + Object Lock |
| **TM-I-01** | D11 | I | Tenant A reads tenant B's catalog versions | NetworkPolicy strict (M12 D21) + RBAC per-tenant |
| **TM-I-02** | D15 | I | OIDC issuer leaks JWKS publicly | TLS-only fetch + JWKS cache + IP allow-list |
| **TM-I-03** | D13 | I | Cardinality bomb leaks tenant identifiers | M12 audit fix + label allowlist |
| **TM-I-04** | D40 | I | Backup S3 bucket world-readable | IAM Policy explicit deny + Object Lock |
| **TM-D-01** | D9 | D | Sustained gRPC load saturates pod | HPA D9 + rate limit M11 + circuit breaker |
| **TM-D-02** | D14 | D | Postgres pool exhausted | Pool sizing D14 + connection limits + RWO mode |
| **TM-D-03** | D17 | D | Backup CronJob runs forever | activeDeadlineSeconds (D40 audit M7-D40-A6) |
| **TM-D-04** | D27 (M12) | D | Bundle reload hook stalled | timeout=300s + retries=3 (M12 D27) |
| **TM-D-05** | M13 D27 | D | bundle-release CI spam | concurrency cancel-in-progress=false (T-M13-4) |
| **TM-E-01** | D11 | E | Tenant A escalates to admin role | RBAC pinned per tenant + JWT `aud` strict |
| **TM-E-02** | M12 D21 | E | Pod escapes network namespace | NetworkPolicy strict + seccompProfile RuntimeDefault |
| **TM-E-03** | M13 D27 | E | CI runner takeover via fork PR | `if: github.repository == 'efe-project/efe'` strict |
| **TM-E-04** | D27 (M12) | E | RBAC dependency missing → Job runs as default SA | `helm.sh/hook: post-install,post-upgrade` (M12-A8) |

| **TM-S-04** | M13 D27 | S | Attacker pushes malicious bundle by spoofing CI identity | OIDC token-based GH Actions auth + `if: github.repository == 'efe-project/efe'` strict ; cosign keyless requires verified GH OIDC |
| **TM-S-05** | M12 D27 | S | Helm hook Job impersonates efe-server SA | RBAC scoped Role + RoleBinding (M12 D27) ; `automountServiceAccountToken: false` on default SA |
| **TM-S-06** | D14 | S | Postgres replica claims primary identity | mTLS between replicas + cluster CA + `pg_hba.conf` cert verification |
| **TM-S-07** | D17 | S | Backup CronJob impersonated by malicious K8s admin | ServiceAccount per CronJob + IRSA / Workload Identity + Object Lock prevents post-write tamper |
| **TM-T-05** | D14 | T | Catalog row updated post-Register without audit | DB triggers + HMAC chain row per write (D14-bis) |
| **TM-T-06** | M11 | T | Rate limit counter manipulation | Per-tenant in-memory counters + reset on pod restart (DoS-resistant by design) |
| **TM-T-07** | M12 D40 | T | DR drill outcome.json tampered post-write | S3 Object Lock COMPLIANCE 7-yr + signed evidence pack |
| **TM-T-08** | D13 | T | Prometheus metrics scrape spoofed | NetworkPolicy strict (M12 D21) — only monitoring ns can scrape |
| **TM-T-09** | M13 D26 | T | SBOM components list manipulated post-publish | SBOM included in bundle, hash in manifest, manifest signed Ed25519 |
| **TM-T-10** | M14 D40 | T | SBOM endpoint S3 object overwritten | IAM Policy + Object Lock + CDN immutable cache 1-yr |
| **TM-R-03** | D11 | R | Tenant denies a Register call they made | gRPC method call + JWT subject + tenant_id all in audit row |
| **TM-R-04** | D17 | R | Backup admin denies running a destructive op | CronJob name + activeDeadlineSeconds + audit row in HMAC chain |
| **TM-R-05** | M13 D27 | R | CI claims to have signed bundle but didn't | Rekor transparency log entry queryable by any third party |
| **TM-I-05** | D14 | I | Postgres backup leaks PII via SQL dump | PII hash D13 (`EFE_PII_HASH=true`) + backup encryption at rest |
| **TM-I-06** | M11 | I | Stack trace leaks internal file paths | Production builds strip debug info ; `RUST_LOG=info` not `debug` |
| **TM-I-07** | D13 | I | OTel spans contain raw user input | PII scrubber in trace span attributes + sampling 5 % default |
| **TM-I-08** | M12 | I | Helm values leak secrets via `helm get values` | `existingSecretName` pattern (D14 / D15 / D16) — values reference Secret name not content |
| **TM-I-09** | M13 D20 | I | Bundle README.md leaks proprietary info | Public sample bundles only ; CI license_gate for redistribution |
| **TM-I-10** | M14 D40 | I | Threat model leaks attacker playbook | `docs/security/threat_model.md` is public + abstracted (no specific exploit code) |
| **TM-D-06** | D15 | D | Sustained OIDC discovery requests DoS issuer | JWKS cache 5 min TTL + circuit breaker + grace stale window |
| **TM-D-07** | M11 | D | Slow client holds gRPC stream open | Per-stream timeout + max concurrent streams = 1024 (D9) |
| **TM-D-08** | D16 | D | Bundle load consumes entire memory | `loadTimeoutS=60` + `EFE_BUNDLE_REQUIRE_SBOM` + size validation pre-load |
| **TM-D-09** | M13 D26 | D | OCI registry pull bandwidth exhausts pod | Pull init container + ReadOnlyMany PVC sharing + pull pre-warm |
| **TM-D-10** | M14 D40 | D | SBOM endpoint scraped repeatedly | CDN edge cache + WAF rate limit per IP + signed URLs (optional) |
| **TM-E-05** | D14-bis | E | HMAC key disclosure → audit forgery | Key rotation quarterly + HMAC chain prevents historical rewrite |
| **TM-E-06** | M12 D22 | E | mTLS client cert stolen → service impersonation | Cert duration 8760h + renewBefore 720h + revocation list |
| **TM-E-07** | M13 D27 | E | GH Secret read access via repo write privilege | `environment: production` gates + workflow_dispatch protection rules + 2-reviewer rule on workflow PRs |
| **TM-E-08** | D17 | E | Backup PVC mounted as RW elsewhere | PVC `accessModes: [ReadOnlyMany]` post-write + IAM policy explicit deny |
| **TM-E-09** | D40 | E | DR drill namespace persists with privileges | Namespace deleted post-drill + RBAC `kubectl auth can-i` audit |
| **TM-E-10** | M14 D42 | E | Post-freeze unauthorized merge | Branch protection 2 reviewers + CODEOWNERS + signed commits + status checks |
| **TM-S-08** | D40 | S | Drill operator SA token leaked → impersonation | Short-lived ServiceAccount tokens (15-min) + RBAC scoped to drill namespace |
| **TM-S-09** | D9 | S | HPA target metric tampered to force scale | Prometheus auth + read-only metrics SA + alerting on scale anomalies |
| **TM-T-11** | D11 | T | Tenant config mutation without audit trail | All `Register` / `Evolve` go through D14-bis HMAC chain |
| **TM-T-12** | M12 D40 | T | Helm values.yaml swapped at deploy time | Helm post-install hook validates `compliance.efe.ai/framework` label set |
| **TM-T-13** | M13 D26 | T | Multilingual eval fixture tampered → false F1 | `tests/fixtures/multilingual_eval.jsonl` git-tracked + signed in release |
| **TM-R-06** | M14 D42 | R | Engineer denies merging post-freeze PR | Signed commits + reviewer attestation + audit log |
| **TM-I-11** | M14 D40 | I | Pen-test report leaks vulnerability pre-fix | Pen-test report under NDA 90-day + sanitized public version |
| **TM-D-11** | M14 D40 | D | Pen-test causes production incident | Pen-test scoped to staging + opt-in production blast-radius window |
| **TM-E-11** | M14 D40 | E | Pen-test finding exploited before fix | 7-day SLA for Critical findings + auto-revert on detection |
| **TM-S-10** | M14 D41 | S | Customer DDQ submitted with false EFE answers | Sales team only uses canonical pre-fill ; legal review for custom answers |
| **TM-T-14** | M14 D41 | T | Audit evidence collection altered before SOC 2 | Evidence pack on Object Lock + hash chain + auditor RO IAM |

**Total : 63 threats consolidated across 6 STRIDE categories**, with
documented mitigations + residual risk acceptance. Full doc at
`docs/security/threat_model.md` (~900 lignes).

### 3.5 Threat × Mitigation matrix (sample row)

| Threat | Mitigations en place | Reliance | Residual risk |
|---|---|:---:|---|
| **TM-T-02** Bundle weights swapped | (1) SHA-256 per file in manifest ; (2) Ed25519 sig over manifest ; (3) cosign on OCI artifact ; (4) Rekor transparency log ; (5) trust-store with grace period | High | Low — 4 trust roots distincts ; compromise simultané requis |
| **TM-D-02** Postgres pool exhausted | (1) `poolMaxConnections=8` ; (2) `poolAcquireTimeoutMs=5000` ; (3) circuit breaker ; (4) HPA scaling | Medium | Medium — burst traffic > 8 concurrent ops/pod can backpressure ; runbook `d14_pg_pool_exhausted.md` (D42) |

### 3.6 Re-evaluation cadence

- **Trimestrielle** : security team review threat model post-each
  sprint (audit findings → new threats).
- **Annuelle** : full STRIDE re-decomposition, system boundary
  diagram updated.
- **Ad-hoc** : new feature shipping → mandatory threat model
  update PR before merge to main (CODEOWNERS enforced).

---

## 4. ISO 27001:2022 Annex A mapping (D40)

### 4.1 Mission

Mapper les **114 controls** de l'Annex A (4 thèmes : Organizational,
People, Physical, Technological) vers les modules EFE existants.

### 4.2 Structure de `docs/compliance/iso27001.md` (~900 lignes)

```markdown
# ISO 27001:2022 Annex A Control Mapping for EFE 1.0 GA

## Statement of Applicability (SoA)
## A.5 Organizational controls (37 controls)
   ### A.5.1 Information security policies
   ### A.5.2 Information security roles and responsibilities
   ...
## A.6 People controls (8 controls)
## A.7 Physical controls (14 controls)
## A.8 Technological controls (34 controls)
## Implementation evidence per control
## Compensating controls
## Not Applicable controls + reasons
```

### 4.3 SoA extract (10 sample controls)

| Control | Title | Status | Implementation evidence |
|---|---|:---:|---|
| **A.5.1** | Information security policies | ✅ Implemented | `SECURITY.md` + `docs/compliance/*` |
| **A.5.7** | Threat intelligence | ✅ Implemented | `docs/security/threat_model.md` (D40) |
| **A.5.15** | Access control | ✅ Implemented | OIDC D15 + RBAC D11 + `docs/security/access_control_policy.md` |
| **A.5.30** | ICT readiness for business continuity | ✅ Implemented | D17 backup + D40 DR drill quarterly + Helm M12 D40 CronJob |
| **A.7.4** | Physical security monitoring | 🟡 Compensating | Cloud-managed (AWS/GCP/Azure SOC 2 attestations) |
| **A.8.5** | Secure authentication | ✅ Implemented | OIDC D15 + JWT validation + JWKS rotation |
| **A.8.7** | Protection against malware | ✅ Implemented | Distroless image + Polaris (M12) + `runAsNonRoot` |
| **A.8.16** | Monitoring activities | ✅ Implemented | Prometheus + Grafana D13 + 10 alerts D13 + 10 runbooks |
| **A.8.24** | Use of cryptography | ✅ Implemented | Ed25519 bundles M13 + HMAC audit D14-bis + mTLS M12 D22 + TLS 1.3 |
| **A.8.32** | Change management | ✅ Implemented | Git-based + branch protection + `.github/workflows/*` |

| **A.5.4** | Management responsibilities | ✅ Implemented | CODEOWNERS + RACI matrix `HANDOFF_OPS.md` |
| **A.5.6** | Contact with authorities | ✅ Implemented | DPO contact + EU DPA notification procedure (Art. 33) |
| **A.5.9** | Inventory of information assets | ✅ Implemented | SBOM endpoint (D40) + asset inventory in `HANDOFF_OPS.md` |
| **A.5.10** | Acceptable use of information | ✅ Implemented | Customer DPA + AUP in customer onboarding |
| **A.5.12** | Classification of information | ✅ Implemented | Tenant data classification + PII flag (D13) |
| **A.5.16** | Identity management | ✅ Implemented | OIDC issuers config (D15) — externally managed |
| **A.5.17** | Authentication information | ✅ Implemented | JWT bearer + JWKS rotation (D15) |
| **A.5.18** | Access rights | ✅ Implemented | RBAC D11 + per-tenant + per-RPC scope claim |
| **A.5.19** | Information security in supplier relationships | ✅ Implemented | DPA template + sub-processor list + SCC for EU transfers |
| **A.5.21** | ICT supply chain | ✅ Implemented | SBOM CycloneDX 1.5 (M13) + signed bundles (Ed25519 M13) + cosign image |
| **A.5.23** | Information security for use of cloud services | ✅ Implemented | Cloud provider attestations + IRSA / Workload Identity + IAM least privilege |
| **A.5.24** | Information security incident management planning | ✅ Implemented | `general_incident_playbook.md` (D42) + on-call rotation |
| **A.5.27** | Learning from information security incidents | ✅ Implemented | Post-mortem template + audit pass per sprint (M11+M12+M13 audit fixes) |
| **A.5.28** | Collection of evidence | ✅ Implemented | HMAC chain audit (D14-bis) + Object Lock 7-yr (D17) |
| **A.5.31** | Legal, statutory, regulatory and contractual requirements | ✅ Implemented | `docs/compliance/*.md` + DPO + legal review quarterly |
| **A.5.33** | Protection of records | ✅ Implemented | Object Lock COMPLIANCE 7-yr (D17) + HMAC chain |
| **A.5.34** | Privacy and protection of PII | ✅ Implemented | PII hash (D13) + GDPR Art. 32 doc (D41) + DPIA template |
| **A.6.3** | Information security awareness, education and training | ✅ Implemented | Operator guide + 25+ runbooks + onboarding doc |
| **A.7.1** | Physical security perimeters | 🟡 Compensating | Cloud provider data center attestations |
| **A.7.2** | Physical entry | 🟡 Compensating | Cloud provider |
| **A.7.3** | Securing offices, rooms and facilities | 🟡 Compensating | Cloud provider |
| **A.8.1** | User endpoint devices | 🟡 Compensating | Customer responsibility (BYOD policy) |
| **A.8.2** | Privileged access rights | ✅ Implemented | RBAC D11 admin claim + 2-reviewer rule (post-freeze) |
| **A.8.3** | Information access restriction | ✅ Implemented | Per-tenant RBAC + ResourceQuota Tier 2 |
| **A.8.6** | Capacity management | ✅ Implemented | HPA D9 + ResourceQuota + HPA scaling alerts |
| **A.8.10** | Information deletion | ✅ Implemented | Tenant offboarding procedure + GDPR Art. 17 right to erasure |
| **A.8.11** | Data masking | ✅ Implemented | PII hash D13 + tokenization customer-side |
| **A.8.12** | Data leakage prevention | ✅ Implemented | NetworkPolicy strict (M12 D21) + egress allowlist + audit log |
| **A.8.13** | Information backup | ✅ Implemented | D17 4-CronJob backup pipeline |
| **A.8.14** | Redundancy of information processing facilities | ✅ Implemented | D18 multi-replica + leader election + PDB |
| **A.8.15** | Logging | ✅ Implemented | D14-bis HMAC chain + Loki + 7-yr retention |
| **A.8.20** | Networks security | ✅ Implemented | NetworkPolicy strict + mTLS M12 D22 + TLS 1.3 |
| **A.8.21** | Security of network services | ✅ Implemented | gRPC over HTTP/2 + TLS 1.3 + mTLS optional |
| **A.8.22** | Segregation of networks | ✅ Implemented | NetworkPolicy strict + per-tenant namespace (Tier 2) |
| **A.8.23** | Web filtering | 🟡 Compensating | Customer NetworkPolicy / egress proxy |
| **A.8.25** | Secure system development life cycle | ✅ Implemented | Pin tests + audit pass per sprint + CODEOWNERS |
| **A.8.26** | Application security requirements | ✅ Implemented | OWASP Top 10 review + threat model |
| **A.8.27** | Secure system architecture and engineering principles | ✅ Implemented | Architecture docs `docs/EFE_*_EXTENSION_ARCHITECTURE.md` |
| **A.8.28** | Secure coding | ✅ Implemented | Rust safety + clippy + cargo audit + Polaris GREEN |
| **A.8.29** | Security testing in development and acceptance | ✅ Implemented | Pin tests CI + Polaris CI gate + helm-unittest + ct install matrix |
| **A.8.30** | Outsourced development | ✅ Implemented | Vendor security assessment for sub-processors |
| **A.8.31** | Separation of development, test and production environments | ✅ Implemented | Helm 3 values profiles : default / staging / prod |
| **A.8.33** | Test information | ✅ Implemented | Test fixtures use synthetic data ; no production PII in tests |

(+ 60 autres controls dans le doc complet `docs/compliance/iso27001.md`)

### 4.4 Statement of Applicability (SoA) summary

| Theme | # Total | Implemented | Compensating | N/A |
|---|---:|---:|---:|---:|
| **A.5 Organizational** | 37 | 31 | 4 | 2 |
| **A.6 People** | 8 | 5 | 2 | 1 |
| **A.7 Physical** | 14 | 0 | 14 | 0 |
| **A.8 Technological** | 34 | 30 | 3 | 1 |
| **Total** | **93** | **66 (71 %)** | **23 (25 %)** | **4 (4 %)** |

> **Audit gate** : SoA coverage ≥ 80 % "Applicable" (Implemented +
> Compensating combined). EFE 1.0 GA target **96 %** (89 / 93).
> Wait — 93 because some controls require organizational maturity
> (e.g., legal entity + insurance) that's customer-side.

### 4.5 Compensating controls — pattern

Pour les controls "Physical" (data center physical access, etc.),
EFE relies on the cloud provider's attestations (AWS SOC 2 Type II,
GCP ISO 27001, Azure FedRAMP). Documented in `iso27001.md §A.7`
with a per-CSP attestation reference table.

### 4.6 Non-Applicable controls

4 controls explicitly N/A :

| Control | Why N/A |
|---|---|
| A.5.32 | *Intellectual property rights* — EFE is OSS Apache-2.0, no proprietary IP claims to enforce against customers |
| A.6.2 | *Terms and conditions of employment* — EFE is software, not employer to customers |
| A.7.10 | *Storage media* — Cloud-managed, no physical media handled |
| A.8.34 | *Protection of information systems during audit testing* — N/A pre-GA ; will become Applicable post-pen-test (D40 plan) |

---

## 5. SBOM endpoint architecture (D40)

### 5.1 Mission

Live HTTP endpoint where consumers can fetch the CycloneDX 1.5 SBOM
for any EFE artifact (image, bundle, helm chart) by digest.

### 5.2 Endpoint contract

```
GET https://efe.ai/sbom/<algorithm>:<digest>.json

Response 200 :
  Content-Type: application/vnd.cyclonedx+json
  ETag: "<digest>"
  Cache-Control: public, max-age=31536000, immutable
  X-EFE-Sbom-Spec: cyclonedx-1.5
  Body: <CycloneDX 1.5 JSON>

Response 404 :
  Body: {"error":"sbom_not_found", "digest":"<input>"}

Response 410 :
  Body: {"error":"sbom_recalled", "advisory_url":"https://..."}
```

### 5.3 Supported digest algorithms

- `sha256:` — primary, all artifacts
- `sha512:` — future, for FIPS-conformant deployments

### 5.4 Storage

```
                   ┌────────────────────────────┐
                   │   CI workflow              │
                   │   (release.yml /           │
                   │    bundle-release.yml /    │
                   │    helm-publish.yml)       │
                   └─────────┬──────────────────┘
                             │
                             │  on each release :
                             │   1. compute artifact digest
                             │   2. upload SBOM to S3 :
                             │      s3://efe-sbom-prod/<algo>/<digest>.json
                             │
                             ▼
                   ┌────────────────────────────┐
                   │   AWS S3 bucket            │
                   │   efe-sbom-prod            │
                   │   - Object Lock COMPLIANCE │
                   │   - 7-year retention       │
                   │   - public-read via CDN    │
                   └─────────┬──────────────────┘
                             │
                             ▼
                   ┌────────────────────────────┐
                   │   CloudFront CDN           │
                   │   efe.ai/sbom/*            │
                   │   - 1-year cache TTL       │
                   │   - signed URLs (optional) │
                   │   - geo-distributed        │
                   └────────────────────────────┘
```

### 5.5 CI workflow integration

Trois workflows uploadent au S3 bucket :
- `release.yml` (efe-server image)
- `bundle-release.yml` (M13 bundles)
- `helm-publish.yml` (M12 chart)

Chaque step `upload-sbom-to-cdn` :

```yaml
- name: Upload SBOM to public CDN
  run: |
    DIGEST=$(cosign verify --output-attestations <ref> | jq -r '...')
    aws s3 cp ./SBOM.json \
        "s3://efe-sbom-prod/sha256/${DIGEST}.json" \
        --content-type "application/vnd.cyclonedx+json" \
        --cache-control "public, max-age=31536000, immutable" \
        --metadata "spec=cyclonedx-1.5,uploaded_at=$(date -u +%FT%TZ)"
    # Invalidate CDN cache for this digest (idempotent)
    aws cloudfront create-invalidation \
        --distribution-id ${{ secrets.CDN_DIST_ID }} \
        --paths "/sbom/sha256/${DIGEST}.json"
```

### 5.6 SBOM index endpoint

Beyond per-digest fetch, an INDEX endpoint for discoverability :

```
GET https://efe.ai/sbom/index.json

Response 200 :
  {
    "version": "1.0",
    "updated_at": "2026-04-30T12:00:00Z",
    "artifacts": [
      {
        "type": "image",
        "name": "efe-server",
        "version": "1.0.0",
        "digest": "sha256:abc...",
        "sbom_url": "https://efe.ai/sbom/sha256:abc....json",
        "signed_url": "https://efe.ai/sbom/sha256:abc....json.sig"
      },
      ...
    ]
  }
```

Auto-regenerated nightly by CronJob `regenerate-sbom-index.yml`.

### 5.7 Threat model

| Threat | Mitigation |
|---|---|
| Attacker uploads malicious SBOM at known digest | S3 IAM Policy : write only via CI OIDC (no human IAM) ; Object Lock COMPLIANCE prevents overwrite |
| MITM on SBOM fetch | TLS 1.3 + HSTS + Public Key Pinning (CT log monitored) |
| SBOM signature stripped | Companion `.json.sig` file (cosign) referenced in INDEX |
| DDoS on endpoint | CloudFront edge caching + AWS WAF rate limit |

### 5.8 Cyber Resilience Act (CRA) alignment

EU CRA 2024 (effective 2027-12) requires :
- SBOM published machine-readable (✅ CycloneDX 1.5 JSON)
- Vulnerability disclosure pathway (✅ `docs/security/vulnerability_disclosure.md`)
- Security update mechanism (✅ Helm `helm upgrade` rolling)
- 5-year support window post-EOL (✅ documented in RELEASE_NOTES_1.0.md)

---

## 6. Pen-test program (D40 plan)

### 6.1 Pen-test target scope

- **External pen-test** : 1 week, black-box, against staging cluster
  exposed via test ingress.
- **Internal pen-test** : 1 week, gray-box, with read-access to repo
  + service accounts inside cluster.
- **Threat-led pen-test** : 1 week, red team scenario from
  `threat_model.md` top-10 STRIDE threats.

### 6.2 Vendor selection criteria

| Criterion | Required ? |
|---|:---:|
| OSCP / OSCE / GWAPT certs in team | ✅ |
| ML supply-chain pen-test experience | ✅ (rare — narrows to ~5 vendors) |
| EU GDPR-compatible reporting (no data exfil to US) | ✅ |
| Provides remediation guidance | ✅ |
| Re-test included in price | ✅ |
| Clean-room report (no creative writing) | ✅ |

### 6.3 Vendor shortlist

- NCC Group (UK) — strong ML/AI security practice
- Cure53 (DE) — top-tier OSS audits (Mozilla, Tor, etc.)
- Trail of Bits (US) — supply chain pen-test specialty
- Doyensec (US) — AppSec + DevSecOps focus

### 6.4 Schedule

```
J155 (D40)     : RFP issued (3-vendor shortlist)
J165 (D41)     : Vendor selected, contract signed
J175 (D42)     : Repo freeze ; pen-test starts on FROZEN code
J180           : Pen-test report received
J185           : Findings triaged + remediation PRs (security only post-freeze)
J190           : Re-test
J195           : Public attestation in `pentest_2026q3_report.md`
```

### 6.5 Findings triage matrix

| Severity | SLA fix | Override allowed ? |
|---|---|---|
| Critical | 7 days | NEVER (auto-revert release) |
| High | 30 days | Security team only |
| Medium | 90 days | Engineering lead |
| Low | Next minor | Default |
| Informational | Documented | None |

### 6.6 Vulnerability disclosure (VDP)

`docs/security/vulnerability_disclosure.md` :
```
- Coordinated disclosure : 90 days standard, negotiable
- PGP key for security@efe.ai (4096-bit RSA, valid 2026-2031)
- Hall of Fame for responsible reporters
- safe harbor : no legal action against good-faith research
- Bug bounty : (post-1.0 GA) HackerOne private program
```

---

## 7. SOC 2 Trust Services Criteria evidence (D41)

### 7.1 Mission

Document `docs/compliance/soc2_evidence.md` (~1 200 lignes) covering
the 5 Trust Services Categories :

- **Security (CC1-CC9)** — common to all SOC 2
- **Availability (A1)** — operational uptime
- **Confidentiality (C1)** — data protection at rest+transit
- **Processing Integrity (PI1)** — completeness + accuracy
- **Privacy (P1-P9)** — PII handling

EFE 1.0 GA targets **Security + Availability + Confidentiality**.
Privacy + Processing Integrity deferred to post-GA tier 2.

### 7.2 CC1-CC9 inventory

| Category | Title | Sub-criteria | Evidence sources EFE |
|---|---|:---:|---|
| **CC1** | Control Environment | 5 | Code of conduct + CONTRIBUTING + CODEOWNERS + role-based git access |
| **CC2** | Communication and Information | 3 | Slack ops + on-call rotation + status.efe.ai + incident channels |
| **CC3** | Risk Assessment | 4 | `threat_model.md` (D40) + quarterly review + risk register (this doc §20) |
| **CC4** | Monitoring Activities | 2 | Prometheus + 10 D13 alerts + 15+ D42 runbooks + Loki + OTel D13-bis |
| **CC5** | Control Activities | 3 | Helm M12 controls + bundle signing M13 + audit log D14-bis |
| **CC6** | Logical and Physical Access | 8 | OIDC D15 + RBAC D11 + Helm SecurityContext + NetworkPolicy strict M12 D21 |
| **CC7** | System Operations | 5 | M12 chart + D17 backup + D40 quarterly DR drill + monitoring D13 |
| **CC8** | Change Management | 1 | Git + branch protection + 92 pin tests + audit pass per sprint |
| **CC9** | Risk Mitigation | 2 | D17 backup + D40 DR drill + Object Lock 7-year |

### 7.3 Evidence pack structure

For each CC point, the doc includes :

```markdown
## CC6.1 — Logical access security

**Control objective** : The entity implements logical access security
software, infrastructure, and architectures over protected information
assets.

**EFE evidence** :

  1. **OIDC authentication** (D15) — JWT validation per request,
     issuer allow-list, JWKS rotation, replay protection (jti).
     Evidence : `crates/efe-server/src/auth/oidc.rs` +
     `docs/bench/d15_onboarding_idp_architecture.md`.
  2. **Per-tenant RBAC** (D11) — `tenant_id` claim propagated
     through gRPC context, resource isolation enforced at handler
     level. Evidence : `crates/efe-server/src/handlers/*.rs` +
     pin tests `test_d11_per_tenant_isolation.py`.
  3. **Pod-level controls** (M12) — `securityContext.runAsNonRoot=true`,
     `readOnlyRootFilesystem=true`, `allowPrivilegeEscalation=false`,
     `capabilities.drop=ALL`. Polaris GREEN gate enforces.
     Evidence : `deploy/helm/efe/values.yaml` + `.polaris.yaml`.
  4. **Network policy strict** (M12 D21, opt-in) — default-deny +
     tenant whitelist. Evidence :
     `deploy/helm/efe/templates/d21-networkpolicy-strict.yaml`.

**Test of design (audit period 2026-04-01 to 2026-09-30)** :

  - 100 % of pods spawned in audit period had `runAsNonRoot=true`
    (verified via Polaris CI gate).
  - 0 escapes from per-tenant context (verified via D11 pin tests
    + production audit log review).
  - JWKS rotation cadence : 5min cache TTL × 4 issuers = 20 min
    average revalidation.

**Test of operating effectiveness** :

  - Sample 25 production requests over 6 months → all had valid
    OIDC JWT (validated via audit log query, see Annex E).
  - 0 P0 incidents related to access control during audit period.

**Auditor talking points** :

  - "How is privilege managed ?" → JWT scope claim + per-tenant
    RBAC + cluster-level RBAC.
  - "What happens if JWT signing key is compromised ?" → JWKS
    rotation procedure (`docs/runbooks/d15_oidc_jwks_stale.md`)
    + immediate revocation of issuer in `oidc-issuers.json`.
```

### 7.3-bis Sample evidence per CC point (extended)

#### CC1.1 — Demonstrates Commitment to Integrity and Ethical Values

**EFE evidence** :
  - `CONTRIBUTING.md` declares conduct standards
  - `SECURITY.md` declares secure-by-default posture
  - PR template requires "I have run pin tests + reviewed for
    security" checklist
  - Audit pass per sprint (M11+M12+M13) demonstrates remediation
    culture
  - All audit findings public in `docs/audits/dXX_audit_report.md`

**Auditor talking points** :
  - "How do you handle whistleblower reports ?" → Public security
    contact + safe harbor in VDP
  - "What's your remediation track record ?" → 8 audit fixes M12 +
    7 audit fixes M13 + 8 audit fixes M11 = 23 defects caught + fixed
    pre-merge

#### CC1.4 — Authority and Responsibility

**EFE evidence** :
  - CODEOWNERS file enforces per-area approver requirements
  - `HANDOFF_OPS.md` RACI matrix per component
  - Branch protection rules require 2 reviewers post-freeze

#### CC2.2 — Internal Communication

**EFE evidence** :
  - Slack channels per concern (`#eng-incidents`, `#security`,
    `#compliance`)
  - PagerDuty / Opsgenie on-call rotation
  - Status page status.efe.ai for customer-facing incident comm
  - Quarterly all-hands compliance briefing

#### CC3.1 — Specifies Suitable Objectives

**EFE evidence** :
  - SLOs documented per sprint (D9, D11, D13, D17, D40, M11, M12,
    M13, M14) — see `SLO.md` consolidated
  - SLO targets reviewed quarterly per `continuous_compliance.md`

#### CC3.3 — Risk Mitigation Activities

**EFE evidence** :
  - Threat model `docs/security/threat_model.md` (M14 D40) — 63
    threats × mitigations matrix
  - Risk register `docs/security/risk_register.md` (D40 placeholder)
  - Pen-test program `docs/security/pentest_2026q3_report.md` (post-D42)
  - Bug bounty (post-1.0 GA, HackerOne private)

#### CC4.1 — Selects, Develops, and Performs Ongoing Evaluations

**EFE evidence** :
  - Prometheus continuous metrics collection (D13)
  - 10 D13 alerts + 15+ M14 D42 NEW runbooks
  - HMAC chain audit log (D14-bis) — tamper-evident
  - Quarterly DR drill (D40) — operational evidence pack

#### CC5.2 — Selects and Develops General Controls Over Technology

**EFE evidence** :
  - Pin tests (~92 across M12+M13 + 25 M14) — anti-regression structurelle
  - Polaris GREEN gate (M12) — security best-practice automated
  - CI workflows : helm-test, helm-publish, bundle-release
  - Helm chart audit fixes (M12 audit 8 defects) demonstrate gate effectiveness

#### CC6.3 — Authorizes, Modifies, or Removes Access

**EFE evidence** :
  - OIDC issuer integration (D15) — externalizes account management
  - JWT exp + jti claims — automatic timeout + replay prevention
  - Issuer revocation procedure : update `oidc-issuers.json`
    Secret + rolling restart (~ 2 min)

#### CC6.6 — Implements Logical Access Security

**EFE evidence** :
  - NetworkPolicy strict (M12 D21) — default-deny + tenant whitelist
  - mTLS (M12 D22) — mutual auth between services
  - cert-manager rotation 30d before expiry — auto-renewal
  - SBOM endpoint (M14 D40) — supply chain validation

#### CC6.8 — Prevents or Detects and Acts Upon Unauthorized Software

**EFE evidence** :
  - Image cosign verify in admission webhook (Tier 3 deployments)
  - Bundle Ed25519 sig + trust store (M13)
  - SBOM endpoint validates known-good components
  - Polaris GREEN refuses non-compliant Pods (M12)

#### CC7.2 — Monitors System Components

**EFE evidence** :
  - Grafana dashboards (D13) — 6 dashboards, 30+ panels
  - OTel SDK (D13-bis) — distributed tracing
  - Prometheus alerts D13 → runbooks D42

#### CC7.4 — Develops and Implements Activities to Prevent Disruptions

**EFE evidence** :
  - HPA D9 — auto-scale 3-30 replicas
  - PodDisruptionBudget D18 — minimum 2 pods always available
  - Multi-replica D18 — leader election + read-replica routing
  - Backup D17 — 4 CronJobs daily
  - DR drill D40 — quarterly full restore validation

#### CC8.1 — Authorizes, Designs, Develops, Acquires, Implements

**EFE evidence** :
  - All code changes via PR + 2 reviewers (post-freeze)
  - Branch protection on main
  - Pin tests + Polaris + helm-unittest CI gates
  - Audit pass per sprint (caught 23 defects pre-merge)

#### CC9.1 — Identifies, Selects, and Develops Risk Mitigation Activities

**EFE evidence** : (already covered in `dr_drill_quarterly_evidence.md`)
  - DR drill quarterly outcome.json + Hello-World workflow validation
  - 7-year Object Lock COMPLIANCE retention (immutable evidence)
  - 4 frameworks asserted via labels (SOC 2 CC9.1 / ISO A.17.1.3 / NIS2
    Art. 21 / RGPD Art. 32)

#### CC9.2 — Compliance with Confidentiality Commitments

**EFE evidence** :
  - DPA template + customer DPAs signed
  - Object Lock COMPLIANCE for evidence (no admin tampering)
  - HMAC chain audit prevents repudiation
  - PII hash (D13) limits PII exposure
  - mTLS (M12 D22) + TLS 1.3 in transit

#### A1.1 — Maintains, Monitors, and Evaluates Current Processing Capacity

**EFE evidence** :
  - HPA D9 metrics-driven scaling
  - ResourceQuota Tier 2 per-tenant limits
  - Capacity planning quarterly review (per `continuous_compliance.md`)

#### A1.2 — Authorizes, Designs, Develops, Acquires, Implements

**EFE evidence** :
  - Multi-AZ deployment via topologySpreadConstraints D18
  - PDB minimum 2 pods
  - Backup D17 + DR drill D40 RTO 30 min target
  - SLO M14-COMP-3 quarterly drill success ≥ 99 %

#### C1.1 — Identifies and Maintains Confidential Information

**EFE evidence** :
  - PII flagged via D13 PII hash
  - Tenant data classified per DPA
  - Customer-specific encryption keys (Tier 3 BYOK option)
  - Sub-processor list maintained in DPA addendum

### 7.4 SOC 2 audit timeline

```
2026-Q3 : Type I (point-in-time, 1 month)
2027-Q1 : Type II (continuous, 6-month observation period 2026-Q3 → 2026-Q4)
2028-Q1 : Type II annual renewal
```

---

## 8. RGPD / GDPR Art. 32 evidence (D41)

### 8.1 Mission

Document `docs/compliance/gdpr_art32.md` (~600 lignes) covering the
4 mesures techniques + organisationnelles required by Art. 32(1) :

- **(a)** Pseudonymisation et chiffrement
- **(b)** Confidentialité, intégrité, disponibilité, résilience
- **(c)** Capacité de restauration
- **(d)** Procédure pour tester, évaluer, apprécier régulièrement

### 8.2 Mapping mesures → controls EFE

| Mesure | EFE control |
|---|---|
| **a.** Pseudonymisation | D13 PII hash (`EFE_PII_HASH=true`) — emails / SSN / etc. hashed before logging |
| **a.** Chiffrement transit | mTLS M12 D22 + cert-manager rotation |
| **a.** Chiffrement repos | K8s Secret AES-256 (cloud-provider-managed) + SealedSecrets bitnami / external-secrets ; Postgres TDE optional |
| **b.** Confidentialité | OIDC D15 + RBAC D11 + NetworkPolicy strict M12 D21 |
| **b.** Intégrité | HMAC chain audit D14-bis + bundle Ed25519 sig M13 + cosign image |
| **b.** Disponibilité | HPA D9 + multi-replica D18 + PDB + DR drill D40 |
| **b.** Résilience | D17 backup multi-region + D40 quarterly drill + RTO 30 min target |
| **c.** Restauration | D17 4-CronJob backup pipeline + Hello-World drill validates restore E2E |
| **d.** Tests réguliers | D40 quarterly DR drill + Polaris CI gate + 92 pin tests |

### 8.3 DPIA template `gdpr_dpia.md`

Pour les déploiements traitant catégories particulières (Art. 9) ou
profilage automatisé (Art. 22), template DPIA :

```markdown
# DPIA — <Customer Tenant Name>

## 1. Description du traitement
## 2. Finalité légitime
## 3. Catégories de données
## 4. Catégories de personnes concernées
## 5. Évaluation nécessité + proportionnalité
## 6. Risques pour droits + libertés
## 7. Mesures de mitigation
## 8. Consultation DPO
## 9. Avis Personnes concernées
## 10. Décision finale
```

### 8.4 Data Processing Agreement (DPA)

Template `legal/dpa_template.md` à fournir aux customers EU :
- EFE in role of "processor" (Art. 28.3)
- Customer is "controller"
- Sub-processors list (cloud providers + monitoring vendors)
- International transfers : SCC (Standard Contractual Clauses) 2021/914
- Sub-processor notification : 30-day prior

### 8.5 Article 33 breach notification procedure

Runbook `general_incident_playbook.md` (D42) section §Breach :

```
T+0      : Incident detected (alert / customer report)
T+1h     : SOC severity assessed (P0/P1)
T+4h     : DPO notified
T+24h    : Customer notification draft prepared
T+72h    : CNIL / EU DPA notification sent (Art. 33 deadline)
T+1d     : Customer notification sent (Art. 34 if high risk)
T+7d     : Public statement on status.efe.ai
T+14d    : Post-mortem published (sanitized)
T+30d    : Remediation report to DPA
```

---

## 9. AI Act 2024/1689 Annex IV (D41)

### 9.1 Mission

Document `docs/compliance/ai_act_annex_iv.md` (~800 lignes) — EU
AI Act Regulation 2024/1689 Annex IV technical documentation
mandatory for **high-risk** AI systems (and recommended for
limited-risk).

EFE classification : **limited-risk AI system** (semantic
classification → not high-risk per Annex III). However we ship
Annex IV documentation pre-emptively for transparency.

### 9.2 Annex IV 8 sections

| Section | Title | EFE coverage |
|---|---|---|
| **§1** | General description | Architecture overview + intended purpose + interactions with hardware/software |
| **§2** | Detailed description elements | Methods used, training datasets, validation methods, optimization, computational resources |
| **§3** | Risk management system | `threat_model.md` + risk register + mitigation tracking |
| **§4** | Data governance | PII pseudonymization D13 + retention policy + lineage tracking |
| **§5** | Human oversight measures | Admin RPC for model rollback + manual catalog approval flow |
| **§6** | Predetermined changes + their performance | Model versioning catalog D12 + bundle versioning M13 |
| **§7** | Post-market monitoring | Prometheus metrics M11 + drift detection D13 + customer feedback |
| **§8** | Cybersecurity | Bundle Ed25519 sig M13 + HMAC audit D14-bis + supply chain SLSA Level 2 |

### 9.3 §1 General description (sample)

```markdown
## §1 General description

### 1.1 System name and version
EFE — Ephemeral Fluid Swarm v1.0.0

### 1.2 Intended purpose
EFE is a semantic classification system that assigns text inputs to
operator-defined classes via embedding-based nearest-cluster matching.

Use cases :
  - Customer support ticket routing
  - Content moderation triage
  - Product feature classification
  - Multi-tenant intent recognition

### 1.3 NOT intended for :
  - Biometric identification
  - Emotion recognition in workplaces or schools
  - Predictive policing
  - Social scoring
  - Real-time remote biometric identification in public spaces
  - Critical infrastructure safety functions

### 1.4 AI Act risk classification
Per Annex III enumeration, EFE is **NOT high-risk** :
  - Not biometric (no face/fingerprint inputs)
  - Not critical infrastructure (no SCADA / power / transport)
  - Not employment / education (no candidate scoring)
  - Not access to essential services (no welfare eligibility)
  - Not law enforcement
  - Not migration / asylum
  - Not justice administration

Classification : **Limited-risk transparency obligations apply
(Art. 52)** — users interacting with EFE outputs must be informed
they're interacting with an AI system.

### 1.5 Interactions
gRPC (Tonic) protocol over HTTP/2, mTLS optional. No direct user-
facing UI ; consumed via SDKs (Python/Go/TypeScript/Java) or REST
gateway.

### 1.6 Hardware
CPU x86_64 / ARM64. Optional GPU (NVIDIA, K-A3 path). No specialized
ML accelerators.
```

### 9.4 §2 Detailed description (sample)

```markdown
## §2 Detailed description

### 2.1 Methods
Sentence-transformer embeddings (Apache-2.0) + cosine similarity
nearest-class lookup + softmax thresholding for confidence.

### 2.2 Training datasets
**EFE does NOT train models.** EFE consumes pre-trained models
shipped via M13 bundles. Training data provenance documented per
bundle :
  - all-MiniLM-L6-v2 : MS MARCO + SNLI + Reddit (~1B sentence pairs)
    — see `bundles/all-MiniLM-L6-v2-en-1.0.0/SBOM.json` provenance
    properties
  - paraphrase-multilingual-MiniLM-L12-v2 : ParallelCorpora 50+
    languages (~10M parallel pairs)

### 2.3 Validation methods
  - Cross-lingual eval F1 ≥ 0.80 (D26 gate, fixture
    `tests/fixtures/multilingual_eval.jsonl`)
  - Cosine similarity post-quantization ≥ 0.99 vs FP32 baseline
  - Operator-defined catalog validation (D12) — accuracy threshold
    per customer

### 2.4 Optimization
INT8 dynamic quantization (D20 sample) or FP16 (D26 production)
to reduce on-disk size + inference latency. Trade-off documented
in `docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md` §5.3.

### 2.5 Computational resources
  - CPU: 1-2 cores per pod
  - Memory: 8 GiB per pod (incl. bundle in RAM)
  - GPU: optional (D28+ K-A3 path)
  - Storage: 100 MiB - 500 MiB per bundle

### 2.6 Limitations
  - Training data has inherited biases (model card refs)
  - English-centric : multilingual quality varies by language
  - Out-of-distribution texts produce low-confidence outputs
    (handled via confidence-threshold customization D12)
```

### 9.4-bis §3 Risk management system

```markdown
## §3 Risk management system

### 3.1 Risk identification methodology
EFE follows a hybrid STRIDE + ML-specific threat modeling approach :
  - **STRIDE-per-asset** : 63 threats consolidated in
    `docs/security/threat_model.md` (M14 D40)
  - **ML-specific risks** :
    - Embedding drift (mitigated via D6 cone fitting + manual approval D12)
    - Adversarial inputs (mitigated via input length cap + rate limit M11)
    - Training data bias (inherited from upstream model card disclosure)
    - Model inversion / extraction (out-of-scope for embeddings — no
      generative output ; semantic class output only)

### 3.2 Risk assessment cadence
  - Quarterly threat model review (per `continuous_compliance.md`)
  - Per-sprint audit pass (M11+M12+M13 audit pass 1 patterns)
  - External pen-test annual (M14 D40 plan)

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

### 3.5 Risk register link
See `docs/security/risk_register.md` (D40 placeholder, populated D41).
```

### 9.4-ter §4 Data governance

```markdown
## §4 Data governance + quality

### 4.1 Data sources
**Training data** : EFE does NOT train. Bundles come from upstream
HF Hub with pinned revisions (M13 D26) :
  - all-MiniLM-L6-v2 : MS MARCO + SNLI + Reddit Webis (publicly
    documented in HF model card)
  - paraphrase-multilingual-MiniLM-L12-v2 : ParallelCorpora 50+
    languages (publicly documented)

**Operational data** : customer-provided sentences for catalog
fitting (D12). Encrypted in transit (TLS 1.3) + at rest (Postgres
TDE).

### 4.2 Data quality assurance
  - Cosine similarity gate post-quantization (≥ 0.99 vs FP32)
  - Cross-lingual F1 ≥ 0.80 (D26 fixture)
  - Confidence threshold per-tenant (operator-tunable)
  - Drift detection via D13 dashboard (`classify_drift` runbook)

### 4.3 Bias mitigation
  - Inherited model card disclosures shipped with bundle (`README.md`)
  - Customer responsibility : test against their data + adjust
    confidence_threshold + add adversarial training examples
  - Operator may freeze bundle version if drift detected

### 4.4 Data retention
  - Audit log : 7 years (Object Lock COMPLIANCE)
  - Operational data (catalog fitting sentences) : tenant-defined,
    default 30 days
  - PII : hashed pre-storage (D13 `EFE_PII_HASH=true`)
  - Backup : per D17 retention policy (4 weeks full + 14 days diff +
    14 days WAL + 30 days vault)

### 4.5 Data minimization
  - Embedding outputs : 384-dim float — no PII reconstruction possible
  - Catalog metadata : tenant-provided, no inference
  - Audit log : action+actor only, no payload content
```

### 9.4-quater §6 Predetermined changes + their performance

```markdown
## §6 Predetermined changes for which the system was designed

### 6.1 Bundle versioning (M13)
  - Semver `MAJOR.MINOR.PATCH` per bundle slug
  - Backward-compat within MAJOR
  - Performance metrics tracked per release :
    - Cosine similarity post-quantization
    - Cross-lingual F1
    - Latency P95/P99
    - Memory footprint
  - Performance regression > 5 % blocks release (CI gate)

### 6.2 Catalog versioning (D12)
  - Per-tenant immutable catalog versions
  - `Register` creates new version, never mutates existing
  - `Evolve` re-fits on additional sentences
  - Operator manual approval gate

### 6.3 Configuration changes
  - Helm values controlled via Git + 2-reviewer rule
  - `helm upgrade` rolls forward ; `helm rollback` reverts
  - All values changes audited in D14-bis chain
```

### 9.4-quinquies §7 Post-market monitoring

```markdown
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

### 7.3 Customer feedback channel
  - Support email : support@efe.ai
  - Bug bounty (post-1.0 GA) : HackerOne private program
  - Status page : status.efe.ai
  - Vulnerability disclosure : `docs/security/vulnerability_disclosure.md`

### 7.4 Incident reporting (Art. 73 of AI Act)
  - Serious incidents reported to EU AI Office within 15 days
  - Report format per upcoming EU implementing acts
  - Customer notification via SLA matrix
```

### 9.4-sexies §8 Cybersecurity

```markdown
## §8 Cybersecurity measures (Art. 15 of AI Act)

### 8.1 Data integrity
  - HMAC chain audit log (D14-bis) — tamper-evident
  - Bundle Ed25519 signature (M13) — supply chain integrity
  - Cosign image signature (image release) — image integrity
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
```

### 9.5 §5 Human oversight (mandatory per Art. 14)

```markdown
## §5 Human oversight measures

### 5.1 Built-in oversight features
  - **Admin RPC for model rollback** (D27 bundle reload) — operator
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
  - `docs/runbooks/` — 25+ scenarios
  - `docs/HANDOFF_OPS.md` — escalation tree + ownership matrix

### 5.3 Disengagement procedure
Helm chart M12 supports rollback via `helm rollback efe <REVISION>`
in < 60 seconds. Documented in `docs/runbooks/m12_helm_upgrade_rollback.md`.
```

---

## 10. NIST SP 800-53 r5 mapping (D41)

### 10.1 Mission

Document `docs/compliance/nist_800_53.md` (~1 500 lignes) — NIST
SP 800-53 r5 control mapping for **moderate baseline** (FedRAMP
Moderate equivalent).

### 10.2 17 control families

| Family | Title | # Controls (Moderate) | EFE coverage |
|---|---|---:|---|
| **AC** | Access Control | 25 | OIDC D15 + RBAC D11 + NetworkPolicy M12 D21 |
| **AT** | Awareness and Training | 5 | Operator guide + runbooks (D42) |
| **AU** | Audit and Accountability | 16 | HMAC chain D14-bis + Loki + 7-year retention |
| **CA** | Assessment, Authorization | 9 | Pen-test D40 + audit pass per sprint + ATO process |
| **CM** | Configuration Management | 12 | Git + Helm versioning + Polaris GREEN gate |
| **CP** | Contingency Planning | 13 | D17 backup + D40 DR drill + RTO 30 min |
| **IA** | Identification, Authentication | 12 | OIDC D15 + JWT validation + JWKS |
| **IR** | Incident Response | 10 | `general_incident_playbook.md` (D42) + on-call rotation |
| **MA** | Maintenance | 6 | Helm `helm upgrade` + freeze procedure (D42) |
| **MP** | Media Protection | 8 | Cloud-managed (compensating control) |
| **PE** | Physical, Environmental Protection | 20 | Cloud-managed (compensating) |
| **PL** | Planning | 11 | Architecture docs (this directory) |
| **PS** | Personnel Security | 9 | CODEOWNERS + role-based git access |
| **RA** | Risk Assessment | 10 | `threat_model.md` (D40) + quarterly review |
| **SA** | System and Services Acquisition | 22 | SBOM endpoint (D40) + supply chain SLSA |
| **SC** | System and Communications Protection | 45 | mTLS M12 D22 + TLS 1.3 + boundary protection |
| **SI** | System and Information Integrity | 17 | Bundle sig M13 + HMAC audit + Polaris |

### 10.3 Sample control mapping (AC-2 Account Management)

```markdown
## AC-2 Account Management

**Control statement** : The organization manages information system
accounts, including establishing, activating, modifying, reviewing,
disabling, and removing accounts.

**EFE implementation** :

  - **AC-2(1) Automated System Account Management** : OIDC issuer
    integration (D15) — accounts managed externally (Auth0 / Keycloak
    / Azure AD) ; EFE consumes JWTs, no internal account database.
  - **AC-2(3) Disable Inactive Accounts** : N/A (managed by issuer)
  - **AC-2(4) Automated Audit Actions** : All account-related events
    (token issuance, refresh, revocation) logged via D14-bis HMAC
    chain.
  - **AC-2(5) Inactivity Logout** : JWT exp claim + cache TTL ;
    typical 1-hour expiration (issuer-configurable).
  - **AC-2(7) Privileged User Accounts** : RBAC `admin` claim
    required for `Register` / `Evolve` RPCs ; logged separately.
  - **AC-2(11) Usage Conditions** : Customer DPA enforces acceptable
    use ; technical enforcement via rate limit (M11) + per-tenant
    quotas.
  - **AC-2(12) Account Monitoring/Atypical Usage** : D13 dashboard
    `unknown_principal` alert + runbook
    `d13_audit_unknown_principal.md`.

**Inheritance** : Customer responsability for AC-2(2) Removal of
Temporary/Emergency Accounts (their issuer config).
```

### 10.4 Family coverage summary

| Family | # Total | Implemented | Compensating | N/A |
|---|---:|---:|---:|---:|
| AC | 25 | 22 | 1 | 2 |
| AT | 5 | 5 | 0 | 0 |
| AU | 16 | 14 | 1 | 1 |
| CA | 9 | 7 | 1 | 1 |
| CM | 12 | 12 | 0 | 0 |
| CP | 13 | 13 | 0 | 0 |
| IA | 12 | 11 | 0 | 1 |
| IR | 10 | 10 | 0 | 0 |
| MA | 6 | 6 | 0 | 0 |
| MP | 8 | 0 | 8 | 0 |
| PE | 20 | 0 | 20 | 0 |
| PL | 11 | 11 | 0 | 0 |
| PS | 9 | 6 | 3 | 0 |
| RA | 10 | 10 | 0 | 0 |
| SA | 22 | 18 | 2 | 2 |
| SC | 45 | 38 | 5 | 2 |
| SI | 17 | 16 | 1 | 0 |
| **Total** | **250** | **199 (80 %)** | **42 (17 %)** | **9 (3 %)** |

> **FedRAMP Moderate gate** : 95+ % "Applicable" coverage. EFE 1.0
> achieves 97 % (241 / 250). Above threshold ✓.

---

## 11. Customer DDQ pre-fill template (D41)

### 11.1 Mission

`docs/compliance/ddq_prefill.md` (~1 800 lignes) — pre-filled answers
to common Due Diligence Questionnaires (CAIQ v4.0.2 / SIG Lite /
custom enterprise DDQs).

Reduces sales cycle 30+ days. Sales team copies relevant sections
into customer's DDQ form.

### 11.2 Question categories (200+ questions)

| Category | # Questions | Source |
|---|---:|---|
| Application Security | 25 | OWASP ASVS + CSA CCM |
| Audit Assurance | 18 | SOC 2 + ISO 27001 |
| Business Continuity | 12 | NIST CP family |
| Change Control | 9 | ITIL + SOC 2 CC8 |
| Data Security | 35 | GDPR + ISO 27001 A.8 + NIST SC |
| Encryption | 14 | NIST SC-13 + FIPS 140 |
| Governance, Risk, Compliance | 22 | SOC 2 CC1 + ISO 27001 A.5 |
| Human Resources | 8 | ISO A.6 + NIST PS |
| IAM | 18 | NIST AC + IA |
| Incident Response | 11 | NIST IR family |
| Infrastructure Security | 19 | CSA CCM IVS |
| Mobile Security | N/A | EFE n'a pas de mobile app |
| Operations Management | 14 | ITIL + SOC 2 CC7 |
| Physical Security | 8 | Compensating (cloud) |
| Vulnerability Management | 12 | NIST RA + SI |
| Privacy | 18 | GDPR + AI Act |
| **Total** | **243** | |

### 11.3 Sample Q&A entry

```markdown
## Q-AS-12 Does the system enforce least privilege ?

**Answer** : YES. EFE enforces least privilege at multiple layers :

  1. **Application** : RBAC per-tenant (D11) — JWT `tenant_id` claim
     scoped to allowed namespaces.
  2. **Container** : `securityContext.runAsNonRoot=true`,
     `runAsUser=10001`, `runAsGroup=10001`,
     `allowPrivilegeEscalation=false`, `readOnlyRootFilesystem=true`,
     `capabilities.drop=ALL`. Polaris GREEN gate enforces.
  3. **K8s** : ServiceAccount `automountServiceAccountToken=false`
     by default ; only the M12 D27 bundle-reload SA has automount
     (with scoped Role : get/list pods/services/configmaps only).
  4. **Network** : NetworkPolicy strict (M12 D21, opt-in) — default-
     deny + tenant whitelist + monitoring scrape allow-list.
  5. **Cloud IAM** : Helm chart documents IAM Role minimum permissions
     in `docs/compliance/iam_least_privilege.md`.

**Evidence** :
  - `deploy/helm/efe/values.yaml`
  - `crates/efe-server/src/auth/rbac.rs`
  - `scripts/tests/test_d11_per_tenant_isolation.py`
  - SOC 2 evidence file `docs/compliance/soc2_evidence.md` §CC6.1
```

---

## 12. Release Notes 1.0 (D42)

### 12.1 Mission

`docs/RELEASE_NOTES_1.0.md` (~1 200 lignes) — comprehensive GA notes.

### 12.2 Structure

```markdown
# EFE 1.0.0 — Release Notes

## Highlights
## Breaking changes (vs 0.x)
## Migration guide (0.x → 1.0)
## New features (M11+M12+M13+M14)
## Performance improvements
## Security improvements
## Compatibility matrix (K8s versions, Postgres versions, etc.)
## SLAs (uptime, support response)
## Support matrix
   - Tier 1 / Tier 2 / Tier 3 / Tier 4 (paid)
## Long-Term Support (LTS) policy
   - 1.0 LTS through 2031-12-31 (5 years)
   - Security patches only post-1.x EOL
## Known issues
## Deprecation notices
## Acknowledgments
```

### 12.3 SLA matrix

| Severity | Response | Resolution target |
|---|---|---|
| P0 (data loss / outage) | 1 hour | 4 hours |
| P1 (degraded service) | 4 hours | 24 hours |
| P2 (functional issue) | 1 business day | 5 business days |
| P3 (cosmetic/feature request) | 5 business days | Next release |

### 12.4 Compatibility matrix

| Component | Supported versions |
|---|---|
| Kubernetes | 1.27, 1.28, 1.29, 1.30 |
| Postgres | 14, 15, 16 |
| OpenJDK (Java SDK M11) | 17, 21 LTS |
| Python (Python SDK M8) | 3.10, 3.11, 3.12 |
| Go (Go SDK M9) | 1.21, 1.22 |
| Node.js (TS SDK M10) | 18, 20, 22 LTS |
| cert-manager | ≥ 1.13 |
| OIDC providers | Auth0, Keycloak, Azure AD, Okta |
| OCI registries | GHCR, ECR, GCR, Harbor, Nexus |

### 12.5 Migration guide skeleton

```markdown
## Migration from 0.x to 1.0

### 1. Read the breaking changes section first
### 2. Backup your data (D17 procedure)
### 3. Upgrade Helm chart 0.x → 1.0
### 4. Run smoke tests
### 5. Update SDK clients
### 6. Cutover
### 7. Rollback procedure
```

---

## 13. Handoff doc équipe ops (D42)

### 13.1 Mission

`docs/HANDOFF_OPS.md` (~700 lignes) — transition build team → operate
team.

### 13.2 Structure

```markdown
# EFE 1.0 GA — Operations Handoff Document

## 1. Ownership matrix (RACI)
   - Component → Primary / Secondary / Backup
## 2. On-call rotation template
   - PagerDuty / Opsgenie configuration
   - Escalation tree (L1 → L2 → L3 → CISO)
## 3. Critical credentials inventory
   - What lives where, who has access
   - Rotation calendar (quarterly / biannual / annual)
## 4. Production environment overview
   - Cluster topology
   - Network architecture
   - DNS records authoritative source
## 5. Build pipeline ownership
## 6. Customer escalation procedure
## 7. Vendor contacts (CSP, CDN, monitoring)
## 8. Compliance officer contacts
## 9. Legal / DPO contacts
## 10. Post-incident review template
```

### 13.3 RACI matrix sample

| Component | Owner | Backup | Reviewer | Decision |
|---|---|---|---|---|
| Helm chart M12 | DevOps lead | Eng lead | Security lead | Eng VP |
| Bundle pipeline M13 | ML lead | DevOps lead | Security lead | Eng VP |
| OIDC D15 | Security lead | DevOps lead | Eng lead | CISO |
| Postgres backend D14 | Data eng | DevOps lead | Eng lead | Eng VP |
| Multi-tenant D11 | Eng lead | Security lead | DevOps lead | Eng VP |
| Backup D17 | DevOps lead | Eng lead | Security lead | Eng VP + DPO |
| DR drill D40 | DevOps lead | Compliance lead | CISO | CEO |
| SBOM endpoint M14-D40 | Security lead | DevOps lead | Eng lead | CISO |
| Threat model M14-D40 | Security lead | Eng lead | CISO | CISO |
| Pen-test M14-D40 | Security lead | Eng VP | CISO | CEO + CISO |

### 13.4 Secrets rotation calendar

| Secret | Owner | Cadence | Last rotated | Next rotation |
|---|---|---|---|---|
| `EFE_BUNDLE_PRIVKEY_PEM` (M13) | Security lead | Quarterly | 2026-Q1 | 2026-Q2 |
| OIDC issuer secrets (D15) | Security lead | Annual | 2025-Q4 | 2026-Q4 |
| HMAC keys (D14-bis) | Security lead | Quarterly | 2026-Q1 | 2026-Q2 |
| TLS certificates (M12 D22) | DevOps lead | Auto-rotate cert-manager (30d before expiry) | rolling | rolling |
| Cloud IAM (S3, KMS) | DevOps lead | Annual | 2025-Q4 | 2026-Q4 |
| GHCR PAT | DevOps lead | Annual | 2025-Q4 | 2026-Q4 |
| Slack webhook | DevOps lead | Ad-hoc | 2025-Q3 | as-needed |

### 13.5 Escalation tree

```
                  L1 — On-call engineer
                        │
                        │ (15-min escalate if no response)
                        ▼
                  L2 — Eng lead
                        │
                        │ (30-min escalate if P0 not contained)
                        ▼
                  L3 — CTO / Eng VP
                        │
                        │ (60-min if critical breach / regulatory)
                        ▼
                  L4 — CEO + CISO + DPO + Legal
                        │
                        │ (Customer notification approval)
                        ▼
                  External : Customers + EU DPA
```

---

## 14. Runbook inventory (D42)

### 14.1 Mission

Extend `docs/runbooks/` from 10 D13-only files to **25+ files** covering
all post-D13 sprints.

### 14.2 NEW runbooks (15+ files, ~80 LOC each)

| Runbook | Sprint | Trigger alert | ETA |
|---|:---:|---|:---:|
| `d14_postgres_failover.md` | D14 | `D14ObsPgConnectionFailureRate > 5%` | 30 min |
| `d14_pg_pool_exhausted.md` | D14 | `D14ObsPgPoolExhausted` | 10 min |
| `d15_oidc_jwks_stale.md` | D15 | `D15ObsJwksStale > 1h` | 15 min |
| `d15_tenant_onboarding_failed.md` | D15 | Customer support ticket | 1 hour |
| `d16_bundle_signature_invalid.md` | D16 | `D16ObsBundleSigInvalid` | Immediate boot fail |
| `d16_bundle_load_timeout.md` | D16 | `D16ObsBundleLoadDuration > 60s` | 5 min |
| `d17_backup_failed.md` | D17 | `D17ObsBackupFailureRate > 0%` | 30 min |
| `d17_dr_drill_rto_breach.md` | D17 | `D17ObsDrillRtoBreach` | Quarterly post-drill |
| `d18_pod_unable_to_acquire_lease.md` | D18 | `D18ObsLeaseAcquireFail` | 2 min |
| `d18_replicas_below_minimum.md` | D18 | `D18ObsReplicasBelowMin` | 5 min |
| `d21_networkpolicy_strict_blocks.md` | M12 D21 | Customer report blocked traffic | 30 min |
| `d22_mtls_cert_renewal_failed.md` | M12 D22 | `cert-manager.io/Certificate` not Ready | 1 hour |
| `d27_bundle_reload_hook_failed.md` | M12 D27 | Helm hook Job exit non-zero | Immediate post-upgrade |
| `d40_dr_drill_quarterly_failed.md` | M12 D40 | `D40DrillQuarterlyFailed` | Within quarter |
| `d26_bundle_pull_oci_failed.md` | M13 D26 | `M13ObsBundlePullFailed` | 5 min |
| `d20_sample_bundle_quickstart_broken.md` | M13 D20 | Customer support | 1 hour |
| `m12_helm_upgrade_rollback.md` | M12 cross-cutting | Manual trigger | 5 min |
| `m13_signing_key_rotation.md` | M13 cross-cutting | Quarterly | Quarterly |
| `general_incident_playbook.md` | All | Any P0/P1 | Immediate |
| `general_breach_notification.md` | All | Confirmed breach | T+24h |

(Sub-total : 20 NEW runbooks. With the 10 existing D13 = 30 runbooks
total post-M14.)

### 14.3 Standard runbook template

```markdown
# Runbook : <Alert Name>

**Severity** : P0 | P1 | P2 | P3
**ETA to resolution** : <minutes / hours>
**Sprint origin** : DXX
**Last updated** : YYYY-MM-DD

## Symptom
## Why this matters (impact)
## Diagnostics
## Mitigation steps
## Resolution criteria
## Post-incident actions
## Related runbooks
```

---

## 15. Freeze procedure (D42)

### 15.1 Mission

Transition repo from "build mode" (heavy commits, feature PRs) to
"operate mode" (security patches + critical bug-fix only).

### 15.2 Branch protection rules

```yaml
# .github/branch-protection-rules.yml (effective post-J175)
main:
  required_status_checks:
    strict: true
    contexts:
      - ci/build
      - ci/test
      - ci/polaris
      - ci/pin-tests
      - ci/sbom-validate
      - ci/license-gate
  enforce_admins: true
  required_pull_request_reviews:
    required_approving_review_count: 2
    dismiss_stale_reviews: true
    require_code_owner_reviews: true
    require_last_push_approval: true
  restrictions:
    apps: []
    users: []
    teams:
      - core-engineering
      - security-team
  required_conversation_resolution: true
  required_signatures: true
  block_creations: false
```

### 15.3 Allowed PR types post-freeze

| PR type | Examples | Approver | SLA |
|---|---|---|---|
| **Security patch** | CVE response, pen-test finding fix | 2× security team | < 7 days |
| **Critical bug-fix** | Data loss, P0 outage cause | 2× engineering team | < 7 days |
| **Compliance** | New audit finding, regulator update | 2× compliance + security | < 30 days |
| **Documentation** | Runbook update, clarification | 1× any reviewer | < 3 days |
| **Dependency security update** | CVE in dependency | 1× security team | < 7 days |
| **Test addition** | New pin test for existing code | 1× any reviewer | < 7 days |
| **Feature** | New capability | ❌ REFUSED — opens 2.0 cycle | N/A |
| **Refactor** | Non-trivial code restructuring | ❌ REFUSED post-freeze | N/A |

### 15.4 Freeze announcement

`CONTRIBUTING.md` post-J175 :

```markdown
# 🔒 EFE 1.0 GA — REPO IS FROZEN as of 2026-XX-XX

The repository is in **operate mode**. Only the following PR types
are accepted :

  - Security patches (CVE response, pen-test findings)
  - Critical bug-fixes (P0 outage causes)
  - Compliance updates
  - Documentation clarifications

Feature PRs and non-trivial refactors will be **closed without merge**
and may be considered for the EFE 2.0 cycle (planned start 2027-Q1).

## How to submit an accepted PR

1. Open an issue first — describe the bug/CVE
2. Wait for maintainer triage (typically < 48 hours for P0/P1)
3. Submit PR with explicit "TYPE: <security|critical-fix|compliance|docs>"
   in the title
4. Add the `accepted-post-freeze` label
5. Two reviewers required (CODEOWNERS-enforced)
```

### 15.5 Tag signing

```bash
# Generate v1.0.0 tag, signed by release lead PGP key
git tag -s v1.0.0 \
    -m "EFE 1.0.0 GA — first stable release" \
    -u D40-RELEASE-KEY-2026

git push origin v1.0.0

# Verify tag signature
git verify-tag v1.0.0
# gpg: Signature made <timestamp>, RSA key 0xABCD...
# gpg: Good signature from "EFE Release Authority <release@efe.ai>"
```

### 15.6 Post-freeze release cadence

```
1.0.x patches : every 30 days (or earlier for security)
1.0 LTS support : through 2031-12-31 (5 years)
1.0 EOL : 2031-12-31

2.0.0 GA target : 2027-Q1 (in parallel during 2026)
```

---

## 16. Continuous compliance post-GA

### 16.1 Quarterly compliance review

`docs/compliance/continuous_compliance.md` :

```markdown
## Quarterly compliance review (Q1, Q2, Q3, Q4)

### Inputs reviewed
  - DR drill outcome (per `dr_drill_quarterly_evidence.md`)
  - Threat model updates
  - SOC 2 evidence collection (continuous)
  - GDPR data subject requests log
  - Pen-test findings status
  - SBOM index health (size / age)

### Outputs
  - Quarterly compliance report (signed by CISO)
  - Updated SoA if controls drift
  - Risk register update
```

### 16.2 Annual SOC 2 Type II audit

```
Q1     : Engagement letter + scope letter
Q2     : Audit kickoff + walk-throughs
Q3     : Field work + sample testing
Q4     : Report draft + management response
Q1+1   : Final report + bridge letter to next year
```

### 16.2-bis Pre-built compliance evidence collectors

CI-driven evidence collection runs on a schedule + on-demand, writing
to immutable storage :

```yaml
# .github/workflows/evidence-collection.yml (M14 D42, NEW)
name: evidence-collection
on:
  schedule:
    - cron: "0 4 * * 1"  # weekly Monday 04:00 UTC
  workflow_dispatch:

jobs:
  collect-evidence:
    runs-on: ubuntu-22.04
    permissions:
      contents: read
      id-token: write   # for AWS OIDC role
    steps:
      - uses: actions/checkout@v4
      - name: Collect evidence
        run: |
          DATE=$(date +%Y-%m-%d)
          mkdir -p evidence/${DATE}/

          # 1. Pin tests results
          python3 -m pytest scripts/tests/ \
              --junitxml evidence/${DATE}/pin_tests.xml

          # 2. Polaris audit results (3 profiles)
          for profile in default staging prod; do
            helm template efe deploy/helm/efe/ \
                -f deploy/helm/efe/values.${profile}.yaml \
                > evidence/${DATE}/manifest_${profile}.yaml
            polaris audit \
                --audit-path evidence/${DATE}/manifest_${profile}.yaml \
                --output-file evidence/${DATE}/polaris_${profile}.json \
                --output-format json
          done

          # 3. SBOM index integrity
          curl -fsSL https://efe.ai/sbom/index.json \
              > evidence/${DATE}/sbom_index.json

          # 4. SLO measurements (last 7 days)
          # ... (Prometheus query for compliance metrics §16.3)

          # 5. Audit log integrity check
          # (HMAC chain validation, sample 100 random rows)

      - name: Upload to immutable evidence bucket
        env:
          AWS_ROLE_ARN: ${{ secrets.AWS_EVIDENCE_ROLE }}
        run: |
          aws s3 cp evidence/${DATE}/ \
              s3://efe-evidence-prod/${DATE}/ \
              --recursive \
              --metadata "collected_at=$(date -u +%FT%TZ)"
```

### 16.2-ter Auditor read-access provisioning

External auditors (SOC 2, ISO 27001) receive scoped read-access via
short-lived AWS IAM sessions :

```bash
# Issued by CISO at audit kickoff
aws sts assume-role \
    --role-arn arn:aws:iam::<ACCT>:role/AuditorReadOnly \
    --role-session-name auditor-soc2-2026-q3 \
    --duration-seconds 14400  # 4-hour session
```

Auditor role permissions :
  - `s3:GetObject` on `efe-evidence-prod/*`
  - `s3:GetObject` on `efe-sbom-prod/*` (already public)
  - No write/delete anywhere
  - CloudTrail logged + DPO notified per session

### 16.3 Compliance metrics

| Metric | Target | Source |
|---|---|---|
| Pin tests GREEN rate | 100 % | CI |
| Polaris GREEN rate | 100 % | CI |
| DR drill RTO compliance | ≥ 99 % | D40 outcome.json |
| Pen-test critical findings | 0 open > 7d | Security tracker |
| GDPR data subject request response time | < 30 days | DPO log |
| SBOM endpoint uptime | ≥ 99.9 % | CDN metrics |
| Customer DDQ response time | < 5 business days | Sales tracker |

---

## 17. Audit-readiness checklist

### 17.1 Pre-audit (T-30 days)

- [ ] All `docs/compliance/*.md` files updated for current quarter
- [ ] Pin tests 100 % GREEN on `main`
- [ ] Polaris GREEN on all 3 values profiles
- [ ] Latest pen-test report ≤ 12 months old
- [ ] DR drill quarterly evidence pack present for last 4 quarters
- [ ] SoA reviewed + signed by CISO
- [ ] Risk register updated
- [ ] Threat model re-evaluated post-last-sprint
- [ ] Audit log integrity check (HMAC chain) passes
- [ ] Backup integrity check passes
- [ ] All credentials rotated per calendar

### 17.2 During audit

- [ ] Auditor liaison assigned (typically Security lead)
- [ ] Daily sync at 10 AM
- [ ] Evidence repository read-access provisioned (RO IAM Role)
- [ ] Pre-prepared talking points per CC point
- [ ] Customer reference list provided

### 17.3 Post-audit

- [ ] Findings tracked in security backlog (Jira / GitHub issues)
- [ ] Remediation PRs opened with `audit-finding` label
- [ ] Re-test scheduled for blocking findings
- [ ] Updated SoA published
- [ ] Customer notification if findings disclosed

---

## 18. Test plan M14

### 18.1 Pin tests M14 (~25 tests)

| File | Tests | Coverage |
|---|---:|---|
| `test_m14_compliance_docs_present.py` | 8 | All `docs/compliance/*.md` exist + non-empty |
| `test_m14_threat_model_consolidated.py` | 5 | `threat_model.md` references all sources |
| `test_m14_runbook_inventory.py` | 6 | 25+ runbooks present + each has Symptom/Mitigation/Resolution sections |
| `test_m14_release_notes_1_0.py` | 3 | Release notes structure complete + version metadata |
| `test_m14_freeze_branch_protection.py` | 3 | Branch protection rule file present |
| **Total** | **25** | |

### 18.2 Document validation

- Markdown lint via `mdformat` / `markdownlint`
- Link checking (no 404s) via `markdown-link-check`
- SBOM endpoint mock test (curl / response shape)
- DDQ Q&A schema validation

### 18.3 Audit dry-run

Pre-audit M14 D42 :
- Internal compliance team walks through `soc2_evidence.md`
  end-to-end with auditor's perspective
- Each CC point demonstrated in production
- Findings tracked + closed before external audit

---

## 19. SLOs M14-COMP-*

| SLO | Metric | Target | Window |
|---|---|---|---|
| **M14-COMP-1** | Compliance docs coverage | 100 % frameworks targeted | Per-release |
| **M14-COMP-2** | Pin tests M14 GREEN rate | 100 % | Continuous |
| **M14-COMP-3** | DR drill quarterly success | ≥ 99 % within RTO | Per-quarter |
| **M14-COMP-4** | Pen-test critical findings | 0 open > 7d | Continuous |
| **M14-COMP-5** | SBOM endpoint uptime | ≥ 99.9 % | 30-day rolling |
| **M14-COMP-6** | DDQ response time | < 5 business days | Per-customer-request |
| **M14-COMP-7** | Runbook coverage rate | ≥ 95 % alerts have runbook | Per-quarter |
| **M14-COMP-8** | Audit cycle time | ≤ 90 days first audit | Annual |

---

## 20. Risk register & open questions

### 20.1 Risk register

| ID | Risk | Probability | Impact | Mitigation |
|---|---|:---:|:---:|---|
| **R-M14-1** | First SOC 2 audit fails (control gaps) | MED | HIGH | Internal dry-run pre-audit + early auditor engagement |
| **R-M14-2** | Pen-test reveals CRIT/HIGH findings post-freeze | MED | MED | Security-only PRs allowed post-freeze |
| **R-M14-3** | Customer DDQ format not compatible with our pre-fill | LOW | LOW | DDQ template covers CAIQ + SIG Lite (most common) |
| **R-M14-4** | EU AI Act 2026 changes interpretation | MED | MED | Quarterly legal review + flexible Annex IV doc |
| **R-M14-5** | NIS2 transposition delay creates ambiguity | LOW | LOW | Direct EU regulation primary source |
| **R-M14-6** | SBOM endpoint scaling (DDoS) | LOW | MED | CloudFront edge cache + WAF rate limit |
| **R-M14-7** | Documentation drift post-freeze | HIGH | LOW | Quarterly compliance review + automated link checking |
| **R-M14-8** | Compliance officer turnover post-handoff | MED | MED | Doc onboarding + 90-day overlap with predecessor |

### 20.2 Open questions

| ID | Question | Owner | Resolution target |
|---|---|---|---|
| **Q-M14-1** | SOC 2 auditor selection : Big 4 (Deloitte/EY/KPMG/PwC) vs boutique ? | CISO | Pre-D41 |
| **Q-M14-2** | FedRAMP path : pursue Moderate or High baseline ? | CTO + CEO | Post-1.0 GA |
| **Q-M14-3** | Bug bounty : HackerOne private vs Bugcrowd vs self-hosted ? | Security lead | Post-pen-test |
| **Q-M14-4** | LTS support window : 5 years vs 7 years ? | CTO | Pre-D42 |
| **Q-M14-5** | DPO outsourced vs in-house ? | CEO + Legal | Pre-1.0 GA |
| **Q-M14-6** | AI Act : self-classify limited-risk vs request EU formal opinion ? | Legal | Pre-2026 (effective date) |
| **Q-M14-7** | Open source license freeze 1.0 onwards : Apache-2.0 confirmed ? | Legal + CTO | Pre-D42 |

---

## 21. Appendix A — control matrix (5 frameworks × 30 controls)

| EFE Module | SOC 2 | ISO 27001 | GDPR | NIST 800-53 | AI Act |
|---|---|---|---|---|---|
| **OIDC D15** | CC6.1 | A.5.15 | Art. 32(1)(b) | AC-2, IA-2 | Art. 9(2)(d) |
| **RBAC D11** | CC6.2 | A.5.15 | Art. 32(1)(b) | AC-3, AC-6 | Art. 14 |
| **Audit log D14-bis** | CC4.1 | A.8.15 | Art. 32(1)(b) | AU-2, AU-3, AU-9 | Art. 12 |
| **Bundle signing M13** | CC8.1 | A.8.30 | Art. 32(1)(b) | SI-7 | Art. 15 |
| **mTLS M12 D22** | CC6.7 | A.8.20 | Art. 32(1)(a) | SC-8, SC-12 | Art. 15 |
| **NetworkPolicy M12 D21** | CC6.6 | A.8.20 | Art. 32(1)(b) | SC-7 | — |
| **Backup D17** | A1.2 | A.8.13 | Art. 32(1)(c) | CP-9 | — |
| **DR drill D40** | A1.3 | A.5.30 | Art. 32(1)(d) | CP-4 | — |
| **HMAC chain D14-bis** | CC4.2 | A.8.15 | Art. 32(1)(b) | AU-9 | Art. 12 |
| **SBOM endpoint M14-D40** | CC8.1 | A.8.32 | — | SA-15, SI-7 | Art. 15 |
| **Threat model M14-D40** | CC3.2 | A.5.7 | — | RA-3 | Art. 9 |
| **Pen-test M14-D40 plan** | CC3.4 | A.8.34 | — | CA-8 | — |
| **Polaris GREEN M12** | CC8.1 | A.8.32 | — | CM-6 | Art. 15 |
| **Multi-replica D18** | A1.2 | A.8.14 | Art. 32(1)(b) | CP-2 | — |
| **HPA D9** | A1.2 | A.5.30 | — | CP-2 | — |
| **OTel D13-bis** | CC4.1 | A.8.16 | — | AU-12 | Art. 12 |
| **Quotas D11** | CC6.5 | A.8.6 | — | SC-5 | — |
| **Rate limit M11** | CC6.5 | A.8.6 | — | SC-5 | — |
| **PII hash D13** | C1.1 | A.8.10 | Art. 25, 32(1)(a) | SC-28 | Art. 10 |
| **TLS 1.3** | CC6.7 | A.8.20 | Art. 32(1)(a) | SC-8 | — |
| **Distroless image** | CC8.1 | A.8.7 | — | SI-3 | — |
| **HelloWorld drill D40** | CC9.1 | A.5.30 | Art. 32(1)(d) | CP-4 | — |
| **Object Lock 7-yr D17** | CC9.2 | A.8.13 | Art. 32(1)(c) | AU-11 | Art. 12 |
| **Helm hook idempotency M12** | CC8.1 | A.8.32 | — | CM-6 | — |
| **Pin tests pre-merge** | CC8.1 | A.8.32 | — | CM-6 | — |
| **Polaris exemptions M12** | CC8.1 | A.8.32 | — | CM-7 | — |
| **License gate M13** | CC8.1 | A.5.32 | — | SA-22 | — |
| **Reproducibility check M13** | CC8.1 | A.8.32 | — | SI-7 | Art. 15 |
| **OCI Rekor entry M13** | CC4.1 | A.8.30 | — | AU-2 | Art. 12 |
| **CONTRIBUTING + CODEOWNERS** | CC1.4 | A.5.4 | — | PS-2 | — |

---

## 22. Appendix B — runbook template

```markdown
# Runbook : <Alert Name>

> **Severity** : P0 / P1 / P2 / P3
> **ETA to resolution** : <minutes / hours>
> **Sprint origin** : DXX
> **Last updated** : YYYY-MM-DD
> **Owner** : <DevOps / Security / Eng>

---

## 1. Symptom

What you observe :
  - Alert name : `D14ObsPgConnectionFailureRate > 5%`
  - Customer report : "Cannot save catalog version"
  - Dashboard : `dashboard.efe.ai/d/postgres` panel "Conn fail %"

## 2. Why this matters (impact)

  - <e.g.> All Postgres-backed catalog operations failing
  - Tenant can't deploy new model versions
  - Cascade : if > 30 min, audit chain (D14-bis) starts queueing
  - Customer SLA (P1, 4-hour resolution target)

## 3. Diagnostics

```bash
# Check current pool state
kubectl exec -n efe efe-server-0 -- /efe-admin db-status

# Check Postgres-side connection count
kubectl exec -n efe-pg efe-pg-0 -- psql -c "SELECT * FROM pg_stat_activity"

# Check NetworkPolicy isn't blocking
kubectl get networkpolicies -n efe -o yaml | grep -A 5 efe-server-strict
```

## 4. Mitigation steps

### Quick mitigations (< 5 min)

  1. Bump pool max : `helm upgrade efe charts/efe \
     --set config.catalog.postgres.poolMaxConnections=16`
  2. Restart Postgres : `kubectl rollout restart sts/efe-pg -n efe-pg`
  3. Failover to replica (if in HA setup)

### Longer mitigations (15-60 min)

  4. Investigate slow queries : `pg_stat_statements`
  5. Add Postgres replicas (if quotas allow)
  6. Fall back to memory backend temporarily :
     `helm upgrade efe charts/efe --set config.catalog.backend=memory`

## 5. Resolution criteria

  - `D14ObsPgConnectionFailureRate` < 1 % for 15 min
  - `pg_stat_activity` count < 80 % of `max_connections`
  - Customer report acknowledged

## 6. Post-incident actions

  - [ ] Add Postgres metric to long-term retention
  - [ ] Open ticket if pool size needs perm bump
  - [ ] Update D14 architecture doc if pattern recurs
  - [ ] Add anti-regression pin test if root cause is code

## 7. Related runbooks

  - `d14_postgres_failover.md` — when to fail over
  - `d14_pg_catalog_corrupt.md` — recovery from corrupted state
```

---

## 22-bis. Appendix B-extra — full sample runbook

Below is a complete sample runbook (`d22_mtls_cert_renewal_failed.md`)
demonstrating the canonical M14 D42 runbook format end-to-end.

```markdown
# Runbook : d22_mtls_cert_renewal_failed

> **Severity** : P1 (degraded service — TLS cert nearing expiry)
> **ETA to resolution** : 2 hours
> **Sprint origin** : M12 D22 (mTLS via cert-manager)
> **Last updated** : 2026-04-30
> **Owner** : DevOps lead (backup : Security lead)

---

## 1. Symptom

Multiple signals can fire :

  - **Alert** : `cert-manager.io/Certificate` resource in `Ready=False`
    state for > 1 hour
  - **Alert** : Prometheus `cert_manager_certificate_expiration_timestamp_seconds`
    < (now() + 7 days)
  - **Customer report** : "TLS handshake fails on efe-server"
  - **Helm test** : `helm test efe -n <ns>` returns failure on
    `efe-test-mtls-handshake` Pod (M12 D22 livré)

## 2. Why this matters (impact)

  - Without valid TLS cert, mTLS handshake fails → all efe-server
    connections refused.
  - Customer SLA P1 : 4-hour resolution target.
  - Cascade : if cert expires fully, automatic restart of efe-server
    pods will fail → outage.
  - Compliance impact : SOC 2 CC6.7 (transmission encryption) +
    ISO A.8.20 (network security) + GDPR Art. 32(1)(a) (chiffrement).

## 3. Diagnostics

```bash
# Check Certificate state
kubectl get certificate -n efe efe-mtls -o yaml | yq '.status'
# Expected : .status.conditions[0].status == "True", type == "Ready"

# Check renewal time
kubectl get certificate -n efe efe-mtls -o yaml | \
    yq '.status.renewalTime'

# Check cert-manager events for this Certificate
kubectl describe certificate -n efe efe-mtls | tail -20

# Check Issuer state
kubectl get clusterissuer <issuerName> -o yaml | yq '.status'

# Check secret content
kubectl get secret -n efe efe-mtls-tls -o yaml | \
    yq '.data."tls.crt"' | base64 -d | \
    openssl x509 -noout -dates -subject -issuer
```

## 4. Mitigation steps

### Quick mitigations (< 15 min)

  1. **If cert is just stuck on renewal** :
     ```bash
     kubectl annotate certificate efe-mtls -n efe \
         cert-manager.io/issue-temporary-certificate="true" \
         --overwrite
     ```
     Force cert-manager to retry immediately.

  2. **If the Issuer is broken** (e.g., Vault unavailable) :
     Switch to fallback Issuer per `pubkeys.json` rotation procedure :
     ```bash
     helm upgrade efe charts/efe \
         --set mtls.issuerRef.name=fallback-ca-issuer \
         --reuse-values
     ```

### Longer mitigations (15-120 min)

  3. **Manual cert provision** (if cert-manager fully broken) :
     Use `cmctl` CLI to provision a temporary cert :
     ```bash
     cmctl create cert efe-mtls-temp \
         --duration 720h \
         --renew-before 168h \
         --dns-names efe-server.efe.svc.cluster.local
     ```

  4. **Disable mTLS temporarily** (LAST RESORT) :
     Reduces security posture but restores availability :
     ```bash
     helm upgrade efe charts/efe \
         --set mtls.enabled=false \
         --reuse-values
     ```
     **MUST** notify customers + post-mortem within 48h.

### Root cause investigation

  5. Check cert-manager controller logs :
     ```bash
     kubectl logs -n cert-manager deployment/cert-manager | \
         grep efe-mtls | tail -50
     ```
  6. If Issuer is `ClusterIssuer` backed by Vault : check Vault audit log
     for token expiry / sealing events.
  7. If Issuer is Let's Encrypt : check rate limit
     (`https://letsencrypt.org/docs/rate-limits/`).

## 5. Resolution criteria

  - `kubectl get certificate efe-mtls -n efe` shows `Ready=True`
  - Cert expiration > 60 days from now
  - Helm test `efe-test-mtls-handshake` passes
  - 0 customer reports of TLS failure for 1 hour

## 6. Post-incident actions

  - [ ] Add Prometheus alert if not already firing
    (`D22ObsCertExpiringSoon` should fire 14 days before expiry)
  - [ ] If Issuer was the root cause, document in `HANDOFF_OPS.md`
    sub-processor list with backup contact
  - [ ] If renewal failure pattern recurs > 2x per quarter, evaluate
    Issuer replacement
  - [ ] Add anti-regression pin test if root cause is Helm template :
    `test_m12_d22_*` already covers most invariants
  - [ ] Update runbook based on learnings

## 7. Related runbooks

  - `d15_oidc_jwks_stale.md` — adjacent JWKS fetching pattern
  - `m13_signing_key_rotation.md` — overall key rotation procedure
  - `general_incident_playbook.md` — escalation tree if customer-impacting
```

---

## 23. Appendix C — DDQ skeleton (200+ questions)

```markdown
# EFE Customer Due Diligence Questionnaire — Pre-fill

## A. Application Security (25 questions)
### Q-AS-01 — Does the system implement secure SDLC ?
### Q-AS-02 — Are dependencies scanned for vulnerabilities ?
... (23 more)

## B. Audit Assurance (18 questions)
### Q-AA-01 — Is the system SOC 2 Type II certified ?
### Q-AA-02 — Is there an annual external audit ?
... (16 more)

## C. Business Continuity (12 questions)
### Q-BC-01 — What is the RTO ? RPO ?
### Q-BC-02 — How frequently is DR tested ?
... (10 more)

[... 14 more sections, 200+ questions total]
```

(Full content : ~1 800 lignes — out of scope for this architecture
preview ; will be detailed in `docs/compliance/ddq_prefill.md` D41 ship.)

---

## 24. Appendix D — release notes 1.0 skeleton

```markdown
# EFE 1.0.0 — Release Notes

> **Release date** : 2026-XX-XX
> **Type** : First stable General Availability release
> **Code freeze** : 2026-XX-XX
> **Tag** : `v1.0.0` (signed by release authority key)
> **Documentation** : https://docs.efe.ai/1.0/

## 🎉 Highlights

EFE 1.0 is the first stable, production-ready release after 18 months
of development across 14 milestones (M1-M14). Headline features :

  - Multi-tenant semantic classification gRPC server (M1-M9)
  - 4 SDKs (Python M8, Go M9, TypeScript M10, Java M11)
  - Production Helm chart (M12)
  - Ed25519-signed model bundles + OCI Artifacts (M13)
  - SOC 2 + ISO 27001 + GDPR + AI Act + NIST 800-53 audit-ready (M14)

## 📋 Compatibility matrix
[... full matrix ...]

## ⚠ Breaking changes (vs 0.x)
[... migration items ...]

## 🚀 Migration guide (0.x → 1.0)
[... step-by-step ...]

## ✨ New features (M11-M14)
[... per-sprint summary ...]

## ⚡ Performance improvements
[... benchmarks ...]

## 🔒 Security improvements
[... CVE fixes + new controls ...]

## 📊 SLAs (paid support tiers)
[... matrix ...]

## 🛡 Long-Term Support (LTS) policy
1.0 LTS through 2031-12-31 (5 years).

## 🐛 Known issues
[... transparently disclosed ...]

## 📅 Deprecation notices
[... post-1.0 sunset items ...]

## 🙏 Acknowledgments
[... contributors + customers + auditors ...]
```

(Full content : ~1 200 lignes — detailed in `docs/RELEASE_NOTES_1.0.md`
D42 ship.)

---

## 25. Appendix E — pin tests inventory M14

### 25.1 `test_m14_compliance_docs_present.py` (8 tests)

```python
def test_m14_iso27001_md_present_and_nonempty():
def test_m14_soc2_evidence_md_covers_cc1_cc9():
def test_m14_gdpr_art32_md_covers_4_mesures():
def test_m14_ai_act_annex_iv_md_covers_8_sections():
def test_m14_nist_800_53_md_covers_17_families():
def test_m14_threat_model_md_consolidates_5_sources():
def test_m14_ddq_prefill_md_has_200_plus_questions():
def test_m14_continuous_compliance_md_present():
```

### 25.2 `test_m14_runbook_inventory.py` (6 tests)

```python
def test_m14_runbook_count_at_least_25():
def test_m14_runbook_naming_convention_dXX_or_mXX():
def test_m14_each_runbook_has_symptom_section():
def test_m14_each_runbook_has_mitigation_section():
def test_m14_each_runbook_has_resolution_section():
def test_m14_each_runbook_has_severity_metadata():
```

### 25.3 `test_m14_release_notes_1_0.py` (3 tests)

```python
def test_m14_release_notes_1_0_md_present():
def test_m14_release_notes_has_breaking_changes_section():
def test_m14_release_notes_has_migration_guide():
```

### 25.4 `test_m14_freeze_branch_protection.py` (3 tests)

```python
def test_m14_branch_protection_yml_present():
def test_m14_branch_protection_requires_2_reviewers():
def test_m14_branch_protection_requires_signed_commits():
```

### 25.5 `test_m14_threat_model_consolidated.py` (5 tests)

```python
def test_m14_threat_model_references_m12_helm_threats():
def test_m14_threat_model_references_m13_bundle_threats():
def test_m14_threat_model_references_d11_isolation_threats():
def test_m14_threat_model_references_d14bis_audit_threats():
def test_m14_threat_model_uses_stride_categorization():
```

**Total M14 pin tests : 25**

---

## 26. Appendix F — glossaire

| Term | Definition |
|---|---|
| **AICPA** | American Institute of Certified Public Accountants — owns SOC 2 standard |
| **AI Act** | EU Regulation 2024/1689 on Artificial Intelligence (effective 2026-08-02) |
| **Annex A** | ISO 27001 controls list (114 controls in 4 themes : Organizational, People, Physical, Technological) |
| **Annex IV** | EU AI Act Annex IV — technical documentation requirements for high-risk AI |
| **Bridge letter** | SOC 2 Type II addendum extending audit period coverage |
| **CAIQ** | Cloud Security Alliance Consensus Assessments Initiative Questionnaire (v4.0.2) |
| **CC1-CC9** | SOC 2 Common Criteria for Security (9 categories) |
| **CISO** | Chief Information Security Officer |
| **CRA** | EU Cyber Resilience Act 2024 (effective 2027-12) |
| **CSA CCM** | Cloud Security Alliance Cloud Controls Matrix |
| **CTO** | Chief Technology Officer |
| **DDQ** | Due Diligence Questionnaire (customer security/compliance survey) |
| **DPA** | Data Processing Agreement (per GDPR Art. 28) |
| **DPIA** | Data Protection Impact Assessment (per GDPR Art. 35) |
| **DPO** | Data Protection Officer (per GDPR Art. 37) |
| **EOL** | End of Life |
| **EU DPA** | EU Data Protection Authority (e.g., CNIL France, BfDI Germany) |
| **FedRAMP** | US Federal Risk and Authorization Management Program |
| **GA** | General Availability |
| **GDPR** | General Data Protection Regulation (EU 2016/679) — RGPD in French |
| **HIPAA** | US Health Insurance Portability and Accountability Act |
| **IP** | Intellectual Property |
| **ISO 27001** | International Organization for Standardization — Information Security Management Systems standard (2022 revision) |
| **LTS** | Long-Term Support |
| **NIS2** | EU Network and Information Security Directive 2 (2022/2555) |
| **NIST 800-53** | NIST Special Publication 800-53 — Security and Privacy Controls |
| **OSCP** | Offensive Security Certified Professional (pen-test cert) |
| **PCI DSS** | Payment Card Industry Data Security Standard |
| **PII** | Personally Identifiable Information |
| **RACI** | Responsible / Accountable / Consulted / Informed (matrix) |
| **RGPD** | French/EU acronym for GDPR |
| **RPO** | Recovery Point Objective (data loss tolerance) |
| **RTO** | Recovery Time Objective (downtime tolerance) |
| **SaaS** | Software as a Service |
| **SBOM** | Software Bill of Materials |
| **SCC** | Standard Contractual Clauses (EU 2021/914 — international transfers) |
| **SIG Lite** | Shared Assessments Standardized Information Gathering questionnaire |
| **SLA** | Service Level Agreement |
| **SLO** | Service Level Objective |
| **SLSA** | Supply-chain Levels for Software Artifacts (Google framework) |
| **SoA** | Statement of Applicability (ISO 27001 mandatory document) |
| **SOC 2** | AICPA Service Organization Control 2 — operational controls audit |
| **STRIDE** | Spoofing/Tampering/Repudiation/Information Disclosure/DoS/Elevation (threat model framework) |
| **TLS** | Transport Layer Security |
| **VDP** | Vulnerability Disclosure Program |

---

**Fin du document M14 architecture extension.**

> Volumétrie totale : ~2 600 lignes (cible ≥ 2 500). Suit le pattern
> M12 (2 269) + M13 (2 691) + D16 (2 213). Couverture : 26 sections
> + 6 appendices + 25 pin tests.
>
> Prochaine étape (post-architecture) : code drop ultra-détaillé en
> 3 sprints distincts (D40 → D41 → D42) suivant le pattern M12/M13
> livraison + audit pass 1 + freeze announcement.
>
> **🔒 Post-D42 : repo entre en mode operate-only. Code freeze
> effectif le J175. EFE 1.0 GA officiellement released.**
