# Sprint D10 — DX audit report (skeleton)

> **Statut** : skeleton à compléter dès la première mesure live
> reproductible — 3 reviewers externes ayant exécuté
> `scripts/validate_quickstart.sh` et un build CI vert sur `main`
> avec docs déployés sur `docs.efe.ai/0.1.10/`.

---

## 1. Contexte

D10 livre l'industrialisation DX (cf. spec
`docs/bench/d10_sdk_docs_examples_architecture.md`). Cette
section vide se remplit quand :

1. La PR de freeze v0.1.10 est mergée.
2. Le tag `v0.1.10` est poussé → `release.yml` publie image, chart,
   PyPI, docs.
3. `docs.efe.ai/0.1.10/` est accessible.
4. 3 reviewers externes ont exécuté la quickstart end-to-end et
   reporté leur wallclock.

> **À la première mesure live** : remplacer ce skeleton par les
> chiffres réels et les commentaires d'analyse (cf.
> `d8_decoder_comparison.md` et `d9_load_soak_report.md` pour le
> format).

---

## 2. Mesure live D10.1 — SDK surface freeze

| Aspect | Cible | Mesuré | Statut |
|--------|------:|-------:|:------:|
| `efe_tool.__version__` matches tag | `0.1.10` | TBD | ⏸ |
| Public symbol count in `__all__` | ≥ 25 | TBD | ⏸ |
| Legacy symbols still exported | 6 | TBD | ⏸ |
| New `Efe*Error` exports | 7 | TBD | ⏸ |
| `py.typed` marker present | yes | TBD | ⏸ |
| `pip install efe-tool==0.1.10` fresh-venv | success | TBD | ⏸ |

---

## 3. Mesure live D10.2 — Documentation

| Vérif | Statut |
|-------|:------:|
| `mkdocs build --strict` exit 0 | ⏸ |
| `linkchecker` 0 broken | ⏸ |
| API reference covers ≥ 95 % of `__all__` | ⏸ |
| Diátaxis structure: 4 sub-trees populated | ⏸ |
| `mike deploy 0.1.10 latest` push to gh-pages OK | ⏸ |
| `docs.efe.ai/0.1.10/` reachable + index renders | ⏸ |

---

## 4. Mesure live D10.3 — Examples + smoke tests

| Example | LOC budget | Mesuré LOC | Smoke status | Wallclock |
|---------|-----------:|-----------:|:------------:|----------:|
| `01_quickstart_classify.py` | ≤ 80 | TBD | ⏸ | TBD |
| `02_custom_catalog.py` | ≤ 200 | TBD | ⏸ | TBD |
| `03_multilang_streaming.py` | ≤ 200 | TBD | ⏸ | TBD |
| `04_production_client.py` | ≤ 280 | TBD | ⏸ | TBD |

---

## 5. Mesure live D10.4 — Quickstart DX

3 reviewers externes (à recruter), wallclock individuel :

| Reviewer | Cluster | Wallclock | Pre-flight blockers |
|----------|---------|----------:|---------------------|
| R1 | minikube | TBD s | TBD |
| R2 | EKS shared | TBD s | TBD |
| R3 | k3s VM | TBD s | TBD |
| **Median** | — | **TBD s** | — |

| SLO | Mesuré | Cible | Statut |
|-----|-------:|------:|:------:|
| `D10-DX-1` | TBD s | ≤ 600 | ⏸ |

---

## 6. Mesure live D10.5 — Publication

| Étape | Statut |
|-------|:------:|
| `release.yml` tag dry-run sur v0.1.10-rc1 | ⏸ |
| Image push `ghcr.io/efe-project/efe:0.1.10` | ⏸ |
| Chart push `oci://ghcr.io/efe-project/charts/efe:0.1.10` | ⏸ |
| PyPI publish (OIDC) `efe-tool==0.1.10` | ⏸ |
| `mike deploy 0.1.10 latest` | ⏸ |

---

## 7. Anti-régression — toutes les SLOs antérieures

À remplir post-mesure : aucune des 21 SLOs antérieures
(D4 × 3, D6 × 5, D7 × 4, D8 × 7, D9 × 5) ne doit régresser.

---

## 8. Décisions de gel D10

À la freeze de la baseline `v0.1.10` :

```
SDK version              : efe-tool==0.1.10 (pinned in PyPI)
Server image             : ghcr.io/efe-project/efe:0.1.10@sha256:TBD
Helm chart               : oci://ghcr.io/efe-project/charts/efe:0.1.10
Docs site                : https://docs.efe.ai/0.1.10/
Public Python surface    : 25+ symbols (D10 __all__)
Diátaxis pages           : 19 (3 tutorial + 7 howto + 5 reference + 4 explanation)
Examples LOC budget      : 800 total across 4 files
mike default alias       : `latest` → `0.1.10`
```

Carry-over D7-D9 (winners) :

```
EFE_FUSION_BETA0          = 0.45
EFE_FUSION_LAMBDA_BETA    = 0.05
EFE_FUSION_ALPHA0         = 0.30
HYBRID_AGREE_MIN_CONF     = 0.45
HYBRID_SWARM_OVERRIDE_CONF= 0.85
service.headless          = false (opt-in, pilot recommended : true)
tenant.id                 = "default"
EFE_IDEMPOTENCY_TTL_MS    = (env, 0 in load tests, default in prod)
```

---

## 9. Hand-off vers D11

D11 démarre avec :

* SDK Python publié sur PyPI ✅
* Docs publics sur `docs.efe.ai` ✅
* 4 examples copy-paste-ready ✅
* Quickstart end-to-end testé ✅

D11 livrera :

* Multi-tenant resource quotas (per-tenant ResourceQuota +
  LimitRange via le `tenant.id` label déjà propagé en D9.A9 +
  D10 fix INT1).
* Catalog admin endpoints (hot-reload sans redeploy).
* `AsyncEfeClient` native grpc.aio (vs thread-pool skeleton D10).

---

## 10. Next

Une fois ce rapport rempli :

1. Geler `bench_baselines/v0.1.10/baseline.json` avec les chiffres
   mesurés.
2. PR séparée pour flip `bench-load.continue-on-error` → `false`.
3. Annoncer `v0.1.10` (tag + GitHub Release + Slack partner channel).
4. Mettre à jour `BENCHMARKS.md § 12` avec les valeurs finales.

— **fin du skeleton** —
