# RB_M30_INPUT_GUARD_SPIKE — Multimodal input-guard rejection spike

**Severity** : P3 (investigate ; potential abuse).
**Trigger** : `M30MultimodalGuardRejectionSpike` for ≥ 15 min.

## 1. Identify the offender
```bash
# Top tenants triggering rejections in the last hour.
curl -s "$PROM_URL/api/v1/query" --data-urlencode \
    'query=sum by(tenant_id, reason) (increase(efe_multimodal_rejections_total[1h]))'
```

The label `reason` is one of :
- `too_large` — payload exceeds the cap (DDOS or misconfig).
- `invalid_format` — magic-byte mismatch (spoofing or bug).
- `invalid_duration` — audio too long.
- `modality_disabled` — caller without DPIA signature.

## 2. Triage

### Legitimate customer bug
- Reach out via support — likely a client SDK serialization bug.
- Send them a fixture that round-trips correctly.

### Abuse / pen-test probing
- Cross-reference the request log : look for the same tenant
  trying 10+ different magic-byte combinations.
- If confirmed abuse : apply tighter Cloudflare WAF rule for
  this tenant's API key. Suspend if criminal probing pattern.

### Configuration drift
- If `modality_disabled` spike : check the tenant's
  `tenant_modality_policy` row.
- Common cause : DPIA timestamp accidentally NULL'd by a UI bug.

## 3. Mitigation
1. If a specific tenant is hammering : enable per-tenant rate
   limit at the Cloudflare layer.
2. If the rejection is across many tenants : check the M30
   architecture doc § 6 for a recent change in the magic-byte
   allow-list. Roll back if regressive.

## 4. Post-incident
- If abuse confirmed : open a Compliance ticket for ToS violation.
- If client bug confirmed : add the pattern to the M30 pen-test
  bundle `bench/m30_multimodal_pentest/`.
