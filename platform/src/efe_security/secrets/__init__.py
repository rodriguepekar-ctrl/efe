"""Secrets management : KMS client, envelope encryption, key registry,
rotation, sealer.
"""

from __future__ import annotations

from efe_security.secrets.envelope import EnvelopeManager
from efe_security.secrets.key_registry import (
    KeyAlreadyRegistered,
    KeyEntry,
    KeyPurpose,
    KeyRegistry,
    KeyState,
)
from efe_security.secrets.kms_client import (
    InMemoryKmsClient,
    KmsClient,
    VaultTransitClient,
)
from efe_security.secrets.rotation import (
    RotationOutcome,
    RotationPlan,
    RotationScheduler,
)
from efe_security.secrets.sealer import Sealer, SealedPayload

__all__ = [
    "EnvelopeManager",
    "InMemoryKmsClient",
    "KeyAlreadyRegistered",
    "KeyEntry",
    "KeyPurpose",
    "KeyRegistry",
    "KeyState",
    "KmsClient",
    "RotationOutcome",
    "RotationPlan",
    "RotationScheduler",
    "SealedPayload",
    "Sealer",
    "VaultTransitClient",
]
