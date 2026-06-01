# Drill : quarterly evidence revocation (ERRATA + RETRACTION)

**Drill ID** : `D-2026-Q2-01`
**Frequency** : annual (Q2 each year, aligned with SOC 2 audit cycle)
**Duration** : 90 minutes (60 min exercise + 30 min debrief)
**Owner** : `@efe-ai/release-eng`
**Last rehearsal** : N/A (first run)
**Next rehearsal** : 2027-04-15

This drill exercises the two end-states of the quarterly evidence
runbook ([`bench-publish-quarterly.md`](../runbooks/bench-publish-quarterly.md)) :

- **Phase A : ERRATA** — correctable error, original methodology valid.
- **Phase B : RETRACTION** — signature compromised, full retraction.

Both paths preserve the critical invariant : **published evidence is
NEVER deleted** (AD30 / AD52 — auditor chain of custody).

---

## 0. Why this drill exists

PR #24 § 12.1 + AD52 + AD53 require an operational test of the
revocation flow at least once per year.  Without rehearsal :

- The cosign keyless OIDC workflow drift would surface only at a real
  incident — when stakes are highest.
- The Phase-2 file-naming conventions (`-rev2`, `.retraction`) drift
  silently in operators' muscle memory.
- The runbook documentation drifts from the actual CLI surface (D-AP
  audit-pass caught this in test land, but a real drill catches it in
  human-procedure land).

The 2026 audit caught a **real defect** in the original quarterly
evidence build : the PDF xref table was all zeros (D-AQ, fixed in
pass 8).  Strict PDF parsers would have rejected the file at audit
time.  A drill before that audit would have caught it manually.

---

## 1. Pre-drill checklist (T-7 days)

- [ ] Schedule a 90-minute slot ; book a war-room channel.
- [ ] Identify participants : 1 on-call lead, 1 observer (writes notes),
      1 escalation buddy (legal / compliance available).
- [ ] Confirm tools available :
  - `cosign` 2.x installed
  - `gh` CLI authenticated against `efe-ai/efe`
  - `efe-bench-harness` v0.2.0+ release binary on PATH
  - Read access to the **staging** gh-pages branch (NOT prod).
- [ ] Stand up a clean staging repo as a scratch space (forks of
      `efe-ai/efe` are not allowed — use a private clone instead).
- [ ] Verify the prod state before drill :

  ```bash
  efe-bench-harness evidence verify \
      bench/v0.6.0/quarterly/Q1-2026.pdf
  # MUST exit 0.  If it doesn't, postpone the drill — fix prod first.
  ```

---

## 2. Drill scenario (T-0)

The fictional setup, briefed at minute 0 :

> Auditor `Acme-CPA` emails at 09:14 UTC : "The Q1-2026 bench evidence
> PDF you handed us last month verifies against the JSON sidecar
> sha256 you embedded, BUT the underlying scenario data for S08
> (multi-region replay) shows a recovery_time_s of 12.3 seconds —
> our other source has 0.8 seconds.  Can you confirm?
>
> Your customer-success had told us SLO 3 (recovery time) was 1 s
> max ; the 12.3 s number puts you out of compliance.  We need to
> understand the discrepancy by EOD."

The lead investigates with `grep` over the published `Q1-2026.json` :

```bash
jq '.scenarios.S08_multi_region_replay' \
    bench/v0.6.0/quarterly/Q1-2026.json
```

After 10 minutes of investigation, the lead discovers : the
loadgen profile used in Q1-2026 had a stale `--target-region`
override that targeted a single AZ instead of multi-region.  The
12.3 s figure is real for that misconfigured run, but it does NOT
represent the production multi-region topology.

**Decision tree** :

- **Path A (ERRATA)** — the methodology was wrong, the numbers were
  produced honestly with that wrong methodology.  Issue a corrected
  PDF (`Q1-2026-rev2.pdf`) with the right loadgen profile + a note
  explaining the supersession.
- **Path B (RETRACTION)** — choose this only if the original signature
  is compromised (key leak, OIDC issue).  Not the case here ; path A
  is correct.

This drill exercises BOTH paths — path A in § 3, path B in § 4.

---

## 3. Phase A — ERRATA exercise (40 min)

### 3.1 Step-by-step

```bash
# T+5    Reproduce the bad data : re-run the original loadgen profile
#        + measure recovery_time_s.  Confirm 12.3 s reproduces.
./target/release/efe-bench-harness run \
    --scenario S08_multi_region_replay \
    --target-override target_region=us-east-1 \
    --output-dir /tmp/drill-bad

# T+12   Run with the corrected profile (genuine multi-region).
./target/release/efe-bench-harness run \
    --scenario S08_multi_region_replay \
    --output-dir /tmp/drill-good
# Expect recovery_time_s ≤ 1.0 s.

# T+18   Re-generate the quarterly PDF using the corrected data.
./target/release/efe-bench-harness publish-quarterly \
    --period Q1-2026 \
    --chart-version 0.6.0 \
    --output ./bench \
    --cosign-identity "https://token.actions.githubusercontent.com"

# T+22   Phase-2 stub : the publisher emits Q1-2026.pdf — rename to
#        Q1-2026-rev2 BEFORE committing (per quarterly runbook § Phase 1).
mv ./bench/v0.6.0/quarterly/Q1-2026.pdf ./bench/v0.6.0/quarterly/Q1-2026-rev2.pdf
mv ./bench/v0.6.0/quarterly/Q1-2026.json ./bench/v0.6.0/quarterly/Q1-2026-rev2.json
mv ./bench/v0.6.0/quarterly/Q1-2026.pdf.bundle \
   ./bench/v0.6.0/quarterly/Q1-2026-rev2.pdf.bundle

# T+25   Verify the new PDF.
./target/release/efe-bench-harness evidence verify \
    ./bench/v0.6.0/quarterly/Q1-2026-rev2.pdf
# MUST exit 0.

# T+27   Append the ERRATUM entry to /api/history.json.
#        Use the AD52 schema (note the `replaces` field).
cat <<EOF >> /tmp/erratum-entry.json
{
  "date": "Q1-2026-rev2",
  "url": "/quarterly/Q1-2026-rev2.pdf",
  "sha256": "$(sha256sum ./bench/v0.6.0/quarterly/Q1-2026-rev2.json | awk '{print $1}')",
  "overall_verdict": "pass",
  "replaces": "Q1-2026"
}
EOF

# T+33   Open the PR.  The original Q1-2026 files STAY in the repo —
#        gh-pages is append-only.
git checkout -b bench-quarterly-Q1-2026-rev2
git add bench/v0.6.0/quarterly/Q1-2026-rev2.*
git commit -m "chore(bench): Q1-2026 erratum — fix S08 multi-region profile"
gh pr create --base main --title "[bench] Erratum Q1-2026-rev2 (S08 fix)" \
    --body "Erratum for Q1-2026 evidence.  Replaces $(git log -1 --format=%H bench/v0.6.0/quarterly/Q1-2026.pdf).  Auditor : @acme-cpa"

# T+40   Verify auditor can reach BOTH the old and the new PDF.
curl -fsSI https://efe.github.io/bench/v0.6.0/quarterly/Q1-2026.pdf       # 200
curl -fsSI https://efe.github.io/bench/v0.6.0/quarterly/Q1-2026-rev2.pdf  # 200
```

### 3.2 Success criteria

- [ ] Both `Q1-2026.pdf` AND `Q1-2026-rev2.pdf` reachable on gh-pages.
- [ ] `evidence verify Q1-2026-rev2.pdf` exits 0.
- [ ] `history.json` carries the erratum entry with `replaces: "Q1-2026"`.
- [ ] No content was deleted from gh-pages (`git diff` shows only adds).
- [ ] Customer-success can email the auditor a link to `Q1-2026-rev2.pdf`.

### 3.3 Common failures to surface in debrief

- Forgetting to rename ALL THREE files (`.pdf`, `.json`, `.pdf.bundle`).
- Forgetting the `replaces` field in `history.json`.
- Using `--cosign-disabled` in the drill — would produce an unsigned
  rev2 PDF (production must NEVER use this flag).
- Force-pushing the gh-pages branch (defeats AD30 immutability).

---

## 4. Phase B — RETRACTION exercise (30 min)

Run **after** Phase A, with a fresh fictional trigger :

> Security team alert : the GitHub Actions OIDC token issuer
> certificate rotation between Q1-2026 sign time and now caused
> an inadvertent re-use of an OIDC trust assumption that the security
> team now considers invalid.  All Q1-2026 evidence signatures must
> be marked as retracted, even though the numbers themselves stand.

### 4.1 Step-by-step

```bash
# T+0    Generate the retraction notice.  ASCII-only (D-AL fix : PDF
#        can't render Unicode under WinAnsi).
cat <<EOF > /tmp/retraction-Q1-2026.txt
RETRACTION NOTICE -- Q1-2026 bench evidence
Issued by : @efe-ai/release-eng
Reason    : OIDC trust-assumption invalidation (security incident
            S-2026-04-22 -- see internal advisory).
Date      : $(date -u -Iseconds)

Affected PDF : https://efe.github.io/bench/v0.6.0/quarterly/Q1-2026.pdf
Sha256       : $(sha256sum bench/v0.6.0/quarterly/Q1-2026.json | awk '{print $1}')

Replacement  : Q1-2026-rev2.pdf (issued $(date -u +%Y-%m-%d) via ERRATA
               flow ; signature on rev2 uses the post-rotation OIDC
               trust assumption).
Customer action :
  - If you held a copy of Q1-2026.pdf : treat its signature as advisory.
  - Cross-reference Q1-2026-rev2.pdf for the authoritative numbers.
EOF

# T+10   Sign the retraction notice with the SAME cosign keyless OIDC
#        identity used for original evidence (shared regex § 24.8).
cosign sign-blob --yes \
    --bundle /tmp/retraction-Q1-2026.txt.bundle \
    /tmp/retraction-Q1-2026.txt

# T+15   Upload to gh-pages under quarterly/Q1-2026.retraction.{txt,txt.bundle}.
cp /tmp/retraction-Q1-2026.txt        ./bench/v0.6.0/quarterly/Q1-2026.retraction.txt
cp /tmp/retraction-Q1-2026.txt.bundle ./bench/v0.6.0/quarterly/Q1-2026.retraction.txt.bundle

# T+18   Open the PR.
git checkout -b bench-retract-Q1-2026
git add bench/v0.6.0/quarterly/Q1-2026.retraction.*
git commit -m "chore(bench): Q1-2026 retraction notice (OIDC rotation)"
gh pr create --base main --title "[bench] Retraction Q1-2026 (OIDC rotation)" \
    --body "Retraction notice for Q1-2026.  Replacement at Q1-2026-rev2.pdf."

# T+25   Notify auditors via the comms template in
#        bench-publish-incident-response.md § 5.2.
```

### 4.2 Success criteria

- [ ] `Q1-2026.retraction.txt` reachable on gh-pages.
- [ ] `cosign verify-blob --bundle Q1-2026.retraction.txt.bundle Q1-2026.retraction.txt`
      succeeds against the shared identity regex.
- [ ] Customer-success email sent with both the retraction link and
      the rev2 link.
- [ ] **Original `Q1-2026.pdf` is STILL on gh-pages** — retraction
      annotates, does not delete.

### 4.3 Common failures to surface in debrief

- Operator instinct says "delete the bad PDF" — must be resisted.
- Operator uses a NEW cosign identity instead of the shared one — would
  fail the `--certificate-identity-regexp` check at verify time.
- Operator forgets to sign the retraction notice — auditors have no
  trust anchor for the retraction itself.
- Operator writes the retraction in markdown (`§`, em-dashes) — these
  are non-ASCII and would mojibake if the retraction ever ends up
  embedded in a PDF.  Pure ASCII is the rule (D-AL).

---

## 5. Debrief (30 min, in war room channel)

### 5.1 Round-the-room (5 min each participant)

Each participant answers, in order :
1. What surprised me during this drill?
2. What command did I look up that I should have memorised?
3. What part of the runbook felt wrong / outdated / missing?
4. What's the ONE thing I'd add to the runbook before the next drill?

### 5.2 Action items (10 min)

Write each as `[ ]` checklist with an owner + deadline.  Examples
that came up in scoping this drill template :

- [ ] Add `--no-pager` to all `git log` commands in the runbook ; operators
      were getting stuck in less.
- [ ] Replace the `printf '%s' ...` SHA computation hint with
      `sha256sum < file` — the printf form was tripping people up.
- [ ] Confirm that `gh pr create` with `--body` accepting multi-line
      strings works in zsh ; one drill participant got escaping issues.
- [ ] Add an explicit sentence in § 4 saying "DO NOT delete the original
      PDF".  Three operators in three drills proposed deleting it.

### 5.3 Drill report (filed within 48 h)

File at `docs/drills/reports/drill-YYYY-MM-DD-quarterly.md` :

```markdown
# Drill report : quarterly revocation

- Date : YYYY-MM-DD UTC
- Participants : <names>
- Duration : actual minutes (vs target 90)
- Verdict : pass / partial / fail
- Issues found : <count>
- Action items opened : <count, links>
- Runbook PR : <link, if any>
```

Verdict criteria :
- **pass** : both phases completed, all success-criteria checkboxes
  hit, no untouched runbook section.
- **partial** : one phase completed clean ; the other had ≥ 1 failed
  success criterion that was non-blocking (i.e. resolved at debrief).
- **fail** : an irrecoverable mistake (force-pushed gh-pages, deleted
  an original PDF, signed with wrong identity).  Triggers a re-drill
  within 30 days.

---

## 6. Out of scope for this drill

- Cosign infrastructure outage (sigstore.dev down) — separate drill
  candidate, currently exercised only via tabletop.
- Rekor transparency-log outage — Phase-3 G.12 will add an offline
  inclusion-proof verifier, then this drill should be extended.
- gh-pages branch protection bypass — see [`drill-defacement-response.md`](drill-defacement-response.md).
- Customer-facing legal review of the retraction wording — handled by
  Legal counsel out-of-band ; this drill only validates the technical chain.

---

## 7. References

- Runbook : [`docs/runbooks/bench-publish-quarterly.md`](../runbooks/bench-publish-quarterly.md)
- STABILITY_CONTRACT v1.6 § 24.4 (quarterly evidence path), § 24.5
  (CLI surface), § 24.8 (cosign identity regex)
- PR #24 AD52 (write-once + chain-pointer `replaces`)
- PR #24 AD53 (RETRACTION flow)
- PR #22 § 13 (parallel SLO chain — same flow applies)
- Audit pass 10 D-BE (cross-chain marker fix that this drill implicitly tests)
- Audit pass 8 D-AQ (PDF xref correctness — without the fix this
  drill would have produced auditor-rejected files)
