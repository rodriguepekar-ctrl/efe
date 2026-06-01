# EFE Distribution Channels

EFE artifacts are published across **6 distribution channels** so
downstream consumers can pick the surface that fits their workflow.
Every channel ships **signed, verifiable, and SLSA-3 provenance-attested**
artifacts.

---

## Channel matrix

| #  | Channel                | Artifact(s)                                                     | URI prefix                                                | Auth model      | SLA       |
|----|------------------------|----------------------------------------------------------------|-----------------------------------------------------------|-----------------|-----------|
| 1  | OCI registry (Docker)  | 4 images                                                       | `ghcr.io/efe-ai/{efe-server,mcp-server,portal-admin,efe-server-migrate}` | Public read     | 99.95 %   |
| 2  | OCI Helm registry      | 1 umbrella + 6 sub-charts                                      | `oci://ghcr.io/efe-ai/charts/efe-stack`                   | Public read     | 99.95 %   |
| 3  | GitHub Releases        | CLI tarballs (6) + checksums + cosign sig + cert + provenance  | `https://github.com/efe-ai/efe/releases/tag/v<X.Y.Z>`     | Public read     | 99.99 %   |
| 4  | Homebrew tap           | 1 cask (`efe-admin`)                                           | `efe-ai/homebrew-efe`                                     | Public read     | 99.9 %    |
| 5  | npm registry           | 1 package (`@efe/sdk`)                                      | `https://registry.npmjs.org/@efe/sdk`                  | Public read     | 99.9 %    |
| 6  | Terraform Registry     | 3 modules (`efe-stack/aws`, `efe-stack/gcp`, `efe-stack/azure`) | `registry.terraform.io/efe-ai/efe-stack/{aws,gcp,azure}`  | Public read     | 99.9 %    |
| 7  | crates.io              | 1 library crate (`efe-server`)                                 | `https://crates.io/crates/efe-server`                     | Public read     | 99.9 %    |

> **#7 is a 7th channel** — listed separately from the "core 6" because
> it is library-only (the binary is shipped via OCI). It still respects
> the same signing and provenance guarantees.

---

## Per-channel detail

### 1. OCI registry (Docker images)

- **Push driver** : `actions/attest-build-provenance@v1` + `cosign sign`
  in the `docker-images` job of `release-v1-ga.yml`.
- **Tag scheme** : `1.0.0` (semver) and `1.0.0-<git-sha-short>` (immutable
  pin). Optionally `latest` on stable releases (toggleable via workflow
  input).
- **Signature artifacts** : `cosign sign` produces the in-toto signature
  attached to the OCI layout (no extra files).
- **Provenance artifacts** : the SLSA provenance is uploaded as a
  separate `*.intoto.jsonl` blob attached via `actions/attest-build-provenance`.
- **Verification** :
  ```bash
  cosign verify ghcr.io/efe-ai/efe-server:1.0.0 \
    --certificate-identity-regexp "^https://github\.com/efe-ai/efe/.*release-v1-ga\.yml.*" \
    --certificate-oidc-issuer "https://token.actions.githubusercontent.com"
  ```
- **Mirroring** : tenants can mirror to private registries (Harbor,
  ECR, ACR, Artifactory) — signatures are layer-attached and survive
  mirror unchanged.

### 2. OCI Helm registry

- **Push driver** : `helm push` to OCI in the `helm-chart` job.
- **Sub-charts** : 6 sub-charts (`efe-server`, `mcp-server`, `portal-admin`,
  `migrations-job`, `monitoring`, `compliance-cron`) packaged inside the
  `efe-stack` umbrella.
- **Signature artifacts** : same `cosign sign` model as Docker (OCI-attached).
- **Verification** :
  ```bash
  cosign verify ghcr.io/efe-ai/charts/efe-stack:1.0.0 \
    --certificate-identity-regexp "^https://github\.com/efe-ai/efe/.*" \
    --certificate-oidc-issuer "https://token.actions.githubusercontent.com"
  ```

### 3. GitHub Releases

- **Push driver** : `gh release create` in the `github-release` job.
- **Assets** :
  - `efe-admin-linux-amd64.tar.gz` (+ `.sig` + `.cert.pem`)
  - `efe-admin-linux-arm64.tar.gz` (+ `.sig` + `.cert.pem`)
  - `efe-admin-darwin-amd64.tar.gz` (+ `.sig` + `.cert.pem`)
  - `efe-admin-darwin-arm64.tar.gz` (+ `.sig` + `.cert.pem`)
  - `efe-admin-windows-amd64.zip` (+ `.sig` + `.cert.pem`)
  - `efe-admin-windows-arm64.zip` (+ `.sig` + `.cert.pem`)
  - `SHA256SUMS` + `SHA256SUMS.sig`
  - `efe-stack-1.0.0.tgz` (Helm chart packed)
  - `slsa-provenance.intoto.jsonl`
- **Permanence** : GitHub Releases assets are immutable once attached.
  Yank converts the Release to **draft** but preserves the assets.

### 4. Homebrew tap

- **Push driver** : `scripts/release/render-homebrew.sh` → commit + push to
  `efe-ai/homebrew-efe` repository in the `homebrew-tap` job.
- **Formula** : reference template at `homebrew/efe-admin.rb` in this
  repo ; rendered formula committed to the tap repo.
- **SHA-256 substitution** : digests of the 4 macOS/Linux tarballs are
  pulled from the GitHub Release assets and substituted into the
  formula at render time.
- **Test** : the formula's `test` block runs `efe-admin --version` and
  `efe-admin --help` to validate command-line surface.
- **Yank** : the formula commit is reverted in the tap repo (a `revert`
  commit, not a force-push), preserving git history.

### 5. npm registry

- **Push driver** : `npm publish --provenance` in the `npm-publish` job.
  The `--provenance` flag attaches the SLSA-3 provenance via npm's
  built-in provenance mechanism (compatible with `actions/attest-build-provenance`).
- **Package** : `@efe/sdk` (TypeScript SDK).
- **Verification** :
  ```bash
  npm audit signatures @efe/sdk@1.0.0
  ```
- **Yank** : `npm deprecate @efe/sdk@1.0.0 "yanked, see VEX feed"` —
  npm does not allow `unpublish` after 72 h.

### 6. Terraform Registry

- **Push driver** : tag-based discovery (`tf-aws/v1.0.0`, `tf-gcp/v1.0.0`,
  `tf-azure/v1.0.0`). The Terraform Registry watches the repo and
  auto-imports modules on tag push.
- **Discovery delay** : up to 30 minutes after tag push.
- **Verification** : Terraform Registry validates module structure and
  surfaces the SLSA provenance attestation in the module landing page.
- **Yank** : modules cannot be deleted from Terraform Registry. A new
  patch tag (`tf-aws/v1.0.1`) supersedes ; the deprecated tag is
  flagged in the module README via a follow-up commit.

### 7. crates.io

- **Push driver** : `cargo publish` in a dedicated `cargo-publish` job
  (only run on master `v` tags, not sub-tags).
- **Crate** : `efe-server` library (binary distributed via OCI).
- **Yank** : `cargo yank --version 1.0.0 efe-server`. Yanked versions
  remain downloadable but not selectable by `cargo` resolver (with
  warnings).

---

## Mirroring guidance

For air-gapped tenants :

1. **Docker** : pull from `ghcr.io/efe-ai/...` and re-tag/push to private
   registry. `cosign verify` works against the new URL.
2. **Helm** : `helm pull oci://...` then `helm push` to private OCI.
3. **CLI** : download tarballs from GH Release ; vendored Linux distros
   may also offer .deb / .rpm via the `releases-bundle` extension.
4. **Terraform** : `terraform get` with a private registry mirror.
5. **npm** : `verdaccio` proxy ; SLSA provenance is preserved.
6. **Cargo** : `cargo vendor` or private registry.

For air-gapped verification, see `.sigstore/trusted-root.json` (TUF
trusted root pin) which allows offline cosign verification.

---

## Channel-by-channel SLA

The 99.95 % uptime targets above derive from underlying provider SLAs
(GHCR : 99.95 %, GitHub : 99.99 %, npm : 99.9 %). EFE adds no SLA on top
beyond a commitment to **republish within 24 h** if any channel becomes
unavailable for an extended period (e.g. via mirror).

A 7-day signed health snapshot for each channel is published at :
`https://efe.example.com/distribution/health.json`.

---

## References

- Release pipeline : `.github/workflows/release-v1-ga.yml`
- Verification : `docs/release/VERIFY_RELEASE.md`
- Compliance pack distribution : `docs/release/COMPLIANCE_PACKAGE.md`
- Sigstore policies : `tools/sigstore/cosign-policy.yaml`
