# Runbook — GPU Driver Upgrade

> **Cadence** : 1×/trimestre (= aligné CUDA Toolkit release).
> **Sévérité** : info (= ops planifiée) ou critical (= patch sécu CVE).
> **Trigger** : alerte `GPU_DRIVER_DRIFT` quand drift > 1 h.
> **Sprint d'origine** : Φ-D D31.
> **Référence architecture** : `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md`
> § 5.1 + § 14.

---

## TL;DR

Upgrader le driver NVIDIA (e.g. 535 → 545) sur le fleet sans casser
l'opposabilité juridique SLO-C-1. La procédure rolling exige :

1. Pre-validation : nouveau driver compile + tests numerical equiv PASS.
2. Update du `host_fingerprint_baseline` (= signature attestée).
3. Rolling restart fleet 25 %/25 %/25 %/25 %.
4. Monitoring `GPU_DRIVER_DRIFT` alert : doit fire pendant la
   rotation, doit revenir vert dans 1 h.

---

## 1. Pré-flight (T = -7 j)

### 1.1 Décision Go/No-Go

Question avant chaque upgrade :
- [ ] Le driver target supporte-t-il toujours CUDA Runtime 12.4 ?
- [ ] NCCL est-il compatible (= check release notes NCCL) ?
- [ ] cudarc 0.10.x supporte-t-il le nouveau driver ? (= test cross-version)
- [ ] Y a-t-il un CVE motivant l'upgrade ? (= cf. NVIDIA security advisories)

### 1.2 Test sur staging (= 1 GPU dédié)

```bash
# Sur la machine staging avec nouveau driver pré-installé :
$ cargo test --package efe-kernels --features cuda --release
$ cargo test --package efe-kernels --features cuda --test numerical_equivalence --release
$ cargo bench --package efe-kernels --features cuda --bench '*_bench' --no-fail-fast
```

Acceptance : tous tests PASS + speedup ≥ 95 % du baseline ancien
driver (= pas de régression perf).

### 1.3 Compute le nouveau fingerprint

```bash
$ ./target/release/efe-server fingerprint > /tmp/new-fingerprint.json
$ cat /tmp/new-fingerprint.json | jq .gpu
{
  "driver_version": "545.23.06",
  "cuda_runtime_version": "12.4",
  ...
}
$ ./target/release/efe-attest sign-baseline /tmp/new-fingerprint.json
```

---

## 2. Communication client (T = -5 j → T = 0)

Webhook `BUNDLE_FINGERPRINT_ROTATION_SCHEDULED` envoyé aux tenants
Pro+ avec :

```json
{
  "event": "GPU_DRIVER_ROTATION_SCHEDULED",
  "current_driver": "535.104.05",
  "target_driver": "545.23.06",
  "scheduled_at": "2026-08-01T04:00:00Z",
  "opposability_suspended_during_h": 1,
  "action_required": "none",
  "expected_perf_impact": "negligible (= < 1 %)"
}
```

---

## 3. Rolling upgrade (T = 0 → +60 min)

### 3.1 Cordon + drain 25 % du fleet

```bash
# Étape 1 : cordon les 2/8 premiers nodes GPU
$ for n in node-gpu-{1,2}; do
    kubectl cordon $n
    kubectl drain $n --ignore-daemonsets --delete-emptydir-data --force
  done

# Étape 2 : upgrade driver sur ces nodes via Ansible / SSM
$ ansible-playbook -i inventory upgrade-gpu-driver.yml --limit node-gpu-{1,2}

# Étape 3 : uncordon + monitor
$ for n in node-gpu-{1,2}; do kubectl uncordon $n; done
```

### 3.2 Validation avant prochain batch

Sur les 2 replicas upgradés :
- [ ] `efe_gpu_loaded_at_boot = 1`
- [ ] `efe_gpu_driver_version_info{driver="545.23.06"} = 1`
- [ ] Pas d'alerte `GPU_KERNEL_FAILURE_BURST`
- [ ] p99 latence ≤ baseline + 5 %

Si OK → passer aux 2 next nodes (= 25 % suivants), etc.

### 3.3 Watchdog `GPU_DRIVER_DRIFT`

Pendant les ~60 min du rolling upgrade, l'alerte
`GPU_DRIVER_DRIFT` fire (= attendu). On note dans le journal :

```
2026-08-01 04:05 UTC — GPU_DRIVER_DRIFT firing (expected during rolling upgrade)
2026-08-01 04:55 UTC — GPU_DRIVER_DRIFT resolved (= fleet uniform)
```

Si l'alerte ne resolve pas après 60 min → escalade.

---

## 4. Post-upgrade validation (T = +1 h → +24 h)

### 4.1 Sanity checks

- [ ] `count(count by (driver) (efe_gpu_driver_version_info)) == 1`
      (= un seul driver actif sur le fleet)
- [ ] p99 latence Mode-1 inchangé (= cf. dashboard P1)
- [ ] Aucun client ticket avec `category=gpu_performance`
- [ ] SLO-CUDA-FUSION-ROUND reste PASS

### 4.2 Re-attestation tiers (= SOC2 evidence)

```bash
$ ./target/release/efe-attest verify-fingerprint \
    --baseline bench_baselines/v0.6.0/p1_slos.yaml \
    --fingerprint /tmp/new-fingerprint.json
```

Insert row dans `bundle_release_attestations` avec
`attestor_kind=internal_security`.

---

## 5. Rollback (en cas d'échec)

Si le nouveau driver cause des régressions :

```bash
# 1. Bumper trust store pour blacklist new driver SHA
# 2. Helm chart bump avec image embedding old driver baseline
$ kubectl set image deployment/efe-server -n efe \
    efe-server=registry.efe.ai/efe-server:v0.5.x-baseline
```

Fleet revient à l'ancien driver dans ~ 30 min.

---

## 6. Escalade

| Durée incident | Décision |
|----------------|----------|
| < 30 min | SRE on-call gère seul |
| 30 min - 4 h | Engager tech lead + NVIDIA support si needed |
| > 4 h | Rollback + post-mortem |

---

## 7. Liens utiles

- Architecture : `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` § 5.1
- NVIDIA security advisories : https://www.nvidia.com/en-us/security/
- Runbook OOM recovery : `docs/runbook/gpu_oom_recovery.md`
- Runbook NCCL failover : `docs/runbook/multi_gpu_failover.md`
- Status page : https://status.efe.ai
