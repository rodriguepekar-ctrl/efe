# RB_PR19_MSW4 — `TLSCertExpiringSoon`

| Field | Value |
|---|---|
| **Alert** | `TLSCertExpiringSoon` |
| **Severity** | warning (ticket) |
| **Dashboards** | [D6 TLS](https://grafana.efe.internal/d/efe-msw-tls) |
| **PR ref** | [§ 25](../EFE_A1_PHASE2_PR19_ARCHITECTURE.md#25-alert-a4--tlscertexpiringsoon) |

## Summary

A TLS certificate expires in less than 7 days. cert-manager is configured
to auto-renew at 2/3 of the cert lifetime — this alert firing typically
means renewal is stuck.

## Symptom

- Slack ticket with `cert_name` + `subject_cn`.
- **D6 P1** "Days until next cert expiry" stat is red.
- **D6 P2** "Cert expiry table" shows the offending cert in red row.

## Diagnose

```bash
# Inspect the cert-manager Certificate resource
kubectl -n efe describe certificate <cert_name>

# Check the Ready condition + Renewal status
kubectl -n efe get certificate <cert_name> -o yaml | yq '.status'

# Cert-manager controller logs
kubectl -n cert-manager logs deployment/cert-manager --tail=200 | grep <cert_name>
```

Common Ready=False reasons :
- `Failing` — Issuer / ClusterIssuer is broken (ACME challenge failing).
- `NotReady` — DNS-01 propagation slow.
- `Issuing` — renewal in progress but stuck.

## Mitigate

1. **Trigger manual renewal** (cert-manager v1.13+) :
   ```bash
   kubectl -n efe annotate certificate <cert_name> \
     cert-manager.io/issue-temporary-certificate=true --overwrite
   kubectl -n efe delete secret <cert_name>-tls
   # cert-manager will reissue
   ```
2. **If Issuer is broken**, fix the upstream :
   ```bash
   kubectl describe clusterissuer <issuer_name>
   ```
3. **As a last resort** : manually issue an emergency cert with `cfssl` and
   patch the Secret. Document in the postmortem.

## Escalate

| Time / state | Action |
|---|---|
| < 24 h until expiry | Page meta-swarm-sre lead immediately |
| Sustained 24 h+ | Page platform-engineering lead |
| Multiple certs expiring | Treat as a cert-manager outage ; page cert-manager owner |

## Postmortem

Required when renewal automation fails. Identify the gap (cert-manager
config drift, Issuer credential rotation, ACME provider outage).
