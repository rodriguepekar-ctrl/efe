# RB_M27_EDGE_DDOS — Edge DDoS incident response

**Severity** : P0 if customer-impacting ; P1 if mitigated at CF.
**Owner** : SRE on-call.
**Tools** : Cloudflare dashboard, kubectl, terraform, PagerDuty.

## 1. Confirm

```bash
# Edge analytics in CF dashboard : look for sudden RPS spike,
# unusual geo / ASN concentration, sustained 429/403 from WAF.
# CLI :
curl -s -H "Authorization: Bearer $CF_TOKEN" \
  "https://api.cloudflare.com/client/v4/zones/$ZONE_ID/analytics/dashboard?since=-30m" \
  | jq '.result.totals'
```

Also check origin metrics : are `envoy_cluster_upstream_rq_total`
spiking too ? If yes, attack reached origin → escalate.

## 2. Identify pattern

- **L3/L4 flood (UDP/SYN)** : CF Anycast absorbs automatically.
  Confirm via "Network Layer DDoS Attack Protection" in CF dashboard.
- **L7 application flood** : look at top IPs + top paths in CF
  analytics → "Security Events".
- **Per-tenant amplification** : check `efe_envoy_rate_limit_exceeded_total`
  per tenant in Grafana ; one tenant spamming = different mitigation.

## 3. Mitigate

### 3.1 Enable IUAM (I'm Under Attack Mode) — global emergency

```bash
# In CF dashboard : Security > Settings > Security Level → "I'm Under Attack"
# Or via terraform :
cd terraform/cloudflare
# Edit main.tf adding cloudflare_zone_settings_override security_level
terraform apply -var-file=prod.tfvars
```

**Impact** : every browser visitor sees JS challenge. API clients
without Authorization header are blocked. Use only as nuclear option.

### 3.2 Tighten rate limits per IP

```hcl
# In terraform/cloudflare/main.tf, lower requests_per_period :
resource "cloudflare_ruleset" "rate_limit" {
  rules {
    ratelimit {
      requests_per_period = 50      # was 100
      ...
    }
  }
}
```

```bash
terraform plan && terraform apply
```

Propagates < 5 min.

### 3.3 Block specific ASN / country

```hcl
resource "cloudflare_ruleset" "waf_custom" {
  rules {
    action      = "block"
    description = "Block ASN 12345 (incident YYYY-MM-DD)"
    expression  = "(ip.geoip.asnum eq 12345)"
  }
}
```

### 3.4 Per-tenant quota slash

If one tenant is the source :

```sql
UPDATE tenant_feature
SET classify_quota_per_min = 100
WHERE id = '<offending_tenant>';
```

Picked up within 30s by `LimitsCache::limits_for`.

## 4. Escalate

- If origin RPS > 1000 → page Backend lead.
- If sustained > 1h → contact Cloudflare Enterprise Support (24/7).
- If customer-impacting > 15 min → P0 incident channel + status page
  update.

## 5. Post-incident

- File P0 incident report (template `docs/incidents/template.md`).
- Review WAF custom rules → add learned signatures.
- Lower CF Rate Limit thresholds permanently if attack pattern persists.
- Update threat intel (which ASNs / patterns are now blocklisted).

## 6. Rollback IUAM

```bash
# In CF dashboard : security_level → "medium" (default).
# Or terraform :
cd terraform/cloudflare
git diff   # confirm change reverted
terraform apply
```
