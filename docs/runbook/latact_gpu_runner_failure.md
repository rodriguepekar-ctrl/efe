# Runbook — GPU runner failure (P1)

> **Doctrine** : D-LATACT-1 (= aucun PENDING > 90 jours) + D-LATACT-2 (= activation chirurgicale).
> **Symptôme** : `latact-bench.yml` job `tail-100k` ou `p1-gpu-bench.yml` échoue ; `SLO-C-1` retombe PENDING.

## Détection

* Alert PagerDuty : `efe_workflow_failures_total{workflow="latact-bench"} > 0`.
* GitHub Actions UI : un job `gpu-*` reste « queued > 30 min » → runner indisponible.
* `nvidia-smi` retourne erreur sur le runner (= driver crash, ECC error).

## Causes probables

1. **Spot termination AWS** : EC2 g4dn.xlarge récupéré par AWS (= spot interruption notice 2 min).
2. **Quota limit** : compte AWS atteint le quota g4dn dans la région.
3. **Driver crash** : CUDA 12.4 driver instable après update OS.
4. **Pool 0 runners** : tous les 3 runners terminés simultanément.
5. **GitHub Actions runner deregistered** : token expiré.

## Triage rapide

```bash
# 1. État pool runners.
aws ec2 describe-spot-instance-requests --filters Name=tag:Sprint,Values=Phi-D-LATACT

# 2. État GitHub registrations.
gh api /orgs/efe-ai/actions/runners | jq '.runners[] | {name, status, busy}'

# 3. Logs côté runner (si SSH possible).
ssh ec2-user@<runner-ip> "sudo journalctl -u 'actions.runner.*' --since '30 min ago'"
```

## Mitigation immédiate

1. **Spot termination** : Terraform recrée automatiquement (= persistent spot request).
2. **Quota** : `aws service-quotas request-service-quota-increase --service-code ec2 --quota-code L-3819A6DF`.
3. **Driver crash** : `sudo reboot` + verify `nvidia-smi`.
4. **All runners down** : `terraform apply -var runner_count=3` pour reprovisionner.
5. **Token expired** : `terraform apply -var github_token=<new>`.

## Mitigation durable

* Pool minimum 3 runners (= survives 2 spot terminations simultanées).
* Health-check Prometheus toutes 60s sur chaque runner.
* Alert quotidienne sur quota vs usage.

## Cross-references

* SLO : SLO-C-1, SLO-P1-BUDGET-5, SLO-P1-BUDGET-7, SLO-LATACT-TAIL-P9999
* Bench : `bench_p1_gpu_sustained.rs` (= TODO production)
* Infra : `infra/terraform/gpu-runner/main.tf`
* DT : DT-LATACT-GPU-CAPACITY-PROD
