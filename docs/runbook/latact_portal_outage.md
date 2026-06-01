# Runbook — Portal Next.js outage (P2)

> **Doctrine** : D-LATACT-5 (= portail first-class) + D-LATACT-9 (= funnel telemetry granular).
> **Symptôme** : `SLO-P2-TTFC-2` (Sandbox ≤ 5 min) FAIL ou les 8 SLOs funnel return SKIP.

## Détection

* Alert : `portal_web_vitals_lcp_seconds{p99} > 4` (= LCP dégradé).
* Synthetic monitor : Pingdom check `/signup` → 5xx.
* Funnel : `efe_activation_signup_total[1h]` chute à 0.

## Causes probables

1. **Build broken** : dernier déploiement Next.js a régressé.
2. **PG down** : `onboarding_sessions` table inaccessible.
3. **Cloudflare WAF règle bloque tout** : false-positive sur Bot Fight.
4. **Cookie session SameSite=strict + cross-origin** : utilisateurs sortent du funnel à mi-parcours.
5. **`/api/activation/event` 5xx** : audit chain HMAC key absente.

## Triage rapide

```bash
# 1. Vérifier déploiement portail.
curl -I https://portal.efe.ai/signup | head -5
curl -I https://portal.efe.ai/api/health | head -5

# 2. Vérifier funnel insertions PG.
psql efe -c "SELECT COUNT(*) FROM onboarding_sessions WHERE signed_up_at > NOW() - INTERVAL '1 hour';"

# 3. Vérifier audit chain.
python3 eval/scripts/comp_audit_chain_daily.py --mode synth_pass
```

## Mitigation immédiate

1. **Build broken** : rollback via Vercel/Cloudflare Pages dashboard.
2. **PG down** : failover replica.
3. **WAF** : whitelist temporaire de `/signup` + `/api/*`.
4. **Session cookies** : vérifier SameSite + Domain config.
5. **Audit chain HMAC** : reload key depuis Key Vault.

## Mitigation durable

* Synthetic monitor toutes 60s pour `/signup` + `/sandbox` + `/api/activation/event`.
* Pin test `test_latact_portal_telemetry_typescript_present` empêche la régression du scaffold.
* Funnel events emis AVANT UI updates (= D-LATACT-9, voir `telemetry.ts`).

## Cross-references

* SLOs : SLO-P2-TTFC-* (8), AHA-* (2), CONV-*, ABANDON-*
* DT : DT-LATACT-PORTAL-LIVE-FUNNEL
