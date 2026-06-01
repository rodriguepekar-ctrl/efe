# M11 — Security, Multi-tenancy, Compliance — Architecture détaillée

> **Statut** : Draft v1.0.0 — Document normatif pour le plan de confiance
> (trust plane) de la plateforme EFE.
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-21
> **Dépendances doc** : `M01_ARCHITECTURE.md` (fondations), `M02_ARCHITECTURE.md`
> (ingestion + PII amont), `M03_ARCHITECTURE.md` (stockage), `M05_ARCHITECTURE.md`
> (fine-tuning), `M09_ARCHITECTURE.md` (dérive), `M10_SCHEDULING_ARCHITECTURE.md`
> (scheduling), `MODE1_ARCHITECTURE.md` (classifier par essaim).
> **Périmètre** : le plan de confiance unifié — authentification, autorisation,
> isolation multi-tenant, gestion des secrets, protection des données
> personnelles, piste d'audit inviolable, conformité réglementaire (RGPD,
> AI Act, SOC 2, HIPAA si opt-in), et réponse à incident.
> **Précision** : chirurgicale, claim-par-claim, tag-par-tag, contrôle-par-
> contrôle.
> **Note** : ce document décrit le **nouveau** M11 dédié à la sécurité et à la
> conformité. Tout fichier M11 pré-existant concernant l'API REST est
> préservé ; M11-Security établit les contrats transverses que M11-API
> applique en bordure.

---

## Sommaire

1. [Mission de M11](#1-mission-de-m11)
2. [Positionnement dans l'architecture EFE](#2-positionnement-dans-larchitecture-efe)
3. [Principes directeurs](#3-principes-directeurs)
4. [Invariants structurels](#4-invariants-structurels)
5. [Modèle de menace (threat model)](#5-modèle-de-menace-threat-model)
6. [Acteurs, rôles et topologie](#6-acteurs-rôles-et-topologie)
7. [Vue d'ensemble architecturale](#7-vue-densemble-architecturale)
8. [Sous-système 11.1 — **Authentification**](#8-sous-système-111--authentification)
9. [Sous-système 11.2 — **Autorisation (RBAC + ABAC + RLS)**](#9-sous-système-112--autorisation-rbac--abac--rls)
10. [Sous-système 11.3 — **Multi-tenancy**](#10-sous-système-113--multi-tenancy)
11. [Sous-système 11.4 — **Gestion des secrets & cryptographie**](#11-sous-système-114--gestion-des-secrets--cryptographie)
12. [Sous-système 11.5 — **Protection des données personnelles (PII / PHI)**](#12-sous-système-115--protection-des-données-personnelles-pii--phi)
13. [Sous-système 11.6 — **Piste d'audit inviolable**](#13-sous-système-116--piste-daudit-inviolable)
14. [Sous-système 11.7 — **Consentement et base légale**](#14-sous-système-117--consentement-et-base-légale)
15. [Sous-système 11.8 — **Cycle de vie des données**](#15-sous-système-118--cycle-de-vie-des-données)
16. [Sous-système 11.9 — **Réponse à incident**](#16-sous-système-119--réponse-à-incident)
17. [Sous-système 11.10 — **Agilité cryptographique**](#17-sous-système-1110--agilité-cryptographique)
18. [Sous-système 11.11 — **Mappings de conformité**](#18-sous-système-1111--mappings-de-conformité)
19. [Budget de performance](#19-budget-de-performance)
20. [Taxonomie d'erreurs](#20-taxonomie-derreurs)
21. [Stratégie de tests](#21-stratégie-de-tests)
22. [Plan de sprints](#22-plan-de-sprints)
23. [Critères Go / No-Go spécifiques à M11](#23-critères-go--no-go-spécifiques-à-m11)
24. **Annexes**
    - A. [Domain-separation tags HMAC étendus](#annexe-a--domain-separation-tags-hmac-étendus)
    - B. [Structure canonique du JWT interne](#annexe-b--structure-canonique-du-jwt-interne)
    - C. [Matrice RBAC — rôles × scopes](#annexe-c--matrice-rbac--rôles--scopes)
    - D. [Classification PII/PHI — niveaux et règles](#annexe-d--classification-piiphi--niveaux-et-règles)
    - E. [DDL Postgres — tables M11](#annexe-e--ddl-postgres--tables-m11)
    - F. [Matrice de risques M11](#annexe-f--matrice-de-risques-m11)
    - G. [Matrice de conformité](#annexe-g--matrice-de-conformité)
    - H. [Glossaire sécurité](#annexe-h--glossaire-sécurité)

---

## §1. Mission de M11

### §1.1 Énoncé opérationnel

M11 est **la colonne vertébrale de la confiance de la plateforme EFE**. Son
rôle unique et opposable est :

> *Fournir un plan de contrôle transverse garantissant que (a) chaque requête
> entrante est authentifiée, (b) chaque ressource est consultée ou modifiée
> dans le respect d'une politique explicite, (c) chaque tenant est
> cryptographiquement isolé de tous les autres, (d) chaque donnée personnelle
> est classifiée, minimisée et protégée selon la réglementation applicable,
> (e) chaque décision algorithmique est enregistrée dans une piste d'audit
> inviolable exportable à la demande, et (f) toute opération contrevenant à
> l'un de ces points échoue bruyamment plutôt que silencieusement.*

Ce mandat se décompose en onze responsabilités non-chevauchantes :

1. **Authentifier** tous les appels entrants — utilisateurs humains,
   services internes (mTLS), systèmes externes (JWT).
2. **Autoriser** chaque opération via un modèle composite RBAC + ABAC.
3. **Isoler** multi-tenant à toutes les couches — base de données (RLS),
   objets (chiffrement per-tenant), réseau (NetworkPolicy K8s), calcul
   (cgroups / namespaces).
4. **Gérer** les secrets cryptographiques — HMAC, AES-GCM, signatures
   TLS — avec rotation automatique et séparation DEK/KEK.
5. **Protéger** les données personnelles — classification, masking,
   pseudonymisation, expiration.
6. **Auditer** tout événement sensible dans un journal append-only scellé
   cryptographiquement et tamper-evident.
7. **Gérer** le consentement et la base légale par tenant / sujet.
8. **Orchestrer** le cycle de vie des données — ingestion → transformation
   → rétention → destruction irréversible.
9. **Détecter** et **répondre** aux incidents de sécurité — alertes,
   confinement, forensique.
10. **Offrir** l'agilité cryptographique — rotation de clés, ajout
    d'algorithmes, dépréciation ordonnée.
11. **Mapper** les contrôles techniques aux obligations réglementaires
    (RGPD, AI Act, SOC 2, HIPAA).

### §1.2 Ce que M11 **n'est pas**

- **Pas un WAF** : la protection réseau périphérique (DDoS, injection SQL
  au niveau HTTP) relève de la gateway (Envoy + ModSecurity + Cloud WAF).
- **Pas un SIEM** : M11 produit des *logs* de sécurité de qualité ; leur
  agrégation, corrélation et alerting centralisé sont délégués à un SIEM
  externe (Splunk, Datadog, Panther).
- **Pas un IAM externe** : M11 fédère avec un IdP (Okta, Auth0, Azure AD,
  Keycloak) ; il ne prétend pas gérer les mots de passe utilisateurs ni
  les flux SAML.
- **Pas un antivirus** : la détection de malware dans les artefacts ML
  relève d'un scanner dédié (Clam, YARA) sur le pipeline de training.
- **Pas un certificat officieux** : M11 consomme des certificats émis par
  une AC (cert-manager + Let's Encrypt en dev, AC interne + cert-manager
  en prod). Il n'en émet pas.

### §1.3 Promesses externes

M11 garantit, contractuellement :

| Promesse | Valeur opposable |
|---|---|
| Toute requête non authentifiée est rejetée avant d'atteindre la couche métier | 401 en ≤ 1 ms en p99 |
| Toute autorisation est décidée par un unique engine (PDP central) | 0 code métier ne compare de scope localement |
| Toute fuite cross-tenant est détectable via un test automatisé | Test CI « tenant poisoning » bloqué |
| Tout secret cryptographique est rotation-ready sous 24 h | KEK rotation automatique mensuelle |
| Toute donnée personnelle est classifiée à l'ingestion | 0 champ `text_prompt` non taggé en base |
| L'audit trail est tamper-evident | Vérification de la chaîne Merkle tous les 5 min |
| Toute demande DSAR (Data Subject Access Request) est traitée ≤ 7 jours | Pipeline automatisé, intervention manuelle optionnelle |
| Toute demande d'effacement RGPD est exécutée ≤ 30 jours | Marquage immédiat + destruction asynchrone |
| Aucun secret n'est écrit en log | Filtrage obligatoire au niveau du logger |
| Toute migration détruisant des données doit passer par un flag explicite | `EFE_ALLOW_DESTRUCTIVE_MIGRATION=yes` requis |

---

## §2. Positionnement dans l'architecture EFE

### §2.1 Graphe de dépendances

```
 ┌───────────────┐     ┌───────────────┐     ┌───────────────┐
 │ Utilisateur   │     │ Service       │     │ Partenaire    │
 │ (humain)      │     │ interne       │     │ externe       │
 └───────┬───────┘     └───────┬───────┘     └───────┬───────┘
         │ OIDC                │ mTLS + JWT          │ JWT signé
         ▼                     ▼                     ▼
 ┌─────────────────────────────────────────────────────────────┐
 │                  Gateway (Envoy)                            │
 │   TLS termination · rate limit · request_id injection       │
 └─────────────────────────────┬───────────────────────────────┘
                               │
                               ▼
 ┌─────────────────────────────────────────────────────────────┐
 │                     M11 — Trust plane                       │
 │                                                             │
 │  ┌─────────┐ ┌─────────┐ ┌────────────┐ ┌──────────────┐   │
 │  │ AuthN   │ │ AuthZ   │ │ Tenancy    │ │ PII Guard    │   │
 │  │ (JWT+   │ │ (PDP)   │ │ context    │ │ (classifier+ │   │
 │  │  mTLS)  │ │         │ │ injection  │ │  masker)     │   │
 │  └────┬────┘ └────┬────┘ └──────┬─────┘ └──────┬───────┘   │
 │       │           │             │              │            │
 │       └───────────┴─────────────┴──────────────┘            │
 │                          │                                  │
 │                          ▼                                  │
 │        ┌───────────────────────────────────────┐            │
 │        │ AuditSink (append-only, sealed)       │            │
 │        └───────────────────────────────────────┘            │
 └─────────────────────────────┬───────────────────────────────┘
                               │
                               ▼
 ┌─────────────────────────────────────────────────────────────┐
 │   Modules métier : M02 · M03 · M05 · M06 · M07 · M09 · M10  │
 └─────────────────────────────────────────────────────────────┘
                               │
                               ▼
 ┌─────────────────────────────────────────────────────────────┐
 │  Vault (KMS) · Postgres (RLS) · Object store (SSE-KMS)      │
 │  NATS (mTLS + cred ACL) · Observability (OTel + Prometheus) │
 └─────────────────────────────────────────────────────────────┘
```

M11 est un **plan transversal** : il n'est jamais appelé « en bout de
chaîne » par une ressource métier ; il est traversé comme une couche
d'injection de contexte. Les modules métier reçoivent un
`SecurityContext` immuable qu'ils ne peuvent qu'inspecter — jamais
modifier.

### §2.2 Couches EFE

| Couche | Présence de M11 | Raison |
|--------|-----------------|--------|
| L1 Fondations | Imports seulement (`efe_platform.clock`, `efe_platform.hashing`) | Fournit des primitives sans dépendre de modules métier |
| L2 Persistance | RLS Postgres + chiffrement au repos + audit store | S'installe par migration Alembic partagée |
| L3 Services | `AuthNService`, `AuthZService`, `PiiService`, `AuditService`, `SecretService` | Cœur métier |
| L4 Orchestration | `SecurityInterceptor`, `TenantContext`, `DataLifecycleDaemon` | Middleware + long-runners |
| L5 API / UI | Décorateur `@requires(...)`, contrats ProblemDetails, routes `/v1/compliance/*` | Surface publique |

### §2.3 Relation avec les autres modules

- **M01** — M11 consomme `hmac_seal`, `canonical_json`, `new_uuid`,
  `Clock`. Il ne les redéfinit pas.
- **M02 (Ingestion)** — M11 y publie les règles PII que l'ingesteur
  applique en amont. M02 est donc **le premier point d'application**
  des règles M11.
- **M03 (Stockage)** — M11 pose la RLS Postgres (via migrations), les
  clés KMS pour le chiffrement des blobs objet, et la table `audit_log`.
- **M05 / M07** — M11 valide les JWT des jobs de training / rollout et
  applique les politiques de confidentialité sur les datasets.
- **M09 / M10** — M11 signe les évaluations de dérive (traçabilité) et
  les incidents de scheduling (non-répudiation).

---

## §3. Principes directeurs

### §3.1 Principes opposables en revue PR

1. **P1 — Fail closed.** En cas d'ambiguïté sur l'autorisation, le
   résultat est `403 forbidden` ou `401 unauthorized`, jamais `allow`.
2. **P2 — Zero trust.** Chaque requête (interne ou externe) est
   authentifiée et autorisée. Aucun réseau n'est « de confiance par
   défaut ».
3. **P3 — Defense in depth.** Chaque contrôle (authN, authZ, RLS,
   chiffrement) est indépendant ; le contournement d'un seul ne
   compromet pas les autres.
4. **P4 — Separation of concerns.** Les décisions d'autorisation passent
   par un unique Policy Decision Point (PDP) ; les vérifications
   dispersées dans le code métier sont interdites par lint CI.
5. **P5 — Least privilege.** Chaque principal possède le minimum de
   scopes nécessaires à sa mission ; les scopes sont révoqués à
   l'expiration de la mission.
6. **P6 — Crypto agility.** Toute utilisation cryptographique passe par
   une interface abstraite permettant la substitution d'algorithme sans
   changement d'API.
7. **P7 — Audit-first.** Toute opération sensible (auth réussi,
   autorisation refusée, export de données, destruction) écrit une ligne
   d'audit **avant** d'exécuter l'action.
8. **P8 — Explicit consent.** Aucun traitement de données personnelles
   n'est exécuté sans une base légale enregistrée et vérifiable.
9. **P9 — Data minimisation.** Les champs personnels sont tronqués,
   pseudonymisés ou hashés à la première opportunité.
10. **P10 — Tamper evidence.** Toute modification rétroactive de
    l'audit trail est détectable par vérification cryptographique.
11. **P11 — Secrets hors du code et hors du log.** Aucun secret n'est
    jamais stocké en dur ou écrit dans un log, même en debug.
12. **P12 — Observable compliance.** Chaque contrôle produit une
    métrique et un événement consultable en temps réel.

### §3.2 Ce que M11 refuse d'implémenter

- **Federation SAML native** — on se borne à l'OIDC ; SAML passe par un
  broker (Keycloak ou équivalent).
- **Mots de passe** — seule l'authentification par token est supportée
  (JWT-OIDC + mTLS) ; les mots de passe vivent dans l'IdP.
- **Chiffrement homomorphique** — hors périmètre MVP ; les calculs se
  font sur données claires dans des environnements isolés.
- **Effacement physique immédiat** — la destruction est soft d'abord
  (marquage), hard ensuite (sous 30 j via daemon de GC).
- **Portable audit across tenants** — l'audit est par-tenant ; aucune
  vue cross-tenant n'existe, même pour l'opérateur de la plateforme.

---

## §4. Invariants structurels

| Code | Nom | Énoncé formel |
|------|-----|---------------|
| **I1** | AuthN obligatoire | Toute requête atteignant la couche métier a un `SecurityContext.principal != None` |
| **I2** | AuthZ centralisée | Toute décision `allow/deny` passe par un appel unique à `AuthZService.decide()` |
| **I3** | Tenant binding | Tout objet persistant porte un `tenant_id` non nul immuable |
| **I4** | RLS enforcement | `SELECT 1 FROM t WHERE tenant_id='X'` depuis un contexte tenant='Y' retourne 0 lignes |
| **I5** | Domain tag HMAC | Chaque usage HMAC porte un tag unique et documenté (invariant partagé avec M01, M10) |
| **I6** | Audit immutability | `UPDATE audit_log` est refusé par la RLS ; seule la table mère accepte des INSERT |
| **I7** | Audit tamper evidence | Pour toute ligne *i > 0*, `hash(i) = SHA-256(seal(i-1) || canonical(i))` |
| **I8** | PII classification | Toute colonne `TEXT` ingérée possède une classification ∈ `{public, internal, pii, phi}` |
| **I9** | Consent binding | Aucune opération de training ou d'inférence ne s'exécute sans `consent_record.purpose` correspondant |
| **I10** | Key rotation bounded | Chaque clé active est âgée de ≤ 90 j ; au-delà, l'API refuse les signatures neuves |
| **I11** | No plaintext secrets | Les fichiers de configuration et les logs ne contiennent aucun secret ≥ 16 octets de haute entropie |
| **I12** | Forward secrecy on channels | TLS 1.3 uniquement ; cipher suites sans PFS rejetées |
| **I13** | DSAR completeness | Pour tout sujet S avec événements, la réponse DSAR contient ≥ 1 ligne par catégorie ayant du contenu |
| **I14** | Erasure guarantee | 30 jours après une demande d'effacement validée, aucune donnée pointant vers le sujet ne subsiste |
| **I15** | Incident notifiability | Tout incident de sécurité détecté est horodaté, scellé, et un préavis RGPD est brouillonné en ≤ 72 h |

---

## §5. Modèle de menace (threat model)

### §5.1 Cadre STRIDE

| Catégorie | Exemple EFE | Mitigation M11 |
|---|---|---|
| **S** Spoofing | Attaquant usurpe l'identité d'un tenant | JWT signé + mTLS + fingerprint binding |
| **T** Tampering | Modification d'un artefact ML en transit | HMAC signature sur manifest + sceau |
| **R** Repudiation | Un opérateur nie avoir déclenché un rollback | Audit trail avec actor + timestamp + seal chain |
| **I** Information disclosure | Exfiltration de PII via requête SQL | RLS + classification + masking + audit des exports |
| **D** Denial of service | Flood d'appels gRPC | Rate-limit per-tenant + circuit breakers |
| **E** Elevation of privilege | Escalade d'un reader vers un admin | Scopes bornés + validation à chaque saut + PDP central |

### §5.2 Acteurs menace modélisés

- **Attaquant externe non authentifié** — peut atteindre la gateway.
  Mitigation : TLS 1.3, WAF, rate-limit, JWT obligatoire.
- **Attaquant externe authentifié (utilisateur légitime compromis)** —
  possède un JWT valide. Mitigation : scopes bornés, RLS, audit, MFA
  côté IdP.
- **Tenant malveillant** — cherche à accéder aux données d'un autre
  tenant. Mitigation : RLS stricte, chiffrement per-tenant, tests
  tenant-poisoning.
- **Insider (opérateur plateforme)** — a un accès étendu à
  l'infrastructure. Mitigation : 4-eyes sur les opérations sensibles,
  audit non-répudiable, break-glass journalisé.
- **Fournisseur d'infrastructure compromis** — cloud provider
  compromis. Mitigation : chiffrement au repos avec clés clients
  (BYOK), minimisation des données en clair.
- **Supply chain attack** — dépendance Python / Rust compromise.
  Mitigation : `pip-audit`, `cargo audit`, SBOM, reproducible builds.

### §5.3 Surface d'attaque et kill-chain

```
 ┌────────────────────────────────────────────────────────────────┐
 │ 1. Reconnaissance   → surface réduite (pas d'enum de tenants)  │
 │ 2. Weaponization    → JWT forgé                                │
 │ 3. Delivery         → Gateway bloque signatures invalides      │
 │ 4. Exploitation     → RLS stoppe toute fuite de données        │
 │ 5. Installation     → pods read-only, capabilities dropped      │
 │ 6. Command & Control→ egress NetworkPolicy strict              │
 │ 7. Actions on obj.  → audit trail détecte exfil volumineux      │
 └────────────────────────────────────────────────────────────────┘
```

### §5.4 Hypothèses de sécurité explicites (security assumptions)

- Le cluster Kubernetes est à jour de ses CVE critiques (SLA ≤ 30 j).
- L'IdP externe applique la MFA pour tous les comptes administrateurs.
- Les clés root HSM ne quittent jamais le matériel.
- Le time-sync du cluster est < 1 s (ntpd + chrony).
- Les sauvegardes Postgres sont chiffrées avec une clé séparée.

---

## §6. Acteurs, rôles et topologie

### §6.1 Rôles logiques

| Rôle | Portée | Droit principal |
|------|--------|-----------------|
| `anonymous` | Gateway uniquement | `/health`, `/ready` |
| `reader` | Tenant × Task | Lire les baselines, drift, jobs, runs |
| `operator` | Tenant × Task | Créer schedules, triggers, acquitter incidents |
| `admin` | Tenant | Toute action sur un tenant (hors compliance) |
| `compliance_officer` | Tenant | Gérer consentements, répondre aux DSAR, purger |
| `platform_operator` | Cluster | Santé, métriques, déploiements (AUCUN accès données) |
| `security_officer` | Cluster | Audit, investigations, break-glass |
| `service_account:ingester` | System | Ingestion d'événements |
| `service_account:worker` | System | Exécution des jobs |
| `service_account:scheduler` | System | Leader election, enqueue |
| `data_subject` | Self | Consulter / rectifier / effacer ses propres données |

### §6.2 Topologie physique

```
 ┌───────────────────────────────────────────────────────────────────┐
 │                  Public edge                                      │
 │   ┌────────────────┐   ┌────────────────┐   ┌────────────────┐    │
 │   │ Cloud WAF      │   │ ALB / Ingress  │   │ cert-manager   │    │
 │   └────────┬───────┘   └────────┬───────┘   └────────┬───────┘    │
 └─────────────┼────────────────────┼──────────────────┼─────────────┘
               │                    │                  │
               ▼                    ▼                  ▼
 ┌───────────────────────────────────────────────────────────────────┐
 │                  Cluster K8s — namespace: efe-prod                │
 │                                                                   │
 │  ┌───────────┐  ┌───────────┐  ┌──────────────┐  ┌─────────────┐  │
 │  │ Gateway   │  │ API pods  │  │ Worker pods  │  │ Scheduler   │  │
 │  │ (Envoy)   │  │ FastAPI   │  │              │  │ Daemon      │  │
 │  └─────┬─────┘  └─────┬─────┘  └──────┬───────┘  └──────┬──────┘  │
 │        │              │               │                  │        │
 │        └──────────────┴───────────────┴──────────────────┘        │
 │                             │                                     │
 │                             ▼                                     │
 │   ┌────────────────────────────────────────────────────────────┐  │
 │   │ Service mesh (Linkerd/Istio) — mTLS auto, mesh policy      │  │
 │   └────────────────────────────────────────────────────────────┘  │
 │                             │                                     │
 │                             ▼                                     │
 │   ┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐       │
 │   │ Postgres │   │ NATS     │   │ Object   │   │ Prometheus│      │
 │   │ (RLS)    │   │ (creds)  │   │ Store    │   │ + Loki    │      │
 │   └──────────┘   └──────────┘   └──────────┘   └──────────┘       │
 │                                                                   │
 │   ┌────────────────────────────────────────────────────────────┐  │
 │   │ Vault (KMS) — ephemeral DEK + long-lived KEK (HSM-backed)  │  │
 │   └────────────────────────────────────────────────────────────┘  │
 └───────────────────────────────────────────────────────────────────┘
```

### §6.3 Modèle de confiance réseau

- **Egress** : par défaut deny-all ; whitelist explicite (KMS, IdP,
  object store, gateway outbound si nécessaire).
- **Ingress** : deny-all sauf depuis la gateway interne.
- **Pod-to-pod** : mTLS via service mesh, auto-rotation 24 h des certs
  de pod (SPIFFE IDs).
- **Human SSH** : interdit en prod ; seul un break-glass JIT via
  bastion journalisé et à révocation automatique ≤ 1 h.

---

## §7. Vue d'ensemble architecturale

### §7.1 Arborescence des sous-modules

```
efe_security/
├── __init__.py                    -- exposition publique
├── context.py                     -- SecurityContext frozen dataclass
├── errors.py                      -- taxonomie d'exceptions
├── fsm.py                         -- FSM consent / key-rotation / audit
│
├── authn/                         -- §8
│   ├── __init__.py
│   ├── jwt_verifier.py            -- signature + claims + exp + audience
│   ├── jwks_cache.py              -- cache JWKS avec rotation
│   ├── mtls_verifier.py           -- SPIFFE ID + peer cert
│   ├── service_account.py         -- issuance + rotation des SA internes
│   ├── session.py                 -- session opaque (hash) + renew
│   └── factor.py                  -- MFA hints (délégué IdP)
│
├── authz/                         -- §9
│   ├── __init__.py
│   ├── policy.py                  -- PolicyRequest / PolicyDecision
│   ├── pdp.py                     -- Policy Decision Point (async)
│   ├── rbac.py                    -- rôles + scopes
│   ├── abac.py                    -- conditions attributaires
│   ├── rls_bridge.py              -- set_config('efe.tenant_id', ...)
│   └── enforcer.py                -- décorateur @requires(...)
│
├── tenancy/                       -- §10
│   ├── __init__.py
│   ├── context.py                 -- TenantContext injection
│   ├── quota.py                   -- quotas par tenant (rps, bytes, compute)
│   ├── isolation.py               -- NetworkPolicy hints, namespace bind
│   └── provisioner.py             -- onboarding + offboarding automatisé
│
├── secrets/                       -- §11
│   ├── __init__.py
│   ├── kms_client.py              -- Vault Transit / AWS KMS adapter
│   ├── envelope.py                -- DEK (AES-GCM) ⊕ KEK
│   ├── rotation.py                -- scheduler + politique de rotation
│   ├── sealer.py                  -- HMAC / Ed25519 sur manifest
│   └── key_registry.py            -- registre de clés actives + rétirées
│
├── pii/                           -- §12
│   ├── __init__.py
│   ├── classifier.py              -- détecteurs PII/PHI (regex + ML optionnel)
│   ├── masker.py                  -- masquage / redaction / tokenisation
│   ├── pseudonymizer.py           -- HMAC(DOMAIN_TAG_USER_ID, user_id)
│   ├── policy.py                  -- PiiPolicy per-tenant / per-feature
│   └── registry.py                -- registre de champs classifiés
│
├── audit/                         -- §13
│   ├── __init__.py
│   ├── records.py                 -- AuditRecord frozen dataclass
│   ├── chain.py                   -- seal chain (Merkle-like linear hash)
│   ├── store.py                   -- AuditStore Protocol + impls
│   ├── exporter.py                -- SIEM / S3 / jq-friendly export
│   ├── verifier.py                -- intégrité de la chaîne
│   └── search.py                  -- requêtes indexées par actor/subject
│
├── consent/                       -- §14
│   ├── __init__.py
│   ├── records.py                 -- ConsentRecord + LegalBasis
│   ├── manager.py                 -- CRUD + binding à l'action
│   ├── dsar.py                    -- DSAR pipeline (access/rectify/erase)
│   └── purpose.py                 -- Purpose catalog + validation
│
├── lifecycle/                     -- §15
│   ├── __init__.py
│   ├── retention.py               -- politiques de rétention par catégorie
│   ├── erasure.py                 -- effacement soft puis hard
│   ├── anonymizer.py              -- anonymisation irréversible
│   └── gc_daemon.py               -- GC asynchrone quotidien
│
├── incident/                      -- §16
│   ├── __init__.py
│   ├── detector.py                -- détecteurs (auth failures, fuites)
│   ├── triage.py                  -- priorité + catégorie
│   ├── runbook.py                 -- playbooks automatisables
│   └── notify.py                  -- rédaction préavis RGPD (72h)
│
├── crypto/                        -- §17
│   ├── __init__.py
│   ├── primitives.py              -- HMAC, AES-GCM, Ed25519, X25519
│   ├── agility.py                 -- algorithm IDs + migration helpers
│   └── prng.py                    -- CSPRNG wrapper (os.urandom + audit)
│
├── compliance/                    -- §18
│   ├── __init__.py
│   ├── gdpr.py                    -- mappings contrôles → articles
│   ├── ai_act.py                  -- classification risque + transparence
│   ├── soc2.py                    -- contrôles SOC 2 CC-1..CC-9
│   ├── hipaa.py                   -- si opt-in BAA
│   └── evidence.py                -- génération preuves d'audit
│
└── api/                           -- §13.3 & §14.3
    ├── __init__.py
    ├── http_app.py                -- FastAPI routes /v1/security/*
    ├── schemas.py                 -- Pydantic v2
    └── deps.py                    -- conteneur de dépendances
```

### §7.2 Flux d'une requête authentifiée / autorisée

```
 Client ─── HTTPS ──▶ Gateway (Envoy)
                        │
                        │ terminate TLS, inject request_id
                        ▼
                     Interceptor (M11)
                        │
                        │ extract Bearer token
                        ├─▶ AuthNService.verify_jwt(token)
                        │     └─▶ JwksCache.get(kid) → key
                        │     └─▶ signature + exp + nbf + iss + aud
                        │     └─▶ principal = Principal(sub, tenant, scopes)
                        │
                        ├─▶ TenantContext.bind(principal.tenant_id)
                        │     └─▶ SQL SET LOCAL efe.tenant_id = ...
                        │
                        ├─▶ AuthZService.decide(
                        │       PolicyRequest(principal, action, resource))
                        │     └─▶ RBAC.check_scopes()
                        │     └─▶ ABAC.check_attributes()
                        │     └─▶ PolicyDecision(allow=true/false, reasons)
                        │
                        ├─▶ AuditService.log(
                        │       kind="authz.decision", principal, action,
                        │       resource, allow, reasons)
                        │
                        └─▶ if deny: raise ScopeMissing → 403
                            else: enter business handler
                                  with SecurityContext injected
```

### §7.3 Flux d'un effacement RGPD (DSAR)

```
 data_subject submits DSAR via self-service portal
       │
       ▼
 ConsentManager.request_erasure(subject_id, tenant_id, purpose)
       │
       ▼
 ErasureJob (via M10 scheduler, kind="dsar_erasure")
       │
       ├─▶ Audit : "dsar.erasure.requested"
       │
       ├─▶ Phase 1 (immediate, <5 min) : mark for erasure
       │     UPDATE pii_subjects SET erasure_requested_at = NOW()
       │     UPDATE inference_events SET subject_erasure = true
       │                                WHERE user_id = subject_id;
       │
       ├─▶ Phase 2 (async, ≤ 30 days) : hard erasure
       │     DELETE FROM inference_events WHERE subject_erasure = true;
       │     DELETE FROM prompts_blob WHERE id IN (SELECT ...);
       │     → remove object-store chunks via KMS key destruction
       │
       └─▶ Phase 3 : produce Erasure Certificate (signed)
                     Audit : "dsar.erasure.completed"
```

---

## §8. Sous-système 11.1 — **Authentification**

### §8.1 Méthodes supportées

| Canal | Méthode | Usage |
|-------|---------|-------|
| Humain → API | OIDC Authorization Code + PKCE | Delegated auth via IdP |
| Humain → CLI | OIDC Device Code | `efe-admin login` |
| Service ↔ Service | mTLS (SPIFFE) + JWT interne | Mesh-wide |
| Partenaire → API | Client JWT signé par AC partenaire | B2B |
| Data subject → Self | Magic link email + JWT court | DSAR portal |

### §8.2 Structure d'un JWT interne

Voir Annexe B pour la forme canonique. Claims obligatoires :

- `iss` — émetteur (l'IdP ou le Token Service interne).
- `aud` — audience (ex. `efe.api.v1`).
- `sub` — identifiant du principal (UUID stable).
- `tenant_id` — claim privé, toujours présent sauf pour `platform_operator`.
- `scopes` — liste serializée stable, un par espace, ex.
  `sched:read job:cancel audit:export`.
- `iat`, `exp`, `nbf` — fenêtre temporelle ≤ 1 h pour humains, ≤ 15 min
  pour services.
- `jti` — identifiant unique pour révocation.
- `alg` — `ES256` (humains) ou `EdDSA` (services).
- `kid` — clé utilisée, vérifiée via le cache JWKS.

### §8.3 JWKS cache

Le cache JWKS est un dict `kid → JWK` avec TTL 5 min + refresh
proactif toutes les 4 min 30 s. À la rotation de l'IdP, l'ancienne clé
reste en cache pendant sa fenêtre de validité (grace period 24 h).

**Pseudo-algorithme** :

```
fn verify_jwt(token):
    header = parse_header(token)
    kid = header.kid
    jwk = jwks_cache.get_or_refresh(kid)
    if jwk is None: raise InvalidKid
    verify_signature(token, jwk.alg, jwk.key)
    verify_exp(token, now)
    verify_aud(token, config.expected_aud)
    verify_iss(token, config.expected_iss)
    return Principal(
        sub=token.sub, tenant_id=token.tenant_id,
        scopes=token.scopes, jti=token.jti, exp=token.exp
    )
```

### §8.4 mTLS avec SPIFFE

- Chaque pod reçoit un cert X.509 avec SAN `spiffe://efe.local/
  <namespace>/<service_account>`.
- Le `AuthNService.mtls_verify(peer_cert)` extrait le SPIFFE ID et le
  transforme en `Principal(type=service_account, ...)`.
- Rotation auto via cert-manager toutes les 24 h ; pas de CRL, on
  s'appuie sur la courte durée.

### §8.5 Sessions opaques

Pour les appels répétés depuis un même client, on expose un token
opaque (random 256 bits) qui résout en `Principal` via un store.
Avantages : révocation instantanée possible (`DELETE` row). Inconvénient :
state server-side.

Durée de vie : `humain ≤ 1 h`, `service ≤ 15 min`. Renouvellement via
`POST /v1/auth/refresh` avec preuve d'authentification préalable.

### §8.6 MFA

M11 ne gère pas la MFA directement ; l'IdP la fait respecter. M11 vérifie
simplement le claim `amr` (Authentication Methods References) et le claim
`acr` (Authentication Context Class Reference) pour exiger une MFA sur
les scopes sensibles :

```
if action == "audit.export" and "mfa" not in token.amr:
    raise MfaRequired
```

---

## §9. Sous-système 11.2 — **Autorisation (RBAC + ABAC + RLS)**

### §9.1 Modèle composite

M11 implémente un PDP composite :

1. **RBAC** — le principal a-t-il les scopes nécessaires ?
2. **ABAC** — les attributs (owner, tenant, time-of-day, ip-range)
   satisfont-ils la politique ?
3. **RLS** — la base de données applique-t-elle le filtre tenant ?

Un deny à n'importe quelle étape → `PolicyDecision(allow=false)`.

### §9.2 PolicyRequest / PolicyDecision

```text
PolicyRequest:
  principal: Principal
  action: str             # e.g. "drift.evaluation.read"
  resource: Resource      # {kind, id, tenant_id, owner_id, labels}
  context: dict[str, Any] # {ip, geo, time, device_fingerprint}

PolicyDecision:
  allow: bool
  reasons: list[str]        # human-readable
  obligations: list[str]    # e.g. "mask_pii", "audit=high"
  eval_ms: float
  cache_key: bytes
```

### §9.3 PDP central

Un unique service `PolicyDecisionPoint` est instancié par pool. Toutes
les vérifications passent par :

```python
decision = await pdp.decide(request)
```

Le code métier **ne connaît pas les scopes** — il passe une `action`
sémantique (`"schedule.create"`, `"drift.evaluation.read"`, ...) et
reçoit un verdict. Les scopes vivent dans la table `rbac_role_scopes`.

### §9.4 Décorateur `@requires`

Sucre syntaxique pour les routes FastAPI :

```python
@requires(action="schedule.create", resource_kind="schedule")
async def create_schedule(body: ScheduleCreateRequest, ctx: SecurityContext):
    ...
```

Le décorateur appelle le PDP avant l'invocation et enrichit le `ctx`
avec les obligations.

### §9.5 RLS Postgres

Les migrations installent sur chaque table multi-tenant :

```sql
ALTER TABLE t ENABLE ROW LEVEL SECURITY;
CREATE POLICY t_tenant_isolation ON t
  USING (tenant_id = current_setting('efe.tenant_id', true));
```

À chaque début de transaction :

```sql
SET LOCAL efe.tenant_id = '<tenant>';
```

La session Postgres est configurée `BEGIN ISOLATION LEVEL READ
COMMITTED` ; `SET LOCAL` est donc transaction-scoped → aucun leak entre
requêtes.

### §9.6 Cache PDP

Les décisions sont cachées par clé
`HMAC(DOMAIN_TAG_PDP_CACHE, canonical(PolicyRequest))`. TTL = 60 s.
Invalidation :

- Au logout / révocation de token (DELETE par `jti`).
- À la modification des rôles ou scopes (flush du tenant).
- Au changement de `tenant_id`.

### §9.7 ABAC — conditions typiques

- **Time-based** : `audit.export` interdit en dehors de 08:00–20:00 UTC
  sauf scope `compliance_officer`.
- **Geo-based** : opérations sur données HIPAA uniquement depuis une IP
  whitelistée.
- **Rate-based** : `dsar.submit` limité à 1/min/sujet.
- **Ownership-based** : `schedule.cancel` uniquement si
  `resource.owner_id = principal.sub` ou scope `sched:admin`.

---

## §10. Sous-système 11.3 — **Multi-tenancy**

### §10.1 Modèle d'isolation

| Niveau | Mécanisme | Granularité |
|--------|-----------|-------------|
| Identité | `tenant_id` immuable sur chaque principal | Token |
| Requête | `TenantContext` Python injecté dans chaque handler | Coroutine |
| SQL | RLS + `SET LOCAL efe.tenant_id` | Transaction |
| Objet | Préfixe S3 `s3://efe-data/{tenant}/...` + KMS per-tenant | Clé |
| NATS | Subjects tenant-scoped `efe.{tenant}.>` | Subject ACL |
| Compute | Anti-affinity K8s par tenant (opt-in) | Pod |
| Réseau | NetworkPolicy par tenant (opt-in, pour compliance tier) | Namespace |

### §10.2 Provisioning

**`TenantProvisioner.onboard(spec)`** — crée :

1. Une ligne `tenants` avec `tenant_id`, `display_name`, `tier`,
   `region`, `created_by`.
2. Un `DEK` propre au tenant, wrappé par le KEK global (envelope
   encryption).
3. Un bucket S3 virtuel (préfixe) et un ACL NATS.
4. Les rôles par défaut (admin, reader, compliance_officer).
5. Un audit `"tenant.onboarded"`.

### §10.3 Off-boarding

**`TenantProvisioner.offboard(tenant_id, mode)`** — trois modes :

- `soft` — désactive les accès, conserve les données 90 jours (pour
  rétraction éventuelle).
- `export` — génère un archive chiffrée, la remet au client, puis
  passe en `hard`.
- `hard` — détruit le DEK → rendu inopérable (effacement
  cryptographique), puis GC des blobs associés.

### §10.4 Quotas

Par tenant :

| Axe | Default | Configurable |
|-----|---------|--------------|
| Requêtes API | 60 rps | Oui |
| Ingestion events | 5 000 evt/s | Oui |
| Storage bytes | 100 GiB | Oui |
| Compute CPU-hours/mois | 1 000 | Oui |
| Jobs enqueued/day | 100 000 | Oui |
| Schedules | 10 000 | Oui |

Dépassement → 429 + audit + alerte au tenant admin.

### §10.5 Cross-tenant leakage testing

Un test CI « tenant poisoning » injecte dans chaque API un attaquant
tenant B qui demande des ressources tenant A. Attendu : 0 fuite.
Exécuté à chaque PR touchant L2 ou L3.

---

## §11. Sous-système 11.4 — **Gestion des secrets & cryptographie**

### §11.1 Hiérarchie de clés

```
                           ┌──────────────┐
                           │ HSM / Cloud  │  (root, never exported)
                           │ KMS          │
                           └──────┬───────┘
                                  │ wraps
                                  ▼
                           ┌──────────────┐
                           │ KEK          │  (tenant-scoped)
                           │ (Vault       │   rotation: 90 j
                           │  Transit)    │
                           └──────┬───────┘
                                  │ wraps / generates
                                  ▼
                           ┌──────────────┐
                           │ DEK          │  (short-lived, per-object)
                           │ AES-256-GCM  │   rotation: per-write
                           └──────────────┘
```

### §11.2 Envelope encryption (pour données au repos)

1. Pour chaque blob à chiffrer : generate random `DEK` (256 bits).
2. `ciphertext = AES-256-GCM(DEK, plaintext, nonce, aad)`.
3. `wrapped_dek = KEK.wrap(DEK)` via Vault.
4. Stocker `{ciphertext, wrapped_dek, nonce, aad, key_version, alg_id}`.
5. `DEK` est zéroisé en mémoire.

Déchiffrement symétrique. Au changement de KEK, on ré-ouvre les DEK
wraps et on les re-wrappe (batch asynchrone, `gc_daemon`).

### §11.3 Signature de manifest (pour intégrité)

Toute baseline / dataset / artefact porte une signature :

- **HMAC-SHA256** avec `DOMAIN_TAG_*` dédié pour les signatures à
  vérification rapide.
- **Ed25519** pour les signatures exportées (publiables).

L'API `Sealer.seal(payload, purpose)` sélectionne l'algorithme selon le
`purpose` et la politique.

### §11.4 Rotation

- **JWT signing keys** : rotation hebdomadaire (IdP gère) ; JWKS
  rafraîchi.
- **KEK** : rotation trimestrielle ; daemon ré-enveloppe tous les DEKs
  en 7 jours max.
- **TLS certs** : 24 h (auto-cert-manager + short-lived).
- **Audit seal secret** : rotation mensuelle — la chaîne intègre un
  `key_version` par ligne.
- **HMAC domain secrets** : rotation trimestrielle via `set_key()` +
  coexistence avec ancien secret pendant 7 j (grace period).

### §11.5 Algorithm IDs

Chaque artefact chiffré / signé porte un `alg_id` explicite :

| ID | Algorithme | Usage |
|----|-----------|-------|
| `AES256-GCM-v1` | AES-256-GCM | Chiffrement au repos |
| `HMAC-SHA256-v1` | HMAC-SHA256 | Sceau rapide |
| `Ed25519-v1` | Ed25519 | Signature publiable |
| `ES256-v1` | ECDSA P-256 + SHA-256 | JWT humains |
| `EdDSA-v1` | Ed25519 | JWT service |

Ajout d'un `v2` : le code route selon `alg_id` ; les anciens restent
lisibles. Dépréciation : warning 6 mois, blocage ensuite.

### §11.6 Zéroisation mémoire

Tous les secrets en mémoire sont contenus dans des objets
`SecureBytes` qui :

- Implémentent `__del__` pour overwrite 3 fois (`00`, `FF`, `random`).
- Empêchent la sérialisation (`__reduce__` raise).
- Sont alloués via `bytearray` (mutable) plutôt que `bytes` (immutable).

---

## §12. Sous-système 11.5 — **Protection des données personnelles (PII / PHI)**

### §12.1 Classification tiers

| Tier | Exemples | Politique par défaut |
|------|----------|----------------------|
| `public` | version de modèle, noms de métriques | Aucune restriction |
| `internal` | task_id, feature_id | Restreint au tenant |
| `pii` | email, téléphone, IP, user_id | Masqué, pseudonymisé, audit |
| `phi` | diagnostic, dossier médical | Chiffré par clé dédiée, audit strict, HIPAA |
| `secret` | clés, mots de passe, tokens | Jamais persisté, zéroisé |

### §12.2 Registre des champs

Une table `pii_field_registry` enregistre chaque colonne classifiée :

| Colonne | Classification | Détection | Transformation | Rétention |
|---------|---------------|-----------|----------------|-----------|
| `inference_events.prompt_text` | pii | regex email/phone | `redact:[PII:<kind>]` | 30 j |
| `inference_events.user_id` | pii | n/a | `pseudonymize:HMAC(DOMAIN_TAG_USER_ID, ...)` | 365 j |
| `inference_events.covariates.ip_address` | pii | regex IP | `hash:SHA-256(salted)` | 30 j |

Tout champ nouveau non enregistré est **rejeté à l'ingestion** (fail-
closed sur la classification).

### §12.3 Détecteurs

- **Regex** — email, phone (+CC), IBAN, SSN, IP v4/v6, carte bancaire
  (Luhn), date-de-naissance FR.
- **Unicode normalisation NFKC** avant matching (anti full-width).
- **Depth-limit** sur récursion dans les dicts : 10 niveaux.
- **ML classifier (optionnel)** — pour les noms propres, activé
  opt-in par tenant.

### §12.4 Transformations

- **Redact** — remplace par `[PII:<kind>]`.
- **Pseudonymize** — HMAC avec domain tag ; réversible *uniquement*
  via une table protégée par un rôle dédié.
- **Hash** — one-way (salé par tenant).
- **Tokenize** — remplace par un UUID opaque ; mapping stocké dans un
  coffre.
- **Minimize** — ne garde que la partie utile (ex. code postal → région).

### §12.5 Politique par tenant

Chaque tenant a une `PiiPolicy` active :

```text
PiiPolicy:
  tenant_id: str
  profile: "default" | "strict" | "hipaa"
  field_overrides: dict[field_name, transform]
  enabled_detectors: set[str]
  retention_days: int
```

Le profil `strict` active des détecteurs supplémentaires (noms propres
heuristiques, adresses) ; `hipaa` force le chiffrement par clé dédiée
et active l'audit `phi_access`.

### §12.6 Invariant : 0 champ `TEXT` sans classification

Un test CI parse toutes les migrations Alembic et refuse l'ajout d'un
`TEXT NOT NULL` sans ligne correspondante dans `pii_field_registry`.
C'est l'invariant I8.

---

## §13. Sous-système 11.6 — **Piste d'audit inviolable**

### §13.1 Modèle `AuditRecord`

```text
AuditRecord (frozen, slots):
  audit_id: str (UUIDv7)
  occurred_at: datetime (UTC, ms)
  tenant_id: str | None      # None for platform-wide
  actor: Actor
  action: str                # stable code : "authz.decision.deny" etc.
  resource: Resource | None
  outcome: "allow"|"deny"|"success"|"failure"
  reasons: list[str]
  severity: "info"|"warning"|"critical"
  payload_digest: bytes(32)  # HMAC of PII-scrubbed payload
  prev_seal: bytes(32)       # chaining field
  seal: bytes(32)            # HMAC(DOMAIN_TAG_AUDIT_CHAIN, prev_seal || canonical(record))
  key_version: int
```

### §13.2 Chaîne de seal

Pour toute ligne *i > 0* :

```
seal(0) = HMAC(K_v, DOMAIN_TAG_AUDIT_CHAIN || "GENESIS" || canonical(record_0))
seal(i) = HMAC(K_v, DOMAIN_TAG_AUDIT_CHAIN || seal(i-1) || canonical(record_i))
```

Modifier un enregistrement rétroactivement casserait la chaîne et
serait détecté par `AuditChainVerifier.verify(range)`.

### §13.3 Write path

1. L'appelant fournit un `AuditDraft` sans seal.
2. `AuditService.append(draft)` :
   - récupère `prev_seal` de la dernière ligne (lock optimiste),
   - calcule `seal` avec la clé active,
   - `INSERT` dans la table `audit_log` (immutable — pas d'UPDATE),
   - émet une métrique `audit_append_total`,
   - publie un event NATS optionnel.

### §13.4 Read path

- Par `actor`, `action`, `resource_id`, plage temporelle.
- Les lectures sont elles-mêmes audités (meta-audit `"audit.read"`).
- Export asymmetric signé (Ed25519) pour remise aux régulateurs.

### §13.5 Verifier daemon

Toutes les 5 min, un daemon échantillonne une plage aléatoire de
l'audit et vérifie :

- `seal(i) == recompute(i)` ;
- `occurred_at` monotone ;
- `key_version` connu dans le registre.

Incohérence → incident critique auto-créé (§16).

### §13.6 Rétention et export

- Conservé 365 jours online (requêtable live).
- Archivé S3 en `WORM` (Object Lock) pendant 7 ans.
- Export SIEM via pull / push (STREAM compatible).
- Découpage par jour pour export.

---

## §14. Sous-système 11.7 — **Consentement et base légale**

### §14.1 Modèle

```text
LegalBasis = Literal[
  "consent", "contract", "legal_obligation",
  "vital_interests", "public_task", "legitimate_interests"
]

ConsentRecord (frozen):
  consent_id: str
  tenant_id: str
  subject_id: str
  purposes: set[str]        # e.g. {"model_training", "drift_analysis"}
  legal_basis: LegalBasis
  consent_text_digest: bytes(32)
  locale: str
  collected_at: datetime
  collected_by: str         # interface / endpoint / form
  expires_at: datetime | None
  withdrawn_at: datetime | None
  seal: bytes(32)
```

### §14.2 Purpose catalog

Chaque « purpose » est déclaré dans une table immuable (versionnée) :

| Purpose | Description | Necessary scopes |
|---------|-------------|------------------|
| `model_training` | Entraîner un modèle sur les interactions | `training:write` |
| `drift_analysis` | Mesurer la dérive | `drift:write` |
| `quality_evaluation` | Évaluer la qualité | `evaluation:write` |
| `rollout_canary` | Tester une nouvelle version | `rollout:write` |
| `fraud_detection` | Détecter des abus | `security:write` |

### §14.3 Binding à l'action

Avant toute opération sur données personnelles, le code métier doit
obtenir un `ConsentAssertion` :

```python
assertion = await consent_manager.require(
    subject_id=user_id,
    purpose="model_training",
    at=clock.now(),
)
# if no valid consent → ConsentMissing raised
```

### §14.4 Pipeline DSAR

Trois opérations normalisées :

| Demande | Pipeline | SLA |
|---------|----------|-----|
| **Access** | `DsarAccess.execute(subject_id)` → archive signée remise par lien court | 7 j |
| **Rectify** | `DsarRectify.execute(subject_id, corrections)` → validation + audit | 7 j |
| **Erase** | `DsarErase.execute(subject_id)` → phases 1-3 décrites en §7.3 | 30 j |

Chaque demande est tracée dans `dsar_requests` avec statut
(`received`, `in_progress`, `completed`, `rejected`). Le rejet exige
une justification (ex. « litige en cours »).

### §14.5 Révocation de consentement

Un sujet peut retirer son consentement à tout moment :

- Le `ConsentRecord.withdrawn_at` est daté.
- Les effets :
  - Future ingestion : refusée.
  - Datasets en cours : flaggés `suspended`.
  - Modèles entraînés : préservés (base légale « intérêt légitime » à
    l'agrégé) ; mais les exemples individuels sont purgés.

---

## §15. Sous-système 11.8 — **Cycle de vie des données**

### §15.1 Catégories et rétention par défaut

| Catégorie | Rétention | Base |
|-----------|-----------|------|
| Événements d'inférence bruts | 30 j | Minimisation |
| Événements normalisés (PII scrubbed) | 365 j | Training / drift |
| Baselines actives | Pendant la durée de vie de la tâche | Observabilité |
| Audit trail | 365 j online + 7 ans WORM | Obligation légale |
| Incidents de sécurité | 7 ans | Obligation légale |
| Consent records | 3 ans après révocation | Preuve |
| Artefacts ML | 2 ans | Rollback / A/B |
| Prompts bruts | 0 j (ne sont persistés que via digest) | Minimisation |

### §15.2 Purge automatique

Le `DataLifecycleDaemon` tourne tous les jours à 02:00 UTC :

```
for cat in categories:
    expired = find_expired(cat, now - cat.retention)
    for record in expired:
        if record.subject_erasure: hard_delete(record)
        elif cat == "prompts_blob": anonymize_then_delete(record)
        else: soft_delete(record)
    audit("lifecycle.purge", category=cat, count=len(expired))
```

### §15.3 Effacement cryptographique

Pour les blobs volumineux (prompts, réponses), on préfère
l'**effacement cryptographique** : destruction du DEK. Les octets
chiffrés deviennent inexploitables, sans nécessiter de rewrite.

### §15.4 Legal hold

Un administrateur `security_officer` peut poser un legal hold sur un
ensemble de données (ex. litige) — la purge automatique les ignore
jusqu'à retrait du hold. Chaque action est auditée.

---

## §16. Sous-système 11.9 — **Réponse à incident**

### §16.1 Détecteurs

Événements générant un incident automatique :

| Détecteur | Seuil |
|-----------|-------|
| `authn.failure_rate` | > 10/min sur un même `sub` → brute-force suspecté |
| `authz.deny_rate` | > 100/min sur un même `principal` → scope probing |
| `pii.export_volume` | > 10 000 rows en 1 h → potentielle exfiltration |
| `audit.chain_break` | 1 occurrence → tamper |
| `key.expired_usage` | 1 occurrence → rotation manquée |
| `tenant_boundary_violation` | 1 occurrence → fuite |

### §16.2 Triage

Chaque incident détecté reçoit :

- `severity ∈ {low, medium, high, critical}`,
- `category ∈ {authn, authz, pii, key, chain, tenancy, network}`,
- un runbook à exécuter (`docs/runbook/<category>.md`).

### §16.3 Playbooks automatisables

Pour certaines catégories, M11 peut exécuter des actions confinées
sans intervention humaine :

- **Brute-force suspect** → lock du compte (révocation du `jti`), alerte
  au tenant admin, exigence d'une MFA au prochain login.
- **Tamper audit** → bloque les écritures audit, escalade au
  security_officer, bascule en read-only du tenant.
- **Tenant boundary violation** → pause automatique des jobs en cours,
  snapshot forensique, escalade.

### §16.4 Notification RGPD

Lorsqu'un incident catégorisé `high` ou `critical` implique des données
personnelles, un brouillon de notification (72 h RGPD) est rédigé
automatiquement :

```
{
  "severity": "high",
  "incident_id": "...",
  "observed_at": "...",
  "affected_tenants": ["t1"],
  "affected_categories": ["pii"],
  "estimated_subjects": 12_345,
  "measures_taken": ["token revocation", "export blocked"],
  "draft_text": "<templated>",
  "signed_by": "security_officer"
}
```

Le brouillon est soumis au DPO pour approbation via un workflow interne.

---

## §17. Sous-système 11.10 — **Agilité cryptographique**

### §17.1 Interface abstraite

Toutes les primitives passent par :

```text
CryptoProvider (Protocol):
    def hmac(self, data: bytes, tag: bytes, alg_id: str) -> bytes
    def sign(self, data: bytes, key_id: str, alg_id: str) -> bytes
    def encrypt(self, plaintext: bytes, key_id: str, alg_id: str) -> Envelope
    ...
```

L'implémentation `DefaultCryptoProvider` dispatche sur `alg_id`.
L'ajout d'un algo = nouvelle constante `alg_id` + nouvelle branche +
test contract.

### §17.2 Dépréciation ordonnée

1. Un `alg_id` est annoncé déprécié avec une date cible (6 mois).
2. Les nouvelles écritures utilisent le nouvel algo.
3. Les lectures acceptent les deux pendant la période de grâce.
4. À la date cible, les lectures de l'ancien lèvent `AlgorithmDeprecated`.
5. Les artefacts restants sont re-chiffrés par le daemon.

### §17.3 Résistance quantique

Le document note (sans engagement de date) que le passage à
Kyber / Dilithium est prévu lorsque NIST finalisera les standards. Les
`alg_id` réservent `"Kyber-v1"`, `"Dilithium-v1"`.

---

## §18. Sous-système 11.11 — **Mappings de conformité**

Chaque contrôle M11 est cartographié aux exigences réglementaires
connues. Voir Annexe G pour la matrice complète.

### §18.1 RGPD (GDPR)

| Article | Contrôle M11 |
|---------|-------------|
| Art. 5 — minimisation, limitation de finalité | §12, §14 |
| Art. 6 — base légale | §14 |
| Art. 15 — droit d'accès | §14.4 (DsarAccess) |
| Art. 16 — rectification | §14.4 (DsarRectify) |
| Art. 17 — effacement | §14.4 (DsarErase) + §15.3 |
| Art. 25 — privacy by design | §3 (principes) |
| Art. 30 — registre des traitements | `/v1/compliance/records` |
| Art. 32 — sécurité du traitement | §11, §13 |
| Art. 33 — notification | §16.4 |
| Art. 35 — DPIA | §5 (threat model) |

### §18.2 AI Act (UE)

- **Classification risque** : système à risque limité (classifier) →
  obligations de transparence, journalisation.
- **Exigences article 9 (risk management)** : §5, §16.
- **Exigences article 12 (record-keeping)** : §13 (audit JSONL).
- **Exigences article 14 (human oversight)** : PDP obligations
  « human_confirmation » sur les décisions critiques.
- **Exigences article 15 (accuracy, robustness)** : M09 dérive +
  M10 retraining.

### §18.3 SOC 2

Mapping aux Common Criteria :

| CC | M11 |
|----|-----|
| CC6.1 Logical access | §8, §9 |
| CC6.2 User provisioning | §8.4, §10.2 |
| CC6.3 Role changes | §13 (audit) |
| CC6.6 Encryption | §11 |
| CC6.7 Key management | §11, §17 |
| CC6.8 Malware | Scanner supply chain (externe) |
| CC7.1 Monitoring | §13, §16 |
| CC7.2 Anomaly detection | §16.1 |

### §18.4 HIPAA (opt-in BAA)

Pour les tenants santé :

- Chiffrement **renforcé** (AES-256-GCM + clé par patient).
- Audit strict de tout accès PHI (`phi_access` ⊆ audit).
- BAA signé ; rôles `phi_handler` dédiés.
- Effacement en ≤ 72 h après demande.

---

## §19. Budget de performance

| Axe | Cible | Conditions |
|-----|-------|------------|
| Latence AuthN (JWT verify + JWKS cache hit) | p99 < 1 ms | cache chaud |
| Latence AuthZ (PDP decide + cache hit) | p99 < 2 ms | cache chaud |
| Latence RLS (SET LOCAL + transaction) | p99 < 3 ms | pool saturé 30 % |
| Latence audit.append | p99 < 5 ms | SQL local |
| Latence audit chain verify (batch 10k) | < 10 s | single thread |
| Latence rotation KEK (full) | < 7 j | batch asynchrone |
| Latence DSAR access | < 24 h | volume moyen |
| Latence DSAR erase (phase 1) | < 5 min | immediate mark |
| Latence DSAR erase (phase 2) | < 30 j | SLA légal |

Les axes sont benchmarkés via `pytest-benchmark` + charge Locust.

---

## §20. Taxonomie d'erreurs

```
SecurityError (root)
├── AuthNError
│   ├── MissingToken
│   ├── InvalidSignature
│   ├── ExpiredToken
│   ├── InvalidAudience
│   ├── InvalidIssuer
│   ├── InvalidKid
│   ├── MfaRequired
│   └── SessionRevoked
├── AuthZError
│   ├── ScopeMissing
│   ├── OwnershipViolation
│   ├── AbacConditionFailed
│   ├── TenantMismatch
│   └── ActionForbidden
├── TenancyError
│   ├── TenantNotFound
│   ├── TenantDisabled
│   └── QuotaExceeded
├── CryptoError
│   ├── KeyNotFound
│   ├── KeyExpired
│   ├── SealInvalid
│   ├── AlgorithmDeprecated
│   └── RotationFailed
├── PiiError
│   ├── FieldNotClassified
│   ├── TransformFailed
│   └── PolicyViolation
├── AuditError
│   ├── ChainBroken
│   ├── VerifierFailed
│   └── ExportDenied
├── ConsentError
│   ├── ConsentMissing
│   ├── ConsentExpired
│   ├── ConsentWithdrawn
│   └── PurposeNotDeclared
├── DsarError
│   ├── SubjectNotFound
│   ├── RequestAlreadyProcessed
│   └── LegalHoldInForce
├── LifecycleError
│   ├── ErasureBlocked
│   └── RetentionConflict
└── IncidentError
    ├── PlaybookFailed
    └── NotificationBlocked
```

Mapping RFC 7807 identique à M10 (§17.2).

---

## §21. Stratégie de tests

### §21.1 Matrice de tests

| Couche | Type | Outil |
|--------|------|-------|
| `jwt_verifier` | Unit + fuzz signature | pytest + cryptography |
| `pdp` | Table de vérité | pytest parametrize |
| `rls_bridge` | Integration Postgres | testcontainers |
| `envelope` | Round-trip | pytest + property |
| `classifier` | Golden dataset PII | pytest + fixtures |
| `audit.chain` | Property (tamper detected) | Hypothesis |
| `consent.dsar` | Scenario | pytest-asyncio |
| `lifecycle.erasure` | End-to-end 30 days simulated | FakeClock |
| `incident.detector` | Simulation | scenario tests |
| `crypto.agility` | Two-algo coexistence | contract tests |
| `tenancy poisoning` | Cross-tenant leak attempts | auto-generated fuzz |

### §21.2 Tests de robustesse

- **Fuzz JWT** : 10 000 tokens forgés → 0 faux positif accept.
- **Fuzz PII** : full-width, homoglyphes, zero-width joiners → 100 %
  détection des variantes communes.
- **Tamper audit** : flip d'un bit au milieu de la chaîne → détection
  en ≤ 5 min.
- **Tenant poisoning** : attaquant tenant B invoque toutes les API avec
  ressources tenant A → 0 leak.
- **Erasure completeness** : fuzzed datasets → après DSAR erase, 0 ligne.

### §21.3 Gates CI

- Couverture ≥ 95 % sur `authn`, `authz`, `pii`, `audit`, `consent`.
- mypy --strict zero erreur.
- ruff ALL sans dérogation.
- `bandit` scan sécurité : 0 finding critique.
- `pip-audit` : 0 CVE non mitigée.
- SBOM à jour à chaque PR.

---

## §22. Plan de sprints

| Sprint | Thème | Livrables | DoD |
|--------|-------|-----------|-----|
| **S1** | Fondations (SecurityContext, errors, crypto primitives) | `context.py`, `errors.py`, `crypto/`, tests | 100 % coverage |
| **S2** | AuthN (JWT, JWKS, mTLS) | `authn/*` | fuzz JWT OK |
| **S3** | AuthZ (PDP, RBAC, ABAC) | `authz/*` | table de vérité exhaustive |
| **S4** | Multi-tenancy (context, quotas, provisioner) | `tenancy/*` | tenant-poisoning green |
| **S5** | Secrets (KMS client, envelope, rotation) | `secrets/*` | rotation 7j OK |
| **S6** | PII / PHI (classifier, masker, policy) | `pii/*` | golden PII OK |
| **S7** | Audit trail (chain, store, verifier) | `audit/*` | tamper test green |
| **S8** | Consent + DSAR | `consent/*` | DSAR 7j OK |
| **S9** | Lifecycle + erasure | `lifecycle/*` | erasure 30j simulated |
| **S10** | Incident response | `incident/*` | 4 playbooks dry-run |
| **S11** | Compliance mappings | `compliance/*` + evidence | audit export OK |
| **S12** | Pen-test externe + release | rapports, release v1.0 | 0 high / critical finding |

---

## §23. Critères Go / No-Go spécifiques à M11

### §23.1 Go si

- Les 15 invariants I1..I15 sont testés et verts.
- Pen-test externe (black + white box) : 0 critique, ≤ 2 moyens avec
  plan de correction ≤ 30 j.
- DPIA validée par le DPO.
- Runbook incident exécuté en staging par un opérateur (pas l'auteur).
- Export audit trail acceptable par un auditeur SOC 2 fictif.
- Au moins un tenant « HIPAA-opt-in » fonctionne en staging sans
  dérogation.

### §23.2 No-Go si

- Un test de tenant-poisoning produit une fuite.
- Un secret se retrouve en log (scan gitleaks ≠ 0).
- Le Verifier détecte une rupture de chaîne sur 1 heure de trafic.
- Un DSAR échoue à supprimer une ligne attendue.
- La MFA n'est pas exigée sur `audit.export` et `tenant.offboard`.

---

## Annexe A — Domain-separation tags HMAC étendus

Ces tags **complètent** la table unifiée maintenue conjointement avec
M10 (cf. M10 Annexe D). Les tags M11 sont :

| Constante | Valeur | Usage |
|-----------|--------|-------|
| `DOMAIN_TAG_USER_ID` | `b"efe-sec/user-id/v1\x00"` | Pseudonymisation d'identifiants utilisateurs |
| `DOMAIN_TAG_AUDIT_CHAIN` | `b"efe-sec/audit-chain/v1\x00"` | Scellé de la chaîne d'audit |
| `DOMAIN_TAG_PDP_CACHE` | `b"efe-sec/pdp-cache/v1\x00"` | Clé de cache PDP |
| `DOMAIN_TAG_CONSENT_SEAL` | `b"efe-sec/consent-seal/v1\x00"` | Signature des ConsentRecord |
| `DOMAIN_TAG_TENANT_DEK` | `b"efe-sec/tenant-dek/v1\x00"` | Dérivation des DEK tenant |
| `DOMAIN_TAG_SESSION_OPAQUE` | `b"efe-sec/session/v1\x00"` | Hash des tokens de session |
| `DOMAIN_TAG_PII_FIELD_HASH` | `b"efe-sec/pii-hash/v1\x00"` | Hash salé des valeurs PII |
| `DOMAIN_TAG_DSAR_RECEIPT` | `b"efe-sec/dsar-receipt/v1\x00"` | Sceau du reçu DSAR |
| `DOMAIN_TAG_INCIDENT_SEAL` | `b"efe-sec/incident-seal/v1\x00"` | Scellé d'un incident |
| `DOMAIN_TAG_KEY_ROTATION_V1` | `b"efe-sec/key-rotation/v1\x00"` | Journal de rotation |

Règle d'or (identique aux autres modules) : toute nouvelle utilisation
HMAC crée son propre tag. Revue PR rejette toute collision.

---

## Annexe B — Structure canonique du JWT interne

```
Header:
{
  "alg": "ES256",            // or EdDSA
  "kid": "<key-id>",
  "typ": "JWT"
}

Payload:
{
  "iss": "https://idp.efe.local",
  "sub": "usr_01HX...",      // UUIDv7
  "aud": "efe.api.v1",
  "iat": 1745190000,
  "nbf": 1745190000,
  "exp": 1745193600,         // 1h
  "jti": "jti_01HY...",
  "tenant_id": "t_01HA...",
  "task_id": "task_01HB..." | null,
  "scopes": "sched:read job:cancel audit:export",
  "amr": ["pwd","mfa"],
  "acr": "urn:efe:acr:high",
  "groups": ["operators","analysts"],
  "session_id": "sess_01HZ..."
}
```

---

## Annexe C — Matrice RBAC — rôles × scopes

| Scope | reader | operator | admin | compliance_officer | security_officer |
|-------|:------:|:--------:|:-----:|:------------------:|:----------------:|
| `drift:read` | ✔ | ✔ | ✔ | ✔ | ✔ |
| `drift:write` |   | ✔ | ✔ |   |   |
| `baseline:retire` |   |   | ✔ |   | ✔ |
| `sched:read` | ✔ | ✔ | ✔ | ✔ | ✔ |
| `sched:write` |   | ✔ | ✔ |   |   |
| `sched:admin` |   |   | ✔ |   |   |
| `job:read` | ✔ | ✔ | ✔ | ✔ | ✔ |
| `job:cancel` |   | ✔ | ✔ |   | ✔ |
| `rule:write` |   | ✔ | ✔ |   |   |
| `policy:write` |   |   | ✔ |   | ✔ |
| `audit:read` |   |   | ✔ | ✔ | ✔ |
| `audit:export` |   |   |   | ✔ | ✔ |
| `consent:write` |   |   |   | ✔ |   |
| `dsar:process` |   |   |   | ✔ |   |
| `tenant:onboard` |   |   |   |   | ✔ |
| `tenant:offboard` |   |   |   |   | ✔ |
| `key:rotate` |   |   |   |   | ✔ |
| `break_glass:activate` |   |   |   |   | ✔ (MFA) |

---

## Annexe D — Classification PII/PHI — niveaux et règles

| Niveau | Détection | Transformation par défaut | Rétention | Encryption |
|--------|-----------|--------------------------|-----------|------------|
| public | n/a | none | per-usage | TLS |
| internal | allowlist par tenant | none | per-tenant | TLS |
| pii | regex + NFKC + denylist | redact/pseudonymize/hash | 30–365 j | AES-GCM + DEK tenant |
| phi | regex ICD + allowlist PHI | encrypt with per-subject key | 365 j + legal hold | AES-GCM + DEK subject |
| secret | entropy ≥ 5 bit/char | never persist | 0 | in-memory only |

---

## Annexe E — DDL Postgres — tables M11

```sql
-- Tenants
CREATE TABLE tenants (
    tenant_id         TEXT PRIMARY KEY,
    display_name      TEXT NOT NULL,
    tier              TEXT NOT NULL CHECK (tier IN ('standard','hipaa','eu_sovereign')),
    region            TEXT NOT NULL,
    onboarded_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    offboarded_at     TIMESTAMPTZ,
    dek_wrapped       BYTEA NOT NULL,
    dek_key_version   INTEGER NOT NULL,
    labels            JSONB NOT NULL DEFAULT '{}'::jsonb
);

-- RBAC
CREATE TABLE rbac_roles (
    role_id           TEXT PRIMARY KEY,
    display_name      TEXT NOT NULL,
    description       TEXT
);

CREATE TABLE rbac_role_scopes (
    role_id           TEXT NOT NULL REFERENCES rbac_roles(role_id),
    scope             TEXT NOT NULL,
    PRIMARY KEY (role_id, scope)
);

CREATE TABLE rbac_principal_roles (
    tenant_id         TEXT NOT NULL REFERENCES tenants(tenant_id),
    principal_id      TEXT NOT NULL,
    role_id           TEXT NOT NULL REFERENCES rbac_roles(role_id),
    granted_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    granted_by        TEXT NOT NULL,
    revoked_at        TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, principal_id, role_id)
);

-- PII registry
CREATE TABLE pii_field_registry (
    field_id          UUID PRIMARY KEY,
    table_name        TEXT NOT NULL,
    column_name       TEXT NOT NULL,
    classification    TEXT NOT NULL CHECK (
        classification IN ('public','internal','pii','phi','secret')
    ),
    detector          TEXT NOT NULL,
    transform         TEXT NOT NULL,
    retention_days    INTEGER NOT NULL,
    notes             TEXT,
    UNIQUE (table_name, column_name)
);

-- Consent
CREATE TABLE consent_records (
    consent_id        UUID PRIMARY KEY,
    tenant_id         TEXT NOT NULL REFERENCES tenants(tenant_id),
    subject_id        TEXT NOT NULL,
    purposes          TEXT[] NOT NULL,
    legal_basis       TEXT NOT NULL CHECK (
        legal_basis IN ('consent','contract','legal_obligation',
                        'vital_interests','public_task','legitimate_interests')
    ),
    consent_text_digest BYTEA NOT NULL,
    locale            TEXT NOT NULL,
    collected_at      TIMESTAMPTZ NOT NULL,
    collected_by      TEXT NOT NULL,
    expires_at        TIMESTAMPTZ,
    withdrawn_at      TIMESTAMPTZ,
    seal              BYTEA NOT NULL
);
CREATE INDEX ON consent_records (tenant_id, subject_id) WHERE withdrawn_at IS NULL;

-- DSAR
CREATE TABLE dsar_requests (
    request_id        UUID PRIMARY KEY,
    tenant_id         TEXT NOT NULL REFERENCES tenants(tenant_id),
    subject_id        TEXT NOT NULL,
    kind              TEXT NOT NULL CHECK (kind IN ('access','rectify','erase','portability')),
    state             TEXT NOT NULL CHECK (state IN ('received','in_progress','completed','rejected')),
    submitted_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at      TIMESTAMPTZ,
    receipt_seal      BYTEA,
    notes             TEXT
);

-- Audit
CREATE TABLE audit_log (
    audit_id          UUID PRIMARY KEY,
    occurred_at       TIMESTAMPTZ NOT NULL,
    tenant_id         TEXT,
    actor_kind        TEXT NOT NULL,
    actor_id          TEXT NOT NULL,
    action            TEXT NOT NULL,
    resource_kind     TEXT,
    resource_id       TEXT,
    outcome           TEXT NOT NULL,
    reasons           TEXT[] NOT NULL DEFAULT '{}',
    severity          TEXT NOT NULL,
    payload_digest    BYTEA NOT NULL,
    prev_seal         BYTEA,
    seal              BYTEA NOT NULL,
    key_version       INTEGER NOT NULL
);
CREATE INDEX ON audit_log (tenant_id, occurred_at);
CREATE INDEX ON audit_log (actor_id, occurred_at);
-- RLS : deny UPDATE/DELETE, allow INSERT only.
ALTER TABLE audit_log ENABLE ROW LEVEL SECURITY;
CREATE POLICY audit_log_insert ON audit_log FOR INSERT
  WITH CHECK (true);
-- UPDATE / DELETE denied by default (no policy).

-- Incidents
CREATE TABLE security_incidents (
    incident_id       UUID PRIMARY KEY,
    tenant_id         TEXT,
    detected_at       TIMESTAMPTZ NOT NULL,
    severity          TEXT NOT NULL,
    category          TEXT NOT NULL,
    summary           TEXT NOT NULL,
    payload           JSONB NOT NULL,
    state             TEXT NOT NULL,
    notified_at       TIMESTAMPTZ,
    resolved_at       TIMESTAMPTZ,
    seal              BYTEA NOT NULL
);

-- Keys (registry, not the keys themselves)
CREATE TABLE key_registry (
    key_id            TEXT PRIMARY KEY,
    alg_id            TEXT NOT NULL,
    purpose           TEXT NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL,
    rotated_from      TEXT REFERENCES key_registry(key_id),
    retired_at        TIMESTAMPTZ,
    state             TEXT NOT NULL CHECK (state IN ('active','retiring','retired')),
    metadata          JSONB NOT NULL DEFAULT '{}'::jsonb
);
```

Toutes les tables multi-tenant appliquent RLS
`USING (tenant_id = current_setting('efe.tenant_id', true))`.

---

## Annexe F — Matrice de risques M11

| Id | Risque | Probabilité | Impact | Mitigation |
|----|--------|:-----------:|:------:|------------|
| R1 | JWT forgé passe la vérif | Très faible | Critique | Rotation kid + cache invalidation + PDP |
| R2 | Fuite cross-tenant via SQL brut | Faible | Critique | RLS + tests poisoning + lint SQL |
| R3 | Exfiltration de PII via export | Moyenne | Haute | Audit export + quota + MFA |
| R4 | Compromission du KEK | Très faible | Critique | HSM + rotation + journalisation |
| R5 | Tamper de l'audit trail | Faible | Haute | Chaîne + verifier + WORM |
| R6 | DSAR oublie une table | Moyenne | Haute | Registre central des tables PII + audit |
| R7 | Rotation de clé échoue | Faible | Moyenne | Retry + alerte + grace period |
| R8 | Incident non notifié dans 72 h | Faible | Haute | Détecteurs + brouillon auto + SLA |
| R9 | Secret en log | Moyenne | Haute | Scrubber + bandit + revue PR |
| R10 | Break-glass abusé | Très faible | Haute | 4-eyes + audit + durée ≤ 1 h |

---

## Annexe G — Matrice de conformité

| Réglementation | Article / Contrôle | M11 |
|----------------|--------------------|-----|
| **RGPD** | Art 5 | §12, §14, §15 |
| | Art 6 | §14 |
| | Art 15 | §14.4 |
| | Art 17 | §14.4, §15.3 |
| | Art 25 | §3, §12 |
| | Art 32 | §11, §13 |
| | Art 33 | §16.4 |
| | Art 35 DPIA | §5 |
| **AI Act** | Art 9 RM | §5, §16 |
| | Art 12 logs | §13 |
| | Art 14 oversight | §9.4 obligations |
| **SOC 2** | CC6.1 | §8, §9 |
| | CC6.6 | §11 |
| | CC7.1 | §13, §16 |
| **HIPAA** | §164.308 | §8, §9, §10 (hipaa tier) |
| | §164.312 | §11, §13 |
| | §164.314 | BAA |
| **ISO 27001** | A.9 | §8, §9 |
| | A.10 | §11 |
| | A.12 | §13 |
| | A.18 | §18 |

---

## Annexe H — Glossaire sécurité

| Terme | Définition |
|-------|------------|
| **AC** | Autorité de Certification émettant des certs X.509 |
| **ABAC** | Attribute-Based Access Control |
| **Audit chain** | Séquence scellée d'enregistrements liés par hash |
| **BAA** | Business Associate Agreement (HIPAA) |
| **Break-glass** | Accès d'urgence temporaire, journalisé |
| **CRL** | Certificate Revocation List |
| **DEK** | Data Encryption Key — éphémère, par objet |
| **DPIA** | Data Protection Impact Assessment |
| **DSAR** | Data Subject Access Request (RGPD) |
| **Envelope encryption** | DEK chiffré par KEK |
| **HSM** | Hardware Security Module |
| **JWK / JWKS** | JSON Web Key (Set) |
| **KEK** | Key Encryption Key — long-vécue |
| **Least privilege** | Minimum de droits nécessaires |
| **mTLS** | Mutual TLS |
| **PDP** | Policy Decision Point |
| **PEP** | Policy Enforcement Point |
| **PFS** | Perfect Forward Secrecy |
| **PII / PHI** | Personally / Protected Health Information |
| **Purpose limitation** | Un traitement sert une finalité déclarée |
| **RBAC** | Role-Based Access Control |
| **RLS** | Row Level Security |
| **SPIFFE** | Secure Production Identity Framework for Everyone |
| **SSE-KMS** | Server-Side Encryption with KMS (S3) |
| **Tamper-evident** | Toute modification est détectable |
| **Tenant** | Client isolé logiquement dans la plateforme |
| **Trust plane** | Ensemble des contrôles transverses (M11 ici) |
| **WORM** | Write-Once-Read-Many |
| **Zero trust** | Aucun réseau / principal de confiance par défaut |

---

*Fin du document. Relecture trimestrielle obligatoire par l'équipe
sécurité + DPO. Mise à jour après chaque pen-test, chaque incident
critique, chaque évolution réglementaire, ou tout ajout de contrôle.
Toute modification d'invariant ou de tag HMAC requiert une ADR signée.*
