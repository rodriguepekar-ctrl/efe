"""PII / PHI protection : classifier, masker, pseudonymizer, registry."""

from __future__ import annotations

from efe_security.pii.classifier import (
    Classification,
    PiiClassifier,
    PiiHit,
    PiiKind,
)
from efe_security.pii.masker import MaskMode, Masker, scrub
from efe_security.pii.policy import PiiPolicy, PiiProfile, default_policy_for
from efe_security.pii.pseudonymizer import pseudonymize, pseudonymize_hashed
from efe_security.pii.registry import (
    FieldDescriptor,
    FieldTransform,
    PiiFieldRegistry,
)

__all__ = [
    "Classification",
    "FieldDescriptor",
    "FieldTransform",
    "MaskMode",
    "Masker",
    "PiiClassifier",
    "PiiFieldRegistry",
    "PiiHit",
    "PiiKind",
    "PiiPolicy",
    "PiiProfile",
    "default_policy_for",
    "pseudonymize",
    "pseudonymize_hashed",
    "scrub",
]
