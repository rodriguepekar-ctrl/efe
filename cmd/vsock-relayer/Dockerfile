# vsock-relayer multi-stage build.
#
# Stage 1 : compile static binary.
# Stage 2 : distroless base + binary.

FROM golang:1.22-bookworm AS builder
WORKDIR /src
COPY cmd/vsock-relayer/go.mod cmd/vsock-relayer/go.sum* ./cmd/vsock-relayer/
WORKDIR /src/cmd/vsock-relayer
RUN go mod download || true
COPY cmd/vsock-relayer ./
ENV CGO_ENABLED=0 GOOS=linux GOARCH=amd64
RUN go build -trimpath -ldflags="-s -w" -o /out/vsock-relayer .

FROM gcr.io/distroless/static-debian12:nonroot
LABEL org.opencontainers.image.source="https://github.com/efe-ai/efe"
LABEL org.opencontainers.image.title="efe-vsock-relayer"
LABEL org.opencontainers.image.description="EFE M26 vsock-relayer (privileged DaemonSet)"
LABEL org.opencontainers.image.version="2.1.0"
LABEL org.opencontainers.image.licenses="Apache-2.0"
COPY --from=builder /out/vsock-relayer /usr/local/bin/vsock-relayer
USER nonroot
ENTRYPOINT ["/usr/local/bin/vsock-relayer"]
