# Runbook — SDK Contract Test Failure

> Sprint **Φ-C-SDK Completion (D33)** — artefact #30.
> Cf. `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md` § 5.

## Symptômes

| Indicateur | Implication |
|---|---|
| `SLO-SDK-CONTRACT` fail en CI | < 400/400 bit-identity passes |
| `compare.py` reports `label mismatches` | Codegen / wire format drift |
| `compare.py` reports `confidence drift > 1e-6` | f32 precision divergence cross-language |
| One SDK times out | gRPC channel issue OR backend overload |
| All 4 SDKs return identical but wrong verdicts | Backend regression in `efe-server` |

## Diagnostic chirurgical

### Étape 1 : Identifier les SDKs divergents

```bash
cat sdk/contract-runner/shared/contract.json | jq '.slo_compliance.SLO-SDK-CONTRACT.per_sdk_breakdown'
```

Output attendu (= sain) :
```json
{
  "go":   {"passes": 100, "failures": 0},
  "ts":   {"passes": 100, "failures": 0},
  "java": {"passes": 100, "failures": 0},
  "py":   {"passes": 100, "failures": 0}
}
```

Si un SDK isolé fail (= ex `go: 0/100`), c'est un bug spécifique à ce SDK.
Si 3/4 SDKs fail vs `py` (reference), c'est `py` qui dérive.

### Étape 2 : Examiner les premières erreurs

```bash
cat sdk/contract-runner/shared/contract.json | jq '.slo_compliance.SLO-SDK-CONTRACT.first_10_failures'
```

Pattern : `<sdk>: <fixture_id>: <field>: ref=<X> other=<Y>`.

### Étape 3 : Confidence drift uniquement (= acceptable selon seuil)

Si `confidence_drift_max < 1e-6` mais > 0, c'est attendu (= f32 wire
roundtrip). Le SLO accepte ce niveau de drift.

Si `confidence_drift_max > 1e-6`, c'est un bug :
* Le SDK source effectue le calcul confidence en f64 au lieu de f32.
* Le wire decoder convertit f32 → f64 différemment.

### Étape 4 : Cas par SDK

#### SDK Go diverge

```bash
# Vérifier la version protoc-gen-go-grpc utilisée.
cd clients/go/efe-sdk
go list -m all | grep grpc
# Doit être : google.golang.org/grpc v1.62+

# Re-générer le codegen :
SDK_PROTO_BUMP=no ./scripts/sdk/regen_go.sh
```

#### SDK TS diverge

```bash
cd clients/typescript/@efe/sdk
pnpm list ts-proto
# Doit être : ts-proto 1.169+

# Re-générer :
./scripts/sdk/regen_ts.sh
```

#### SDK Java diverge

```bash
# Vérifier la version grpc-java.
mvn -B -pl efe-sdk-core dependency:tree | grep grpc-java

# Re-générer :
./scripts/sdk/regen_java.sh
```

#### SDK Python diverge (rare)

Le SDK Python utilise `efe._native` (= PyO3 wrap direct), donc il ne
passe pas par le wire format. Un mismatch ici signifie un bug dans
le Rust core, pas dans le SDK. Voir `EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md`.

## Mitigation immédiate

```bash
# 1. Reproduire localement
cd sdk/contract-runner
docker-compose down -v
EFE_CONTRACT_MOCK=1 docker-compose up --abort-on-container-exit \
    --exit-code-from runner-compare

# 2. Inspecter chaque sortie
for sdk in py go ts java; do
    echo "=== $sdk ==="
    jq '.verdicts[0]' shared/out_${sdk}.json
done
```

Si reproductible :
* Bisect git pour identifier le commit fautif.
* Si c'est un proto change accidentel : revert + bump baseline.
* Si c'est un codegen tool bump : revert ou hold le bump.

Si non-reproductible :
* Investiguer flaky network / timing en CI (= `runner-compare` healthcheck).
* Add retry logic au compare step si transient.

## Escalation

| Sévérité | Qui notifier | Délai |
|---|---|---|
| Single-SDK drift confirmed | #efe-sdk Slack | 4 h |
| Multi-SDK drift (backend regression) | #efe-incidents PagerDuty | 1 h |
| Proto bump accidentel | Block PR ; require explicit MAJOR bump | immediate |

## Liens

* Archi § 5 : `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md`
* SLO : `SLO-SDK-CONTRACT`
* Test : `sdk/contract-runner/compare.py`
* Workflow : `.github/workflows/sdk-quartet-contract.yml`
