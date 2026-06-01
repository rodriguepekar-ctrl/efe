# PR #20 § 22.1 — distroless multi-arch Docker image.
#
# Build :
#     docker buildx build --platform linux/amd64,linux/arm64 \
#         -t ghcr.io/efe-ai/efe-meta-swarm-loadgen:0.1.0 \
#         -f tools/loadgen/Dockerfile \
#         --push .
#
# Run :
#     docker run --rm \
#         -v /etc/efe/tls:/etc/efe/tls:ro \
#         ghcr.io/efe-ai/efe-meta-swarm-loadgen:0.1.0 \
#         run --target grpc://controller:50051 --qps 50 --duration 1m

# ─── Builder stage : full Rust toolchain ──────────────────────────
FROM rust:1.83-slim-bookworm AS builder

# protoc — required by tonic-build for the meta-swarm proto crate.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        protobuf-compiler \
        pkg-config \
        libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependencies separately from sources.  Copy the workspace
# manifests first, then a minimal stub, then run a release build to
# pre-populate the dep cache.
COPY Cargo.toml Cargo.lock ./
COPY crates/efe-meta-swarm-proto/Cargo.toml ./crates/efe-meta-swarm-proto/
COPY tools/loadgen/Cargo.toml ./tools/loadgen/

# Stub the lib/main so `cargo build` picks up deps only.
RUN mkdir -p crates/efe-meta-swarm-proto/src tools/loadgen/src && \
    echo "fn main() {}" > tools/loadgen/src/main.rs && \
    echo "" > tools/loadgen/src/lib.rs && \
    echo "" > crates/efe-meta-swarm-proto/src/lib.rs

# Real build : copy sources.
COPY . .

# Build with release profile + locked lockfile.  Strip happens via the
# workspace profile (Cargo.toml [profile.release]).
ARG EFE_GIT_SHA=unknown
ARG EFE_BUILD_DATE=unknown
ENV EFE_GIT_SHA=${EFE_GIT_SHA} \
    EFE_BUILD_DATE=${EFE_BUILD_DATE} \
    EFE_RUSTC_VERSION=1.83.0
RUN cargo build --locked --release -p efe-meta-swarm-loadgen


# ─── Runtime stage : distroless nonroot ───────────────────────────
FROM gcr.io/distroless/cc-debian12:nonroot

LABEL org.opencontainers.image.title="efe-meta-swarm-loadgen" \
      org.opencontainers.image.description="EFE Phase-2 meta-swarm L2 load generator (PR #20)" \
      org.opencontainers.image.source="https://github.com/efe-ai/efe/tree/main/tools/loadgen" \
      org.opencontainers.image.licenses="UNLICENSED" \
      org.opencontainers.image.vendor="EFE Platform Team"

COPY --from=builder /build/target/release/efe-meta-swarm-loadgen /usr/local/bin/efe-meta-swarm-loadgen

USER nonroot
ENTRYPOINT ["/usr/local/bin/efe-meta-swarm-loadgen"]
CMD ["--help"]
