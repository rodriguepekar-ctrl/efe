"""``SignatureVerifier`` — single entry point for all 4 verification layers.

We wrap the ``cosign`` CLI via a :class:`CosignRunner` protocol so that
tests can plug a deterministic :class:`ScriptedCosignRunner`. In
production the default runner shells out to ``cosign`` (path configurable
via :class:`~efe_registry.config.SigningSettings.cosign_path`).

The verifier never raises on "signature invalid"; it returns a
:class:`VerificationResult` with ``ok=False``. It *does* raise on
operational issues (KMS/Rekor unavailable, misconfiguration).
"""

from __future__ import annotations

import asyncio
import re
import time
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Protocol

from efe_registry.config import SigningSettings
from efe_registry.errors import (
    KmsUnavailable,
    NoSigningMethodConfigured,
    RekorUnavailable,
    SignatureError,
    ValidationError,
)
from efe_registry.signing.cache import InMemorySignatureCache, SignatureCache
from efe_registry.signing.models import (
    SigningMethod,
    VerificationInput,
    VerificationLayer,
    VerificationResult,
)

# Accepted OCI reference charset. Intentionally conservative: alphanumerics,
# dot, slash, colon, underscore, dash, @, plus the SHA-256 digest separator.
# Rejects anything starting with ``-`` so cosign cannot interpret the ref
# as a flag even if the positional sentinel is missed.
_OCI_REF_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:/@+-]{0,511}$")


def validate_oci_ref(ref: str) -> None:
    """Raise :class:`ValidationError` if ``ref`` is not a safe OCI reference.

    Safe means: starts with an alphanumeric character, 1–512 chars, contains
    only the conservative OCI reference charset. No leading dash, no spaces,
    no shell metacharacters. Used both at publish time and before passing to
    cosign.
    """
    if not isinstance(ref, str) or not _OCI_REF_RE.match(ref):
        raise ValidationError(
            f"invalid OCI reference: {ref!r}; must match {_OCI_REF_RE.pattern}",
        )

# ---------------------------------------------------------------------------
# Cosign runner protocol + implementations
# ---------------------------------------------------------------------------


class CosignRunner(Protocol):
    async def run(
        self,
        argv: Sequence[str],
        *,
        timeout_s: float,
    ) -> tuple[int, str, str]:  # pragma: no cover
        """Return (returncode, stdout, stderr)."""


class NullCosignRunner:
    """Always returns success. Used in unit tests that don't care about cosign."""

    async def run(
        self, argv: Sequence[str], *, timeout_s: float,
    ) -> tuple[int, str, str]:
        return 0, '{"ok":true}', ""


@dataclass
class ScriptedCosignRunner:
    """Returns scripted responses keyed by a simple command fingerprint.

    Used to test behaviour without actually forking cosign. Commands are
    matched by joining ``argv`` with spaces and looking up
    :attr:`responses`; unmatched commands fall through to :attr:`default`.
    """

    responses: Mapping[str, tuple[int, str, str]]
    default: tuple[int, str, str] = (0, '{"ok":true}', "")
    calls: list[tuple[tuple[str, ...], float]] | None = None

    async def run(
        self, argv: Sequence[str], *, timeout_s: float,
    ) -> tuple[int, str, str]:
        key = " ".join(argv)
        if self.calls is not None:
            self.calls.append((tuple(argv), timeout_s))
        return self.responses.get(key, self.default)


class SubprocessCosignRunner:
    """Real runner; shells out to ``cosign``."""

    def __init__(self, *, cosign_path: str = "cosign") -> None:
        self._cosign_path = cosign_path

    async def run(
        self, argv: Sequence[str], *, timeout_s: float,
    ) -> tuple[int, str, str]:
        cmd = [self._cosign_path, *argv]
        proc = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        try:
            stdout_b, stderr_b = await asyncio.wait_for(
                proc.communicate(), timeout=timeout_s,
            )
        except asyncio.TimeoutError:
            proc.kill()
            await proc.wait()
            raise SignatureError(
                f"cosign timed out after {timeout_s:.1f}s: {' '.join(cmd)}",
            ) from None
        return (
            proc.returncode or 0,
            stdout_b.decode(errors="replace"),
            stderr_b.decode(errors="replace"),
        )


# ---------------------------------------------------------------------------
# Verifier
# ---------------------------------------------------------------------------


class SignatureVerifier:
    """Thin façade over cosign that caches hits and normalises results."""

    def __init__(
        self,
        *,
        settings: SigningSettings,
        runner: CosignRunner | None = None,
        cache: SignatureCache | None = None,
    ) -> None:
        self._settings = settings
        self._runner = runner or SubprocessCosignRunner(
            cosign_path=settings.cosign_path,
        )
        self._cache = cache or InMemorySignatureCache()

    # -------- public API --------

    async def verify(
        self, inp: VerificationInput,
    ) -> VerificationResult:
        cache_key = self._cache_key(inp)
        cached = await self._cache.get(cache_key)
        if cached is not None:
            return cached

        if inp.oci_ref is not None:
            result = await self._verify_image(inp)
        elif inp.blob_path is not None:
            result = await self._verify_blob(inp)
        else:
            raise SignatureError(
                "VerificationInput must provide oci_ref or blob_path",
            )

        if result.ok:
            await self._cache.set(
                cache_key, result, self._settings.verify_cache_ttl_s,
            )
        return result

    async def verify_artifact(self, artifact_id: str) -> bool:
        """Convenience helper used by :class:`ArtifactPromoter`.

        **Fails closed**: returns ``False`` when no verified record is
        present in the cache. Callers must explicitly seed the cache via
        :meth:`seed_artifact_ok` (e.g. after a successful admission L2
        check) before a promotion can succeed. This protects against an
        attacker crafting a promote request against an artifact that was
        never scanned.
        """
        cached = await self._cache.get(f"artifact:{artifact_id}")
        if cached is not None:
            return cached.ok
        return False

    # -------- L1/L2/L4 : image refs --------

    async def _verify_image(
        self, inp: VerificationInput,
    ) -> VerificationResult:
        assert inp.oci_ref is not None
        # Defense in depth: validate + use ``--`` sentinel before the
        # positional ref so cosign cannot interpret a hostile ref as a flag.
        validate_oci_ref(inp.oci_ref)
        method = self._pick_method()
        argv: list[str] = [
            "verify", "--output", "json",
        ]
        if method == SigningMethod.KEY_KMS:
            if not self._settings.kms_key_arn:
                raise NoSigningMethodConfigured("kms_key_arn not set")
            argv += ["--key", self._settings.kms_key_arn]
        elif method == SigningMethod.KEYLESS_OIDC:
            if not (
                self._settings.keyless_cert_identity
                and self._settings.keyless_issuer
            ):
                raise NoSigningMethodConfigured(
                    "keyless cert identity + issuer required",
                )
            argv += [
                "--certificate-identity",
                self._settings.keyless_cert_identity,
                "--certificate-oidc-issuer",
                self._settings.keyless_issuer,
                "--rekor-url", self._settings.rekor_url,
            ]
        argv += ["--", inp.oci_ref]
        rc, stdout, stderr = await self._runner.run(
            argv, timeout_s=self._settings.cli_timeout_s,
        )
        if rc != 0:
            _maybe_service_unavailable(stderr)
            return VerificationResult(
                ok=False, method=method, key_id=None,
                rekor_uuid=None, verified_at_ns=time.time_ns(),
                layer=inp.layer, details={"stderr": stderr[:512]},
            )
        # Attestations
        if inp.require_attestations:
            for pred in inp.require_attestations:
                argv2 = [
                    "verify-attestation", "--type", pred,
                    "--output", "json",
                ]
                if method == SigningMethod.KEY_KMS and self._settings.kms_key_arn:
                    argv2 += ["--key", self._settings.kms_key_arn]
                argv2 += ["--", inp.oci_ref]
                rc2, _, stderr2 = await self._runner.run(
                    argv2, timeout_s=self._settings.cli_timeout_s,
                )
                if rc2 != 0:
                    return VerificationResult(
                        ok=False, method=method, key_id=None,
                        rekor_uuid=None, verified_at_ns=time.time_ns(),
                        layer=inp.layer,
                        details={
                            "attestation_missing": pred,
                            "stderr": stderr2[:512],
                        },
                    )
        return VerificationResult(
            ok=True, method=method,
            key_id=self._settings.kms_key_arn,
            rekor_uuid=None,
            verified_at_ns=time.time_ns(),
            layer=inp.layer, details={"stdout_head": stdout[:256]},
        )

    # -------- L3 : blob verify (pre-load) --------

    async def _verify_blob(
        self, inp: VerificationInput,
    ) -> VerificationResult:
        assert inp.blob_path is not None
        if inp.signature_path is None:
            raise SignatureError(
                "blob verification requires signature_path",
            )
        # Paths cannot look like flags.
        for p in (inp.blob_path, inp.signature_path, inp.certificate_path):
            if p is not None and str(p).startswith("-"):
                raise SignatureError(
                    f"path starts with '-' and could be parsed as a flag: {p!s}",
                )
        method = self._pick_method()
        argv: list[str] = ["verify-blob"]
        if method == SigningMethod.KEY_KMS:
            if not self._settings.kms_key_arn:
                raise NoSigningMethodConfigured("kms_key_arn not set")
            argv += ["--key", self._settings.kms_key_arn]
        else:
            if inp.certificate_path is None:
                raise SignatureError(
                    "keyless blob verification requires certificate_path",
                )
            argv += [
                "--certificate", str(inp.certificate_path),
                "--certificate-identity",
                self._settings.keyless_cert_identity or "",
                "--certificate-oidc-issuer",
                self._settings.keyless_issuer or "",
            ]
        argv += [
            "--signature", str(inp.signature_path),
            "--", str(inp.blob_path),
        ]
        rc, stdout, stderr = await self._runner.run(
            argv, timeout_s=self._settings.cli_timeout_s,
        )
        ok = rc == 0
        if not ok:
            _maybe_service_unavailable(stderr)
        return VerificationResult(
            ok=ok, method=method,
            key_id=self._settings.kms_key_arn,
            rekor_uuid=None,
            verified_at_ns=time.time_ns(),
            layer=inp.layer,
            details={
                "stderr": stderr[:512] if not ok else "",
                "stdout_head": stdout[:256],
            },
        )

    # -------- helpers --------

    def _pick_method(self) -> SigningMethod:
        if self._settings.kms_key_arn:
            return SigningMethod.KEY_KMS
        if (
            self._settings.keyless_cert_identity
            and self._settings.keyless_issuer
        ):
            return SigningMethod.KEYLESS_OIDC
        raise NoSigningMethodConfigured(
            "either kms_key_arn or (keyless_cert_identity+keyless_issuer) "
            "must be configured",
        )

    def _cache_key(self, inp: VerificationInput) -> str:
        tag = inp.oci_ref or (
            str(inp.blob_path) if inp.blob_path is not None else ""
        )
        return f"verify:{inp.layer.value}:{tag}"

    # -------- cache utilities exposed for callers --------

    async def seed_artifact_ok(
        self, artifact_id: str, *, layer: VerificationLayer,
    ) -> None:
        """Manually mark an artifact verified — used after a successful
        out-of-band check (e.g. admission controller)."""
        key = f"artifact:{artifact_id}"
        await self._cache.set(
            key,
            VerificationResult(
                ok=True,
                method=SigningMethod.KEY_KMS,
                key_id=self._settings.kms_key_arn,
                rekor_uuid=None,
                verified_at_ns=time.time_ns(),
                layer=layer,
                details={"seeded": "true"},
            ),
            self._settings.verify_cache_ttl_s,
        )

    async def invalidate_all(self) -> None:
        await self._cache.invalidate("verify:")
        await self._cache.invalidate("artifact:")


def _maybe_service_unavailable(stderr: str) -> None:
    low = stderr.lower()
    if "kms" in low and ("unavailable" in low or "throttl" in low):
        raise KmsUnavailable(stderr[:512])
    if "rekor" in low and ("unavailable" in low or "timeout" in low):
        raise RekorUnavailable(stderr[:512])


__all__ = [
    "CosignRunner",
    "NullCosignRunner",
    "ScriptedCosignRunner",
    "SignatureVerifier",
    "SubprocessCosignRunner",
]
