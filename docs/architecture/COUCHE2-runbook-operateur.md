# Couche 2 — Runbook opérateur (mise en production de la frontière)

> Compagnon opérationnel de [`COUCHE2-M4-gate-production-ARCHITECTURE.md`](./COUCHE2-M4-gate-production-ARCHITECTURE.md) et du [modèle de confiance](./COUCHE2-M4-modele-confiance.md). · 2026-07-12
> Cible : déployer et opérer `efe-swarm-gateway` avec la **frontière** (`/v1/llm/*`) activée, **sans se tirer une balle dans le pied**.

---

## 0. Les deux chemins (à ne jamais confondre)

| | **Essaim** (`/v1/swarm/*`) | **Frontière** (`/v1/llm/*`) |
|---|---|---|
| Nature | **Souverain** (SLM local / self-host) | **Escape-hatch NON-souverain** |
| Egress | aucun (ou routeur gouverné) | prompt+contexte **EN CLAIR** vers Anthropic/OpenAI/Google |
| Coût | compute local | **argent réel** par appel |
| Activation | toujours monté | **opt-in** : feature `frontier-llm` **+** `EFE_LLM_FRONTIER_ENABLED=true` |
| Gate de boot | master HKDF recommandé | **triade OBLIGATOIRE** (§2) — sinon `exit(78)` |

**Par défaut la frontière est OFF.** Ce runbook ne concerne QUE son activation. Le chemin essaim tourne sans rien de tout ceci.

---

## 1. Prérequis de l'image (sinon rien ne marche)

La Couche 2 est derrière la feature `frontier-llm` **à la compilation**. L'image DOIT être buildée avec :

```dockerfile
cargo build --release -p efe-swarm-gateway --bin efe-swarm-gateway \
  --features "postgres oidc pii frontier-llm"
```

> **Symptôme si `frontier-llm` manque** : `POST /v1/llm/vote` → **404** (route non montée), `EFE_LLM_FRONTIER_ENABLED=true` ignoré silencieusement, le boot guard M4 ne s'exécute jamais. Le `Dockerfile` du repo inclut désormais `frontier-llm`.

---

## 2. La triade de boot (go/no-go) — ce qui fait `exit(78)`

Quand `EFE_LLM_FRONTIER_ENABLED=true`, le binaire **refuse de démarrer** (code **78 / EX_CONFIG**) si l'un des trois manque :

| # | Invariant | Exigence | Message FATAL |
|---|---|---|---|
| 2 | **Non-forgeable** | `EFE_NANO_AUDIT_HMAC_MASTER` **≥ 32 octets** | « …provenance scellée sous test_key() PUBLIQUE ⟹ FORGEABLE » |
| 7 | **Authentifié** | feature `oidc` **+** `EFE_OIDC_PUBLIC_KEY_PEM` posé | « …egress tiers PAYANT sans authentification » |
| 3 | **Durable** | feature `postgres` **+** `DATABASE_URL` posé | « …reçus R4 perdus au restart » |

> La triade garantit qu'on ne sert **jamais** une frontière **forgeable**, **non authentifiée** ou **volatile**. Elle est calculée **avant** tout câblage : un `exit(78)` ne laisse aucun état dangereux assemblé. (Un master **non-blanc mais < 32 o** fait aussi `exit(78)`, tous chemins confondus — pas seulement la frontière.)

**Ce que la triade NE vérifie PAS** (⟹ à surveiller, cf. §6) : la présence des **clés provider** et d'une **allowlist**. Le préflight peut être vert alors que `/v1/llm/*` renvoie 501 (pas de clé) ou 403 (pas d'allowlist). C'est le rôle de `/readyz` (§5).

---

## 3. Matrice complète des variables d'environnement

**Frontière — requises pour servir** :

| Variable | Rôle | Requis |
|---|---|---|
| `EFE_LLM_FRONTIER_ENABLED` | interrupteur maître (**strict `=="true"`** ; `1`/`TRUE`/`yes` ⟹ OFF) | oui (activation) |
| `EFE_NANO_AUDIT_HMAC_MASTER` | master HKDF ≥ 32 o (SIGNE la provenance) | **triade #2** |
| `EFE_OIDC_PUBLIC_KEY_PEM` | clé PUBLIQUE ES256 (valide les JWT) | **triade #7** |
| `DATABASE_URL` | Postgres (reçus durables) | **triade #3** |
| `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GOOGLE_AI_KEY` | clés provider (≥ 1) — sinon coordinator absent ⟹ 501 | oui (≥ 1) |
| `EFE_LLM_TENANT_ALLOWLIST` | allowlist egress par tenant (§4) — absente ⟹ **403 partout** | oui |

**Frontière — optionnelles** :

| Variable | Rôle | Défaut |
|---|---|---|
| `EFE_OIDC_ISSUER_URL` | validation stricte de l'`iss` (essaim + frontière) | `iss` non validé |
| `EFE_OIDC_LLM_AUDIENCE` | audience épinglée du JWT frontière | `efe-llm` |
| `EFE_PII_DEFAULT_POLICY` | `off\|detect\|redact\|strict` — `redact`/`strict` caviardent le prompt AVANT egress | `off` (**prompt en clair**) |

**Essaim / commun** (rappel) : `EFE_OIDC_AUDIENCE` (aud essaim), `EFE_SWARM_ROUTER_ENDPOINT`/`_MODELS` (chemin Network essaim, distinct de la frontière), `EFE_GATEWAY_MAX_INFLIGHT_RUNS`/`_PER_TENANT` (admission), `EFE_GATEWAY_MAX_WIDTH`/`EFE_GATEWAY_MAX_DEADLINE_MS`, `EFE_SWARM_GATEWAY_ADDR` (bind, défaut `0.0.0.0:8080`).

> **`EFE_OLLAMA_ENDPOINT`/`_MODEL`** ne sont lues que par des tests `#[ignore]` — **PAS** de la configuration du binaire prod. À ignorer dans un runbook.

---

## 4. Le format de l'allowlist (le piège n°1 en pratique)

```
EFE_LLM_TENANT_ALLOWLIST="<tenant_uuid>=<id1>,<id2>[@cap=<cents>];<uuid2>=<id3>"
```

- **Les `<id>` DOIVENT égaler EXACTEMENT les ids enregistrés** : `claude_<model>`, `gpt_<model>`, `gemini_<model>` (le modèle est suffixé). Écrire `claude` (famille nue) ⟹ **400 « autorisé mais non enregistré »** systématique. **Les ids réels sont loggés au boot** (`→ providers frontière enregistrés (ids …) : […]`) — copiez-les.
- `@cap=<cents>` : plafond de coût **par requête** pour ce tenant (le budget client est borné à `min(req, cap)`). Sans cap, seul le `max_budget_cents` du client s'applique.
- Le tenant système = `00000000-0000-0000-0000-000000000001`.
- Une entrée **malformée** (pas de `=`, UUID invalide) est **ignorée silencieusement** — vérifiez le log de boot du nombre d'entrées parsées.

> **Rotation de modèle** = l'id change ⟹ l'allowlist (keyée sur l'ancien id suffixé) **cesse de matcher** (403/400 de masse). Re-synchronisez l'allowlist à chaque bump de modèle.

---

## 5. Sondes k8s

| Endpoint | Sémantique |
|---|---|
| `GET /healthz` | **liveness** : `200` si le process répond (sinon k8s redémarre). |
| `GET /readyz` | **readiness** : `200` normalement ; **`503`** si frontière **annoncée** (`EFE_LLM_FRONTIER_ENABLED=true`) mais coordinator **non câblé** (clés provider absentes) — retire le pod du load-balancer plutôt que servir des 501. |
| `GET /metrics` | Prometheus (ne plus utiliser comme sonde : répond 200 dès le bind). |

Les probes du chart pointent désormais liveness → `/healthz`, readiness → `/readyz`.

---

## 6. Observabilité du coût (le chemin PAYANT)

`/metrics` expose (M4 §2.4) :

- `efe_llm_frontier_calls_total` — appels LLM frontière scellés.
- `efe_llm_frontier_remote_calls_total` — appels à barreau **R4** (egress distant réel).
- `efe_llm_frontier_cost_microcents_total` — **coût egress cumulé** (1 µ¢ = 1/1000 ¢).
- `efe_llm_frontier_cost_by_tenant_microcents_total{tenant="…"}` — dépense **par tenant**.

**Alertes recommandées** : dérivée de `_cost_microcents_total` (pic de dépense), ratio `remote/calls` (fuite d'egress inattendue), `_cost_by_tenant` (tenant qui s'emballe malgré `@cap`).

---

## 7. Déploiement Helm (chart `efe-swarm-gateway`)

Le chart existe (profil par défaut = **self-host souverain**, egress deny-all). Pour la frontière, il faut fournir :

1. **Master HKDF** : `hkdf.enabled=true` ; le Secret est monté sous **`EFE_NANO_AUDIT_HMAC_MASTER`** (nom aligné sur `MASTER_ENV` — corrigé). En prod, préférez `hkdf.existingSecret` (Vault/externalSecrets).
2. **OIDC frontière** : `oidc.enabled=true` + Secret de la clé publique PEM + (option) `EFE_OIDC_LLM_AUDIENCE`.
3. **Postgres** : `DATABASE_URL` (via `gateway.extraEnv` ou un Secret).
4. **Clés provider** : `ANTHROPIC_API_KEY`/… en `secretKeyRef` (jamais en clair).
5. **Flag + allowlist** : `EFE_LLM_FRONTIER_ENABLED=true` + `EFE_LLM_TENANT_ALLOWLIST`.
6. **NetworkPolicy** : autoriser l'egress vers les endpoints providers (le défaut deny-all les bloque).

> **Dette connue** (à câbler dans le chart, cf. arch §G3) : il n'existe pas encore de bloc `frontier:` de haut niveau ; posez ces valeurs via `gateway.extraEnv` + Secrets en attendant. La NetworkPolicy d'egress providers est à ajouter.

---

## 8. Troubleshooting (symptôme → cause → correctif)

| Symptôme | Cause | Correctif |
|---|---|---|
| Pod `CrashLoopBackOff`, `exit 78` « …FORGEABLE » | `EFE_NANO_AUDIT_HMAC_MASTER` absent/< 32 o | poser un master ≥ 32 o (Secret) |
| `exit 78` « …sans authentification » | feature `oidc` absente **ou** `EFE_OIDC_PUBLIC_KEY_PEM` non posé | builder avec `oidc` + poser la clé publique |
| `exit 78` « …perdus au restart » | feature `postgres` absente **ou** `DATABASE_URL` non posé | builder avec `postgres` + poser `DATABASE_URL` |
| `exit 78` « …master invalide » | master fourni mais < 32 o (tous chemins) | corriger la longueur du master |
| `POST /v1/llm/*` → **404** | image buildée **sans** `frontier-llm` | rebuild avec `frontier-llm` (§1) |
| `POST /v1/llm/*` → **501**, `/readyz` = 503 | frontière ON mais **aucune clé provider** (coordinator absent) | poser ≥ 1 `*_API_KEY` |
| `POST /v1/llm/*` → **403** | pas d'allowlist, ou LLM hors allowlist, ou policy tenant absente | poser `EFE_LLM_TENANT_ALLOWLIST` avec le bon UUID tenant |
| `POST /v1/llm/*` → **400** « non enregistré » | id d'allowlist ≠ id réel (`claude_<model>`) | copier les ids du **log de boot** ; re-sync après rotation de modèle |
| `POST /v1/llm/*` → **429** + `Retry-After` | saturation d'admission | scaler ; ajuster `EFE_GATEWAY_MAX_INFLIGHT_*` |
| **Prompts en clair** vers le provider | `EFE_PII_DEFAULT_POLICY` non `redact`/`strict` (ou typo — désormais **avertie** au boot) | poser `redact`/`strict` si conformité requise |
| `verify` accepte un reçu forgé | master absent (repli `test_key` public) **ou** nom d'env divergent (Helm) | poser le master ; vérifier `hkdf.envName == EFE_NANO_AUDIT_HMAC_MASTER` |

---

## 9. Go-live checklist

- [ ] Image buildée `--features "postgres oidc pii frontier-llm"`.
- [ ] Secret master HKDF ≥ 32 o, monté sous `EFE_NANO_AUDIT_HMAC_MASTER`.
- [ ] `DATABASE_URL` posé, Postgres joignable (migrate OK au boot).
- [ ] Clé publique OIDC posée ; `EFE_OIDC_LLM_AUDIENCE` = `efe-llm` (défaut).
- [ ] ≥ 1 clé provider posée en Secret.
- [ ] `EFE_LLM_TENANT_ALLOWLIST` renseignée avec les **ids réels** (log de boot) + `@cap` par tenant.
- [ ] NetworkPolicy autorise l'egress providers.
- [ ] `EFE_PII_DEFAULT_POLICY=redact` si la conformité l'exige.
- [ ] Sondes k8s = `/healthz` (liveness) + `/readyz` (readiness).
- [ ] Alertes Prometheus sur `efe_llm_frontier_cost_microcents_total`.
- [ ] `EFE_LLM_FRONTIER_ENABLED=true` **en dernier**, une fois tout le reste vert.
- [ ] Post-boot : `/readyz` = 200 ; un `POST /v1/llm/vote` de fumée → 200 + `GET /v1/swarm/verify/:id` ⟹ `verified=true`.
