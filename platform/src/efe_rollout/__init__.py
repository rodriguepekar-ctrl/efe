"""EFE Rollout & A/B Testing (M08).

High-level surface for application code and ops consoles :

* :class:`RolloutClient`      — top-level SDK (assign, record, analyze).
* :class:`RolloutSettings`    — configuration bundle.
* :class:`ExperimentRegistry` — lifecycle + pre-registration.
* :class:`ExperimentSpec`, :class:`Variant`, :class:`GuardrailSpec` —
  value objects.

Typical usage from production code::

    from efe_rollout import RolloutClient, RolloutSettings

    settings = RolloutSettings.from_env()
    client = RolloutClient.in_memory(settings=settings)   # dev
    variant = await client.get_variant(
        user_id="u-123",
        experiment_id="exp-summarize-v2",
    )
    if variant.variant_id is None:
        # holdout or outside-traffic — use the reference model
        ...

The package ships with submodules for every layer of the stack
(``assignment``, ``analysis``, ``bandits``, ``guardrails``,
``metrics``, ``layers``, ``nats``, ``db``, ``api``). Import from the
public façade unless you are extending the module itself.
"""

from __future__ import annotations

from efe_rollout.adapter import RolloutClient, VariantResolution
from efe_rollout.config import (
    AnalysisSettings,
    ApiSettings,
    AssignmentSettings,
    BanditSettings,
    DatabaseSettings,
    GuardrailSettings,
    IngestSettings,
    LayerSaltSpec,
    LayerSettings,
    NatsSettings,
    ObservabilitySettings,
    PreRegistrationSettings,
    RedisSettings,
    RolloutSettings,
)
from efe_rollout.experiment import (
    Experiment,
    ExperimentFsm,
    ExperimentKind,
    ExperimentSpec,
    ExperimentState,
    GuardrailSpec,
    StoppingRule,
    Variant,
)
from efe_rollout.prereg import PreRegistration
from efe_rollout.records import AnalysisResult, Exposure, Outcome
from efe_rollout.registry import (
    ExperimentRegistry,
    InMemoryRegistryStore,
    RegistryStore,
)
from efe_rollout.version import __version__

__all__ = [
    "AnalysisResult",
    "AnalysisSettings",
    "ApiSettings",
    "AssignmentSettings",
    "BanditSettings",
    "DatabaseSettings",
    "Experiment",
    "ExperimentFsm",
    "ExperimentKind",
    "ExperimentRegistry",
    "ExperimentSpec",
    "ExperimentState",
    "Exposure",
    "GuardrailSettings",
    "GuardrailSpec",
    "InMemoryRegistryStore",
    "IngestSettings",
    "LayerSaltSpec",
    "LayerSettings",
    "NatsSettings",
    "ObservabilitySettings",
    "Outcome",
    "PreRegistration",
    "PreRegistrationSettings",
    "RedisSettings",
    "RegistryStore",
    "RolloutClient",
    "RolloutSettings",
    "StoppingRule",
    "Variant",
    "VariantResolution",
    "__version__",
]
