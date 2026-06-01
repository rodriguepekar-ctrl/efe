# d13_audit_unknown_principal — Runbook : audit events with empty principal

> **Severity** : warning (compliance).
> **Alert** : `D13ObsAuditPrincipalUnknown`.
> **Owner** : SRE + Security.

## 1. Alert summary

> 5 % of audit events over the last hour have `principal=""`.
The handler that propagates the JWT `sub` claim into the
audit emitter regressed. Forensic evidence may be incomplete.

## 2. First 5 minutes — triage

1. LogQL :
   ```logql
   {stream="audit", level="INFO"} | json | principal=""
   ```
   to enumerate which events lack the principal.
2. Identify the action mix : is it `delete` (D12 audit fix
   territory), `evolve`, or `register` ?
3. Check the recent deploy : did the gRPC handler for that RPC
   land a regression ?

## 3. Common root causes

| Cause | Symptom | Fix |
|-------|---------|-----|
| `delete` calls with default `principal="(test)"` | All deletes have stub | Enforce JWT propagation at handler |
| Gateway not forwarding JWT | All RPCs affected | Fix ingress config |
| Pre-auth event leaking into audit stream | Unauthenticated paths emitting audit | Audit emitter check : skip when principal empty |

## 4. Mitigation

- If `delete` is the offender : ensure the gRPC handler reads
  the JWT `sub` from interceptor metadata and threads it to
  `registry.delete(key, force, principal=&sub)` (cf. D12 audit
  fix H-4).
- If gateway issue : fix ingress to forward `Authorization`
  header.

## 5. Resolution

- Add a Rust integration test that asserts no audit event
  has `created_by=""` after a successful authenticated call.

## 6. Escalation

If compliance audit happens : security team must be informed
of the gap window.

## 7. Postmortem

Mandatory for compliance reasons.
