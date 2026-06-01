# Runbook : d20_sample_bundle_quickstart_broken

> **Severity** : P2 (developer experience)
> **ETA to resolution** : 1 hour
> **Sprint origin** : M13 D20
> **Last updated** : 2026-04-30
> **Owner** : ML lead

---

## 1. Symptom

- Customer support : "`efe-admin classify` fails immediately on quickstart"
- Quickstart smoke test failing in customer's local dev

## 2. Why this matters

Adoption blocker for new users → revenue impact + bad first impression.

## 3. Diagnostics

```bash
# Run bootstrap
python3 scripts/release/bootstrap_dev_bundles.py

# Verify bundle structure
ls bundles/all-MiniLM-L6-v2-en-1.0.0/

# Verify signature
python3 scripts/release/verify_pulled_bundle.py \
    --bundle-dir bundles/all-MiniLM-L6-v2-en-1.0.0/ \
    --pubkey-id efe-dev-bundle-2026 \
    --allow-dev-placeholder

# Run quickstart pin tests
python3 -m pytest scripts/tests/test_d20_sample_bundle.py -v
```

## 4. Mitigation

1. Re-run bootstrap if bundle missing
2. Verify trust store has dev key
3. Verify customer's deployment uses `--allow-dev-placeholder`

## 5. Resolution criteria

Customer can run `efe-admin classify "hello"` and get a label.

## 6. Post-incident actions

- [ ] Update quickstart docs if pattern
- [ ] Update test coverage

## 7. Related runbooks

- `d16_bundle_signature_invalid.md`
- `d26_bundle_pull_oci_failed.md`
