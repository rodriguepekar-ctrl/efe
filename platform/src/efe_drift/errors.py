"""Exception hierarchy for :mod:`efe_drift`.

All exceptions descend from a single :class:`DriftError` root so one
``except`` clause traps the whole module. Every subclass carries :

* a stable ``code`` for audit logs + metric labels ;
* an ``http_status`` default that pins the REST mapping ;
* a ``retryable_hint`` — best-guess for clients / daemons.

Categories (in order of appearance) :

    Config                — configuration / startup errors
    Catalog               — feature_catalog errors
    Extraction            — feature extractor failures
    Ingestion             — NATS / HTTP ingestion path
    Aggregation           — rollup / histogram failures
    Baseline FSM          — invalid state transitions
    Baseline integrity    — signature, seal, sanity
    PII                   — PII linter detections
    NATS                  — eventing integrity
"""

from __future__ import annotations

from typing import Any


class DriftError(Exception):
    """Root of the :mod:`efe_drift` error hierarchy."""

    code: str = "drift_error"
    retryable_hint: bool = False
    http_status: int = 500

    def __init__(self, message: str, *, detail: Any = None) -> None:
        super().__init__(message)
        self.message = message
        self.detail = detail

    def to_dict(self) -> dict[str, Any]:
        out: dict[str, Any] = {
            "code": self.code,
            "message": self.message,
            "retryable_hint": self.retryable_hint,
        }
        if self.detail is not None:
            out["detail"] = self.detail
        return out

    def __repr__(self) -> str:
        return (
            f"{type(self).__name__}(code={self.code!r}, "
            f"message={self.message!r})"
        )


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------


class ConfigError(DriftError):
    code = "config_error"
    http_status = 500


class ValidationError(DriftError):
    code = "validation_error"
    http_status = 400


class NotFound(DriftError):
    code = "not_found"
    http_status = 404


class TenantMismatch(DriftError):
    code = "tenant_mismatch"
    http_status = 403


# ---------------------------------------------------------------------------
# Catalog
# ---------------------------------------------------------------------------


class CatalogError(DriftError):
    code = "catalog_error"
    http_status = 409


class FeatureNotFound(CatalogError):
    code = "feature_not_found"
    http_status = 404


class DuplicateFeature(CatalogError):
    code = "duplicate_feature"
    http_status = 409


class UnknownExtractor(CatalogError):
    """The declared extractor name is not registered."""

    code = "unknown_extractor"
    http_status = 400


class InvalidBinningStrategy(CatalogError):
    code = "invalid_binning_strategy"
    http_status = 400


# ---------------------------------------------------------------------------
# Extraction
# ---------------------------------------------------------------------------


class ExtractionError(DriftError):
    code = "extraction_error"
    http_status = 500


class ExtractorMissing(ExtractionError):
    code = "extractor_missing"
    http_status = 500


class ExtractorTypeMismatch(ExtractionError):
    """Extractor returned a FeatureValue of the wrong kind."""

    code = "extractor_type_mismatch"
    http_status = 500


# ---------------------------------------------------------------------------
# Ingestion
# ---------------------------------------------------------------------------


class IngestionError(DriftError):
    code = "ingestion_error"
    http_status = 500


class BufferFull(IngestionError):
    """The in-memory event buffer rejected the write."""

    code = "buffer_full"
    http_status = 503
    retryable_hint = True


class BackpressureExceeded(IngestionError):
    code = "backpressure_exceeded"
    http_status = 503
    retryable_hint = True


# ---------------------------------------------------------------------------
# Aggregation
# ---------------------------------------------------------------------------


class AggregationError(DriftError):
    code = "aggregation_error"
    http_status = 500


class HistogramBinMismatch(AggregationError):
    """Bin-edge length does not match expected."""

    code = "histogram_bin_mismatch"
    http_status = 500


class EmptySampleSet(AggregationError):
    code = "empty_sample_set"
    http_status = 409


# ---------------------------------------------------------------------------
# Baseline FSM
# ---------------------------------------------------------------------------


class BaselineError(DriftError):
    code = "baseline_error"
    http_status = 409


class BaselineNotFound(BaselineError):
    code = "baseline_not_found"
    http_status = 404


class InvalidBaselineTransition(BaselineError):
    code = "invalid_baseline_transition"
    http_status = 409


class BaselineImmutable(BaselineError):
    """Cannot mutate an ACTIVE baseline (invariant I4)."""

    code = "baseline_immutable"
    http_status = 409


class BaselineStateVersionConflict(BaselineError):
    """Optimistic-lock (CAS) failure on ``state_version``."""

    code = "baseline_state_version_conflict"
    http_status = 409
    retryable_hint = True


# ---------------------------------------------------------------------------
# Baseline integrity
# ---------------------------------------------------------------------------


class BaselineIntegrityError(BaselineError):
    code = "baseline_integrity_error"
    http_status = 409


class BaselineSanityCheckFailed(BaselineIntegrityError):
    code = "baseline_sanity_failed"
    http_status = 409


class BaselineSignatureInvalid(BaselineIntegrityError):
    code = "baseline_signature_invalid"
    http_status = 409


class BaselineSpecHashMismatch(BaselineIntegrityError):
    code = "baseline_spec_hash_mismatch"
    http_status = 409


class NoActiveBaseline(BaselineIntegrityError):
    """No active baseline exists for the requested feature."""

    code = "no_active_baseline"
    http_status = 409


# ---------------------------------------------------------------------------
# PII
# ---------------------------------------------------------------------------


class PiiError(DriftError):
    code = "pii_error"
    http_status = 409


class PiiDetected(PiiError):
    """The linter detected PII-like content in a histogram or snapshot."""

    code = "pii_detected"
    http_status = 409


# ---------------------------------------------------------------------------
# NATS
# ---------------------------------------------------------------------------


class NatsError(DriftError):
    code = "nats_error"
    http_status = 500
    retryable_hint = True


class NatsMessageIntegrity(NatsError):
    code = "nats_message_integrity"
    http_status = 401


class NatsEnvelopeStale(NatsError):
    """Envelope timestamp outside the skew window."""

    code = "nats_envelope_stale"
    http_status = 401


__all__ = [
    "AggregationError",
    "BackpressureExceeded",
    "BaselineError",
    "BaselineImmutable",
    "BaselineIntegrityError",
    "BaselineNotFound",
    "BaselineSanityCheckFailed",
    "BaselineSignatureInvalid",
    "BaselineSpecHashMismatch",
    "BaselineStateVersionConflict",
    "BufferFull",
    "CatalogError",
    "ConfigError",
    "DriftError",
    "DuplicateFeature",
    "EmptySampleSet",
    "ExtractionError",
    "ExtractorMissing",
    "ExtractorTypeMismatch",
    "FeatureNotFound",
    "HistogramBinMismatch",
    "IngestionError",
    "InvalidBaselineTransition",
    "InvalidBinningStrategy",
    "NatsEnvelopeStale",
    "NatsError",
    "NatsMessageIntegrity",
    "NoActiveBaseline",
    "NotFound",
    "PiiDetected",
    "PiiError",
    "TenantMismatch",
    "UnknownExtractor",
    "ValidationError",
]
