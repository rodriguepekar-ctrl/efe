"""Consent records + DSAR pipeline."""

from __future__ import annotations

from efe_security.consent.dsar import (
    DsarKind,
    DsarRequest,
    DsarService,
    DsarState,
)
from efe_security.consent.manager import ConsentManager
from efe_security.consent.purpose import (
    Purpose,
    PurposeCatalog,
    default_purpose_catalog,
)
from efe_security.consent.records import (
    ConsentAssertion,
    ConsentRecord,
    ConsentState,
    LegalBasis,
)

__all__ = [
    "ConsentAssertion",
    "ConsentManager",
    "ConsentRecord",
    "ConsentState",
    "DsarKind",
    "DsarRequest",
    "DsarService",
    "DsarState",
    "LegalBasis",
    "Purpose",
    "PurposeCatalog",
    "default_purpose_catalog",
]
