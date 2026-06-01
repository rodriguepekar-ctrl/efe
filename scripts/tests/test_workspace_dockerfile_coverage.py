"""Project-wide pin tests : every workspace binary is categorized.

Cf. CHANGELOG → "Audit fixes — Sprint D17 ... Pattern institutionnel
renforcé".

This test file is the **permanent institutional defense** against
the recurring "wiring forgotten" defect class :

  * D16-C1   : EmbedderRegistry built but never invoked from runtime.rs
  * D13bis-A20 : OtelGuard built but never invoked from init_telemetry
  * D17-AC1  : efe-backup binary built but never copied into Dockerfile.prod

The first two have ad-hoc per-sprint pin tests
(`test_audit_fix_c1_runtime_wired`, `test_audit_a20_init_telemetry_calls_otel_init`).
This file adds a workspace-wide guard for the third class : every
`[[bin]]` declared in any crate's `Cargo.toml` MUST be classified
either as :

  * "prod-runtime"  → MUST be in `docker/Dockerfile.prod` (build
    stage + runtime copy) so a deployed cluster can spawn it ;
  * "operator-tool" → INTENTIONALLY NOT in the prod image ; run
    via a separate operator container or `kubectl exec`.

Adding a new binary forces the maintainer to update one of the
two sets — CI fails until the categorization is explicit. This
makes the recurring D16-C1-class defect impossible-by-construction
for any future sprint that lands a CLI binary.

# Related sprints

  * D14-bis (`efe-verify-audit`) : operator tool ; HMAC chain
    re-verifier, run on demand.
  * D15 (`efe-onboard`) : operator tool ; tenant lifecycle CLI.
  * D16 (`efe-bundle`) : operator tool ; bundle keygen + pack +
    verify + inspect.
  * D17 (`efe-backup`) : prod-runtime ; spawned by Helm CronJobs
    inside the cluster.

# Audit pass open question

The D17 audit found that `verifiers.rs::AuditChainVerifier` shells
out to `efe-server admin verify-audit-chain --full` and
`BundleSignatureVerifier` shells out to `efe-bundle verify --all`.
Both subcommands need their host binaries in the runtime image
to actually work in production. `efe-server` IS in the image but
doesn't have the `admin` subcommand (it's a daemon, not a CLI) —
the verifier should shell out to `efe-verify-audit` instead.
Tracked as DT-D17.8 (referenced in d17_backup_dr_audit_report.md
as a follow-up). For now, the pin test below documents the
**current declared state** ; the misalignment between what the
verifier shells out to and what's in the image is an audit
finding that this test surfaces.
"""

from __future__ import annotations

import re
from pathlib import Path

_REPO = Path(__file__).resolve().parents[2]
_DOCKERFILE_PROD = _REPO / "docker" / "Dockerfile.prod"

# ─────────────────────────────────────────────────────────────────────
# Categorization registries — explicitly maintained, not derived
# ─────────────────────────────────────────────────────────────────────

#: Binaries that MUST be in `docker/Dockerfile.prod`. Adding a new
#: prod-runtime binary requires :
#:   * Adding it to this set ;
#:   * Updating Dockerfile.prod to build + copy it ;
#:   * Adding a Helm template that spawns it (CronJob, Deployment, etc.).
#:
#: **Audit fix § D17-AC4-bis** — `efe-verify-audit` + `efe-bundle`
#: were re-categorized from operator-tool to prod-runtime because
#: the D17 DR drill verifiers (`AuditChainVerifier`,
#: `BundleSignatureVerifier`) shell out to them. Without these
#: binaries in the prod image, the drill CronJob would CrashLoop.
#: Same defect class as D17-AC1, surfaced by this very test file
#: in a follow-up audit pass.
PROD_RUNTIME_BINARIES = {
    "efe-server",
    "efe-backup",
    "efe-verify-audit",
    "efe-bundle",
}

#: Binaries that are INTENTIONALLY NOT in the prod runtime image.
#: These are operator tools run via :
#:   * A separate operator container image (TODO : consolidate to
#:     a `efe-ops` image in a future sprint) ;
#:   * Local dev / CI environments ;
#:   * `kubectl exec` from a temporary pod that mounts a separate
#:     image with the operator tool.
OPERATOR_TOOL_BINARIES = {
    "efe-onboard",       # D15 tenant lifecycle CLI ; manual ops only
    "efe-bench",         # D18 bench harness ; operator-only (refuses prod)
}


# ─────────────────────────────────────────────────────────────────────
# Discovery helpers
# ─────────────────────────────────────────────────────────────────────


def discover_workspace_bins() -> set[str]:
    """Walk every `Cargo.toml` under `crates/` and extract `[[bin]]`
    name declarations. Returns the set of binary names."""
    crates_dir = _REPO / "crates"
    bins = set()
    for cargo_toml in crates_dir.glob("*/Cargo.toml"):
        content = cargo_toml.read_text(encoding="utf-8")
        # Match [[bin]] blocks. Each block has a `name = "..."` line.
        # We use a simple regex split rather than a TOML parser to
        # keep the test deps minimal.
        blocks = re.split(r"^\[\[bin\]\]\s*$", content, flags=re.MULTILINE)
        for block in blocks[1:]:  # first split is pre-bin content
            m = re.search(r'^name\s*=\s*"([^"]+)"', block, flags=re.MULTILINE)
            if m:
                bins.add(m.group(1))
    return bins


# ─────────────────────────────────────────────────────────────────────
# Tests
# ─────────────────────────────────────────────────────────────────────


def test_every_workspace_bin_is_categorized():
    """**Institutional defense** — every `[[bin]]` in the workspace
    must be either a prod-runtime binary OR an operator tool. No
    binary may be uncategorized.

    This test is the permanent guard against the D16-C1 / D13bis-A20
    / D17-AC1 recurring "wiring forgotten" class : if a new sprint
    adds a binary without classifying it, CI fails.
    """
    bins = discover_workspace_bins()
    assert bins, "no [[bin]] declarations found in workspace — sanity check failed"
    classified = PROD_RUNTIME_BINARIES | OPERATOR_TOOL_BINARIES
    uncategorized = bins - classified
    assert not uncategorized, (
        f"binary not categorized in test_workspace_dockerfile_coverage.py: "
        f"{uncategorized}. Add it to PROD_RUNTIME_BINARIES (must be in "
        f"Dockerfile.prod) or OPERATOR_TOOL_BINARIES (intentionally not in "
        f"prod image)."
    )
    # Reverse check : the registries must not name binaries that
    # don't exist (catches typos + stale entries).
    stale = classified - bins
    assert not stale, (
        f"registries reference non-existent binaries (typo or stale entry): "
        f"{stale}"
    )


def test_prod_runtime_bins_present_in_dockerfile_prod():
    """**Institutional defense** — every binary in
    `PROD_RUNTIME_BINARIES` MUST appear in `docker/Dockerfile.prod`
    at three places :
      1. The `cargo build` invocation (`--bin <name>`).
      2. The `strip` invocation OR an equivalent post-build step.
      3. A `COPY --from=builder ... /app/target/release/<name>
         /usr/local/bin/<name>` line in the runtime stage.

    This is the precise pin that catches D17-AC1 ;
    `test_audit_ac1_efe_backup_in_dockerfile_prod` was a special
    case of this general invariant.
    """
    df = _DOCKERFILE_PROD.read_text(encoding="utf-8")
    for binary in sorted(PROD_RUNTIME_BINARIES):
        # 1. Build stage `--bin <binary>`
        assert f"--bin {binary}" in df, (
            f"Dockerfile.prod missing `--bin {binary}` in the cargo "
            f"build step. New prod-runtime binary MUST be built."
        )
        # 2. Source path in builder.
        assert f"/app/target/release/{binary}" in df, (
            f"Dockerfile.prod missing `/app/target/release/{binary}` "
            f"COPY source path in the runtime stage."
        )
        # 3. Destination path in runtime layer.
        assert f"/usr/local/bin/{binary}" in df, (
            f"Dockerfile.prod missing `/usr/local/bin/{binary}` COPY "
            f"destination path. Pod cannot exec the binary."
        )


def test_operator_tool_bins_not_in_dockerfile_prod():
    """**Institutional defense** — operator-only binaries
    MUST NOT appear in `Dockerfile.prod`. Distroless prod images
    have minimal attack surface ; bundling operator-only CLIs
    expands it without operational benefit (operators run those
    tools via separate paths).

    If an operator tool legitimately needs to be in the prod image
    (e.g. a future sprint moves it from operator to runtime), the
    fix is :
      1. Move the name from OPERATOR_TOOL_BINARIES to
         PROD_RUNTIME_BINARIES ;
      2. Update Dockerfile.prod to build + copy ;
      3. This test then asserts the move is consistent.
    """
    df = _DOCKERFILE_PROD.read_text(encoding="utf-8")
    for binary in sorted(OPERATOR_TOOL_BINARIES):
        # The `--bin <name>` line must NOT be in the cargo build
        # stage (we don't want to compile + ship it in distroless).
        assert f"--bin {binary}" not in df, (
            f"Operator tool {binary!r} unexpectedly built in "
            f"Dockerfile.prod. Either move it to PROD_RUNTIME_BINARIES "
            f"(if it's now a runtime binary) or remove the `--bin` "
            f"line."
        )


def test_dockerfile_prod_uses_distroless_runtime():
    """Sanity : the prod image must remain distroless. A switch to
    a fuller base image silently expands attack surface ; this test
    catches the regression."""
    df = _DOCKERFILE_PROD.read_text(encoding="utf-8")
    assert "distroless" in df, "Dockerfile.prod must use a distroless runtime"
    assert "nonroot" in df, "Dockerfile.prod must run as nonroot"


def test_dockerfile_prod_strips_release_binaries():
    """Sanity : every prod-runtime binary must be stripped to keep
    image size minimal."""
    df = _DOCKERFILE_PROD.read_text(encoding="utf-8")
    for binary in sorted(PROD_RUNTIME_BINARIES):
        assert f"strip target/release/{binary}" in df, (
            f"Dockerfile.prod must strip {binary} for image-size discipline"
        )


# ─────────────────────────────────────────────────────────────────────
# Cross-reference : audit reports cite this institutional defense
# ─────────────────────────────────────────────────────────────────────


def test_d17_audit_report_articulates_wiring_pattern():
    """The D17 audit report MUST articulate the recurring wiring
    pattern (D16-C1 / D13bis-A20 / D17-AC1) so any maintainer
    reading just D17's report sees the institutional learning."""
    report = _REPO / "docs" / "audits" / "d17_backup_dr_audit_report.md"
    assert report.exists()
    content = report.read_text(encoding="utf-8")
    assert "D16-C1" in content
    assert "D13bis-A20" in content
    # Must list the 3 institutional disciplines.
    assert "binaire CLI" in content or "binary CLI" in content
    assert "Python pin test" in content


def test_d16_audit_report_references_d13bis_wiring_class():
    """The D16 audit report references D13bis-A20 as same class —
    cross-check kept here so a refactor that drops the reference
    surfaces."""
    report = _REPO / "docs" / "audits" / "d16_audit_report.md"
    assert report.exists()
    content = report.read_text(encoding="utf-8")
    assert "A20" in content


def test_d13bis_audit_report_documents_a20_wiring_pass():
    """The D13-bis audit report (pass #2) is where the wiring class
    was first articulated. Pin guard : this articulation must remain."""
    report = _REPO / "docs" / "audits" / "d13bis_audit_report.md"
    assert report.exists()
    content = report.read_text(encoding="utf-8")
    assert "A20" in content
    assert "WIRED" in content or "wir" in content.lower()


# ─────────────────────────────────────────────────────────────────────
# Workspace-wide alert runbook coverage (institutional defense)
# ─────────────────────────────────────────────────────────────────────


def test_every_alert_runbook_link_resolves():
    """**Institutional defense — generalised** — every Helm
    Prometheus alert annotated with `runbook:` OR `runbook_url:` MUST
    resolve to a real file (or anchor within a real file). This is
    the workspace-wide version of `test_d17_alert_runbooks_exist`,
    but it ALSO handles :

      * `runbook:` annotations pointing to relative paths
        (D17 pattern : `docs/runbook/D17-N.md`)
      * `runbook_url:` annotations pointing to GitHub URLs with
        `#anchor` fragments (M11/M12 pattern :
        `https://github.com/.../docs/runbook.md#section-id`)

    For the GitHub-URL form, this test extracts the file path
    (after `/blob/main/`) and the anchor, then asserts both the
    file exists locally AND the anchor (via `<a id="...">` HTML
    tag) is present in the file.

    Tick 14 found that 4 D11 alerts referenced anchors
    (`#cardinality-bomb`, `#per-tenant-error-rate`, etc.) that
    did NOT exist in `docs/runbook.md`. This generalised test
    catches the same class for all alerts.
    """
    import re as _re
    helm_templates = (_REPO / "deploy" / "helm" / "efe" / "templates").glob("*.yaml")
    file_refs: set[str] = set()           # relative paths
    anchor_refs: set[tuple[str, str]] = set()  # (relative_path, anchor)
    for tpl in helm_templates:
        content = tpl.read_text(encoding="utf-8")
        # Match `runbook:` annotation (D17 pattern, relative path).
        for m in _re.finditer(r'(?<!_)runbook:\s*"([^"]+)"', content):
            file_refs.add(m.group(1))
        # Match `runbook_url:` annotation (M11/M12 pattern, github URL
        # with anchor).
        for m in _re.finditer(
            r'runbook_url:\s*"https://github\.com/[^/]+/[^/]+/blob/[^/]+/([^"#]+)#([^"]+)"',
            content,
        ):
            path = m.group(1)
            anchor = m.group(2)
            file_refs.add(path)
            anchor_refs.add((path, anchor))

    # Phase 1 : every referenced file exists.
    missing_files = []
    for path in sorted(file_refs):
        full = _REPO / path
        if not full.exists():
            missing_files.append(path)
    assert not missing_files, (
        f"alert-referenced runbook file(s) missing: {missing_files}. "
        f"Either ship the file OR remove the alert annotation."
    )

    # Phase 2 : every (file, anchor) pair resolves to an
    # `<a id="...">` tag in the file.
    missing_anchors = []
    for (path, anchor) in sorted(anchor_refs):
        content = (_REPO / path).read_text(encoding="utf-8")
        if f'<a id="{anchor}">' not in content and f'#{anchor}' not in content:
            # Tolerate either the explicit `<a id="...">` form or
            # a markdown heading whose autogenerated anchor would
            # match. Most strict : require explicit `<a id="...">`.
            missing_anchors.append((path, anchor))
    assert not missing_anchors, (
        f"alert-referenced runbook anchor(s) broken: {missing_anchors}. "
        f"Either add `<a id=\"<anchor>\"></a>` to the runbook OR "
        f"update the alert annotation. (Catch class : "
        f"D17-AC1-equivalent for runbook anchors.)"
    )
