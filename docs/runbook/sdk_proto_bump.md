# Runbook — SDK Proto Bump (= MAJOR semver coordination)

> Sprint **Φ-C-SDK Completion (D33)** — artefact #33.
> Cf. `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md` § 7.2.

## Quand utiliser ce runbook

Un developer veut modifier `protos/efe/v1/*.proto` (= ajout / suppression
de field, change de type, rename de message). Cette modification :

1. Casse le contract test (= D-SDK-1) si on ne regénère pas les 4 SDKs.
2. Casse le proto hash baseline (= D-SDK-2) → PR-gate red.
3. Force un **MAJOR semver bump** sur les 4 SDKs (= D-SDK-6).

Ce runbook décrit le process **chirurgical** pour faire le bump
proprement.

## Étape 1 : Confirmer la nécessité

```bash
git diff origin/main -- protos/efe/v1/
```

Si les diffs sont :
* Ajout d'un field optional → MINOR bump suffit (= rétro-compatible).
* Ajout d'un field required → MAJOR bump.
* Suppression / rename → MAJOR bump.
* Changement de type → MAJOR bump.

## Étape 2 : Local validation

```bash
# 1. Acknowledge breaking change explicitly.
SDK_PROTO_BUMP=yes ./scripts/sdk/regen_all.sh

# 2. Inspect generated files (= what changed in the 4 SDKs).
git diff -- clients/go/efe-sdk/generated/
git diff -- clients/typescript/@efe/sdk/src/generated/
git diff -- clients/java/efe-sdk/efe-sdk-core/target/generated-sources/
git diff -- clients/python/efe/generated/

# 3. Run contract test locally with mock mode.
cd sdk/contract-runner
EFE_CONTRACT_MOCK=1 docker-compose up --abort-on-container-exit \
    --exit-code-from runner-compare
```

Si le contract test FAIL en mock, c'est attendu (= les runners utilisent
des formules hardcoded qui doivent être mises à jour). Update les runners
mock pour matcher le nouveau format.

Si le contract test PASS en mock, le proto change est isomorphic ; c'est
OK.

## Étape 3 : Bump versions

Tous les 4 SDKs doivent passer à la **même** version MAJOR :

| SDK | Manifest | Field |
|-----|----------|-------|
| Python | `crates/efe-ffi/Cargo.toml` + workspace `Cargo.toml` | `version = "X+1.0.0"` |
| Go | `clients/go/efe-sdk/version.go` | `const Version = "X+1.0.0"` |
| TS | `clients/typescript/@efe/sdk/package.json` | `"version": "X+1.0.0"` |
| Java | `clients/java/efe-sdk/pom.xml` | `<version>X+1.0.0</version>` |

Vérifier le sync :
```bash
python3 scripts/sdk/verify_version_sync.py
```

## Étape 4 : Mettre à jour le baseline

```bash
# Le baseline est mis à jour AUTOMATIQUEMENT par regen_all.sh avec
# SDK_PROTO_BUMP=yes (= cat $CURRENT_HASH > baseline).
cat bench_baselines/v0.6.0/sdk_proto_hash.txt
```

Si vous avez fait le bump manuellement (= sans `regen_all.sh`), mettre à
jour manuellement :
```bash
NEW=$(find protos/efe/v1 -name '*.proto' -print0 | sort -z | xargs -0 cat | sha256sum | awk '{print $1}')
echo "$NEW" > bench_baselines/v0.6.0/sdk_proto_hash.txt
```

## Étape 5 : Document the breaking change

Update `CHANGELOG.md` :

```markdown
## v1.0.0 (2026-MM-DD)

### BREAKING CHANGES

* Renamed `Verdict.foo` → `Verdict.bar` (= proto v1.0)
* Required field `Verdict.baz` added (= proto v1.0)

### Migration guide

Cf. `docs/sdk-MIGRATION.md` v0.x → v1.0 section.
```

## Étape 6 : PR + CI verification

```bash
git add -A
git commit -m "feat(proto): breaking — rename Verdict.foo to Verdict.bar (MAJOR bump)"
git push origin proto-rename-foo

gh pr create --title "[BREAKING] Proto rename foo → bar" --body "..."
```

CI must verify :
* `sdk-quartet-contract.yml` proto-and-version-sync : PASS (= new baseline matches).
* `sdk-quartet-contract.yml` contract-mock : PASS (= runners updated).
* `sdk-quartet-contract.yml` pre-flight : PASS (= pin tests including `test_phi_c_sdk_proto_hash_matches_current_files`).

## Étape 7 : Release coordination

Une fois la PR mergee sur `main`, **NE PAS TAG IMMEDIATELY**. Annoncer
en interne d'abord :

```text
[#efe-eng] BREAKING : proto v1.0 lands tomorrow.
- Customers on v0.x will need to upgrade their SDK to v1.x.
- Migration guide : docs/sdk-MIGRATION.md
- LTS commitment : v0.6.x receives security patches for 6 months.
```

Puis tag + release :
```bash
git tag -a v1.0.0 -m "Φ-D quartet : proto v1.0 breaking changes"
git push origin v1.0.0
```

## Anti-patterns à éviter

| Pattern | Conséquence |
|---|---|
| Modifier proto sans `SDK_PROTO_BUMP=yes` | CI red sur proto-and-version-sync |
| Bumper seulement 1 SDK | SLO-SDK-VERSION-SYNC red |
| Skip le contract test mock | Régression silencieuse dans les runners |
| Force-push le tag v1.0.0 | Casse `go get` cache |
| Oublier le CHANGELOG | Customers confused, no migration path |

## Liens

* Archi § 7 + § 6 : `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md`
* Migration guide : `docs/sdk-MIGRATION.md`
* SLO : `SLO-SDK-PROTO-HASH`, `SLO-SDK-VERSION-SYNC`
* Pin test : `test_phi_c_sdk_proto_hash_matches_current_files`
