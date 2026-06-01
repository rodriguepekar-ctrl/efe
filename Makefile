# ─────────────────────────────────────────────────────────────────────
# PR #15 — Repository Makefile.
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 17.
#
# Local-dev targets that mirror CI Jobs 02-13.  Run before pushing
# to surface lint failures without round-tripping through CI.
# ─────────────────────────────────────────────────────────────────────

SHELL := /usr/bin/env bash
.SHELLFLAGS := -eu -o pipefail -c
.DEFAULT_GOAL := help

# ─── Paths ──────────────────────────────────────────────────────────
CHART_DIR := charts/efe-stack/charts/efe-meta-swarm
POLICY_DIR := policies/rego/efe
DATA_DIR := policies/rego/efe/data
RENDERED_DIR := /tmp/efe-rendered

# ─── Tool versions (must match .github/workflows/helm-meta-swarm-lint.yml) ──
HELM_DOCS_VERSION := v1.12.0
CONFTEST_VERSION  := v0.49.1

# ─── Phony targets ──────────────────────────────────────────────────
.PHONY: help \
        helm-docs helm-docs-check \
        lint lint-yaml lint-helm lint-rego lint-rendered \
        render render-clean \
        rego-test \
        trivyignore-prune \
        bats-self-test

# ════════════════════════════════════════════════════════════════════
help:                ## Show this help.
	@awk 'BEGIN {FS = ":.*##"} /^[a-zA-Z0-9_-]+:.*##/ {printf "  \033[36m%-22s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

# ════════════════════════════════════════════════════════════════════
# Job 06 — helm-docs
# ════════════════════════════════════════════════════════════════════
helm-docs:           ## Regenerate $(CHART_DIR)/README.md from values.yaml + Chart.yaml.
	@if command -v helm-docs >/dev/null 2>&1 ; then \
	  helm-docs \
	    --chart-search-root=$(CHART_DIR) \
	    --template-files=README.md.gotmpl \
	    --output-file=README.md ; \
	elif command -v docker >/dev/null 2>&1 ; then \
	  docker run --rm \
	    -v "$(PWD)/$(CHART_DIR):/helm-docs" \
	    "jnorwood/helm-docs:$(HELM_DOCS_VERSION)" \
	    --template-files=README.md.gotmpl \
	    --output-file=README.md ; \
	else \
	  echo "✗ neither helm-docs nor docker installed" ; exit 2 ; \
	fi
	@echo "✅ $(CHART_DIR)/README.md regenerated"

helm-docs-check:     ## CI mirror : assert README.md is byte-identical to helm-docs regeneration.
	@$(MAKE) helm-docs HELM_DOCS_VERSION=$(HELM_DOCS_VERSION) > /dev/null
	@if ! git diff --exit-code -- $(CHART_DIR)/README.md ; then \
	  echo "❌ $(CHART_DIR)/README.md out of sync.  Commit the diff." ; exit 1 ; \
	fi
	@echo "✅ README.md in sync"

# ════════════════════════════════════════════════════════════════════
# Job 02 — style
# ════════════════════════════════════════════════════════════════════
lint-yaml:           ## yamllint everything against `.yamllint`.
	yamllint -c .yamllint .

# ════════════════════════════════════════════════════════════════════
# Job 03 — helm lint --strict
# ════════════════════════════════════════════════════════════════════
lint-helm:           ## helm lint --strict for the meta-swarm sub-chart.
	helm lint $(CHART_DIR) --strict -f $(CHART_DIR)/values.yaml
	@for scen in kovant-3p kovant-disabled kovant-network-policy \
	             kovant-service-monitor kovant-prod kovant-hooks ; do \
	  echo "── lint $$scen ──" ; \
	  helm lint $(CHART_DIR) --strict \
	    -f $(CHART_DIR)/tests/scenarios/$$scen.values.yaml ; \
	done

# ════════════════════════════════════════════════════════════════════
# Render every scenario into $(RENDERED_DIR) — mirror Job 01.
# ════════════════════════════════════════════════════════════════════
render:              ## Render all 6 scenarios + 4 overlays into $(RENDERED_DIR).
	@mkdir -p $(RENDERED_DIR)
	@for scen in kovant-3p kovant-disabled kovant-network-policy \
	             kovant-service-monitor kovant-prod kovant-hooks ; do \
	  echo "── render $$scen ──" ; \
	  helm template tst $(CHART_DIR) --namespace efe \
	    -f $(CHART_DIR)/tests/scenarios/$$scen.values.yaml \
	    > $(RENDERED_DIR)/$$scen.yaml ; \
	done
	@for flavor in dev staging prod canary ; do \
	  echo "── render values-$$flavor ──" ; \
	  helm template tst $(CHART_DIR) --namespace efe \
	    -f $(CHART_DIR)/examples/values-$$flavor.yaml \
	    > $(RENDERED_DIR)/values-$$flavor.yaml ; \
	done
	@ls -la $(RENDERED_DIR)/

render-clean:        ## Wipe $(RENDERED_DIR).
	rm -rf $(RENDERED_DIR)

# ════════════════════════════════════════════════════════════════════
# Job 08 — Conftest.
# ════════════════════════════════════════════════════════════════════
rego-test:           ## conftest verify (Rego unit tests).
	conftest verify --policy $(POLICY_DIR) --data $(DATA_DIR)

lint-rendered: render ## conftest test on every rendered scenario.
	@# PR #15 audit pass — `--all-namespaces` is REQUIRED so Conftest
	@# collects denies from every `package efe.<rule>`.
	@for f in $(RENDERED_DIR)/*.yaml ; do \
	  echo "── policy check $$(basename $$f) ──" ; \
	  conftest test --all-namespaces \
	    --policy $(POLICY_DIR) --data $(DATA_DIR) --no-color "$$f" ; \
	done

lint-rego:           ## opa fmt --diff (Rego style check, advisory).
	@if command -v opa >/dev/null 2>&1 ; then \
	  opa fmt --diff $(POLICY_DIR) ; \
	else \
	  echo "ℹ️  opa not installed ; skipping format check" ; \
	fi

# ════════════════════════════════════════════════════════════════════
# Aggregator : all-of-the-above
# ════════════════════════════════════════════════════════════════════
lint: lint-yaml lint-helm rego-test lint-rendered helm-docs-check  ## Run the full lint suite locally.
	@echo "✅ full lint suite passed"

# ════════════════════════════════════════════════════════════════════
# Self-test BATS suite.
# ════════════════════════════════════════════════════════════════════
bats-self-test:      ## Run tests/workflow/lint-self-test.bats.
	bats tests/workflow/lint-self-test.bats

# ════════════════════════════════════════════════════════════════════
# Hygiene utilities
# ════════════════════════════════════════════════════════════════════
trivyignore-prune:   ## Prune .trivyignore entries with stale `Re-review by:` dates.
	@today=$$(date +%Y-%m-%d) ; \
	awk -v today="$$today" ' \
	  /^# Re-review by :/ { \
	    if ($$5 < today) print "::warning::" FILENAME ":" NR " - .trivyignore entry past re-review date (" $$5 " < " today ")" ; \
	  } \
	  { print } \
	' .trivyignore

# ════════════════════════════════════════════════════════════════════
# .PHONY guard at end-of-file caught by yamllint as "trailing-spaces"
# even though it's not.  Pad with explicit content.
# ════════════════════════════════════════════════════════════════════
