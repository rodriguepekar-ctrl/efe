# EFE Portal Admin — multi-stage build.
# Final image : gcr.io/distroless/nodejs22-debian12:nonroot, ~ 180-200 MB.

# ─── Stage 1 : install deps + build ─────────────────────────────────
FROM node:22-alpine AS builder
WORKDIR /app

# Install pnpm via corepack.
RUN corepack enable && corepack prepare pnpm@9.7.0 --activate

# Copy lockfile + manifests for cached dep install.
COPY package.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile

# Copy source.
COPY . .

# Build (output : standalone for distroless runtime).
ARG NEXT_PUBLIC_VERSION=dev
ENV NEXT_TELEMETRY_DISABLED=1
ENV NODE_ENV=production
RUN pnpm build

# ─── Stage 2 : distroless runtime ───────────────────────────────────
FROM gcr.io/distroless/nodejs22-debian12:nonroot AS runtime
WORKDIR /app

ENV NODE_ENV=production
ENV PORT=3000
ENV NEXT_TELEMETRY_DISABLED=1

# Copy standalone build (Next.js ships its own minimal node_modules).
COPY --from=builder --chown=nonroot:nonroot /app/.next/standalone ./
COPY --from=builder --chown=nonroot:nonroot /app/.next/static ./.next/static
COPY --from=builder --chown=nonroot:nonroot /app/public ./public

USER nonroot
EXPOSE 3000

# Distroless contains node binary at /nodejs/bin/node.
ENTRYPOINT ["/nodejs/bin/node", "server.js"]
