"""M12 absorption layer — concrete adapters to M1-M11.

This package is the SOLE production code that imports from the other
platform modules. It demonstrates that M12 fully absorbs M1-M11 :

* :mod:`efe_platform` (M01) → :mod:`.adapters_m01`
* :mod:`efe_registry` (M03) → :mod:`.adapters_m03`
* :mod:`efe_ft`      (M05) → :mod:`.adapters_m03_m05`
* :mod:`efe_inference` (M06) → :mod:`.adapters_m06`
* :mod:`efe_rollout` (M07) → :mod:`.adapters_m07`
* :mod:`efe_drift`   (M09) → :mod:`.adapters_m09`
* :mod:`efe_scheduling` (M10) → :mod:`.adapters_m10`
* :mod:`efe_security` (M11) → :mod:`.adapters_m11`
"""

from __future__ import annotations

from efe_swarm_integration.integration.absorption import (
    PlatformWiring,
    wire_up_platform,
)
from efe_swarm_integration.integration.adapters_m01 import (
    M01PlatformAdapter,
    m01_spec_to_m12,
)
from efe_swarm_integration.integration.adapters_m03 import (
    M03ArtifactBlobSource,
    chunks_from_bytes,
)
from efe_swarm_integration.integration.adapters_m03_m05 import (
    M03BlobReader,
    M03RegistryBlobSource,
    M05ArtefactEventHandler,
    M05FtArtefactEvent,
)
from efe_swarm_integration.integration.adapters_m06 import (
    M06InferenceAdapter,
)
from efe_swarm_integration.integration.adapters_m07 import (
    M07PreflightAdapter,
    PreflightVerdict,
)
from efe_swarm_integration.integration.adapters_errors import (
    ABSORBED_ERROR_TAXONOMY,
    translate_to_swarm_error,
)
from efe_swarm_integration.integration.adapters_m09 import (
    DriftReactionOutcome,
    M09DriftAdapter,
    drift_result_to_event,
)
from efe_swarm_integration.integration.adapters_m10 import (
    DefaultM10Handlers,
    M10JobDispatcher,
    M10JobOutcome,
    build_default_dispatcher,
)
from efe_swarm_integration.integration.adapters_m11 import (
    M11AuditBridge,
    M11PdpAdapter,
    M11PiiAdapter,
    SwarmSecurityBundle,
    security_context_to_swarm,
)

__all__ = [
    "ABSORBED_ERROR_TAXONOMY",
    "DefaultM10Handlers",
    "DriftReactionOutcome",
    "M01PlatformAdapter",
    "M03ArtifactBlobSource",
    "M03BlobReader",
    "M03RegistryBlobSource",
    "M05ArtefactEventHandler",
    "M05FtArtefactEvent",
    "M06InferenceAdapter",
    "M07PreflightAdapter",
    "M09DriftAdapter",
    "M10JobDispatcher",
    "M10JobOutcome",
    "M11AuditBridge",
    "M11PdpAdapter",
    "M11PiiAdapter",
    "PlatformWiring",
    "PreflightVerdict",
    "SwarmSecurityBundle",
    "build_default_dispatcher",
    "chunks_from_bytes",
    "drift_result_to_event",
    "m01_spec_to_m12",
    "security_context_to_swarm",
    "translate_to_swarm_error",
    "wire_up_platform",
]
