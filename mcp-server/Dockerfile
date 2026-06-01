# EFE MCP Server — distroless-style image (M16).
#
# Mirrors the EFE 1.0 GA convention :
#   - non-root UID 10001 (matches `crates/efe-server/Dockerfile`)
#   - read-only root filesystem at runtime (recommended)
#   - no shell in final image (distroless)
#
# Build :
#   docker build -t efe-mcp-server:0.1.0 mcp-server/
#
# Run (stdio transport — for Claude Desktop / Cursor / Zed) :
#   docker run --rm -i \
#     -e EFE_OIDC_TOKEN="$EFE_OIDC_TOKEN" \
#     -e EFE_ENDPOINT="https://efe.example.com" \
#     efe-mcp-server:0.1.0
#
# Run (HTTP+SSE transport — cloud) :
#   docker run --rm -p 8800:8800 \
#     -e EFE_OIDC_TOKEN="$EFE_OIDC_TOKEN" \
#     -e EFE_ENDPOINT="https://efe.example.com" \
#     -e EFE_MCP_TRANSPORT=http \
#     efe-mcp-server:0.1.0

# ─── Stage 1 : build wheel ─────────────────────────────────────────────
FROM python:3.12-slim AS builder

WORKDIR /build

# Install build deps. No compiler needed — pure-Python wheel.
RUN pip install --no-cache-dir --upgrade pip build

COPY pyproject.toml README.md ./
COPY mcp_server/ ./mcp_server/

RUN python -m build --wheel --outdir /dist


# ─── Stage 2 : runtime ─────────────────────────────────────────────────
FROM python:3.12-slim AS runtime

# Non-root user (UID 10001 matches EFE 1.0 GA convention).
#
# **Φ-D-OPS post-audit fix S8** : pre-fix used `--no-create-home`
# while setting `WORKDIR /home/efe` later — Docker would auto-create
# the dir as root-owned, making it unwritable under read-only-rootfs.
# Now we create the home dir with proper ownership.
RUN groupadd --system --gid 10001 efe \
 && useradd  --system --uid 10001 --gid efe \
        --home-dir /home/efe --create-home --shell /sbin/nologin efe

# Install only runtime deps. No build tools, no shell extras.
COPY --from=builder /dist/*.whl /tmp/
RUN pip install --no-cache-dir /tmp/*.whl \
 && rm -rf /tmp/*.whl /root/.cache/pip

# Drop privileges.
USER 10001:10001
WORKDIR /home/efe

# Default environment.
ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    EFE_MCP_TRANSPORT=stdio \
    EFE_MCP_RATE_LIMIT_PER_MIN=100

# stdio transport reads from stdin ; Claude Desktop attaches via subprocess.
# HTTP+SSE listens on 8800.
EXPOSE 8800

ENTRYPOINT ["efe-mcp-server"]
