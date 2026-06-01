# Sprint D11 — Per-tenant isolation report (skeleton)

> **Statut** : skeleton à compléter dès l'arrivée des Rust isolation
> tests + 3 weeks de runs verts post-merge sur le workflow
> `bench-isolation.yml`. Suit `d6_accuracy_validation.md`,
> `d8_decoder_comparison.md`, `d9_load_soak_report.md`,
> `d10_dx_report.md` dans la série des rapports d'audit chiffrés.

---

## 1. Contexte

D11 livre l'industrialisation multi-tenant (cf. spec
`docs/bench/d11_per_tenant_isolation_architecture.md`). Cette
section vide se remplit quand :

1. La PR de freeze v0.1.11 est mergée.
2. Les 4 fichiers Rust `d11_isolation_*.rs` sont implémentés
   (~30 tests).
3. 3 main runs verts de `bench-isolation.yml`.
4. Au moins un partenaire pilote est onboardé en Tier 1 ou Tier 2.

> **À la première mesure live** : remplacer ce skeleton par les
> chiffres réels et les commentaires d'analyse (cf.
> `d10_dx_report.md` pour le format).

---

## 2. Mesure live D11.1 — Server hardening

| Aspect | Cible | Mesuré | Statut |
|--------|------:|-------:|:------:|
| `assert_tenant_match` helper + 4 call sites | livré | TBD | ⏸ |
| Idempotency cache `(tenant, request_id)` keying | confirmé existant (M11) | ✅ | ✅ |
| `AdminService::list_catalogs` filter par JWT | livré | TBD | ⏸ |
| 30+ Rust isolation tests | livré | TBD | ⏸ |
| 2 nouvelles métriques (compute_seconds, egress_bytes) | livré | TBD | ⏸ |

---

## 3. Mesure live D11.2 — Helm Tier 2

| Vérif | Statut |
|-------|:------:|
| `templates/resourcequota.yaml` + opt-in toggle | ✅ livré |
| `templates/limitrange.yaml` + opt-in toggle | ✅ livré |
| `templates/networkpolicy-tenant.yaml` + opt-in toggle | ✅ livré |
| `templates/servicemonitor.yaml` extension (relabel `tenant`) | ✅ livré |
| `templates/prometheusrule.yaml` extension (4 alertes per-tenant) | ✅ livré |
| `values.yaml` block + `values.tier2.yaml` | ✅ livré |
| `templates/tests/test-cross-tenant-denial.yaml` | ✅ livré (opt-in) |
| `helm lint` clean | ⏸ (à vérifier post-merge) |
| `helm template` Tier 2 dual-tenant smoke | ⏸ (CI workflow on PR) |

---

## 4. Mesure live D11.3 — Observabilité

| Vérif | Statut |
|-------|:------:|
| Grafana dashboard JSON committed | ✅ |
| `docs/operations/multi-tenant-observability.md` | ✅ |
| Per-tenant alerts in PrometheusRule (4 alertes) | ✅ livré |

---

## 5. Mesure live D11.4 — SDK admin

| Vérif | Statut |
|-------|:------:|
| `efe_tool.admin` sub-package (`__init__.py`, `client.py`) | ✅ livré |
| `EfeAdminClient` constructeur + 6 méthodes | ✅ livré |
| `__all__` étendu avec EfeAdminClient/CatalogRef/AdminError | ✅ livré |
| 3 doc pages Diataxis (howto/multi-tenant, explanation/security-model, reference/tenant-claims) | ✅ livré |
| `mkdocs.yml` nav étendu | ✅ livré |
| `test_d11_surface_extensions.py` 11 tests | ✅ verts |

---

## 6. Mesure live D11.5 — Cost attribution

| Vérif | Statut |
|-------|:------:|
| `scripts/billing/aggregate_usage.py` | ✅ livré (300+ LOC, stdlib-only) |
| `scripts/billing/tests/test_aggregate_usage.py` (20 tests) | ✅ verts |
| `docs/operations/billing.md` runbook | ✅ livré |

---

## 7. Mesure live D11.6 — CI integration

| Vérif | Statut |
|-------|:------:|
| `.github/workflows/bench-isolation.yml` | ✅ livré |
| 7 SLOs `D11-MT-*` dans `parse_criterion_report.py` | ✅ livré |
| `RESERVED_PREFIXES` étendu à 9 namespaces | ✅ livré |
| `scripts/tests/test_d11_disjoint_keys.py` (8 tests) | ✅ verts |
| `scripts/tests/test_d11_surface_extensions.py` (11 tests) | ✅ verts |
| `scripts/tests/d11_helm_render_isolation.py` (CI script) | ✅ livré |

---

## 8. SLOs `D11-MT-*` — état

| SLO | Mesuré | Cible | Statut |
|-----|-------:|------:|:------:|
| `D11-MT-1` cross_tenant_leakage | TBD | = 0 | ⏸ |
| `D11-MT-2` rate_limit_disjoint_pct | TBD | ≥ 99 % | ⏸ |
| `D11-MT-3` audit_log_attribution_pct | TBD | = 100 % | ⏸ |
| `D11-MT-4` metric_label_coverage_pct | TBD | = 100 % | ⏸ |
| `D11-MT-5` aggregator_drift_pct | TBD | ≤ 1 % | ⏸ |
| `D11-MT-6` usage_export_latency_s | TBD | ≤ 60 s | ⏸ |
| `D11-MT-7` tenant_cardinality_count | TBD | ≤ 1 000 | ⏸ |

---

## 9. Anti-régression — toutes les SLOs antérieures

À remplir post-mesure : aucune des 26 SLOs antérieures
(D4 × 3, D6 × 5, D7 × 4, D8 × 7, D9 × 5, D10 × 5) ne doit régresser.

---

## 10. Décisions de gel D11

À la freeze de la baseline `v0.1.11` :

```
SDK version              : efe-tool==0.1.11 (PyPI)
Server image             : ghcr.io/efe-project/efe:0.1.11@sha256:TBD
Helm chart               : oci://ghcr.io/efe-project/charts/efe:0.1.11
Tier 1 default           : tenant.namespace.isolated=false (backward compat)
Tier 2 preset            : deploy/helm/efe/values.tier2.yaml
Cost aggregator          : scripts/billing/aggregate_usage.py
Grafana dashboard        : deploy/observability/grafana/per-tenant-dashboard.json
RESERVED_PREFIXES count  : 9 namespaces (3 new D11)
SLOs gated total         : 33 (26 inherited + 7 D11-MT-*)
```

Carry-over D7-D10 (winners) :

```
EFE_FUSION_BETA0          = 0.45
EFE_FUSION_LAMBDA_BETA    = 0.05
EFE_FUSION_ALPHA0         = 0.30
HYBRID_AGREE_MIN_CONF     = 0.45
HYBRID_SWARM_OVERRIDE_CONF= 0.85
service.headless          = false (opt-in)
tenant.id                 = "default"
tenant.namespace.isolated = false (Tier 1 default)
EFE_IDEMPOTENCY_TTL_MS    = (env, 0 in load tests, default in prod)
```

---

## 11. Hand-off vers D12

D12 démarre avec :

* Multi-tenant fully wired ✅
* Cost attribution functional ✅
* Per-tenant observability dashboards ✅
* SDK admin surface stable ✅

D12 livrera :

* Catalog hot-reload (sans redeploy) — admin endpoints UI ?
* Per-tenant catalog versioning UI ?
* Production-grade SRE pack pour Phase 2.

---

## 12. Next

Une fois ce rapport rempli :

1. Geler `bench_baselines/v0.1.11/baseline.json` avec les chiffres
   mesurés.
2. PR séparée pour flip `bench-isolation.yml` `continue-on-error`
   à false (3 main runs verts).
3. Annoncer `v0.1.11` (tag + GitHub Release + Slack partner channel).
4. Mettre à jour `BENCHMARKS.md § 13` avec les valeurs finales.

— **fin du skeleton** —
