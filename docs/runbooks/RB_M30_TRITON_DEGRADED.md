# RB_M30_TRITON_DEGRADED — Triton Inference Server degraded

**Severity** : P2 (multimodal degraded). P1 if `efe_multimodal_triton_up == 0` for > 10 min.
**Trigger** : `M30TritonDown` or `M30MultimodalHighErrorRate` or `M30MultimodalP99HighLatency`.

## 1. Diagnose
```bash
kubectl -n efe-system get pod -l app.kubernetes.io/name=efe-triton
kubectl -n efe-system logs <triton-pod> --tail=300
kubectl -n efe-system describe pod <triton-pod>  # OOM, MIG slice
```

Also check the GPU node :
```bash
node=$(kubectl -n efe-system get pod <triton-pod> -o jsonpath='{.spec.nodeName}')
kubectl describe node $node | grep -A 5 "nvidia.com/gpu"
```

## 2. Common causes

### 2.1 Model file missing / corrupt
- Logs : `Failed to load model ...`. Re-mount the model PVC :
  ```bash
  kubectl -n efe-system rollout restart statefulset/efe-stack-triton
  ```
- If the PVC itself is corrupt, restore from the last snapshot in
  S3 : `s3://efe-models/<version>/`.

### 2.2 GPU OOM
- Multiple models loaded into the same MIG slice exceeded VRAM.
- Reduce `models.<name>.instances` in values.yaml from 2 → 1 and
  redeploy.

### 2.3 NVIDIA driver crash
- Logs : `nvml: device not found`. The kubelet auto-restarts but
  in-flight requests are 5xx. Wait 30 s ; if persistent, drain the
  node and replace.

### 2.4 MIG slice exhaustion
- Logs : `Failed to allocate GPU instance`.
- 1 × A100 with MIG 1g.10gb provides 7 slices. If > 7 Triton pods
  land on one node : HPA + cluster-autoscaler should grow the
  pool. If they don't, check the IAM role on the autoscaler.

## 3. Customer impact

| Modality   | Behavior when Triton down                                                                                  |
|------------|------------------------------------------------------------------------------------------------------------|
| Vision     | 503 with `Retry-After: 30`. Vision endpoint returns `multimodal.triton_unavail`.                            |
| Audio      | Same as vision.                                                                                            |
| Long-text  | Unaffected — chunker is CPU-only. Fusion of text-only inputs still works.                                  |
| Fusion     | Returns the text-only embedding if the multimodal inputs are missing ; otherwise 503.                       |

## 4. Emergency mitigation
1. Scale up replicas immediately :
   ```bash
   kubectl -n efe-system scale statefulset/efe-stack-triton --replicas=4
   ```
2. If GPU pool is at max : pause non-priority load via the global
   rate-limit (Cloudflare Edge → 0.1 RPS for free-tier tenants).
3. Communicate via status page — `Multimodal inference degraded`.

## 5. Post-incident
- File P2 (or P1) incident.
- If recurrent : evaluate increasing MIG profile to 2g.20gb (3
  slices per A100) for headroom.
- Audit `efe_multimodal_triton_errors_total{kind="transport"}` 7-day
  trend.
