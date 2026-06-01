# Drill : gh-pages defacement response

**Drill ID** : `D-2026-Q2-02`
**Frequency** : quarterly (aligned with release cadence)
**Duration** : 60 minutes (45 min exercise + 15 min debrief)
**Owner** : `@efe-ai/release-eng`
**Last rehearsal** : N/A (first run)
**Next rehearsal** : 2026-08-15

This drill exercises [`bench-publish-defacement.md`](../runbooks/bench-publish-defacement.md)
end-to-end against a **staging** gh-pages branch.  It validates :

- Detection time : how fast does an operator notice a defacement?
- Branch-protection bypass : the time-boxed override + restore.
- Roll-forward (NOT roll-back) — gh-pages is append-only.
- Comms cadence under pressure.

**Threat model covered** : T116 (compromised maintainer push) +
T122 (concurrent publish race).  These are the two scenarios where
gh-pages content can be corrupted by writes that bypass the normal
publish workflow.

---

## 0. Why this drill exists

**Defacement is the highest-blast-radius incident class** for the
bench-publish surface :

- Auditors read the JSON API endpoints directly.  A doctored
  `current.json` could falsely claim SLO compliance.
- Customer trust collapses if the page shows defamatory content even
  briefly.  Cached CDN copies can persist for hours.
- An attacker with maintainer push access could backdate signed evidence.

The 10-pass audit found **D-Z** (concurrent `write_canonical` race)
and **D-J / D-N** (the smoke-flag was a no-op, meaning auditors got
silently misleading badges).  Neither is a "defacement" strictly,
but they're failure modes adjacent to it.  This drill makes operator
response to *active* defacement muscle-memory before it's needed.

---

## 1. Pre-drill checklist (T-7 days)

- [ ] Schedule a 60-minute slot in a war-room channel.
- [ ] Confirm participants : 1 on-call lead, 1 observer, 1 GitHub-admin
      with the `gh-pages` branch-protection-bypass token.
- [ ] Stand up a **staging** gh-pages branch as the scratch space :
  ```bash
  # NEVER drill against the prod gh-pages branch.  Clone the current
  # state into a private fork or a renamed branch in the prod repo.
  git clone --branch gh-pages https://github.com/efe-ai/efe.git /tmp/gh-pages-staging
  cd /tmp/gh-pages-staging
  git remote rename origin upstream
  git remote add origin git@github.com:efe-ai/efe-drill-staging.git
  git push -u origin gh-pages:drill-staging
  ```
- [ ] Verify branch protection rules :
  - Force-push : **disabled** (target state for drill)
  - Signed commits : **required**
  - Linear history : **enforced**
  - CODEOWNERS for `bench/**` : `@efe-ai/release-eng`
  - 2 approvals required for merges
- [ ] Verify tools available :
  - `gh api` authenticated as admin (for branch-protection mutations)
  - `cosign` 2.x on PATH
  - `efe-bench-harness` ≥ v0.2.0 release binary

---

## 2. Drill scenario (T-0)

The fictional setup, briefed at minute 0 :

> Maintainer `bob@efe-ai` reports at 14:22 UTC that a customer-success
> engineer noticed `https://efe.github.io/bench/index.html` shows the
> string "Owned by L33tCrew" between the smoke badge and the scenario
> table.  The current.json endpoint still shows the right verdict
> (pass), but the HTML page itself carries this content.
>
> The most recent commits on gh-pages :
>
> - `commit ef0123...` (now)  "publish bench results from <legit sha>"
> - `commit ab9876...` (5 min ago)  "h4h4 owned" — author `attacker@`
>
> The push that introduced ab9876 bypassed the normal
> `bench-publish.yml` workflow and was force-pushed direct to gh-pages.
> Branch protection should have prevented this — investigate.

This is a T116 incident (compromised maintainer push, force-push that
SHOULDN'T have been allowed).

---

## 3. Exercise (45 min)

### 3.1 Phase A — Detection + containment (T+0 to T+10)

```bash
# T+0    Acknowledge the alert in #release-eng (template in master runbook § 5.2).
# T+1    Confirm the defacement by direct curl.
curl -fsSL https://efe.github.io/bench/ \
    | grep -E "L33tCrew|h4h4|owned"

# T+2    Disable the deploy workflows so further normal-flow publishes
#        don't compete with you while you investigate.
gh workflow disable bench-publish --repo efe-ai/efe-drill-staging
gh workflow disable bench-publish-quarterly --repo efe-ai/efe-drill-staging

# T+4    Snapshot the current state for the post-mortem.
mkdir -p incident-evidence/2026-Q2-drill
git -C /tmp/gh-pages-staging log --oneline -10 \
    > incident-evidence/2026-Q2-drill/gh-pages-recent.txt

# T+6    Inspect the attacker commit.
git -C /tmp/gh-pages-staging show ab9876 --stat
git -C /tmp/gh-pages-staging show ab9876 -- index.html | head -100

# T+8    Confirm branch-protection state.
gh api repos/efe-ai/efe-drill-staging/branches/drill-staging/protection \
    > incident-evidence/2026-Q2-drill/branch-protection.json
# Compare against the expected state (force-push: disabled).
```

### 3.2 Phase B — Time-boxed branch protection bypass (T+10 to T+25)

To roll forward (NOT back) over the defaced commit, the on-call needs
to add a CLEAN new commit that re-publishes the legitimate content.
But the normal publish workflow is disabled in Phase A, so we need
either :

- (a) Re-enable the workflow and trigger it — preferred when the
  workflow itself is uncompromised.
- (b) Hand-publish a single recovery commit — fallback if (a) is
  blocked by a deeper issue.

Choose (a) for this drill.

```bash
# T+11   Verify the workflow file itself isn't tampered with.
git -C /efe-source log -1 --stat .github/workflows/bench-publish.yml
# Output MUST show only the most recent legit commit modifying it.

# T+13   Re-enable the workflow.
gh workflow enable bench-publish --repo efe-ai/efe-drill-staging

# T+14   Trigger a forced publish from the last known-good ref.
gh workflow run bench-publish --repo efe-ai/efe-drill-staging --ref main

# T+15   Monitor.
gh run watch <run-id>
# The publish workflow's `peaceiris/actions-gh-pages` step uses
# `keep_files: true` (AD30) — past content is preserved, but the
# attacker's defacement of `index.html` will be OVERWRITTEN by the
# new publisher's index.html.  No force-push required.

# T+22   Verify the live site is clean.
curl -fsSL https://efe.github.io/bench/ \
    | grep -E "L33tCrew|h4h4|owned"
# MUST return empty (exit 1).

# T+24   Verify the attacker commit is STILL in the gh-pages history
#        — we do NOT erase it.  Auditor chain of custody requires the
#        defacement be visible as part of the timeline.
git -C /tmp/gh-pages-staging log --oneline | grep ab9876
```

### 3.3 Phase C — Branch-protection hardening (T+25 to T+35)

The attacker's commit got through.  Why?  Investigate :

```bash
# T+26   List recent push events to gh-pages.
gh api repos/efe-ai/efe-drill-staging/events \
    --jq '.[] | select(.type == "PushEvent" and .payload.ref == "refs/heads/drill-staging")
                | {actor: .actor.login, sha: .payload.head, forced: .payload.forced, time: .created_at}'

# Look for `"forced": true` on the attacker commit.

# T+28   Inspect the bypass actors list (a small set of admins can
#        legitimately bypass protections).
gh api repos/efe-ai/efe-drill-staging/branches/drill-staging/protection/required_pull_request_reviews \
    --jq '.bypass_pull_request_allowances'

# T+30   If attacker is in the bypass list : a maintainer account was
#        compromised.  Phase D : credential rotation (escalate to
#        SecOps — out of scope for this drill but the comm template
#        is rehearsed).

# T+32   Hard-restore protection (this is the time-boxed bypass
#        restoration — DO this even if protection is already correct,
#        to rehearse the procedure).
gh api repos/efe-ai/efe-drill-staging/branches/drill-staging/protection \
    --method PUT \
    --input - <<'EOF'
{
  "required_status_checks": null,
  "enforce_admins": true,
  "required_pull_request_reviews": {
    "required_approving_review_count": 2,
    "require_code_owner_reviews": true
  },
  "restrictions": null,
  "allow_force_pushes": false,
  "required_linear_history": true,
  "required_signatures": true
}
EOF
```

### 3.4 Phase D — Communications (T+35 to T+45)

Comms templates from the master runbook ; rehearse the cadence.

```
# T+36   Slack #release-eng update.
:white_check_mark: bench-publish defacement (drill)
- Time   : 14:22 UTC
- Cause  : maintainer push bypassed branch protection
- Sev    : P0 (live page showed unauthorised content)
- Status : rolled forward, page clean as of 14:42 UTC
- Audit  : attacker commit ab9876 retained in gh-pages history
- Comms next : customer-success digest at T+1h, exec briefing T+2h
- Post-mortem owner : <name>, deadline 2026-05-29

# T+40   Customer success briefing email.
Subject : Brief bench page defacement (resolved)

Between 14:17 UTC and 14:42 UTC, https://efe.github.io/bench/ showed
unauthorised content injected by an attacker-controlled push that
bypassed our gh-pages branch protection.  The signed quarterly evidence
PDFs and the JSON API endpoints were NOT affected.

We rolled forward (not back) via the normal publish workflow.  The
attacker commit remains visible in our gh-pages git history as part
of the audit trail ; the index.html is now restored.

Customer action : none.  Auditors holding past quarterly PDFs are
unaffected — those signatures still verify.

We will publish a full post-mortem at docs/postmortems/ by 2026-05-29.

— Release-Eng
```

### 3.5 Success criteria

- [ ] Live page clean (defacement string no longer present).
- [ ] Attacker commit retained in `git log gh-pages`.
- [ ] Branch protection re-asserted (force-push : disabled).
- [ ] All comms sent within their target windows (Slack T+5, customer
      success T+1h, exec T+2h).
- [ ] Post-mortem ticket opened (need not be filled out).
- [ ] No force-push, no manual `git reset --hard` was used.

### 3.6 Anti-patterns to call out at debrief

- **Force-pushing gh-pages to "remove" the attacker commit.**  Destroys
  the audit trail ; defeats AD30 ; gets flagged at next SOC 2 review.
- **Resetting the gh-pages branch to a prior commit.**  Same problem.
- **Disabling branch protection "to make the recovery push easier."**
  The time-boxed bypass procedure exists precisely so you don't have to.
- **Re-running the cron-trigger of bench-publish** to "auto-fix."  The
  attacker commit's content might survive if `keep_files: true` doesn't
  overwrite the specific files touched.  An explicit `workflow_dispatch`
  on `main` is the right call.
- **Engaging customer-success or legal before T+10.**  Premature
  comms set the wrong cadence ; the on-call needs the first 10 min
  uninterrupted for detection + containment.

---

## 4. Debrief (15 min)

### 4.1 Round-the-room (3 min each)

1. What was the FIRST thing I did wrong / right?
2. Did I refer to the runbook or did I freelance?
3. What command did I look up that should be in the runbook?
4. What's missing from the runbook BEFORE the next quarterly drill?

### 4.2 Action items

Examples that the 2026-Q2 scoping surfaced :

- [ ] Add a `gh api` cheatsheet to the master runbook § 4.5 — operators
      were Googling the branch-protection PUT payload.
- [ ] Add a `keep_files: true` reminder in Phase B of the defacement
      runbook — operators mistakenly thought they had to manually
      restore the files.
- [ ] Add a "DO NOT force-push" warning banner at the top of every
      defacement-related file.  The instinct is strong enough that
      explicit anti-instruction is justified.

### 4.3 Drill report

File at `docs/drills/reports/drill-YYYY-MM-DD-defacement.md` with the
same shape as the quarterly revocation drill report.

Verdict criteria :
- **pass** : containment ≤ 10 min, recovery ≤ 25 min, comms cadence
  respected, no force-push, attacker commit retained.
- **partial** : recovery > 25 min OR one comms step missed.
- **fail** : a force-push happened, OR branch protection was
  permanently disabled, OR the attacker commit was removed from git
  history.

---

## 5. Out of scope for this drill

- Real attacker (vs. fictional) — incidence response on a real attack
  goes through SecOps, not release-eng.
- Cosign signing key compromise (the attacker doesn't sign anything
  in this drill ; the existing quarterly PDFs are untouched).
- CDN cache invalidation — gh-pages is served via fastly-edge ; the
  prod runbook adds a `cdn purge` step at T+8 that the drill omits
  for tractability.
- Customer-visible legal advisory wording — Legal counsel handles
  it out-of-band ; the drill only exercises the technical chain.

---

## 6. References

- Runbook : [`docs/runbooks/bench-publish-defacement.md`](../runbooks/bench-publish-defacement.md)
- Master runbook : [`docs/runbooks/bench-publish-incident-response.md`](../runbooks/bench-publish-incident-response.md)
- PR #24 § 20 T116 + T122 (threat model)
- PR #24 AD30 (`keep_files: true` immutability)
- PR #24 AD31 (concurrency group `bench-publish`)
- PR #24 audit pass 5 D-Z (concurrent write race that motivates the
  T122 mitigation existing — without D-Z fix, the publish workflow
  itself could have produced torn files indistinguishable from low-
  effort defacement)
- Comms templates : `docs/runbooks/bench-publish-incident-response.md` § 5.2
