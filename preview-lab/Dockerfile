# SPDX-License-Identifier: UNLICENSED
#
# Image single-binary : le bridge `efe-preview-server` sert l'UI (export Next.js)
# + l'API SSE. Contexte de build = racine du workspace :
#
#   docker build -f preview-lab/Dockerfile -t efe-preview .
#   docker run -p 8700:8700 efe-preview          # mode mock
#
# (Le build Rust compile le workspace — long la première fois.)

# ── Stage 1 : export statique du frontend ───────────────────────────────────
FROM node:20-alpine AS frontend
WORKDIR /app
RUN npm i -g pnpm
COPY preview-lab/package.json ./
RUN pnpm install --ignore-workspace
COPY preview-lab/ ./
RUN pnpm build                      # → /app/out

# ── Stage 2 : build du bridge Rust ──────────────────────────────────────────
FROM rust:1-bookworm AS bridge
WORKDIR /src
COPY . .
RUN cargo build --release -p efe-preview-server

# ── Stage 3 : runtime minimal ───────────────────────────────────────────────
FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/*
COPY --from=bridge /src/target/release/efe-preview-server /usr/local/bin/efe-preview-server
COPY --from=frontend /app/out /app/static
ENV EFE_PREVIEW_MOCK=1 \
    EFE_PREVIEW_STATIC_DIR=/app/static \
    EFE_PREVIEW_ADDR=0.0.0.0:8700 \
    EFE_PREVIEW_CORS_ORIGIN=http://localhost:8700
EXPOSE 8700
CMD ["efe-preview-server"]
