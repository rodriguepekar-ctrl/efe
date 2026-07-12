# Couche 2 — M4 : le gate production (architecture, préparation complète)

> Document d'architecture · 2026-07-12 · branche `couche2-m0-frontier` (M0+M1+M2+M3 livrés/poussés, commit `f71f5b8`)
> Issu d'une reconnaissance chirurgicale (**5 agents**) + **2 passes adverses** (sécurité red-team ; honnêteté anti-sur-vente). Clôt [`COUCHE2-M3-surface-servie-ARCHITECTURE.md`](./COUCHE2-M3-surface-servie-ARCHITECTURE.md) et la [roadmap](./COUCHE2-roadmap.md) §118-129.

---

## 0. Verdict & principe

La roadmap présente M4 comme un jalon **« S » (1-2 j)** : une checklist go/no-go à 6 cases, purement défensive (fail-fast HMAC, Pg durable, guard re-synchronisé, doc de confiance, non-régression essaim). **L'analyse adverse renverse ce cadrage sur deux axes** — et ré-estime l'effort à **M (3-5 j)**.

**Axe 1 — la checklist a un trou de niveau BLOCKER qu'elle ne voit pas : l'authentification.** L'item #5 (auth `aud=efe-llm`) a été fermé en M3… mais **uniquement sous la feature `oidc`**. La checklist M4 n'exige `oidc` nulle part. Or un binaire `--features frontier-llm,postgres` (master valide + `DATABASE_URL`) **passe les 6 items** mais, sans `oidc`, l'extracteur `LlmCaller` retombe sur `dev_caller_from_headers` ([auth.rs:124-131](../../crates/efe-swarm-gateway/src/auth.rs)) : le client fournit **lui-même** `x-efe-tenant` **et** `x-efe-roles: operator`. Un appelant anonyme conduit alors l'egress **payant** Anthropic/OpenAI/Google, **facturé au tenant qu'il choisit**, en usurpant l'allowlist de n'importe qui (`ADV-1`, blocker). Le go/no-go doit donc devenir une **triade** : **non-forgeable** (#2) + **authentifié** (#7, NOUVEAU) + **durable** (#3).

**Axe 2 — le nom « Gate production » sur-promet devant Kovant.** Le cœur de la thèse EFE est la **souveraineté** (essaim local). Or `/v1/llm/*` envoie le prompt+contexte utilisateur **EN CLAIR** à des tiers US, **sans cap de coût** ni **redaction PII** ([frontier.rs](../../crates/efe-swarm-gateway/src/frontier.rs) sans `with_redactor` ; [pii.rs](../../crates/efe-swarm-gateway/src/pii.rs) ne caviarde que la *présentation en sortie*). Appeler cela « production » à un investisseur dont la thèse est le SLM-souverain invite l'accusation de sur-vente. Le code, lui, est déjà honnête (`routes/llm.rs:8-9` dit « chemin NON-souverain ») — c'est la **doc destinée à Kovant** qui doit porter ce cadrage, et **corriger deux faussetés internes** (voir §4).

**Donc M4 n'est pas un tampon.** M4 =
1. un **préflight de boot UNIQUE** — la triade non-forgeable/authentifié/durable (§1) ;
2. le **durcissement de l'egress payant** que la roadmap a omis : cap de coût, PII opt-in, clé Gemini en header, métriques (§2) ;
3. le **filet anti-drift** du guard SSRF + l'honnêteté sur ce qu'il couvre réellement (§3) ;
4. la **consolidation + correction** du modèle de confiance (§4).

> **Principe directeur** : sur un chemin qui **brûle de l'argent réel** et **sort les données du périmètre tenant**, chaque défaut de config doit **refuser de démarrer** (fail-fast), jamais dégrader en silence. Le reçu R4 est la dot ; un reçu forgeable, non authentifié ou non durable **vaut zéro**.

---

## 1. Le préflight de boot unique — la triade (le cœur de M4)

Aujourd'hui, `main.rs` prend **trois décisions découplées** : la clé (L32-33/41), la frontière (L58-70), le store (L73-84), l'auth OIDC (L87-99). Rien ne les **lie**. Résultat : la frontière peut être servie sous une clé publique, sans auth, sur un store volatil — trois blockers indépendants. M4 introduit **un seul garde**, calculé depuis l'**intention env** (pas l'état déjà construit), inséré **juste après [main.rs:32](../../crates/efe-swarm-gateway/src/main.rs)** — donc **avant** `with_keys` (L41), `with_coordinator` (L62) et le service (L110). Aucun état forgeable/non-authentifié/volatil n'est même assemblé.

```rust
// main.rs — inséré après L32 (calcul de master_present), AVANT toute construction d'état.
// Le garde est PUR (testable sans muter l'env), sur le patron de key_source_for.

/// Le préflight go/no-go de la frontière (M4) : la triade de production.
/// Refuse de démarrer si la frontière est activée sans les 3 invariants.
#[cfg(feature = "frontier-llm")]
fn frontier_boot_guard(frontier_on: bool, key_ok: bool, auth_ok: bool, store_durable: bool) -> Result<(), &'static str> {
    if !frontier_on {
        return Ok(()); // frontière OFF ⟹ dev/essaim souverain intact (InMemory + test_key légitimes)
    }
    if !key_ok {
        return Err("EFE_LLM_FRONTIER_ENABLED exige EFE_NANO_AUDIT_HMAC_MASTER valide (≥ 32 o) — \
                    sinon la provenance est scellée sous test_key() PUBLIQUE ⟹ FORGEABLE.");
    }
    if !auth_ok {
        return Err("EFE_LLM_FRONTIER_ENABLED exige l'auth OIDC frontière (feature `oidc` + clé publique, \
                    aud=efe-llm) — sinon egress tiers PAYANT sans authentification (tenant/rôles en clair).");
    }
    if !store_durable {
        return Err("EFE_LLM_FRONTIER_ENABLED exige un store DURABLE (feature `postgres` + DATABASE_URL) — \
                    sinon les reçus R4 (la dot) sont perdus à chaque rollout.");
    }
    Ok(())
}
```

L'appel, avec les trois booléens **dérivés de l'intention env** (chacun reflétant exactement le contrat appliqué plus tard) :

```rust
// après L32 :
let frontier_on = frontier_flag_enabled();                                    // §1.4 — un seul parseur
#[cfg(feature = "frontier-llm")]
{
    // #2 : le MÊME contrat que le scellement (dry-run derive, PAS master_present — cf. §1.1).
    let key_ok = HmacKeySource::EnvHkdf.derive(efe_nano_sdk::TenantId::system()).is_ok();
    // #7 : la clé publique OIDC frontière est-elle configurée ? (env EFE_OIDC_PUBLIC_KEY_PEM)
    #[cfg(feature = "oidc")]
    let auth_ok = efe_swarm_gateway::OidcVerifier::llm_from_env().map(|o| o.is_some()).unwrap_or(false);
    #[cfg(not(feature = "oidc"))]
    let auth_ok = false; // feature oidc absente ⟹ pas d'auth ⟹ refus (ADV-1)
    // #3 : store durable = intention (postgres compilé + DATABASE_URL) ; connect/migrate re-valident en L75-76.
    let store_durable = cfg!(feature = "postgres") && std::env::var("DATABASE_URL").is_ok();

    if let Err(why) = frontier_boot_guard(frontier_on, key_ok, auth_ok, store_durable) {
        eprintln!("FATAL (M4 gate production) : {why}");
        std::process::exit(78); // EX_CONFIG (sysexits.h) — erreur de configuration, non-retryable
    }
}
```

### 1.1 #2 — Non-forgeabilité : DRY-RUN derive, jamais `master_present`

**Le piège** : un garde naïf `if frontier_on && !master_present { exit }` est **contourné par un master court**. `master_present` ([main.rs:32](../../crates/efe-swarm-gateway/src/main.rs)) ne teste que « non-blanc après trim », **pas la longueur**. Or `EnvHkdf::derive` ([keys.rs:59-70](../../crates/efe-audit-core/src/keys.rs)) rejette `master.len() < 32` (longueur **brute**, sans trim). Un master de 5 octets (`"short"`) passe `master_present=true` → `key_source_for` rend `EnvHkdf` → boot « PRODUCTION »… puis **chaque scellement échoue** (`state.derive_key` en [mission_graph.rs:325](../../crates/efe-swarm-gateway/src/mission_graph.rs)) ⟹ **500 systématique**, l'opérateur croyant la provenance active (`ADV-2`/`M4-2-SHORTKEY`, blocker).

**Le correctif** : le garde **appelle `derive(TenantId::system())`** — le contrat *identique* à ce qui scelle. Ce qui boote == ce qui scelle. Cela referme d'un seul point : master absent, master blanc, master < 32 o. **Ne pas** réimplémenter un check de longueur parallèle (il re-divergerait de `keys.rs`, `M4-2-TRIM-DERIVE-SKEW`). La dérivation tenant-système est représentative : la vérification de longueur précède HKDF et est indépendante du tenant.

### 1.2 #7 (NOUVEAU) — Authentification obligatoire : le blocker que la roadmap a manqué

C'est la découverte majeure de la passe adverse (`ADV-1`). La frontière est un egress **payant** vers un **tiers** ; la servir sans authentification cryptographique est **pire** qu'une provenance forgeable — c'est une **prise de contrôle de la facturation** de n'importe quel tenant. Le garde exige donc `auth_ok` : **feature `oidc` compilée ET `OidcVerifier::llm_from_env()` renvoie `Some`** (la clé publique `aud=efe-llm` est configurée). Sans elle → refus de boot.

**Ce que le garde NE casse pas** : il vit dans le **binaire prod** (`main.rs`). Les tests M3 (`--features frontier-llm` sans `oidc`) construisent l'état **directement**, hors `main.rs`, et gardent le chemin dev-headers légitime. Seul le **binaire de production** refuse de servir la frontière sans OIDC. Bon découplage : la sécurité mord là où l'egress réel est possible, sans amputer le harnais de test.

### 1.3 #3 — Durabilité : un `bool` HORS de tout `#[cfg]`, jamais un downcast

**Le piège** (`ADV-4`) : un garde de durabilité placé **à l'intérieur** du bloc `#[cfg(feature = "postgres")]` (L73-84) **n'existe pas à la compilation** dans un build `--features frontier-llm` **sans** `postgres` — il « passe la revue » mais ne s'exécute jamais, et `in_memory()` (L41) reste effectif **en silence**. À chaque rollout k8s (SIGTERM → graceful shutdown L115), la `Mutex<HashMap>` d'`InMemoryRunStore` ([store.rs:154-228](../../crates/efe-swarm-gateway/src/store.rs)) est effacée : **tous les reçus R4 disparaissent**, invisiblement, jusqu'à ce qu'un auditeur demande à re-vérifier un run qui n'existe plus (`DUR-1`, blocker).

**Le correctif** : `store_durable` est calculé depuis l'**intention env** (`postgres` compilé **ET** `DATABASE_URL` présent), **hors de tout `#[cfg]`** — il couvre donc le cas « frontier-llm sans postgres » (⟹ `false` ⟹ refus). Le `connect()`/`migrate()?` de L75-76 **re-valide** ensuite (mauvaise URL ou DDL cassé ⟹ `?` ⟹ exit non-zéro, **déjà** fail-fast, `DUR-4`). Scoper à `frontier_on` : l'essaim souverain / la démo M0-M3 utilisent **légitimement** InMemory — ne pas casser `/v1/swarm/*` ni le dev.

> **Précision de vérité (à corriger dans la roadmap, `DUR-3`/`HON-2`)** : la roadmap:125 dit « InMemory les perd au restart, **avec re-scellement sous une autre clé** ». **FAUX.** `EnvHkdf::derive` est **déterministe** (HKDF-SHA256, salt = UUID tenant, info fixe, [keys.rs:89-94](../../crates/efe-audit-core/src/keys.rs)) : même master ⟹ **MÊME clé** au restart. La panne InMemory est une **pure perte de données** (le reçu n'existe plus), pas un changement de clé. Le vrai scénario « autre clé » n'existe qu'à la **rotation du master** — concern **orthogonal** à traiter séparément.

### 1.4 Le flag unique + le footgun ops

`EFE_LLM_FRONTIER_ENABLED` est lu par égalité stricte `== "true"` ([main.rs:59](../../crates/efe-swarm-gateway/src/main.rs)) : `1`/`TRUE`/`yes` ⟹ frontière **OFF** (fail-closed, sûr) mais l'opérateur croit l'avoir activée (`ADV-8`). Et si le garde relit le flag avec une logique **différente** de L59, on introduit un drift. **Correctif** : factoriser `fn frontier_flag_enabled() -> bool` (une seule vérité), réutilisé par le garde **et** l'activation L59. Optionnel : logger un avertissement si la valeur est non-vide mais ≠ `"true"`.

### 1.5 Tests (la table de vérité pure)

```rust
#[test] fn frontier_boot_guard_triad() {
    // frontière OFF ⟹ dev toujours OK
    assert!(frontier_boot_guard(false, false, false, false).is_ok());
    // frontière ON : chaque invariant manquant ⟹ refus, dans l'ordre non-forgeable → auth → durable
    assert!(frontier_boot_guard(true, false, true,  true ).is_err()); // #2 clé
    assert!(frontier_boot_guard(true, true,  false, true ).is_err()); // #7 auth
    assert!(frontier_boot_guard(true, true,  true,  false).is_err()); // #3 durable
    assert!(frontier_boot_guard(true, true,  true,  true ).is_ok());  // triade complète ⟹ GO
}
```
La testabilité du `key_ok` (dry-run derive) et de `auth_ok` (`llm_from_env`) est déjà couverte par les tests existants de `keys.rs` et le test M3 `vote_401_*` ; le garde n'ajoute qu'une composition pure.

---

## 2. Durcissement de l'egress payant non-souverain (au-delà de la roadmap)

La roadmap M4 est **muette** sur le fait que `/v1/llm/*` est le **seul chemin qui dépense de l'argent réel** et **sort les données du tenant**. La recon a confirmé 4 trous prod-critiques que le gate doit adresser.

### 2.1 Plafond de coût par tenant — NON appliqué (`EGR-1`/`ADV-3`, blocker) → IN-M4

`egress_gate` ([routes/llm.rs:69-88](../../crates/efe-swarm-gateway/src/routes/llm.rs)) ne vérifie **que** l'allowlist + le registre. Le budget d'une mission vient de **`req.max_budget_cents`** — un `u32` **contrôlé par le client** (défaut 500 c, plafond ≈ **42 M$/requête**), passé tel quel à `run_*` ([vote.rs:40](../../crates/efe-multi-llm/src/workflows/vote.rs) `BudgetTracker::new(req.max_budget_cents)`). `TenantPolicies::max_cost_microcents_per_query`/`cap_microcents` existe ([multi_tenant.rs:140](../../crates/efe-coordinator/src/multi_tenant.rs)) mais n'est câblé **que** dans les workflows méta-swarm ([mission_a.rs:149](../../crates/efe-coordinator/src/workflows/mission_a.rs)), **jamais** sur la frontière. Un tenant allowlisté a donc un cap `None` (via `..TenantPolicies::open(tid)` en [frontier.rs:94](../../crates/efe-swarm-gateway/src/frontier.rs)) ⟹ **DoS-coût trivial**.

**Correctif** (purement **gateway-side**, jamais `efe-multi-llm`) : dans/juste après `egress_gate`, lire `policies.get(tenant).cap_microcents(...)` et **borner** `req.max_budget_cents` avant `run_*` (attention à l'unité : policy en **microcents**, requête en **cents** — convertir). Politique : `min(req, cap)`, ou **400** si `req > cap`. **Étendre le format** `EFE_LLM_TENANT_ALLOWLIST` pour porter un cap par tenant, p.ex. `uuid=claude_x,gpt_y@cap=50000`. C'est le durcissement egress **le plus payant**.

### 2.2 Redaction PII avant egress — absente (`EGR-2`, high) → IN-M4, opt-in

Les providers frontière sont construits **sans `with_redactor`** ([frontier.rs:46-57](../../crates/efe-swarm-gateway/src/frontier.rs)) ⟹ `redactor=None` ⟹ le prompt part **en clair**. La feature `pii` ne caviarde **que** la sortie/prévisualisation ([state.rs:336,350-364](../../crates/efe-swarm-gateway/src/state.rs)). Le record l'atteste **honnêtement** (`pii_strategy_applied=None`, hash du prompt **brut**, [mission.rs:170,223](../../crates/efe-swarm-gateway/src/mission.rs)) — mais pour Kovant (procurement/**compliance**), envoyer de la PII non-caviardée à un tiers US **en payant** est un risque conformité direct.

**Correctif** (gateway-side) : quand la policy tenant est `Redact`/`Strict`, caviarder `req.user_text` (détection **locale**, regex [pii.rs](../../crates/efe-swarm-gateway/src/pii.rs), sans réseau) **avant** `run_*`. **Défaut `Off` honnête** + doc explicite : « la frontière n'est pas souveraine ; sans policy PII, le prompt sort en clair ». Sans cela, M4 laisse ouverte la **plus grosse** fuite du chemin.

### 2.3 Clé Gemini en query-string (`EGR-3`, dette M3 §13) → IN-M4, small

`gemini.rs` place `?key={expose()}` dans **3 URLs** (generate/stream/health, [gemini.rs:194-201,254-259,449-453](../../crates/efe-coordinator/src/llm/providers/gemini.rs)). Sur erreur, le `reason` embarque l'URL keyée. `map_mission_err` la rédige du **corps HTTP** ([routes/llm.rs:93-107](../../crates/efe-swarm-gateway/src/routes/llm.rs)) et le record ne la capture pas — mais elle reste une surface **logs**. `gemini.rs` est dans `efe-coordinator` (**modifiable**, hors liste interdite). **Correctif** : basculer les 3 URLs vers l'en-tête `x-goog-api-key: {key}` (comme Anthropic/OpenAI), retirer `?key=`. Ferme la dette, réduit la charge de `map_mission_err`, supprime la classe « URL keyée dans un `Display` ». Coût ≈ 1 fichier.

### 2.4 Métriques de coût/egress — absentes (`EGR-4`, small) → IN-M4

`/metrics` ([routes/metrics.rs](../../crates/efe-swarm-gateway/src/routes/metrics.rs)) expose admission/provenance/substrat/mémoire mais **aucun** compteur d'egress frontière. En prod, l'opérateur ne peut **ni observer ni alerter** sur la dépense (indispensable pour rendre §2.1 actionnable). **Correctif** : ajouter `efe_llm_frontier_calls_total`, `efe_llm_frontier_remote_calls_total` (R4), `efe_llm_frontier_cost_microcents_total` (idéalement par tenant). Le seam existe (`build_record` connaît déjà `cost`/`attestation`).

### 2.5 Kill-switch runtime (`EGR-5`, low) → DÉFÉRÉ

`EFE_LLM_FRONTIER_ENABLED` n'est lu qu'au **boot** ; `PolicyRegistry` est immuable après construction. Couper l'egress lors d'un incident exige un **redémarrage**. **Décision** : différer (post-M4). **Interim IN-M4** : documenter la procédure de coupure (scale-to-zero / redeploy `flag=off`) comme kill-switch **de fait**. Futur : flag atomique consulté dans `egress_gate`, ou rechargement à chaud des policies (deny-all).

---

## 3. Le guard SSRF — filet anti-drift + honnêteté de portée (#4)

**Statut factuel** : les deux agents ont **répliqué et exécuté** les deux implémentations contre une table de vecteurs ; elles sont **sémantiquement identiques AUJOURD'HUI** (IPv6 étendu/mappé/zoné, IPv4 décimal `2130706433`/hex `0x7f000001`/octal `0177.0.0.1`/court `127.1`, `0.0.0.0`, `169.254.169.254` metadata, multi-`@`, casse, point-final). La checklist #4 est donc **VERTE à l'instant t** — mais **par accident, pas par construction** :
- **`provenance-capture/src/guard.rs`** (canonique, `pub`, `host_of → Option<&str>`) et **`efe-swarm-sdk/src/backend/mod.rs:231`** (copie, `pub(crate)`, `host_of → Option<String>`) sont **deux copier-coller** hand-maintained ;
- **aucun test cross-crate** n'exécute les deux contre une table **partagée** ⟹ une édition d'un seul côté passe ses propres tests pendant que l'autre diverge — **le drift SSRF a déjà eu lieu une fois** (mémoire projet).

### 3.1 Honnêteté cruciale : le guard NE gate PAS la frontière

**À écrire noir sur blanc (sinon un reviewer croit #4 « couvrir » l'egress tiers)** : sur `/v1/llm/*`, les endpoints sont des hôtes providers **hardcodés** (`api.anthropic.com`, …) non contrôlables par l'attaquant. `is_local_endpoint` n'y est **pas** un gate — il ne sert qu'à **CLASSER** R2LocalWitnessed vs R4RemoteChannel dans `build_record` ([mission.rs:138-151](../../crates/efe-swarm-gateway/src/mission.rs)) ; `assert_local` n'est **jamais** appelé côté frontière. **#4 protège le chemin essaim / SLM-local, PAS la frontière** (`ADV-6`/`EGR-6`).

### 3.2 Le filet anti-drift (le vrai livrable #4)

Le go/no-go exige les deux « durcis **ensemble** » : le transformer d'une **règle humaine** en un **test qui échoue** si l'une régresse.
- **Minimum M4** (`F2`, small) : une **table de vecteurs unique** (`const &[(&str, bool)]`) exécutée contre les **deux** `is_local_endpoint` — via un fichier `include!` partagé par les deux suites, ou un test dans `efe-swarm-gateway` (qui dépend déjà des deux) exposant la copie SDK sous `#[cfg(any(test, feature="test-util"))]`. Table = union des deux jeux + les résiduels ci-dessus.
- **Correctif `F3` (backslash, medium/attaque)** : `is_local_endpoint("http://evil.com\@127.0.0.1/v1") == true` (host `127.0.0.1`) alors qu'un client **WHATWG** (`url`/`reqwest`) normalise `\`→`/` ⟹ host `evil.com`. Le docstring **promet** l'accord octet-pour-octet avec reqwest — **faux** sur ce vecteur. Non exploitable aujourd'hui (chemin SDK via `curl` qui, lui, s'accorde ; chemin frontière = classification, pas gate) **mais landmine**. **Fail-close** les autorités contenant `\`, espace ou caractères de contrôle (retour `None`) + ajouter le vecteur à la table + **corriger le docstring** (viser l'accord avec le client HTTP *réellement utilisé sur le chemin gaté*).
- **Recommandé, différable** (`F1`, medium) : extraire un crate feuille **zéro-dép** `egress-guard` (host_of/is_loopback_ipv4/is_local_endpoint/assert_local/SovereigntyError + la table), dont `provenance-capture` et `efe-swarm-sdk` **dépendent et ré-exportent** (`guard.rs` devient `pub use egress_guard::*` → les call-sites `audit-api:72`→audit-console + `mission.rs:139` restent source-compatibles, **0 modif downstream**). **Rejeter** l'alternative « SDK dépend de provenance-capture » (élargit l'API, inverse les couches, tire la capture dans le SDK keyless mince). Non bloquant M4 (parité verte) : le **test partagé** suffit à figer le gate.

### 3.3 Bug de correction adjacent : l'allowlist ≠ les ids réels (`ADV-5`, high)

**Trouvé dans mon propre M3** : les ids enregistrés sont `format!("claude_{}", model)` (`gemini_{model}`, `gpt_{model}` — [anthropic.rs:210](../../crates/efe-coordinator/src/llm/providers/anthropic.rs)), mais le docstring de `policy_registry_from_str` ([frontier.rs:83-85](../../crates/efe-swarm-gateway/src/frontier.rs)) dit d'écrire `claude`/`gpt`/`gemini`. Une allowlist écrite **selon la doc** ne matche **jamais** le registre ⟹ `egress_gate` étage 3 renvoie **400 systématique** (fail-closed, mais frontière **inutilisable**). Pire : changer le modèle (env provider) **mute l'id** ⟹ toutes les allowlists cessent silencieusement de matcher. **Correctif M4** : (a) corriger le docstring ; (b) idéalement **matcher sur la famille** (préfixe avant `_`) dans `egress_gate`, OU (c) **logger au boot les ids réellement enregistrés** pour que les allowlists soient authorables.

---

## 4. Le modèle de confiance honnête (#5) — consolidation + corrections

**Bonne nouvelle** : les **4 clauses honnêtes existent déjà**, éparpillées — R4 tamper-evident write-side / **détenteur-de-clé** (M2 §0, §11) ; `held_strength` rapporté **hors** de `verified` (M3 §7/§11) ; **intégrité ≠ correction** (M2 §11 + [reprovability report](../reports/EFE_Reprovabilite_Analyse_2026-07-07.html)) ; egress frontière **non-souverain**, prompt en clair (M3 §2). **Mauvaise nouvelle** : aucun doc unique publiable, **et deux défauts internes** à corriger **avant** Kovant.

**Livrable** : créer **`docs/architecture/COUCHE2-M4-modele-confiance.md`** (destiné à Kovant), qui **cite verbatim** les 3 invariants et **purge** toute occurrence de « tiers » au sens trustless. Ce n'est **pas du code**.

### Corrections obligatoires (crédibilité devant un investisseur technique)

| # | Défaut | Où | Correction |
|---|---|---|---|
| `HON-3` | « **audit tierce** de l'arité » / « auditable par un **tiers** (détenteur de clé) » — c'est **exactement** l'overclaim trustless-tiers banni par M2 §0. Un détenteur de clé **n'est pas** un tiers. | M3 §8 (l.180, l.193) | **BANNIR** « tiers ». **REMPLACER** : « auditable **hors-bande par le détenteur de la clé** », « complément détenteur-de-clé de l'Invariant #2 ». Aligner §8 sur §11. |
| `HON-2` | « re-scellement sous une **autre clé** » — **faux** (EnvHkdf déterministe). | roadmap:125 | **REMPLACER** : « perd tous les reçus au restart (HashMap process) ; un reçu perdu ne peut plus être re-prouvé — **perte de données, pas altération** ». Traiter la **rotation** du master à part. |
| `HON-1` | « **Gate production** » nu laisse croire à une posture souveraine/compliance. | roadmap:118, doc M4 | **CADRER** la frontière comme **escape-hatch** : opt-in (OFF par défaut), borné (allowlist fail-closed), **NON-souverain** (prompt en clair vers un tiers). Ajouter un go/no-go : « à n'activer que pour les tenants qui l'ont explicitement accepté ». |
| `HON-4` | « production » **implique** SLA/DR/observabilité/kill-switch/conformité — **aucun** présent. | doc M4 | **ÉNUMÉRER** le périmètre **et** ce qu'il ne couvre **pas** (pas de SLA/DR au-delà de la durabilité Pg ; observabilité de coût = §2.4 ; kill-switch = §2.5 procédure ; pas de certification). |
| `HON-5` | « aucun credential ne fuit » (**vrai**) risque d'être lu comme « données protégées » (**faux**). | doc M4 | **JUXTAPOSER** : (a) reçu + corps HTTP propres de tout credential ; (b) **MAIS** prompt+contexte en clair vers le provider — la provenance l'atteste (`PiiStrategy::None`). Ne jamais présenter (a) comme confidentialité des données. |

---

## 5. Non-régression essaim (#6)

Toute la surface `/v1/swarm/*` vit dans `efe-swarm-gateway` (tests d'intégration + `lib.rs`). Les tests **flaky-en-workspace** (mTLS pr16, efe-metering global counter, daemon boot port) sont dans **d'AUTRES crates** ⟹ l'isolation `-p efe-swarm-gateway` les écarte **mécaniquement**. La non-régression est **100 % côté harnais** (aucune touche à `verify.rs`/`replay.rs`).

**Commande de non-régression (exacte)** :
```bash
cargo test -p efe-swarm-gateway                       # défaut : /v1/swarm/* + gardien route-non-montée (lib.rs:1069)
cargo test -p efe-swarm-gateway --features frontier-llm
cargo test -p efe-swarm-gateway --features frontier-llm,oidc
cargo test -p efe-swarm-gateway --features frontier-llm,postgres
# JAMAIS --workspace pour ce gate (flakiness connue hors du crate)
```

**Trou à combler** (`M4-F5`) : `no_llm_routes_when_frontier_off` ([lib.rs:1069](../../crates/efe-swarm-gateway/src/lib.rs)) ne compile qu'en build **défaut** — rien n'assure que `/v1/swarm/run`+`verify` restent inchangés **quand `frontier-llm` est compilée**. **Ajouter** un test compagnon sous `#[cfg(feature = "frontier-llm")]` : `POST /v1/swarm/run` + `GET /v1/swarm/verify/:id` ⟹ `verified=true` (prouve que monter `/v1/llm/*` ne perturbe pas le routeur essaim).

**Dette CI** (`DUR-4`, medium, possiblement hors-code-M4) : les tests Pg round-trip sont `#[ignore]` (requièrent `DATABASE_URL`). Ajouter une **étape CI Postgres** (testcontainers/service DB) qui lève le `--ignored` pour verrouiller l'**idempotence de `migrate`**, le round-trip JSONB (N17/request) et le fail-fast sur URL invalide.

---

## 6. Plan d'implémentation & DoD (go/no-go révisé)

**Fichiers** : `main.rs` (préflight triade §1 + `frontier_flag_enabled` §1.4) ; `routes/llm.rs`+`frontier.rs` (cap coût §2.1, PII opt-in §2.2, log ids/allowlist §3.3) ; `efe-coordinator/.../gemini.rs` (header §2.3) ; `routes/metrics.rs`+`mission.rs`/`build_record` (métriques §2.4) ; un `include!`/crate `egress-guard` + table de vecteurs (§3.2) ; **nouveau** `docs/architecture/COUCHE2-M4-modele-confiance.md` (§4) + corrections M3 §8 / roadmap:125. **Interdits** : `efe-multi-llm`, `verify.rs`, `replay.rs`.

**DoD révisée** :
1. **Triade au boot** : `frontier ON` + (master invalide **OU** oidc absent **OU** store non-durable) ⟹ **exit(78)** ; les 3 présents ⟹ boot. *(test de la fonction pure §1.5)*
2. **#2 non-forgeable** : master `< 32 o` ⟹ refus (dry-run derive, pas `master_present`). *(couvre `ADV-2`)*
3. **#7 authentifié** : build `frontier-llm` **sans** `oidc`, ou `oidc_llm` non configuré ⟹ refus. *(ferme `ADV-1`)*
4. **#3 durable** : build `frontier-llm` **sans** `postgres`, ou sans `DATABASE_URL` ⟹ refus (bool hors `cfg`). *(couvre `ADV-4`)*
5. **Cap coût** : `req.max_budget_cents > cap_tenant` ⟹ borné (ou 400) ; test d'un tenant sans cap ⟹ défaut serveur borné, pas `u32::MAX`. *(ferme `ADV-3`)*
6. **PII opt-in** : policy `Redact` ⟹ `req.user_text` caviardé avant egress ; défaut `Off` documenté.
7. **Gemini header** : `grep '?key='` dans `gemini.rs` ⟹ 0.
8. **Guard anti-drift** : table de vecteurs partagée verte sur les **deux** impls ; le vecteur backslash ⟹ **DENY**.
9. **Doc de confiance** : `COUCHE2-M4-modele-confiance.md` publié ; M3 §8 et roadmap:125 corrigés ; `grep -i 'trustless\|audit tierce\|auditable par un tiers'` ⟹ 0 (hors définitions négatives).
10. **Non-régression** : les 4 commandes §5 vertes ; test compagnon swarm sous `frontier-llm`.
11. **Métriques** : `/metrics` expose les 3 compteurs frontière.

---

## 7. Ce qu'on NE promet PAS

- **Pas trustless-tiers** : R4 = tamper-evident **write-side** + re-vérifiable par le **détenteur de la clé** ; `verified` ≠ correction ; `held_strength` **rapporté**, jamais replié.
- **La frontière n'est PAS souveraine** : prompt+contexte sortent **en clair** ; escape-hatch opt-in, jamais le chemin souverain.
- **Pas de SLA/DR/compliance** : le gate tient non-forgeabilité + authentification + durabilité + non-régression. Kill-switch runtime = **procédure** (scale-to-zero) ; observabilité de coût = compteurs (§2.4), pas d'alerting fourni.
- **Cap coût = plafond, pas facturation** : borne la dépense par requête ; pas de metering/quota cumulatif inter-requêtes (best-effort, cf. `multi_tenant.rs:55`).

---

## 8. Corrections vs la roadmap M4 (traçabilité)

| Proposition roadmap (§118-129) | Réalité (preuve) | Correctif M4 |
|---|---|---|
| Checklist go/no-go **6 items** | **manque l'authentification** : build sans `oidc` ⟹ egress payant en auth par en-têtes client (`ADV-1`) | **§1.2** — #7 auth obligatoire (la triade) |
| Fail-fast « master absent » | contourné par un **master court** (`master_present` ≠ contrat `derive`, `ADV-2`) | **§1.1** — **dry-run `derive`**, pas `master_present` |
| « PgRunStore requis en prod » | un garde dans `#[cfg(postgres)]` **n'existe pas** en build sans postgres (`ADV-4`) | **§1.3** — `bool` durable **hors `cfg`** |
| « re-scellement sous une autre clé » | **faux** : EnvHkdf déterministe (`HON-2`/`DUR-3`) | **§1.3/§4** — perte de **données**, pas de clé |
| Guard « re-synchronisé » | vrai **par accident** ; pas de test cross-crate ; **backslash** diverge de reqwest (`F3`) | **§3.2** — table partagée + fail-close `\` |
| — (non prévu) | cap coût tenant **non appliqué** ⟹ dépense non-bornée (`EGR-1`) | **§2.1** — clamp gateway-side |
| — (non prévu) | prompt egress **en clair**, pas de redaction PII (`EGR-2`) | **§2.2** — PII opt-in |
| « Doc du modèle de confiance » | déjà écrit **mais contredit** par M3 §8 « audit tierce » (`HON-3`) | **§4** — consolider + **corriger** §8 |
| « Gate production » | sur-promet devant Kovant (`HON-1/4`) | **§4** — cadrer **escape-hatch non-souverain** |
| Allowlist `claude`/`gpt`/`gemini` (mon M3) | ids réels = `claude_<model>` ⟹ **400 systématique** (`ADV-5`) | **§3.3** — docstring + matching famille |

*Préparation M4 : le gate n'est pas un tampon. La roadmap voyait 6 cases défensives ; l'analyse adverse a trouvé un **blocker d'authentification** invisible à la checklist, deux **faussetés internes** à corriger avant Kovant, et le fait que servir cette frontière « en production » exige d'abord d'admettre **honnêtement** que c'est un escape-hatch qui **brûle de l'argent** et **sort les données** — puis de le **borner** (clé non-forgeable, auth obligatoire, store durable, cap coût, PII opt-in) avant de prononcer le mot « production ».*
