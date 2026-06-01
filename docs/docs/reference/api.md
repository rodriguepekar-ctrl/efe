# API Reference

> Auto-generated from the SDK docstrings via
> [mkdocstrings](https://mkdocstrings.github.io/). Updated on every
> ``mkdocs build``. Coverage pinned by
> ``scripts/tests/test_d10_api_reference_coverage.py``.

---

## efe_tool

::: efe_tool
    options:
      show_root_heading: true
      show_root_full_path: false
      show_submodules: false
      members:
        - __version__
        - server_compatible_version
        - classify

---

## efe_tool.client

::: efe_tool.client
    options:
      show_root_heading: true
      members:
        - EfeClient

---

## efe_tool catalog

::: efe_tool.catalog
    options:
      show_root_heading: true
      members:
        - AnchorCatalog
        - ClassDef
        - ThresholdPolicy

---

## efe_tool.embedder

::: efe_tool.embedder
    options:
      show_root_heading: true
      members:
        - Embedder
        - LocalSTEmbedder

---

## efe_tool.types

::: efe_tool.types
    options:
      show_root_heading: true
      members:
        - ClassifyResult
        - SubScore

---

## efe_tool.errors

::: efe_tool.errors
    options:
      show_root_heading: true
      members:
        - EfeError
        - EfeAuthError
        - EfeRateLimitError
        - EfeInvalidArgumentError
        - EfeNotFoundError
        - EfeUnavailableError
        - EfeInternalError
        - raise_from_grpc
        - BridgeError
        - ConfigError
        - CatalogError
        - EmbedderError
        - NetworkError
        - LowConfidenceWarning

---

## efe_tool.integrations.hybrid_decoder

::: efe_tool.integrations.hybrid_decoder
    options:
      show_root_heading: true

---

## efe_tool — async (skeleton in v0.1.x)

::: efe_tool._async.client
    options:
      show_root_heading: true
      members:
        - AsyncEfeClient

---

## efe_tool.admin — multi-tenant admin (Sprint D11 + D12)

::: efe_tool.admin
    options:
      show_root_heading: true
      members:
        - EfeAdminClient
        - CatalogRef
        - AdminError
        - EvolveOp
        - RegisteredCatalog
        - CatalogAuditMetadata
        - CompiledClassRef
