# EFE Operator Guide

> **Status** : Sprint M14 D42 audit fix M14-A2 — operator orientation guide.
> **Audience** : New operators / SRE / DevOps onboarding to EFE.
> **Last updated** : 2026-04-30.

---

## 1. Welcome

Welcome to the EFE operator role. This guide orients you to the
EFE 1.0 GA platform.

## 2. Day 1 reading list

Read in this order :
1. `docs/RELEASE_NOTES_1.0.md` — what shipped
2. `docs/HANDOFF_OPS.md` — your responsibilities
3. `docs/security/threat_model.md` — what we defend against
4. `docs/runbooks/general_incident_playbook.md` — incident response baseline
5. 5 sprint-specific runbooks (`d13_*` first, then sprint relevant)

## 3. Architecture orientation

Read in this order :
1. `docs/EFE_SERVER_ARCHITECTURE.md` — core gRPC server
2. `docs/EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md` (M12) — deployment
3. `docs/EFE_BUNDLES_EXTENSION_ARCHITECTURE.md` (M13) — model bundles
4. `docs/EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md` (M14) — compliance

## 4. Tooling setup

```bash
# Required CLIs
brew install kubectl helm jq yq oras cosign cyclonedx-cli aws-cli
brew install --cask google-cloud-sdk azure-cli  # if multi-cloud

# K8s context
kubectl config use-context efe-production-eu-west

# Helm OCI auth (GHCR)
echo "${GH_PAT}" | helm registry login ghcr.io --username "${GH_USER}" --password-stdin
```

## 5. First-week shadowing

- Pair with current on-call
- Read 5 runbooks per day in detail
- Run staging deploy + helm test
- Run a DR drill in dev

## 6. Customer support orientation

Read :
- Tier 1-4 SLA matrix in RELEASE_NOTES_1.0
- Escalation tree in HANDOFF_OPS §3.4
- Customer DDQ pre-fill `docs/compliance/ddq_prefill.md`

## 7. Compliance orientation

Read :
- `docs/compliance/iso27001.md` (90 controls)
- `docs/compliance/soc2_evidence.md` (38 criteria)
- `docs/compliance/gdpr_art32.md` (4 mesures)
- `docs/compliance/ai_act_annex_iv.md` (8 sections)

## 8. On-call onboarding

Per HANDOFF_OPS §3 :
- Week 1 : shadow primary
- Month 1 : solo P3
- Month 3 : primary rotation

## 9. Channels

- Slack `#eng-incidents` — production alerts
- Slack `#security` — security topics
- Slack `#compliance` — compliance topics
- PagerDuty / Opsgenie — paging

## 10. Help

- Internal Slack `#ops-help`
- email : ops@efe.ai
- Mentor assignment per HANDOFF_OPS

---

## Appendix : keyboard shortcuts for common ops

```bash
# kubectl shortcuts
alias kp='kubectl get pods -n efe-production'
alias kj='kubectl get jobs -n efe-production'
alias kl='kubectl logs -n efe-production'
alias kex='kubectl exec -n efe-production -it'

# Helm shortcuts
alias hh='helm history efe -n efe-production'
alias ht='helm test efe -n efe-production --timeout 5m'
alias hr='helm rollback efe'

# AWS shortcuts (with assume-role)
alias awsl='aws s3 ls s3://efe-evidence-prod/ --recursive | tail -20'
```
