# EFE Security Policy

> **Status** : Sprint M14 D40 audit fix M14-A3 — top-level security policy.
> **Audience** : security researchers, customers, regulators.
> **Last updated** : 2026-04-30.

---

## 🛡 Reporting a vulnerability

We take security vulnerabilities seriously and welcome coordinated
disclosure from researchers.

### Quick steps

1. **Email** : security@efe.ai (PGP-encrypted preferred — key at
   `https://efe.ai/.well-known/security.txt`)
2. **GitHub Security Advisory** : for non-critical findings,
   https://github.com/efe-project/efe/security/advisories/new
3. **Bug bounty (post-1.0 GA)** : HackerOne private program

For full details on our coordinated disclosure procedure, see
`docs/security/vulnerability_disclosure.md`.

---

## 🔐 Supported versions

EFE 1.0 LTS support window :

| Version | Supported until | Severity threshold |
|---|---|---|
| **1.0.x** | 2031-12-31 (5-year LTS) | All severities until 2027-12 ; Critical+High until 2030-12 ; Critical only until 2031-12 |
| 0.x | EOL 2026-12-31 | Critical only |

---

## 🔒 Security architecture

EFE 1.0 GA implements defense-in-depth via :

- **Authentication** : OIDC + JWT + JWKS rotation (D15)
- **Authorization** : Per-tenant RBAC (D11)
- **Network isolation** : NetworkPolicy strict (M12 D21)
- **Transport** : TLS 1.3 + mTLS optional (M12 D22)
- **Audit log** : HMAC-chained tamper-evident (D14-bis)
- **Supply chain** : Bundle Ed25519 signature + SBOM CycloneDX 1.5 (M13)
- **Backup + DR** : Quarterly drill + 7-yr Object Lock (D17 + D40)
- **Continuous monitoring** : Prometheus + Loki + OTel (D13)

For details, see :
- `docs/security/threat_model.md` — STRIDE consolidated, 66 threats
- `docs/compliance/iso27001.md` — ISO 27001:2022 Annex A mapping
- `docs/compliance/soc2_evidence.md` — SOC 2 Trust Services Criteria
- `docs/compliance/gdpr_art32.md` — GDPR Art. 32 mesures
- `docs/compliance/ai_act_annex_iv.md` — EU AI Act Annex IV
- `docs/compliance/nist_800_53.md` — NIST SP 800-53 r5

---

## 🏛 Compliance certifications

EFE 1.0 GA targets :
- **SOC 2 Type II** — initial audit 2026-Q3 → annual
- **ISO 27001:2022** — Stage 1+2 2026-Q3-Q4 → 3-year recert cycle
- **GDPR Art. 32** — documented + DPIA template + DPA template
- **EU AI Act 2024/1689** — Annex IV technical doc (limited-risk)
- **NIST SP 800-53 r5** — 97 % Moderate baseline

---

## ⚖ Responsible disclosure timeline

Per `docs/security/vulnerability_disclosure.md` :

```
T+0       : Report received → automated ack
T+1d      : Triage by security team
T+3d      : Severity assigned + reporter notified
T+7d      : Critical findings → mitigation in progress
T+30d     : High findings → fix shipped or compensating control
T+60d     : Medium findings → fix in next release
T+90d     : Public disclosure (default ; negotiable)
T+90d+1   : CVE assigned (if applicable) + advisory published
```

---

## 🛠 Severity classification

| Severity | Description | Fix SLA |
|---|---|---|
| **Critical** | RCE, privilege escalation, data exfil at scale | 7 days |
| **High** | Auth bypass, cross-tenant info disclosure | 30 days |
| **Medium** | Limited info disclosure, DoS single-tenant | 90 days |
| **Low** | Hardening recommendation, info leak | Next minor release |
| **Informational** | Best practice deviation | Documented, no fix |

---

## 🤝 Safe harbor

Per `docs/security/vulnerability_disclosure.md` §5 — researchers
acting in good faith have safe harbor.

---

## 📞 Contact

| Role | Email | PGP |
|---|---|---|
| Security team | security@efe.ai | yes (`.well-known/security.txt`) |
| DPO (privacy issues) | dpo@efe.ai | yes |
| Press / coordinated disclosure | comms@efe.ai | optional |
| Compliance | compliance@efe.ai | optional |

---

## 🏆 Hall of fame

Researchers who responsibly disclose are listed at
`https://efe.ai/security/researchers/` (with consent).

---

## 📜 Standards we follow

- ISO 29147 (Vulnerability Disclosure)
- ISO 30111 (Vulnerability Handling)
- NIST SP 800-216 (Coordinated Vulnerability Disclosure)
- EU CRA 2024 Art. 11 (vulnerability handling)
- SOC 2 CC3.4 (vulnerability identification + response)

---

## 📜 License

EFE is open source under the **Apache License 2.0**.
