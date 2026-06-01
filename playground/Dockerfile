# EFE Playground — Docker image (single-container, all-in-one).
#
# Build :
#   docker build -t efe-playground:1.0 -f playground/Dockerfile .
#
# Run (hash fallback embedder, no model download) :
#   docker run -p 8765:8765 efe-playground:1.0
#
# Run with real semantic embedder (~500 MiB image) :
#   docker build --build-arg INSTALL_REAL_EMBEDDER=yes \
#       -t efe-playground:1.0-real -f playground/Dockerfile .

FROM python:3.12-slim

# Build arg : whether to install sentence-transformers (heavy)
ARG INSTALL_REAL_EMBEDDER=no

# OS deps for pip install + curl for healthcheck
RUN apt-get update && \
    apt-get install -y --no-install-recommends curl && \
    rm -rf /var/lib/apt/lists/*

# Non-root user
RUN groupadd --system --gid 10001 efe && \
    useradd --system --uid 10001 --gid 10001 --no-create-home efe

WORKDIR /app

# Copy backend + frontend + samples
COPY playground/backend/ /app/backend/
COPY playground/frontend/ /app/frontend/
COPY playground/samples/ /app/samples/
COPY playground/tutorials/ /app/tutorials/

# Install Python deps
RUN pip install --no-cache-dir --upgrade pip && \
    pip install --no-cache-dir -r /app/backend/requirements.txt

# Optional : real semantic embedder (heavy)
RUN if [ "$INSTALL_REAL_EMBEDDER" = "yes" ]; then \
        pip install --no-cache-dir "sentence-transformers>=2.2,<3.0" "torch>=2.0,<3.0"; \
    fi

# Drop privileges
USER efe

EXPOSE 8765

HEALTHCHECK --interval=10s --timeout=5s --start-period=5s --retries=3 \
    CMD curl -fsSL http://localhost:8765/health || exit 1

CMD ["python3", "/app/backend/app.py", "--host", "0.0.0.0", "--port", "8765"]
