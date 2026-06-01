# RB_M32_TENANT_ISOLATION_AUDIT — Quarterly cross-tenant leak audit

**Severity** : N/A (planned). Calendar reminder.

## 1. Procedure

Run the pen-test bench every quarter :
```bash
EFE_API_URL=https://staging.efe.ai \
EFE_BEARER_A=<tenant_a_bearer> \
EFE_BEARER_B=<tenant_b_bearer> \
python3 bench/m32_vector_pentest/test_cross_tenant_leak.py
```

The bench :
1. Creates a collection under tenant A.
2. Tries to read / search / snapshot it as tenant B.
3. Asserts every cross-tenant operation returns 403 or 404.

## 2. If the bench fails

- Any LEAK finding ⇒ P1 incident.
- Escalate to Security + Compliance.
- Pull access for the affected tenant(s) immediately.

## 3. Archive evidence
- Upload the bench output to the SOC 2 evidence vault
  (`s3://efe-evidence/quarterly_audits/m32_isolation_<date>.txt`).
- Update `docs/compliance/audit_history.md`.

## 4. Post-audit
- File P3 retrospective if any non-blocking surprises.
- If the bench needs new attack vectors : extend
  `test_cross_tenant_leak.py`.
