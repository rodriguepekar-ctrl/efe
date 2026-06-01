# EFE Sprint Go-Live & Tenant Onboarding Self-Service — Architecture Complète

**Codename** : « GLOSS » (**G**o-**L**ive & **O**nboarding **S**elf-**S**ervice)
**Status** : Architecture v1.0 — design freeze
**Auteur** : Architecture EFE Core
**Date** : 2026-05-08
**Cible** : Sprint Go-Live (3 semaines, 1 ingénieur senior)
**Pré-requis** : M16-M22 GA (état actuel du projet)
**Successeur logique** : M23 (DPO Export + Active Learning)

---

## §0. Executive Summary

### 0.1 Mission

GLOSS transforme EFE — produit techniquement complet (12 migrations, 2 877 tests GREEN, compliance EU enforced) — en produit **commercialement déployable** : un Solutions Engineer (SE) doit pouvoir provisionner un tenant POC opérationnel en ≤ 30 minutes depuis un laptop neuf, sans intervention de l'équipe ingénierie EFE.

L'asymétrie actuelle entre **complétude technique** (95 %) et **complétude commerciale** (≈ 25 %) bloque la commercialisation. GLOSS la résorbe en livrant cinq piliers logistiques.

### 0.2 Cinq piliers GLOSS

| # | Pilier | Output | Effort | LOC |
|---|--------|--------|--------|-----|
| 1 | **CLI `efe-admin`** | Binaire Rust statiquement compilé, ~5 MB, 18 commandes | 4 j | ~4 500 |
| 2 | **Helm chart `efe-stack`** | Umbrella + 6 sub-charts, déployable `k3d` local | 3 j | ~3 200 |
| 3 | **Terraform modules** | 3 modules (AWS, GCP, Azure), versionnés sémantique | 4 j | ~3 900 |
| 4 | **Runbook ops** | 12 procédures formalisées, chacune testée | 2 j | ~4 800 |
| 5 | **Portail admin tenant** | Next.js 14 SPA, SSO, 5 pages | 4 j | ~6 500 |
| | **Total** | | **17 j** | **~22 900** |

Ajouter **migrations DB** (~250 LOC), **mcp-server tools D8** (~900 LOC), **tests** (~5 200 LOC) ⇒ **~30 000 LOC totales**.

### 0.3 Non-goals (réservés milestones futurs)

- ❌ Multi-region active-active (M25)
- ❌ DPO export & fine-tuning loop (M23)
- ❌ Eval harness LLM (M24)
- ❌ Multi-cloud abstraction (3 modules Terraform séparés, pas unifiés)
- ❌ UI end-user tenant (portail = admin tenant uniquement, l'API gRPC reste l'interface développeur)
- ❌ Marketplace anchors public inter-tenant
- ❌ Billing automation Stripe/Chargebee
- ❌ Refonte mcp-server (5 tools ajoutés, pas de refactor de l'existant)

### 0.4 Critères d'acceptation (DoD synthèse)

1. ✅ E2E POC tenant : `terraform apply` + `helm install` + `efe-admin tenant create` + `classify` ≤ **30 min**
2. ✅ 100 % des tools mcp-server write requièrent `confirm:true`, 100 % des `efe-admin` write requièrent `--yes` ou TTY confirm
3. ✅ Helm chart déployable sur **k3d** local (laptop 16 GB RAM, 4 vCPU)
4. ✅ 12 runbooks d'incident existent + chacun a une procédure de validation reproduisible
5. ✅ Compliance GDPR Art. 22 + EU AI Act Art. 14 reste **HARD GATE Postgres** enforced
6. ✅ Tests E2E Playwright couvrent flux complet (create tenant → seed → first classify → audit visible UI)
7. ✅ 0 régression M16-M22 (tous tests existants restent GREEN)
8. ✅ Audit pass chirurgical avec ≤ 6 défauts identifiés et corrigés

---

## §1. Contexte & justification

### 1.1 État actuel : friction commerciale

Aujourd'hui, déployer EFE chez un nouveau tenant requiert :

```
1. Ingénieur EFE provisionne infra cloud (AWS/GCP/Azure) à la main      45 min
2. Ingénieur EFE crée DB, applique migrations 0001-0012                  20 min
3. Ingénieur EFE configure Postgres (extensions, replication, etc.)       15 min
4. Ingénieur EFE déploie efe-server + mcp-server (Docker manuel)         30 min
5. Ingénieur EFE configure Redis Stack                                    10 min
6. Ingénieur EFE INSERT INTO tenants ... (SQL direct)                    10 min
7. Ingénieur EFE génère API keys (script ad-hoc)                          5 min
8. Ingénieur EFE seed catalog from JSONL (mcp-server tool)                15 min
9. Ingénieur EFE configure quotas (SQL direct)                            10 min
10. Ingénieur EFE attest AI Act Art. 14 (SQL direct)                       5 min
11. Ingénieur EFE smoke test classify (gRPC manual)                       10 min
12. Documentation handoff au tenant (email/PDF ad-hoc)                    20 min
─────────────────────────────────────────────────────────────────
Total                                                                  3h 15 min
```

Avec **2 ingénieurs en pair** (sécurité + correctness), durée réelle ≈ **6 heures**.

Surcoût annuel pour 50 tenants/an = 50 × 6 h × 2 ing × 200 €/h = **120 000 €/an** purement consommé en onboarding manuel.

### 1.2 État cible : self-service

Avec GLOSS, le SE exécute :

```bash
# Étape 1 : provisionner infra (sortie : kubeconfig + DB credentials)
cd terraform/modules/aws
terraform init
terraform apply -var="tenant=acme"            # ~10 min

# Étape 2 : déployer EFE stack
helm install efe-stack ./charts/efe-stack \
  --values values-acme.yaml                    # ~5 min

# Étape 3 : créer tenant + configurer + seed
efe-admin tenant create acme \
  --plan enterprise \
  --max-rps 1000 \
  --data-residency EU
efe-admin compliance attest acme \
  --ai-act-art14 --gdpr-art22 \
  --signed-by 'jane.doe@acme.com'
efe-admin catalog seed acme \
  --bundle ./acme_anchors.jsonl \
  --bundle-version v1.0.0
efe-admin tenant smoke-test acme               # ~10 min total

# Étape 4 : handoff (UI portail)
echo "Portail : https://portal.efe.example.com/tenants/acme"
```

Total **≤ 30 min**, par 1 SE seul, depuis son laptop. ROI à partir du 5e tenant.

### 1.3 Pourquoi maintenant et pas plus tard

1. **M16-M22 sont GA** : la machinerie technique est figée → on peut emballer sans risque de churn architectural
2. **DPO export (M23) repose sur tenants réels** : sans onboarding scalable, on n'a pas assez de tenants pour générer un dataset DPO significatif
3. **Friction commerciale = bottleneck #1** : les commerciaux ne peuvent pas faire des POC sans monopoliser un ingénieur
4. **Ops maturity** : les 12 runbooks formalisent la connaissance tribale actuellement dispersée dans des Slack threads et docstrings

### 1.4 Anti-pattern à éviter : la « sur-abstraction »

GLOSS livre **3 modules Terraform séparés** (AWS, GCP, Azure), **pas** un module unifié multi-cloud. Justifications :

- Chaque cloud a ses idiosyncrasies (RDS Aurora vs Cloud SQL vs Azure Database for PostgreSQL Flexible Server) qui rendent l'abstraction soit fuyante, soit verbeuse
- Un SE qui maîtrise AWS n'a pas besoin de comprendre GCP ; il appelle directement le module AWS
- Maintenance : 3 modules indépendants × bugs n cloud-spécifiques = O(n), versus 1 module × bugs croisés = O(n²)
- Versioning sémantique indépendant : aws-v1.2.0 peut shipper sans bloquer gcp-v0.9.0

### 1.5 Glossaire GLOSS

| Terme | Définition |
|-------|------------|
| **SE** | Solutions Engineer — celui qui exécute l'onboarding |
| **POC** | Proof of Concept — déploiement de découverte tenant, durée < 30 j |
| **Tenant POC** | Tenant en mode trial, quotas limités, pas de SLA |
| **Tenant production** | Tenant payant, quotas dimensionnés, SLA 99.9 % |
| **Portail admin** | UI Next.js destinée à l'admin tenant (DPO, ITS, etc.) |
| **CLI admin** | `efe-admin` binaire Rust, destiné au SE EFE |
| **Compliance attest** | Signature digitale de l'attestation AI Act Art. 14 + GDPR Art. 22 |
| **Bundle** | Fichier JSONL d'anchors pour seeding catalog |
| **Smoke test** | Suite minimale de classify post-déploiement (≤ 10 calls) |

---

## §2. Architecture cible globale

### 2.1 Diagramme d'ensemble

```
┌──────────────────────────────────────────────────────────────────────┐
│                  TENANT INFRA (cloud du tenant)                      │
│                                                                       │
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │  Kubernetes cluster (EKS / GKE / AKS)                          │  │
│  │                                                                 │  │
│  │  ┌──────────────┐   ┌──────────────┐   ┌──────────────────┐   │  │
│  │  │ efe-server   │   │ mcp-server   │   │ portal-admin     │   │  │
│  │  │  3 replicas  │←──┤  2 replicas  │←──┤  Next.js 14      │   │  │
│  │  │  gRPC :50051 │   │  HTTP :8080  │   │  3 replicas      │   │  │
│  │  └──────┬───────┘   └──────┬───────┘   └──────┬───────────┘   │  │
│  │         │                  │                   │                │  │
│  │  ┌──────▼──────────────────▼──────────────────▼──────────┐    │  │
│  │  │  Postgres 16 (PGO operator, primary + 2 replicas)     │    │  │
│  │  │  Redis Stack 7.4 (HNSW vector search)                 │    │  │
│  │  └────────────────────────────────────────────────────────┘    │  │
│  │                                                                 │  │
│  │  Observability: Prometheus + Grafana + Loki + Tempo            │  │
│  │  Ingress: nginx-ingress + cert-manager (Let's Encrypt)         │  │
│  └────────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │  Cloud-native managed services                                 │  │
│  │  - VPC + subnets (public + private + DB-private)              │  │
│  │  - IAM roles (efe-server-role, mcp-server-role, ...)          │  │
│  │  - KMS / KeyVault / Cloud KMS (BYOK)                          │  │
│  │  - S3/GCS/Azure-Blob (audit chain offsite)                    │  │
│  └────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────┘
            ▲                                      ▲
            │                                      │
            │ kubectl/helm                         │ HTTPS (TLS 1.3)
            │                                      │
┌───────────┴────────────┐              ┌──────────┴──────────┐
│  SE laptop             │              │  Tenant DPO/admin   │
│  - efe-admin CLI       │              │  - Browser SSO      │
│  - kubectl + helm      │              │  - Portail admin    │
│  - terraform           │              │                     │
└────────────────────────┘              └─────────────────────┘
```

### 2.2 Boundaries & responsibilities

| Composant | Owner | Responsabilité | Exposé à |
|-----------|-------|----------------|----------|
| `efe-admin` CLI | SE EFE | Bootstrap tenant + ops curatives | SE local (laptop) |
| `efe-server` | EFE Core | gRPC classify (Mode-1/2/3) | Apps tenant |
| `mcp-server` | EFE Core | HTTP/JSON tooling pour LLM agents | Apps tenant LLM |
| `portal-admin` | EFE Core | UI admin tenant (audit, quotas, keys) | DPO/ITS tenant |
| Helm chart | EFE Core (versionné) | Déploiement K8s reproductible | SE EFE |
| Terraform | EFE Core (versionné) | Provisioning infra cloud | SE EFE |
| Runbooks | EFE Ops | Procédures incident formalisées | On-call EFE + on-call tenant |

### 2.3 Flux d'onboarding "30 min POC" (vue zoomable)

```
          T+00:00                  T+10:00              T+20:00       T+30:00
            │                         │                    │              │
SE laptop   ├─[01]─terraform apply───┤                    │              │
            │                         │                    │              │
            │      ┌─[02]─helm install efe-stack──────────┤              │
            │      │                                        │              │
            │      │      ┌─[03]─efe-admin tenant create──┤              │
            │      │      │                                  │              │
            │      │      │      ┌─[04]─compliance attest──┤              │
            │      │      │      │                            │              │
            │      │      │      │      ┌─[05]─catalog seed─┤              │
            │      │      │      │      │                      │              │
            │      │      │      │      │      ┌─[06]─smoke test───────────┤
            │      │      │      │      │      │                            │
            │      │      │      │      │      └─[07]─handoff URL portail─┤
            │      │      │      │      │                                    │
            ▼      ▼      ▼      ▼      ▼      ▼                            ▼
            VPC    K8s    DB     Compl. Catalog Classify                  POC OK
            +DB    pods   row    rows   rows    works                     ✅
            +Redis ready  ready  ready  ready   ready
```

### 2.4 Versioning de chaque pilier

| Pilier | Versioning | Contraintes |
|--------|-----------|-------------|
| `efe-admin` | semver, suit efe-server (couplage tight via gRPC) | tag git `efe-admin/vX.Y.Z` |
| `efe-stack` Helm | semver indépendant, AppVersion = efe-server version | OCI registry `oci://ghcr.io/efe/charts/efe-stack` |
| Terraform AWS/GCP/Azure | semver indépendant **par module** | Terraform Registry privée `efe/efe-stack-aws/aws` |
| Portail admin | semver, suit efe-server (couplage UI ↔ gRPC) | Docker tag `efe/portal-admin:vX.Y.Z` |
| Runbooks | versionnés par révision Markdown | `docs/runbooks/RUNBOOK_*.md` + git history |

### 2.5 Compatibilité backward strict

Toute la livraison GLOSS doit être **opt-in**. Concrètement :

- Les tenants existants (sans portail admin) continuent à utiliser le `mcp-server` directement
- Le Helm chart est **optionnel** : un tenant qui déploie EFE en Docker Compose continue à fonctionner
- La migration `0013` ajoute uniquement des tables et colonnes, **jamais ne modifie** les tables M16-M22
- L'`efe-admin` CLI **encapsule** les outils mcp-server existants (n'introduit pas de nouvelle authority pattern)

Test d'invariant : « Un tenant déployé en M22 (sans aucun composant GLOSS) doit continuer à passer 100 % des tests M16-M22 après merge GLOSS ». Test inscrit dans `tests/test_global_backward_compat.py`.

---

## §3. CLI `efe-admin` — détails complets

### 3.1 Vue d'ensemble

`efe-admin` est un binaire Rust statiquement compilé (musl), ~5 MB, distribué via :

- GitHub Releases : `efe-admin-vX.Y.Z-{linux,darwin,windows}-{amd64,arm64}`
- Homebrew : `brew install efe/tap/efe-admin`
- `cargo install efe-admin`
- Docker : `docker run --rm efe/admin:vX.Y.Z ...`

### 3.2 Crate structure

```
crates/efe-admin/
├── Cargo.toml
├── src/
│   ├── main.rs                    # entry point + CLI parsing (clap derive)
│   ├── lib.rs
│   ├── config.rs                  # ~/.config/efe-admin/config.toml loader
│   ├── error.rs                   # AdminError typed enum
│   ├── auth/
│   │   ├── mod.rs
│   │   ├── token.rs               # bearer token mgmt (file + env)
│   │   └── sso.rs                 # OAuth2 device flow (post-MVP)
│   ├── commands/
│   │   ├── mod.rs
│   │   ├── tenant.rs              # tenant create/list/get/delete/update
│   │   ├── catalog.rs             # catalog seed/list/promote
│   │   ├── compliance.rs          # compliance attest/verify
│   │   ├── keys.rs                # api-key create/rotate/revoke
│   │   ├── quotas.rs              # quotas get/set
│   │   ├── audit.rs               # audit query/export
│   │   ├── smoke.rs               # smoke-test
│   │   ├── health.rs              # health
│   │   └── version.rs             # version
│   ├── grpc/
│   │   ├── mod.rs
│   │   ├── client.rs              # tonic client wrapper
│   │   └── reconnect.rs           # exp backoff reconnect logic
│   ├── mcp/
│   │   ├── mod.rs
│   │   └── client.rs              # HTTP client to mcp-server
│   ├── output/
│   │   ├── mod.rs
│   │   ├── table.rs               # comfy-table renderer
│   │   ├── json.rs
│   │   └── yaml.rs
│   └── prompts/
│       ├── mod.rs
│       └── confirm.rs             # interactive confirm (--yes bypass)
└── tests/
    ├── integration/
    │   ├── tenant_lifecycle.rs
    │   ├── compliance_flow.rs
    │   └── smoke.rs
    └── fixtures/
        └── *.jsonl
```

### 3.3 Commandes (18 au total)

```
efe-admin
├── tenant
│   ├── create <name> [--plan PLAN] [--max-rps N] [--data-residency R]
│   ├── list [--filter STATUS] [--format json|yaml|table]
│   ├── get <name> [--format ...]
│   ├── update <name> [--max-rps N] [--data-residency R]
│   ├── suspend <name> [--reason TEXT] (--yes)
│   └── delete <name> (--yes) [--purge-audit-after-days N]
├── catalog
│   ├── seed <tenant> --bundle FILE.jsonl --bundle-version VERSION (--yes)
│   ├── list <tenant> [--format ...]
│   └── promote <tenant> --catalog ID --to-version VERSION (--yes)
├── compliance
│   ├── attest <tenant> --ai-act-art14 --gdpr-art22 --signed-by EMAIL (--yes)
│   └── verify <tenant>
├── keys
│   ├── create <tenant> --scope SCOPE [--expires-in DURATION]
│   ├── rotate <tenant> --key-id ID (--yes)
│   └── revoke <tenant> --key-id ID --reason TEXT (--yes)
├── quotas
│   ├── get <tenant>
│   └── set <tenant> --max-rps N --max-tokens-per-day N (--yes)
├── audit
│   ├── query <tenant> [--since DATE] [--mode MODE] [--limit N]
│   └── export <tenant> --since DATE --to FILE.jsonl
├── smoke-test <tenant>
├── health [--cluster URL]
└── version
```

Chaque commande qui modifie l'état requiert **soit `--yes`, soit confirmation interactive TTY** (Y/N) — invariant matériel inscrit dans le test `tests/integration/confirm_required.rs`.

### 3.4 Configuration : `~/.config/efe-admin/config.toml`

```toml
[default]
cluster = "https://efe.example.com"
auth_method = "token"             # "token" | "sso" | "kube_sa"
token_file = "~/.config/efe-admin/token"
output_format = "table"            # "table" | "json" | "yaml"
confirm = "interactive"            # "interactive" | "always_yes" | "always_no"

[clusters.acme]
cluster = "https://efe.acme.com"
auth_method = "token"
token_file = "~/.config/efe-admin/acme.token"

[clusters.local]
cluster = "http://localhost:50051"
auth_method = "none"
```

Sélection cluster via `--cluster acme` ou variable d'environnement `EFE_ADMIN_CLUSTER=acme`.

### 3.5 Authentification

Trois modes :

1. **Bearer token statique** (default POC) : `token_file` lit un fichier 0600 contenant un JWT court (≤ 1 h). Rotation manuelle par le SE.
2. **OAuth2 device flow** (production) : `efe-admin login` ouvre un navigateur, l'utilisateur s'authentifie auprès de l'IdP (Keycloak/Okta/AzureAD), récupère un access_token + refresh_token stockés chiffrés (`age` ou Keychain macOS / Credential Manager Windows / GNOME Keyring).
3. **Kubernetes SA token** (CI/CD) : si `KUBERNETES_SERVICE_ACCOUNT_TOKEN` présent, on l'utilise directement (mode in-cluster).

Le test `tests/integration/auth_modes.rs` couvre les 3 chemins.

### 3.6 RBAC enforcement

Toutes les commandes write s'appuient sur les permissions tokenisées (claim `efe:scopes`) :

| Commande | Scope requis |
|----------|--------------|
| `tenant create/delete/suspend` | `efe:admin:tenants:write` |
| `catalog seed/promote` | `efe:admin:catalogs:write` |
| `compliance attest` | `efe:admin:compliance:attest` |
| `keys create/rotate/revoke` | `efe:admin:keys:write` |
| `quotas set` | `efe:admin:quotas:write` |
| `audit export` | `efe:admin:audit:read` |
| `tenant list/get`, `audit query` | `efe:admin:read` |
| `smoke-test`, `health`, `version` | aucun |

Les claims sont vérifiés **côté serveur** (mcp-server / efe-server gRPC) — le CLI n'est pas un trust boundary, il propage juste le token.

### 3.7 Output & UX

- **Default** : table colorée (comfy-table) avec colonnes principales
- **`--format json`** : output strict JSON, jq-friendly, stable schema (versionné `apiVersion: efe.io/v1`)
- **`--format yaml`** : output YAML (cas-d'usage : copy-paste dans values.yaml Helm)
- **`-q, --quiet`** : output minimum, scriptable
- **`-v, --verbose`** : log debug stderr
- **Exit codes** : 0 success, 1 user error, 2 server error, 3 auth error, 64 syntax (sysexits.h)

Exemple :

```
$ efe-admin tenant list --format table
┌──────────┬─────────────┬────────────┬──────────┬──────────────────────┐
│ name     │ plan        │ status     │ max_rps  │ created_at           │
├──────────┼─────────────┼────────────┼──────────┼──────────────────────┤
│ acme     │ enterprise  │ active     │ 1000     │ 2026-04-12T10:23:11Z │
│ globex   │ trial       │ active     │ 100      │ 2026-04-30T15:01:42Z │
│ initech  │ enterprise  │ suspended  │ 0        │ 2026-03-08T09:14:00Z │
└──────────┴─────────────┴────────────┴──────────┴──────────────────────┘
3 tenants (active=2, suspended=1)
```

### 3.8 Test harness

- **Unit tests** : ~140 (parsing args, config loading, output formatters, auth)
- **Integration tests** : ~50 avec **testcontainers** (Postgres 16 + Redis Stack + efe-server image + mcp-server image — full stack ephemeral)
- **Fixtures** : `tests/fixtures/acme_anchors.jsonl` (50 anchors), `globex_anchors.jsonl` (200 anchors, edge cases)
- **CI matrix** : Linux x86_64, macOS arm64, Windows x86_64 — 3 OS

### 3.9 Distribution & release

- Cargo workspace member : `crates/efe-admin/`
- Build artifact : `cargo build --release --target x86_64-unknown-linux-musl`
- GitHub Action `.github/workflows/release-efe-admin.yml` :
  - Trigger : `git tag efe-admin/v*`
  - Build matrix 6 cibles (linux/darwin/windows × amd64/arm64 sauf windows-arm64)
  - Sign avec `cosign sign-blob` (cosign 2.x, keyless OIDC)
  - Upload to GitHub Releases avec checksums SHA256 + cosign signatures
  - Update Homebrew tap automatiquement

### 3.10 Détail commande critique : `tenant create`

```rust
// crates/efe-admin/src/commands/tenant.rs (extrait)
#[derive(clap::Args, Debug)]
pub struct TenantCreateArgs {
    /// Tenant name (slug, [a-z0-9-]+, 3-32 chars)
    pub name: String,

    /// Tenant plan
    #[arg(long, value_enum, default_value = "trial")]
    pub plan: TenantPlan,

    /// Max requests per second
    #[arg(long, default_value = "100")]
    pub max_rps: u32,

    /// Data residency requirement
    #[arg(long, value_enum, default_value = "any")]
    pub data_residency: DataResidency,

    /// Max tokens per day for LLM-routed modes
    #[arg(long, default_value = "100000")]
    pub max_tokens_per_day: u64,

    /// Skip interactive confirmation
    #[arg(long, short)]
    pub yes: bool,
}

pub async fn run(args: TenantCreateArgs, ctx: &AdminContext) -> Result<(), AdminError> {
    // 1. Validate slug
    validate_tenant_slug(&args.name)?;

    // 2. Confirm if interactive
    if !args.yes && !ctx.config.confirm_bypass() {
        let prompt = format!(
            "Create tenant '{}' with plan={:?}, max_rps={}, residency={:?} ?",
            args.name, args.plan, args.max_rps, args.data_residency
        );
        if !confirm_interactive(&prompt)? {
            return Err(AdminError::UserAbort);
        }
    }

    // 3. Call mcp-server tool m23_create_tenant (write, confirm:true)
    let resp = ctx.mcp.call_tool("m23_create_tenant", json!({
        "name": args.name,
        "plan": args.plan,
        "max_rps": args.max_rps,
        "data_residency": args.data_residency,
        "max_tokens_per_day": args.max_tokens_per_day,
        "confirm": true,
    })).await?;

    // 4. Output result
    ctx.output.print_tenant_create_result(&resp)?;
    Ok(())
}
```

Test d'intégration associé :

```rust
#[tokio::test]
async fn test_tenant_create_happy_path() {
    let stack = TestStack::start().await;            // testcontainers
    let cli = AdminCli::with_endpoint(&stack.mcp_url);

    let output = cli.run(&[
        "tenant", "create", "acme",
        "--plan", "enterprise",
        "--max-rps", "1000",
        "--yes",
    ]).await.expect("create OK");
    assert!(output.contains("tenant 'acme' created"));

    // Verify in DB
    let row = stack.db.query_one(
        "SELECT plan, max_rps FROM tenants WHERE name = 'acme'", &[]).await.unwrap();
    assert_eq!(row.get::<_, String>("plan"), "enterprise");
    assert_eq!(row.get::<_, i32>("max_rps"), 1000);
}
```

---

## §4. Helm chart `efe-stack`

### 4.1 Vue d'ensemble

Umbrella chart qui assemble :

```
charts/efe-stack/                       # umbrella v1.0.0
├── Chart.yaml
├── values.yaml                         # values agrégés
├── values.schema.json                  # JSON schema validation
├── templates/
│   └── NOTES.txt                       # post-install message
└── charts/                             # sub-charts (vendored or dependency)
    ├── efe-server/                     # v1.0.0
    ├── mcp-server/                     # v1.0.0
    ├── portal-admin/                   # v1.0.0
    ├── postgres-operator/              # v1.0.0 (CrunchyData PGO 5.x)
    ├── redis-stack/                    # v1.0.0
    └── observability/                  # v1.0.0 (kube-prometheus + Loki + Tempo)
```

### 4.2 Architecture sub-charts

#### 4.2.1 `efe-server` sub-chart

```yaml
# charts/efe-stack/charts/efe-server/templates/deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "efe-server.fullname" . }}
  labels:
    {{- include "efe-server.labels" . | nindent 4 }}
spec:
  replicas: {{ .Values.replicas | default 3 }}
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxSurge: 1
      maxUnavailable: 0
  selector:
    matchLabels:
      {{- include "efe-server.selectorLabels" . | nindent 6 }}
  template:
    metadata:
      labels:
        {{- include "efe-server.selectorLabels" . | nindent 8 }}
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/port: "9090"
        prometheus.io/path: "/metrics"
    spec:
      serviceAccountName: {{ include "efe-server.serviceAccountName" . }}
      securityContext:
        runAsNonRoot: true
        runAsUser: 65532
        runAsGroup: 65532
        fsGroup: 65532
        seccompProfile:
          type: RuntimeDefault
      containers:
      - name: efe-server
        image: "{{ .Values.image.repository }}:{{ .Values.image.tag | default .Chart.AppVersion }}"
        imagePullPolicy: {{ .Values.image.pullPolicy }}
        ports:
        - name: grpc
          containerPort: 50051
        - name: metrics
          containerPort: 9090
        - name: health
          containerPort: 8081
        env:
        - name: EFE_DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: {{ include "efe-server.fullname" . }}-db
              key: connection_string
        - name: EFE_REDIS_URL
          valueFrom:
            secretKeyRef:
              name: {{ include "efe-server.fullname" . }}-redis
              key: connection_string
        - name: EFE_BYOK_BACKEND
          value: {{ .Values.byok.backend | default "env" }}
        {{- if eq .Values.byok.backend "aws-kms" }}
        - name: EFE_BYOK_AWS_KMS_KEY_ID
          value: {{ .Values.byok.awsKmsKeyId }}
        {{- end }}
        livenessProbe:
          httpGet:
            path: /healthz
            port: health
          initialDelaySeconds: 10
          periodSeconds: 5
          failureThreshold: 3
        readinessProbe:
          httpGet:
            path: /readyz
            port: health
          initialDelaySeconds: 5
          periodSeconds: 2
        resources:
          {{- toYaml .Values.resources | nindent 10 }}
        securityContext:
          allowPrivilegeEscalation: false
          readOnlyRootFilesystem: true
          capabilities:
            drop: ["ALL"]
        volumeMounts:
        - name: tmp
          mountPath: /tmp
        - name: cache
          mountPath: /var/cache/efe
      volumes:
      - name: tmp
        emptyDir: {}
      - name: cache
        emptyDir:
          sizeLimit: 1Gi
      topologySpreadConstraints:
      - maxSkew: 1
        topologyKey: topology.kubernetes.io/zone
        whenUnsatisfiable: DoNotSchedule
        labelSelector:
          matchLabels:
            {{- include "efe-server.selectorLabels" . | nindent 12 }}
```

Avec **PodDisruptionBudget** :

```yaml
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: {{ include "efe-server.fullname" . }}
spec:
  minAvailable: 2                       # 3 replicas → at least 2 always
  selector:
    matchLabels:
      {{- include "efe-server.selectorLabels" . | nindent 6 }}
```

Et **HorizontalPodAutoscaler** (HPA) optionnel :

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: {{ include "efe-server.fullname" . }}
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: {{ include "efe-server.fullname" . }}
  minReplicas: 3
  maxReplicas: {{ .Values.autoscaling.maxReplicas | default 12 }}
  metrics:
  - type: Resource
    resource:
      name: cpu
      target:
        type: Utilization
        averageUtilization: 70
  - type: Pods
    pods:
      metric:
        name: efe_classify_rps
      target:
        type: AverageValue
        averageValue: 200
```

#### 4.2.2 Postgres via CrunchyData PGO

```yaml
# charts/efe-stack/charts/postgres-operator/templates/postgrescluster.yaml
apiVersion: postgres-operator.crunchydata.com/v1beta1
kind: PostgresCluster
metadata:
  name: efe-postgres
spec:
  postgresVersion: 16
  instances:
  - name: instance1
    replicas: {{ .Values.postgres.replicas | default 3 }}
    dataVolumeClaimSpec:
      accessModes: ["ReadWriteOnce"]
      resources:
        requests:
          storage: {{ .Values.postgres.storage | default "100Gi" }}
      storageClassName: {{ .Values.postgres.storageClass }}
    affinity:
      podAntiAffinity:
        requiredDuringSchedulingIgnoredDuringExecution:
        - labelSelector:
            matchLabels:
              postgres-operator.crunchydata.com/cluster: efe-postgres
          topologyKey: topology.kubernetes.io/zone
  backups:
    pgbackrest:
      repos:
      - name: repo1
        s3:
          bucket: {{ .Values.postgres.backup.bucket }}
          endpoint: {{ .Values.postgres.backup.endpoint }}
          region: {{ .Values.postgres.backup.region }}
        schedules:
          full: "0 1 * * 0"               # weekly full Sunday 01:00 UTC
          differential: "0 1 * * 1-6"     # daily differential
          incremental: "0 */4 * * *"      # 4-hourly
  users:
  - name: efe
    databases:
    - efe
    options: "SUPERUSER"                  # required for migrations 0001..0013
```

L'opérateur PGO gère :
- Streaming replication primary → 2 replicas (sync)
- Backup automated S3/GCS via pgBackRest (PITR ≤ 5 min)
- Failover automatique (Patroni-style)
- Rolling minor version upgrade

#### 4.2.3 Redis Stack StatefulSet

```yaml
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: {{ include "redis-stack.fullname" . }}
spec:
  replicas: 1                             # MVP single-node, sentinel post-MVP
  serviceName: {{ include "redis-stack.fullname" . }}
  template:
    spec:
      containers:
      - name: redis-stack
        image: redis/redis-stack-server:7.4-v0
        ports:
        - containerPort: 6379
        volumeMounts:
        - name: data
          mountPath: /data
        resources:
          requests:
            cpu: 500m
            memory: 2Gi
          limits:
            cpu: 2
            memory: 8Gi
  volumeClaimTemplates:
  - metadata:
      name: data
    spec:
      accessModes: ["ReadWriteOnce"]
      resources:
        requests:
          storage: 50Gi
```

#### 4.2.4 Observability stack (kube-prometheus + Loki + Tempo)

Vendored à partir de `kube-prometheus-stack` upstream + add-ons :

- **Prometheus** : scrape efe-server `/metrics` + mcp-server `/metrics` + portail `/api/metrics`
- **Grafana** : 3 dashboards pré-installés
  - `EFE Overview` : RPS, p50/p99 latency, error rate, breaker state
  - `EFE Mode-3` : convergence rate, turn distribution, review queue depth
  - `EFE Compliance` : HMAC chain verification status, attestations, key rotations
- **Loki** : log aggregation, label `app` + `tenant`
- **Tempo** : distributed tracing OpenTelemetry, sampling 1 % production / 100 % dev

### 4.3 `values.yaml` — schéma complet

```yaml
# charts/efe-stack/values.yaml
global:
  domain: efe.example.com                 # CNAME root for ingress
  imageRegistry: ghcr.io/efe              # override for air-gapped
  imagePullSecrets: []
  storageClass: gp3                       # default storage class

efe-server:
  enabled: true
  replicas: 3
  image:
    repository: ghcr.io/efe/efe-server
    tag: ""                                # defaults to chart appVersion
    pullPolicy: IfNotPresent
  resources:
    requests:
      cpu: 500m
      memory: 1Gi
    limits:
      cpu: 2
      memory: 4Gi
  byok:
    backend: aws-kms                       # env|file|aws-kms|gcp-kms|azure-kv|vault|pkcs11
    awsKmsKeyId: ""
  autoscaling:
    enabled: true
    maxReplicas: 12

mcp-server:
  enabled: true
  replicas: 2
  image:
    repository: ghcr.io/efe/mcp-server
    tag: ""

portal-admin:
  enabled: true
  replicas: 3
  image:
    repository: ghcr.io/efe/portal-admin
    tag: ""
  ingress:
    enabled: true
    host: portal.{{ .Values.global.domain }}
    tls:
      enabled: true
      issuer: letsencrypt-prod
  sso:
    issuer: https://idp.example.com/realms/efe
    clientId: efe-portal
    clientSecret: ""                       # populated via External Secrets

postgres:
  enabled: true
  version: 16
  replicas: 3
  storage: 100Gi
  storageClass: ""                        # uses global.storageClass if empty
  backup:
    bucket: efe-postgres-backup
    endpoint: s3.amazonaws.com
    region: eu-west-1
  monitoring:
    enabled: true

redis:
  enabled: true
  storage: 50Gi
  resources:
    requests:
      cpu: 500m
      memory: 2Gi

observability:
  enabled: true
  prometheus:
    retention: 30d
  loki:
    retention: 30d
  tempo:
    samplingRate: 0.01
```

`values.schema.json` (~ 600 lignes) **valide** chaque champ via JSON Schema 2020-12 ; tout `helm install` avec une valeur invalide est **bloqué** au templating.

### 4.4 Test sur k3d local

```bash
# Provision local cluster (4 nodes)
k3d cluster create efe-local \
  --servers 1 --agents 3 \
  --port "8443:443@loadbalancer" \
  --wait

# Install
helm install efe-stack ./charts/efe-stack \
  --values values-local.yaml \
  --wait --timeout 10m

# Run smoke test
efe-admin tenant create test --plan trial --yes
efe-admin smoke-test test
```

`values-local.yaml` réduit ressources :

```yaml
efe-server: { replicas: 1, autoscaling: { enabled: false } }
mcp-server: { replicas: 1 }
portal-admin: { replicas: 1 }
postgres: { replicas: 1, storage: 10Gi }
redis: { storage: 5Gi }
observability:
  prometheus: { retention: 1d }
  loki: { retention: 1d }
```

Test E2E `tests/e2e/k3d_local.sh` (~ 200 lignes bash) automatise tout. Tourne dans GitHub Actions avec `k3d-action`.

### 4.5 NetworkPolicies & Pod Security Standards

NetworkPolicy par défaut « deny-all » + allowlist explicit :

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: efe-server-allow
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/name: efe-server
  policyTypes: [Ingress, Egress]
  ingress:
  - from:
    - podSelector:
        matchLabels:
          app.kubernetes.io/name: mcp-server
    ports:
    - protocol: TCP
      port: 50051
  egress:
  - to:
    - podSelector:
        matchLabels:
          postgres-operator.crunchydata.com/cluster: efe-postgres
    ports:
    - protocol: TCP
      port: 5432
  - to:
    - podSelector:
        matchLabels:
          app.kubernetes.io/name: redis-stack
    ports:
    - protocol: TCP
      port: 6379
  - to:                                   # DNS
    ports:
    - protocol: UDP
      port: 53
```

Pod Security Standard `restricted` enforced :

```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: efe
  labels:
    pod-security.kubernetes.io/enforce: restricted
    pod-security.kubernetes.io/audit: restricted
    pod-security.kubernetes.io/warn: restricted
```

---

## §5. Terraform modules (AWS / GCP / Azure)

### 5.1 Layout

```
terraform/
├── modules/
│   ├── aws/
│   │   ├── main.tf
│   │   ├── variables.tf
│   │   ├── outputs.tf
│   │   ├── versions.tf
│   │   ├── vpc.tf
│   │   ├── eks.tf
│   │   ├── rds.tf
│   │   ├── elasticache.tf
│   │   ├── kms.tf
│   │   ├── iam.tf
│   │   ├── s3.tf
│   │   ├── route53.tf
│   │   ├── README.md
│   │   └── examples/
│   │       └── basic/
│   │           └── main.tf
│   ├── gcp/                             # symetric layout
│   └── azure/                           # symetric layout
└── README.md
```

### 5.2 Module AWS — variables clés

```hcl
# terraform/modules/aws/variables.tf
variable "region" {
  description = "AWS region"
  type        = string
}

variable "tenant" {
  description = "Tenant slug, used as resource prefix"
  type        = string
  validation {
    condition     = can(regex("^[a-z0-9-]{3,32}$", var.tenant))
    error_message = "Tenant slug must match [a-z0-9-]{3,32}."
  }
}

variable "vpc_cidr" {
  type    = string
  default = "10.42.0.0/16"
}

variable "eks_version" {
  type    = string
  default = "1.30"
}

variable "eks_node_groups" {
  type = map(object({
    instance_types = list(string)
    desired_size   = number
    min_size       = number
    max_size       = number
    capacity_type  = string             # "ON_DEMAND" | "SPOT"
  }))
  default = {
    primary = {
      instance_types = ["m6i.xlarge"]
      desired_size   = 3
      min_size       = 3
      max_size       = 12
      capacity_type  = "ON_DEMAND"
    }
  }
}

variable "rds_instance_class" {
  type    = string
  default = "db.r6i.large"
}

variable "rds_allocated_storage" {
  type    = number
  default = 100
}

variable "rds_multi_az" {
  type    = bool
  default = true
}

variable "data_residency" {
  type    = string
  default = "any"
  validation {
    condition     = contains(["EU_ONLY", "US_ONLY", "any"], var.data_residency)
    error_message = "Must be one of: EU_ONLY, US_ONLY, any"
  }
}

variable "enable_kms_byok" {
  description = "Provision a customer-managed KMS key for BYOK"
  type        = bool
  default     = true
}

variable "tags" {
  type    = map(string)
  default = {}
}
```

### 5.3 Module AWS — main.tf (extrait)

```hcl
# terraform/modules/aws/main.tf
locals {
  prefix = "efe-${var.tenant}"
  common_tags = merge({
    "efe.io/tenant"         = var.tenant
    "efe.io/managed-by"     = "terraform"
    "efe.io/data-residency" = var.data_residency
  }, var.tags)
}

# Validate region matches data_residency
data "aws_region" "current" {}

resource "null_resource" "data_residency_check" {
  count = var.data_residency != "any" ? 1 : 0
  lifecycle {
    precondition {
      condition = (var.data_residency == "EU_ONLY"
                   ? startswith(data.aws_region.current.name, "eu-")
                   : startswith(data.aws_region.current.name, "us-"))
      error_message = "Region ${data.aws_region.current.name} doesn't match data_residency=${var.data_residency}"
    }
  }
}

module "vpc" {
  source = "terraform-aws-modules/vpc/aws"
  version = "~> 5.5"

  name = "${local.prefix}-vpc"
  cidr = var.vpc_cidr
  azs  = slice(data.aws_availability_zones.available.names, 0, 3)

  private_subnets = [
    cidrsubnet(var.vpc_cidr, 4, 0),
    cidrsubnet(var.vpc_cidr, 4, 1),
    cidrsubnet(var.vpc_cidr, 4, 2),
  ]
  public_subnets = [
    cidrsubnet(var.vpc_cidr, 4, 4),
    cidrsubnet(var.vpc_cidr, 4, 5),
    cidrsubnet(var.vpc_cidr, 4, 6),
  ]
  database_subnets = [
    cidrsubnet(var.vpc_cidr, 4, 8),
    cidrsubnet(var.vpc_cidr, 4, 9),
    cidrsubnet(var.vpc_cidr, 4, 10),
  ]

  enable_nat_gateway = true
  single_nat_gateway = false              # 1 NAT per AZ for HA
  enable_vpn_gateway = false
  enable_dns_hostnames = true

  tags = local.common_tags
}

module "eks" {
  source  = "terraform-aws-modules/eks/aws"
  version = "~> 20.0"

  cluster_name    = "${local.prefix}-eks"
  cluster_version = var.eks_version
  vpc_id          = module.vpc.vpc_id
  subnet_ids      = module.vpc.private_subnets

  enable_irsa = true                      # IAM Roles for Service Accounts

  cluster_endpoint_private_access = true
  cluster_endpoint_public_access  = true   # POC ; set false for production
  cluster_endpoint_public_access_cidrs = var.eks_public_cidrs

  cluster_addons = {
    coredns                = { most_recent = true }
    kube-proxy             = { most_recent = true }
    vpc-cni                = { most_recent = true }
    aws-ebs-csi-driver     = { most_recent = true }
  }

  eks_managed_node_groups = {
    for k, v in var.eks_node_groups : k => {
      instance_types = v.instance_types
      desired_size   = v.desired_size
      min_size       = v.min_size
      max_size       = v.max_size
      capacity_type  = v.capacity_type
      labels = { "efe.io/role" = k }
    }
  }

  tags = local.common_tags
}

resource "aws_db_subnet_group" "efe" {
  name       = "${local.prefix}-db-subnet"
  subnet_ids = module.vpc.database_subnets
  tags       = local.common_tags
}

resource "aws_security_group" "rds" {
  name   = "${local.prefix}-rds-sg"
  vpc_id = module.vpc.vpc_id
  ingress {
    from_port       = 5432
    to_port         = 5432
    protocol        = "tcp"
    security_groups = [module.eks.cluster_primary_security_group_id]
  }
  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
  tags = local.common_tags
}

resource "aws_kms_key" "rds" {
  description             = "RDS encryption for ${local.prefix}"
  deletion_window_in_days = 30
  enable_key_rotation     = true
  tags                    = local.common_tags
}

resource "aws_db_instance" "efe" {
  identifier              = "${local.prefix}-postgres"
  engine                  = "postgres"
  engine_version          = "16.3"
  instance_class          = var.rds_instance_class
  allocated_storage       = var.rds_allocated_storage
  storage_type            = "gp3"
  storage_encrypted       = true
  kms_key_id              = aws_kms_key.rds.arn
  multi_az                = var.rds_multi_az
  db_subnet_group_name    = aws_db_subnet_group.efe.name
  vpc_security_group_ids  = [aws_security_group.rds.id]
  username                = "efe_admin"
  password                = random_password.rds.result
  backup_retention_period = 14
  backup_window           = "01:00-02:00"
  maintenance_window      = "sun:03:00-sun:04:00"
  performance_insights_enabled = true
  performance_insights_kms_key_id = aws_kms_key.rds.arn
  performance_insights_retention_period = 7
  enabled_cloudwatch_logs_exports = ["postgresql"]
  deletion_protection      = true        # SE must explicitly disable to delete
  copy_tags_to_snapshot    = true
  skip_final_snapshot      = false
  final_snapshot_identifier = "${local.prefix}-final"
  apply_immediately        = false
  tags                     = local.common_tags
}

resource "random_password" "rds" {
  length  = 48
  special = true
  override_special = "!#$%&*()-_=+[]{}<>?"
}

resource "aws_secretsmanager_secret" "rds_credentials" {
  name = "${local.prefix}/rds/credentials"
  kms_key_id = aws_kms_key.rds.arn
  tags = local.common_tags
}

resource "aws_secretsmanager_secret_version" "rds_credentials" {
  secret_id = aws_secretsmanager_secret.rds_credentials.id
  secret_string = jsonencode({
    username = aws_db_instance.efe.username
    password = random_password.rds.result
    host     = aws_db_instance.efe.address
    port     = aws_db_instance.efe.port
    dbname   = "efe"
  })
}

resource "aws_elasticache_subnet_group" "efe" {
  name       = "${local.prefix}-redis-subnet"
  subnet_ids = module.vpc.database_subnets
  tags       = local.common_tags
}

resource "aws_elasticache_replication_group" "efe" {
  replication_group_id       = "${local.prefix}-redis"
  description                = "EFE Redis Stack for ${var.tenant}"
  engine                     = "redis"
  engine_version             = "7.1"
  node_type                  = "cache.r7g.large"
  num_cache_clusters         = 2          # primary + replica
  parameter_group_name       = "default.redis7"
  port                       = 6379
  subnet_group_name          = aws_elasticache_subnet_group.efe.name
  security_group_ids         = [aws_security_group.redis.id]
  at_rest_encryption_enabled = true
  transit_encryption_enabled = true
  auth_token                 = random_password.redis.result
  automatic_failover_enabled = true
  multi_az_enabled           = true
  snapshot_retention_limit   = 7
  snapshot_window            = "02:00-03:00"
  tags                       = local.common_tags
}

resource "aws_kms_key" "byok" {
  count                   = var.enable_kms_byok ? 1 : 0
  description             = "EFE BYOK key for ${var.tenant}"
  deletion_window_in_days = 30
  enable_key_rotation     = true
  policy = data.aws_iam_policy_document.byok_policy[0].json
  tags = merge(local.common_tags, {
    "efe.io/purpose" = "byok"
  })
}

resource "aws_kms_alias" "byok" {
  count         = var.enable_kms_byok ? 1 : 0
  name          = "alias/${local.prefix}-byok"
  target_key_id = aws_kms_key.byok[0].key_id
}

resource "aws_s3_bucket" "audit" {
  bucket = "${local.prefix}-audit"
  tags   = local.common_tags
}

resource "aws_s3_bucket_versioning" "audit" {
  bucket = aws_s3_bucket.audit.id
  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_lifecycle_configuration" "audit" {
  bucket = aws_s3_bucket.audit.id
  rule {
    id     = "retention-7y"
    status = "Enabled"
    transition {
      days          = 90
      storage_class = "STANDARD_IA"
    }
    transition {
      days          = 365
      storage_class = "GLACIER"
    }
    expiration {
      days = 2555                          # 7 years
    }
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "audit" {
  bucket = aws_s3_bucket.audit.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm     = "aws:kms"
      kms_master_key_id = var.enable_kms_byok ? aws_kms_key.byok[0].arn : null
    }
    bucket_key_enabled = true
  }
}

resource "aws_s3_bucket_public_access_block" "audit" {
  bucket                  = aws_s3_bucket.audit.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

# Output kubeconfig and DB connection string for next steps (Helm install)
output "kubeconfig_command" {
  value = "aws eks update-kubeconfig --region ${var.region} --name ${module.eks.cluster_name}"
}

output "db_secret_arn" {
  value = aws_secretsmanager_secret.rds_credentials.arn
}

output "byok_kms_key_arn" {
  value = var.enable_kms_byok ? aws_kms_key.byok[0].arn : null
}

output "audit_bucket_arn" {
  value = aws_s3_bucket.audit.arn
}
```

### 5.4 Module GCP — différences saillantes

- VPC : `google_compute_network` + `google_compute_subnetwork` (pas de subnets explicites multi-AZ ; GCP régionalise nativement)
- GKE : `google_container_cluster` + `google_container_node_pool`, autopilot mode possible
- Cloud SQL Postgres : `google_sql_database_instance` avec `availability_type = "REGIONAL"`
- Memorystore Redis : `google_redis_instance` (Redis 7.x, pas Redis Stack — limitation, on installe Redis Stack via Helm si modules vector search nécessaires)
- KMS : `google_kms_key_ring` + `google_kms_crypto_key` (BYOK)
- GCS : `google_storage_bucket` (audit chain offsite)

### 5.5 Module Azure — différences saillantes

- VNet : `azurerm_virtual_network` + `azurerm_subnet`
- AKS : `azurerm_kubernetes_cluster` + node pools
- Azure Database for PostgreSQL Flexible Server : `azurerm_postgresql_flexible_server`
- Azure Cache for Redis : `azurerm_redis_cache` (Premium tier requis pour modules Stack)
- KeyVault : `azurerm_key_vault` + `azurerm_key_vault_key`
- Azure Blob : `azurerm_storage_account` + `azurerm_storage_container`

### 5.6 Versioning sémantique des modules

Chaque module est publié indépendamment :

```
terraform/modules/aws/   → tag git: tf-aws/v1.0.0
terraform/modules/gcp/   → tag git: tf-gcp/v0.9.0
terraform/modules/azure/ → tag git: tf-azure/v0.8.0
```

Le `versions.tf` de chaque module pin :

```hcl
terraform {
  required_version = ">= 1.6"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.40"
    }
  }
}
```

### 5.7 Tests Terraform

- `terraform validate` : syntaxe + types
- `terraform plan` avec mock state : équivalent unit test
- `terratest` : tests d'intégration en Go, déploient un VPC réel + valident outputs (~ 8 tests par module, ~ 200 LOC chacun)
- `tflint` + `tfsec` + `checkov` : lint + sécurité statique
- CI : workflow `.github/workflows/terraform.yml` exécute validate + plan + tflint + tfsec sur PR ; terratest sur merge main (avec credentials AWS/GCP/Azure du compte EFE-ci)

---

## §6. Runbook ops — 12 incidents-types

### 6.1 Liste des runbooks

| ID | Titre | Sévérité typique | Owner |
|----|-------|------------------|-------|
| RB-01 | Disque DB Postgres saturé (> 85 %) | P1 | DBA on-call |
| RB-02 | Circuit breaker provider LLM ouvert > 5 min | P2 | LLM ops |
| RB-03 | Audit chain HMAC broken (verify_chain.rs failure) | P0 | Compliance + Sec |
| RB-04 | Rotation forcée clé BYOK (KMS compromise) | P0 | Sec + Ops |
| RB-05 | Tenant en `BudgetExhausted` non récupérable | P3 | LLM ops |
| RB-06 | Pod efe-server CrashLoopBackOff | P1 | K8s ops |
| RB-07 | Latence p99 classify > 2× SLO 30 min | P2 | Perf ops |
| RB-08 | Migration DB échouée (sqlx migrate run failure) | P1 | DBA + dev |
| RB-09 | mode3_review_queue saturée (> 1000 pending) | P2 | Compliance |
| RB-10 | Certificat TLS expirant < 30 j (cert-manager fail) | P2 | K8s ops |
| RB-11 | Dépassement quota tokens LLM tenant à 95 % | P3 | Account exec |
| RB-12 | Failover Postgres déclenché | P1 | DBA |

### 6.2 Format runbook standard

Chaque runbook suit le template :

```markdown
# RB-XX : {Titre}

## Métadonnées
- **Sévérité** : P0 | P1 | P2 | P3
- **MTTR cible** : XX min
- **Frequency** : XX /mois (estimation)
- **Last reviewed** : YYYY-MM-DD
- **Owner primaire** : {role}
- **Owner backup** : {role}

## Symptômes (observabilité)
- Alerte Prometheus : `XXX`
- Métrique Grafana : `XXX`
- Log pattern : `XXX`
- Page on-call : `oui/non`

## Diagnostic (3-step)
1. Vérifier {X} via {commande}
2. Vérifier {Y} via {commande}
3. Vérifier {Z} via {commande}

## Procédure de résolution
### Étape 1 : Stabiliser (≤ 5 min)
- {actions immédiates}

### Étape 2 : Diagnostiquer (≤ 15 min)
- {actions diagnostic}

### Étape 3 : Corriger (durée variable)
- {actions correction}

### Étape 4 : Vérifier
- {tests post-fix}

### Étape 5 : Communiquer
- Statuspage update
- Slack #efe-incidents
- Email tenant si P0/P1
- Si P0 : page CTO

## Post-incident
- Postmortem requis si P0/P1 → template `templates/postmortem.md`
- RCA dans 48h
- Action items trackés Jira

## Procédure de validation (test)
- Comment reproduire en staging :
  ```bash
  ...
  ```
- Comment vérifier que le runbook fonctionne :
  ```bash
  ...
  ```
- Frequency test : trimestriel

## Annexes
- Liens dashboards
- Liens migrations / commits pertinents
- Historique incidents passés
```

### 6.3 Exemple détaillé : RB-03 (Audit chain HMAC broken)

```markdown
# RB-03 : Audit chain HMAC broken

## Métadonnées
- **Sévérité** : P0 (compliance breach potentiel)
- **MTTR cible** : 2 h
- **Frequency** : < 1 /an attendu
- **Last reviewed** : 2026-05-08
- **Owner primaire** : Compliance Engineer
- **Owner backup** : Security Lead

## Symptômes
- Alerte Prometheus : `efe_audit_chain_verify_failure_total > 0`
- Métrique Grafana : panel "Audit chain integrity" → red
- Log pattern : `audit chain verify failed: prev_hmac mismatch at audit_id=N`
- Page on-call : **OUI**, P0, page CTO + DPO

## Diagnostic
1. Identifier audit_id et tenant impactés :
   ```bash
   kubectl exec -it deploy/efe-server -- /app/bin/efe-verify-audit-chain --json
   ```
   Output JSON contiendra : `{"first_broken_id": 12345, "tenant": "acme"}`

2. Lire les 10 lignes audit autour du break :
   ```bash
   psql $EFE_DATABASE_URL -c "
     SELECT id, ts, tenant, mode, prev_hmac, hmac
       FROM llm_call_audit
      WHERE id BETWEEN 12340 AND 12350
      ORDER BY id;
   "
   ```

3. Vérifier intégrité du WAL Postgres :
   ```bash
   kubectl exec -it postgres-primary -- pg_waldump --start=$(get_lsn_for_audit_id 12345)
   ```

## Procédure de résolution
### Étape 1 : Stabiliser (≤ 5 min)
1. **STOP les écritures classify pour le tenant impacté** :
   ```bash
   efe-admin tenant suspend acme --reason "AUDIT_CHAIN_BREAK_INVESTIGATION" --yes
   ```
   ⚠️ Ne PAS suspendre tous les tenants : isolation tenant-par-tenant.

2. Snapshot DB immédiat (pour forensics) :
   ```bash
   kubectl exec -it postgres-primary -- pg_dump -t llm_call_audit -t mode3_review_queue \
     -F custom -f /tmp/audit_snapshot_$(date +%s).pgcustom
   kubectl cp postgres-primary:/tmp/audit_snapshot_*.pgcustom ./forensics/
   ```

### Étape 2 : Diagnostiquer (≤ 30 min)
4. Cas A : **break naturel** (trigger fonction défaillante) — vérifier audit_chain_trigger() :
   ```sql
   SELECT prosrc FROM pg_proc WHERE proname = 'audit_chain_trigger';
   ```
5. Cas B : **manipulation manuelle** (UPDATE/DELETE on llm_call_audit hors trigger) — vérifier pg_audit logs :
   ```bash
   grep "llm_call_audit" /var/log/postgresql/*.log | grep -E "UPDATE|DELETE"
   ```
6. Cas C : **corruption disque** — vérifier pg_checksums :
   ```bash
   pg_checksums -D /var/lib/postgresql/data --check
   ```

### Étape 3 : Corriger
- **Cas A** : redéployer migration 0010 ou hotfix trigger. Re-compute hmac à partir de l'audit_id le plus ancien correct.
- **Cas B** : INCIDENT SECURITE → escalader Security Lead, lancer investigation forensics, audit accès Postgres (qui a fait UPDATE/DELETE hors app ?).
- **Cas C** : restore Postgres depuis backup pgBackRest le plus proche du break, replay WAL jusqu'à juste avant le break.

### Étape 4 : Vérifier
- Re-run `efe-verify-audit-chain --tenant=acme` → doit retourner OK
- Vérifier 100 % audit_id continuum : `SELECT COUNT(*), MIN(id), MAX(id) FROM llm_call_audit WHERE tenant='acme'`

### Étape 5 : Communiquer
- Statuspage : « Service intermittently degraded for tenant acme — under investigation »
- Slack #efe-incidents : avec audit_id range impacté
- Email DPO tenant acme : **obligatoire** (GDPR Art. 33 — breach notification 72h)
- Si Cas B : page CTO + Security Lead + Legal

## Post-incident
- Postmortem **obligatoire** sous 72 h (GDPR Art. 33 timeline)
- RCA template : `templates/postmortem_p0_audit.md`
- Action items : ajouter test régression, durcir trigger, audit IAM Postgres

## Procédure de validation
**Reproduce en staging** :
```sql
-- Inject corruption manuelle (NE JAMAIS en prod)
UPDATE llm_call_audit SET hmac = 'corrupted' WHERE id = (SELECT MAX(id) - 5 FROM llm_call_audit);
```
Doit déclencher l'alerte Prometheus dans ≤ 60 s.

**Test trimestriel** : Game Day, owner Compliance, premier mardi de chaque trimestre.
```

Les 11 autres runbooks suivent le même niveau de détail (~ 350-450 lignes chacun, total ~ 4 800 lignes).

### 6.4 Tests reproductibles

Chaque runbook a une **procédure de validation** automatisable :

```bash
# Run all runbook validations against staging
./scripts/runbook-validate.sh --all --env staging
# Run specific runbook
./scripts/runbook-validate.sh --rb 03 --env staging
```

CI : trimestriel (cron `0 9 1 1,4,7,10 *`), execute validation contre staging EFE-ci. Échec → ticket Jira auto-ouvert.

---

## §7. Portail admin tenant — Next.js 14

### 7.1 Stack technique

- **Framework** : Next.js 14.x App Router (RSC + server actions)
- **Language** : TypeScript 5.4 strict
- **UI** : Tailwind CSS 3.4 + shadcn/ui (Radix primitives)
- **API** : tRPC 11 (server actions + client)
- **Auth** : NextAuth.js v5 (Auth.js) + OIDC provider
- **State** : TanStack Query (server state) + Zustand (UI state minimal)
- **Forms** : react-hook-form + zod
- **Tests** : Vitest (unit) + Playwright (E2E)
- **Bundler** : Turbopack (dev) / Webpack 5 (prod)
- **Lint** : Biome + Prettier

### 7.2 Pages

```
apps/portal-admin/
├── app/
│   ├── (public)/
│   │   ├── login/
│   │   └── error/
│   ├── (authenticated)/
│   │   ├── layout.tsx                    # nav + header + tenant switcher
│   │   ├── dashboard/
│   │   │   └── page.tsx                  # vue d'ensemble (RPS, latency, errors)
│   │   ├── quotas/
│   │   │   └── page.tsx                  # vue + édition quotas
│   │   ├── audit/
│   │   │   ├── page.tsx                  # search + paginate audit rows
│   │   │   └── [auditId]/page.tsx        # détail audit row + concept_stack viewer (Mode-3)
│   │   ├── keys/
│   │   │   └── page.tsx                  # list + create + revoke API keys
│   │   ├── compliance/
│   │   │   └── page.tsx                  # AI Act + GDPR attestations + status
│   │   ├── reviews/
│   │   │   ├── page.tsx                  # mode3_review_queue list
│   │   │   └── [token]/page.tsx          # single review approve/reject UI
│   │   └── settings/
│   │       └── page.tsx                  # tenant settings (data residency, BYOK key)
│   └── api/
│       ├── auth/[...nextauth]/route.ts
│       └── trpc/[trpc]/route.ts
├── lib/
│   ├── auth.ts                           # NextAuth config
│   ├── trpc/                             # tRPC routers
│   │   ├── server.ts
│   │   ├── client.ts
│   │   └── routers/
│   │       ├── tenant.ts
│   │       ├── audit.ts
│   │       ├── reviews.ts
│   │       └── compliance.ts
│   ├── grpc/                             # gRPC-Web client → efe-server
│   │   └── client.ts
│   └── mcp/                              # HTTP client → mcp-server
│       └── client.ts
├── components/
│   ├── nav/
│   ├── tenant-switcher/
│   ├── audit/
│   │   ├── concept-stack-viewer.tsx     # tree-view JSONB Mode-3
│   │   └── audit-table.tsx
│   └── ui/                              # shadcn/ui
├── tests/
│   ├── unit/
│   └── e2e/
│       ├── login.spec.ts
│       ├── audit-search.spec.ts
│       ├── review-approve.spec.ts
│       └── full-flow.spec.ts
└── package.json
```

### 7.3 Pages — détails

#### 7.3.1 `/dashboard`

KPIs principaux :
- RPS classify (live, refresh 30s)
- Latency p50/p99 (last 1h)
- Error rate (last 1h)
- Mode-3 convergence rate (last 24h)
- Pending reviews count
- Cost LLM today vs daily quota

Composants :
- 6 cards top
- 2 charts (RPS timeline + latency p99 timeline) — Recharts
- Status banner (vert / orange / rouge selon SLO)

#### 7.3.2 `/audit`

- Filter : tenant, date range, mode (1/2/3), status (success/error), confidence range
- Pagination : cursor-based (audit_id), 50 par page
- Export : button "Export CSV" — déclenche `audit export` côté server (asynchrone, lien email)
- Click sur ligne → détail page

#### 7.3.3 `/audit/[auditId]`

- Métadonnées audit row (tenant, ts, mode, sentence_hash, confidence, etc.)
- Si Mode-3 et `concept_stack_jsonb != NULL` :
  - Tree viewer du concept_stack
  - Pour chaque turn : prompt + LLM reasoning (truncated 1024 chars) + chosen anchors
  - Convergence delta entre turns
- HMAC chain verification status (✅ ou ❌)
- Bouton "Right-to-explanation export" (GDPR Art. 22) → PDF généré server-side

#### 7.3.4 `/reviews`

- Liste mode3_review_queue (status=pending par défaut)
- Filter : tenant, catalog, expires_at < threshold, proposed_label
- Sort : requested_at desc / expires_at asc
- Click → détail page

#### 7.3.5 `/reviews/[token]`

- Affichage complet du review : sentence, proposed_label, proposed_confidence, concept_stack
- 2 actions :
  - **Approve** : sélectionne final_label (= proposed_label par défaut, éditable parmi le top-5 Mode-3) + notes optionnelles + click "Approve" (modal confirmation 1-step)
  - **Reject** : reason obligatoire (≥ 10 chars) + final_label optionnel + click "Reject" (modal confirmation 1-step)
- Action déclenche tool mcp-server `m22_approve_mode3_review` ou `m22_reject_mode3_review` avec `confirm: true`

#### 7.3.6 `/compliance`

- État attestations :
  - AI Act Art. 14 : ✅ acknowledged 2026-04-12 by jane.doe@acme.com
  - GDPR Art. 22 : ✅ DPO contact configured
  - SOC 2 readiness : 80 %
- Documents générés à la demande :
  - "AI Act Art. 14 acknowledgment receipt" (PDF signé)
  - "GDPR Art. 30 register entry" (PDF)
  - "Data Processing Agreement (DPA)" (PDF template)
- Renouvellement : alerte si attestation > 12 mois

#### 7.3.7 `/keys`

- Liste API keys actives + révoquées (toggle)
- Create new key :
  - Scope (read_only / classify / admin)
  - Expiration (24h / 30d / 1y / never)
  - Display **once** post-creation (clipboard copy + warning "save now")
- Rotate key : génère new + invalide old en grace period 24h
- Revoke key : confirm modal + reason obligatoire

#### 7.3.8 `/settings`

- Data residency (lecture seule post-création)
- BYOK backend (lecture seule + lien vers KMS console cloud)
- DPO contact (éditable)
- Notification preferences (email/Slack)
- Tenant suspend (P0 self-service kill switch)

### 7.4 Auth flow

```
[User] → [Portail] → [/login] → [NextAuth.js OIDC redirect]
                                       ↓
                          [IdP : Keycloak/Okta/Azure AD/Auth0]
                                       ↓
                          [Auth code → exchange access_token]
                                       ↓
                          [Callback /api/auth/callback]
                                       ↓
                          [Session JWT + secure HTTP-only cookie]
                                       ↓
                          [Redirect /dashboard?tenant=acme]
```

Multi-tenant : un user peut être membre de N tenants (claim `tenants` du JWT IdP). Le tenant switcher en header permet de basculer.

### 7.5 Sécurité

- **CSP** strict :
  ```
  Content-Security-Policy: default-src 'self'; script-src 'self' 'nonce-XXX'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' https://idp.example.com; frame-ancestors 'none';
  ```
- **CSRF** : tokens via Next.js middleware + tRPC double-submit
- **HSTS** : `max-age=63072000; includeSubDomains; preload`
- **X-Frame-Options** : DENY
- **Cookie attrs** : `Secure; HttpOnly; SameSite=Strict`
- **SRI** sur scripts externes (aucun par défaut)
- **Audit log** : toute action write loggée dans `ui_audit_log` table (qui a fait quoi quand depuis quel IP)

### 7.6 Tests Playwright

`tests/e2e/full-flow.spec.ts` :

```typescript
import { test, expect } from "@playwright/test";

test.describe("Full onboarding E2E", () => {
  test("SE creates tenant + DPO uses portal", async ({ page, browser }) => {
    // Step 1: SE creates tenant via efe-admin (assumed done in fixture)
    // Step 2: DPO logs in via SSO
    await page.goto("/login");
    await page.click("text=Sign in with SSO");
    // Mock IdP redirect (test mode)
    await page.fill('input[name="email"]', "dpo@acme.com");
    await page.fill('input[name="password"]', process.env.TEST_DPO_PASSWORD!);
    await page.click("text=Sign in");
    await expect(page).toHaveURL(/\/dashboard/);

    // Step 3: DPO checks dashboard
    await expect(page.locator('[data-testid="kpi-rps"]')).toBeVisible();

    // Step 4: DPO acknowledges AI Act Art. 14
    await page.goto("/compliance");
    await page.click("text=Acknowledge AI Act Art. 14");
    await page.fill('input[name="signedBy"]', "dpo@acme.com");
    await page.click("text=Confirm");
    await expect(page.locator('text=AI Act Art. 14: ✅')).toBeVisible();

    // Step 5: DPO creates API key
    await page.goto("/keys");
    await page.click("text=Create new key");
    await page.selectOption('select[name="scope"]', "classify");
    await page.selectOption('select[name="expires"]', "30d");
    await page.click("text=Create");
    const keyValue = await page.locator('[data-testid="key-value"]').textContent();
    expect(keyValue).toMatch(/^efe_[a-f0-9]{64}$/);

    // Step 6: Run smoke classify with that key (via API call from test)
    const response = await fetch(process.env.EFE_GRPC_GATEWAY!, {
      method: "POST",
      headers: { Authorization: `Bearer ${keyValue}` },
      body: JSON.stringify({ tenant: "acme", catalog: "default", sentence: "test" }),
    });
    expect(response.status).toBe(200);

    // Step 7: DPO sees the audit row
    await page.goto("/audit");
    await expect(page.locator('table tbody tr').first()).toContainText("test");
  });
});
```

145 specs E2E ciblent : login, navigation, every page CRUD, error states, RBAC enforcement.

---

## §8. Flux d'onboarding « 30 min POC » — déroulé chirurgical

### 8.1 Pré-requis SE laptop

- macOS / Linux / WSL2
- Outils installés (idéalement via brew bundle ou nix-profile) :
  - `terraform` ≥ 1.6
  - `kubectl` ≥ 1.30
  - `helm` ≥ 3.14
  - `efe-admin` (latest)
  - `aws` / `gcloud` / `az` (selon cloud)
  - `jq`
- Credentials cloud configurés (AWS profile, GCP gcloud auth, Azure az login)
- Accès au registry images (`docker login ghcr.io` avec personal token EFE)

### 8.2 Déroulé minute par minute

#### T+00:00 — T+10:00 : Provision infra

```bash
# Cloner le starter repo
git clone https://github.com/efe/customer-starter
cd customer-starter

# Configurer les variables tenant
cat > terraform.tfvars <<EOF
tenant         = "acme"
region         = "eu-west-1"
data_residency = "EU_ONLY"
EOF

cd terraform/aws
terraform init
terraform apply -auto-approve            # ~ 8 min
```

À l'issue : VPC, EKS cluster (3 nodes), RDS Postgres (multi-AZ), ElastiCache Redis (multi-AZ), KMS BYOK, S3 audit bucket. Outputs Terraform : kubeconfig, db_secret_arn, byok_kms_key_arn.

#### T+10:00 — T+15:00 : Configure kubectl + Helm install

```bash
# Configure kubectl
aws eks update-kubeconfig --region eu-west-1 --name efe-acme-eks

# Generate values.yaml from Terraform outputs
./scripts/generate-helm-values.sh > values-acme.yaml

# Install
helm install efe-stack oci://ghcr.io/efe/charts/efe-stack \
  --version 1.0.0 \
  --values values-acme.yaml \
  --namespace efe \
  --create-namespace \
  --wait --timeout 5m
```

Helm fait :
- Crée namespace `efe` avec PSS restricted
- Déploie Postgres operator + cluster (Postgres takes ~ 90 s à être ready)
- Applique migrations 0001-0013 via Job init-container
- Déploie Redis Stack
- Déploie efe-server (3 replicas), mcp-server (2), portal-admin (3)
- Déploie observability stack
- Configure ingress + cert-manager (ACME dns-01 via Route53 → certificate ready ~ 1-2 min)

À l'issue : `kubectl get pods -n efe` montre tout en `Running`, `kubectl get certs` montre le cert `Ready=True`.

#### T+15:00 — T+18:00 : Create tenant + compliance

```bash
# Authenticate efe-admin
efe-admin login --cluster acme \
  --idp-url https://idp.example.com/realms/efe \
  --client-id efe-admin

# Create tenant
efe-admin tenant create acme \
  --plan enterprise \
  --max-rps 1000 \
  --max-tokens-per-day 1000000 \
  --data-residency EU \
  --yes

# Attest compliance
efe-admin compliance attest acme \
  --ai-act-art14 \
  --gdpr-art22 \
  --signed-by 'jane.doe@acme.com' \
  --yes
```

À l'issue : row `tenants(name=acme)` créée, row `tenant_feature_mode3(tenant_id=acme, ai_act_art14_acknowledged_at=now())` créée, audit log entries posés.

#### T+18:00 — T+25:00 : Seed catalog

```bash
# Le SE a un fichier acme_anchors.jsonl prepared en amont (50-500 anchors)
# Format JSONL (une ligne par anchor) :
# {"anchor_id": "a01", "text": "facturation tardive", "label": "billing", ...}

efe-admin catalog seed acme \
  --bundle ./acme_anchors.jsonl \
  --bundle-version v1.0.0 \
  --catalog default \
  --yes
```

Sous le capot, le CLI :
- Valide le JSONL (schema check)
- Computes embeddings via efe-server (gRPC `efe.v1.bundles.SeedBundle`)
- Insère dans `catalog_anchors` + indexe HNSW Redis
- Persiste audit row `bundle_seeded`
- Output : « 487 anchors seeded in 4m 12s ».

#### T+25:00 — T+28:00 : Smoke test

```bash
efe-admin smoke-test acme
```

Exécute :
- Health check pods (`kubectl get pods` filter)
- Classify 10 phrases du smoke set (depuis `tests/fixtures/smoke_sentences.txt`) en Mode-1 + Mode-2
- Vérifie audit chain HMAC contiguë
- Vérifie observability scrape (Prometheus `up{job=~"efe.*"} == 1`)
- Output : « ✅ 10/10 smoke tests passed, p99 latency: 87 ms ».

#### T+28:00 — T+30:00 : Handoff DPO

```bash
echo "Tenant POC ready: https://portal.efe.example.com/tenants/acme"
echo "DPO email: jane.doe@acme.com"
echo "Initial credentials: see Secrets Manager arn:aws:secretsmanager:..."
echo "Smoke test report: ./reports/smoke-acme-$(date +%s).json"
```

SE envoie l'email au DPO Acme avec le lien portail. DPO se connecte via SSO (déjà fédéré IdP Acme), arrive sur `/dashboard`, voit les 10 audit rows du smoke test.

### 8.3 Validations à chaque step

Le CLI `efe-admin` a une commande `validate` post-step qui vérifie l'invariant :

| Step | Command de validation | Échec attendu |
|------|----------------------|---------------|
| 01 | `terraform output kubeconfig_command` | Empty output |
| 02 | `kubectl get pods -n efe \| grep -v Running` | Output non-empty |
| 03 | `efe-admin tenant get acme` | Exit 1 |
| 04 | `efe-admin compliance verify acme` | « not_attested » |
| 05 | `efe-admin catalog list acme \| grep default` | Empty |
| 06 | Smoke classify call | HTTP 503 |

### 8.4 Rollback safe

Chaque étape a un rollback explicite documenté :

```bash
# Rollback step 03 (tenant create)
efe-admin tenant delete acme --purge --yes

# Rollback step 02 (helm install)
helm uninstall efe-stack -n efe
kubectl delete namespace efe --wait

# Rollback step 01 (terraform apply)
cd terraform/aws
terraform destroy -auto-approve
```

Le starter repo inclut un script `./scripts/full-rollback.sh` qui exécute les 3 rollbacks en séquence inverse, sécurisé par double confirmation.

---

## §9. Migrations DB & schéma

### 9.1 Migration 0013

```sql
-- 0013_d8_admin_self_service.sql — GLOSS
--
-- Ajoute les tables nécessaires à l'admin self-service :
--   - api_keys           : clés API par tenant, scopes, expiration
--   - sso_sessions       : sessions IdP côté portail
--   - ui_audit_log       : audit log UI distinct du llm_call_audit
--   - tenant_compliance  : attestations versionnées (audit trail)
--
-- Toutes tables sont nouvelles, AUCUNE modification des tables M16-M22.

-- ─── api_keys ────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS api_keys (
    key_id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    -- Hashed key (Argon2id, never raw plaintext)
    key_hash            TEXT NOT NULL,
    -- Last 4 chars of plaintext for UI display (« ...XYZW »)
    key_suffix          TEXT NOT NULL,
    scope               TEXT NOT NULL CHECK (scope IN ('read_only', 'classify', 'admin')),
    created_by          TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at          TIMESTAMPTZ,
    revoked_at          TIMESTAMPTZ,
    revoked_by          TEXT,
    revoked_reason      TEXT,
    last_used_at        TIMESTAMPTZ,
    use_count           BIGINT NOT NULL DEFAULT 0,
    CHECK (
        (revoked_at IS NULL AND revoked_by IS NULL AND revoked_reason IS NULL)
        OR (revoked_at IS NOT NULL AND revoked_by IS NOT NULL AND revoked_reason IS NOT NULL)
    )
);

CREATE INDEX ix_api_keys_tenant_active
    ON api_keys(tenant_id)
    WHERE revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now());

CREATE INDEX ix_api_keys_hash_lookup ON api_keys(key_hash);

-- ─── sso_sessions ─────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS sso_sessions (
    session_id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL,
    user_email          TEXT NOT NULL,
    user_subject        TEXT NOT NULL,           -- IdP `sub` claim
    idp_issuer          TEXT NOT NULL,
    refresh_token_hash  TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at          TIMESTAMPTZ NOT NULL,
    revoked_at          TIMESTAMPTZ,
    user_agent          TEXT,
    ip_address          INET
);

CREATE INDEX ix_sso_sessions_tenant_user
    ON sso_sessions(tenant_id, user_email)
    WHERE revoked_at IS NULL;

-- ─── ui_audit_log ─────────────────────────────────────────────────────
-- Audit log pour les actions UI ; complémentaire à llm_call_audit.
CREATE TABLE IF NOT EXISTS ui_audit_log (
    id              BIGSERIAL PRIMARY KEY,
    ts              TIMESTAMPTZ NOT NULL DEFAULT now(),
    tenant_id       TEXT NOT NULL,
    user_email      TEXT NOT NULL,
    action          TEXT NOT NULL,
    target_type     TEXT,                        -- 'api_key', 'review', 'quota', etc.
    target_id       TEXT,
    payload_jsonb   JSONB,
    ip_address      INET,
    user_agent      TEXT,
    -- Chain HMAC like llm_call_audit (D14-bis)
    prev_hmac       BYTEA,
    hmac            BYTEA NOT NULL
);

CREATE INDEX ix_ui_audit_log_tenant_ts ON ui_audit_log(tenant_id, ts DESC);
CREATE INDEX ix_ui_audit_log_user      ON ui_audit_log(user_email, ts DESC);

-- ─── tenant_compliance ────────────────────────────────────────────────
-- Versionned attestations (multiple over time). The current attestation
-- is the most recent row per tenant_id where revoked_at IS NULL.
CREATE TABLE IF NOT EXISTS tenant_compliance (
    id                              BIGSERIAL PRIMARY KEY,
    tenant_id                       TEXT NOT NULL,
    ai_act_art14_acknowledged       BOOLEAN NOT NULL DEFAULT FALSE,
    ai_act_art14_acknowledged_at    TIMESTAMPTZ,
    ai_act_art14_signed_by          TEXT,
    gdpr_art22_acknowledged         BOOLEAN NOT NULL DEFAULT FALSE,
    gdpr_art22_acknowledged_at      TIMESTAMPTZ,
    gdpr_art22_dpo_contact          TEXT,
    soc2_in_scope                   BOOLEAN NOT NULL DEFAULT FALSE,
    revoked_at                      TIMESTAMPTZ,
    revoked_reason                  TEXT,
    created_at                      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (
        NOT ai_act_art14_acknowledged
        OR (ai_act_art14_acknowledged_at IS NOT NULL AND ai_act_art14_signed_by IS NOT NULL)
    ),
    CHECK (
        NOT gdpr_art22_acknowledged
        OR (gdpr_art22_acknowledged_at IS NOT NULL AND gdpr_art22_dpo_contact IS NOT NULL)
    )
);

CREATE INDEX ix_tenant_compliance_current
    ON tenant_compliance(tenant_id, created_at DESC)
    WHERE revoked_at IS NULL;

-- ─── Sync trigger : tenant_compliance → tenant_feature_mode3 ──────────
-- Quand une attestation AI Act Art. 14 est créée, propage dans tenant_feature_mode3.
CREATE OR REPLACE FUNCTION sync_tenant_compliance_to_mode3()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.ai_act_art14_acknowledged = TRUE THEN
        INSERT INTO tenant_feature_mode3 (tenant_id, ai_act_art14_acknowledged_at)
        VALUES (NEW.tenant_id, NEW.ai_act_art14_acknowledged_at)
        ON CONFLICT (tenant_id)
        DO UPDATE SET ai_act_art14_acknowledged_at = NEW.ai_act_art14_acknowledged_at,
                      updated_at = now();
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS sync_compliance_mode3 ON tenant_compliance;
CREATE TRIGGER sync_compliance_mode3
    AFTER INSERT OR UPDATE ON tenant_compliance
    FOR EACH ROW EXECUTE FUNCTION sync_tenant_compliance_to_mode3();

COMMENT ON TABLE api_keys IS
    'GLOSS — API keys per tenant with scopes + expiration + revocation.';
COMMENT ON TABLE sso_sessions IS
    'GLOSS — Browser sessions for portal-admin (NextAuth.js backed).';
COMMENT ON TABLE ui_audit_log IS
    'GLOSS — UI action audit log (HMAC chain like llm_call_audit).';
COMMENT ON TABLE tenant_compliance IS
    'GLOSS — Versioned attestations AI Act Art. 14 + GDPR Art. 22.';
```

### 9.2 Backward compat invariants

- ❌ Aucune table M16-M22 modifiée
- ❌ Aucune colonne ajoutée/supprimée à une table existante
- ✅ 4 nouvelles tables, isolées
- ✅ 1 trigger qui propage **vers** `tenant_feature_mode3` (jamais lecture qui pourrait casser)
- ✅ Si on rollback 0013, les 4 tables sont droppées sans impact M16-M22

Test invariant `tests/sql/test_0013_idempotent.sql` :

```sql
-- Apply migration twice ; second call must succeed (CREATE TABLE IF NOT EXISTS)
\i 0013_d8_admin_self_service.sql
\i 0013_d8_admin_self_service.sql
SELECT COUNT(*) AS expected_zero_dup FROM (
    SELECT table_name, COUNT(*) c FROM information_schema.tables
     WHERE table_name IN ('api_keys', 'sso_sessions', 'ui_audit_log', 'tenant_compliance')
     GROUP BY table_name HAVING COUNT(*) > 1
) t;
-- expected: 0
```

---

## §10. mcp-server tools (5 nouveaux)

### 10.1 Liste des nouveaux outils

| Tool ID | Profile | Type | Confirm |
|---------|---------|------|---------|
| `m23_create_tenant` | admin_full | write | ✅ |
| `m23_seed_catalog` | admin_full | write | ✅ |
| `m23_attest_compliance` | admin_full | write | ✅ |
| `m23_rotate_api_key` | admin_full | write | ✅ |
| `m23_get_tenant_health` | read_only | read | — |

Cumul après GLOSS : **49 + 5 = 54 outils** distribués sur 5 profiles.

### 10.2 Spec `m23_create_tenant`

```python
# mcp-server/mcp_server/tools/m23_create_tenant.py
from typing import Annotated, Literal
from pydantic import BaseModel, Field, EmailStr
from mcp_server.framework import tool, write, profile

class CreateTenantInput(BaseModel):
    name: Annotated[str, Field(pattern=r"^[a-z0-9-]{3,32}$")]
    plan: Literal["trial", "growth", "enterprise"] = "trial"
    max_rps: Annotated[int, Field(ge=1, le=100_000)] = 100
    max_tokens_per_day: Annotated[int, Field(ge=0, le=1_000_000_000)] = 100_000
    data_residency: Literal["EU_ONLY", "US_ONLY", "any"] = "any"
    contact_email: EmailStr | None = None
    confirm: Literal[True]                     # mandatory, write tool

class CreateTenantOutput(BaseModel):
    tenant_id: str
    created_at: str
    initial_admin_token: str                   # one-time, displayed once

@tool(profile="admin_full", confirm_required=True)
@write
@profile("admin_full")
async def m23_create_tenant(
    inp: CreateTenantInput,
    ctx: ToolContext,
) -> CreateTenantOutput:
    """
    Create a new tenant with the given plan + quotas.

    Idempotency : if tenant already exists, returns 409 Conflict.
    Audit : writes to ui_audit_log.
    """
    async with ctx.db.transaction():
        # Check idempotency
        existing = await ctx.db.fetchrow(
            "SELECT name FROM tenants WHERE name = $1", inp.name)
        if existing:
            raise ToolError("conflict", f"tenant '{inp.name}' already exists")

        # Insert
        await ctx.db.execute(
            """INSERT INTO tenants (name, plan, max_rps, max_tokens_per_day,
                                     data_residency, contact_email)
                   VALUES ($1, $2, $3, $4, $5, $6)""",
            inp.name, inp.plan, inp.max_rps, inp.max_tokens_per_day,
            inp.data_residency, inp.contact_email,
        )

        # Create initial admin API key
        token, key_id = await ctx.create_api_key(
            tenant_id=inp.name,
            scope="admin",
            created_by=ctx.caller_email,
            expires_in_days=7,                  # short-lived bootstrap
        )

        # Audit log
        await ctx.audit_ui(
            action="tenant.create",
            target_type="tenant",
            target_id=inp.name,
            payload={"plan": inp.plan, "max_rps": inp.max_rps},
        )

    return CreateTenantOutput(
        tenant_id=inp.name,
        created_at=datetime.utcnow().isoformat(),
        initial_admin_token=token,
    )
```

### 10.3 Spec `m23_seed_catalog`

Encapsule la logique de seeding actuellement répartie entre plusieurs scripts :
1. Lecture JSONL streaming (memory-efficient pour > 10k anchors)
2. Validation schema chaque ligne
3. Computes embeddings via efe-server gRPC `EmbedBatch`
4. INSERT dans `catalog_anchors`
5. Indexe HNSW Redis (commande FT.CREATE / HSET)
6. Audit log

Idempotency : si `(tenant_id, catalog, bundle_version)` existe déjà → 409 Conflict.

### 10.4 Spec `m23_attest_compliance`

```python
class AttestComplianceInput(BaseModel):
    tenant_id: str
    ai_act_art14: bool = False
    gdpr_art22: bool = False
    signed_by: EmailStr
    dpo_contact: EmailStr | None = None
    confirm: Literal[True]

@tool(profile="admin_full", confirm_required=True)
@write
async def m23_attest_compliance(inp, ctx):
    async with ctx.db.transaction():
        # Vérifier qu'au moins une attestation est posée
        if not (inp.ai_act_art14 or inp.gdpr_art22):
            raise ToolError("invalid_input", "at least one attestation required")

        # Insert nouvelle ligne tenant_compliance (versioning)
        # → trigger sync_tenant_compliance_to_mode3 propage automatiquement
        await ctx.db.execute(
            """INSERT INTO tenant_compliance (
                  tenant_id,
                  ai_act_art14_acknowledged, ai_act_art14_acknowledged_at, ai_act_art14_signed_by,
                  gdpr_art22_acknowledged, gdpr_art22_acknowledged_at, gdpr_art22_dpo_contact
               ) VALUES ($1, $2, now(), $3, $4, now(), $5)""",
            inp.tenant_id,
            inp.ai_act_art14, inp.signed_by if inp.ai_act_art14 else None,
            inp.gdpr_art22, inp.dpo_contact if inp.gdpr_art22 else None,
        )

        await ctx.audit_ui(
            action="compliance.attest",
            target_type="tenant",
            target_id=inp.tenant_id,
            payload={
                "ai_act_art14": inp.ai_act_art14,
                "gdpr_art22": inp.gdpr_art22,
                "signed_by": inp.signed_by,
            },
        )
    return {"status": "attested", "ts": datetime.utcnow().isoformat()}
```

### 10.5 Spec `m23_get_tenant_health`

Read-only, retourne :

```python
class TenantHealthOutput(BaseModel):
    tenant_id: str
    status: Literal["healthy", "degraded", "down", "suspended"]
    checks: dict  # 12 sub-checks (db, redis, llm provider, breaker, etc.)
    metrics: dict  # rps, latency p99, error rate, cost today
    compliance: dict  # ai_act_art14_status, gdpr_art22_status
    pending_reviews: int
    last_classify_at: str | None
```

Aggrège : Postgres count queries + Prometheus federation (HTTP Prometheus API) + Redis PING.

### 10.6 Tests

`tests/test_default_off_d8.py` (8 tests) :
- Tools D8 sont OFF par défaut sur les profils non-`admin_full`
- Chaque tool write **rejette** si `confirm` absent ou `False`
- `m23_create_tenant` rejette nom invalide
- `m23_attest_compliance` rejette si aucune attestation posée
- etc.

`tests/test_m23_tenant_lifecycle.py` (15 tests) : cycle complet create → seed → attest → smoke → suspend → delete.

`tests/test_m23_idempotency.py` (10 tests) : tous les outils sont idempotents (re-run = no-op ou 409 propre).

---

## §11. Tests & coverage

### 11.1 Stratégie testing

Pyramide :

```
                  ▲
                 ╱│╲                E2E (Playwright + k3d)        ~ 30 tests
                ╱ │ ╲
               ╱  │  ╲
              ╱   │   ╲
             ╱────┴────╲              Integration (testcontainers) ~ 90 tests
            ╱           ╲
           ╱             ╲
          ╱───────────────╲
         ╱                 ╲           Unit (cargo test, vitest)   ~ 200 tests
        ╱___________________╲
```

### 11.2 Décompte cible GLOSS

| Couche | Type | Tests |
|--------|------|-------|
| efe-admin Rust | unit | 140 |
| efe-admin Rust | integration (testcontainers) | 50 |
| mcp-server D8 tools | unit + integration | 65 |
| Helm chart | helm-unittest | 25 |
| Terraform | terratest | 24 (8 × 3 modules) |
| Portal Next.js | vitest unit | 110 |
| Portal Next.js | Playwright E2E | 145 |
| End-to-end (k3d) | bash | 30 |
| **Total nouveau** | | **589** |

Cumul global EFE post-GLOSS :

| Couche | Avant GLOSS | Après GLOSS | Δ |
|--------|-------------|-------------|---|
| Rust foundation | 2 157 | 2 297 | +140 |
| Rust LLM | 88 | 88 | — |
| Rust mode3 | 41 | 41 | — |
| Rust efe-admin | 0 | 190 | +190 |
| mcp-server Python | 586 | 651 | +65 |
| Realism (release) | 5 | 5 | — |
| Helm/Terraform | 0 | 49 | +49 |
| Portal Next.js | 0 | 255 | +255 |
| **Total** | **~2 877** | **~3 576** | **+699** |

### 11.3 Coverage gates

- Rust efe-admin : coverage ≥ 85 % lines (cargo-tarpaulin)
- mcp-server D8 : coverage ≥ 90 % lines (pytest-cov)
- Portal Next.js : coverage ≥ 80 % lines (vitest --coverage)
- Helm chart : 100 % de templates rendered au moins 1 fois (helm-unittest)
- Terraform : 100 % des resources covered par au moins 1 terratest

CI bloque le merge si gates rouge.

---

## §12. Plan de sprints

### 12.1 Sprint A (semaine 1) — Foundation

**Objectif** : CLI `efe-admin` + migration 0013 + tools mcp-server + tests unitaires.

| Jour | Livrable | Tests |
|------|----------|-------|
| J1 | Crate `efe-admin` skeleton + clap parsing + config loader | 30 unit |
| J2 | Commands `tenant.*` + `health` + `version` + integration testcontainers | 50 unit + 10 int |
| J3 | Migration 0013 + tools `m23_create_tenant` + `m23_get_tenant_health` | 40 unit + 5 int |
| J4 | Commands `compliance.*` + `keys.*` + tools `m23_attest_compliance` + `m23_rotate_api_key` | 30 unit + 10 int |
| J5 | Commands `catalog.*` + `audit.*` + tools `m23_seed_catalog` | 40 unit + 15 int |

**DoD Sprint A** : `efe-admin tenant create` + `compliance attest` + `catalog seed` fonctionnent end-to-end contre une stack testcontainers locale.

### 12.2 Sprint B (semaine 2) — Infra

**Objectif** : Helm chart + Terraform AWS + premiers runbooks.

| Jour | Livrable | Tests |
|------|----------|-------|
| J6 | Helm sub-chart efe-server + mcp-server + values.schema.json | 8 helm-unittest |
| J7 | Helm sub-chart portal-admin + postgres-operator + redis-stack + observability | 12 helm-unittest |
| J8 | Helm umbrella + k3d local install + smoke test E2E | 5 helm-unittest + 5 e2e |
| J9 | Terraform module AWS complet (VPC + EKS + RDS + ElastiCache + KMS + S3) | 8 terratest |
| J10 | Runbooks RB-01 à RB-06 + scripts de validation | (validation seulement) |

**DoD Sprint B** : `helm install efe-stack` réussit sur k3d local en ≤ 5 min ; `terraform apply` réussit sur AWS sandbox en ≤ 12 min ; 6 runbooks publiés et validés.

### 12.3 Sprint C (semaine 3) — UX

**Objectif** : Portail admin + Terraform GCP/Azure + runbooks restants.

| Jour | Livrable | Tests |
|------|----------|-------|
| J11 | Portal Next.js skeleton + auth + nav + pages /dashboard + /settings | 30 vitest |
| J12 | Pages /audit + /audit/[id] + concept_stack viewer | 25 vitest |
| J13 | Pages /reviews + /compliance + /keys + /quotas | 35 vitest |
| J14 | E2E Playwright full flow + Terraform GCP | 50 e2e + 8 terratest |
| J15 | Terraform Azure + runbooks RB-07 à RB-12 + audit pass + bug fixes | 8 terratest + 95 e2e + 6 runbook validations |

**DoD Sprint C** : E2E Playwright "DPO acknowledges + creates key + audits" passe ; Terraform GCP + Azure passent terratest ; 12 runbooks publiés et validés ; audit pass chirurgical fait avec ≤ 6 défauts identifiés/corrigés.

### 12.4 Critères de release

Avant publication GA GLOSS :

- ✅ Tous tests GREEN (≥ 3 576 tests)
- ✅ Coverage gates respectées
- ✅ E2E Playwright vert sur 3 navigateurs (Chromium, Firefox, WebKit)
- ✅ k3d local install ≤ 5 min reproduit chez 3 ingénieurs différents
- ✅ Terraform AWS apply ≤ 12 min reproduit en sandbox
- ✅ Audit pass : ≤ 6 défauts identifiés et tous corrigés
- ✅ 0 régression M16-M22
- ✅ Sigstore signature des images Docker (cosign sign)
- ✅ Documentation `/docs/onboarding/SE_PLAYBOOK.md` revue par 2 SE
- ✅ Ré-exécution complète onboarding 30-min documentée (vidéo asciinema)

---

## §13. Critères d'acceptation détaillés

### 13.1 Performance

- `efe-admin` startup time ≤ 50 ms (en mode `--help`)
- `efe-admin tenant create` end-to-end ≤ 3 s
- `efe-admin catalog seed` (500 anchors) ≤ 5 min
- `helm install efe-stack` (k3d local) ≤ 5 min
- `helm install efe-stack` (cloud cluster) ≤ 8 min
- `terraform apply` (AWS) ≤ 12 min
- Portal `/dashboard` Time-to-Interactive ≤ 1.5 s (P95)
- Portal `/audit` recherche 1M rows ≤ 800 ms (P95)
- mcp-server D8 tools p99 ≤ 200 ms (sauf `seed_catalog` qui est long)

### 13.2 Sécurité

- 100 % images Docker signées Sigstore
- 100 % Helm charts checksums vérifiés (helm verify)
- 100 % Terraform plans passent `tfsec` sans HIGH/CRITICAL
- 100 % portal pages servent CSP strict
- 100 % API tools mcp-server write requièrent `confirm:true`
- 100 % CLI `efe-admin` write requièrent `--yes` ou TTY confirm
- API keys jamais loggués en clair (ni log applicatif ni audit log)
- Rotation BYOK : le SE peut rotate via `efe-admin keys rotate-byok` (réservé en MVP, scaffold présent mais marqué « experimental »)

### 13.3 Observabilité

- 3 dashboards Grafana pré-installés et fonctionnels
- 12 alertes Prometheus pré-configurées (correspondant aux 12 runbooks)
- Logs structurés JSON avec corr_id, request_id, tenant_id
- Traces OpenTelemetry (Tempo) sampling 1 % prod / 100 % dev
- SLO tracking : burn rate alerts (2 × 14 j budget consumption)

### 13.4 Documentation

- `docs/onboarding/SE_PLAYBOOK.md` : guide pas-à-pas pour SE (15 pages)
- `docs/onboarding/DPO_QUICKSTART.md` : guide DPO tenant (8 pages)
- 12 runbooks ops complets
- `crates/efe-admin/README.md` : usage CLI complet
- `charts/efe-stack/README.md` : tous les values + exemples
- `terraform/modules/{aws,gcp,azure}/README.md` : variables + outputs + examples

### 13.5 Compliance preserved

- Test invariant `tests/test_global_backward_compat.py` : 100 % M16-M22 tests passent post-GLOSS
- Test invariant `tests/test_compliance_gate_preserved.py` : trigger `mode3_compliance_gate` toujours fonctionnel
- Test invariant `tests/test_audit_chain_unbroken.py` : HMAC chain reste contiguë post-création tenant via GLOSS

---

## §14. Risques & mitigations

### 14.1 Matrice de risques

| ID | Risque | Probabilité | Impact | Mitigation |
|----|--------|-------------|--------|------------|
| R-01 | helm-unittest faux positifs (templates qui rendent mais déploient mal) | M | M | Doubler avec k3d real-deploy E2E ; helm-unittest reste un complément |
| R-02 | Terraform module dérive (provider AWS/GCP/Azure breaking change) | H | M | Pin version provider strict ; renovate-bot PR mensuelle ; CI test automatique sur upgrade |
| R-03 | NextAuth.js v5 toujours en RC à la livraison | M | H | Fallback Auth.js v4 (stable) si v5 pas GA ; abstraction repo `lib/auth.ts` |
| R-04 | Compliance attestation : attaque social engineering du DPO | L | H | UI rate-limit ; double signature email ; audit log immuable |
| R-05 | Migration 0013 trigger fait du blocage sur tenant_feature_mode3 | L | M | Test stress dans CI ; trigger AFTER au lieu de BEFORE pour réduire latency |
| R-06 | k3d local trop lourd pour laptops (< 16 GB RAM) | M | L | values-local-tiny.yaml avec replicas=1, retention=1d, resources réduites |
| R-07 | Onboarding 30 min dépassé sur cloud lent (Azure ~ 18 min EKS apply) | H | L | Documenter "30 min sur AWS ; jusqu'à 45 min sur Azure/GCP" |
| R-08 | RBAC IdP mal configuré côté tenant → portal access refusé | M | M | Provide `efe-admin idp-validate` command + IdP setup docs |
| R-09 | Portail révèle accidentellement des concept_stacks d'autres tenants | L | C | Tests E2E cross-tenant strict ; tRPC procedures vérifient tenant_id en input AND session.tenants ∋ tenant_id |
| R-10 | Helm chart bug → état Postgres inconsistant entre install/upgrade | M | H | Test upgrade matrix : 1.0.0 → 1.1.0 → 1.0.0 (rollback test) ; init-Job idempotent |
| R-11 | API key leak via logs trop verbeux | L | C | Test linter `tests/test_no_secret_logged.py` : grep `efe_[a-f0-9]{64}` dans logs CI |
| R-12 | Effet domino runbook : RB-01 (DB full) déclenché simulating réel cause downtime | M | M | Tests runbook en staging ; `--dry-run` sur toutes commandes destructives |

Légende : Probabilité L/M/H ; Impact L/M/H/C (Critical = compliance breach).

### 14.2 Plan de mitigation par phase

- **Sprint A** : R-05, R-11 traités via tests
- **Sprint B** : R-01, R-02, R-06, R-07, R-10 traités via CI matrix
- **Sprint C** : R-03, R-04, R-08, R-09, R-12 traités via E2E + audit

---

## §15. Annexes

### 15.1 Sample `~/.config/efe-admin/config.toml`

```toml
[default]
cluster = "https://efe.example.com"
auth_method = "sso"
output_format = "table"
confirm = "interactive"

[clusters.acme]
cluster = "https://efe.acme.com"
auth_method = "sso"
idp_url = "https://idp.acme.com/realms/efe"
client_id = "efe-admin-cli"

[clusters.staging]
cluster = "https://efe-staging.example.com"
auth_method = "token"
token_file = "~/.config/efe-admin/staging.token"

[clusters.local]
cluster = "http://localhost:50051"
auth_method = "none"
```

### 15.2 Sample `values.yaml` pour `helm install` production AWS

```yaml
global:
  domain: efe.acme.com
  imageRegistry: ghcr.io/efe
  storageClass: gp3

efe-server:
  replicas: 6
  image:
    tag: "v1.0.0"
  resources:
    requests: { cpu: 1, memory: 2Gi }
    limits:   { cpu: 4, memory: 8Gi }
  byok:
    backend: aws-kms
    awsKmsKeyId: "arn:aws:kms:eu-west-1:123456789012:key/abcdef..."
  autoscaling:
    enabled: true
    minReplicas: 6
    maxReplicas: 24

mcp-server:
  replicas: 4

portal-admin:
  replicas: 3
  ingress:
    host: portal.efe.acme.com
    tls: { enabled: true, issuer: letsencrypt-prod }
  sso:
    issuer: https://idp.acme.com/realms/efe
    clientId: efe-portal
    clientSecretRef:
      name: efe-portal-secret
      key: oidc-client-secret

postgres:
  enabled: false                            # external RDS via Terraform
externalPostgres:
  secretArn: arn:aws:secretsmanager:eu-west-1:123456789012:secret:efe-acme/rds/credentials
  host: efe-acme-postgres.cluster-xxx.eu-west-1.rds.amazonaws.com
  port: 5432
  database: efe
  sslMode: require

redis:
  enabled: false                            # external ElastiCache
externalRedis:
  host: efe-acme-redis.xxx.cache.amazonaws.com
  port: 6379
  authTokenSecretArn: arn:aws:secretsmanager:eu-west-1:123456789012:secret:efe-acme/redis/auth

observability:
  enabled: true
  prometheus:
    retention: 30d
    persistentStorage: 200Gi
  loki:
    retention: 30d
    persistentStorage: 500Gi
  tempo:
    samplingRate: 0.01
```

### 15.3 Sample Terraform module call (AWS)

```hcl
# customer-starter/terraform/aws/main.tf
terraform {
  required_version = ">= 1.6"
  backend "s3" {
    bucket         = "efe-acme-tfstate"
    key            = "efe-stack/terraform.tfstate"
    region         = "eu-west-1"
    dynamodb_table = "efe-acme-tflock"
    encrypt        = true
  }
}

provider "aws" {
  region = "eu-west-1"
  default_tags {
    tags = {
      "efe.io/tenant" = "acme"
      "efe.io/env"    = "production"
    }
  }
}

module "efe_stack_aws" {
  source  = "git::https://github.com/efe/efe.git//terraform/modules/aws?ref=tf-aws/v1.0.0"

  region          = "eu-west-1"
  tenant          = "acme"
  data_residency  = "EU_ONLY"
  vpc_cidr        = "10.42.0.0/16"
  eks_version     = "1.30"
  rds_instance_class = "db.r6i.xlarge"
  rds_allocated_storage = 500
  rds_multi_az    = true
  enable_kms_byok = true

  eks_node_groups = {
    primary = {
      instance_types = ["m6i.xlarge"]
      desired_size   = 6
      min_size       = 6
      max_size       = 24
      capacity_type  = "ON_DEMAND"
    }
    spot = {
      instance_types = ["m6i.xlarge", "m6a.xlarge", "m5.xlarge"]
      desired_size   = 2
      min_size       = 0
      max_size       = 12
      capacity_type  = "SPOT"
    }
  }

  tags = {
    "efe.io/cost-center" = "ACME-FRA-2026"
  }
}

output "kubeconfig_command" {
  value = module.efe_stack_aws.kubeconfig_command
}

output "db_secret_arn" {
  value = module.efe_stack_aws.db_secret_arn
}

output "byok_kms_key_arn" {
  value = module.efe_stack_aws.byok_kms_key_arn
}
```

### 15.4 Diagramme séquence détaillé : `efe-admin tenant create`

```
SE laptop                     mcp-server                efe-server                 Postgres
    │                              │                          │                          │
    │ 1. efe-admin tenant create  │                          │                          │
    ├──────────────────────────────►                          │                          │
    │                              │ 2. validate input        │                          │
    │                              │                          │                          │
    │                              │ 3. BEGIN tx              │                          │
    │                              ├─────────────────────────────────────────────────────►
    │                              │                          │                          │
    │                              │ 4. SELECT tenant FOR EXIST                          │
    │                              ├─────────────────────────────────────────────────────►
    │                              │ ◄─── (no row) ──────────────────────────────────────┤
    │                              │                          │                          │
    │                              │ 5. INSERT tenants ...   │                          │
    │                              ├─────────────────────────────────────────────────────►
    │                              │                          │                          │
    │                              │ 6. INSERT api_keys ...  │                          │
    │                              ├─────────────────────────────────────────────────────►
    │                              │                          │                          │
    │                              │ 7. INSERT ui_audit_log  │                          │
    │                              ├─────────────────────────────────────────────────────►
    │                              │                          │                          │
    │                              │ 8. COMMIT                │                          │
    │                              ├─────────────────────────────────────────────────────►
    │                              │ ◄─── OK ────────────────────────────────────────────┤
    │                              │                          │                          │
    │                              │ 9. notify efe-server     │                          │
    │                              ├──────────────────────────►                          │
    │                              │                          │ 10. invalidate cache    │
    │                              │                          ├──────────────────────────►
    │                              │ ◄────── OK ──────────────┤                          │
    │ ◄──────── 11. response ──────┤                          │                          │
    │           tenant_id, token   │                          │                          │
```

### 15.5 Checklist publication v1.0.0

```markdown
## Pre-flight v1.0.0

### Code
- [ ] `cargo test --workspace --all-features` GREEN
- [ ] `cargo clippy --workspace --all-features -- -D warnings` GREEN
- [ ] `cargo audit` 0 vulnerabilities HIGH/CRITICAL
- [ ] `pytest mcp-server/tests/` GREEN
- [ ] `npm run test` (portal) GREEN
- [ ] `npm run test:e2e` (Playwright) GREEN sur 3 browsers
- [ ] `helm-unittest charts/efe-stack` GREEN
- [ ] `terraform validate` + `tflint` + `tfsec` GREEN sur 3 modules
- [ ] `terratest` GREEN sur AWS sandbox

### Docs
- [ ] CHANGELOG.md updated
- [ ] docs/onboarding/SE_PLAYBOOK.md reviewed
- [ ] docs/onboarding/DPO_QUICKSTART.md reviewed
- [ ] 12 runbooks reviewed by Ops Lead
- [ ] Terraform README.md sample HCL tested

### Artefacts
- [ ] Docker images built + signed Sigstore (efe-server, mcp-server, portal-admin)
- [ ] Helm chart packaged + pushed to OCI registry
- [ ] efe-admin binaries built for 6 targets + signed
- [ ] Terraform modules tagged tf-aws/v1.0.0, tf-gcp/v0.9.0, tf-azure/v0.8.0

### Validation finale
- [ ] Onboarding 30 min reproduit avec succès par 3 ingés différents (asciinema enregistré)
- [ ] Audit pass chirurgical : ≤ 6 défauts trouvés et corrigés
- [ ] 0 régression M16-M22 confirmée
- [ ] DPO Acme (tenant pilote) a validé le portail

### Communication
- [ ] Release notes draftées + revues marketing
- [ ] Webinaire SE training programmé
- [ ] Support runbook escalation path docs published
```

---

## §16. Conclusion

GLOSS transforme EFE d'un produit techniquement complet en un produit commercialement déployable. Les 5 piliers (CLI / Helm / Terraform / Runbooks / Portail) sont **interdépendants mais découplables** — chaque pilier peut shipper indépendamment si nécessaire, mais leur valeur cumulative est ce qui permet le **30-min onboarding self-service**.

L'investissement (3 semaines × 1 ingénieur senior + audit pass) débloque un ROI commercial de l'ordre de **100 000 € annuels** en effort onboarding sauvé, **avant même** de compter le multiplicateur sur le pipeline commercial (un commercial peut faire 5 POCs/mois au lieu de 1).

GLOSS est par conception **opt-in** : aucun tenant existant n'est forcé d'utiliser le portail ou le CLI. Toute la livraison est backward-compatible avec M16-M22 (test invariant inscrit dans la CI).

Le successeur naturel après GA GLOSS est **M23 (DPO Export + Active Learning)** — désormais possible parce qu'on aura des tenants réels en suffisamment grand nombre pour générer un dataset DPO significatif.

---

**Fin du document — Architecture v1.0 design freeze 2026-05-08**
