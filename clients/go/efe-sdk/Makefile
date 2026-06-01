# Sprint D23 — `efe-sdk-go` canonical entry points.
#
# All targets are idempotent and CI-friendly. Use via:
#
#   make gen-proto    # regenerate gRPC stubs from protos/
#   make build        # go build ./...
#   make test         # go test -race -coverprofile=coverage.out ./...
#   make cover        # → coverage.html
#   make lint         # golangci-lint + go vet + govulncheck
#   make examples     # build all 3 framework sub-modules
#   make publish      # tag + push (private → public via PUBLISH=public)
.PHONY: help gen-proto build test cover lint bench examples publish clean

help:
	@echo "efe-sdk-go targets:"
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | sort \
	    | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-12s %s\n", $$1, $$2}'

gen-proto: ## Regenerate gRPC stubs into internal/proto/
	@./scripts/gen-proto.sh

build: ## go build ./...
	@go build ./...

test: ## go test -race -coverprofile ./...
	@go test -race -coverprofile=coverage.out -covermode=atomic ./...

cover: test ## Generate coverage HTML
	@go tool cover -func=coverage.out
	@go tool cover -html=coverage.out -o coverage.html
	@echo "→ open coverage.html"

lint: ## Lint via golangci-lint + go vet + govulncheck
	@./scripts/lint.sh

bench: ## go test -bench
	@go test -bench=. -benchmem -run=^$$ ./...

examples: ## Build all 3 framework sub-modules
	@for f in examples/*/go.mod; do \
	    dir=$$(dirname $$f); \
	    echo "→ building $$dir"; \
	    (cd $$dir && go build ./...) || exit 1; \
	done

publish: ## Tag + push the SDK (PUBLISH=public for the proxy)
	@./scripts/publish.sh

clean: ## Remove build artefacts
	@rm -f coverage.out coverage.html
	@find . -name '*.test' -delete
