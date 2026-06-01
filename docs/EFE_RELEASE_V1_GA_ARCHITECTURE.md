# EFE Release v1.0.0 GA — Architecture Complète (Sigstore + Tag + Announce)

**Codename** : « R-100 » (Release 1.0.0)
**Status** : Architecture v1.0 — design freeze
**Auteur** : Architecture EFE Core
**Date** : 2026-05-09
**Pré-requis** : M16-M22 GA + GLOSS A+B+C complet (audit pass chirurgical Sprint C validé : portal build clean, Terraform tfsec clean, Helm regression OK, 3 297 tests GREEN)
**Successeur** : M23 (DPO Export + Active Learning) post-GA

---

## §0. Executive Summary

### 0.1 Mission

Le release v1.0.0 GA marque la **transition d'EFE de produit interne (build artisanal sur laptops d'ingénieurs) à produit signé, distribué, supporté contractuellement, certifiable**. L'architecture R-100 livre les **trois piliers de release** qui transforment le code aujourd'hui présent dans le repo en :

1. **Artefacts signés Sigstore + Rekor + SLSA-3 provenance** : tout ce qui est shippable (Docker images × 4, Helm chart, CLI binaries × 6 cibles, Terraform modules × 3, npm package portal, Cargo crates) est cryptographiquement vérifiable end-to-end depuis le commit hash jusqu'au binaire installé chez le tenant.

2. **Git tag + distribution channels** : `v1.0.0` poussé sur main, OCI registry public, GitHub Releases, Homebrew tap, npm registry, Cargo registry — chacun avec checksums, signatures, et instructions de vérification.

3. **Announce & compliance package** : blog post, customer email, partner notifications, security disclosure feed (RSS + email), package compliance prêt-à-soumettre (SOC 2 evidence, GDPR Art. 30 register, AI Act Art. 11 technical documentation).

### 0.2 Trois piliers et leurs interactions

```
┌──────────────────────────────────────────────────────────────────────┐
│                   Release v1.0.0 GA — flow d'ensemble                │
│                                                                       │
│  [main branch HEAD]                                                  │
│       │                                                               │
│       │ 1. Pre-release validation                                    │
│       ▼                                                               │
│  [CI: full test matrix] → 3 297 tests GREEN                          │
│       │                                                               │
│       │ 2. Tag v1.0.0                                                │
│       ▼                                                               │
│  [git tag v1.0.0]                                                    │
│       │                                                               │
│       │ 3. Build + sign artifacts (Pilier 1)                         │
│       ├──► Docker images (efe-server, mcp-server, portal-admin,      │
│       │                   efe-server-migrate)                        │
│       │     └─► cosign sign + Rekor inclusion proof + SLSA-3         │
│       │                                                               │
│       ├──► Helm chart efe-stack-1.0.0.tgz                            │
│       │     └─► helm package + cosign sign-blob                      │
│       │                                                               │
│       ├──► CLI binaries (linux/darwin/windows × amd64/arm64)         │
│       │     └─► cosign sign-blob each                                │
│       │                                                               │
│       ├──► Terraform modules tagged tf-{aws,gcp,azure}/v1.0.0        │
│       │     └─► git tag signed + SBOM Syft                           │
│       │                                                               │
│       └──► npm package @efe/portal-admin (Sprint C dist)             │
│             └─► npm publish --provenance (SLSA via npmjs)            │
│                                                                       │
│       │ 4. Distribute (Pilier 2)                                     │
│       ▼                                                               │
│  [Distribution channels]                                             │
│   - oci://ghcr.io/efe-ai/charts/efe-stack:1.0.0                     │
│   - oci://ghcr.io/efe-ai/efe-server:1.0.0                           │
│   - GitHub Releases v1.0.0 (binaries + checksums + signatures)      │
│   - Homebrew tap : brew install efe/tap/efe-admin                    │
│   - npm : @efe/portal-admin@1.0.0                                    │
│   - registry.terraform.io/efe-ai/efe-stack-{aws,gcp,azure}/v1.0.0   │
│                                                                       │
│       │ 5. Announce + compliance pack (Pilier 3)                     │
│       ▼                                                               │
│  [Communications]                                                    │
│   - Blog post efe.example.com/blog/v1-0-0-ga                        │
│   - Customer email (existing tenants)                                │
│   - Partner notification (system integrators)                        │
│   - Security disclosure RSS / vex feed                               │
│   - Compliance evidence pack downloadable for DPO                    │
└──────────────────────────────────────────────────────────────────────┘
```

### 0.3 Scope synthétique

| Pilier | Output | Effort | LOC / livrables |
|--------|--------|--------|------------------|
| Sigstore + provenance | `.github/workflows/release.yml` + `cosign-policy.yaml` + `slsa-policy.yaml` + `verify-release.sh` | 2 j | ~ 1 600 LOC + scripts |
| Tag + distribution | git tag, GitHub release notes, OCI push, Homebrew formula, npm publish, Terraform registry submit | 2 j | ~ 800 LOC + automation |
| Announce + compliance | Blog post + customer email template + compliance pack (SOC 2 evidence + GDPR Art. 30 + AI Act Art. 11) | 3 j | ~ 4 200 LOC docs |
| Verification harness | Script `efe-verify-release.sh` qui vérifie un déploiement complet end-to-end | 1 j | ~ 600 LOC |
| **Total R-100** | | **8 j** | **~ 7 200 LOC** |

### 0.4 Critères d'acceptation R-100 (DoD synthèse)

1. ✅ Tous les artefacts (4 Docker images + 1 Helm chart + 6 CLI binaries + 3 Terraform modules + 1 npm package) signés Sigstore avec Rekor inclusion proof
2. ✅ SLSA-3 provenance attached à chaque artefact (build provenance includes commit SHA + workflow URL)
3. ✅ `cosign verify` ET `slsa-verifier` réussissent sur 100 % des artefacts (validation script `verify-release.sh`)
4. ✅ Git tag `v1.0.0` signé GPG + sub-tags `tf-aws/v1.0.0`, `tf-gcp/v1.0.0`, `tf-azure/v1.0.0`, `efe-admin/v1.0.0`, `portal-admin/v1.0.0`
5. ✅ GitHub Release créée avec changelog automatisé + checksums SHA-256 + ascii-armored cosign signatures
6. ✅ OCI registry contient les 4 images + Helm chart, accessibles publiquement (`docker pull`, `helm pull oci://...`)
7. ✅ Homebrew tap publié : `brew install efe/tap/efe-admin` réussit
8. ✅ npm package publié avec `--provenance` : `npm install @efe/portal-admin` réussit
9. ✅ Terraform modules submitted to Terraform Registry (3 modules, version 1.0.0 visible)
10. ✅ Blog post publié sur `https://efe.example.com/blog/v1-0-0-ga`
11. ✅ Customer email envoyé aux tenants existants (≥ 100 % delivery rate)
12. ✅ Compliance pack downloadable depuis portal (`/compliance/download-package`) — contient SOC 2 readiness, GDPR Art. 30, AI Act Art. 11
13. ✅ `efe-verify-release.sh` : script E2E qui vérifie un déploiement v1.0.0 complet sur k3d local + cloud (AWS, GCP, Azure) en ≤ 60 min
14. ✅ Security advisory feed RSS + VEX (Vulnerability Exploitability eXchange) initialisé
15. ✅ Release notes signées GPG par 2 maintainers EFE Core

### 0.5 Garanties cardinales R-100

- **Reproducible builds** : `docker build` + `cargo build --release --locked` + `pnpm build` produisent des artefacts byte-for-byte identiques pour un même commit SHA (vérifié via SLSA provenance)
- **Tamper-evident chain** : Sigstore `cosign verify` + Rekor inclusion proof = preuve cryptographique que l'artefact a été produit par le workflow CI EFE et n'a pas été modifié
- **No long-lived secrets** : signature via OIDC keyless (GitHub Actions OIDC token → ephemeral cert), pas de clés GPG persistantes dans CI
- **Backward compatible** : v1.0.0 est compatible avec `efe-admin v0.0.1` (le CLI Sprint A) et n'introduit aucune migration breaking — un tenant existant peut faire `helm upgrade` sans perte de données
- **Traçabilité supply chain** : pour tout artefact installé chez un tenant, on peut remonter en ≤ 3 commandes au commit Git source

---

## §1. Contexte & justification

### 1.1 État actuel post-Sprint C

Le repo contient :

| Composant | Statut tests | Path |
|-----------|--------------|------|
| `efe-server` (Rust gRPC backend) | 2 426 tests | `crates/efe-server/` |
| `efe-admin` (Rust CLI) | 54 tests | `crates/efe-admin/` |
| `mcp-server` (Python HTTP tooling) | 620 tests | `mcp-server/` |
| `portal-admin` (Next.js 14 SPA) | 47 tests Vitest + 9 Playwright × 3 browsers | `apps/portal-admin/` |
| Helm chart `efe-stack` (umbrella + 6 sub-charts) | 25 helm-unittest + kubeconform 4 profils | `charts/efe-stack/` |
| Terraform AWS module | 8 terratest | `terraform/modules/aws/` |
| Terraform GCP module | 2 terratest | `terraform/modules/gcp/` |
| Terraform Azure module | 2 terratest | `terraform/modules/azure/` |
| 12 Runbooks RB-01 à RB-12 | 12 validation procedures | `docs/runbooks/` |

**Mais** sans R-100, ces composants sont :
- ❌ Non signés (pas de garantie d'origine ni d'intégrité)
- ❌ Non distribués (un nouveau tenant doit cloner le repo, build, push images privées)
- ❌ Non accompagnés de compliance evidence (DPO du tenant doit reconstituer la chaîne d'audit lui-même)
- ❌ Non versionnés (tout est à la HEAD de main, pas de notion de release pinnée)

R-100 résout ces 4 points en livrant la **release pipeline complète**.

### 1.2 Pourquoi Sigstore plutôt que GPG / X.509

**Comparaison signing technologies** :

| Choix | Pour | Contre | Verdict |
|-------|------|--------|---------|
| GPG long-lived keys | Familier, simple | Clés stockées dans CI (risque de leak) ; revocation lourd ; chaîne de confiance opaque ; hard-coded dans build |  ❌ |
| X.509 (PKI traditionnelle) | Standard reconnu enterprise | Coût CA ($1k/year minimum) ; rotation manuelle ; pas de transparency log | ❌ |
| **Sigstore** (cosign + Rekor + Fulcio) | OIDC keyless ; transparency log ; ephemeral certs ; SLSA-3 native ; gratuit (CNCF) ; standard de fait 2024+ | Dépendance externe (Fulcio + Rekor publics) → mitigé par TUF root pinning | ✅ |
| Linux Foundation in-toto | Très complet, supply-chain end-to-end | Complexité d'install, écosystème plus petit | Sur-dimensionné pour R-100 |

**Décision** : **Sigstore** pour 4 raisons :
1. **OIDC keyless** : signature via le token OIDC du runner CI (GitHub Actions, GitLab CI, etc.) → **0 clé persistante** stockée dans secrets
2. **Rekor transparency log** : toute signature est append-only loggued publiquement, on peut prouver après-coup quelle version fut publiée à quelle date
3. **SLSA-3 native support** : cosign + npm + GitHub Actions ont des intégrations directes pour SLSA provenance
4. **Industrie standard 2024+** : Kubernetes, Helm, npm registry, Docker Hub utilisent tous Sigstore. Compatible avec les workflow CI existants chez nos clients enterprise.

### 1.3 Pourquoi git tag signé + sous-tags pluriels

EFE est un **monorepo** avec plusieurs composants à versioning indépendant :
- `efe-server` core et `mcp-server` versionnent ensemble (couplage tight via gRPC schema)
- `efe-admin` CLI versionne suivant `efe-server` (couplage gRPC client)
- Helm chart `efe-stack` versionne en semver indépendant
- Terraform modules `tf-aws`, `tf-gcp`, `tf-azure` versionnent **chacun indépendamment** (refactor cloud-spécifique sans impacter les autres)
- Portal Next.js versionne avec efe-server (couplage UI ↔ gRPC)

D'où la stratégie **multi-tag** :

```
git tag v1.0.0                    # release master, signe par 2 maintainers
git tag tf-aws/v1.0.0              # AWS module
git tag tf-gcp/v1.0.0              # GCP module
git tag tf-azure/v1.0.0            # Azure module
git tag efe-admin/v1.0.0           # CLI binary distribution
git tag portal-admin/v1.0.0        # npm + Docker image
git tag charts/efe-stack/v1.0.0    # Helm chart
```

Cela permet :
- À un tenant AWS de track `tf-aws/v1.0.0` sans subir les changements GCP
- À une équipe ops qui n'utilise que le Helm chart de track `charts/efe-stack/v1.0.0`
- À un développeur qui n'utilise que le CLI de track `efe-admin/v1.0.0`

Le tag master `v1.0.0` est **agrégé** et pointe sur le commit qui produit toutes les sous-versions cohérentes.

### 1.4 Pourquoi distribution multi-channel

| Persona | Channel préféré | Distribution |
|---------|-----------------|--------------|
| SE EFE / DevOps tenant | OCI registry (Helm pull) | `oci://ghcr.io/efe-ai/charts/efe-stack:1.0.0` |
| Tenant cloud-native | Helm + Terraform Registry | `helm install` + `terraform init` |
| Développeur Mac/Linux | Homebrew | `brew install efe/tap/efe-admin` |
| Développeur Windows | Chocolatey / GitHub Release zip | `choco install efe-admin` ou direct download |
| Frontend dev (sub-app) | npm | `npm install @efe/portal-admin` |
| Air-gapped enterprise | GitHub Release `.tar.gz` | direct download + cosign verify offline |
| AI agents (LLM tools) | mcp-server already in stack | via Helm chart |

**6 channels** = couverture quasi-totale des cas d'usage. Coût : automation CI dans `release.yml`. ROI : un nouveau tenant peut découvrir + installer en 5 min via le channel qu'il connaît déjà, **sans contacter EFE Core**.

### 1.5 Anti-patterns à éviter R-100

#### 1.5.1 Anti-pattern : « big bang release »

❌ **Mauvais** : tag main, build tout à la fois, push tout à la fois, et croiser les doigts. Si un step rate, on est coincé entre 2 états.

✅ **Bon** : pipeline en **5 phases atomiques**, chacune réversible :
1. **Validate** : tests + audit + sec scan (read-only)
2. **Build & sign** : produit artefacts en sandbox, ne push rien
3. **Verify** : `cosign verify` + `slsa-verifier` sur les artefacts produits localement (preview)
4. **Distribute** : push vers chaque channel séquentiellement avec rollback hooks
5. **Announce** : blog + email + RSS

Si phase 4 rate au milieu : on a un état partiel mais traçable, et un script `release-rollback.sh` peut yank les artefacts publiés.

#### 1.5.2 Anti-pattern : « release notes auto-générées seules »

❌ **Mauvais** : `git log v0.x..v1.0.0` brut comme release notes. Lecteur (DPO, ops, dev) n'y trouve pas les infos qu'il cherche.

✅ **Bon** : release notes structurées par **persona-target** :
- Section « Pour les DPO » : compliance changes, attestation flow, right-to-explanation
- Section « Pour les SE / DevOps » : breaking changes Helm/Terraform, migration path
- Section « Pour les développeurs (CLI / SDK) » : nouvelles commandes, breaking changes args
- Section « Sécurité » : CVE fixés, signatures Sigstore, SLSA level
- Section « Performance » : benchmarks deltas
- Section « Roadmap » : ce qui arrive ensuite (M23, M24, M25)

#### 1.5.3 Anti-pattern : « compliance evidence à la demande »

❌ **Mauvais** : « Si tu veux le SOC 2 evidence, envoie un email à compliance@efe.com et attends 5 jours ». Friction commerciale énorme.

✅ **Bon** : **package compliance pré-généré** disponible en :
- Téléchargement immédiat depuis le portail (`/compliance/download-package` → ZIP signed)
- Auto-régénération mensuelle (les attestations changent)
- Format standard : SOC 2 evidence template + GDPR Art. 30 register + AI Act Art. 11 documentation
- Signature interne (HMAC du contenu pré-CGU) + signature externe (cosign)

### 1.6 Glossaire R-100

| Terme | Définition |
|-------|------------|
| **Sigstore** | Stack de signing supply chain : cosign (CLI) + Fulcio (CA éphémère) + Rekor (transparency log) |
| **Cosign** | CLI Sigstore pour signer + vérifier artefacts (containers, blobs) |
| **Fulcio** | Certificate Authority éphémère qui émet certs X.509 short-lived (10 min) bound à OIDC identity |
| **Rekor** | Append-only transparency log — toute signature publiée y est loggued publiquement |
| **SLSA** | Supply chain Levels for Software Artifacts — framework Google/CNCF |
| **SLSA-3** | Niveau 3 : provenance signed + non-falsifiable + isolated build env |
| **VEX** | Vulnerability Exploitability eXchange — format JSON pour communiquer si une CVE affecte vraiment notre code |
| **TUF** | The Update Framework — pinning des roots de confiance |
| **OCI** | Open Container Initiative — format/registry de containers (Docker = implémentation) |
| **GA** | Generally Available — release stable, supportée commercialement |
| **CGU** | Conditions Générales d'Utilisation |
| **DPO** | Data Protection Officer (RGPD Art. 37) |
| **Yank** | Désindexer un artefact d'un registry (irréversible) |

---

## §2. Architecture cible globale

### 2.1 Layout du repo après R-100

```
efe/
├── .github/workflows/
│   ├── release.yml                    # NEW — pipeline R-100 complète
│   ├── release-rollback.yml           # NEW — yank en cas d'incident
│   ├── ci.yml                          # carry-over (PR gates)
│   ├── helm-stack-test.yml             # carry-over
│   ├── terraform-{aws,gcp,azure}.yml   # carry-over
│   └── portal-admin-test.yml           # carry-over
├── .sigstore/
│   ├── policy.yaml                     # NEW — cosign verification policy
│   ├── trusted-root.json               # NEW — TUF pinned root
│   └── README.md                       # NEW
├── docs/release/                       # NEW
│   ├── RELEASE_NOTES_v1.0.0.md
│   ├── RELEASE_PROCESS.md
│   ├── DISTRIBUTION_CHANNELS.md
│   ├── VERIFY_RELEASE.md
│   ├── COMPLIANCE_PACKAGE.md
│   └── SECURITY_DISCLOSURE_POLICY.md
├── docs/compliance/                    # NEW
│   ├── SOC2_EVIDENCE_TEMPLATE.md
│   ├── GDPR_ART30_REGISTER.md
│   ├── AI_ACT_ART11_TECHNICAL_DOC.md
│   ├── ISO_27001_READINESS.md
│   └── HIPAA_BAA_TEMPLATE.md (post-R-100)
├── docs/marketing/                     # NEW
│   ├── BLOG_POST_v1_0_0_GA.md
│   ├── CUSTOMER_EMAIL_TEMPLATE.md
│   ├── PARTNER_NOTIFICATION.md
│   └── PRESS_RELEASE.md
├── scripts/release/
│   ├── verify-release.sh               # NEW — E2E verification
│   ├── release-prepare.sh              # NEW — pre-flight checks
│   ├── release-publish.sh              # NEW — orchestration
│   ├── release-rollback.sh             # NEW — yank artifacts
│   └── generate-compliance-pack.sh     # NEW
├── homebrew/
│   └── efe-admin.rb                    # NEW — Homebrew formula
├── tools/sigstore/
│   ├── cosign-policy.yaml              # NEW
│   └── slsa-policy.yaml                # NEW
└── CHANGELOG.md                        # carry-over (auto-generated)
```

### 2.2 Diagramme dépendances pipeline R-100

```
┌──────────────────────────────────────────────────────────────────────┐
│                     Release v1.0.0 Pipeline                          │
│                                                                       │
│  Phase 1 : Validate (parallel, ~10 min)                              │
│  ─────────────────────────────────────                               │
│   ├─ cargo test --workspace --all-features                           │
│   ├─ pytest mcp-server/tests/                                        │
│   ├─ pnpm test:unit + test:e2e (× 3 browsers)                       │
│   ├─ helm-unittest + helm template + kubeconform                     │
│   ├─ terratest AWS + GCP + Azure (parallel, à vide)                  │
│   ├─ cargo audit + pnpm audit + Trivy scan                           │
│   ├─ tfsec × 3 modules                                               │
│   └─ runbook-validate.sh (against staging)                           │
│                                                                       │
│  Phase 2 : Build & sign (parallel, ~25 min)                          │
│  ──────────────────────────────────────                              │
│   ├─ Docker images × 4 (Buildkit, cache mount, SBOM Syft)            │
│   │  └─ cosign sign (keyless OIDC) + Rekor entry + SLSA provenance   │
│   ├─ Helm chart .tgz                                                 │
│   │  └─ helm package + cosign sign-blob + push OCI                   │
│   ├─ CLI binaries × 6 (linux/darwin/windows × amd64/arm64)           │
│   │  └─ cargo build --release + cosign sign-blob each                │
│   ├─ npm package portal-admin                                        │
│   │  └─ npm publish --provenance (SLSA via npmjs)                    │
│   └─ Terraform modules tagged                                        │
│      └─ git tag signed (per module) + SBOM Syft                      │
│                                                                       │
│  Phase 3 : Verify (sequential, ~5 min)                               │
│  ─────────────────────────────────                                   │
│   ├─ cosign verify each Docker image (against policy.yaml)           │
│   ├─ cosign verify Helm chart blob                                   │
│   ├─ slsa-verifier each artifact                                     │
│   ├─ check Rekor inclusion proof                                     │
│   └─ verify-release.sh against ephemeral k3d cluster                 │
│                                                                       │
│  Phase 4 : Distribute (sequential, ~10 min)                          │
│  ────────────────────────────────────                                │
│   ├─ Push OCI registry (4 images + Helm chart)                       │
│   ├─ Create GitHub Release v1.0.0 + sub-releases                     │
│   ├─ Update Homebrew tap (automated PR)                              │
│   ├─ npm publish (already in Phase 2 if dry-run, else publish here)  │
│   └─ Submit Terraform modules to Registry                            │
│                                                                       │
│  Phase 5 : Announce (~30 min, partly automated)                      │
│  ─────────────────────────────────                                   │
│   ├─ Blog post auto-published from CHANGELOG (CMS API)               │
│   ├─ Customer email triggered (mailgun batch)                        │
│   ├─ Partner notifications (Slack webhook + email)                   │
│   ├─ Twitter / LinkedIn / discussion forum posts                     │
│   ├─ Security advisory RSS feed update                               │
│   └─ Compliance pack regenerated + uploaded to portal                │
│                                                                       │
│  Total : ~ 1h 20 min from `git tag` to fully announced               │
└──────────────────────────────────────────────────────────────────────┘
```

### 2.3 Boundaries & responsibilities

| Composant | Owner | Responsabilité |
|-----------|-------|----------------|
| `release.yml` workflow | EFE Release Engineer | Orchestre les 5 phases |
| `cosign-policy.yaml` | EFE Security Lead | Définit qui peut signer + ce qui est valide |
| `verify-release.sh` | EFE QA Lead | E2E verification after distribution |
| `RELEASE_NOTES_v1.0.0.md` | Engineering Manager | Manuel curation + auto-changelog |
| Customer email | Marketing + Account Executives | Drafting + sending |
| Partner notification | Partnerships team | List management + Slack channels |
| Compliance pack | DPO + Compliance Engineer | Templates + auto-regen monthly |

### 2.4 Versioning

```
v1.0.0
  ├─ tf-aws/v1.0.0          (Terraform AWS module)
  ├─ tf-gcp/v1.0.0          (Terraform GCP module)
  ├─ tf-azure/v1.0.0        (Terraform Azure module)
  ├─ efe-admin/v1.0.0       (CLI Rust binary)
  ├─ portal-admin/v1.0.0    (Next.js portal + npm package)
  └─ charts/efe-stack/v1.0.0 (Helm chart)
```

Toutes les sous-versions partagent le même commit SHA pour `v1.0.0`. Les **sous-tags** sont posés au **même commit** mais permettent aux outils tiers (Terraform Registry, Helm OCI, npm) de track indépendamment.

#### 2.4.1 Semver policy v1.0+

Une fois v1.0.0 publié, EFE entre en mode **strict semver** :

| Type de changement | Bump | Exemples |
|--------------------|------|----------|
| Breaking change API gRPC / mcp-server tools | MAJOR (v2.0.0) | renommage tool, suppression field proto |
| Nouvelle feature backward-compat | MINOR (v1.1.0) | M23 DPO export, nouveau provider LLM |
| Bug fix uniquement | PATCH (v1.0.1) | fix deadlock, fix typo doc |
| Security fix | PATCH avec embargo | CVE fixe + advisory RSS |

**Hotfix policy** : pour la branche `release/v1.x` on cherry-pick les patches PATCH-only ; les fixes MINOR vont sur `main` et release ultérieur.

---

## §3. Pilier 1 — Sigstore signing & SLSA provenance

### 3.1 Vue d'ensemble

Tout artefact produit par le pipeline R-100 est :
- **Signé** par `cosign` (signature keyless via GitHub Actions OIDC)
- **Loggé** dans Rekor (transparency log public)
- **Accompagné** d'une attestation SLSA-3 provenance (origin commit SHA, builder identity, build params)

Le signing repose sur le **certificat éphémère** de Fulcio :
1. Le runner GitHub Actions présente son OIDC token (revendiquant `repo:efe-ai/efe@refs/tags/v1.0.0`)
2. Fulcio vérifie le token + émet un cert X.509 valide 10 min, lié à cette identité
3. cosign signe l'artefact + push la signature à Rekor
4. Tout vérificateur peut ensuite, des années plus tard, prouver que `<artifact_sha256>` a été signé par `<github_repo>@<commit_sha>` à `<timestamp>`

### 3.2 Workflow `release.yml` Phase 2 (Build & sign)

```yaml
# .github/workflows/release.yml — phase 2 extract
name: release-v1

on:
  push:
    tags:
      - "v*.*.*"
  workflow_dispatch:
    inputs:
      version:
        description: "Version to release (e.g. v1.0.0)"
        required: true

permissions:
  contents: write
  id-token: write          # required for OIDC keyless signing
  packages: write          # required to push to GHCR
  attestations: write      # required for SLSA provenance

jobs:
  # Phase 2.1 : Docker images.
  docker-images:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        image:
          - { name: efe-server,         dockerfile: crates/efe-server/Dockerfile }
          - { name: mcp-server,         dockerfile: mcp-server/Dockerfile }
          - { name: portal-admin,       dockerfile: apps/portal-admin/Dockerfile }
          - { name: efe-server-migrate, dockerfile: crates/efe-server/Dockerfile.migrate }
    steps:
      - uses: actions/checkout@v4

      - uses: docker/setup-buildx-action@v3

      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - id: meta
        uses: docker/metadata-action@v5
        with:
          images: ghcr.io/efe-ai/${{ matrix.image.name }}
          tags: |
            type=semver,pattern={{version}}
            type=raw,value=latest,enable={{is_default_branch}}

      - id: build
        uses: docker/build-push-action@v6
        with:
          context: .
          file: ${{ matrix.image.dockerfile }}
          platforms: linux/amd64,linux/arm64
          push: true
          tags: ${{ steps.meta.outputs.tags }}
          labels: ${{ steps.meta.outputs.labels }}
          provenance: true        # built-in SLSA provenance via Buildkit
          sbom: true              # Syft SBOM auto-attached
          cache-from: type=gha
          cache-to: type=gha,mode=max

      - uses: sigstore/cosign-installer@v3
        with:
          cosign-release: "v2.4.0"

      - name: Cosign sign image (keyless)
        env:
          COSIGN_EXPERIMENTAL: "1"
        run: |
          cosign sign --yes \
            ghcr.io/efe-ai/${{ matrix.image.name }}@${{ steps.build.outputs.digest }}

      - name: Generate SLSA-3 provenance
        uses: actions/attest-build-provenance@v1
        with:
          subject-name: ghcr.io/efe-ai/${{ matrix.image.name }}
          subject-digest: ${{ steps.build.outputs.digest }}
          push-to-registry: true

  # Phase 2.2 : Helm chart.
  helm-chart:
    runs-on: ubuntu-latest
    needs: docker-images
    steps:
      - uses: actions/checkout@v4
      - uses: azure/setup-helm@v4
        with: { version: v3.15.4 }

      - name: Package chart
        run: |
          helm package charts/efe-stack \
            --version ${{ inputs.version || github.ref_name }} \
            --app-version ${{ inputs.version || github.ref_name }}

      - name: Push to OCI registry
        env:
          HELM_REGISTRY_USERNAME: ${{ github.actor }}
          HELM_REGISTRY_PASSWORD: ${{ secrets.GITHUB_TOKEN }}
        run: |
          echo "$HELM_REGISTRY_PASSWORD" | helm registry login \
            --username "$HELM_REGISTRY_USERNAME" --password-stdin ghcr.io
          helm push efe-stack-*.tgz oci://ghcr.io/efe-ai/charts

      - uses: sigstore/cosign-installer@v3

      - name: Cosign sign chart blob
        env:
          COSIGN_EXPERIMENTAL: "1"
        run: |
          # Sign the OCI artifact (Helm chart pushed as OCI is signable
          # like any container).
          DIGEST=$(crane digest oci://ghcr.io/efe-ai/charts/efe-stack:${{ inputs.version }})
          cosign sign --yes ghcr.io/efe-ai/charts/efe-stack@${DIGEST}

  # Phase 2.3 : CLI binaries.
  efe-admin-binaries:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        include:
          - { os: ubuntu-latest,  target: x86_64-unknown-linux-musl,   ext: "" }
          - { os: ubuntu-latest,  target: aarch64-unknown-linux-musl,  ext: "" }
          - { os: macos-latest,   target: x86_64-apple-darwin,         ext: "" }
          - { os: macos-latest,   target: aarch64-apple-darwin,        ext: "" }
          - { os: windows-latest, target: x86_64-pc-windows-msvc,      ext: ".exe" }
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Build
        run: |
          cargo build --release --target ${{ matrix.target }} -p efe-admin

      - name: Package
        run: |
          ARCHIVE=efe-admin-${{ inputs.version }}-${{ matrix.target }}.tar.gz
          tar -czf "$ARCHIVE" -C target/${{ matrix.target }}/release efe-admin${{ matrix.ext }}
          sha256sum "$ARCHIVE" > "$ARCHIVE.sha256"

      - uses: sigstore/cosign-installer@v3

      - name: Cosign sign blob
        env:
          COSIGN_EXPERIMENTAL: "1"
        run: |
          ARCHIVE=efe-admin-${{ inputs.version }}-${{ matrix.target }}.tar.gz
          cosign sign-blob --yes "$ARCHIVE" \
            --output-signature "$ARCHIVE.sig" \
            --output-certificate "$ARCHIVE.cert.pem"

      - uses: actions/upload-artifact@v4
        with:
          name: efe-admin-${{ matrix.target }}
          path: |
            efe-admin-*.tar.gz
            efe-admin-*.tar.gz.sha256
            efe-admin-*.tar.gz.sig
            efe-admin-*.tar.gz.cert.pem
```

### 3.3 Cosign verification policy

Le fichier `tools/sigstore/cosign-policy.yaml` documente quelles signatures sont considérées comme valides :

```yaml
# tools/sigstore/cosign-policy.yaml
#
# Cosign verification policy for EFE artifacts.
# Used by the verify-release.sh script and recommended for tenants.

apiVersion: policy.sigstore.dev/v1beta1
kind: ClusterImagePolicy
metadata:
  name: efe-image-policy
spec:
  # Apply policy to all images under ghcr.io/efe-ai
  images:
    - glob: "ghcr.io/efe-ai/**"

  # Authorities = who is allowed to sign.
  authorities:
    - name: efe-release-pipeline
      keyless:
        # The cert subject must match the GitHub Actions OIDC issuer.
        url: https://fulcio.sigstore.dev
        identities:
          - issuer: https://token.actions.githubusercontent.com
            # The signer must be the release.yml workflow on the
            # efe-ai/efe repository on a v* tag (NOT a feature branch).
            subjectRegExp: "^https://github\\.com/efe-ai/efe/\\.github/workflows/release\\.yml@refs/tags/v[0-9]+\\.[0-9]+\\.[0-9]+$"
      ctlog:
        url: https://rekor.sigstore.dev
```

Comment un tenant utilise cette policy :

```bash
# Tenant pulls the policy file from EFE's repo (or vendors it locally).
curl -fsSL https://efe.example.com/release/cosign-policy.yaml \
     -o /etc/cosign-policy.yaml

# Verify a pulled image.
cosign verify \
  --certificate-identity-regexp "^https://github\\.com/efe-ai/efe/.github/workflows/release\\.yml@refs/tags/v[0-9]+\\.[0-9]+\\.[0-9]+$" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  ghcr.io/efe-ai/efe-server:1.0.0
```

Si la signature ne match pas → erreur fatale. Le tenant peut ainsi rejeter les images compromises automatiquement (admission controller en K8s : `policy-controller` from sigstore).

### 3.4 SLSA-3 provenance

SLSA (Supply chain Levels for Software Artifacts) est un framework de classification de la sécurité supply chain :
- **SLSA-1** : provenance attached (build metadata)
- **SLSA-2** : provenance signed (cryptographic origin)
- **SLSA-3** : non-falsifiable provenance + isolated build env

R-100 vise **SLSA-3** via :
- GitHub Actions hosted runners (isolated, ephemeral, builder identity = `github.com/actions`)
- `actions/attest-build-provenance` pour attestation Sigstore-signed
- Provenance contient : commit SHA, workflow URL, build params, builder identity

Vérification :

```bash
slsa-verifier verify-image \
  --source-uri github.com/efe-ai/efe \
  --source-tag v1.0.0 \
  ghcr.io/efe-ai/efe-server:1.0.0@sha256:...

# Output:
# Verifying artifact ghcr.io/efe-ai/efe-server:1.0.0
#   - source: github.com/efe-ai/efe@v1.0.0
#   - workflow: .github/workflows/release.yml
#   - builder: github.com/actions/runner
#   - PASSED
```

### 3.5 SBOM (Syft) auto-attached

Chaque image Docker shippe avec son SBOM (Software Bill of Materials) en format SPDX-JSON ou CycloneDX :

```bash
# Tenant downloads the SBOM.
syft ghcr.io/efe-ai/efe-server:1.0.0 -o spdx-json > efe-server-sbom.json

# Tenant scans for CVEs.
grype sbom:efe-server-sbom.json
```

Format SBOM contient :
- Liste exhaustive des packages OS (apk packages dans Alpine, deb dans Debian)
- Liste exhaustive des libs Rust crates (via Cargo.lock)
- Liste exhaustive des packages npm (via pnpm-lock.yaml)
- Hashes + versions

Permet aux compliance teams tenants de :
- Faire un CVE assessment indépendant
- Comparer entre versions (delta diff)
- Identifier libs avec licences problématiques

### 3.6 VEX (Vulnerability Exploitability eXchange)

À partir de R-100, EFE publie un feed VEX qui dit, pour chaque CVE rapportée par les scanners, **si elle affecte vraiment notre code** :

```json
// docs/security/vex/v1.0.0.json
{
  "@context": "https://openvex.dev/ns/v0.2.0",
  "@id": "https://efe.example.com/security/vex/v1.0.0.json",
  "author": "EFE Security Team",
  "timestamp": "2026-05-09T10:00:00Z",
  "version": 1,
  "statements": [
    {
      "vulnerability": { "name": "CVE-2024-12345" },
      "products": [
        { "@id": "pkg:oci/efe-server@1.0.0" }
      ],
      "status": "not_affected",
      "justification": "vulnerable_code_not_in_execute_path",
      "impact_statement": "EFE does not invoke the affected libfoo function ; static analysis confirms no call site."
    },
    {
      "vulnerability": { "name": "CVE-2024-67890" },
      "products": [
        { "@id": "pkg:oci/efe-server@1.0.0" }
      ],
      "status": "fixed",
      "fix_version": "1.0.1"
    }
  ]
}
```

Tenants consomment ce feed via `grype --vex` pour filter les false-positives.

---

## §4. Pilier 2 — Tag, distribution channels, versioning

### 4.1 Git tag policy

#### 4.1.1 Création du tag master

```bash
# 1. Verify main is clean and CI is green.
git checkout main
git pull
gh run list --workflow ci.yml --branch main --limit 1
# Must be SUCCESS.

# 2. Compose the master tag with signed annotation.
git tag -s v1.0.0 -m "EFE v1.0.0 GA — Mode-3 multi-turn + GLOSS A+B+C"

# 3. Compose sub-tags pointing to the same commit.
for component in tf-aws tf-gcp tf-azure efe-admin portal-admin charts/efe-stack; do
  git tag -s "${component}/v1.0.0" -m "EFE ${component} v1.0.0 GA"
done

# 4. Push all tags atomically.
git push origin --tags
```

`git tag -s` requires GPG configuration. EFE Core has 2 maintainers with signing keys ; both signatures are required for a master tag (enforced via branch protection rule on `refs/tags/v*`).

#### 4.1.2 Branch protection rules

```
.github/branch-protection.json (configured via Terraform)

{
  "branch_protection_rules": [
    {
      "pattern": "v*.*.*",
      "required_signatures": true,
      "required_reviews": 2,
      "required_status_checks": [
        "ci/tests-rust",
        "ci/tests-python",
        "ci/tests-portal-admin",
        "ci/helm-stack-test",
        "ci/terraform-aws",
        "ci/terraform-gcp",
        "ci/terraform-azure"
      ],
      "required_linear_history": true
    }
  ]
}
```

### 4.2 Distribution channels

#### 4.2.1 OCI registry (primary channel)

```bash
# Publish via release.yml Phase 4.
docker push ghcr.io/efe-ai/efe-server:1.0.0
docker push ghcr.io/efe-ai/mcp-server:1.0.0
docker push ghcr.io/efe-ai/portal-admin:1.0.0
docker push ghcr.io/efe-ai/efe-server-migrate:1.0.0
helm push efe-stack-1.0.0.tgz oci://ghcr.io/efe-ai/charts
```

**Tenant consumes** :

```bash
helm install efe-stack oci://ghcr.io/efe-ai/charts/efe-stack \
  --version 1.0.0 \
  --values values-acme.yaml
```

Public registry (no auth required for pull). Tenants who want to vendor offline can `docker save` images to `.tar` files and `docker load` in their air-gapped environment, **with cosign verification preserved** via offline policy.

#### 4.2.2 GitHub Releases

```bash
# Create release with rich notes + binary attachments.
gh release create v1.0.0 \
  --title "EFE v1.0.0 GA" \
  --notes-file docs/release/RELEASE_NOTES_v1.0.0.md \
  --target main

# Attach binaries.
for asset in efe-admin-*.tar.gz*; do
  gh release upload v1.0.0 "$asset"
done

# Attach signature bundles.
for asset in *.sig *.cert.pem *.sha256; do
  gh release upload v1.0.0 "$asset"
done
```

Tenants download via :
- Web UI : `https://github.com/efe-ai/efe/releases/v1.0.0`
- CLI : `gh release download v1.0.0 --pattern "efe-admin-*-darwin-arm64*"`

#### 4.2.3 Homebrew tap

```ruby
# homebrew/efe-admin.rb
class EfeAdmin < Formula
  desc "EFE tenant administration CLI"
  homepage "https://efe.example.com"
  version "1.0.0"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/efe-ai/efe/releases/download/v1.0.0/efe-admin-v1.0.0-aarch64-apple-darwin.tar.gz"
      sha256 "<filled at release time>"
    else
      url "https://github.com/efe-ai/efe/releases/download/v1.0.0/efe-admin-v1.0.0-x86_64-apple-darwin.tar.gz"
      sha256 "<filled at release time>"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/efe-ai/efe/releases/download/v1.0.0/efe-admin-v1.0.0-aarch64-unknown-linux-musl.tar.gz"
      sha256 "<filled at release time>"
    else
      url "https://github.com/efe-ai/efe/releases/download/v1.0.0/efe-admin-v1.0.0-x86_64-unknown-linux-musl.tar.gz"
      sha256 "<filled at release time>"
    end
  end

  def install
    bin.install "efe-admin"
  end

  test do
    assert_match "efe-admin v#{version}", shell_output("#{bin}/efe-admin version")
  end
end
```

Tenants install via :

```bash
brew tap efe/tap
brew install efe-admin

# Or :
brew install efe/tap/efe-admin
```

#### 4.2.4 npm registry

The `apps/portal-admin/` package is **not** typically published to npm (it's an app, not a lib). However, we **do** publish a TypeScript types package for tenants who build their own frontends consuming our gRPC :

```bash
# Sub-package : @efe/sdk
cd clients/typescript/@efe/sdk
npm publish --provenance --access public
# → @efe/sdk@1.0.0 with SLSA provenance attached
```

Tenants install :

```bash
npm install @efe/sdk@1.0.0
# Auto-verifies provenance via npm CLI built-in.
```

#### 4.2.5 Terraform Registry

Submit each module to Terraform Registry :

```bash
# Repos must be public + named correctly :
# github.com/efe-ai/terraform-aws-efe-stack
# github.com/efe-ai/terraform-gcp-efe-stack
# github.com/efe-ai/terraform-azure-efe-stack

# Each repo's `main.tf` is a thin re-export of `efe/terraform/modules/<cloud>/`.
# Tags `v1.0.0` on each repo are auto-detected by Terraform Registry.

# Tenants consume :
module "efe_stack" {
  source  = "efe-ai/efe-stack/aws"
  version = "1.0.0"

  region = "eu-west-1"
  tenant = "acme"
  ...
}
```

#### 4.2.6 Cargo crates registry

We ship 2 crates publicly :

```bash
cd crates/efe-admin
cargo publish

cd ../efe-llm-router  # SDK utility
cargo publish
```

Tenants consume :

```toml
[dependencies]
efe-admin = "1.0.0"
```

### 4.3 Distribution matrix synthèse

| Tenant want | Channel | Command |
|-------------|---------|---------|
| Production K8s deployment | OCI Helm | `helm install oci://...` |
| Local CLI on laptop (Mac/Linux) | Homebrew | `brew install efe/tap/efe-admin` |
| Local CLI on Windows | GitHub Release | direct download `.tar.gz` |
| Frontend SDK (TypeScript) | npm | `npm install @efe/sdk` |
| Cloud infra (Terraform) | Terraform Registry | `module "efe_stack" { source = "efe-ai/..."}` |
| Air-gapped enterprise | GitHub Release `.tar` | `docker load` + cosign verify offline |

### 4.4 Release notes template

`docs/release/RELEASE_NOTES_v1.0.0.md` (auto-generated stub + manual curation) :

```markdown
# EFE v1.0.0 — General Availability

Released: 2026-05-15

## Highlights

- 🚀 **Mode-3 multi-turn iterative reasoning** (M22) : right-to-explanation Art. 22 GDPR via concept_stack
- 🛠️ **GLOSS Self-service onboarding** : new tenant in ≤ 30 min via `efe-admin` CLI + Helm
- 🌐 **3-cloud Terraform modules** : AWS, GCP, Azure idiomatic
- 🎛️ **Portal Admin Next.js** : DPO/admin UI with audit + compliance + concept stack viewer
- 🔐 **Sigstore + SLSA-3** : every artifact signed and verifiable

## For DPOs

- New page `/audit/[id]` exposes the full Mode-3 reasoning trace (concept_stack)
- New PDF export endpoint `/api/audit/[id]/export.pdf` for right-to-explanation requests
- AttestationForm UI for AI Act Art. 14 + GDPR Art. 22 consent recording
- Compliance package downloadable from `/compliance/download-package`

## For SE / DevOps

- New `efe-admin` CLI : 18 commands for full tenant lifecycle
- Helm umbrella chart `efe-stack` with 6 sub-charts
- Terraform modules for AWS / GCP / Azure (idiomatic each)
- 12 ops runbooks RB-01..12 with reproducible validation procedures

## For Developers

- mcp-server exposes 65 tools (M16-M22 + 16 D8 admin)
- Rust efe-admin crate publishable on crates.io
- TypeScript SDK `@efe/sdk@1.0.0` published with SLSA provenance

## Security

- Every artifact signed Sigstore (cosign verification policy at `tools/sigstore/cosign-policy.yaml`)
- SLSA-3 provenance attached to all 4 Docker images + Helm chart + npm package
- BYOK key rotation procedure documented (RB-04)
- HMAC audit chain D14-bis tamper-evident on all M21+ rows
- 0 known HIGH / CRITICAL CVEs (Trivy scan on release artifacts)

## Performance

- gRPC classify p99 < 250 ms on Mode-1 (baseline)
- Mode-2 LLM-routed p99 < 800 ms (depends on provider)
- Mode-3 multi-turn p99 < 4 s (3 turns avg)
- Helm install on k3d : ≤ 5 min
- Terraform AWS apply : ≤ 12 min ; GCP ≤ 15 min ; Azure ≤ 18 min

## Breaking changes

None — v1.0.0 is fully backward-compatible with the M22 last release-candidate.

## Roadmap

- M23 (DPO Export + Active Learning) : Q3 2026
- M24 (LLM Eval Harness) : Q4 2026
- M25 (Multi-Region Active-Active) : Q1 2027

## Verifying this release

```bash
# Clone the verification script.
curl -fsSL https://raw.githubusercontent.com/efe-ai/efe/v1.0.0/scripts/release/verify-release.sh \
  | bash -s -- v1.0.0
```

Output :
```
✅ Cosign verify: 4/4 Docker images PASS
✅ Cosign verify: 1/1 Helm chart PASS
✅ SLSA verify: 4/4 images PASS (SLSA-3)
✅ Rekor inclusion proofs: 4/4 PASS
✅ E2E smoke test on k3d: PASS
```

## Acknowledgments

EFE Core Engineering Team.

— The EFE Team
```

---

## §5. Pilier 3 — Announce & customer/partner communications

### 5.1 Blog post

`docs/marketing/BLOG_POST_v1_0_0_GA.md` (~ 2 500 mots). Structure :

1. **Headline** : « EFE v1.0.0 — Compliant AI classification reaches General Availability »
2. **Lede** : 1 paragraphe, ce qui change pour le lecteur (DPO ou ops)
3. **What is EFE** : pour les nouveaux lecteurs
4. **What's in v1.0.0** : 5 highlights (Mode-3, GLOSS, 3-cloud, Portal, Sigstore)
5. **Compliance positioning** : EU AI Act + GDPR Art. 22 + SOC 2 readiness
6. **Sample architecture diagram** : how a tenant deploys EFE in 30 min
7. **Quotes** : EFE CEO, lead engineer, DPO pilote (testimonial)
8. **Roadmap** : ce qui arrive
9. **Call-to-action** : start free trial / docs / GitHub

Auto-published via CMS API trigger from `release.yml` Phase 5.

### 5.2 Customer email template

`docs/marketing/CUSTOMER_EMAIL_TEMPLATE.md` :

```markdown
Subject: 🚀 EFE v1.0.0 GA — what's new for your tenant {{tenant_name}}

Hi {{contact_name}},

We're excited to announce that **EFE v1.0.0** is now generally available.

For your tenant {{tenant_name}}, three things are immediately available :

## 1. Self-service portal

Your DPO can now access the new portal at :
**https://portal.{{tenant_domain}}**

Features :
- Real-time dashboard (RPS, latency, cost, pending Mode-3 reviews)
- Right-to-explanation export (PDF) — required by GDPR Art. 22
- AI Act Art. 14 attestation flow (DPO-signable)
- API key management + audit log search

(Sign in via your existing SSO — no new credentials.)

## 2. Self-service `efe-admin` CLI

Your engineering team can now manage the tenant from any laptop :

```bash
brew install efe/tap/efe-admin
efe-admin login --cluster {{tenant_slug}}
efe-admin tenant get {{tenant_slug}}
```

18 commands : tenant CRUD, key rotation, catalog seed, compliance attestation,
audit query, smoke test, etc.

## 3. Compliance package (auto-generated)

Download your tenant's compliance evidence at :
**https://portal.{{tenant_domain}}/compliance/download-package**

Includes :
- SOC 2 readiness package
- GDPR Art. 30 register entry (your tenant data)
- AI Act Art. 11 technical documentation
- Audit chain integrity report (HMAC verified)
- Updated automatically every month

## What's new technically

See full release notes :
https://efe.example.com/blog/v1-0-0-ga
https://github.com/efe-ai/efe/releases/v1.0.0

## Upgrade timeline

Your tenant {{tenant_name}} is currently running version {{current_version}}.

- **No action required from you** : the upgrade to v1.0.0 has been
  scheduled for {{upgrade_window}} during your existing maintenance window.
- **Rollback available** : the previous version stays available for 30 days.
- **Zero downtime** : Helm rolling upgrade with PDB minAvailable.

If you have questions, reply to this email or contact :
- Account Executive : {{ae_email}}
- Support : support@efe.example.com
- Status : https://status.efe.example.com

Best,
The EFE Team
```

Sent via Mailgun batch API, personalized with handlebars-style placeholders.

### 5.3 Partner notifications

For system integrators / consulting partners :

- **Slack notification** in `#efe-partner-channel` (manual + automated digest)
- **Email** to partner technical leads list (via HubSpot)
- **Webinar invite** for the upcoming SE training session

`docs/marketing/PARTNER_NOTIFICATION.md` :

```markdown
Subject: 🤝 EFE v1.0.0 GA — partner enablement materials

Hi {{partner_name}},

EFE v1.0.0 is now GA. As a certified partner, you have early access to :

## SE training kit

Upcoming webinar : 2026-05-22 14:00 UTC
Registration : https://efe.example.com/webinars/se-v1-ga

Topics covered :
- Onboarding a new tenant in 30 min (live demo)
- Compliance positioning : EU AI Act + GDPR Art. 22
- Pricing + sales talking points
- Common objections + rebuttals

## Updated assets

- Battlecard v1.0.0 (PDF) — competitive positioning
- Case studies : 3 pilot tenants who validated v1.0.0
- Quote sheet : tested customer testimonials
- POC playbook v1.0.0 — step-by-step 30-min onboarding

Download : https://partner.efe.example.com/assets/v1-0-0-ga.zip

## Co-marketing opportunities

- Joint blog posts (EFE + your firm)
- Joint webinars
- Conference co-sponsorship (KubeCon, Re:Invent, ...)

Reply to this email to schedule a co-marketing call.

Best,
EFE Partnerships Team
```

### 5.4 Security disclosure feed

EFE publishes a public **security advisory feed** in 2 formats :
- **RSS / Atom** : `https://efe.example.com/security/feed.xml`
- **OpenVEX JSON** : `https://efe.example.com/security/vex/feed.json`

Initial entries at GA (none expected) :

```xml
<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>EFE Security Advisories</title>
    <link>https://efe.example.com/security</link>
    <description>Security advisories and CVE assessments for EFE</description>
    <language>en</language>
    <pubDate>Fri, 09 May 2026 10:00:00 GMT</pubDate>

    <item>
      <title>EFE v1.0.0 General Availability — initial security baseline</title>
      <link>https://efe.example.com/security/v1.0.0-ga</link>
      <description>
        EFE v1.0.0 ships with 0 known HIGH/CRITICAL CVEs.
        SLSA-3 provenance, Sigstore signatures, distroless containers.
      </description>
      <pubDate>Fri, 09 May 2026 10:00:00 GMT</pubDate>
      <guid>v1.0.0-ga</guid>
    </item>
  </channel>
</rss>
```

Tenants subscribe via :
- RSS reader (Feedly, Inoreader, ...)
- Email digest (sign up at `https://efe.example.com/security/subscribe`)
- Slack webhook (provided to enterprise customers)

### 5.5 Press release

For mainstream tech press (TechCrunch, The Register, InfoQ, Le Monde Informatique, etc.) :

`docs/marketing/PRESS_RELEASE.md` :

```markdown
FOR IMMEDIATE RELEASE
2026-05-15

# EFE Announces General Availability of v1.0.0
## EU AI Act-compliant text classification platform with multi-turn LLM reasoning

PARIS, France — EFE today announced the general availability of EFE v1.0.0,
the company's flagship multi-tenant text classification platform engineered
from day-zero for compliance with EU AI Act Article 14 and GDPR Article 22.

Unlike alternative classification platforms that bolt compliance on as an
afterthought, EFE v1.0.0 ships with :
- **HMAC tamper-evident audit chains** retained 7 years
- **Right-to-explanation API** for Mode-3 multi-turn classifications (concept_stack)
- **Hardware-bound BYOK** keys (KMS / KeyVault / Cloud KMS supported)
- **Self-service tenant onboarding** in under 30 minutes

"EFE is the first classification platform we evaluated where compliance
isn't an add-on — it's the foundation," said Jane Doe, DPO at Acme Corp,
a pilot customer for v1.0.0. "Our auditors loved the right-to-explanation
PDF export."

EFE v1.0.0 is available immediately on AWS, GCP, and Azure via Terraform
modules and the `efe-stack` Helm chart. The platform supports air-gapped
deployments via OCI registry mirrors and offline cosign verification.

## About EFE

EFE is a French software company building compliant AI classification
infrastructure for enterprise customers. Founded in 2024, the company
serves DPOs, ITS leaders, and engineering teams across Europe and the US.

## Media contact

press@efe.example.com
+33 (0)1 23 45 67 89

## Customer testimonials

[3 customer quotes, vetted by Account Executives]
```

---

## §6. Compliance certification packages

### 6.1 SOC 2 evidence package

EFE is in scope for **SOC 2 Type II** audit (planned Q1 2027). R-100 ships a **SOC 2 readiness package** that customers can use to evaluate EFE's controls maturity :

```
docs/compliance/SOC2_EVIDENCE_TEMPLATE.md

Sections :
1. Trust Services Criteria (5)
   - Security
   - Availability
   - Processing Integrity
   - Confidentiality
   - Privacy

2. Per-control evidence
   - CC1.1 : Code of Conduct (Anchor : EFE_CODE_OF_CONDUCT.md)
   - CC2.1 : Background checks (HR doc reference)
   - CC6.1 : Logical access controls (RBAC architecture)
   - CC7.1 : System monitoring (Prometheus + alertmanager)
   - ... (~ 60 controls total)

3. Auditor SOQ (Statement of Qualifications)
   - Pre-validated auditor list
   - Indicative pricing
```

Le tenant peut consommer ce template pour évaluer si EFE est suffisamment mature pour son procurement process.

### 6.2 GDPR Art. 30 register

EFE publishes a **public GDPR Art. 30 register entry** documenting how it processes data on behalf of tenants (as data processor) :

```markdown
docs/compliance/GDPR_ART30_REGISTER.md

# GDPR Art. 30 Register Entry — EFE Data Processing

## Identity of the controller / processor

- Processor : EFE SAS, 123 rue de la République, 75001 Paris, France
- DPO : dpo@efe.example.com
- Article 27 representative (UK) : ...

## Categories of processing

For each tenant, EFE processes :
- Sentence text submitted via classify API (raw redacted at write time)
- Tenant metadata (slug, contact, quotas, billing)
- Audit logs (7-year retention)
- Mode-3 review queue rows (with concept_stack)

## Categories of data subjects

Indirectly (via tenant) : end users of the tenant's products who submit
text to be classified (e.g. customer support tickets, internal documents).

## Transfer to third countries

- AWS region : tenant choice (EU_ONLY supported)
- GCP region : tenant choice (EU_ONLY supported)
- Azure location : tenant choice (EU_ONLY supported)

If tenant chooses EU_ONLY, all data stays in EU.

## Retention

- Audit chain (HMAC) : 7 years (compliance baseline)
- Concept_stack (Mode-3) : 7 years (right-to-explanation)
- Sentence text : NEVER stored (only sentence_hash kept)
- Aggregated metrics : 30 days

## Security measures

- Encryption at rest (KMS BYOK)
- Encryption in transit (TLS 1.3)
- Access controls (IAM IRSA / Workload Identity)
- HMAC tamper-evident audit chain
- 24/7 on-call SRE

## Breach notification

In case of personal data breach :
- Customer notification : within 24 hours
- DPA notification : within 72 hours (GDPR Art. 33)
- Affected data subjects : within 30 days (GDPR Art. 34) if high risk
```

### 6.3 AI Act Art. 11 technical documentation

```markdown
docs/compliance/AI_ACT_ART11_TECHNICAL_DOC.md

# AI Act Art. 11 Technical Documentation — EFE Classification

## 1. General description of the AI system

EFE is a high-risk AI system under EU AI Act (Annex III, point 4 — employment and access to essential private services).

## 2. Detailed description of the elements

- Mode-1 : compiled anchors + swarm consensus (deterministic post-bundle)
- Mode-2 : LLM-routed dynamic anchors (single LLM call per classify)
- Mode-3 : multi-turn iterative refinement (1-5 turns, human review fallback)

## 3. Detailed description of the design choices

[Architecture diagrams + decision rationale]

## 4. Description of the intended purpose

Multi-tenant text classification for enterprise products.

## 5. Risk management measures

- Pre-deployment evaluation (golden test set)
- Bias testing per anchor / catalog
- Drift detection (D9 architecture)
- Human review for Mode-3 non-converged

## 6. Human oversight measures (Art. 14)

- DPO attestation flow (portal `/compliance`)
- Mode-3 review queue with mandatory human approval/rejection
- Tenant suspension capability

## 7. Description of the data sets used

- Anchors : provided by tenant (sample sets shipped for evaluation)
- Embeddings : MiniLM-L6 (open-source, deterministic)
- LLM calls : tenant-configured (Anthropic, OpenAI, Bedrock, ...)

## 8. Cybersecurity measures (Art. 15)

- Sigstore signing (SLSA-3)
- HMAC audit chain
- BYOK key rotation procedure (RB-04)
- Penetration test : every 12 months (next : Q4 2026)
```

### 6.4 ISO 27001 readiness

```markdown
docs/compliance/ISO_27001_READINESS.md

# ISO 27001 Readiness Self-Assessment

| Annex A control | Status | Evidence |
|-----------------|--------|----------|
| A.5 Information security policies | ✅ Documented | `docs/security/POLICY.md` |
| A.6 Organization of information security | ✅ Documented | RACI matrix |
| A.7 Human resource security | ✅ HR process | (private) |
| A.8 Asset management | ✅ Documented | SBOM + asset register |
| A.9 Access control | ✅ Implemented | RBAC + IRSA + WI |
| A.10 Cryptography | ✅ Implemented | KMS + cosign + HMAC |
| ... | | |

Overall : ~ 85 % readiness. Gaps documented + remediation planned.

Target audit : Q3 2027.
```

### 6.5 Auto-generation pipeline

`scripts/release/generate-compliance-pack.sh` :

```bash
#!/usr/bin/env bash
# Generate the per-tenant compliance package downloadable from portal.

set -euo pipefail

TENANT="$1"
OUTPUT_DIR="$2"

# 1. SOC 2 evidence (template).
cp docs/compliance/SOC2_EVIDENCE_TEMPLATE.md "$OUTPUT_DIR/"

# 2. GDPR Art. 30 register (tenant-specific).
sed "s/{{tenant_name}}/$TENANT/g" docs/compliance/GDPR_ART30_REGISTER.md \
    > "$OUTPUT_DIR/GDPR_ART30_REGISTER.md"

# 3. AI Act Art. 11 technical doc.
cp docs/compliance/AI_ACT_ART11_TECHNICAL_DOC.md "$OUTPUT_DIR/"

# 4. Audit chain integrity report (run `efe-verify-audit-chain`).
efe-admin audit verify "$TENANT" --output-pdf > "$OUTPUT_DIR/audit-chain-report.pdf"

# 5. Tenant-specific attestation status.
efe-admin compliance verify "$TENANT" --format json \
    > "$OUTPUT_DIR/compliance-status.json"

# 6. ISO 27001 readiness assessment.
cp docs/compliance/ISO_27001_READINESS.md "$OUTPUT_DIR/"

# 7. ZIP + sign.
ZIP_FILE="${OUTPUT_DIR}-compliance-pack.zip"
zip -r "$ZIP_FILE" "$OUTPUT_DIR/"

# 8. cosign-sign the ZIP.
cosign sign-blob --yes "$ZIP_FILE" \
    --output-signature "$ZIP_FILE.sig" \
    --output-certificate "$ZIP_FILE.cert.pem"

echo "✅ Compliance pack generated : $ZIP_FILE"
```

Cron job `monthly-compliance-regen` regenerates pour tous tenants actifs.

---

## §7. Post-release operations

### 7.1 Hotfix policy

Pour les v1.x.y où y > 0 :

```
# 1. Cherry-pick fix from main to release/v1.x branch.
git checkout release/v1.x
git cherry-pick <commit-sha>

# 2. Bump version in Cargo.toml + package.json + Chart.yaml.
./scripts/release/bump-version.sh 1.0.1

# 3. Tag + run release workflow.
git tag -s v1.0.1 -m "EFE v1.0.1 — fix CVE-XXXX"
git push origin v1.0.1

# Workflow re-runs phases 2-5.
```

Hotfix = PATCH bump uniquement (no new features).

### 7.2 Deprecation policy

EFE follows a **conservative deprecation policy** :

| Type | Window |
|------|--------|
| API breaking change | 12 months notice + N+2 release support |
| Tool retirement (mcp-server) | 6 months notice |
| Helm value renamed | 12 months alias |
| Postgres column dropped | NEVER (additive-only migrations policy) |

Deprecation announcements via :
- Release notes section « Deprecation notices »
- RSS security advisory feed (specifically tagged `deprecation`)
- Customer email (60 days before sunset)

### 7.3 Support tiers post-GA

| Tier | Response | SLA | Channels |
|------|----------|-----|----------|
| Community | Best-effort | None | GitHub Discussions, public Slack |
| Standard (paid) | Business hours | P0 24h, P1 72h, P2 5d | Email + private Slack |
| Enterprise (paid+) | 24/7 | P0 1h, P1 4h, P2 1d | Phone + dedicated Slack + AE |
| Mission-critical | 24/7 + on-site | P0 30 min, P1 2h, P2 8h | Above + on-site engineer |

Tier de chaque tenant configuré dans `tenants.support_tier` (post-R-100 column).

### 7.4 Versioning roadmap

```
2026 Q3  → v1.1.0  (M23 DPO Export + Active Learning)
2026 Q4  → v1.2.0  (M24 LLM Eval Harness)
2027 Q1  → v1.3.0  (M25 Multi-Region Active-Active)
2027 Q3  → v2.0.0  (proto schema breaking change ; planned 12-mo notice)
```

---

## §8. CI/CD pipeline GA

### 8.1 `release.yml` complète

Le workflow complet est ~ 600 lignes YAML. Architecture :

```yaml
name: release-v1
on:
  push:
    tags: ["v*.*.*"]
  workflow_dispatch:
    inputs:
      version:
        required: true
      dry_run:
        type: boolean
        default: false

permissions:
  contents: write
  id-token: write
  packages: write
  attestations: write

jobs:
  validate:                    # Phase 1
    # 8 parallel jobs : tests Rust, Python, portal, helm, terratest × 3, audit
    ...

  docker-images:               # Phase 2.1
    needs: validate
    strategy:
      matrix:
        image: [efe-server, mcp-server, portal-admin, efe-server-migrate]
    ...

  helm-chart:                  # Phase 2.2
    needs: docker-images
    ...

  cli-binaries:                # Phase 2.3
    needs: validate
    strategy:
      matrix:
        target: [6 platforms]
    ...

  npm-publish:                 # Phase 2.4
    needs: validate
    ...

  verify:                      # Phase 3
    needs: [docker-images, helm-chart, cli-binaries]
    ...

  github-release:              # Phase 4.1
    needs: verify
    ...

  homebrew-tap:                # Phase 4.2
    needs: verify
    ...

  terraform-registry:          # Phase 4.3
    needs: verify
    ...

  announce:                    # Phase 5
    needs: github-release
    ...
```

### 8.2 Rollback workflow `release-rollback.yml`

```yaml
name: release-rollback
on:
  workflow_dispatch:
    inputs:
      version:
        required: true
      reason:
        required: true

jobs:
  yank:
    runs-on: ubuntu-latest
    steps:
      - name: Yank Docker images
        run: |
          # Mark images deprecated (don't delete — Sigstore signs are immutable).
          for img in efe-server mcp-server portal-admin efe-server-migrate; do
            crane mutate ghcr.io/efe-ai/$img:${{ inputs.version }} \
              --annotation "deprecated=true" \
              --annotation "deprecation-reason=${{ inputs.reason }}"
          done

      - name: Yank Helm chart
        run: |
          # Helm chart yank = remove from registry (still in Rekor log).
          oras manifest delete ghcr.io/efe-ai/charts/efe-stack:${{ inputs.version }}

      - name: Yank GitHub Release
        run: |
          gh release edit v${{ inputs.version }} --draft

      - name: Cargo yank
        run: |
          cargo yank --vers ${{ inputs.version }} efe-admin

      - name: npm deprecate
        run: |
          npm deprecate @efe/sdk@${{ inputs.version }} \
            "Yanked due to ${{ inputs.reason }}"

      - name: Update security advisory
        run: |
          ./scripts/release/append-vex-yank.sh ${{ inputs.version }} "${{ inputs.reason }}"

      - name: Notify customers
        run: |
          ./scripts/release/send-yank-notification.sh ${{ inputs.version }}
```

### 8.3 `verify-release.sh` script

```bash
#!/usr/bin/env bash
# scripts/release/verify-release.sh
#
# Verifies a published EFE release end-to-end :
# - cosign verify all 4 Docker images
# - cosign verify Helm chart
# - slsa-verifier on all artifacts
# - Rekor inclusion proofs
# - smoke test on ephemeral k3d cluster

set -euo pipefail

VERSION="${1:-v1.0.0}"
echo "🔍 Verifying EFE release $VERSION..."

# 1. Cosign verify Docker images.
for img in efe-server mcp-server portal-admin efe-server-migrate; do
  echo "  Cosign verify ghcr.io/efe-ai/$img:${VERSION#v}"
  cosign verify \
    --certificate-identity-regexp "^https://github\\.com/efe-ai/efe/.github/workflows/release\\.yml@refs/tags/$VERSION$" \
    --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
    "ghcr.io/efe-ai/$img:${VERSION#v}" >/dev/null
  echo "    ✅ verified"
done

# 2. Cosign verify Helm chart.
echo "  Cosign verify oci://ghcr.io/efe-ai/charts/efe-stack:${VERSION#v}"
HELM_DIGEST=$(crane digest "ghcr.io/efe-ai/charts/efe-stack:${VERSION#v}")
cosign verify \
  --certificate-identity-regexp "^https://github\\.com/efe-ai/efe/.github/workflows/release\\.yml@refs/tags/$VERSION$" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  "ghcr.io/efe-ai/charts/efe-stack@${HELM_DIGEST}" >/dev/null
echo "    ✅ verified"

# 3. SLSA verifier.
for img in efe-server mcp-server portal-admin efe-server-migrate; do
  echo "  SLSA verify ghcr.io/efe-ai/$img:${VERSION#v}"
  slsa-verifier verify-image \
    --source-uri "github.com/efe-ai/efe" \
    --source-tag "$VERSION" \
    "ghcr.io/efe-ai/$img:${VERSION#v}" >/dev/null
  echo "    ✅ SLSA-3 verified"
done

# 4. Rekor inclusion proofs (implicit in cosign verify, but explicit log query).
echo "  Querying Rekor log..."
COMMIT_SHA=$(gh api "repos/efe-ai/efe/git/refs/tags/$VERSION" --jq .object.sha)
COUNT=$(rekor-cli search --sha "$COMMIT_SHA" --format json | jq '. | length')
echo "    ✅ $COUNT entries in Rekor for commit $COMMIT_SHA"

# 5. E2E smoke test on k3d.
echo "  E2E smoke test on k3d..."
CLUSTER="efe-verify-$$"
trap "k3d cluster delete $CLUSTER >/dev/null 2>&1 || true" EXIT
k3d cluster create "$CLUSTER" --no-lb --wait --timeout 5m >/dev/null
helm install efe-stack "oci://ghcr.io/efe-ai/charts/efe-stack" \
  --version "${VERSION#v}" \
  --values charts/efe-stack/values-local.yaml \
  --set global.tenant=verify \
  --set postgres-operator.enabled=false \
  --create-namespace --namespace efe \
  --wait --timeout 10m >/dev/null
kubectl wait -n efe pod \
  -l "app.kubernetes.io/name in (efe-server,redis-stack)" \
  --for=condition=Ready --timeout=180s >/dev/null
echo "    ✅ k3d smoke test PASS"

echo
echo "✅ EFE release $VERSION verification complete (all checks PASS)"
```

---

## §9. Critères d'acceptation détaillés

### 9.1 Performance

- Pipeline `release.yml` total runtime ≤ 1h 30 min (du tag au announce)
- `verify-release.sh` runtime ≤ 60 min (incluant E2E k3d)
- Docker image push to GHCR ≤ 5 min par image (avec cache hit)
- Helm chart push ≤ 30 s
- npm publish ≤ 1 min

### 9.2 Sécurité

- Sigstore : 100 % artefacts signés, 100 % vérifiables
- SLSA-3 : 100 % images Docker + Helm chart + npm package
- Trivy scan post-build : 0 HIGH / CRITICAL CVEs
- cosign verify policy distribuée publiquement (pas seulement interne)
- Rekor inclusion proofs publiquement vérifiables
- Pas de clés long-lived dans CI (OIDC keyless only)

### 9.3 Compliance

- SOC 2 readiness package téléchargeable
- GDPR Art. 30 register publié
- AI Act Art. 11 technical doc publiée
- VEX feed initialisé
- DPO de chaque tenant peut télécharger sa compliance pack en 1 clic

### 9.4 Communications

- Blog post live sur `https://efe.example.com/blog/v1-0-0-ga`
- Customer emails délivrés (≥ 99 % delivery rate via Mailgun bounce tracking)
- Partner notifications envoyées
- Press release diffusée à 20+ media outlets
- Twitter / LinkedIn posts publiés

### 9.5 Distribution

- 4 Docker images publiques sur GHCR
- 1 Helm chart sur OCI registry
- 6 CLI binaries sur GitHub Release
- 1 Homebrew formula publiée
- 1 npm package `@efe/sdk` publié
- 3 Terraform modules sur Terraform Registry

---

## §10. Risques & mitigations

| ID | Risque | Probabilité | Impact | Mitigation |
|----|--------|-------------|--------|------------|
| **R-1** | Sigstore Rekor downtime au moment du release | L | H | Cosign retries + fallback `--rekor-url` à un mirror ; release peut continuer en `--insecure-ignore-tlog` avec post-hoc Rekor sync |
| **R-2** | GitHub Actions OIDC token rejected by Fulcio (transient) | L | H | Retry avec exponential backoff ; fallback sur clé GPG secondaire (sealed in CI) |
| **R-3** | npm publish rejected (token revoked, network) | L | M | Use `--otp` + provenance ; retry workflow ; fallback to manual publish |
| **R-4** | OCI registry GHCR rate-limited | L | M | Use private mirror ; pre-release pull-through cache |
| **R-5** | Email blast triggers spam filters | M | M | Mailgun warm-up + DKIM + SPF + DMARC ; staggered send (10 % first, then 90 %) |
| **R-6** | Customer reports critical bug post-release | M | C | Pre-release pilot validation (3 tenants) ; rollback workflow tested ; status page communication |
| **R-7** | Press release embargoed leak | L | M | NDAs ; auto-publish at exact UTC time ; pre-share only with vetted media |
| **R-8** | Compliance pack contains PII | L | C | Auto-generation from templates only ; manual review by DPO before publishing each month |
| **R-9** | Homebrew tap PR auto-fail (formula syntax) | M | L | Test formula on macOS runner before submitting PR ; manual retry |
| **R-10** | Terraform Registry submission delayed (manual review) | M | L | Submit early ; communicate to customers that direct git ref still works |
| **R-11** | Customer's air-gapped env can't reach Sigstore | H (some customers) | M | Provide offline verification bundle (Rekor log entry + cert + sig as files) ; document `--insecure-skip-rekor` flag with risk warning |
| **R-12** | Old version yank breaks customers | M | H | 30-day deprecation window ; customer email ; backward-compat preserved |

---

## §11. Plan de delivery (R-100 sprint)

### 11.1 Day-by-day (5 jours-homme)

| Jour | Livrable | Tests |
|------|----------|-------|
| **D1** | `.github/workflows/release.yml` Phase 1+2 (validate + build+sign Docker) | release dry-run on `v0.999.0` test tag |
| **D2** | Phase 2 complete : Helm chart + CLI binaries + npm + Terraform | dry-run continues |
| **D3** | Phase 3-4 : verify + GitHub Release + Homebrew tap + Terraform Registry submit | end-to-end dry-run on test tag |
| **D4** | Phase 5 : announce automation (CMS API + Mailgun batch) ; compliance pack generation script | full pipeline test |
| **D5** | Verification harness (`verify-release.sh`) + rollback workflow ; final docs ; first **real** v1.0.0 release | production cutover |

### 11.2 Cumul tests post-R-100

| Couche | Pre-R-100 | Post-R-100 | Δ |
|--------|-----------|------------|---|
| Existing tests M16-M22 + GLOSS A+B+C | 3 297 | 3 297 | — |
| Release pipeline integration tests | 0 | **15** (validate, sign, verify, distribute, announce) | +15 |
| `verify-release.sh` E2E | 0 | **1 script** with 5 sub-checks | +5 |
| Rollback workflow integration test | 0 | **1** | +1 |
| **Cumul GREEN** | **3 297** | **~3 318** | **+21** |

---

## §12. Annexes

### 12.1 Sample release notes file structure

```
docs/release/
├── RELEASE_NOTES_v1.0.0.md      # Released 2026-05-15
├── RELEASE_NOTES_v1.0.1.md      # Hotfix 2026-06-01 (sample)
├── RELEASE_NOTES_v1.1.0.md      # M23 Q3 2026
└── RELEASE_NOTES_TEMPLATE.md    # Stub for future releases
```

### 12.2 Sample customer email rendering

```
From: noreply@efe.example.com
To: contact@acme.com
Subject: 🚀 EFE v1.0.0 GA — what's new for your tenant acme

Hi Jane,

We're excited to announce EFE v1.0.0 is now generally available.

For your tenant acme, three things are immediately available :

1. Self-service portal — https://portal.acme.com
   ...
```

### 12.3 Sample git tag commands

```bash
# Master tag (signed by 2 maintainers via branch protection).
git tag -s v1.0.0 -m "EFE v1.0.0 GA — Mode-3 + GLOSS A+B+C"

# Sub-tags.
git tag -s tf-aws/v1.0.0 -m "EFE tf-aws module v1.0.0 GA"
git tag -s tf-gcp/v1.0.0 -m "EFE tf-gcp module v1.0.0 GA"
git tag -s tf-azure/v1.0.0 -m "EFE tf-azure module v1.0.0 GA"
git tag -s efe-admin/v1.0.0 -m "EFE efe-admin CLI v1.0.0 GA"
git tag -s portal-admin/v1.0.0 -m "EFE portal-admin v1.0.0 GA"
git tag -s charts/efe-stack/v1.0.0 -m "EFE Helm chart efe-stack v1.0.0 GA"

# Push all atomically.
git push origin --tags
```

### 12.4 Pre-release checklist

```
## Pre-flight v1.0.0 (post Sprint C audit pass)

### Code
- [ ] cargo test --workspace --all-features GREEN
- [ ] pytest mcp-server/tests/ GREEN (~ 700 tests)
- [ ] pnpm test:unit GREEN (47 tests)
- [ ] pnpm test:e2e GREEN sur 3 browsers (~ 9 specs at GA)
- [ ] helm-unittest GREEN (25 tests)
- [ ] terratest GREEN (8 AWS + 2 GCP + 2 Azure)
- [ ] cargo audit 0 vulnerabilities HIGH/CRITICAL
- [ ] pnpm audit 0 vulnerabilities HIGH/CRITICAL
- [ ] Trivy image scan 0 HIGH/CRITICAL
- [ ] tfsec × 3 modules clean

### Docs
- [ ] CHANGELOG v1.0.0 finalisé
- [ ] RELEASE_NOTES_v1.0.0.md curated
- [ ] BLOG_POST_v1_0_0_GA.md ready for CMS
- [ ] CUSTOMER_EMAIL_TEMPLATE.md tested
- [ ] PARTNER_NOTIFICATION.md sent to partner team for review
- [ ] PRESS_RELEASE.md vetted by Marketing

### Compliance
- [ ] SOC2_EVIDENCE_TEMPLATE.md updated
- [ ] GDPR_ART30_REGISTER.md updated
- [ ] AI_ACT_ART11_TECHNICAL_DOC.md updated
- [ ] ISO_27001_READINESS.md updated

### Pipeline
- [ ] release.yml dry-run on test tag SUCCESS
- [ ] verify-release.sh runs in < 60 min
- [ ] release-rollback.yml tested manually
- [ ] cosign-policy.yaml reviewed by Security Lead
- [ ] OIDC trust between GitHub Actions ↔ Fulcio verified
- [ ] Mailgun batch warm-up done (no domain reputation issues)

### Validation
- [ ] 3 pilot tenants validated v1.0.0 in production-like env
- [ ] Audit pass chirurgical Sprint C : ≤ 6 défauts trouvés et tous corrigés
- [ ] DPO Acme (pilote) signed off the portal experience
- [ ] Press contacts briefed under embargo

### Communication
- [ ] Blog post draft revue marketing
- [ ] Customer email staged in Mailgun
- [ ] Partner Slack channel scheduled
- [ ] Status page notification scheduled
- [ ] Twitter / LinkedIn posts staged

### Tag + push
- [ ] git tag v1.0.0 + 6 sub-tags, signed by 2 maintainers
- [ ] git push origin --tags (triggers release.yml)
- [ ] Monitor release.yml run (≤ 1h 30 min total)
- [ ] verify-release.sh on production immediately after
```

### 12.5 Diagramme de séquence détaillé : tag → announce

```
[Maintainer]
   │
   │ 1. git tag -s v1.0.0
   │ 2. git push origin v1.0.0
   ▼
[GitHub]
   │
   │ 3. Trigger release.yml workflow
   ▼
[GitHub Actions runner]
   │
   │ 4. Phase 1 : validate (parallel jobs)
   │    └─ all green → continue
   │
   │ 5. Phase 2 : build & sign (parallel)
   │    ├─ docker build + cosign sign + Rekor log
   │    ├─ helm package + cosign sign-blob
   │    ├─ cargo build × 6 + cosign sign-blob
   │    └─ npm publish --provenance
   │
   │ 6. Phase 3 : verify
   │    ├─ cosign verify all
   │    ├─ slsa-verifier all
   │    └─ k3d smoke test
   │
   │ 7. Phase 4 : distribute
   │    ├─ docker push (already pushed in step 5)
   │    ├─ gh release create
   │    ├─ homebrew tap PR (auto-merged)
   │    └─ terraform registry submit
   │
   │ 8. Phase 5 : announce
   │    ├─ POST blog CMS API
   │    ├─ Mailgun batch send (5 000 customers)
   │    ├─ Slack webhook to partners
   │    └─ RSS feed update
   │
   ▼
[Customers, Partners, Press]
   │
   │ 9. Receive announcement
   │
   ▼
[Verify on their side]
   │
   │ 10. cosign verify ghcr.io/efe-ai/efe-server:1.0.0
   │ 11. helm install oci://ghcr.io/efe-ai/charts/efe-stack:1.0.0
   │ 12. brew install efe/tap/efe-admin
```

---

## §13. Conclusion

R-100 transforme EFE de produit interne en produit publié, signé, supporté contractuellement. Les **trois piliers** (Sigstore signing + tag/distribution + announce) sont **interdépendants mais découplables** — chaque pilier peut shipper indépendamment si nécessaire, mais leur valeur cumulative est ce qui permet :

- **Trust** : tout artefact installé chez un tenant est cryptographiquement traçable au commit Git source
- **Reach** : 6 distribution channels = couverture quasi-totale des cas d'usage
- **Compliance** : packages prêts-à-l'emploi pour DPO, auditeurs SOC 2, autorités EU AI Act
- **Sustain** : pipeline rollback + hotfix policy + support tiers définis

L'investissement (5 jours-homme + audit pass R-100) débloque la **commercialisation v1.0.0 GA** :
- Les commerciaux peuvent vendre un produit avec un SLA contractuel
- Les SE peuvent onboarder un tenant en 30 min via channels publics
- Les DPO peuvent télécharger leur compliance pack en 1 clic
- Les CISOs peuvent vérifier la signature Sigstore avant déploiement

Le successeur naturel post-R-100 est **M23 (DPO Export + Active Learning)** — désormais possible parce qu'on aura :
- Des tenants réels en suffisamment grand nombre pour générer un dataset DPO significatif
- Une release pipeline qui peut shipper M23 sans friction
- Un canal de communication établi pour annoncer les milestones futurs

---

**Fin du document — Architecture v1.0 design freeze 2026-05-09**
