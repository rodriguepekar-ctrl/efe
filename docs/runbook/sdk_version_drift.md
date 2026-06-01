# Runbook — SDK Version Drift

> Sprint **Φ-C-SDK Completion (D33)** — artefact #31.
> Cf. `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md` § 7.

## Symptômes

| Indicateur | Implication |
|---|---|
| `SLO-SDK-VERSION-SYNC` fail | 4 SDK manifests declare different versions |
| `verify_version_sync.py` reports `<missing>` | A manifest file was deleted |
| PR-gate blocks release at "pre-flight" | Version sync is gating the publish |
| Customer reports : "Python is v0.6 but Java is still v0.5" | Live drift in production |

## Diagnostic

```bash
python3 scripts/sdk/verify_version_sync.py 2>&1
```

Sample output (= drift detected) :

```
SDK version sync check :
──────────────────────────────────────────────────
  python   : 0.6.0
  go       : 0.6.0
  ts       : 0.5.2     ← drifted
  java     : 0.6.0
──────────────────────────────────────────────────

FAIL : version drift detected. Unique values : {'0.6.0', '0.5.2'}
```

## Root causes typiques

1. **Manual bump forgotten** : un dev a bumped Python + Go + Java mais
   oublié TS. Most common cause.

2. **CI release partiel** : un publish job a fail au milieu (= ex Maven
   GPG expired), donc certains SDKs sont à v0.6 et d'autres encore à
   v0.5 en registry. Côté repo le bump est fait pour tous.

3. **Hotfix path** : un patch v0.5.1 sur le branch `release/0.5` a été
   merged dans `main` sans rebase, créant un commit qui a v0.5.1 sur
   un SDK + v0.6.0 sur les autres.

## Fix path

### Cas 1 : Manuel bump forgotten

```bash
# Identifier le SDK qui a drift.
python3 scripts/sdk/verify_version_sync.py

# Bumper le manifest manquant. Exemple pour TS :
vim clients/typescript/@efe/sdk/package.json
# Change "version": "0.5.2" → "version": "0.6.0"

# Re-vérifier.
python3 scripts/sdk/verify_version_sync.py
# Doit retourner exit 0 + "ALL 4 SDKs aligned at version : 0.6.0"

# Commit + push.
git add clients/typescript/@efe/sdk/package.json
git commit -m "fix(sdk): align TS SDK to v0.6.0 (audit fix Φ-C-SDK)"
```

### Cas 2 : CI release partiel

```bash
# Identifier quels SDKs sont effectivement publiés à v0.6.0.
# PyPI :
pip index versions efe | grep "Available versions"
# npm :
npm view @efe-ai/sdk versions --json
# Go modules :
git ls-remote --tags origin | grep "clients/go/efe-sdk/v0.6"
# Maven Central :
curl -s "https://search.maven.org/solrsearch/select?q=g:ai.efe+AND+a:efe-java&rows=20" | jq

# Re-trigger les jobs qui ont fail :
gh workflow run sdk-quartet-publish.yml --ref v0.6.0
```

### Cas 3 : Hotfix path

Le branch `release/0.5` doit être merged avec stratégie `revert-and-recommit`
plutôt que straight merge :

```bash
# Sur main, revert le commit qui a introduit le mismatch.
git revert <sha>

# Cherry-pick le fix sur main avec un bump cohérent.
git cherry-pick <fix-sha>
# Puis manually bumper les versions pour rester à v0.6.x.
```

## Pre-publish enforcement

Le workflow `sdk-quartet-publish.yml` pre-flight job verifie version
sync AVANT de publier. Si drift, tous les publish jobs sont bloqués.

Pour le bypass (URGENT only, manual override) :
```bash
gh workflow run sdk-quartet-publish.yml \
    --ref v0.6.0 \
    -f version=0.6.0 \
    -f skip_version_check=true
```

⚠️ Le bypass NÉCESSITE un follow-up immediate pour réaligner les manifests.

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| Local drift (dev branch) | self-fix | n/a |
| PR drift (= caught by CI) | author re-aligns | 4 h |
| Release drift (= in registries) | #efe-sdk Slack | 24 h |
| Customer-impacting (= different versions in prod) | #efe-incidents PagerDuty | 1 h |

## Liens

* Archi § 7 : `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md`
* SLO : `SLO-SDK-VERSION-SYNC`
* Tool : `scripts/sdk/verify_version_sync.py`
