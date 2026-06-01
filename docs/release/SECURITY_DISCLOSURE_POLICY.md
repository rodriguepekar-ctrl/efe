# EFE Security Disclosure Policy

**Effective date** : 2026-05-09 (v1.0.0 GA)
**Owner** : EFE Security Team (`security@efe.example.com`)
**Public summary** : `https://efe.example.com/security`

---

## Reporting a vulnerability

We take vulnerability reports seriously. Please report any potential
security issue through one of the following channels :

| Channel               | URL / Address                                              | When to use                            |
|-----------------------|-----------------------------------------------------------|----------------------------------------|
| Email                 | `security@efe.example.com` (PGP key fingerprint below)     | Default channel                        |
| Web form              | `https://efe.example.com/security/report`                  | If you don't have PGP                  |
| GitHub Security Advisory | `https://github.com/efe-ai/efe/security/advisories`     | For source-tree / CVE-eligible issues  |
| Bug bounty            | `https://bounty.efe.example.com` (HackerOne)               | For paid disclosures                   |

**PGP fingerprint** (rotate yearly) : `00DE ADBE EFFE EDED FEED ...`

We commit to :
- Acknowledging receipt within **1 business day**.
- Triaging severity within **3 business days**.
- Providing a fix or mitigation timeline within **7 business days**.
- Publishing a CVE (where applicable) and crediting the reporter (with
  consent) at disclosure time.

---

## What we consider in scope

| Component                              | In scope | Notes                                                |
|----------------------------------------|---------|------------------------------------------------------|
| `efe-server` (gRPC, Rust)              | Yes      | Including FFI boundaries to llama.cpp                |
| `mcp-server` (Python)                  | Yes      | Tool-call sandbox boundaries                         |
| `portal-admin` (Next.js)               | Yes      | XSS / CSRF / CORS / RBAC bypass                      |
| `efe-admin` CLI (Rust)                 | Yes      | Including auth token handling                        |
| Helm chart (`efe-stack`)               | Yes      | Default-values security defaults                     |
| Terraform modules                      | Yes      | IAM minimization, secret handling                    |
| SDKs (Python / Go / TypeScript / Java) | Yes      | Including local cache / token storage               |
| Sigstore / SLSA pipeline               | Yes      | Pipeline integrity, identity claims                  |
| Public docs site                       | No       | Report directly to docs@                             |
| Test / dev fixtures                    | No       | Out of scope unless leaking real credentials         |

---

## What we don't accept

- **Theoretical attacks without PoC.** We need a working exploit or a
  clear chain of reasoning.
- **Self-XSS** that requires the user to paste content into their own
  console.
- **Outdated browser issues** that affect browsers older than the
  current Edge / Chrome / Firefox / Safari ESR.
- **DoS via traffic flooding.** Use load-testing platforms with our
  written consent (`security@efe.example.com`) instead.
- **Issues in third-party services** unless they directly compromise
  EFE's security posture (e.g. our GitHub org configuration).

---

## Severity scale

We use **CVSS 3.1** with EFE-specific mapping :

| Severity | CVSS                | Examples                                                                             |
|----------|---------------------|--------------------------------------------------------------------------------------|
| **P0** Critical | 9.0–10.0    | RCE on `efe-server`, full tenant isolation breach, KMS key extraction                |
| **P1** High     | 7.0–8.9     | Auth bypass on portal, audit chain forgery, CVE in shipped binary with active exploit |
| **P2** Medium   | 4.0–6.9     | Privilege escalation requiring authenticated access, info-leak across tenants        |
| **P3** Low      | 0.1–3.9     | XSS in admin-only page, missing security header                                      |

---

## Response timeline

| Severity | Acknowledge  | Mitigation     | Public CVE            |
|----------|--------------|----------------|-----------------------|
| P0       | 1 hour       | 24 hours       | Coordinated, ≤ 30 days |
| P1       | 4 hours      | 72 hours       | Coordinated, ≤ 60 days |
| P2       | 1 business day | 7 days       | Coordinated, ≤ 90 days |
| P3       | 3 business days | Next release | Standard, ≤ 90 days   |

---

## Yank policy for affected releases

If a confirmed P0/P1 vulnerability is found in a published release :

1. **Sigstore signatures cannot be revoked.** Rekor is append-only by
   design. The signatures from the affected release remain valid in the
   transparency log forever — that is the desired property for audit.
2. We **deprecate** the release through every other channel :
   - Docker images : `crane mutate` adds `org.efe.deprecated=true` and
     `org.efe.yank-reason=<reason>` annotations.
   - Helm chart : republished as `1.0.0-yanked` on OCI ; the original
     `1.0.0` tag remains pullable but flagged.
   - GitHub Release : converted to **draft** (assets remain accessible
     via direct URL, but the Release page returns 404 to anonymous
     users).
   - npm : `npm deprecate @efe/sdk@1.0.0 "yanked, see VEX feed"`.
   - Cargo : `cargo yank --version 1.0.0`.
   - Homebrew : the formula commit is reverted in the tap repo.
3. We **append** a VEX statement to `docs/security/vex/feed.json` with
   `status: fixed`, `fix_version`, and `action_statement`.
4. We **append** an RSS event to `docs/security/feed.xml` (publicly
   crawlable feed).
5. We **email** every customer on the security mailing list within 1
   hour (P0/P1) with the yank reason and recommended action.
6. We **file** a CVE record with MITRE if not already filed.

The exact runbook is in `scripts/release/release-rollback.sh` and the
CI workflow `.github/workflows/release-rollback.yml`.

---

## Coordinated disclosure timeline

Our default coordinated-disclosure window is :

```
T+0       : reporter sends report
T+1 day   : we acknowledge
T+3 days  : triage and severity decision
T+X days  : fix shipped (X = 7 to 90 depending on severity)
T+X+30 days : public CVE published (allow customers to upgrade)
T+X+60 days : reporter may publish technical details (with our coordination)
```

If the reporter prefers a shorter or longer disclosure window, we
negotiate in good faith. We do not pursue legal action against
researchers acting in good faith under this policy.

---

## Safe harbor

EFE adopts a **safe harbor** posture for security research conducted
in accordance with this policy. Specifically :

- We will not pursue civil or criminal action against you for security
  research that is consistent with this policy.
- We consider security research conducted under this policy to be :
  - Authorized in view of the Computer Fraud and Abuse Act (CFAA),
  - Authorized in view of the Digital Millennium Copyright Act (DMCA),
  - Exempt from restrictions in our Terms of Service that conflict with
    this policy.

---

## Reporter recognition

With the reporter's consent, we publicly credit them :

1. In the CVE description.
2. In the GitHub Security Advisory.
3. In the EFE Hall of Fame (`https://efe.example.com/security/hall-of-fame`).

Reporters may opt for **anonymous credit** ("Anonymous Researcher")
without affecting their bounty eligibility.

---

## Bug bounty

Eligible reports are paid through **HackerOne**
(`https://bounty.efe.example.com`). Payouts (USD) :

| Severity | Min   | Max    |
|----------|-------|--------|
| P0       | $5,000 | $25,000 |
| P1       | $1,500 | $7,500  |
| P2       | $500  | $2,000  |
| P3       | $100  | $500    |

Bounties are funded by EFE Security and are paid within 30 days of
report validation, regardless of when the public disclosure happens.

---

## Subscribe

- **RSS feed** : `https://efe.example.com/security/feed.xml`
- **VEX feed** : `https://efe.example.com/security/vex/feed.json`
- **Mailing list** : `security-announce@efe.example.com`
  (subscribe at `https://efe.example.com/security/subscribe`)
- **Status page** : `https://status.efe.example.com`

---

## References

- Pipeline architecture : `docs/EFE_RELEASE_V1_GA_ARCHITECTURE.md`
- Yank workflow : `.github/workflows/release-rollback.yml`
- VEX feed format : OpenVEX 0.2.0 (https://openvex.dev/ns/v0.2.0)
- CVE publication : MITRE CVE Numbering Authority (CNA) status pending
