# EFE M25 — Agent Core image (runs inside Firecracker microVM).
#
# Multi-stage minimal Python build. Final image is distroless : only
# python + agent_core code + curated allow-listed deps. Cosign-signed
# in CI ; pinned by digest in the image whitelist.
#
# **Φ-D-OPS post-audit fix S6** : the runtime base
# `gcr.io/distroless/python3-debian12:nonroot` ships Python **3.11.2**
# (verified : `docker run …:nonroot --entrypoint /usr/bin/python3
# --version` → `Python 3.11.2`). The pre-fix builder used
# `python:3.12-slim` and PYTHONPATH pointed at `python3.12/site-packages`,
# which doesn't exist in the runtime → `ImportError` at first import.
# Builder is now pinned to 3.11 to match the runtime.

FROM python:3.11-slim AS builder
WORKDIR /build
COPY pyproject.toml README.md ./
COPY agent_core ./agent_core
RUN pip install --no-cache-dir --upgrade pip wheel \
 && pip install --no-cache-dir --user .

FROM gcr.io/distroless/python3-debian12:nonroot AS final
LABEL org.opencontainers.image.source="https://github.com/efe-ai/efe"
LABEL org.opencontainers.image.title="efe-agent-core"
LABEL org.opencontainers.image.description="EFE M25 agent core (runs inside Firecracker microVM)"
LABEL org.opencontainers.image.version="2.0.0"
LABEL org.opencontainers.image.licenses="Apache-2.0"
# The builder installed deps with `--user` so they live under
# /root/.local — copy them into the nonroot home dir.
COPY --from=builder /root/.local /home/nonroot/.local
COPY agent_core /home/nonroot/agent_core
ENV PYTHONPATH=/home/nonroot:/home/nonroot/.local/lib/python3.11/site-packages
ENV PATH=/home/nonroot/.local/bin:$PATH
USER nonroot
ENTRYPOINT ["python", "-m", "agent_core.main"]
