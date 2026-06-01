# Reference — `efe-tool` CLI

The SDK ships a small CLI for ad-hoc classification, catalog
inspection, and connectivity checks. Installed as a console-script
entry point when you `pip install efe-tool`.

## Sub-commands

```text
$ efe-tool --help

efe-tool — EFE Mode-1 client (v0.1.10).

Sub-commands:
    classify    Classify a text against a catalog.
    healthcheck Probe an EFE server's gRPC health endpoint.
    catalog     Inspect a catalog YAML.
    config      Show resolved env-var configuration.
```

## `classify`

```bash
efe-tool classify \
    --catalog catalogs/support_triage.yaml \
    --endpoint 127.0.0.1:50051 \
    --insecure \
    "Someone charged my card without my consent"

# Output (one JSON object on stdout):
# {"label":"fraud","confidence":0.92,"reject":false,"rounds":7}
```

## `healthcheck`

```bash
efe-tool healthcheck --endpoint 127.0.0.1:50051 --insecure
# OK
# (exits 0 on SERVING, 1 otherwise)
```

## `catalog`

```bash
efe-tool catalog inspect catalogs/support_triage.yaml

# name        : support_triage
# version     : 1.0.0
# embedder    : paraphrase-multilingual-MiniLM-L12-v2 (dim=384)
# classes     : 4 (fraud, billing, question, bug)
# anchors     : 16 total (avg 4 per class)
```

## `config`

```bash
efe-tool config --show

# EFE_ENDPOINT          : 127.0.0.1:50051   (env)
# EFE_JWT               : (unset)
# EFE_TENANT            : (unset)
# EFE_TIMEOUT_S         : 5.0               (default)
# EFE_INSECURE          : 1                 (default)
# EFE_EMBEDDER_MODEL    : sentence-transformers/all-MiniLM-L6-v2  (default)
```

## Exit codes

| Code | Meaning |
|-----:|---------|
| 0 | success |
| 1 | classification failed (gRPC error) |
| 2 | bad invocation (missing args, malformed YAML) |
| 3 | auth failure |
