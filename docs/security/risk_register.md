# EFE 1.0 GA — Risk Register

> **Status** : Sprint M14 D40 (Φ-A) — internal risk register.
> **Owner** : CISO + DPO (joint).
> **Last updated** : 2026-04-30.
> **Re-evaluation** : quarterly + post-incident.

---

## Methodology

Each risk evaluated on :
- **Likelihood** : Low / Medium / High
- **Impact** : Low / Medium / High / Critical
- **Inherent Risk** = Likelihood × Impact (pre-mitigation)
- **Residual Risk** = post-mitigation rating
- **Owner** : person responsible for monitoring
- **Treatment** : Accept / Mitigate / Transfer / Avoid

---

## Risk inventory

| ID | Risk | Likelihood | Impact | Inherent | Residual | Treatment | Owner |
|---|---|:---:|:---:|:---:|:---:|:---:|---|
| **R-01** | Supply chain compromise (HF model backdoored) | M | H | H | L | Mitigate (M13 sig + SBOM) | Security lead |
| **R-02** | OIDC issuer takeover | L | H | M | L | Mitigate (issuer revocation + JWKS) | Security lead |
| **R-03** | Postgres data corruption | L | H | M | L | Mitigate (D17 backup + DR drill) | DevOps lead |
| **R-04** | Multi-tenant data leakage | M | C | H | L | Mitigate (D11 RBAC + NP strict) | Eng lead |
| **R-05** | DDoS on customer-facing endpoint | M | M | M | M | Mitigate (HPA + WAF + circuit breaker) | DevOps lead |
| **R-06** | Compliance audit failure (SOC 2) | M | H | H | M | Mitigate (M14 docs + dry-run) | CISO |
| **R-07** | Pen-test reveals critical findings post-freeze | M | M | M | L | Mitigate (security-only PRs allowed) | Security lead |
| **R-08** | Customer breach via social engineering | M | H | H | M | Mitigate (training + 2FA + audit) + Transfer (cyber insurance) | CEO |
| **R-09** | EU AI Act 2026 reclassification → high-risk | L | H | M | M | Monitor (legal review quarterly) | Legal + CISO |
| **R-10** | Key person dependency post-handoff | H | M | H | M | Mitigate (HANDOFF_OPS + 90-day overlap) | CEO |
| **R-11** | Cloud provider region outage | L | H | M | L | Mitigate (multi-region backup + DR drill) | DevOps lead |
| **R-12** | Bundle Ed25519 key compromise | L | C | H | L | Mitigate (KMS + rotation + Rekor) | Security lead |
| **R-13** | GDPR DSR (data subject request) backlog | M | M | M | L | Mitigate (automation + 30-day SLA) | DPO |
| **R-14** | License non-redistributable smuggled in bundle | L | H | M | L | Mitigate (license_gate.py + manual review) | Security + Legal |
| **R-15** | Customer DDQ disagreement on N/A controls | M | L | M | L | Mitigate (DDQ pre-fill + rationale) | Sales eng |
| **R-16** | Insider threat (eng leaks signing key) | L | C | H | L | Mitigate (KMS + audit + branch protection + background checks) | Security lead |
| **R-17** | Unpatched CVE in transitive dep | M | M | M | L | Mitigate (Dependabot + cargo audit + monthly review) | Security lead |
| **R-18** | Helm chart upgrade rolls forward to broken state | M | M | M | L | Mitigate (helm test + rollback procedure + canary) | DevOps lead |
| **R-19** | OCI registry takeover | L | H | M | L | Mitigate (cosign + Rekor verify on pull) | Security lead |
| **R-20** | Documentation drift post-freeze | H | L | M | L | Mitigate (quarterly compliance review + link checking CI) | CISO |

---

## Quarterly review log

| Quarter | Reviewed by | Risks added | Risks closed | Risks re-rated |
|---|---|:---:|:---:|:---:|
| 2026-Q1 | CISO + DPO + Eng VP | 20 (initial) | 0 | 0 |
| 2026-Q2 | (planned) | TBD | TBD | TBD |
