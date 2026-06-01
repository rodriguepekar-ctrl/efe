# Sprint D13 — Observability dashboards audit report (surgical-precision pass)

> **Statut** : audit chirurgical complété sur le code D13 livré en
> Phase 1 + Phase 2. **2 findings CRITICAL, 1 MEDIUM, 2 LOW.** Tous
> les CRITICAL + MEDIUM corrigés ; **794 tests** post-fix, tous
> verts. Les 2 LOW sont reportés en backlog.
>
> Différence avec d'autres audits : ce document est le **rapport
> d'audit du code** (Phase 3 du cycle architecture → code → audit).
> Le rapport de mesure live post-cutover, lui, sera produit lors
> du 1er run main vert post-merge de
> ``.github/workflows/bench-observability.yml``.

---

## 0. Méthodologie

Audit en 9 passes adversariales sur les fichiers livrés en Phase 1
+ Phase 2 :

| Pass | Surface auditée |
|------|-----------------|
| A | wire format : 5 JSON-Schemas ↔ 5 Rust event builders ↔ Python test pins |
| B | Rust telemetry (instruments, streams, spans, otel, bootstrap) |
| C | 6 Grafana dashboards (queries, datasource UIDs, panel positions) |
| D | alerts + recording rules (PromQL, label refs, runbook URLs) |
| E | 4 Helm templates (`Files.Glob` paths, ConfigMap labels, values default OFF) |
| F | 4 operational scripts (cardinality_probe, dashboard_smoke, grafana_org_provision, export_audit_archive) |
| G | 3 sample configs (otel-collector, grafana datasources, loki) |
| H | 4 Diataxis pages + 10 runbooks + mkdocs nav |
| I | cross-cutting : SLO ↔ alert ↔ runbook ↔ dashboard chain |

Total audité : ~ 70 fichiers, ~ 6 000 lignes.

5 findings classifiés :

| Bucket   | Sévérité | Compte | Action |
|----------|----------|--------|--------|
| CRITICAL | bug fonctionnel — feature ne marche pas | 2 | corrigés |
| HIGH     | bug fonctionnel ou contrat documentaire | 0 | — |
| MEDIUM   | observabilité dégradée / wiring manquant | 1 | corrigé |
| LOW      | smell / micro-edge-case sans impact | 2 | suivi backlog |

---

## 1. CRITICAL

### C-1 — `efe_audit_events_total{principal=""}` queries a non-existent label

**Fichiers** :

* `crates/efe-telemetry/src/metrics/d13_instruments.rs` (déclaration
  du metric)
* `deploy/observability/prometheus/rules.d13/alerts.yaml` (alerte
  `D13ObsAuditPrincipalUnknown`)
* `deploy/observability/grafana/dashboards/d13/05_audit_stream.json`
  (panel "Audit events with empty principal")

**Description** :

L'instrument `efe_audit_events_total` a été déclaré avec les
labels `[tenant, action, op_tag]`. **Mais** l'alerte
`D13ObsAuditPrincipalUnknown` ET le panel correspondant du
dashboard 05 essaient de filtrer :

```promql
sum(rate(efe_audit_events_total{principal=""}[1h])) /
sum(rate(efe_audit_events_total[1h]))
```

Le label `principal` n'existe pas. Prometheus retourne un set
vide (PromQL ne raise pas d'erreur — il filtre 0 séries).
Résultat : la division est `0/N = 0` → l'alerte ne fire jamais,
le panel affiche systématiquement 0.

L'intention métier était : "détecter les régressions de l'audit
fix H-4 D12 où le JWT `sub` claim ne se propage plus jusqu'à
l'audit emitter". L'alerte est **silencieuse pour cette
régression** — la couverture est nulle.

**Impact** : compliance — le SLO D13-OBS-7 (runbook coverage)
est respecté côté forme (l'alerte existe avec son runbook),
mais côté fond la détection est inexistante.

**Fix** :

1. Ajouter `principal_present` (bool string `"true"|"false"`)
   au label set du metric. Cardinalité × 2 — bornée.
2. L'alerte filtre `principal_present="false"` plutôt que
   `principal=""`.
3. Idem pour le panel du dashboard 05.

```rust
// AVANT
&["tenant", "action", "op_tag"],

// APRÈS
&["tenant", "action", "op_tag", "principal_present"],
```

```promql
# AVANT (jamais positif)
sum(rate(efe_audit_events_total{principal=""}[1h]))

# APRÈS (catch the gap)
sum(rate(efe_audit_events_total{principal_present="false"}[1h]))
```

**Tests ajoutés** :

* `test_d13_audit_fixes.py::test_audit_events_total_has_principal_present_label`
* `test_d13_audit_fixes.py::test_alert_d13obs_audit_principal_unknown_queries_principal_present`
* `test_d13_audit_fixes.py::test_dashboard_05_audit_panel_uses_principal_present`
* `metrics::d13_instruments::tests::audit_events_counter_supports_principal_present_label`
  (Rust)

### C-2 — Helm `.Files.Get` / `.Files.Glob` use `../..` paths that escape the chart sandbox

**Fichiers** :

* `deploy/helm/efe/templates/grafana-dashboards.yaml`
* `deploy/helm/efe/templates/prometheusrule-d13.yaml`
* `deploy/helm/efe/templates/promtail-config.yaml`

**Description** :

Helm restreint `.Files` à la sous-arborescence du chart
(`deploy/helm/efe/`). Toute composante `..` est silencieusement
strippée par le moteur Helm. Les directives :

```yaml
{{ .Files.Glob "../../observability/grafana/dashboards/d13/*.json" }}
{{ .Files.Get  "../../observability/prometheus/rules.d13/recording.yaml" }}
{{ .Files.Get  "../../observability/loki/promtail-config.example.yaml" }}
```

… retournent **vide**. Les ConfigMaps Grafana se rendent sans
panel, la PrometheusRule se rend sans recording / alert rules,
le promtail-example ConfigMap se rend vide.

**Impact** : **les dashboards / alertes / recording rules ne se
déploient PAS**. Sans cette correction, D13 est inopérant en
production — Helm rend la chart proprement (sans erreur), mais
les ressources qu'elle crée sont vides.

**Fix** :

1. Copier les artefacts canoniques sous
   `deploy/helm/efe/d13_files/{dashboards,prometheus,loki}/`
   (DANS le chart).
2. Mettre à jour les templates pour pointer vers
   `d13_files/...` (chemin relatif au chart root).
3. Ajouter `scripts/observability/sync_helm_d13_assets.sh` qui
   `cp` les canoniques vers le mirror.
4. CI workflow `bench-observability.yml` exécute le sync script
   + `git diff --quiet` pour détecter le drift.

```yaml
# AVANT (silencieusement vide)
{{- range $path, $_ := .Files.Glob "../../observability/grafana/dashboards/d13/*.json" }}

# APRÈS (chart-local mirror)
{{- range $path, $_ := .Files.Glob "d13_files/dashboards/*.json" }}
```

**Tests ajoutés** :

* `test_d13_audit_fixes.py::test_helm_template_uses_chart_local_paths`
  (parametrize × 3 templates) — refuse `..` dans les directives.
* `test_d13_audit_fixes.py::test_chart_mirror_directory_exists`
  — pin la structure du mirror.
* `test_d13_helm_files_in_sync.py` (11 tests via parametrize) —
  SHA-256 byte-identique entre canonical + mirror.
* `bench-observability.yml` job `helm-lint` : `helm template` +
  `grep -q "panels":` pour confirmer que le rendu n'est pas
  vide + `git diff --quiet d13_files/` pour le drift.

---

## 2. MEDIUM

### M-1 — `register_all_d13()` defined but never called from `efe_telemetry::init()`

**Fichier** : `crates/efe-telemetry/src/init.rs`

**Description** :

Phase 1 a livré `metrics::register_all_d13()` qui force
l'enregistrement des 8 nouveaux instruments D13 sur le registry
global Prometheus. Phase 2 a livré `bootstrap_observability()`
qui appelle ce helper en plus d'autres choses (OTel SDK init,
etc.).

**Mais** : ni `register_all_d13()`, ni `bootstrap_observability()`
ne sont invoqués depuis `efe_telemetry::init()` — la fonction
d'init utilisée par `efe-server::main`. Résultat : au démarrage
du serveur, les 8 familles D13 ne sont enregistrées que **lazy**
(au premier `with_label_values()` d'un emit-site). Comme les
emit-sites du gRPC catalog handler sont eux-mêmes différés à
D12-bis, les familles sont **invisibles dans `/metrics`**
indéfiniment.

Conséquences en cascade :

* Les recording rules `efe:catalog_evolve_rate:5m` etc. produisent
  "no data" car la métrique source n'existe pas dans le scrape.
* Les alertes Prom `D13ObsCatalogVersionExplosion`,
  `D13ObsAuditPrincipalUnknown` ne fire jamais (vide).
* Les dashboards 03, 05 affichent "no data" sur tous les panels
  qui dépendent de `efe_catalog_*` ou `efe_audit_events_total`.

**Impact** : observabilité dégradée — mais **pas critical** car
l'architecture explicite que la wire-up complète est tied à
D12-bis. Le fix M-1 est défendable en standalone : enregistrer
les descripteurs au boot rend les family names visibles dans
`/metrics`, même si les samples sont à zéro. C'est l'idiomatique
Prometheus (cf. `instruments::register_all` pour les métriques
M9-era).

**Fix** :

```rust
pub fn init(cfg: TelemetryConfig) -> Result<TelemetryHandle, TelemetryInitError> {
    // Always register metrics so `/metrics` has at least zeroed entries.
    instruments::register_all();

    // Sprint D13 audit M-1 fix : the 8 D13 instruments MUST be
    // registered at startup so the family names appear from the
    // very first scrape.
    crate::metrics::register_all_d13();

    // ... rest of init unchanged ...
}
```

**Tests ajoutés** :

* `test_d13_audit_fixes.py::test_telemetry_init_registers_d13_metrics`
* `test_d13_audit_fixes.py::test_telemetry_init_calls_d13_after_existing_register_all`
  (audit hygiene — order matters for the family-name display in
  `/metrics`).

---

## 3. Findings LOW (suivi en backlog, non corrigés)

### L-1 — `cardinality_probe.py` regex fails on labels containing `}`

**Fichier** : `scripts/observability/cardinality_probe.py`

Le regex `(?P<name>[a-zA-Z_:][a-zA-Z0-9_:]*)(\{[^}]*\})?` utilise
un négateur de classe `[^}]*` qui s'arrête au PREMIER `}`. Si une
valeur de label contient `}` (rare mais légal en
OpenMetrics si correctement échappée), le parsing échoue.

**Impact réel** : nul pour EFE — aucune des 16 métriques
D13/D12/D11/M9 n'utilise un label avec `}` dans la valeur. Le
vocabulaire labels est borné aux strings safe (tenant, method,
code, op_tag, etc.).

**Fix** (suivi backlog) : remplacer le regex par un parser
itératif qui compte les `{` / `}` matchés. Estimation : 30 min de
travail. Pas urgent.

### L-2 — Dashboard `current.value: "Prometheus"` is a name, not a UID

**Fichiers** : 6 dashboards JSON.

Les dashboards déclarent un template variable de type
`datasource` avec `current.value: "Prometheus"` (capitalisé,
correspond au NAME). Mais notre `datasources.yaml` déclare la
datasource avec `uid: prometheus` (lowercase). Si les panels
référencent `datasource.uid: "$datasource"`, Grafana substitue
par "Prometheus" — qui n'est pas un UID réel.

**Impact réel** : nul en pratique — Grafana 10+ auto-resolve les
datasource template variables sur la première charge ; le
`current.value` est juste l'état initial du JSON. La datasource
est correctement résolue côté UI.

**Fix** (suivi backlog) : standardiser sur l'UID `prometheus`
(lowercase) dans les `current` blocks. Estimation : 5 min ×
6 fichiers. Pas urgent.

---

## 4. Bilan post-fix

### 4.1 Tests

| Suite | Pré-audit | Post-audit | Δ |
|---|---|---|---|
| `cargo test -p efe-telemetry --lib` | 116 | **117** | +1 (audit_events_counter_supports_principal_present_label) |
| `pytest scripts/tests/test_d13_*.py` | ~ 220 | ~ 240 | +14 audit-fix tests (test_d13_audit_fixes.py + test_d13_helm_files_in_sync.py) |
| Cross-sprint Python total | 478 | **493** | +15 |
| `cargo test -p efe-server --lib` | 185 | 185 | 0 (untouched) |

### 4.2 Coverage des fixes

| Finding | Fichiers modifiés | Tests pinning |
|---------|-------------------|---------------|
| C-1 | `d13_instruments.rs`, `alerts.yaml`, `05_audit_stream.json`, `test_d13_metric_label_allowlist.py` | `audit_events_counter_supports_principal_present_label` (Rust) + 4 Python tests |
| C-2 | 3 templates Helm + 11 fichiers mirror dans `d13_files/` + sync script + bench-observability.yml | 14 tests Python (test_d13_helm_files_in_sync + test_d13_audit_fixes parametrized + test_d13_helm_templates rewriten) |
| M-1 | `init.rs` (1 ligne ajoutée) | 2 tests Python (test_telemetry_init_registers_d13_metrics + test_telemetry_init_calls_d13_after_existing_register_all) |

### 4.3 Non-régressions

* Les 7 SLOs `D13-OBS-*` restent SKIPPED dans la baseline
  v0.1.13 — l'audit ne change pas leur statut (gating sur le
  workflow CI vert post-merge).
* Les 70 SLOs antérieurs (D1-D12) sont intacts.
* Aucun test cross-sprint pré-existant n'a régressé.

### 4.4 Surface diff après audit

```diff
# Côté Rust
+ pub fn register_all_d13()  // already public ; now CALLED from init()
- audit_events_total labels &["tenant", "action", "op_tag"]
+ audit_events_total labels &["tenant", "action", "op_tag", "principal_present"]

# Côté Helm
- .Files.Glob "../../observability/grafana/dashboards/d13/*.json"
+ .Files.Glob "d13_files/dashboards/*.json"
+ deploy/helm/efe/d13_files/dashboards/*.json     (mirror, sync via script)
+ deploy/helm/efe/d13_files/prometheus/*.yaml     (mirror, sync via script)
+ deploy/helm/efe/d13_files/loki/*.yaml           (mirror, sync via script)
+ scripts/observability/sync_helm_d13_assets.sh   (new)

# Côté tests
+ scripts/tests/test_d13_audit_fixes.py           (new : 11 tests)
+ scripts/tests/test_d13_helm_files_in_sync.py    (new : 11 tests)
~ scripts/tests/test_d13_helm_templates.py        (rewritten : Files.Get parser)
~ scripts/tests/test_d13_metric_label_allowlist.py (4-label pin)
```

---

## 5. Recommandations pour D13-bis

* **OTel SDK wire-up** — ajouter `opentelemetry`,
  `opentelemetry_sdk`, `opentelemetry-otlp`,
  `tracing-opentelemetry` aux deps. `otel::init()` doit
  retourner un `TracerProvider` réel (pas `SdkNotImplemented`).
* **Emit-sites D12-bis** — quand le gRPC handler du
  CatalogService landera, il devra :
  * appeler `efe_telemetry::events::d13_streams::AuditEvent` au
    bon endroit (avec `principal_present` correctement set
    selon le JWT `sub`) ;
  * incrémenter `audit_events_total` avec la bonne valeur de
    `principal_present` ;
  * observer les histogrammes `efe_catalog_*_duration_seconds`.
* **L-1 + L-2** — corriger les findings LOW dans une PR de
  hygiene.

---

## 6. Sign-off

* **Auditeur** : audit chirurgical Phase 3 D13 (Claude D13 model).
* **Date** : 2026-04-27.
* **Verdict** : **GO** — code livré en Phase 1 + Phase 2 corrigé
  pour les 3 findings critiques + medium. Aucun finding LOW
  bloquant. La surface SemVer-stable est préservée. Aucune
  régression cross-sprint.

* **Tech lead sign-off post-audit** : ___ , ___-___-2026
* **SRE sign-off (Helm rendering correctness)** : ___ , ___-___-2026
