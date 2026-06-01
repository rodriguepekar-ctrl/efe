# Runbook : bench-publish defacement (T116 / T122 mitigation)

**Trigger** : monitoring detects an unauthorised commit on the
`gh-pages` branch, OR a customer reports that `https://efe.github.io/bench/`
serves unexpected content.

## Phase 0 — Detect

Monitoring sources :
- GitHub branch-protection audit log (force-push attempt)
- Public uptime monitor on `/api/version.json` checks `harness_version`
  match expected
- Customer report channel `#bench-public-incident`

```bash
# Inspect the last commit on gh-pages.
gh api repos/efe-ai/efe/branches/gh-pages | jq '.commit.commit'
```

## Phase 1 — Triage

```bash
# Identify the unauthorised commit.
git fetch origin gh-pages
git log --oneline origin/gh-pages | head -20

# Compare against the last known-good commit (recorded in the
# release-eng on-call's notebook).
LAST_GOOD=<commit-from-on-call-notebook>
git log $LAST_GOOD..origin/gh-pages --oneline
```

The unauthorised commit must :
- NOT be signed by an `@efe-ai/release-eng` member
- NOT come from a workflow with the frozen identity regex
- Show diff that doesn't match the expected `bench/**` paths

## Phase 2 — Contain

```bash
# Page release-eng on-call (PagerDuty).
# Page security review (Slack #security).
# Temporarily remove the gh-pages CNAME from DNS if customer-visible
# defacement is harmful (last resort — clears propagation in ~5 min).
```

## Phase 3 — Recover

```bash
# 1. Branch-protect bypass (release-eng admin only).
gh api -X PATCH repos/efe-ai/efe/branches/gh-pages/protection \
    -f required_pull_request_reviews=null \
    -f restrictions=null

# 2. Revert to last known-good commit.
git checkout gh-pages
git reset --hard $LAST_GOOD

# 3. Force-push (logged in GitHub audit — release-eng admin signature).
git push --force-with-lease origin gh-pages

# 4. Restore branch protection IMMEDIATELY.
# (Use repo settings UI ; verifiable via :)
gh api repos/efe-ai/efe/branches/gh-pages/protection | jq .
```

## Phase 4 — Post-mortem (within 14 days)

File `docs/postmortems/bench-publish-defacement-YYYY-MM-DD.md` :
- Timeline (UTC) with each commit's author + signer + workflow source
- Root cause (leaked token / compromised account / CI bypass)
- 5-whys
- Process improvement (e.g. token rotation, additional CI guard)

## Reference

- PR #24 AD29 (gh-pages branch protection)
- PR #24 § 18 (branch-protection rules)
- PR #24 § 20 T116 + T122
- STABILITY_CONTRACT v1.6 § 24.7
