# Runbook — SDK Publish Failure

> Sprint **Φ-C-SDK Completion (D33)** — artefact #32.
> Cf. `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md` § 8.

## Symptômes

| Indicateur | Implication |
|---|---|
| `publish-python` job fail at tag-gate | PyPI upload error |
| `publish-go` job fail | Git tag conflict / push permission |
| `publish-ts` job fail | npm 403 / NPM_TOKEN expired |
| `publish-java` job fail | OSSRH / GPG / Sonatype issue |
| `compliance` job fail at "Gate via check_slo.py" | SBOM count or Sigstore count < 4 |

## Cas 1 : PyPI upload failure

```
ERROR: HTTPError: 403 Forbidden from https://upload.pypi.org/legacy/
Reason: The user X isn't allowed to upload to project Y.
```

Diagnostic :
* Vérifier que `PYPI_TOKEN` secret est valide.
* Vérifier que le nom du package PyPI est correct (= `efe` pas `efe-ai`).
* Vérifier que la version n'existe pas déjà (PyPI refuse re-upload).

Mitigation :
```bash
# Renouveler le token PyPI via https://pypi.org/manage/account/token/
# Update GitHub secret.
gh secret set PYPI_TOKEN
# Re-trigger.
gh workflow run sdk-quartet-publish.yml
```

## Cas 2 : Go module tag conflict

```
error: failed to push some refs to 'origin'
remote: error: refusing to update because the tag 'clients/go/efe-sdk/v0.6.0' already exists
```

Diagnostic : le tag a été créé par une release précédente.

Mitigation :
```bash
# Identifier la cause (= release répétée ? remediation manuelle ?).
git tag -l 'clients/go/efe-sdk/*'

# Si le tag existant pointe sur le même commit : safe to skip (= idempotent).
# Si pointe sur un autre commit : DANGEREUX, ne pas force-push. Bumper le patch :
# Ex : v0.6.0 existe → v0.6.1 nouveau.
```

## Cas 3 : npm 403 Forbidden

```
npm ERR! code E403
npm ERR! 403 403 Forbidden - PUT https://registry.npmjs.org/@efe-ai/sdk
npm ERR! 403 You cannot publish over the previously published versions
```

Diagnostic : version existe déjà sur npm.

Mitigation :
```bash
# Vérifier les versions publiées.
npm view @efe-ai/sdk versions --json

# Bumper la version + re-aligner tous les SDKs.
# Cf. runbook sdk_version_drift.md.
```

## Cas 4 : OSSRH GPG signing failure

```
[ERROR] Failed to execute goal org.apache.maven.plugins:maven-gpg-plugin
[ERROR] gpg: signing failed: No secret key
```

Diagnostic :
* GPG secret key not loaded → check `GPG_PRIVATE_KEY` secret.
* Passphrase mismatch → check `GPG_PASSPHRASE`.
* Key expired → re-generate + upload pubkey to keyserver.

Mitigation :
```bash
# Renouveler la clé GPG.
gpg --full-generate-key  # RSA 4096
gpg --armor --export-secret-keys <key-id> | base64 > /tmp/gpg-private.b64
gh secret set GPG_PRIVATE_KEY < /tmp/gpg-private.b64

# Upload pubkey to keyserver.
gpg --send-keys --keyserver keyserver.ubuntu.com <key-id>

# Update OSSRH Jira ticket avec la nouvelle clé.
```

## Cas 5 : Cosign signing failure

```
Error: signing [...]: getting OIDC token: searching for token: 
no matching credentials were found
```

Diagnostic : `permissions: id-token: write` missing dans le workflow YAML.

Mitigation : voir `sdk-quartet-publish.yml` :
```yaml
permissions:
  id-token: write   # required for Sigstore keyless OIDC
  contents: write
```

## Cas 6 : Sigstore / SBOM count < 4

```
SLO-SDK-SIGSTORE : FAIL (measured 3, expected ≥ 4)
```

Diagnostic : 1 SDK n'a pas produit son artefact signé. Inspecter les
4 artifacts uploads :
```bash
gh run download <run-id>
ls artifacts/
# Doit contenir : sdk-python-release, sdk-go-release, sdk-ts-release, sdk-java-release
```

Mitigation : identifier le SDK manquant + re-trigger SON job uniquement.

## Recovery général

Si la release est partiellement publiée (= 2/4 SDKs out + 2/4 fail) :

```bash
# 1. Identifier l'état de chaque registry.
./scripts/sdk/check_release_state.sh v0.6.0  # OUTPUT : per-SDK ✓/✗

# 2. Re-trigger les 2 SDKs manquants.
gh workflow run sdk-quartet-publish.yml --ref v0.6.0

# 3. Si impossible (= tag déjà consumed), bumper le patch :
git tag -a v0.6.1 -m "Re-release Φ-C-SDK (= recovery from partial publish)"
git push origin v0.6.1
```

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| Single registry fail | self-fix via retry | 4 h |
| Multi-registry fail | #efe-sdk Slack | 24 h |
| Customer-impacting (= release announced but artifacts down) | #efe-incidents PagerDuty | 1 h |

## Liens

* Archi § 8 : `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md`
* SLO : `SLO-SDK-SIGSTORE`, `SLO-SDK-SBOM`
* Workflow : `.github/workflows/sdk-quartet-publish.yml`
