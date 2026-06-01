# Sprint D14-bis — Architecture détaillée : HMAC-chained tamper-evident audit log

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit D14 (`PostgresCatalogRegistry`) + son audit
> interne (`docs/audits/d14_audit_report.md` — 7 bugs trouvés, 6
> fixés, dont C1 cohérence DDL/validation).
> **Pré-requis** : avoir lu
> [`d14_postgres_catalog_backend_architecture.md`](d14_postgres_catalog_backend_architecture.md)
> (2 391 lignes — Postgres backend), `d14_audit_report.md` (audit
> findings), `d12_catalog_admin_versioning_architecture.md`
> (`AuditEmitter` D12), `d11_per_tenant_isolation_architecture.md`
> (per-tenant invariant), `M12_ARCHITECTURE.md` § 9 (HS256 secret
> rotation pattern qu'on réutilise verbatim).
>
> **Mission** : transformer le log d'audit catalogue d'un **signal
> informatif** (`tracing::info!(target: "audit", …)`) en une
> **preuve cryptographique d'intégrité**. Chaque audit event est
> chaîné HMAC-SHA-256 au précédent ; toute modification, suppression
> ou réordonnancement est détectable par un verify externe avec la
> clé symétrique.
>
> **Pain point que D14-bis résout** :
>
> 1. **Tamper-evidence** : aujourd'hui, un attaquant avec accès au
>    log file (Loki, Splunk, file appender) peut modifier ou
>    supprimer des entries — l'opérateur ne peut pas le détecter.
>    Pour SOC 2, HIPAA, GDPR Article 32(1), cela bloque les audits
>    réglementaires.
> 2. **Reorder protection** : sans chaîne, l'ordre des events n'est
>    pas authentifié. Un adversaire peut réordonner pour cacher une
>    séquence (e.g. delete → register → re-add).
> 3. **Append-only enforcement** : la table SQL `audit_events`
>    livrée en D14-bis a un trigger qui interdit UPDATE/DELETE,
>    couplé à la chaîne HMAC pour le row-level intégrité.
>
> **Note importante sur le périmètre** : D14-bis n'est PAS de la
> cryptographie asymétrique (Ed25519 / RSA-PSS). C'est un MAC
> symétrique. Le verify peut être fait par toute partie ayant accès
> à la clé HMAC ; ça suffit pour prouver l'intégrité au sein de
> l'organisation. Pour une preuve **vérifiable par tiers** (e.g.
> régulateur sans accès clé), ajouter Ed25519 par-dessus est laissé
> à un sprint ultérieur (`D14-ter`, hors scope D14-bis).

---

## Table des matières

1. [Contexte & mission D14-bis](#1-contexte--mission-d14-bis)
2. [État des lieux post-D14 — pré-conditions remplies](#2-état-des-lieux-post-d14--pré-conditions-remplies)
3. [Threat model — qu'est-ce qu'on protège](#3-threat-model--quest-ce-quon-protège)
4. [Décision majeure #1 — HMAC chained vs Merkle tree vs Ed25519](#4-décision-majeure-1--hmac-chained-vs-merkle-tree-vs-ed25519)
5. [Décision majeure #2 — algorithme HMAC-SHA-256 vs BLAKE3-keyed vs autre](#5-décision-majeure-2--algorithme-hmac-sha-256-vs-blake3-keyed-vs-autre)
6. [Décision majeure #3 — storage : `audit_events` table SQL vs JSONL append vs in-line dans `catalogs`](#6-décision-majeure-3--storage--audit_events-table-sql-vs-jsonl-append-vs-in-line-dans-catalogs)
7. [Décision majeure #4 — granularité de la chaîne : per-tenant vs global](#7-décision-majeure-4--granularité-de-la-chaîne--per-tenant-vs-global)
8. [Décision majeure #5 — canonical serialisation pour le HMAC input](#8-décision-majeure-5--canonical-serialisation-pour-le-hmac-input)
9. [Format du record signé — schema détaillé](#9-format-du-record-signé--schema-détaillé)
10. [Schéma SQL `audit_events` — DDL complet](#10-schéma-sql-audit_events--ddl-complet)
11. [`HmacChainedAuditSink` — Rust impl + concurrency](#11-hmacchainedauditsink--rust-impl--concurrency)
12. [Genesis event — bootstrap de la chaîne](#12-genesis-event--bootstrap-de-la-chaîne)
13. [Key management — `EFE_AUDIT_HMAC_SECRETS_FILE`](#13-key-management--efe_audit_hmac_secrets_file)
14. [Key rotation — multi-key verify, single-key sign](#14-key-rotation--multi-key-verify-single-key-sign)
15. [Verify — library + CLI `efe-verify-audit`](#15-verify--library--cli-efe-verify-audit)
16. [Performance budget vs D14 baseline](#16-performance-budget-vs-d14-baseline)
17. [Observability D13/D14 extensions](#17-observability-d13d14-extensions)
18. [Helm wiring — secret + env vars](#18-helm-wiring--secret--env-vars)
19. [Runtime wire-up — `Server::build` augmenté](#19-runtime-wire-up--serverbuild-augmenté)
20. [Tests strategy — ~30 nouveaux tests](#20-tests-strategy--30-nouveaux-tests)
21. [Threat model détaillé — 8 vecteurs](#21-threat-model-détaillé--8-vecteurs)
22. [Decisions log — récap des décisions clés](#22-decisions-log--récap-des-décisions-clés)
23. [Risques + mitigations](#23-risques--mitigations)
24. [Plan wallclock — 4 j](#24-plan-wallclock--4-j)
25. [SLO ajoutés en D14-bis](#25-slo-ajoutés-en-d14-bis)
26. [Anti-régression D14-bis](#26-anti-régression-d14-bis)
27. [Hand-off vers D15 + D14-ter](#27-hand-off-vers-d15--d14-ter)
28. [Annexes](#28-annexes)

---

## 1. Contexte & mission D14-bis

### 1.1 État post-D14

Au gel de v0.1.14 (post-D14-audit) :

* **`AuditEmitter` D12** émet 3 types d'events
  (register / evolve / delete) avec un schema documenté
  (`AuditEvent` struct, 7 champs typés).
* **`AuditSink` trait** déjà présent
  (`crates/efe-server/src/catalog/audit.rs:30`) — un seul impl actif
  (`TracingAuditSink` qui pousse vers
  `tracing::info!(target: "audit", …)`).
* **Le log d'audit transite via Loki / fluentbit / vector** vers un
  backend de log centralisé (Splunk, Loki, Elastic, S3 cold storage).
* **Aucune signature** : le contenu est en clair, modifiable par
  quiconque a write access au log file.
* **Aucune chaîne** : les events sont indépendants, pas de
  `prev_hmac` ou équivalent ; un attaquant peut réordonner sans
  laisser de trace.
* **`PostgresCatalogRegistry`** émet l'audit via
  `self.audit.emit_register(&key, &metadata)` — pas de table SQL
  dédiée à l'audit (les events vont dans le log stream).

### 1.2 Le problème opérationnel

L'audit log actuel est un **signal de monitoring**, pas une **preuve
juridique**. Pour les déploiements qui ont besoin de la preuve :

| Use case | Exigence | Aujourd'hui (D14) | D14-bis |
|----------|----------|-------------------|---------|
| **SOC 2 Type II** "Logical and Physical Access Controls" | Audit trail tamper-evident | ❌ | ✅ |
| **HIPAA §164.312(b)** Audit Controls | "Implement hardware, software, and/or procedural mechanisms that record and examine activity" — interprété comme tamper-evident | ❌ partiel | ✅ |
| **GDPR Article 32(1)(b)** "ability to ensure the ongoing confidentiality, integrity..." | Integrity sur l'audit trail | ❌ | ✅ |
| **PCI-DSS 10.5.5** | "Use file-integrity monitoring or change-detection software on logs" | ❌ partiel (FIM externe possible mais pas fait) | ✅ par construction |
| **NIST 800-53 AU-9** | Protection of Audit Information — "The information system protects audit information and audit tools from unauthorized access, modification, and deletion." | ❌ | ✅ |

Concrètement, sans D14-bis :

1. **Adversaire interne** (DBA, ops, root sur la VM) : peut éditer
   le log Loki ou la rotation file pour cacher une action. Aucune
   trace.
2. **Adversaire externe** (RCE compromise) : pareil, avec en plus
   l'opportunité de modifier les events live avant qu'ils n'arrivent
   à Loki.
3. **Bug dans la pipeline log** : si fluentbit drop des messages
   silencieusement, l'opérateur ne le sait pas. La chaîne HMAC le
   détecte (gap → verify fail).
4. **Régulateur** : demande "prouvez-moi que cet audit log n'a pas
   été modifié entre tel timestamp et tel timestamp". Aujourd'hui,
   réponse : "trust us" (insuffisant). Demain (D14-bis) : "voici la
   chaîne HMAC + la clé verify ; ré-exécutez le verify, voici la
   commande exacte".

### 1.3 Mission D14-bis (énoncé chirurgical)

D14-bis livre **trois choses** :

1. **`HmacChainedAuditSink`** : nouveau impl du trait `AuditSink`
   D12 qui :
   - Maintient un **état chaîné** (le HMAC de l'event précédent).
   - Calcule le HMAC du nouvel event via la clé HMAC active.
   - Insère le record dans la table SQL `audit_events` (D14-bis
     migration `0003_audit_events.sql`).
   - Conserve le emit `tracing::info!(target: "audit", ...)` pour
     compatibilité avec la pipeline Loki existante (le sink
     délègue au `TracingAuditSink` après avoir signé — chained
     emission).
2. **`audit_events` table SQL** : append-only, INSERT-only via un
   trigger PostgreSQL qui interdit UPDATE/DELETE. Même row contient
   le record canonique JSON + le HMAC + le `prev_hmac` + le
   `key_id` (pour key rotation).
3. **Verify utility** : library + CLI `efe-verify-audit` qui :
   - Lit la table `audit_events` (ou un dump JSONL).
   - Replay la chaîne HMAC depuis genesis.
   - Reporte les positions de tampering éventuelles.
   - Supporte multi-key verify (rotation in flight).

### 1.4 Hors-périmètre

Pour éviter l'embonpoint :

| Hors scope D14-bis | Justification | Sprint cible |
|--------------------|---------------|--------------|
| Cryptographie asymétrique (Ed25519, RSA-PSS) pour preuve verifiable par tiers sans clé | HMAC suffit pour intégrité interne ; verifiable-by-third-party = sprint dédié | **D14-ter** |
| Logging des **lectures** (`Get`, `List`) | D12 audit ne couvre que les mutations ; cohérent avec NIST 800-53 AU-2 baseline (mutation-only) | future |
| Encryption at rest du payload audit | Postgres standard encryption (RDS, pgcrypto) ; out-of-scope EFE | external |
| Sealing storage (write-once filesystem, AWS Object Lock) | Responsabilité ops | external |
| Real-time anomaly detection (ML sur les patterns audit) | D17+, hors scope sprint MVP | **D17+** |
| Audit log replication multi-region | D17+ | **D17+** |
| Compaction / archival | DT-D14.2 backlog ; D15 | **D15** |
| Audit des actions Postgres elles-mêmes (DBA queries) | `pgaudit` extension externe ; out-of-scope EFE | external |

---

## 2. État des lieux post-D14 — pré-conditions remplies

### 2.1 Invariants stables (NE PAS modifier)

D14-bis doit honorer chaque invariant ci-dessous sans exception :

| Invariant | Source | Impact si violé |
|-----------|--------|-----------------|
| Trait `AuditSink` D12 ne change pas (signature `fn emit(&self, event: &AuditEvent)`) | D12 + tests | TracingAuditSink + MemoryAuditSink + handler emit-sites cassent |
| `AuditEvent` struct ne change pas (sept champs typés) | D12 contract test | Rétro-compat des consommateurs Loki/Splunk |
| `AuditEmitter::emit_*` sont **synchrones** | Le registry les appelle dans une transaction PG asynchrone, mais le `emit_*` lui-même est sync | Si on les rend async, signature trait casse |
| Trait `CatalogRegistry` reste inchangé (D12-bis-audit fix) | Pas de propagation client-side requis | `Arc<dyn CatalogRegistry>` reste opaque pour le handler |
| Tenant isolation D11 — l'audit ne fuit pas inter-tenant | Check côté handler (`assert_tenant_match`) | Confidentialité multi-tenant |
| `principal_present_label` D13 fix C-1 | Audit metric counter | Alerte `D13ObsAuditPrincipalUnknown` continue |
| Erreur `Internal` ne leak pas | D12-bis fix T-12bis-2 | Info disclosure |
| Migration `0001_init.sql` + `0002_tenant_name_format_check.sql` (D14 + audit fix) | sqlx migrate forward-only | Schéma cohérent |

### 2.2 Surface trait ne change pas

**Le trait `AuditSink` est invariant entre D12 (TracingAuditSink) et
D14-bis (HmacChainedAuditSink).** Le sprint D14-bis ajoute **un
seul nouveau impl** ; le code qui consomme `Arc<dyn AuditSink>` ne
sait pas que la signature HMAC est calculée — c'est transparent.

```rust
// crates/efe-server/src/catalog/audit.rs (existant, NON modifié)
pub trait AuditSink: Send + Sync {
    fn emit(&self, event: &AuditEvent);
}
```

→ Tous les emit-sites du registry (`PostgresCatalogRegistry::register`,
`InMemoryRegistry::register`, etc.) restent inchangés. Le sink est
échangé au niveau de `runtime::Server::build` :

```rust
// runtime.rs — D14-bis branch (extrait)
let audit_sink: Arc<dyn AuditSink> = if config.catalog.audit_signing.enabled {
    Arc::new(HmacChainedAuditSink::connect(
        pool.clone(),
        &config.catalog.audit_signing,
    ).await?)
} else {
    Arc::new(TracingAuditSink)
};
let catalog_audit = AuditEmitter::with_sink(audit_sink);
```

### 2.3 Gaps que D14-bis ferme

| Gap | Symptôme actuel | Fix D14-bis |
|-----|-----------------|-------------|
| Audit modifiable | Adversaire avec write-access edit le log Loki sans laisser de trace | HMAC-chained ; toute modification casse la chaîne |
| Audit suppressible | Adversaire avec write-access supprime des lignes du log file | Chaîne brisée → verify fail à la position du gap |
| Audit reorderable | Adversaire avec write-access change l'ordre des events | `prev_hmac` enchaîne les events ; reorder casse la chaîne |
| Audit perdu silencieusement | fluentbit drop, Loki tombe, retention policy expire trop tôt | Chaîne brisée détectée par verify ; alerte ops déclenchée |
| Pas de table SQL d'audit | Audit lives only in stdout/Loki ; cold storage manuel | `audit_events` table append-only en Postgres ; backup PG = backup audit |
| Pas de preuve juridique | "Trust us" insuffisant pour SOC 2 / HIPAA / GDPR | `efe-verify-audit` reproductible ; opérateur peut prouver intégrité |

---

## 3. Threat model — qu'est-ce qu'on protège

### 3.1 Adversaires considérés

| # | Adversaire | Capacité | D14-bis mitige ? |
|--:|-----------|----------|:----------------:|
| 1 | **Insider read-only** (intern, monitoring user) | Lit logs, métriques, traces | n/a — non-issue (pas de modif) |
| 2 | **Insider write-DB** (DBA, on-call ops) | UPDATE/DELETE sur PG | ✅ table append-only via trigger + chaîne HMAC |
| 3 | **Insider write-fs** (root sur la VM, CI/CD compromised) | Edit log files Loki/fluentbit | ✅ chaîne HMAC dans la table SQL → cohérence détectable |
| 4 | **External RCE** | Tout l'inventaire interne | ✅ partiel — la clé HMAC est en RAM ; un attaquant qui obtient la RAM peut signer des events frauduleux. Mitigation : key rotation rapide en cas de breach (D14-bis support live key swap) + monitoring `efe_audit_chain_break_total` |
| 5 | **Cluster-admin K8s** (RBAC compromis) | Lit Secrets, modify pods | ⚠ partiel — l'attaquant peut lire la clé HMAC. Mitigation : External-secrets Vault (rotate la clé dès qu'on suspecte un breach) ; les events PASSÉS restent inviolables tant que l'attaquant n'avait pas la clé pendant la fenêtre signée |
| 6 | **Compromised replica** (un pod compromis sur 3) | Sign avec la clé courante | ⚠ partiel — peut signer des events frauduleux. Mitigation : monitoring `audit_events.replica_id` divergent + `efe_audit_replica_sign_total` |
| 7 | **Time-skew attack** (NTP compromise) | Wrong timestamps in audit | ✅ partiel — `created_at_unix_ms` est dans le HMAC payload ; un attaquant peut changer l'horloge mais pas modifier des events passés |
| 8 | **Replay attack** (re-emit ancien event) | Insert duplicate of old event with new HMAC | ✅ — chaque event a un `event_id UUID v7` (timestamp-monotone) + le `prev_hmac` est dans le HMAC payload, donc un re-emit aurait un prev_hmac différent → casse la chaîne |

### 3.2 Garanties D14-bis

| Garantie | Force | Limite |
|----------|:-----:|--------|
| **Append-only** sur table SQL `audit_events` | strong | bypass possible si `pg_read_only` désactivé manuellement par DBA + trigger DROP — détectable par les logs PG eux-mêmes (audit du DBA = `pgaudit`, hors scope EFE) |
| **Tamper-evidence** (modification détectée) | strong | bypass possible si attaquant a la clé HMAC active **pendant** la modif — mitigé par rotation rapide |
| **Suppression-evidence** (gap dans la chaîne détecté) | strong | bypass possible si attaquant supprime un suffix CONTINU de la chaîne (la dernière entry vue par verify aura un HMAC valide ; pas de gap visible). Mitigation : checkpointing périodique vers external sink (cf. D15) |
| **Reorder-evidence** | strong | bypass impossible (chaîne ordonnée par construction) |
| **Replay-evidence** | strong | bypass impossible (event_id unique + chained) |
| **Time-skew-evidence** | medium | timestamp est signé, mais une attaque NTP avant les events les rendrait vrais |
| **Verifiable-by-third-party** | weak | nécessite la clé HMAC (vs Ed25519) — laissé à D14-ter |

---

## 4. Décision majeure #1 — HMAC chained vs Merkle tree vs Ed25519

### 4.1 Critères d'évaluation

| Critère | Poids | Pourquoi |
|---------|:-----:|----------|
| Performance signature (events/s) | 5/5 | Le sink est sur le hot path register/evolve/delete |
| Performance verify | 4/5 | Le verify CLI doit ingérer ~10⁶ events en quelques secondes |
| Footprint per-event (octets) | 4/5 | Stockage long-terme — 10⁶ events × N octets |
| Verify-by-third-party (sans clé) | 3/5 | Nice-to-have ; pas exigence MVP |
| Maturité crypto | 5/5 | Pas d'expérimentation à v0.1.14.5 |
| Implémentation Rust standard | 5/5 | sha2 + hmac crates sont workspace-friendly |
| Key rotation support | 4/5 | Doit être atomic et live |
| Streaming verify (sans tout charger) | 3/5 | Pour tables 10⁹+ events |

### 4.2 Comparaison

#### Option A : **HMAC chained (HMAC-SHA-256)**

**Mécanisme** : chaque event contient `prev_hmac = HMAC(key, prev_event_canonical)`,
puis `hmac = HMAC(key, this_event_canonical)`. Le verify replay
toute la chaîne et compare.

**Pour** :
- Performance signature : ~500 MB/s sha2 ; per-event ~0.5 µs.
- Performance verify : symétrique à signature.
- Footprint : 32 octets HMAC + 32 octets prev_hmac = 64 octets supplémentaires/event.
- Maturité : RFC 2104, ubiquitous.
- Streaming verify trivial : just iterate.
- Key rotation : multi-key verify supporté (try all keys for each event).

**Contre** :
- Verify nécessite la clé. Pas de preuve indépendante.
- Single point of failure si la clé est compromise (mais : events
  passés restent valides tant que la clé était secrète à l'époque).

#### Option B : **Merkle tree (binary)**

**Mécanisme** : on construit un arbre de Merkle sur les hashes des
events. Le root hash est la "racine" de la batch. Le verify d'un
event individuel demande O(log N) sibling hashes.

**Pour** :
- Verify partiel possible (one event sans replay tout).
- Compact root hash si on n'a besoin que du résumé.

**Contre** :
- Streaming insertion plus complexe (rebuild root à chaque insert).
- Verify d'event individuel plus complexe (sibling path).
- Footprint variable (sibling path).
- Pas de gain réel pour le streaming append-only use case.
- Maturité : oui mais surengineering pour un audit log.

#### Option C : **Ed25519 signatures**

**Mécanisme** : chaque event est signé avec une clé privée
Ed25519. Le verify utilise la clé publique.

**Pour** :
- Verifiable-by-third-party (régulateur n'a besoin que de la pub key).
- Aucune fuite de capacité de signature en distribuant la pub.
- Maturité : ubiquitous.

**Contre** :
- Performance signature : ~50 µs/event (vs 0.5 µs HMAC) — **100× plus lent**.
- Footprint : 64 octets signature/event.
- Pas de chaîne intrinsèque ; il faut quand même chaîner via `prev_signature_hash`.
- Complexité key management (private key handling, HSM/KMS).
- Out of scope pour le MVP : on n'a pas de régulateur tiers à v0.1.14.5.

#### Option D : **HMAC chained avec BLAKE3-keyed**

**Mécanisme** : remplace SHA-256 par BLAKE3 pour speed.

**Pour** :
- 5-10× plus rapide que SHA-256 sur AVX2 / NEON.
- Output configurable (32 ou 64 octets).

**Contre** :
- Maturité moindre (publié 2020 ; widely adopted depuis 2022 mais moins ubiquitous que HMAC-SHA-256).
- Pas de support FIPS 140-3 (bloque certains déploiements régulés).
- Gain perf marginal au volume audit (events/s relatively low).

### 4.3 Décision

| Critère | Poids | HMAC-SHA-256 | Merkle | Ed25519 | BLAKE3-keyed |
|---------|:-----:|:------------:|:------:|:-------:|:------------:|
| Perf sign | 5 | ✅ 5 | ✅ 5 | ❌ 1 | ✅ 5 |
| Perf verify | 4 | ✅ 5 | 🟡 3 | ❌ 1 | ✅ 5 |
| Footprint | 4 | ✅ 4 | 🟡 3 | ✅ 4 | ✅ 4 |
| Verify-by-3rd-party | 3 | ❌ 0 | 🟡 2 | ✅ 5 | ❌ 0 |
| Maturité | 5 | ✅ 5 | ✅ 5 | ✅ 5 | 🟡 3 |
| Rust impl | 5 | ✅ 5 | ✅ 4 | ✅ 5 | ✅ 5 |
| Key rotation | 4 | ✅ 5 | 🟡 3 | ✅ 4 | ✅ 5 |
| Streaming verify | 3 | ✅ 5 | 🟡 3 | ✅ 5 | ✅ 5 |
| **Total pondéré** (max 165) | | **143** | 109 | 95 | 132 |

→ **Décision : HMAC-SHA-256 chained**.

Rationale chirurgical :

1. Aligned avec D11 HS256 (JWT) et D12 SHA-256 (`content_sha256`)
   — un seul algorithme, un seul key-management pattern, un seul
   audit posture.
2. Performance acceptable pour le volume audit (registers/s borné
   à ~10² par tenant ; 10⁴ tenants × 10² = 10⁶/s aggregate, mais
   en pratique << 10²/s).
3. FIPS 140-3 compliant (HMAC-SHA-256 est dans la liste approved).
4. Library Rust mature : `hmac` crate v0.12 + `sha2` crate v0.10,
   already in workspace deps post-D14.
5. Verify-by-third-party laissé à **D14-ter** (Ed25519 ajouté
   par-dessus, ne casse rien).

---

## 5. Décision majeure #2 — algorithme HMAC-SHA-256 vs BLAKE3-keyed vs autre

Cette décision est emboîtée dans la #1 (déjà décidée HMAC-SHA-256),
mais on liste explicitement les paramètres pour traçabilité audit.

| Paramètre | Choix | Rationale |
|-----------|-------|-----------|
| Hash function | **SHA-256** | FIPS 140-3 ; aligned avec `content_sha256` D12 |
| MAC construction | **HMAC** (RFC 2104) | Standard ; resistant length-extension contrairement à un H(key‖msg) naïf |
| Key length | **32 octets** (256 bits) | Optimal pour SHA-256 (block size 64, key ≤ block sans truncation) |
| Output length | **32 octets** = 64 hex chars | Aligned avec content_sha256 hex format |
| Encoding (storage) | **lowercase hex** | Aligned avec `content_sha256` ; ergonomic SQL queries |
| Canonical JSON | **sorted keys, no whitespace, ASCII-safe escapes** | Déterministe cross-version Rust, cross-platform |
| Genesis prev_hmac | **constante "0…0" 32 octets** (64 zeros hex) | Marqueur "pas de prev" |

---

## 6. Décision majeure #3 — storage : `audit_events` table SQL vs JSONL append vs in-line dans `catalogs`

### 6.1 Trade-offs

#### Option A : table SQL `audit_events` dédiée

**Pour** :
- Append-only enforce-able via PG trigger.
- Backup automatique (pg_basebackup, pg_dump).
- Queryable (SELECT WHERE tenant=… AND created_at_unix_ms BETWEEN … AND …).
- Indexable.
- Multi-replica safe (single source of truth).

**Contre** :
- Coût stockage (10⁶ events × ~500 octets ≈ 500 MB/an au volume cible v0.1.14).
- Coût write : ~5 ms (pg roundtrip) par audit event.

#### Option B : JSONL append à un file appender

**Pour** :
- Léger, pas de PG dépendence.
- File rotation standard (logrotate).

**Contre** :
- Pas append-only sans configuration FS spéciale (chattr +a, ZFS RO, etc.).
- Multi-replica → N files divergents.
- Pas queryable sans jq + grep.
- Pas backupable sans pipeline custom.

#### Option C : in-line dans `catalogs.audit_chain_hmac` (column ajoutée)

**Pour** :
- Pas de table supplémentaire.

**Contre** :
- Couple structurellement le content au audit. Une mise à jour
  `audit_chain_hmac` viole "append-only catalogs".
- Pas de granularité sur les events delete (la row est supprimée).
- Mauvais design.

### 6.2 Décision : Option A

→ **Décision : table SQL `audit_events` dédiée + trigger
PostgreSQL block UPDATE/DELETE**.

DDL résumé (détail § 10) :

```sql
CREATE TABLE audit_events (
    seq             BIGSERIAL PRIMARY KEY,
    event_id        UUID NOT NULL UNIQUE,        -- v7 timestamp-monotone
    chain_id        TEXT NOT NULL,                -- "global" or "tenant:<X>"
    tenant          TEXT NOT NULL,
    action          TEXT NOT NULL,                -- "register"|"evolve"|"delete"
    op_tag          TEXT NOT NULL,
    catalog_key     TEXT NOT NULL,
    principal       TEXT NOT NULL,
    description     TEXT NOT NULL,
    parent_key      TEXT NOT NULL,
    content_sha256  TEXT NOT NULL,
    created_at_unix_ms BIGINT NOT NULL,
    canonical_payload  JSONB NOT NULL,            -- exact bytes that fed the HMAC
    prev_hmac       TEXT NOT NULL,                -- 64-char hex
    hmac            TEXT NOT NULL,                -- 64-char hex
    key_id          TEXT NOT NULL,                -- which key signed (rotation)
    db_inserted_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Plus le trigger :

```sql
CREATE OR REPLACE FUNCTION audit_events_block_mutation()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'audit_events is append-only ; UPDATE/DELETE forbidden';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_events_no_update BEFORE UPDATE ON audit_events
    FOR EACH ROW EXECUTE FUNCTION audit_events_block_mutation();
CREATE TRIGGER audit_events_no_delete BEFORE DELETE ON audit_events
    FOR EACH ROW EXECUTE FUNCTION audit_events_block_mutation();
```

(le DBA peut DROP TRIGGER, mais c'est lui-même un audit-able event
côté `pg_audit` ou file-system)

---

## 7. Décision majeure #4 — granularité de la chaîne : per-tenant vs global

### 7.1 Trade-offs

#### Option A : chaîne **globale** (1 chaîne pour tout le cluster)

**Pour** :
- Simple.
- Une seule séquence à verify.
- Empêche un tenant adversarial d'effacer ses events sans casser la chaîne globale.

**Contre** :
- Sérialise tous les inserts d'audit globalement → bottleneck à
  scale.
- Verify de la chaîne d'un seul tenant nécessite verify de toute
  la chaîne.

#### Option B : chaîne **per-tenant** (1 chaîne par tenant)

**Pour** :
- Verify isolé par tenant (un audit régulateur d'un seul tenant).
- Pas de bottleneck cross-tenant.
- Aligned avec D11 per-tenant isolation.

**Contre** :
- Multi-key state à maintenir (le `last_hmac` par tenant).
- Un tenant adversarial peut se réécrire sa propre chaîne s'il a la clé.

#### Option C : chaîne **mixte** (1 chaîne globale + 1 chaîne per-tenant)

**Pour** :
- Garanties cumulatives.

**Contre** :
- Doublement de coûts.
- Pas de gain sécurité réel : si la clé est compromise, les 2 chaînes le sont.

### 7.2 Décision : option B (per-tenant) avec un `chain_id` extensible

→ **Décision : chaîne per-tenant**, `chain_id = "tenant:<X>"`.

Le `chain_id` column dans `audit_events` permet une **future
migration** vers une chaîne globale (D14-ter ou D17+) en ajoutant
juste un `chain_id = "global"` parallèlement, sans casser la
compat.

**Justification** :
1. D11 zero-trust per-tenant est l'invariant central de l'archi.
2. Verify isolated par tenant matche les use cases régulateurs (un audit pour un seul client final).
3. Pas de bottleneck (advisory lock per-tenant comme D14 quotas).
4. Forward-compat global chain via `chain_id` extension.

---

## 8. Décision majeure #5 — canonical serialisation pour le HMAC input

### 8.1 Le problème

Pour que la signature soit reproductible, le **bytestream signé
doit être déterministe**. Deux clients qui voient le même event
logique doivent produire la même bytestream avant HMAC, sinon le
verify échoue.

### 8.2 Options évaluées

| Option | Description | Pour | Contre |
|--------|-------------|------|--------|
| A. Plain `serde_json::to_vec(&event)` | Output dépend de l'ordre des fields HashMap | Simple | Non-déterministe (HashMap RNG) |
| B. JCS RFC 8785 | Standard "JSON Canonicalization Scheme" | Spec'd | Pas de impl Rust mature ; complex Unicode rules |
| C. **JSON sorted keys + no whitespace + UTF-8** (custom) | BTreeMap + `serde_json::to_vec` minified | Déterministe par construction (BTreeMap iter alphabétique) ; ASCII-safe escapes | Pas de RFC formel, mais bien défini |
| D. CBOR canonical | Binary canonical form (RFC 8949) | Compact, déterministe | Pas de inspection humaine ; debug harder |
| E. Custom byte format | `tenant\nname\n…` | Contrôle total | Bug-prone, pas de versioning |

### 8.3 Décision : option C

→ **Décision : JSON sorted keys + minified + UTF-8** via le helper
`canonical_audit_payload` (cf. § 9.2).

**Garanties** :

1. Tous les fields sont déclarés en `BTreeMap<String, serde_json::Value>` (alphabetical iteration).
2. `serde_json::to_vec_pretty=false` (pas de whitespace).
3. Strings encodées en UTF-8 ; escape `\\u00xx` pour les chars < 0x20.
4. Floats : `f32::to_string()` Rust standard ; pas de events portant des floats dans D14-bis (tous les fields sont strings ou i64).

**Test** :
- `canonical_audit_payload(event)` produces N bytes identical across rust versions, OS, locale.
- Round-trip test : `deserialize(canonical_audit_payload(e))` == `e`.

### 8.4 Schema versioning

Le canonical payload contient un champ `schema_version: 1` pour
permettre une **migration future** sans casser les chaînes existantes.

```jsonc
{
  "schema_version": 1,
  // …les autres champs sortis…
}
```

Si D14-ter ajoute des champs, il bump à `schema_version: 2` ; le
verify utilise le `schema_version` pour choisir le décodeur
canonique correspondant.

---

## 9. Format du record signé — schema détaillé

### 9.1 Champs signés (input du HMAC)

```jsonc
{
  "schema_version": 1,
  "event_id": "01234567-89ab-7def-8123-456789abcdef",
  "chain_id": "tenant:acme",
  "tenant": "acme",
  "action": "register",
  "op_tag": "register",
  "catalog_key": "acme/billing@v1",
  "principal": "alice@acme",
  "description": "init",
  "parent_key": "",
  "content_sha256": "5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8",
  "created_at_unix_ms": 1700000000000,
  "prev_hmac": "0000000000000000000000000000000000000000000000000000000000000000"
}
```

Le HMAC est calculé sur `canonical_serialize(this_payload)`.

### 9.2 Pseudo-code canonical serialise

```rust
pub fn canonical_audit_payload(event: &AuditEventSigned) -> Vec<u8> {
    use std::collections::BTreeMap;
    use serde_json::{Map, Value};
    let mut m: BTreeMap<&str, Value> = BTreeMap::new();
    m.insert("schema_version", Value::from(1u32));
    m.insert("event_id", Value::from(event.event_id.to_string()));
    m.insert("chain_id", Value::from(event.chain_id.clone()));
    m.insert("tenant", Value::from(event.tenant.clone()));
    m.insert("action", Value::from(event.action.clone()));
    m.insert("op_tag", Value::from(event.op_tag.clone()));
    m.insert("catalog_key", Value::from(event.catalog_key.clone()));
    m.insert("principal", Value::from(event.principal.clone()));
    m.insert("description", Value::from(event.description.clone()));
    m.insert("parent_key", Value::from(event.parent_key.clone()));
    m.insert("content_sha256", Value::from(event.content_sha256.clone()));
    m.insert("created_at_unix_ms", Value::from(event.created_at_unix_ms));
    m.insert("prev_hmac", Value::from(event.prev_hmac.clone()));
    let map: Map<String, Value> = m.into_iter()
        .map(|(k, v)| (k.to_owned(), v)).collect();
    serde_json::to_vec(&Value::Object(map))
        .expect("BTreeMap sorted Value serialise")
}
```

`BTreeMap` itère en ordre alphabétique, donc l'output JSON est
field-order stable.

### 9.3 Champs HORS payload signé

| Champ | Pourquoi exclu |
|-------|----------------|
| `hmac` | Self-référence : le HMAC ne peut pas se signer |
| `key_id` | Conserve la flexibilité de re-signer avec une nouvelle clé sans changer la canonical form |
| `seq` (BIGSERIAL) | Assigned par PG, pas connu côté client au moment du sign |
| `db_inserted_at` | Wall-clock du DB, indépendant du `created_at_unix_ms` côté app |
| `canonical_payload` (JSONB) | C'est lui-même la sortie du canonical_serialise — stocker pour debug, mais non-signé (sinon récursion) |

### 9.4 Schema sign-then-store

```
event = AuditEventSigned {
    event_id: uuid_v7(),
    chain_id: format!("tenant:{}", tenant),
    tenant, action, op_tag, catalog_key, principal,
    description, parent_key, content_sha256,
    created_at_unix_ms: now_unix_ms(),
    prev_hmac: state.last_hmac_for_chain(chain_id),  // genesis = "00..00"
};
canonical = canonical_audit_payload(&event);
hmac = HMAC_SHA256(active_key, canonical);
hmac_hex = hex_lower(hmac);
INSERT INTO audit_events (..., canonical_payload, prev_hmac, hmac, key_id) VALUES (...);
state.update_last_hmac(chain_id, hmac_hex);
```

---

## 10. Schéma SQL `audit_events` — DDL complet

### 10.1 Migration `0003_audit_events.sql`

```sql
-- ============================================================================
-- Sprint D14-bis — HMAC-chained tamper-evident audit log.
-- ============================================================================

CREATE TABLE audit_events (
    -- Monotonic insertion order (DB-assigned). Useful for streaming
    -- verify and pagination.
    seq                BIGSERIAL PRIMARY KEY,

    -- UUIDv7 (timestamp-monotone) — opaque id consumed by external
    -- correlation.
    event_id           UUID      NOT NULL UNIQUE,

    -- Chain identifier ("tenant:<X>" today, "global" reserved D14-ter).
    chain_id           TEXT      NOT NULL,

    -- Per-event metadata (mirror of the `AuditEvent` struct in
    -- `crates/efe-server/src/catalog/audit.rs`).
    tenant             TEXT      NOT NULL,
    action             TEXT      NOT NULL,        -- "register"|"evolve"|"delete"
    op_tag             TEXT      NOT NULL,        -- "register"|"append_class"|...
    catalog_key        TEXT      NOT NULL,
    principal          TEXT      NOT NULL,
    description        TEXT      NOT NULL DEFAULT '',
    parent_key         TEXT      NOT NULL DEFAULT '',
    content_sha256     TEXT      NOT NULL,
    created_at_unix_ms BIGINT    NOT NULL,

    -- Exact bytes that fed the HMAC (debug / re-verify by external tools).
    canonical_payload  JSONB     NOT NULL,

    -- Chain pointers + signature.
    prev_hmac          TEXT      NOT NULL,        -- 64-char hex
    hmac               TEXT      NOT NULL,        -- 64-char hex
    key_id             TEXT      NOT NULL,        -- "k1", "k2", ... (rotation)

    -- DB-side audit (orthogonal to created_at_unix_ms).
    db_inserted_at     TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Hex format invariants.
    CONSTRAINT audit_hmac_hex_format
        CHECK (hmac ~ '^[0-9a-f]{64}$'),
    CONSTRAINT audit_prev_hmac_hex_format
        CHECK (prev_hmac ~ '^[0-9a-f]{64}$'),

    -- Action enum.
    CONSTRAINT audit_action_enum
        CHECK (action IN ('register', 'evolve', 'delete')),

    -- Chain id format (forward-compat for "global" + future).
    CONSTRAINT audit_chain_id_format
        CHECK (chain_id ~ '^(tenant:[a-z0-9_-]{1,63}|global)$')
);

-- Append-only enforcement via trigger : reject UPDATE/DELETE.
-- (Note : a DBA with sufficient privileges can DROP TRIGGER ; that's
-- a separate, audit-able event in pg_audit / file system.)
CREATE OR REPLACE FUNCTION audit_events_block_mutation()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION
        'audit_events is append-only (D14-bis HMAC chain) ; \
         UPDATE/DELETE rejected by trigger %.\
         If you really need to mutate, DROP the trigger first ; doing \
         so will be visible in pg_audit / DBA audit trail.',
        TG_NAME;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_events_no_update
    BEFORE UPDATE ON audit_events
    FOR EACH ROW EXECUTE FUNCTION audit_events_block_mutation();

CREATE TRIGGER audit_events_no_delete
    BEFORE DELETE ON audit_events
    FOR EACH ROW EXECUTE FUNCTION audit_events_block_mutation();

-- TRUNCATE bypass UPDATE/DELETE triggers in PG ≥ 8.4 ; we add a
-- separate trigger for it.
CREATE TRIGGER audit_events_no_truncate
    BEFORE TRUNCATE ON audit_events
    FOR EACH STATEMENT EXECUTE FUNCTION audit_events_block_mutation();

-- Indexes.
CREATE INDEX audit_events_chain_seq_idx
    ON audit_events (chain_id, seq);
CREATE INDEX audit_events_tenant_created_at_idx
    ON audit_events (tenant, created_at_unix_ms);
CREATE INDEX audit_events_event_id_idx
    ON audit_events (event_id);

COMMENT ON TABLE audit_events IS
    'Sprint D14-bis — HMAC-chained tamper-evident audit log. \
     Append-only via trigger. Each row''s `hmac` is computed from \
     `canonical_payload` + the active HMAC key. The `prev_hmac` chains \
     this row to the previous row in the same `chain_id` ; the verify \
     CLI replays the chain from genesis to detect tampering, deletion, \
     or reordering.';
```

### 10.2 Indexes rationale

| Index | Use case |
|-------|----------|
| `audit_events_chain_seq_idx` | Verify replay : `SELECT … FROM audit_events WHERE chain_id = $1 ORDER BY seq` |
| `audit_events_tenant_created_at_idx` | Time-range query for compliance reports : "show me all audit events for tenant X between ts1 and ts2" |
| `audit_events_event_id_idx` | Correlation with external systems (Loki) by event_id |

### 10.3 Cardinalité estimée

Volume cible v0.1.14.5 :
- 100 tenants
- 10 mutations/tenant/jour
- 1 000 events/jour aggregate
- 365 000 events/an
- 500 octets/event canonical_payload + 200 octets autres = 700 octets
- ~256 MB/an de stockage audit_events

→ négligeable. PG handle 10⁹+ rows trivialement.

---

## 11. `HmacChainedAuditSink` — Rust impl + concurrency

### 11.1 Type

```rust
// crates/efe-server/src/catalog/audit_hmac.rs (NEW)

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use sqlx::PgPool;

use super::audit::{AuditEvent, AuditSink};
use crate::config::AuditSigningConfig;

/// HMAC-chained tamper-evident audit sink.
///
/// State : per-chain `last_hmac` (the HMAC of the most recent event
/// committed to `audit_events`). Persisted in the DB ; the in-memory
/// cache is hydrated lazily on first use of each chain.
pub struct HmacChainedAuditSink {
    pool: PgPool,
    /// Active signing key (the head of the keys file).
    active_key: Arc<HmacKey>,
    /// All keys (active + rotated-out, kept for verify). Indexed by `key_id`.
    all_keys: Arc<HashMap<String, HmacKey>>,
    /// Per-chain last_hmac cache. Keyed by chain_id.
    last_hmac_cache: Arc<Mutex<HashMap<String, String>>>,
    /// Optional fan-out to the legacy tracing sink (for Loki shipping).
    chain_to_tracing: bool,
}

#[derive(Clone)]
pub struct HmacKey {
    pub id: String,
    pub material: secrecy::SecretBox<[u8; 32]>,
}
```

### 11.2 `emit()` flow (the hot path)

```rust
impl AuditSink for HmacChainedAuditSink {
    fn emit(&self, event: &AuditEvent) {
        // The trait is sync, but the SQL insert is async. Spawn onto
        // the tokio runtime ; the audit emit is best-effort from the
        // caller's perspective (the registry has already committed
        // its own transaction).
        let pool = self.pool.clone();
        let active_key = self.active_key.clone();
        let last_hmac_cache = self.last_hmac_cache.clone();
        let chain_to_tracing = self.chain_to_tracing;
        let event = event.clone();
        tokio::spawn(async move {
            if let Err(e) = sign_and_persist(
                pool, active_key, last_hmac_cache, &event,
            ).await {
                tracing::error!(
                    target: "efe.audit.hmac",
                    err = %e,
                    "audit sign+persist failed ; chain integrity at risk",
                );
                efe_telemetry::metrics::d14bis_instruments::audit_sign_failures_total()
                    .with_label_values(&[event.action.as_str()])
                    .inc();
            }
            if chain_to_tracing {
                super::audit::TracingAuditSink.emit(&event);
            }
        });
    }
}

async fn sign_and_persist(
    pool: PgPool,
    active_key: Arc<HmacKey>,
    last_hmac_cache: Arc<Mutex<HashMap<String, String>>>,
    event: &AuditEvent,
) -> Result<(), AuditSignError> {
    let chain_id = format!("tenant:{}", event.key.tenant);
    let lock_id = lock_id_for_chain(&chain_id);

    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock_id).execute(&mut *tx).await?;

    // Fetch (or hydrate) the chain head.
    let prev_hmac = {
        let mut cache = last_hmac_cache.lock().await;
        if let Some(h) = cache.get(&chain_id) {
            h.clone()
        } else {
            let h: Option<String> = sqlx::query_scalar(
                "SELECT hmac FROM audit_events
                 WHERE chain_id = $1 ORDER BY seq DESC LIMIT 1",
            ).bind(&chain_id).fetch_optional(&mut *tx).await?;
            let h = h.unwrap_or_else(|| GENESIS_PREV_HMAC.to_owned());
            cache.insert(chain_id.clone(), h.clone());
            h
        }
    };

    let signed = AuditEventSigned::from_audit_event_with_prev(
        event, &chain_id, &prev_hmac,
    );
    let canonical = canonical_audit_payload(&signed);
    let hmac = compute_hmac(active_key.material.expose_secret(), &canonical);
    let hmac_hex = hex_lower(&hmac);

    sqlx::query("INSERT INTO audit_events (
        event_id, chain_id, tenant, action, op_tag, catalog_key,
        principal, description, parent_key, content_sha256,
        created_at_unix_ms, canonical_payload, prev_hmac, hmac, key_id
    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)")
    .bind(signed.event_id)
    .bind(&chain_id)
    .bind(&signed.tenant)
    .bind(&signed.action)
    .bind(&signed.op_tag)
    .bind(&signed.catalog_key)
    .bind(&signed.principal)
    .bind(&signed.description)
    .bind(&signed.parent_key)
    .bind(&signed.content_sha256)
    .bind(signed.created_at_unix_ms)
    .bind(serde_json::Value::from_slice(&canonical)?)
    .bind(&prev_hmac)
    .bind(&hmac_hex)
    .bind(&active_key.id)
    .execute(&mut *tx).await?;

    tx.commit().await?;

    // Update the in-memory chain head only after the COMMIT lands.
    last_hmac_cache.lock().await.insert(chain_id, hmac_hex);
    Ok(())
}
```

### 11.3 Concurrency invariants

1. **`pg_advisory_xact_lock` per-chain** sérialise les inserts pour
   un `chain_id` donné. Garantit que `prev_hmac` est correct (read
   under lock + insert under lock + commit).
2. **In-memory cache** est mis à jour SEULEMENT après le COMMIT.
   Si COMMIT fail, le cache n'est pas pollué.
3. **Cache miss** : on read `MAX(seq)` sous le lock, donc cohérent.
4. **Multi-replica** : chaque réplique a son propre cache, mais
   sous lock ils convergent. Le premier à commit gagne, le second
   read sa propre row depuis DB et continue.

### 11.4 Trade-off async vs sync

`AuditSink::emit()` est sync (héritage D12). On choisit de spawn
sur tokio plutôt que de bloquer le caller :

**Pour spawn** :
- Le caller (trait `CatalogRegistry::register`) a déjà commit sa
  transaction PG ; bloquer le retour pour signer l'audit ajoute
  ~10 ms à chaque mutation.
- L'audit est best-effort du point de vue du caller — le success
  d'un register ne dépend pas du success de l'audit (elle est
  émise APRÈS le commit).

**Contre spawn** :
- Si l'audit fail (unique violation event_id, PG down), le caller
  ne le sait pas. **Mitigation** : counter Prometheus
  `efe_audit_sign_failures_total` + alert.
- Ordering : un crash entre `commit catalog tx` et `audit insert`
  laisse une mutation sans audit. **Mitigation** : un job de
  réconciliation (D15) qui scan le PG `catalogs` pour des rows
  sans audit_event correspondant et émet l'audit a posteriori.

→ Décision : **spawn** + counter + alert + D15 job réconciliation.

---

## 12. Genesis event — bootstrap de la chaîne

### 12.1 Comportement par défaut

À la **toute première** mutation pour une chaîne (`chain_id`),
`prev_hmac` = `GENESIS_PREV_HMAC` = `"00...00"` (64 zeros hex).

```rust
const GENESIS_PREV_HMAC: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";
```

C'est conventionnel — pas un secret. Le verify utilise cette
constante comme starting state.

### 12.2 Genesis explicite (optionnel)

D14-bis ne livre PAS un genesis event explicite (overhead). Mais
l'architecture prévoit (D14-ter ou D17+) :

```jsonc
// première row de chaque chain_id
{
  "schema_version": 1,
  "event_id": "00000000-0000-7000-8000-000000000000",  // UUID v7 epoch
  "chain_id": "tenant:acme",
  "tenant": "acme",
  "action": "genesis",
  "op_tag": "genesis",
  "catalog_key": "",
  "principal": "system",
  "description": "Chain genesis",
  "parent_key": "",
  "content_sha256": "",
  "created_at_unix_ms": <chain creation time>,
  "prev_hmac": "0000...0000"
}
```

À v0.1.14.5 : non livré ; le default `prev_hmac = GENESIS_PREV_HMAC`
suffit pour le verify. Documentation pour D14-ter.

---

## 13. Key management — `EFE_AUDIT_HMAC_SECRETS_FILE`

### 13.1 Pattern réutilisé de M12 HS256

D11/M12 livre un pattern propre pour les JWT HS256 secrets :
- Fichier `EFE_JWT_SECRETS_FILE`, 1 ligne = 1 clé base64.
- 1ère ligne = active (signing) ; suivantes = verify-only (rotation in-flight).

D14-bis applique le **même pattern verbatim** pour les HMAC keys :

| Aspect | M12 HS256 | D14-bis HMAC |
|--------|-----------|--------------|
| Env var | `EFE_JWT_SECRETS_FILE` | `EFE_AUDIT_HMAC_SECRETS_FILE` |
| Format ligne | base64-url-encoded key | base64-url-encoded key |
| Key length | ≥ 32 octets | **exactement** 32 octets (HMAC-SHA-256 optimal) |
| Active key | 1ère ligne | 1ère ligne |
| Verify keys | tous les autres | tous les autres |
| Comment | `#` au début | `#` au début |
| Reload | redémarrage du pod | redémarrage du pod (D17+ : SIGHUP reload) |

### 13.2 Format fichier

```
# Active signing key (used for new audit events).
RG9UTm90VXNlVGhpc0tleS1FZmVBdWRpdEtleVZlcnNpb24w
# Old key kept for verification of historical events.
T2xkSE1BQ0tleVByZS1Sb3RhdGlvbi1JbkF1Z3VzdC0yMDI1
```

(Le contenu réel est généré par `openssl rand -base64 32` ;
l'opérateur édite le fichier + restart les pods.)

### 13.3 Key ID assignment

Chaque clé a un `key_id` calculé déterministiquement à partir de
ses bytes (premiers 8 chars de SHA-256(key_bytes)) :

```rust
fn key_id_of(material: &[u8; 32]) -> String {
    let h = Sha256::digest(material);
    hex_lower(&h)[..8].to_owned()
}
```

Avantages :
- Pas de collision avec la rotation (deux clés distinctes ont des `key_id` distincts avec proba 2⁻³² par clé).
- Pas de state à maintenir côté opérateur.

### 13.4 Validation au boot

`runtime::Server::build` valide le fichier :

1. Existe.
2. Au moins 1 ligne non-commentaire.
3. Chaque clé est exactement 32 octets après decode.
4. Pas de doublons (deux clés identiques = bug rotation).

Refus avec `ConfigError::Invalid` au boot.

### 13.5 Storage Helm

`EFE_AUDIT_HMAC_SECRETS_FILE` pointe vers un volume mount :

```yaml
volumeMounts:
  - name: audit-hmac
    mountPath: /etc/efe/secrets/audit-hmac
    readOnly: true
volumes:
  - name: audit-hmac
    secret:
      secretName: {{ .Values.config.audit.signing.existingSecretName }}
```

Le contenu du Secret est typiquement :

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: efe-audit-hmac-secrets
type: Opaque
data:
  hmac.keys: |
    UklhWjI1RjZhUzlyRkx5UThzRjJqd1VPMUFiQzNlcw==  # base64-encoded file content
```

---

## 14. Key rotation — multi-key verify, single-key sign

### 14.1 Pattern

* **Sign** : utilise toujours la 1ère clé (active).
* **Verify** : itère sur toutes les clés ; un event match si N'IMPORTE
  laquelle des clés produit le HMAC stocké.
* **Rotation** : opérateur ajoute une NEW clé en haut du fichier ;
  l'ancienne descend ; restart le pod. La NEW clé devient active.
  L'ancienne reste pour verify-only.

### 14.2 Dans `audit_events`, `key_id` rend explicite quelle clé a signé

```sql
SELECT chain_id, key_id, count(*) FROM audit_events
GROUP BY chain_id, key_id
ORDER BY chain_id, key_id;
```

→ Réponse opérateur : "key abcdef01 a signé 12k events, key 12345678
en a signé 3k". Si la 1ère est obsolète mais reste verify-only, OK.
Si elle a disparu du fichier, **les events historiques signés avec
elle ne peuvent plus être verify** → alerte.

### 14.3 Decommissioning

Pour retirer une vieille clé en toute sécurité :

1. Re-encrypt les events historiques signés par l'ancienne clé avec
   la nouvelle. **Pas implémenté D14-bis** (D17+ ; complexité — il
   faut maintenir append-only en re-signant).
2. OU **archiver la chaîne historique** (D15) en un dump signé,
   puis supprimer les rows et la clé.

D14-bis livre un **lint job** qui alerte si une clé qui a signé
des events n'est plus dans le fichier (key_id non-trouvable lors
du verify).

---

## 15. Verify — library + CLI `efe-verify-audit`

### 15.1 Library `audit_hmac::verify`

```rust
// crates/efe-server/src/catalog/audit_hmac.rs

pub struct VerifyReport {
    pub chain_id: String,
    pub n_events: u64,
    pub first_break_seq: Option<i64>,  // None = chain intact
    pub last_seq_verified: Option<i64>,
    pub keys_used: Vec<String>,        // distinct key_ids encountered
}

pub async fn verify_chain(
    pool: &PgPool,
    chain_id: &str,
    keys: &HashMap<String, HmacKey>,
) -> Result<VerifyReport, VerifyError> {
    let mut prev_hmac = GENESIS_PREV_HMAC.to_owned();
    let mut report = VerifyReport {
        chain_id: chain_id.to_owned(),
        n_events: 0, first_break_seq: None,
        last_seq_verified: None, keys_used: Vec::new(),
    };
    let mut rows = sqlx::query(
        "SELECT seq, event_id, canonical_payload, prev_hmac, hmac, key_id
         FROM audit_events WHERE chain_id = $1 ORDER BY seq",
    ).bind(chain_id).fetch(pool);

    while let Some(row) = rows.try_next().await? {
        let seq: i64 = row.try_get("seq")?;
        let stored_prev: String = row.try_get("prev_hmac")?;
        let stored_hmac: String = row.try_get("hmac")?;
        let key_id: String = row.try_get("key_id")?;
        let canonical: serde_json::Value = row.try_get("canonical_payload")?;

        if stored_prev != prev_hmac {
            report.first_break_seq = Some(seq);
            return Ok(report);  // chain broken at seq
        }
        let canonical_bytes = serde_json::to_vec(&canonical)?;
        let key = keys.get(&key_id).ok_or(VerifyError::UnknownKey(key_id.clone()))?;
        let computed_hmac = compute_hmac(key.material.expose_secret(), &canonical_bytes);
        if hex_lower(&computed_hmac) != stored_hmac {
            report.first_break_seq = Some(seq);
            return Ok(report);
        }
        prev_hmac = stored_hmac;
        report.n_events += 1;
        report.last_seq_verified = Some(seq);
        if !report.keys_used.contains(&key_id) {
            report.keys_used.push(key_id);
        }
    }
    Ok(report)
}
```

### 15.2 CLI `efe-verify-audit`

```bash
$ efe-verify-audit --chain tenant:acme \
    --pg-url postgres://efe:efe@localhost:5432/efe_catalog_db \
    --keys-file /etc/efe/secrets/audit-hmac/hmac.keys

Verifying chain tenant:acme...
✓ 12 894 events verified
✓ chain intact through seq 89 422
✓ keys used : abcdef01, 12345678
```

Ou en cas de break :

```bash
$ efe-verify-audit --chain tenant:acme ...

Verifying chain tenant:acme...
✗ chain BREAK at seq 89 421
  expected prev_hmac : 7e2f...
  stored   prev_hmac : 0000...
  → row at seq 89 421 was inserted out-of-order or modified
  → audit integrity compromised after seq 89 420
```

### 15.3 Output formats

* **TTY** (default) : human-readable colorized.
* **JSON** (`--json`) : machine-parseable.
* **JUnit** (`--junit`) : pour CI integration.

### 15.4 Streaming mode

Pour les chaînes très longues (> 10⁶ events), le verify charge
batch-par-batch via le cursor PostgreSQL :

```sql
SELECT … FROM audit_events WHERE chain_id = $1 AND seq > $2 ORDER BY seq LIMIT 10000;
```

Mémoire bornée à ~10 MB peu importe la chaîne.

---

## 16. Performance budget vs D14 baseline

### 16.1 D14 `register` p99 baseline

D14 baseline (loopback PG) : `register` p99 ≤ 20 ms.

### 16.2 D14-bis overhead

Le sink emit (`spawn` + audit insert) est **off the critical path**
du `register` (le sink emit après le `tx.commit()`). Donc le
`register` p99 ne change pas.

L'overhead spawn-side :

| Step | Time |
|------|------|
| `tokio::spawn` overhead | ~1 µs |
| `pg_advisory_xact_lock(chain_id)` | ~100 µs (loopback) |
| `SELECT MAX(seq) WHERE chain_id` (cache miss) | ~1 ms |
| `canonical_audit_payload` | ~5 µs |
| `HMAC-SHA-256` 500 octets | ~1 µs |
| `INSERT INTO audit_events` | ~3 ms |
| `tx.commit()` | ~2 ms |
| **Total per audit event** | **~7 ms** |

Aggregate : si `register` p99 = 20 ms et l'audit s'exécute en
parallèle (spawn), le **client voit 20 ms** mais le pool PG est
chargé à hauteur de l'audit en plus.

Capacity planning : un pool de 8 connexions soutient ~500
events/sec aggregate (audit + catalog), bien au-delà des cibles
v0.1.14.5 (~10/sec).

### 16.3 Vérification

`benches/audit_hmac.rs` (criterion) :
- `bench_emit_synchronous` : sign + insert sans spawn ; mesure
  ground truth.
- `bench_emit_via_sink_spawn` : sign + insert via spawn ; mesure
  l'overhead vs synchronous.

---

## 17. Observability D13/D14 extensions

### 17.1 Nouveaux instruments D14-bis

```rust
// crates/efe-telemetry/src/metrics/d14bis_instruments.rs (NEW)

once_metric!(
    /// `efe_audit_events_signed_total{action, chain_id}` — count of
    /// events successfully signed and persisted.
    audit_events_signed_total, IntCounterVec, …
);

once_metric!(
    /// `efe_audit_sign_failures_total{action, error}` — count of
    /// audit emit failures (PG insert error, key missing, etc.).
    /// Critical alert : non-zero rate triggers `D14bisAuditSignFailing`.
    audit_sign_failures_total, IntCounterVec, …
);

once_metric!(
    /// `efe_audit_chain_head_seq{chain_id}` — last committed seq per
    /// chain. Useful for monitoring chain growth + catching stalled
    /// chains (no new events for > X minutes).
    audit_chain_head_seq, IntGaugeVec, …
);

once_metric!(
    /// `efe_audit_sign_duration_seconds{action}` — wall-clock
    /// duration of one sign+insert.
    audit_sign_duration_seconds, HistogramVec, …
);

once_metric!(
    /// `efe_audit_keys_active` — number of HMAC keys loaded
    /// (active + verify-only). Sanity check : 0 = misconfiguration.
    audit_keys_active, IntGauge, …
);

once_metric!(
    /// `efe_audit_chain_break_detected_total` — cumulative count of
    /// chain breaks detected by the embedded verify (called
    /// periodically by the runtime). Fires the alert
    /// `D14bisAuditChainCompromised`.
    audit_chain_break_detected_total, IntCounter, …
);
```

### 17.2 Spans

```rust
// crates/efe-telemetry/src/spans.rs additions
pub const AUDIT_SIGN:   &str = "efe.audit.sign";
pub const AUDIT_VERIFY: &str = "efe.audit.verify";
```

### 17.3 Alerts

| Alert ID | Condition | Severity | Runbook |
|----------|-----------|:--------:|---------|
| **D14bisAuditSignFailing** | `rate(efe_audit_sign_failures_total[5m]) > 0` | 🔴 critical | `runbooks/d14bis_audit_sign_failing.md` |
| **D14bisAuditChainCompromised** | `efe_audit_chain_break_detected_total > 0` | 🔴 critical | `runbooks/d14bis_chain_compromised.md` |
| **D14bisAuditChainStalled** | `time() - efe_audit_chain_head_ts{chain_id="…"} > 86400` | 🟡 warning | `runbooks/d14bis_chain_stalled.md` |
| **D14bisAuditKeysMissing** | `efe_audit_keys_active == 0` | 🔴 critical | misconfiguration |

### 17.4 Periodic embedded verify

Le runtime peut **auto-vérifier** une fraction de la chaîne en
arrière-plan (cf. § 19) :

```rust
async fn periodic_self_verify(pool: PgPool, keys: Arc<HashMap<…>>) {
    let mut interval = tokio::time::interval(Duration::from_secs(3600));
    loop {
        interval.tick().await;
        for chain_id in list_active_chains(&pool).await {
            match verify_chain(&pool, &chain_id, &keys).await {
                Ok(rep) if rep.first_break_seq.is_none() => continue,
                Ok(rep) => {
                    tracing::error!(target: "efe.audit.hmac",
                        chain_id, break_seq = rep.first_break_seq.unwrap(),
                        "chain integrity compromised");
                    audit_chain_break_detected_total().inc();
                }
                Err(e) => tracing::error!(target: "efe.audit.hmac",
                    err = %e, "verify failed"),
            }
        }
    }
}
```

Self-verify horaire suffit pour détecter le tampering en moins
d'une heure.

---

## 18. Helm wiring — secret + env vars

### 18.1 `values.yaml` additions

```yaml
config:
  catalog:
    # ... existing fields ...
    audit:
      # =====================================================
      # Sprint D14-bis — HMAC-chained audit log signing.
      # =====================================================
      signing:
        # Enable HMAC chained signing of audit events. When false,
        # only the legacy `tracing::info!(target="audit", ...)` sink
        # is used (D12 baseline).
        enabled: false
        # Path to the keys file (1 line = 1 base64 key, first line
        # is the active signing key).
        keysFile: "/etc/efe/secrets/audit-hmac/hmac.keys"
        # Volume mount for the secret. The secret SHOULD be managed
        # by external-secrets / Vault for prod ; existingSecretName
        # is the Tier 2 path.
        existingSecretName: ""
        existingSecretKey: "hmac.keys"
        # Forward each event to the legacy tracing sink (Loki). On by
        # default for Tier 2/3 — preserves the existing Loki pipeline.
        chainToTracing: true
        # Self-verify interval. Set to 0 to disable.
        selfVerifyEverySeconds: 3600
```

### 18.2 ConfigMap rendering

```yaml
{{- if and (eq .Values.config.catalog.backend "postgres") .Values.config.catalog.audit.signing.enabled }}
EFE_AUDIT_SIGNING_ENABLED: "true"
EFE_AUDIT_HMAC_SECRETS_FILE: {{ .Values.config.catalog.audit.signing.keysFile | quote }}
EFE_AUDIT_CHAIN_TO_TRACING: {{ .Values.config.catalog.audit.signing.chainToTracing | quote }}
EFE_AUDIT_SELF_VERIFY_EVERY_S: {{ .Values.config.catalog.audit.signing.selfVerifyEverySeconds | quote }}
{{- end }}
```

### 18.3 Secret volume mount dans `deployment.yaml`

```yaml
{{- if and (eq .Values.config.catalog.backend "postgres") .Values.config.catalog.audit.signing.enabled .Values.config.catalog.audit.signing.existingSecretName }}
volumeMounts:
  - name: audit-hmac
    mountPath: {{ .Values.config.catalog.audit.signing.keysFile | dir | quote }}
    readOnly: true
volumes:
  - name: audit-hmac
    secret:
      secretName: {{ .Values.config.catalog.audit.signing.existingSecretName | quote }}
      items:
        - key: {{ .Values.config.catalog.audit.signing.existingSecretKey | default "hmac.keys" | quote }}
          path: hmac.keys
{{- end }}
```

### 18.4 Default `enabled: false`

D14-bis est **opt-in**. Les opérateurs sur memory backend ou ceux
qui n'ont pas besoin de tamper-evidence continuent sans changement.

Quand `enabled=true` mais `backend != postgres` → fail-fast au boot
(le sink HMAC nécessite le pool PG).

---

## 19. Runtime wire-up — `Server::build` augmenté

```rust
// runtime.rs — D14-bis additions

let catalog_audit_emitter = if config.catalog.audit_signing.enabled {
    if !matches!(config.catalog.backend, CatalogBackend::Postgres) {
        return Err(anyhow!(
            "EFE_AUDIT_SIGNING_ENABLED=true requires \
             EFE_CATALOG_BACKEND=postgres (HMAC sink uses the PG pool)",
        ));
    }
    let pool = pg_registry.pool().clone();
    let keys = load_hmac_keys(&config.catalog.audit_signing.keys_file)?;
    let sink = Arc::new(HmacChainedAuditSink::new(
        pool, keys, config.catalog.audit_signing.chain_to_tracing,
    ));
    if config.catalog.audit_signing.self_verify_every_s > 0 {
        tokio::spawn(periodic_self_verify(
            sink.pool().clone(), sink.keys().clone(),
            Duration::from_secs(config.catalog.audit_signing.self_verify_every_s),
            shutdown.clone(),
        ));
    }
    AuditEmitter::with_sink(sink)
} else {
    AuditEmitter::new()  // legacy tracing sink
};

// Inject into the registry. **Note** : we re-construct the registry
// with the new audit emitter so the trait sees the right sink.
let pg_registry = PostgresCatalogRegistry::from_pool(
    pg_registry.pool().clone(),
    catalog_policy.clone(),
    catalog_audit_emitter,
);
```

(légère restructure : on construit le pool d'abord, puis le
registry, puis le sink — pour réutiliser le pool.)

### 19.2 Order of init

1. Pool PG.
2. Migrations sqlx (`0001`, `0002`, `0003`).
3. Load HMAC keys (fail-fast if file missing/malformed).
4. Construct `HmacChainedAuditSink`.
5. Construct `PostgresCatalogRegistry` with the sink.
6. Spawn `periodic_self_verify` (optional).
7. Spawn `pool_metrics_ticker` (D14).

---

## 20. Tests strategy — ~30 nouveaux tests

### 20.1 Unit tests (`audit_hmac.rs::tests`)

| # | Test | Cible |
|--:|------|-------|
| 1 | `canonical_payload_is_deterministic` | round-trip serialise produces identical bytes |
| 2 | `canonical_payload_field_order_stable` | swap insertion order of BTreeMap → identical output |
| 3 | `compute_hmac_matches_rfc_test_vector` | RFC 4231 test vector check |
| 4 | `key_id_of_is_stable` | same key → same id |
| 5 | `key_id_of_distinguishes_keys` | different keys → different ids |
| 6 | `genesis_prev_hmac_is_64_zeros` | constant correctness |

### 20.2 Integration tests (`tests/d14bis_audit_hmac_integration.rs`, gated `EFE_PG_TEST_URL`)

| Section | # tests | Cible |
|---------|--------:|-------|
| A. Happy paths | 4 | sign register / sign evolve / sign delete / multi-event chain |
| B. Concurrency | 3 | concurrent register on same chain → distinct seq, monotonic prev_hmac chain ; cross-chain registers parallel ; spawn race scenarios |
| C. Verify | 5 | verify intact chain ; verify after tampered hmac ; verify after deleted row ; verify after reordered seq (cant happen in append-only, but verify a synthetic dump JSONL) ; verify multi-key chain |
| D. Append-only enforcement | 4 | UPDATE rejected by trigger ; DELETE rejected ; TRUNCATE rejected ; trigger NOT bypassed by superuser unless DROP TRIGGER |
| E. Key rotation | 3 | rotate key in flight ; verify chain spanning rotation ; reject decommissioned key |
| F. Periodic self-verify | 2 | self-verify detects tampering ; counter increments |
| G. Performance | 3 | sign p99 ≤ 10 ms ; verify 10⁶ events ≤ 10 s ; pool not exhausted under load |
| H. Edge cases | 3 | empty description event ; long description (10 KB) ; UTF-8 description |

Total : **~27 integration tests + 6 unit tests = ~33**.

### 20.3 Python pin tests

`scripts/tests/test_d14bis_audit_hmac.py` (~15 tests) :

* Helm `values.yaml` audit.signing block structure.
* ConfigMap rendering conditional on `enabled=true`.
* Deployment Secret volume mount.
* DDL `audit_events` schema invariants (PRIMARY KEY, triggers,
  CHECK constraints).
* Spans constants present in `spans.rs`.
* 6 D14-bis instruments declared in `d14bis_instruments.rs`.
* Runtime wire-up tokens : `HmacChainedAuditSink`, `load_hmac_keys`,
  `periodic_self_verify`.

---

## 21. Threat model détaillé — 8 vecteurs

(détaillé § 3.1 ; ici les fixes en code)

| # | Vecteur | Mitigation D14-bis | Test |
|--:|---------|--------------------|------|
| 1 | Adversaire DBA UPDATE/DELETE rows audit_events | Trigger `audit_events_block_mutation` | Section D tests |
| 2 | Adversaire DBA TRUNCATE | Trigger BEFORE TRUNCATE | Section D tests |
| 3 | Adversaire RCE compromise live signing | Counter `audit_sign_failures_total` + alerts → ops rotates the key | Section A tests + alert smoke |
| 4 | Reorder via re-INSERT après DELETE | Append-only trigger empêche DELETE ; chaîne brisée si DELETE force-bypassed | Section C tests (synthetic dump) |
| 5 | Replay (re-emit ancien event) | event_id UNIQUE → INSERT fail with 23505 | Section A test happy path duplicate |
| 6 | Time-skew (NTP attack) | created_at_unix_ms est dans le HMAC payload ; replay impossible | Section H test |
| 7 | Tampering du `canonical_payload` JSONB | Le HMAC est sur ce payload ; verify recalcule et refuse | Section C tests |
| 8 | Key compromise (clé HMAC fuit) | Key rotation rapide via `EFE_AUDIT_HMAC_SECRETS_FILE` ; events HISTORIQUES restent intègres tant que la clé était secrète à leur signature | Section E tests |

---

## 22. Decisions log — récap des décisions clés

| # | Décision | Alternatives | Rationale |
|--:|----------|--------------|-----------|
| 1 | HMAC-SHA-256 chained | Merkle tree, Ed25519, BLAKE3-keyed | Alignement crypto avec D11/D12 ; perf ; FIPS 140-3 ; verify-by-3rd-party = D14-ter |
| 2 | Algorithme HMAC-SHA-256 (RFC 2104) | H(key‖msg) naïf, KMAC | Standard ; resistant length-extension |
| 3 | Storage : table SQL `audit_events` | JSONL append, in-line catalogs | Append-only via trigger ; backup native ; queryable |
| 4 | Granularité : per-tenant chain | global, mixte | Aligned D11 ; verify isolated par tenant ; chain_id forward-compat global |
| 5 | Canonical : JSON sorted keys minified | JCS RFC 8785, CBOR canonical | Déterministe ; debug-friendly ; pas de dep crate |
| 6 | UUIDv7 pour event_id | UUIDv4, ULID | Timestamp-monotone (utile pour streaming verify + secondary index) |
| 7 | Spawn async pour `emit()` | bloquer sync | Le caller a déjà commit ; audit best-effort + alert |
| 8 | Self-verify horaire | self-verify chaque event, jamais | Trade-off détection vs coût ; 1 h détecte tampering rapide sans charge |
| 9 | `enabled: false` par défaut Helm | `enabled: true` | Opt-in : déploiements Memory ne paient pas le coût |
| 10 | `efe-verify-audit` CLI | library only | Ergonomie ops + reproductibilité audit régulateur |

---

## 23. Risques + mitigations

| ID | Sévérité | Risque | Mitigation |
|----|:--------:|--------|------------|
| **R-D14bis.1** | 🔴 HIGH | Crash entre `tx.commit()` catalog et `audit_events INSERT` → mutation sans audit | Job de réconciliation D15 ; alert sur `efe_audit_lag_total` (catalogs INSERTed - audit_events INSERTed) |
| **R-D14bis.2** | 🔴 HIGH | Clé HMAC compromise live | Rotation immédiate + monitor `key_id` distribution dans audit_events |
| **R-D14bis.3** | 🟠 MED | Self-verify failure miss tampering between ticks | Réduire `selfVerifyEverySeconds` ou ajouter trigger-based verify |
| **R-D14bis.4** | 🟠 MED | Pool PG saturé par audit traffic sous charge | Pool sizing tuning ; D14 baseline + 50 % marge |
| **R-D14bis.5** | 🟡 LOW | UUID v7 collision | Probabilité 2⁻⁶² par event — négligeable |
| **R-D14bis.6** | 🟡 LOW | Append-only trigger DROPped par DBA inattention | `pgaudit` extension externe pour tracker les DDL ; ops process |
| **R-D14bis.7** | 🟡 LOW | Verify CLI `efe-verify-audit` mal utilisé (mauvais keys-file → false negative) | Documentation runbook + `key_id` distribution check |
| **R-D14bis.8** | 🟢 INFO | Schema migration `0003_*` corruption mid-flight | sqlx migrate transaction wrap ; idempotent re-run |

---

## 24. Plan wallclock — 4 j

| Jour | Activités | Output |
|-----:|-----------|--------|
| **J1** | DSN audit signing config + Helm rendering + key loader + 1 unit test sign | `config.rs::AuditSigningConfig` + `values.yaml::audit.signing` + `audit_hmac.rs::load_hmac_keys` |
| **J2** | Migration `0003_audit_events.sql` + `HmacChainedAuditSink` + spawn + 8 integration tests happy paths + concurrency | `audit_hmac.rs` ~600 lignes + tests verts |
| **J3** | Verify library + `efe-verify-audit` CLI + 5 verify tests + key rotation tests | `audit_hmac::verify_chain` + `bin/efe-verify-audit.rs` ~250 lignes + tests verts |
| **J4** | Periodic self-verify + observability (6 instruments) + alerts + Helm Secret + Python pin tests + doc + audit interne (skeleton) | release v0.1.14.5 RC + `docs/audits/d14bis_audit_report.md` skeleton |
| **+0.5** | Buffer audit interne (pattern D8/D12-bis/D14 : ~5-7 bugs attendus) | corrections post-audit |

**Total : 4 j wallclock + 0.5 j buffer.**

---

## 25. SLO ajoutés en D14-bis

| ID | Description | Cible | Type |
|----|-------------|------:|------|
| **D14bis-AUD-1** | `register` p99 sans regression vs D14 (audit spawn off critical path) | ≤ 22 ms (D14 baseline + 10 %) | GATE |
| **D14bis-AUD-2** | `audit_sign_duration_seconds` p99 (sign + insert) | ≤ 10 ms | GATE |
| **D14bis-AUD-3** | Verify 10⁶ events streaming | ≤ 10 s | OBSERVATIONAL |
| **D14bis-AUD-4** | `audit_sign_failures_total` rate sustained 5 min | == 0 | ALERT |
| **D14bis-AUD-5** | Self-verify cycle complete every chain | ≤ 1 h | OBSERVATIONAL |
| **D14bis-AUD-6** | Append-only invariant : UPDATE/DELETE rejected | 100 % | INVARIANT |
| **D14bis-AUD-7** | Chain integrity verifiable end-to-end | 100 % | INVARIANT |

---

## 26. Anti-régression D14-bis

```bash
# Lib (helpers + new audit_hmac module) — 266 + ~6 = ~272 verts
cargo test -p efe-server --lib

# D14-bis integration tests gated EFE_PG_TEST_URL — 27 tests
docker run --rm -d --name efe-test-pg \
    -e POSTGRES_USER=efe -e POSTGRES_PASSWORD=efe \
    -e POSTGRES_DB=efe_catalog_test -p 5433:5432 postgres:15
EFE_PG_TEST_URL='postgres://efe:efe@127.0.0.1:5433/efe_catalog_test' \
    cargo test -p efe-server --test d14bis_audit_hmac_integration

# CLI smoke test
EFE_PG_TEST_URL=… cargo run --bin efe-verify-audit -- \
    --chain tenant:acme --keys-file /tmp/test-keys

# Cross-sprint regression
PYTHONPATH=clients/python python3 -m pytest scripts/tests/ -q
cargo test -p efe-server
```

Cibles :
- Tests Rust D14-bis : 33 verts.
- Tests Python D14-bis : ~15 verts.
- 0 régression sur D8/D11/D12/D12-bis/D13/D14.

---

## 27. Hand-off vers D15 + D14-ter

D14-bis ouvre 2 sprints futurs :

* **D15 (compaction + réconciliation)** :
  - Job réconciliation `audit_events` ↔ `catalogs` pour détecter
    les mutations sans audit (R-D14bis.1).
  - Compaction des vieilles versions `catalogs` + archival des
    audit chunks anciens vers cold storage (S3 + Object Lock).
  - Decommissioning de keys HMAC obsolètes.
* **D14-ter (Ed25519 verifiable-by-third-party)** :
  - Ajouter une signature Ed25519 par-dessus le HMAC ; la pub key
    est publiée (e.g. via DNS TXT + transparency log).
  - Régulateur peut verify l'audit sans accès clé HMAC ni infra
    interne.
  - Cf. `docs/bench/d14ter_ed25519_audit_architecture.md`
    (à écrire).

---

## 28. Annexes

### Annexe A — DDL complet (cf. § 10.1)

→ `crates/efe-server/migrations/0003_audit_events.sql` (post-impl).

### Annexe B — `EFE_AUDIT_*` env vars matrix

| Env var | Default | Helm key |
|---------|---------|----------|
| `EFE_AUDIT_SIGNING_ENABLED` | `false` | `config.catalog.audit.signing.enabled` |
| `EFE_AUDIT_HMAC_SECRETS_FILE` | `/etc/efe/secrets/audit-hmac/hmac.keys` | `config.catalog.audit.signing.keysFile` |
| `EFE_AUDIT_CHAIN_TO_TRACING` | `true` | `config.catalog.audit.signing.chainToTracing` |
| `EFE_AUDIT_SELF_VERIFY_EVERY_S` | `3600` | `config.catalog.audit.signing.selfVerifyEverySeconds` |

### Annexe C — RFC test vectors

`hmac-sha256` test vectors from RFC 4231 §4.2 :
- key = 0x0b × 20
- data = "Hi There"
- expected hmac = b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7

→ `tests/d14bis_audit_hmac_integration.rs::compute_hmac_matches_rfc_test_vector`.

### Annexe D — Bibliographie

- **RFC 2104** HMAC : <https://datatracker.ietf.org/doc/html/rfc2104>
- **RFC 4231** HMAC test vectors : <https://datatracker.ietf.org/doc/html/rfc4231>
- **NIST SP 800-117** HMAC : <https://csrc.nist.gov/publications/detail/sp/800-117/rev-1/final>
- **NIST 800-53 AU-9** Protection of Audit Information : <https://csrc.nist.gov/Projects/risk-management/sp800-53-controls/release-search>
- **GDPR Article 32** Security of processing : <https://gdpr-info.eu/art-32-gdpr/>
- **HIPAA §164.312(b)** Audit Controls : <https://www.hhs.gov/hipaa/for-professionals/security/laws-regulations/index.html>

---

> **Architecture finalisée** — 2026-04-27.
> **Prochaine étape** : J1 du plan wallclock (config + Helm + key loader).
> **Audit interne D14-bis** : à programmer en J5 selon pattern D8/D12-bis/D14.
