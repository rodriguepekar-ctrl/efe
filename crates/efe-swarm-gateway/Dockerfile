# syntax=docker/dockerfile:1
# SPDX-License-Identifier: UNLICENSED
#
# Image de production de la passerelle d'essaim souveraine (efe-swarm-gateway).
# Le contexte de build est la RACINE du workspace (le crate dépend des autres membres) :
#
#   docker build -f crates/efe-swarm-gateway/Dockerfile -t ghcr.io/efe-ai/efe-swarm-gateway:dev .
#
# Le binaire produit est `efe-swarm-gateway` (src/main.rs) : il sélectionne HmacKeySource::EnvHkdf
# dès que EFE_NANO_AUDIT_HMAC_MASTER est présent (signature server-side), bind 0.0.0.0:8080, arrêt
# gracieux SIGTERM. Câble store(postgres)/auth(oidc)/PII/routeur Network depuis l'environnement.

# ── Étage build ───────────────────────────────────────────────────────────────────────────────
FROM rust:1-bookworm AS builder
WORKDIR /build
COPY . .
# Features prod activées ; ajustez selon le déploiement (ex. self-host pur : aucune feature requise).
RUN cargo build --release -p efe-swarm-gateway --bin efe-swarm-gateway --features "postgres oidc pii"

# ── Étage runtime (mince) ───────────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim
# curl : les backends locaux (self-host) ET le routeur (Network) sont appelés par SHELL-OUT curl
#        (keyless, aucune dépendance réseau/TLS dans le crate SDK — garde-fou G10).
# ca-certificates : TLS vers le routeur frontière (mode Network) le cas échéant.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/efe-swarm-gateway /usr/local/bin/efe-swarm-gateway

# Exécution non-root (défense en profondeur).
RUN useradd -r -u 10001 -m efe
USER efe

ENV EFE_SWARM_GATEWAY_ADDR=0.0.0.0:8080
EXPOSE 8080

# Sonde : /metrics répond dès que la passerelle écoute (cf. liveness/readiness du chart Helm).
HEALTHCHECK --interval=15s --timeout=3s --start-period=5s --retries=3 \
  CMD curl -fsS http://127.0.0.1:8080/metrics || exit 1

# AUCUNE clé dans l'image. Le master HKDF (signature) et les secrets OIDC/DB viennent de
# l'environnement / des Secrets Kubernetes au runtime, jamais de l'image.
ENTRYPOINT ["efe-swarm-gateway"]
