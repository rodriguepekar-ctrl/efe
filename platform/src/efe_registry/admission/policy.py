"""Render a Sigstore ``ClusterImagePolicy`` from Python settings.

The CRD lives in the cluster; this module only generates YAML to be
applied by Argo CD or ``kubectl``. The tests validate stable output
across Python versions so drift is caught in CI.
"""

from __future__ import annotations

from dataclasses import dataclass

from efe_registry.config import AdmissionSettings, SigningSettings


@dataclass(frozen=True, slots=True)
class ClusterImagePolicy:
    name: str
    image_globs: tuple[str, ...]
    kms_key_arn: str | None
    kms_key_arn_secondary: str | None
    keyless_issuer: str | None
    keyless_identity: str | None
    rekor_url: str
    require_sbom: bool
    require_slsa: bool
    mode: str

    @classmethod
    def from_settings(
        cls,
        *,
        admission: AdmissionSettings,
        signing: SigningSettings,
    ) -> ClusterImagePolicy:
        return cls(
            name="efe-model-artifacts",
            image_globs=(admission.image_glob,),
            kms_key_arn=signing.kms_key_arn,
            kms_key_arn_secondary=signing.kms_key_arn_secondary,
            keyless_issuer=signing.keyless_issuer,
            keyless_identity=signing.keyless_cert_identity,
            rekor_url=signing.rekor_url,
            require_sbom=signing.require_sbom,
            require_slsa=signing.require_slsa,
            mode=admission.mode,
        )


def render_policy_yaml(policy: ClusterImagePolicy) -> str:
    """Render a human-readable YAML (indent=2) from the policy."""
    lines: list[str] = [
        "apiVersion: policy.sigstore.dev/v1beta1",
        "kind: ClusterImagePolicy",
        "metadata:",
        f"  name: {policy.name}",
        "spec:",
        "  images:",
    ]
    for glob in policy.image_globs:
        lines.append(f'    - glob: "{glob}"')
    lines.append("  authorities:")
    if policy.kms_key_arn:
        lines.extend([
            "    - name: efe-prod-kms",
            "      key:",
            f'        kms: "{policy.kms_key_arn}"',
            "      ctlog:",
            f'        url: "{policy.rekor_url}"',
        ])
        if policy.require_sbom or policy.require_slsa:
            lines.append("      attestations:")
            if policy.require_sbom:
                lines.extend([
                    "        - name: must-have-sbom",
                    "          predicateType: "
                    "https://cyclonedx.org/bom",
                ])
            if policy.require_slsa:
                lines.extend([
                    "        - name: must-have-slsa",
                    "          predicateType: "
                    "https://slsa.dev/provenance/v1",
                ])
    if policy.kms_key_arn_secondary:
        lines.extend([
            "    - name: efe-prod-kms-secondary",
            "      key:",
            f'        kms: "{policy.kms_key_arn_secondary}"',
            "      ctlog:",
            f'        url: "{policy.rekor_url}"',
        ])
    if policy.keyless_issuer and policy.keyless_identity:
        lines.extend([
            "    - name: efe-ci-keyless",
            "      keyless:",
            "        identities:",
            f'          - issuer: "{policy.keyless_issuer}"',
            f'            subject: "{policy.keyless_identity}"',
        ])
    lines.append(f"  mode: {policy.mode}")
    lines.append("")
    return "\n".join(lines)


__all__ = ["ClusterImagePolicy", "render_policy_yaml"]
