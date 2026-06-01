# 🔒 EFE 1.0 GA — REPOSITORY IS FROZEN

> **Effective date** : 2026-XX-XX (post-J175)
> **Cf.** : `docs/EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md` §15.
>
> **Audit fix M14-A6 (LOW)** — this file's name (`CONTRIBUTING-POST-FREEZE.md`)
> is custom and NOT auto-loaded by GitHub's PR templates. At freeze time,
> these contents should be **merged into the root `CONTRIBUTING.md`** (or
> `.github/CONTRIBUTING.md`) so GitHub auto-displays them in the PR UI.
>
> ```bash
> # At freeze time (J175) :
> cat .github/CONTRIBUTING-POST-FREEZE.md >> CONTRIBUTING.md
> # Or replace the active section :
> cp .github/CONTRIBUTING-POST-FREEZE.md .github/CONTRIBUTING.md
> ```
>
> Until merged, this file serves as the staged version of the
> post-freeze contribution guide.

The repository is in **operate mode**. Only the following PR types
are accepted :

- 🔒 **Security patches** (CVE response, pen-test findings)
- 🐛 **Critical bug-fixes** (P0 outage causes)
- 📋 **Compliance updates** (regulator changes, audit findings)
- 📚 **Documentation clarifications** (no behavioral changes)

Feature PRs and non-trivial refactors will be **closed without merge**
and may be considered for the EFE 2.0 cycle (planned start 2027-Q1).

---

## How to submit an accepted PR

1. **Open an issue first** — describe the bug / CVE / compliance change
2. **Wait for maintainer triage** (typically < 48 hours for P0/P1)
3. **Submit PR** with explicit type prefix in the title :
   - `[SECURITY]` for security patches
   - `[CRITICAL-FIX]` for critical bug fixes
   - `[COMPLIANCE]` for compliance updates
   - `[DOCS]` for documentation clarifications
4. **Add the `accepted-post-freeze` label**
5. **2 reviewers required** (CODEOWNERS-enforced)
6. **All status checks must pass**
7. **Commit must be signed**

---

## Severity-driven SLAs

| PR type | Approver | SLA |
|---|---|---|
| Security patch | 2× security team | < 7 days |
| Critical bug-fix | 2× engineering team | < 7 days |
| Compliance | 2× compliance + security | < 30 days |
| Documentation | 1× any reviewer | < 3 days |
| Dependency security update | 1× security team | < 7 days |

---

## What's NOT accepted

- ❌ New features
- ❌ Non-trivial refactors
- ❌ Performance optimizations (unless fixing P0 SLO breach)
- ❌ API surface changes
- ❌ Breaking changes
- ❌ Documentation rewrites for tone/style only

If your contribution doesn't fit, please :
- File it as an issue with the `2.0-candidate` label
- Or contact the team via security@efe.ai for urgent items

---

## Tag signing requirement

All release tags post-freeze MUST be GPG-signed by the release authority key :

```bash
git tag -s v1.0.<patch> -m "EFE 1.0.<patch> — <description>"
git push origin v1.0.<patch>
```

Unsigned tags will be rejected.

---

## Current support window

- 1.0.x patch releases : every 30 days (or earlier for security)
- 1.0 LTS support : through 2031-12-31 (5 years)
- 1.0 EOL : 2031-12-31

---

## Roadmap to 2.0

EFE 2.0 development planned to start **2027-Q1** in parallel with 1.0
operations. Major themes :
- FedRAMP Moderate authorization path
- Multi-bundle per-tenant routing
- GPU-first deployments
- Additional language SDKs (.NET / Ruby / Rust client)

If you're interested in 2.0 contributions, please email
roadmap@efe.ai for early access.

---

## Contact

- **Security PRs** : security@efe.ai
- **Compliance PRs** : compliance@efe.ai
- **General** : maintainers@efe.ai
- **Bug reports** : https://github.com/efe-project/efe/issues
