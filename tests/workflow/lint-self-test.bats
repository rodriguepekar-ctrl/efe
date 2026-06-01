#!/usr/bin/env bats
# ─────────────────────────────────────────────────────────────────────
# PR #15 — lint-self-test.bats.
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 35.
#
# Black-box test : introduce DELIBERATE violations into each rendered
# scenario and assert that the corresponding Rego rule fires (deny).
# Then introduce the inverse (compliant) version and assert no deny.
#
# Audit-pass update : every `conftest test` invocation uses
# `--all-namespaces` to collect denies from per-rule packages
# (`package efe.<rule>`).  Each assertion targets the rule's
# `[<rule_name>]` message prefix.
#
# Run locally :
#   bats tests/workflow/lint-self-test.bats
#
# Requires : `helm`, `yq` (v4+), `conftest`, `bats`.
# ─────────────────────────────────────────────────────────────────────

setup() {
  for tool in helm yq conftest bats ; do
    command -v "$tool" >/dev/null 2>&1 || {
      skip "tool not installed : $tool"
    }
  done

  REPO_ROOT="$(git -C "$BATS_TEST_DIRNAME" rev-parse --show-toplevel)"
  CHART="$REPO_ROOT/charts/efe-stack/charts/efe-meta-swarm"
  POLICY="$REPO_ROOT/policies/rego/efe"
  DATA="$REPO_ROOT/policies/rego/efe/data"
  TMP="$(mktemp -d)"

  helm template tst "$CHART" --namespace efe \
    -f "$CHART/tests/scenarios/kovant-3p.values.yaml" \
    > "$TMP/baseline.yaml"
}

teardown() {
  rm -rf "$TMP"
}

# Helper : run conftest with the right flags every time.
_conftest_test() {
  conftest test --all-namespaces \
    --policy "$POLICY" \
    --data "$DATA" \
    --no-color \
    "$@"
}

# ─── Test 1 — Rego rules themselves pass `conftest verify` ──────────

@test "Rego rules pass conftest verify (unit tests)" {
  run conftest verify --policy "$POLICY" --data "$DATA"
  [ "$status" -eq 0 ]
}

# ─── Test 2 — baseline render is policy-clean ───────────────────────

@test "baseline kovant-3p render is policy-clean" {
  run _conftest_test "$TMP/baseline.yaml"
  [ "$status" -eq 0 ]
}

# ─── Test 3 — tenant_label fires on missing efe.io/tenant ───────────

@test "tenant_label fires on missing efe.io/tenant" {
  yq 'del(.metadata.labels."efe.io/tenant")' "$TMP/baseline.yaml" \
    > "$TMP/no-tenant.yaml"

  run _conftest_test "$TMP/no-tenant.yaml"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "[tenant_label]" ]]
}

# ─── Test 4 — no_latest_tag fires on :latest image ─────────────────

@test "no_latest_tag fires on :latest image" {
  yq '(select(.kind == "Deployment") | .spec.template.spec.containers[0].image) = "ghcr.io/efe-ai/srv:latest"' \
    "$TMP/baseline.yaml" > "$TMP/latest-tag.yaml"

  run _conftest_test "$TMP/latest-tag.yaml"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "[no_latest_tag]" ]]
}

# ─── Test 5 — security_context fires when caps.drop misses ALL ─────

@test "security_context fires when capabilities.drop missing ALL" {
  yq '(select(.kind == "Deployment") | .spec.template.spec.containers[0].securityContext.capabilities.drop) = ["NET_RAW"]' \
    "$TMP/baseline.yaml" > "$TMP/no-drop-all.yaml"

  run _conftest_test "$TMP/no-drop-all.yaml"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "[security_context]" ]]
}

# ─── Test 6 — no_privileged fires on privileged: true ──────────────

@test "no_privileged fires on privileged: true" {
  yq '(select(.kind == "Deployment") | .spec.template.spec.containers[0].securityContext.privileged) = true' \
    "$TMP/baseline.yaml" > "$TMP/privileged.yaml"

  run _conftest_test "$TMP/privileged.yaml"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "[no_privileged]" ]]
}

# ─── Test 7 — no_host_namespace fires on hostNetwork: true ─────────

@test "no_host_namespace fires on hostNetwork: true" {
  yq '(select(.kind == "Deployment") | .spec.template.spec.hostNetwork) = true' \
    "$TMP/baseline.yaml" > "$TMP/host-net.yaml"

  run _conftest_test "$TMP/host-net.yaml"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "[no_host_namespace]" ]]
}

# ─── Test 8 — sa_token fires when runtime pod doesn't disable token ─

@test "sa_token fires when runtime Deployment doesn't disable token" {
  yq '(select(.kind == "Deployment") | .spec.template.spec.automountServiceAccountToken) = true' \
    "$TMP/baseline.yaml" > "$TMP/sa-token.yaml"

  run _conftest_test "$TMP/sa-token.yaml"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "[sa_token]" ]]
}

# ─── Test 9 — share_process_namespace gates on sidecar presence ────

@test "share_process_namespace fires when no bundle-loader present" {
  yq '(select(.kind == "Deployment") | .spec.template.spec.shareProcessNamespace) = true' \
    "$TMP/baseline.yaml" > "$TMP/share-pid.yaml"

  run _conftest_test "$TMP/share-pid.yaml"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "[share_process_namespace]" ]]
}

# ─── Test 10 — service_appprotocol fires when appProtocol missing ──

@test "service_appprotocol fires when appProtocol missing" {
  yq '(select(.kind == "Service") | .spec.ports[].appProtocol) = null | del(.. | select(. == null))' \
    "$TMP/baseline.yaml" > "$TMP/no-appproto.yaml"

  run _conftest_test "$TMP/no-appproto.yaml"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "[service_appprotocol]" ]]
}

# ─── Test 11 — container_resources fires when limits missing ───────

@test "container_resources fires when resources.limits missing" {
  yq '(select(.kind == "Deployment") | del(.spec.template.spec.containers[0].resources.limits))' \
    "$TMP/baseline.yaml" > "$TMP/no-limits.yaml"

  run _conftest_test "$TMP/no-limits.yaml"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "[container_resources]" ]]
}

# ─── Test 12 — networkpolicy_types fires on empty policyTypes ──────

@test "networkpolicy_types fires when policyTypes empty" {
  helm template tst "$CHART" --namespace efe \
    -f "$CHART/tests/scenarios/kovant-network-policy.values.yaml" \
    > "$TMP/np-baseline.yaml"

  yq '(select(.kind == "NetworkPolicy") | .spec.policyTypes) = []' \
    "$TMP/np-baseline.yaml" > "$TMP/np-empty.yaml"

  run _conftest_test "$TMP/np-empty.yaml"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "[networkpolicy_types]" ]]
}

# ─── Test 13 — prometheusrule_runbook fires when runbook_url missing ─

@test "prometheusrule_runbook fires when runbook_url missing" {
  helm template tst "$CHART" --namespace efe \
    -f "$CHART/tests/scenarios/kovant-prod.values.yaml" \
    > "$TMP/pr-baseline.yaml"

  yq '(select(.kind == "PrometheusRule") | del(.. | select(has("runbook_url")).runbook_url))' \
    "$TMP/pr-baseline.yaml" > "$TMP/pr-no-runbook.yaml"

  run _conftest_test "$TMP/pr-no-runbook.yaml"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "[prometheusrule_runbook]" ]]
}

# ─── Test 14 — kubernetes.rego fires on default ServiceAccount ─────

@test "kubernetes.rego fires on default ServiceAccount" {
  yq '(select(.kind == "Deployment") | del(.spec.template.spec.serviceAccountName))' \
    "$TMP/baseline.yaml" > "$TMP/default-sa.yaml"

  run _conftest_test "$TMP/default-sa.yaml"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "[kubernetes.cis]" ]]
}

# ─── Test 15 — kubernetes.rego enforces Phase-2 HPA min=max=1 ──────

@test "kubernetes.rego rejects meta-swarm controller HPA with min != 1" {
  helm template tst "$CHART" --namespace efe \
    -f "$CHART/tests/scenarios/kovant-prod.values.yaml" \
    --set controller.hpa.enabled=true \
    > "$TMP/hpa-baseline.yaml"

  yq '(select(.kind == "HorizontalPodAutoscaler") | .spec.minReplicas) = 2' \
    "$TMP/hpa-baseline.yaml" > "$TMP/hpa-min2.yaml"

  run _conftest_test "$TMP/hpa-min2.yaml"
  [ "$status" -ne 0 ]
  [[ "$output" =~ "[kubernetes.cis]" ]]
}

# ─── Test 16 — packages are correctly per-rule (audit regression) ──

@test "per-rule packages discovered by conftest verify" {
  # Every rule file has its own package efe.<name> ; ensure conftest
  # finds them all.  Greps the verify output for 'efe.<rule>' headers.
  run conftest verify --policy "$POLICY" --data "$DATA" --trace 2>&1 || true
  for pkg in tenant_label no_latest_tag security_context no_privileged \
             no_host_namespace sa_token share_process_namespace \
             service_appprotocol container_resources networkpolicy_types \
             prometheusrule_runbook kubernetes ; do
    [[ "$output" =~ "efe.${pkg}" ]] || {
      echo "package efe.${pkg} not discovered by conftest verify"
      return 1
    }
  done
}
