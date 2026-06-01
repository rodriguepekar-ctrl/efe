# EFE M26 — agent-runner controller image (Rust binary).
#
# Multi-stage : cargo-chef → cargo build → distroless runtime.
# Static binary ; no glibc surprises ; signed via cosign in CI.
#
# Build context : the workspace root (parent dir contains workspace Cargo.toml).
#
# Usage :
#   docker build -f crates/efe-agent-runner/Dockerfile \
#                -t ghcr.io/efe-ai/efe-agent-runner:2.1.0 .

# ----- Stage 1 : recipe ---------------------------------------------------
FROM rust:1.88-bookworm AS chef
RUN cargo install --locked cargo-chef@^0.1
WORKDIR /build
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ----- Stage 2 : cook (build dep cache layer) -----------------------------
FROM rust:1.88-bookworm AS cacher
WORKDIR /build
# Φ-D-OPS post-audit fix B6 : the workspace contains `efe-ffi`
# (pyo3-build-config probes for a Python interpreter) + `sqlx` /
# `reqwest` (openssl-sys → needs `pkg-config` + `libssl-dev`) +
# `prost-build` (vendored protoc, but `protobuf-compiler` keeps
# user-friendly errors if vendored bin fails on a new arch). The
# runtime image is distroless and never sees any of these.
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev protobuf-compiler python3 python3-dev \
    && rm -rf /var/lib/apt/lists/*
# Φ-D-OPS post-audit fix C2 : pin cargo-chef exact version for
# reproducibility (matches docker/Dockerfile.prod CHEF_VERSION=0.1.67).
RUN cargo install --locked cargo-chef --version 0.1.67
COPY --from=chef /build/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json \
                    --bin efe-agent-runner

# ----- Stage 3 : real build ----------------------------------------------
FROM rust:1.88-bookworm AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev protobuf-compiler python3 python3-dev \
    && rm -rf /var/lib/apt/lists/*
COPY . .
COPY --from=cacher /build/target target
COPY --from=cacher /usr/local/cargo /usr/local/cargo
RUN cargo build --release --bin efe-agent-runner --locked

# ----- Stage 4 : runtime --------------------------------------------------
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
LABEL org.opencontainers.image.source="https://github.com/efe-ai/efe"
LABEL org.opencontainers.image.title="efe-agent-runner"
LABEL org.opencontainers.image.description="EFE M26 agent runtime controller + webhook"
LABEL org.opencontainers.image.version="2.1.0"
LABEL org.opencontainers.image.licenses="Apache-2.0"
COPY --from=builder /build/target/release/efe-agent-runner /usr/local/bin/efe-agent-runner
USER nonroot
EXPOSE 8443 9091
ENTRYPOINT ["/usr/local/bin/efe-agent-runner"]
