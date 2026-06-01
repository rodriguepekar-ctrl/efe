# RB_M33_NER_DOWN — NER extractor sidecar unhealthy

**Severity** : P2 (extraction unavailable). P3 if reads still work.
**Trigger** : `M33NerExtractorDown`.

## 1. Diagnose
```bash
kubectl -n efe-system get pod -l app.kubernetes.io/name=efe-kg,component=ner
kubectl -n efe-system logs <pod> --tail=200
```

## 2. Common causes

### 2.1 Model artifact missing
- The init container failed to download `efe-ner-bert-multilingual`.
- Mitigation : check S3 bundle URL + IRSA permissions.

### 2.2 OOM
- BERT-base + tokenizer + spaCy ~ 2 GB. Default limit 4 GB.
- Mitigation : scale up pod memory limit OR enable model quantization.

## 3. Customer impact
- `POST /v1/kg/extract` returns 502.
- Existing graph queries (`GET /entities`, `POST /traverse`,
  `POST /graphrag/search`) keep working — NER is only on the
  ingest path.

## 4. Mitigation
1. Restart the deployment.
2. If sustained : the runtime can fall-back to a no-op NER (returns
   zero spans). Tenants must re-extract once the sidecar is back.
