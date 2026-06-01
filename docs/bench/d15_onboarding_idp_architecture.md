# Sprint D15 — Architecture détaillée : Onboarding documentation + IdP integration (OIDC)

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit D14-bis (HMAC tamper-evident audit) + son
> audit interne (`docs/audits/d14bis_audit_report.md` — 9 bugs,
> 7 fixés). Ouvre la voie aux déploiements **multi-tenants
> régulés** où chaque client provisionne son propre tenant via
> son IdP d'entreprise (Auth0, Okta, Keycloak, AWS Cognito,
> Azure AD, Google Workspace).
> **Pré-requis** : avoir lu
> [`d14_postgres_catalog_backend_architecture.md`](d14_postgres_catalog_backend_architecture.md)
> (Postgres D14), [`d14bis_hmac_audit_chain_architecture.md`](d14bis_hmac_audit_chain_architecture.md)
> (HMAC audit D14-bis), [`M12_ARCHITECTURE.md`](../../M12_ARCHITECTURE.md)
> § 9 (HS256 secrets pattern actuel),
> [`d11_per_tenant_isolation_architecture.md`](d11_per_tenant_isolation_architecture.md)
> (zero-trust per-tenant, base de la mapping IdP → tenant),
> [`d13_observability_dashboards_architecture.md`](d13_observability_dashboards_architecture.md)
> (les dashboards qu'on étend pour les nouvelles métriques OIDC).
>
> **Mission** : transformer EFE d'un service avec **auth de PoC**
> (HS256 JWT signé par un fichier secret partagé) en un service
> avec **auth de production** (OIDC/JWKS RS256 fédéré avec un
> IdP d'entreprise), accompagné d'une **documentation onboarding
> Diátaxis** complète qui permet à un opérateur ou à un client
> final de provisionner un tenant et d'émettre son premier
> `RegisterCatalog` en moins d'une heure.
>
> **Pain point opérationnel résolu** :
>
> 1. **HS256 ne scale pas en multi-tenant régulé** : la clé HMAC
>    est partagée entre celui qui sign les tokens (= client) et
>    celui qui verify (EFE). Pour SOC 2 / ISO 27001 / NIST 800-53,
>    les rôles signing et verifying doivent être séparés ; sans
>    asymmetric crypto, c'est impossible.
> 2. **Pas d'intégration IdP** : aujourd'hui chaque client doit
>    convaincre l'opérateur EFE de partager une clé HMAC. Pas
>    SSO, pas MFA enforced, pas SCIM provisioning, pas audit
>    federation côté IdP.
> 3. **Onboarding flou** : un nouveau client n'a aucun guide pour
>    provisionner un tenant. Les ops doivent répondre par email,
>    le time-to-first-RPC est jours.
> 4. **Algorithm confusion attack non documenté** : un attaquant
>    qui connaît le "secret" RS256 public peut tenter de
>    re-signer un token avec HS256 en utilisant la pub key comme
>    HMAC key — `jsonwebtoken` mitige par défaut mais le
>    threat model n'est pas explicit.
> 5. **Pas de JWKS rotation native** : les clés signing IdP
>    rotent (typically chaque ~24-90 j). EFE doit fetcher
>    `/.well-known/jwks.json` périodiquement pour suivre.
>
> **Note importante sur le périmètre** : ce sprint n'introduit PAS
> un IdP. EFE reste un **resource server** OAuth2 (RFC 6749 § 1.1)
> qui valide les access tokens émis par un IdP tiers. Bring Your
> Own IdP. Pour les déploiements qui veulent un IdP intégré
> (e.g. Tier 1 dev), on recommande Keycloak self-hosted en mode
> dev — documenté en howto.

---

## Table des matières

1. [Contexte & mission D15](#1-contexte--mission-d15)
2. [État des lieux post-D14-bis — pré-conditions remplies](#2-état-des-lieux-post-d14-bis--pré-conditions-remplies)
3. [Threat model OIDC — 8 vecteurs](#3-threat-model-oidc--8-vecteurs)
4. [Décision majeure #1 — IdP target : OIDC standard vs vendor-specific](#4-décision-majeure-1--idp-target--oidc-standard-vs-vendor-specific)
5. [Décision majeure #2 — algorithmes : RS256 vs ES256 vs EdDSA](#5-décision-majeure-2--algorithmes--rs256-vs-es256-vs-eddsa)
6. [Décision majeure #3 — JWKS fetch : pull vs push](#6-décision-majeure-3--jwks-fetch--pull-vs-push)
7. [Décision majeure #4 — Tenant mapping strategy](#7-décision-majeure-4--tenant-mapping-strategy)
8. [Décision majeure #5 — Multi-issuer support](#8-décision-majeure-5--multi-issuer-support)
9. [Décision majeure #6 — Backwards compat HS256](#9-décision-majeure-6--backwards-compat-hs256)
10. [Décision majeure #7 — Algorithm-confusion mitigation](#10-décision-majeure-7--algorithm-confusion-mitigation)
11. [Décision majeure #8 — Onboarding flow design (4 phases)](#11-décision-majeure-8--onboarding-flow-design-4-phases)
12. [Architecture du module `auth/oidc.rs`](#12-architecture-du-module-authoidcrs)
13. [Schéma de configuration `EFE_OIDC_*`](#13-schéma-de-configuration-efe_oidc_)
14. [Documentation deliverables — Diátaxis tree](#14-documentation-deliverables--diátaxis-tree)
15. [Schéma SQL `tenants` (registre)](#15-schéma-sql-tenants-registre)
16. [Helm wiring — issuers + Secret + sidecar](#16-helm-wiring--issuers--secret--sidecar)
17. [Runtime wire-up — `Server::build` augmenté](#17-runtime-wire-up--serverbuild-augmenté)
18. [Observability D13/D14/D14-bis extensions](#18-observability-d13d14d14-bis-extensions)
19. [Tests — ~50 nouveaux tests](#19-tests--50-nouveaux-tests)
20. [Migration compat HS256 → OIDC](#20-migration-compat-hs256--oidc)
21. [Decisions log — récap des décisions clés](#21-decisions-log--récap-des-décisions-clés)
22. [Risques + mitigations](#22-risques--mitigations)
23. [Plan wallclock — 6 j](#23-plan-wallclock--6-j)
24. [SLO ajoutés en D15](#24-slo-ajoutés-en-d15)
25. [Anti-régression D15](#25-anti-régression-d15)
26. [Hand-off vers D16](#26-hand-off-vers-d16)
27. [Annexes](#27-annexes)

---

## 1. Contexte & mission D15

### 1.1 État post-D14-bis

Au gel de v0.1.14.5 (post-D14-bis-audit) :

* **`AuthGuard`** D11 stable, méthode `check<T>(req, scope)`
  consommée par les 6 RPC catalog (D12-bis) + admin/swarm.
* **`TokenStore::from_hs256(...)`** unique implémentation crypto :
  HMAC-SHA-256 avec clé symétrique 32 octets. RFC 7519 + RFC 7515.
* **`Claims`** struct contient `iss`, `sub`, `aud`, `exp`, `tenant_id`,
  `scopes` (cf. `auth/claims.rs`). Le `tenant_id` est un custom
  claim qui mappe directement au tenant EFE.
* **`EFE_JWT_SECRETS_FILE`** lit un fichier 1 ligne = 1 clé base64 ;
  pattern réutilisé par D14-bis HMAC audit.
* **D11 zero-trust per-tenant** : chaque RPC enforce
  `assert_tenant_match(jwt.tenant_id, request.tenant)` via
  `RequestTenant` extension injectée par l'auth interceptor.
* **6 dashboards D13** observent `efe_grpc_*{tenant=…}` sans
  jamais leak un tenant inter-dashboard.

### 1.2 Le problème opérationnel

Trois personas, trois douleurs :

#### 1.2.1 Le client final ("acme.com") qui veut adopter EFE

* **Aujourd'hui** : envoie un email à eng@efe.ai, attend 2-5 jours,
  reçoit une clé HMAC partagée par chat sécurisé. Doit signer
  manuellement ses propres JWTs avec cette clé.
* **Pain points** :
  - SSO d'entreprise (Okta, Azure AD) ne peut pas signer ces
    tokens custom EFE.
  - Pas de MFA enforce — quiconque a la clé peut signer.
  - Pas de SCIM provisioning — l'admin ajoute/retire des users
    EFE-side via... quoi ? Pas d'API user-management dans EFE.
  - Pas de federated audit — l'IdP du client a son propre log
    auth, EFE a le sien, aucune correlation.
  - **Time to first RegisterCatalog** : ~5 jours.

#### 1.2.2 L'opérateur EFE qui héberge le multi-tenant

* **Aujourd'hui** : chaque tenant onboardé = nouvelle clé HMAC à
  ajouter dans `hs256.secrets`. Rotation = restart pod. Provisioning
  = manuel.
* **Pain points** :
  - Sécurité : la clé HMAC vit dans les pods EFE ; un compromis
    pod = compromise tous les tenants.
  - Audit : aucune trace centralisée des "qui a émis quel token
    quand".
  - Compliance SOC 2 : le contrôle CC6.1 exige separation of
    signing role — impossible avec HMAC partagé.

#### 1.2.3 Le régulateur qui audite l'EFE

* **Aujourd'hui** : "voici nos logs, trust us" — EFE peut leak un
  audit event chez Loki ; l'IdP du client peut leak l'auth event
  côté son propre log. Aucune fédération.
* **Pain points** :
  - Compliance HIPAA §164.312(d) "Person or Entity Authentication"
    exige que l'IdP soit auditable indépendamment.
  - GDPR Article 30 "Records of processing activities" : un
    régulateur veut prouver que tel `sub` a accédé à telle data
    à tel moment, avec corrélation IdP ↔ EFE.

### 1.3 Mission D15 (énoncé chirurgical)

D15 livre **trois choses** :

1. **Module `auth/oidc.rs`** : nouveau path de validation tokens
   qui supporte RS256 / ES256 (asymmetric) en plus du HS256
   existant. Le path détecte le `alg` du JWT header et dispatch.
   Multi-issuer : 1 cluster EFE peut servir N tenants venant de
   N IdP distincts.

2. **JWKS fetch + cache** : module `auth/jwks.rs` qui pull
   `/.well-known/jwks.json` de chaque issuer configuré, cache
   les pub keys, refresh périodique (5 min default), retry
   exponential backoff sur erreur, signal SIGHUP pour force
   refresh out-of-band.

3. **Documentation onboarding Diátaxis** :
   - **3 howto** : `onboarding-tenant.md` (le client),
     `idp-integration-keycloak.md` (Tier 1 dev),
     `idp-integration-auth0.md` (Tier 2/3 prod).
   - **2 reference** : `idp-config-schema.md`,
     `tenant-claim-format.md`.
   - **1 explanation** : `auth-architecture.md` —
     pourquoi OIDC, comment EFE est resource server.
   - **1 runbook** : `jwks-fetch-failing.md` —
     que faire quand l'IdP est down.

Plus :

4. **CLI `efe-onboard`** : binary compagnon qui automatise
   le provisioning d'un tenant : crée la row `tenants`,
   génère un Keycloak realm/Auth0 organization (templated),
   émet le premier token admin, donne à l'opérateur un
   bash one-liner à exécuter pour son premier RegisterCatalog.

5. **Schema SQL `tenants`** : nouvelle table SQL append-mostly
   qui registre chaque tenant onboardé : `tenant_id`,
   `display_name`, `created_at`, `idp_issuer`, `status`
   (`pending` | `active` | `suspended`), `meta JSONB`.

6. **Migration `0004_tenants.sql`** : DDL + 3 indexes + audit
   trigger qui logge tout INSERT/UPDATE.

### 1.4 Hors-périmètre

Pour éviter l'embonpoint :

| Hors scope D15 | Justification | Sprint cible |
|----------------|---------------|--------------|
| Implémenter un IdP intégré | Bring Your Own IdP — Keycloak self-hosted documenté pour Tier 1 | external |
| OAuth 2.1 PKCE flow / device code flow | EFE est resource server, pas auth client | external |
| SCIM 2.0 provisioning | Vendor-specific (Okta/Azure AD/Auth0 ont leur API) | **D17+** |
| User management API custom (`POST /users`) | Out of scope — IdP gère les users | n/a |
| MFA enforcement côté EFE | IdP responsabilité ; EFE peut juste lire `acr` claim | **D16** |
| JWT replay protection avancée (jti registry) | `exp` court ≤ 1 h suffit pour MVP ; jti registry coûteux | **D17+** |
| Token introspection RFC 7662 | Stateless validation via JWKS suffit | external (ops) |
| Refresh token rotation | Client-side concern | n/a |
| OAuth client_credentials grant | Service-to-service flow, géré par IdP | external |
| Onboarding self-service web UI | Backend-first ; UI sprint dédié | **D18+** |
| Migration automatique HS256 → OIDC | Documenté manuel, automation = scope creep | n/a |
| Audit log fédération (cross-IdP-EFE correlation) | Compliance feature, sprint dédié | **D17+** |
| Federation transparency log (Sigsum, Certificate Transparency) | Out of scope MVP | **D17+** |

---

## 2. État des lieux post-D14-bis — pré-conditions remplies

### 2.1 Invariants stables (NE PAS modifier)

D15 doit honorer chaque invariant ci-dessous sans exception :

| Invariant | Source | Impact si violé |
|-----------|--------|-----------------|
| Trait `AuthGuard::check` ne change pas (signature `(req, Option<scope>) -> Result<()>`) | M9 + 6 RPC handlers | Tous les call-sites cassent |
| `Claims` struct fields `(iss, sub, aud, exp, tenant_id, scopes)` ne disparaissent pas | M12 + tests | Tests régression cassent |
| `RequestTenant` extension injectée AVANT le handler | D11 zero-trust | `assert_tenant_match` échoue silencieusement |
| `RBAC scopes` mapping inchangé (cf. `scopes.rs` D12-bis fix) | D12-bis audit | RBAC paths déconnectés |
| **`HS256 path` reste fonctionnel** pour backwards compat | M12 deployments existants | Breakage opérateurs Tier 1 |
| Erreur `Internal` ne leak pas (audit T-12bis-2) | D12-bis | Info disclosure |
| `principal_present` label D13 fix C-1 | Audit metric counter | Alerte D13ObsAuditPrincipalUnknown |
| Forward-only migrations | sqlx convention | Production migration safety |

### 2.2 Surface trait ne change pas

**`AuthGuard` reste l'unique point d'entrée auth.** Le sprint
ajoute des nouveaux backends de validation **derrière**
`TokenStore`. Le call-site (`SwarmServiceImpl`, `AdminServiceImpl`,
`CatalogServiceImpl`) ne sait pas si le token a été vérifié via
HS256 ou RS256/ES256/EdDSA — c'est transparent.

```rust
// Diff D15 : extend TokenStore for asymmetric crypto.
pub struct TokenStore {
    backends: Vec<TokenBackend>,  // chained validation
}

enum TokenBackend {
    Hs256 { keys: Vec<Vec<u8>>, audience: String, issuer: Option<String> },
    Oidc {
        issuers: Arc<IssuerRegistry>,
    },
}

impl TokenStore {
    pub fn validate(&self, token: &str) -> Result<Claims, AuthError> {
        // Try each backend ; first to validate wins.
        for be in &self.backends {
            match be.validate(token) {
                Ok(claims) => return Ok(claims),
                Err(_) => continue,  // try next
            }
        }
        Err(AuthError::NoBackendAccepted)
    }
}
```

### 2.3 Gaps que D15 ferme

| Gap | Symptôme actuel | Fix D15 |
|-----|-----------------|---------|
| Asymmetric crypto manquant | Impossible de séparer signing role | RS256 + ES256 supportés ; HMAC sign reste IdP-side |
| JWKS rotation manuelle | Ops doit redémarrer pour rotater | JWKS fetch périodique automatique |
| Multi-issuer manuel | 1 cluster = 1 audience HS256 | Multi-issuer config + per-issuer audience |
| Tenant mapping rigide | `tenant_id` doit être custom claim | 2 strategies : claim ou issuer-based |
| Onboarding sans guide | Email + chat | 7 docs Diátaxis + CLI `efe-onboard` |
| Pas de registre tenants | Aucune source de vérité "qui a un tenant ?" | Table SQL `tenants` + audit |

---

## 3. Threat model OIDC — 8 vecteurs

| # | Vecteur | Description | Mitigation D15 |
|--:|---------|-------------|----------------|
| 1 | **Algorithm confusion** | Attaquant émet un token avec `alg=HS256` en utilisant la pub RS256 comme HMAC key | Whitelist explicite des algos par issuer (`alg_whitelist`) ; refus de mismatch |
| 2 | **None algorithm** | Attaquant émet `alg=none` (RFC 7519 § 6.1) | `jsonwebtoken` 9.x rejette par défaut ; whitelist explicit `Algorithm::None` interdite |
| 3 | **JWKS poisoning** | Attaquant intercepte la HTTP fetch et injecte une fake pub key | TLS verify-full du JWKS endpoint mandatory ; HSTS pinning via custom CA bundle |
| 4 | **JWKS DoS** | Attaquant down l'IdP pendant rotation → JWKS fetch fail → tous les tokens rejetés | Cache 5 min + grace period configurable ; alert `D15IdpJwksFetchFailing` ; fallback sur les pub keys cached avec warn |
| 5 | **Tenant claim spoof** | Token légit pour `acme` modifié pour prétendre être `globex` | Signature verify rejette ; tenant_claim doit être dans le JWT claims set, signé |
| 6 | **Issuer spoof** | Attaquant forge un token `iss=https://acme.auth0.com` | `iss` claim verifié contre la liste configurée ; JWKS fetch utilise `iss` comme clé du registry |
| 7 | **JWT replay** | Token capturé, replayed après expiration de la session originale | `exp` court ≤ 1 h ; jti registry optionnel (D17+) ; mTLS en plus pour Tier 3 |
| 8 | **Privilege escalation via scope claim** | Token légit avec scope `admin:catalog:read` modifié pour `admin:catalog:write` | Signature rejette ; scopes uniquement consommés via `Claims` post-verify |

---

## 4. Décision majeure #1 — IdP target : OIDC standard vs vendor-specific

### 4.1 Critères

| Critère | Poids |
|---------|:-----:|
| Compat avec IdPs majeurs (Auth0, Okta, Keycloak, Cognito, Azure AD, Google) | 5/5 |
| Specs publiques implémentables sans NDA | 5/5 |
| Effort impl | 4/5 |
| Test matrix CI raisonnable (≤ 2 IdPs testés en CI) | 4/5 |
| Migration future | 3/5 |

### 4.2 Comparaison

| Option | Pour | Contre |
|--------|------|--------|
| **A. OIDC + OAuth 2.0 standard** | RFC ubiquitous (RFC 6749, RFC 7519, RFC 8414), tous les IdPs majeurs supportent | "Standard" = plus larges qu'on a besoin |
| **B. Auth0-specific** | API riche (organizations, actions) | Vendor lock |
| **C. Okta-specific** | Mature SCIM | Vendor lock + payant |
| **D. Keycloak-specific** | Open-source, free | Less ubiquitous en entreprise |

### 4.3 Décision : Option A (OIDC standard)

→ **OIDC** + **OAuth 2.0 RFC 6749** + **JWT RFC 7519** + **JWKS RFC 7517**.

Test matrix CI :
* **Keycloak** (open-source) : impl reference Tier 1, runs in
  testcontainers.
* **Auth0** (SaaS) : reproductible via mock JWKS server (pas de
  call cloud en CI).

Documenté qu'on **support tout IdP OIDC compliant** ; testé contre
2 ; opérateur peut l'utiliser avec n'importe lequel.

---

## 5. Décision majeure #2 — algorithmes : RS256 vs ES256 vs EdDSA

### 5.1 Comparaison

| Algorithm | Pour | Contre |
|-----------|------|--------|
| **RS256** (RSA-2048+ + SHA-256) | Ubiquitous, tous IdPs supportent | Keys volumineux (2048+ bits = 256 octets), verify lent (~200 µs) |
| **ES256** (ECDSA NIST P-256 + SHA-256) | Compact (96 octets sig), verify rapide (~50 µs) | Moins ubiquitous (Auth0 ✓, Okta ✓, Cognito ✓, Azure AD ✓, Keycloak ✓ depuis 14+) |
| **EdDSA Ed25519** | Modern, fastest, no curve params confusion | Moins supporté côté IdPs (Auth0 ✗, Okta ✗ ; Keycloak via plugin) |

### 5.2 Décision : RS256 + ES256 supportés ; EdDSA déféré

→ **Support RS256 ET ES256** simultanément. Un même cluster EFE
peut avoir un IdP RS256 (Auth0) et un autre ES256 (Cognito).

Le sprint **NE livre PAS** EdDSA — pas assez de demandes côté
Tier 1/2 ; à ajouter en D17+ si signal client.

`alg_whitelist` per issuer (cf. décision #7).

---

## 6. Décision majeure #3 — JWKS fetch : pull vs push

### 6.1 Trade-offs

| Option | Pour | Contre |
|--------|------|--------|
| **Pull** : EFE fetch périodiquement | Robuste (no inbound webhook hole), simple | Lag : nouvelle key visible après ≤ TTL |
| **Push** : IdP poke EFE | Réactivité instantanée | Hole pare-feu à ouvrir, IdP doit supporter (peu le font) |

### 6.2 Décision : Pull avec cache 5 min

→ **Pull** avec :
* TTL default 300 s (5 min).
* Cache hit → no fetch.
* Cache miss → fetch + retry 3× exponential backoff (100ms, 1s, 5s).
* Force refresh via SIGHUP signal (stale during rotation incident).
* Stale-while-revalidate : si fetch fail mais cache valide, serve
  with warning log + bump `efe_jwks_stale_total{issuer}`.

---

## 7. Décision majeure #4 — Tenant mapping strategy

### 7.1 Options

| Strategy | Description | IdPs supportant nativement |
|----------|-------------|----------------------------|
| **A. Custom claim** (`tenant_id`, ou `https://efe.example/tenant`) | JWT contient un claim explicite | Auth0 (rules/actions), Okta (custom claims), Keycloak (mapper), Cognito (custom attributes) |
| **B. Issuer-based** | Chaque tenant = 1 issuer URL distinct | Auth0 organizations, Keycloak realms, Azure AD multi-tenant |
| **C. `sub` parsing** | `sub = "acme:user:alice"` | Aucun IdP (custom logic) |

### 7.2 Décision : A préféré + B fallback configurable

→ **Strategy A par défaut** : custom claim. Configurable via
`EFE_OIDC_TENANT_CLAIM` (default `tenant_id`).

→ **Strategy B en option** : `EFE_OIDC_TENANT_STRATEGY=issuer`.
Le mapping `issuer URL → tenant_id` est dans la table `tenants`
(SQL) ou dans la config.

→ **C rejeté** (custom logic, fragile, hors-spec).

---

## 8. Décision majeure #5 — Multi-issuer support

### 8.1 Cas d'usage

* **Tier 1 dev** : 1 issuer (Keycloak local). Trivial.
* **Tier 2 staging** : 1 issuer (Auth0 unique). Trivial.
* **Tier 3 prod multi-tenant** : N issuers (1 par tenant qui
  apporte son IdP). EFE doit dispatcher la verify selon le
  `iss` claim.

### 8.2 Architecture

`IssuerRegistry` :

```rust
pub struct IssuerRegistry {
    /// issuer URL → IssuerEntry.
    by_iss: HashMap<String, IssuerEntry>,
}

pub struct IssuerEntry {
    pub issuer: String,
    pub audience: String,
    pub jwks_url: String,
    pub alg_whitelist: Vec<Algorithm>,  // RS256, ES256, …
    pub tenant_strategy: TenantStrategy,
    pub tenant_claim: Option<String>,   // when strategy == Claim
    pub tenant_id_for_issuer: Option<String>,  // when strategy == Issuer
    pub jwks_cache: Arc<JwksCache>,
}
```

À la verify :

```rust
fn validate_oidc(&self, token: &str) -> Result<Claims, AuthError> {
    let header = decode_header(token)?;  // unverified
    let kid = header.kid.ok_or(AuthError::MissingKid)?;
    let unverified_payload = decode_payload_unverified(token)?;
    let iss = unverified_payload.iss;
    let entry = self.by_iss.get(&iss).ok_or(AuthError::UnknownIssuer)?;
    if !entry.alg_whitelist.contains(&header.alg) {
        return Err(AuthError::AlgorithmNotAllowed);
    }
    let key = entry.jwks_cache.get_key(&kid).await?;
    let claims = jsonwebtoken::decode::<Claims>(
        token,
        &key,
        &Validation { audience: ..., issuer: ..., algorithms: alg_whitelist, .. },
    )?;
    let tenant = match entry.tenant_strategy {
        TenantStrategy::Claim => claims.tenant_id_from_claim(entry.tenant_claim.as_ref())?,
        TenantStrategy::Issuer => entry.tenant_id_for_issuer.clone().unwrap_or_default(),
    };
    Ok(claims.with_tenant(tenant))
}
```

---

## 9. Décision majeure #6 — Backwards compat HS256

### 9.1 Pourquoi compat ?

* M12 deployments existants (Tier 1 dev, smoke tests, integration
  CI) tournent avec HS256.
* Migration breaking = adoption hostile.

### 9.2 Décision : HS256 deprecated mais supporté

→ **HS256 reste fonctionnel** pour les deployments qui ne
configurent pas OIDC.

→ **`EFE_AUTH_HS256_DEPRECATED_WARN=true`** logue un warning
au boot et à chaque token verify (sample 1/100) pour pousser
les opérateurs à migrer.

→ **Deprecation timeline** : v0.1.16 (D17 prévu) deprecates
fully ; v0.1.20 removes. Documenté dans le CHANGELOG.

→ **Mode mixte** : HS256 + OIDC simultanés. Si le JWT a un
`iss` matching la config OIDC, dispatch OIDC ; sinon fallback
HS256. Permet une **migration graduelle tenant-par-tenant**.

---

## 10. Décision majeure #7 — Algorithm-confusion mitigation

### 10.1 Le bug à éviter

Le **algorithm confusion attack** :

1. Attaquant connaît la pub key RSA d'un IdP (publique par
   définition).
2. Attaquant forge un JWT avec `alg=HS256` et utilise la pub key
   comme HMAC secret.
3. Si le serveur valide naïvement avec "the configured key" sans
   vérifier l'alg, il accepte.

### 10.2 Décision : whitelist algos per issuer

→ Chaque `IssuerEntry` a un `alg_whitelist: Vec<Algorithm>`.

→ La verify rejette tout token dont `header.alg` n'est pas dans
la whitelist, AVANT même de chercher la key.

→ `alg=none` est **bannied par construction** :
`jsonwebtoken::Validation` rejette par default ; on ajoute un
test pin pour s'assurer.

→ Pin test en CI : un token forgé avec `alg=HS256` contre un
issuer configuré RS256-only → reject.

---

## 11. Décision majeure #8 — Onboarding flow design (4 phases)

### 11.1 Phase 1 — Pre-flight (opérateur EFE)

**Action** : opérateur reçoit une demande d'onboarding (email,
ticket Jira). Décide :
* `tenant_id` (regex `^[a-z0-9_-]{1,63}$` — D14 audit fix C1).
* `display_name` (libre).
* `idp_issuer` URL (donnée par le client).

**Commande** :
```bash
efe-onboard preflight --tenant acme --display-name "ACME Inc" \
    --idp-issuer https://acme.auth0.com/
```

**Effet** : INSERT row `tenants` avec status=`pending`.

### 11.2 Phase 2 — Bootstrap (opérateur EFE + client)

**Action** : opérateur fournit au client :
* `tenant_id`.
* JWKS endpoint à pointer (`/.well-known/jwks.json`).
* Liste des claims attendus (`iss`, `aud=efe.v1`, `tenant_id`,
  `scopes`).

**Côté client** :
* Configure son IdP pour émettre JWT avec ces claims pour
  ses utilisateurs autorisés.

**Côté opérateur** :
```bash
efe-onboard activate --tenant acme
```

→ Active le tenant : status=`active`. Le client peut maintenant
émettre des tokens et faire `RegisterCatalog`.

### 11.3 Phase 3 — Steady state (client autonome)

**Action** : le client gère ses users via son IdP (SCIM, SSO,
MFA, lifecycle). EFE consomme les tokens sans
réimplémenter user management.

**Monitoring** : opérateur EFE voit dans Grafana :
* `efe_grpc_handled_total{tenant="acme"}` — trafic du tenant.
* `efe_audit_chain_head_seq{chain_id="tenant:acme"}` — audit
  growth.
* `efe_oidc_tokens_validated_total{issuer="https://acme.auth0.com/"}` — auth volume.

### 11.4 Phase 4 — Audit handoff (régulateur)

**Action** : le régulateur demande une preuve d'audit pour
`tenant=acme` entre 2026-01-01 et 2026-03-31.

**Réponse** :
1. Opérateur exporte `audit_events` filtrés par chain_id et
   timestamp via `efe-export-audit` (D14-bis).
2. Opérateur fournit la clé HMAC active pendant la fenêtre +
   le snapshot.
3. Régulateur run `efe-verify-audit --keys-file ... --chain
   tenant:acme` → exit 0 = preuve d'intégrité.
4. **Bonus D15** : le régulateur peut aussi vérifier les JWT
   tokens via la pub key de l'IdP (téléchargeable
   indépendamment via `/.well-known/jwks.json`) — preuve
   verifiable-by-third-party côté **auth** (l'audit reste
   verify-with-key D14-bis ; D14-ter ajouterait Ed25519 audit).

---

## 12. Architecture du module `auth/oidc.rs`

### 12.1 Topologie

```
crates/efe-server/src/auth/
├── claims.rs           ← inchangé D15
├── guard.rs            ← légère extension : multi-backend dispatch
├── mod.rs              ← re-exports
├── token_store.rs      ← extension : Vec<TokenBackend>
├── oidc.rs             ← NEW (~600 lignes)
├── jwks.rs             ← NEW (~400 lignes — fetch + cache)
├── issuers.rs          ← NEW (~300 lignes — registry config)
└── tenants.rs          ← NEW (~250 lignes — SQL CRUD)
```

### 12.2 Surface publique

```rust
// auth/oidc.rs
pub struct OidcValidator {
    issuers: Arc<IssuerRegistry>,
}

impl OidcValidator {
    pub async fn new(issuers: Arc<IssuerRegistry>) -> Self;
    pub async fn validate(&self, token: &str) -> Result<Claims, AuthError>;
}

// auth/jwks.rs
pub struct JwksCache {
    inner: Arc<RwLock<JwksCacheInner>>,
    fetcher: Arc<dyn JwksFetcher>,
}

impl JwksCache {
    pub fn new(jwks_url: String, ttl: Duration) -> Self;
    pub async fn get_key(&self, kid: &str) -> Result<DecodingKey, AuthError>;
    pub async fn refresh(&self) -> Result<(), AuthError>;
    pub fn spawn_periodic_refresh(&self, ttl: Duration, shutdown: Shutdown);
}

#[async_trait]
pub trait JwksFetcher: Send + Sync {
    async fn fetch(&self, jwks_url: &str) -> Result<JwksDocument, AuthError>;
}

// auth/issuers.rs
pub struct IssuerRegistry { ... }
impl IssuerRegistry {
    pub fn from_config(cfgs: &[IssuerConfig]) -> Result<Self, AuthError>;
    pub fn get(&self, iss: &str) -> Option<&IssuerEntry>;
}
```

### 12.3 Async vs sync au verify path

Le `JwksCache::get_key` est async (peut nécessiter un fetch).
Mais `AuthGuard::check<T>` est sync (M9 contract).

**Solution** : pre-warm le cache au boot via JWKS fetch. Au
verify path, lookup synchronous dans le RwLock cache. Si le
kid est manquant (ex: rotation in flight), reject avec
`Unauthenticated` ; le cache ticker async finira par récupérer.

```rust
// Sync verify : look up cached key only.
pub fn validate_sync(&self, token: &str) -> Result<Claims, AuthError> {
    // header parse + iss lookup + cached key lookup + jsonwebtoken::decode
    // No network IO.
}
```

L'arrière-plan `spawn_periodic_refresh` async ensures cache stays
fresh.

### 12.4 Erreurs distinctes

```rust
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("token signature invalid")]
    InvalidSignature,
    #[error("token expired")]
    Expired,
    #[error("token not yet valid (nbf)")]
    NotYetValid,
    #[error("missing kid in JWT header")]
    MissingKid,
    #[error("kid {0:?} not found in JWKS")]
    UnknownKid(String),
    #[error("issuer {0:?} not configured")]
    UnknownIssuer(String),
    #[error("algorithm {0:?} not in whitelist")]
    AlgorithmNotAllowed(String),
    #[error("audience mismatch")]
    AudienceMismatch,
    #[error("tenant claim missing")]
    MissingTenantClaim,
    #[error("JWKS fetch failed: {0}")]
    JwksFetch(String),
    #[error("internal: {0}")]
    Internal(String),
}
```

Mapping vers gRPC `Status` :
* `InvalidSignature`, `Expired`, `NotYetValid`, `MissingKid`,
  `UnknownKid`, `UnknownIssuer`, `AlgorithmNotAllowed`,
  `AudienceMismatch`, `MissingTenantClaim` → `Unauthenticated`.
* `JwksFetch`, `Internal` → `Internal` (sanitized message à
  client, full err in logs).

---

## 13. Schéma de configuration `EFE_OIDC_*`

### 13.1 Env vars

| Env var | Default | Helm key | Validate |
|---------|--------:|----------|----------|
| `EFE_OIDC_ENABLED` | `false` | `config.auth.oidc.enabled` | bool |
| `EFE_OIDC_ISSUERS_FILE` | `/etc/efe/oidc/issuers.json` | `config.auth.oidc.issuersFile` | non-empty when enabled |
| `EFE_OIDC_JWKS_TTL_S` | `300` | `…jwksTtlSeconds` | ∈ [60, 86 400] |
| `EFE_OIDC_JWKS_FETCH_TIMEOUT_MS` | `5000` | `…jwksFetchTimeoutMs` | ≥ 100 |
| `EFE_OIDC_JWKS_RETRY_MAX` | `3` | `…jwksRetryMax` | ≥ 0 |
| `EFE_OIDC_JWKS_BACKOFF_MS` | `1000` | `…jwksBackoffMs` | ≥ 0 |
| `EFE_OIDC_GRACE_STALE_S` | `3600` | `…graceStaleSeconds` | grace period during fetch fail |

### 13.2 Format `issuers.json`

```jsonc
[
  {
    "issuer": "https://acme.auth0.com/",
    "audience": "efe.v1",
    "jwks_url": "https://acme.auth0.com/.well-known/jwks.json",
    "alg_whitelist": ["RS256"],
    "tenant_strategy": "claim",
    "tenant_claim": "https://efe.example/tenant",
    "max_token_age_s": 3600,
    "leeway_s": 30
  },
  {
    "issuer": "https://keycloak.globex.com/realms/efe",
    "audience": "efe.v1",
    "jwks_url": "https://keycloak.globex.com/realms/efe/protocol/openid-connect/certs",
    "alg_whitelist": ["RS256", "ES256"],
    "tenant_strategy": "issuer",
    "tenant_id_for_issuer": "globex",
    "max_token_age_s": 3600,
    "leeway_s": 30
  }
]
```

### 13.3 Validation au boot

`ServerConfig::validate` quand `oidc.enabled=true` :
* `issuers_file` non-empty + lisible.
* Au moins 1 issuer dans la liste.
* Chaque issuer : `issuer` URL valide HTTPS, `audience`
  non-empty, `jwks_url` HTTPS, `alg_whitelist` ⊂ {RS256, ES256},
  `tenant_strategy` ∈ {claim, issuer}.
* Si strategy=claim : `tenant_claim` non-empty.
* Si strategy=issuer : `tenant_id_for_issuer` valide
  (regex `^[a-z0-9_-]{1,63}$`).

Refus avec `ConfigError::Invalid` au boot.

---

## 14. Documentation deliverables — Diátaxis tree

### 14.1 Structure

```
docs/
├── howto/
│   ├── onboarding-tenant.md            ← NEW (le client)
│   ├── idp-integration-keycloak.md     ← NEW (Tier 1 dev)
│   ├── idp-integration-auth0.md        ← NEW (Tier 2/3)
│   └── idp-integration-okta.md         ← NEW (alternative)
├── reference/
│   ├── idp-config-schema.md            ← NEW (issuers.json)
│   ├── tenant-claim-format.md          ← NEW (custom claim spec)
│   └── auth-error-codes.md             ← NEW (error → status mapping)
├── explanation/
│   └── auth-architecture.md            ← NEW (le pourquoi)
└── runbooks/
    ├── d15_jwks_fetch_failing.md       ← NEW
    ├── d15_idp_outage.md               ← NEW
    └── d15_tenant_onboarding_stuck.md  ← NEW
```

### 14.2 Conventions

* **Howto** : task-oriented, commande-bash centric. Format
  Diátaxis howto : "to achieve X, do Y, Z".
* **Reference** : exhaustif, schema, exemples.
* **Explanation** : rationale, alternatives écartées, threat
  model.
* **Runbook** : alert-driven, "when alert X fires, do Y".

### 14.3 Onboarding howto template (extrait `onboarding-tenant.md`)

```markdown
# How to onboard a new tenant on EFE

## Pre-flight checklist

You need :
- A tenant_id (regex `^[a-z0-9_-]{1,63}$`).
- An IdP that emits JWT tokens (Auth0, Okta, Keycloak,
  Cognito, Azure AD, Google Workspace).
- The IdP's issuer URL and JWKS endpoint.

## Step 1 — Configure your IdP to emit EFE-compatible tokens

Tokens MUST contain :

| Claim | Required | Type | Notes |
|-------|:--------:|------|-------|
| `iss` | ✓ | string (URL) | Must match config |
| `aud` | ✓ | string | Must equal `efe.v1` |
| `sub` | ✓ | string | User identifier |
| `exp` | ✓ | int | Unix seconds, ≤ now + 1h |
| `iat` | ✓ | int | Unix seconds |
| `tenant_id` | ✓ | string | Your tenant_id |
| `scopes` | ✓ | string (space-sep) or array | EFE scopes (e.g. `admin:catalog:write`) |

## Step 2 — Provide details to the operator

Send this payload to the EFE operator (eng@efe.ai or via
your usual support channel) :

```json
{
  "tenant_id": "acme",
  "display_name": "ACME Inc",
  "idp_issuer": "https://acme.auth0.com/",
  "jwks_url": "https://acme.auth0.com/.well-known/jwks.json",
  "tenant_strategy": "claim",
  "tenant_claim": "https://efe.example/tenant"
}
```

## Step 3 — Wait for activation

The operator runs `efe-onboard activate --tenant acme`. You'll
receive a confirmation email.

## Step 4 — Smoke test

```bash
TOKEN=$(curl -s ... your IdP token endpoint)
grpcurl -plaintext -H "authorization: Bearer $TOKEN" \
    your.efe.endpoint:50051 \
    efe.v1.catalog.CatalogService/ListCatalogs \
    -d '{"tenant":"acme","page_size":10}'
```

If you see `{"catalogs":[],"next_page_token":""}` — you're in.
```

---

## 15. Schéma SQL `tenants` (registre)

### 15.1 Migration `0004_tenants.sql`

```sql
CREATE TABLE tenants (
    tenant_id     TEXT PRIMARY KEY,
    display_name  TEXT NOT NULL,
    status        TEXT NOT NULL,              -- 'pending'|'active'|'suspended'
    idp_issuer    TEXT NOT NULL,
    idp_strategy  TEXT NOT NULL,              -- 'claim'|'issuer'
    idp_meta      JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    activated_at  TIMESTAMPTZ,
    suspended_at  TIMESTAMPTZ,

    CONSTRAINT tenants_id_format CHECK (tenant_id ~ '^[a-z0-9_-]{1,63}$'),
    CONSTRAINT tenants_status_enum CHECK (status IN ('pending', 'active', 'suspended')),
    CONSTRAINT tenants_idp_strategy_enum CHECK (idp_strategy IN ('claim', 'issuer'))
);

CREATE INDEX tenants_idp_issuer_idx ON tenants (idp_issuer);
CREATE INDEX tenants_status_idx ON tenants (status);

-- Trigger : audit every INSERT/UPDATE.
CREATE OR REPLACE FUNCTION tenants_audit_change()
RETURNS TRIGGER AS $$
BEGIN
    INSERT INTO audit_events (
        event_id, chain_id, tenant, action, op_tag, catalog_key,
        principal, description, parent_key, content_sha256,
        created_at_unix_ms, canonical_payload, prev_hmac, hmac, key_id
    ) SELECT
        gen_random_uuid(),  -- event_id (NOT v7 here, but unique)
        'tenant_admin',
        NEW.tenant_id,
        TG_OP,              -- 'INSERT' or 'UPDATE'
        'tenant_admin',
        NEW.tenant_id,
        current_user,
        format('status=%s', NEW.status),
        '',
        '',                  -- no content_sha256
        EXTRACT(EPOCH FROM now())::BIGINT * 1000,
        row_to_json(NEW)::jsonb,
        '0000000000000000000000000000000000000000000000000000000000000000',
        '0000000000000000000000000000000000000000000000000000000000000000',
        'tenant_admin'
    ;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER tenants_audit_change_trg
    AFTER INSERT OR UPDATE ON tenants
    FOR EACH ROW EXECUTE FUNCTION tenants_audit_change();
```

**Note** : le trigger insère un audit event avec un HMAC fake
("0...0"). Pour un vrai HMAC, la chain `tenant_admin` devrait
être signée séparément côté Rust. Cette simplification SQL est
acceptable v0.1.15 — l'opérateur tenant_admin est lui-même
audité par `pg_audit` côté DBA layer.

### 15.2 Indexes

| Index | Use case |
|-------|----------|
| `tenants_pkey` | exact lookup `WHERE tenant_id = ?` |
| `tenants_idp_issuer_idx` | reverse lookup "qui utilise cet issuer ?" |
| `tenants_status_idx` | filter active vs suspended |

---

## 16. Helm wiring — issuers + Secret + sidecar

### 16.1 `values.yaml` additions

```yaml
config:
  auth:
    # ... existing hs256 block ...
    oidc:
      enabled: false
      issuersFile: "/etc/efe/oidc/issuers.json"
      jwksTtlSeconds: 300
      jwksFetchTimeoutMs: 5000
      jwksRetryMax: 3
      jwksBackoffMs: 1000
      graceStaleSeconds: 3600
      # Reference to a Kubernetes Secret with `issuers.json` key.
      existingSecretName: ""
      existingSecretKey: "issuers.json"
```

### 16.2 ConfigMap rendering

```yaml
{{- if .Values.config.auth.oidc.enabled }}
EFE_OIDC_ENABLED: "true"
EFE_OIDC_ISSUERS_FILE: {{ .Values.config.auth.oidc.issuersFile | quote }}
EFE_OIDC_JWKS_TTL_S: {{ .Values.config.auth.oidc.jwksTtlSeconds | quote }}
EFE_OIDC_JWKS_FETCH_TIMEOUT_MS: {{ .Values.config.auth.oidc.jwksFetchTimeoutMs | quote }}
EFE_OIDC_JWKS_RETRY_MAX: {{ .Values.config.auth.oidc.jwksRetryMax | quote }}
EFE_OIDC_JWKS_BACKOFF_MS: {{ .Values.config.auth.oidc.jwksBackoffMs | quote }}
EFE_OIDC_GRACE_STALE_S: {{ .Values.config.auth.oidc.graceStaleSeconds | quote }}
{{- end }}
```

### 16.3 Volume mount Secret-backed

```yaml
{{- if and .Values.config.auth.oidc.enabled .Values.config.auth.oidc.existingSecretName }}
- name: oidc-issuers
  mountPath: /etc/efe/oidc
  readOnly: true
volumes:
- name: oidc-issuers
  secret:
    secretName: {{ .Values.config.auth.oidc.existingSecretName | quote }}
    defaultMode: 0400
    items:
      - key: {{ .Values.config.auth.oidc.existingSecretKey | quote }}
        path: issuers.json
{{- end }}
```

### 16.4 Sidecar JWKS-prefetcher (optionnel, Tier 3)

Pour les déploiements Tier 3 où on veut découpler la fetch
HTTP de l'app principale (e.g. mTLS-only IdP nécessitant un
sidecar Envoy), on documente un pattern sidecar :

```yaml
extraContainers:
  - name: jwks-prefetcher
    image: ghcr.io/efe-project/jwks-prefetcher:0.1.0
    args: ["--issuers-file", "/etc/efe/oidc/issuers.json", "--out", "/var/jwks-cache"]
    volumeMounts:
      - name: oidc-issuers
        mountPath: /etc/efe/oidc
      - name: jwks-cache
        mountPath: /var/jwks-cache
extraVolumeMounts:
  - name: jwks-cache
    mountPath: /var/jwks-cache
    readOnly: true
extraVolumes:
  - name: jwks-cache
    emptyDir: {}
```

Le sidecar fetch les JWKS et écrit `/var/jwks-cache/<sha256(issuer)>.json`.
EFE lit depuis le cache file ; pas de HTTP fetch in-process.
**Pas livré v0.1.15** — documentation only.

---

## 17. Runtime wire-up — `Server::build` augmenté

```rust
// runtime.rs — D15 additions

let token_store = if config.auth.enabled {
    let mut backends = Vec::new();
    // OIDC backend (preferred when configured).
    if config.auth.oidc.enabled {
        let issuers = IssuerRegistry::from_file(
            std::path::Path::new(&config.auth.oidc.issuers_file),
        )?;
        let oidc = OidcValidator::new(Arc::new(issuers));
        backends.push(TokenBackend::Oidc(oidc));
    }
    // HS256 backend (legacy fallback).
    if !config.auth.hs256_secrets.is_empty() {
        let hs = TokenBackend::Hs256 {
            keys: config.auth.hs256_secrets.clone(),
            audience: config.auth.jwt_audience.clone(),
            issuer: config.auth.jwt_issuer.clone(),
        };
        backends.push(hs);
        if config.auth.oidc.enabled {
            tracing::warn!(
                target: "efe.auth",
                "HS256 + OIDC both enabled — gradual migration mode. \
                 Plan deprecation of HS256 by v0.1.20."
            );
        }
    }
    if backends.is_empty() {
        return Err(anyhow!(
            "auth.enabled=true but no backend configured (HS256 or OIDC)"
        ));
    }
    Arc::new(TokenStore::new(backends))
} else {
    Arc::new(TokenStore::disabled())
};
```

JWKS periodic refresh spawned in `Server::serve` :

```rust
// In serve(), after binding the listener :
if let TokenBackend::Oidc(oidc_validator) = ... {
    for issuer in oidc_validator.issuers() {
        let cache = issuer.jwks_cache.clone();
        let ttl = Duration::from_secs(issuer.jwks_ttl_s);
        let shutdown = shutdown.clone();
        tokio::spawn(jwks::periodic_refresh(cache, ttl, shutdown));
    }
}
```

---

## 18. Observability D13/D14/D14-bis extensions

### 18.1 Nouveaux instruments D15

```rust
// crates/efe-telemetry/src/metrics/d15_instruments.rs

once_metric!(
    /// `efe_oidc_tokens_validated_total{issuer, alg, result}` — count
    /// of OIDC tokens checked. result = "ok" | "expired" | "bad_sig" |
    /// "unknown_kid" | "unknown_issuer" | "alg_not_allowed" | …
    oidc_tokens_validated_total, IntCounterVec, …
);

once_metric!(
    /// `efe_jwks_fetch_total{issuer, result}` — count of JWKS HTTP
    /// fetches. result = "ok" | "timeout" | "http_error" | "parse_error".
    jwks_fetch_total, IntCounterVec, …
);

once_metric!(
    /// `efe_jwks_cache_size{issuer}` — number of pub keys in cache.
    jwks_cache_size, IntGaugeVec, …
);

once_metric!(
    /// `efe_jwks_cache_age_seconds{issuer}` — time since last
    /// successful refresh. Alert `D15IdpJwksStale` fires at > 2 ×
    /// configured TTL.
    jwks_cache_age_seconds, IntGaugeVec, …
);

once_metric!(
    /// `efe_jwks_stale_total{issuer}` — count of validations served
    /// from a stale cache (during fetch failure within the grace
    /// period).
    jwks_stale_total, IntCounterVec, …
);

once_metric!(
    /// `efe_oidc_issuers_loaded` — number of issuers in the registry.
    /// Sanity check : 0 = misconfiguration.
    oidc_issuers_loaded, IntGauge, …
);

once_metric!(
    /// `efe_tenants_total{status}` — count of tenants by status.
    tenants_total, IntGaugeVec, …
);
```

### 18.2 Spans

```rust
pub const OIDC_VALIDATE:    &str = "efe.oidc.validate";
pub const JWKS_FETCH:       &str = "efe.jwks.fetch";
pub const TENANT_ONBOARD:   &str = "efe.tenant.onboard";
```

### 18.3 Alerts

| Alert | Condition | Severity |
|-------|-----------|:--------:|
| `D15OidcAuthFailureSurge` | `rate(efe_oidc_tokens_validated_total{result!="ok"}[5m]) > 5/sec` | 🟡 warning |
| `D15IdpJwksFetchFailing` | `rate(efe_jwks_fetch_total{result!="ok"}[10m]) > 0` | 🟡 warning |
| `D15IdpJwksStale` | `efe_jwks_cache_age_seconds > 2 × ttl` | 🔴 critical |
| `D15OidcIssuersZero` | `efe_oidc_issuers_loaded == 0` (when oidc enabled) | 🔴 critical |
| `D15TenantOnboardingStuck` | tenants in `pending` > 24 h | 🟡 warning |

### 18.4 Dashboards

* **`d15_oidc_auth.json`** : tokens validated / minute, error
  breakdown, latency histogram.
* **`d15_jwks_health.json`** : cache age per issuer, fetch
  success rate, key inventory.
* **`d15_tenant_onboarding.json`** : tenants by status, onboard
  funnel, time-to-active distribution.

---

## 19. Tests — ~50 nouveaux tests

### 19.1 Unit tests

| Module | # tests | Cible |
|--------|--------:|-------|
| `auth/oidc.rs::tests` | 15 | Algorithm whitelist, alg confusion rejected, none alg rejected, kid mismatch, audience match, exp expired, nbf future, multi-issuer dispatch, tenant claim extraction, tenant strategy=issuer fallback |
| `auth/jwks.rs::tests` | 8 | Cache hit, cache miss, TTL expiry, retry exponential backoff, stale grace period, force refresh on SIGHUP, parse JWKS document, unknown kid |
| `auth/issuers.rs::tests` | 6 | Parse issuers.json, validate URLs, reject duplicate issuers, reject empty file, missing tenant_claim with strategy=claim → error |
| `auth/tenants.rs::tests` | 4 | CRUD pending/active/suspended, status transition validation |

### 19.2 Integration tests (gated `EFE_PG_TEST_URL` + mock JWKS server)

| File | # tests |
|------|--------:|
| `tests/d15_oidc_integration.rs` | 12 (token verify happy path, multi-issuer, kid rotation, JWKS fetch failure grace, alg confusion, tenant claim missing, tenant strategy=issuer, etc.) |
| `tests/d15_tenant_onboarding.rs` | 5 (preflight, activate, suspend, list_pending, audit logged) |

### 19.3 Python pin tests

| File | # tests |
|------|--------:|
| `scripts/tests/test_d15_oidc_helm_render.py` | 12 |
| `scripts/tests/test_d15_oidc_runtime_wireup.py` | 10 |
| `scripts/tests/test_d15_docs_diataxis.py` | 8 (each howto exists, references valid, anchors exist) |

### 19.4 Mock JWKS server

```rust
// tests/d15_oidc_integration.rs

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn mock_jwks_server(jwks_doc: serde_json::Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jwks_doc))
        .mount(&server).await;
    server
}
```

`wiremock` est déjà workspace-friendly, pas de testcontainers.

---

## 20. Migration compat HS256 → OIDC

### 20.1 Phase 1 — Co-existence (v0.1.15)

* HS256 + OIDC simultanément actifs.
* Dispatch par `iss` claim : si configuré OIDC, OIDC ; sinon HS256.
* Warning log boot + sample 1/100 token verify "HS256 deprecated".

### 20.2 Phase 2 — HS256 deprecated (v0.1.16, ~D17)

* `EFE_AUTH_HS256_DEPRECATED_WARN=true` par défaut.
* Helm values.yaml par défaut `hs256.enabled=false`.
* Tier 1 dev qui veut HS256 doit explicitly set `hs256.enabled=true`.

### 20.3 Phase 3 — HS256 removed (v0.1.20, ~D20)

* Code path supprimé.
* Tests HS256 marked `#[ignore]`.
* Fail-fast au boot si `hs256_secrets` set.

### 20.4 Migration playbook (clients existants)

```
1. Deploy v0.1.15 (no behaviour change).
2. Configure your IdP to emit JWT with our claim format.
3. Set EFE_OIDC_ENABLED=true alongside HS256.
4. Validate tokens from your IdP work.
5. Remove EFE_JWT_SECRETS_FILE references — switch to OIDC only.
6. Upgrade to v0.1.20.
```

---

## 21. Decisions log — récap des décisions clés

| # | Décision | Alternatives écartées |
|--:|----------|----------------------|
| 1 | OIDC standard (RFC ubiquitous) | Auth0/Okta-specific, Keycloak-specific |
| 2 | RS256 + ES256 | Ed25519 (less ubiquitous IdP support) |
| 3 | JWKS pull avec cache 5 min + grace stale | Push (firewall hole), no cache (DoS sensitivity) |
| 4 | Tenant strategy claim default + issuer fallback | sub parsing (custom logic, fragile) |
| 5 | Multi-issuer registry | 1-issuer-only (limite multi-tenant) |
| 6 | HS256 backwards compat dev-only | Breaking removal v0.1.15 |
| 7 | alg whitelist per issuer | Globale (algorithm confusion possible) |
| 8 | 4-phase onboarding (preflight, bootstrap, steady, audit) | Single-phase (no audit handoff documenté) |
| 9 | `tenants` table SQL + audit trigger | In-memory tenants list (pas de persistence) |
| 10 | wiremock pour CI (pas testcontainers IdP) | Keycloak in CI (heavy + slow) |

---

## 22. Risques + mitigations

| ID | Sévérité | Risque | Mitigation |
|----|:--------:|--------|------------|
| **R-D15.1** | 🔴 HIGH | JWKS fetch fail pendant rotation IdP → tous tokens rejetés | Grace period 1 h via cache stale ; alert `D15IdpJwksStale` |
| **R-D15.2** | 🔴 HIGH | Algorithm confusion attack | alg_whitelist per issuer ; pin test forge HS256-against-RS256 → reject |
| **R-D15.3** | 🟠 MED | IdP outage (Auth0 down) → tous validations fail | Multi-issuer permet de fallback sur backup IdP ; documented in runbook |
| **R-D15.4** | 🟠 MED | Tenant claim spoofing | Signature verify rejette ; le claim est dans le payload signé |
| **R-D15.5** | 🟠 MED | Doc onboarding obsolète vs realité | CI pin test que howto refs files existants ; test smoke `efe-onboard preflight` end-to-end |
| **R-D15.6** | 🟡 LOW | HS256 + OIDC mismatch dispatch | Test pin per-issuer dispatch correct |
| **R-D15.7** | 🟡 LOW | JWKS cache cardinality explosion (10⁶ kids) | Bounded cache 100 kids/issuer ; LRU eviction |

---

## 23. Plan wallclock — 6 j

| Jour | Activités | Output |
|-----:|-----------|--------|
| **J1** | Config (env vars + validate) + Helm rendering + lib.rs exports + Python pins | `config.rs::OidcConfig` + values.yaml + 12 Python pins |
| **J2** | `auth/issuers.rs` (parse + validate) + `auth/jwks.rs` (cache + fetch + retry) | ~700 lignes + 14 unit tests verts |
| **J3** | `auth/oidc.rs` (validator + multi-issuer dispatch) + extension `TokenStore` | ~600 lignes + 15 unit tests verts |
| **J4** | Migration `0004_tenants.sql` + `auth/tenants.rs` (CRUD) + CLI `efe-onboard` | ~800 lignes + integration tests |
| **J5** | 3 howto + 3 reference + 1 explanation + 3 runbook (Diátaxis) | 10 docs Markdown |
| **J6** | Mock JWKS integration tests + wiremock setup + observability (7 instruments + 3 spans + 5 alerts) + audit interne (skeleton) | release v0.1.15 RC + `docs/audits/d15_audit_report.md` skeleton |
| **+1 j buffer** | Audit interne fixes (~5-7 bugs attendus pattern D8/D14) | post-audit corrections |

**Total : 6 j wallclock + 1 j buffer.**

---

## 24. SLO ajoutés en D15

| ID | Description | Cible | Type |
|----|-------------|------:|------|
| **D15-AUTH-1** | OIDC token verify p99 | ≤ 2 ms | GATE |
| **D15-AUTH-2** | JWKS fetch p99 (loopback / mock) | ≤ 100 ms | OBSERVATIONAL |
| **D15-AUTH-3** | JWKS cache age max | ≤ 2 × TTL | ALERT |
| **D15-AUTH-4** | OIDC tokens validated rate, sustained | ≥ 0 (no missing dashboard) | OBSERVATIONAL |
| **D15-AUTH-5** | Algorithm confusion attack | refused 100 % | INVARIANT |
| **D15-AUTH-6** | Multi-issuer dispatch correctness | iss=A → keys=A only | INVARIANT |
| **D15-DOC-1** | Onboarding time-to-first-RegisterCatalog (manual e2e) | ≤ 1 h | OBSERVATIONAL |
| **D15-DOC-2** | Diátaxis tree completeness (7 docs présents + refs valides) | 100 % | GATE CI |

---

## 25. Anti-régression D15

```bash
# Lib (auth + oidc + jwks + issuers + tenants)
cargo test -p efe-server --lib auth::

# OIDC integration tests (mock JWKS via wiremock — no testcontainers)
cargo test -p efe-server --test d15_oidc_integration

# Helm + runtime + docs pin tests
PYTHONPATH=clients/python python3 -m pytest scripts/tests/test_d15_*.py

# Cross-sprint regression (preserves D8/D11/D12/D12-bis/D13/D14/D14-bis)
PYTHONPATH=clients/python python3 -m pytest scripts/tests/ -q
cargo test -p efe-server --lib --test d12bis_catalog_integration \
    --test d14_postgres_catalog_integration --test d14bis_audit_hmac_integration \
    --test d15_oidc_integration
```

Cibles :
* Tests Rust D15 : ~50 verts.
* Tests Python D15 : ~30 verts.
* 0 régression sur D8/D11/D12/D12-bis/D13/D14/D14-bis.

---

## 26. Hand-off vers D16

D15 ouvre 4 sprints futurs :

* **D16 (MFA + acr enforcement)** :
  - `acr` claim parsing (Authentication Context Class Reference).
  - Per-RPC ACR enforcement (`admin:catalog:write` exige `acr=mfa`).
  - Documented in `docs/howto/enforce-mfa-on-write-rpc.md`.

* **D17 (jti registry + refresh token rotation)** :
  - Replay protection avancée via Redis-backed `jti` registry.
  - Token revocation list.

* **D18+ (Onboarding self-service web UI)** :
  - React/Next app branchée sur `efe-onboard` CLI.
  - Tenant admin dashboard.

* **D14-ter (Ed25519 audit + Ed25519 OIDC)** :
  - Audit signing en Ed25519 verifiable-by-third-party.
  - OIDC Ed25519 algorithm support.

---

## 27. Annexes

### Annexe A — `EFE_OIDC_*` env vars matrix

(cf. § 13.1)

### Annexe B — JWT claims schema reference

```jsonc
{
  "iss": "https://acme.auth0.com/",         // REQUIRED, must match config
  "aud": "efe.v1",                           // REQUIRED
  "sub": "auth0|abc123",                     // REQUIRED
  "exp": 1700003600,                         // REQUIRED, ≤ now + 3600s
  "iat": 1700000000,                         // REQUIRED
  "nbf": 1700000000,                         // OPTIONAL
  "tenant_id": "acme",                       // REQUIRED (when strategy=claim)
  "scopes": "admin:catalog:read admin:catalog:write",  // REQUIRED, space-separated
  "acr": "mfa"                               // OPTIONAL (D16+)
}
```

### Annexe C — IdP test matrix

| IdP | RS256 | ES256 | Custom claim | Issuer-based | Notes |
|-----|:-----:|:-----:|:------------:|:------------:|-------|
| **Keycloak** | ✓ | ✓ (14+) | ✓ via mapper | ✓ via realms | Reference Tier 1 |
| **Auth0** | ✓ | ✗ (RS256-only) | ✓ via Action | ✓ via Organizations | Reference Tier 2/3 |
| **Okta** | ✓ | ✓ | ✓ via custom claims | ✓ via authorization servers | Tested manual |
| **AWS Cognito** | ✓ | ✗ | ✓ via custom attribute | ✗ | Tested manual |
| **Azure AD** | ✓ | ✗ | ✓ via app role | ✓ via tenant id | Tested manual |
| **Google Workspace** | ✓ | ✗ | ✗ (no custom claim natively) | ✓ via hd domain | Limited |

### Annexe D — Bibliographie

* **RFC 6749** OAuth 2.0 : <https://datatracker.ietf.org/doc/html/rfc6749>
* **RFC 7519** JWT : <https://datatracker.ietf.org/doc/html/rfc7519>
* **RFC 7515** JWS : <https://datatracker.ietf.org/doc/html/rfc7515>
* **RFC 7517** JWK : <https://datatracker.ietf.org/doc/html/rfc7517>
* **RFC 8414** OAuth 2.0 Authorization Server Metadata
* **OIDC Core 1.0** : <https://openid.net/specs/openid-connect-core-1_0.html>
* **OWASP JWT Cheat Sheet** : <https://cheatsheetseries.owasp.org/cheatsheets/JSON_Web_Token_for_Java_Cheat_Sheet.html>
* **NIST SP 800-63B** Authentication & Lifecycle
* **Diátaxis** : <https://diataxis.fr/>

---

> **Architecture finalisée** — 2026-04-27.
> **Prochaine étape** : J1 du plan wallclock (config + Helm + Python pins).
> **Audit interne D15** : à programmer en J7 selon pattern D8 / D12-bis / D14 / D14-bis.
