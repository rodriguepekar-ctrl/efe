<!--
  Canonical post-mortem template — frozen under STABILITY § 26.1.
  Locked by bench/harness/tests/stability_contract_v17.rs::v1_7_section_26_1_pm_canonical_sections.
  Locked by bench/harness/tests/postmortem_schema.rs::validator_accepts_canonical_reference.

  This file is the SCHEMA, not a post-mortem.
  - The validator (scripts/postmortem-validator.sh) skips it via the
    explicit allow-list at the head of the script.
  - The deadline workflow (.github/workflows/pr26-postmortem-checks.yml)
    skips it because its filename matches `_*.md` (underscore-prefixed).
  - The reference example (docs/postmortems/_VALIDATOR_REFERENCE.md)
    is the worked illustration ; any new post-mortem author can copy
    it into a new file and replace the content.

  Section set, section order, section heading text : all FROZEN.
  Changes are STABILITY-bump PRs.
-->

---
incident_id: INC-YYYY-NNN
severity: S1
detected_at: YYYY-MM-DDTHH:MM:SSZ
mitigated_at: YYYY-MM-DDTHH:MM:SSZ
resolved_at: YYYY-MM-DDTHH:MM:SSZ
author: github-username
reviewers:
  - github-username-1
  - github-username-2
github_issue: https://github.com/efe-ai/efe/issues/NNNN
status: DRAFT
---

# Postmortem — `<incident-slug>`

> **Status** : DRAFT | REVIEW | PUBLISHED
> **Severity** : Sev1 | Sev2 | Sev3
> **Detected at (UTC)** :
> **Mitigated at (UTC)** :
> **Resolved at (UTC)** :
> **Author** :
> **Reviewers** : (≥ 2 — incident commander + adjacent team lead)
> **PagerDuty incident** : <link>
> **Linear epic** : EFE-XXX

> **New here ?** A fully-worked example post-mortem written under
> this exact schema lives at `docs/postmortems/_VALIDATOR_REFERENCE.md`.
> Copy that file into `docs/postmortems/INC-YYYY-NNN.md` and replace
> its content with your incident. Run
> `bash scripts/postmortem-validator.sh docs/postmortems/INC-YYYY-NNN.md`
> locally before opening the PR.

---

## 1. Summary (= 3-4 sentences)

What happened, in plain English. Avoid jargon. Define abbreviations.

---

## 2. Timeline (= UTC timestamps)

| Time | Event |
|------|-------|
| `HH:MM` | First signal (= alert / customer ticket / monitor) |
| `HH:MM` | Acknowledged by on-call |
| `HH:MM` | Diagnosed root cause hypothesis |
| `HH:MM` | Mitigation applied |
| `HH:MM` | All-clear confirmed |

---

## 3. Root cause

A direct, unambiguous statement. One paragraph.

If multiple causes contributed, list them in order of contribution
and cross-link the supporting evidence (logs, dashboards, code refs).

---

## 4. Contributing factors

Use this for organisational, process, or systemic factors that made
the root cause more likely to occur or more painful when it did.

- …
- …

---

## 5. What went well

Recognise specific actions that helped contain or resolve the
incident.

- …
- …

---

## 6. What went wrong

Specific gaps in detection, diagnosis, communication, mitigation, or
recovery.

- …
- …

---

## 7. Action items (= Linear-tracked)

Each item must have :

- ✅ Owner (single accountable person)
- ✅ Due date (default ≤ 30 days post-postmortem)
- ✅ Linear issue (one-to-one)
- ✅ Acceptance criteria

| Action | Owner | Linear | Due | Status |
|--------|-------|--------|-----|--------|
| … | @ | EFE-XXX | YYYY-MM-DD | TODO |

---

## 8. Prevention (= what we change permanently)

Highest-leverage controls that prevent recurrence. Often :

- New SLO + alert.
- New pin test.
- New runbook section.
- New CI gate.
- Architectural change (= ADR ref).

Document the **decision criteria** here so future reviewers can audit
whether the prevention worked.

---

## 9. Customer impact

| Tenant | Surface | Duration | Customers impacted | Communication sent |
|--------|---------|----------|--------------------|--------------------|
| | | | | |

> **Blameless** : this postmortem is **not** a performance review. Names
> appear only when crediting a specific contribution. Otherwise, refer
> to roles (on-call, release lead, etc.).

---

## 10. Compliance ramifications

If the incident touches PII, audit chain, encryption keys, or any
regulated control. Each checkbox must be either ticked or annotated
`N/A` ; an un-addressed checkbox fails the validator (exit code 7).

- [ ] DPO notified (≤ 24 h) — N/A if not applicable.
- [ ] Breach assessment under GDPR Art. 33 — N/A if not applicable.
- [ ] AI Act Art. 9 risk register updated — N/A if not applicable.
- [ ] Supervisory authority notification required ? (yes / no) — N/A if not applicable.
- [ ] Customer-facing trust center entry updated — N/A if not applicable.

---

## 11. Cross-references

- Runbook(s) consulted :
- Dashboards used :
- ADRs proposed :
- Releases involved :

---

_Template owner : SRE leadership. Update this template if the process
itself changes (e.g. new compliance step)._

_Frozen under STABILITY § 26.1 ; changes require a STABILITY-bump PR._
