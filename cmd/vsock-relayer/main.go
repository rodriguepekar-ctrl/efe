// vsock-relayer — bridges in-microVM HTTP calls to in-cluster services.
//
// Listens on vsock CID 2 port 9999 ; for each request :
//   1. Validates the JWT bearer (audience efe-agent-relay).
//   2. Looks up the upstream by Host header against an allowlist.
//   3. Applies per-microVM (per run_id) rate limiting.
//   4. Forwards via mTLS to LLM router or mcp-server.
//   5. Streams the response back over vsock.
//   6. Emits an audit relay event (fire-and-forget).
//
// See `docs/EFE_M26_AGENT_RUNTIME_PRODUCTION_ARCHITECTURE.md` § 10.

package main

import (
	"context"
	"errors"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/mdlayher/vsock"

	"github.com/efe-ai/efe/cmd/vsock-relayer/pkg/audit"
	"github.com/efe-ai/efe/cmd/vsock-relayer/pkg/proxy"
	"github.com/efe-ai/efe/cmd/vsock-relayer/pkg/ratelimit"
)

func main() {
	logger := slog.New(slog.NewJSONHandler(os.Stdout, &slog.HandlerOptions{Level: slog.LevelInfo}))
	slog.SetDefault(logger)

	upstreams, err := proxy.LoadUpstreamsFromEnv(logger)
	if err != nil {
		logger.Error("failed to load upstreams", "err", err)
		os.Exit(1)
	}

	jwtPub, err := proxy.LoadJWTPublicKey()
	if err != nil {
		logger.Error("failed to load JWT public key", "err", err)
		os.Exit(1)
	}

	rl := ratelimit.New(logger)
	auditClient := audit.NewClient(logger)

	handler := proxy.NewHandler(upstreams, rl, auditClient, jwtPub, logger)

	port := uint32(9999)
	if v := os.Getenv("EFE_RELAY_VSOCK_PORT"); v != "" {
		// allow override for staging; not normally set.
		_ = v
	}

	listener, err := vsock.Listen(port, nil)
	if err != nil {
		logger.Error("vsock.Listen failed", "port", port, "err", err)
		os.Exit(1)
	}
	defer listener.Close()

	server := &http.Server{
		Handler:           handler,
		ReadHeaderTimeout: 10 * time.Second,
		ReadTimeout:       60 * time.Second,
		WriteTimeout:      120 * time.Second,
		IdleTimeout:       30 * time.Second,
		MaxHeaderBytes:    1 << 16,
		ErrorLog:          slogErrorLog(logger),
	}

	// Health server on a separate localhost TCP port for the K8s probes.
	go func() {
		health := http.NewServeMux()
		health.HandleFunc("/healthz", func(w http.ResponseWriter, _ *http.Request) {
			_, _ = w.Write([]byte("ok"))
		})
		health.HandleFunc("/readyz", func(w http.ResponseWriter, _ *http.Request) {
			if !handler.Ready() {
				http.Error(w, "not ready", http.StatusServiceUnavailable)
				return
			}
			_, _ = w.Write([]byte("ready"))
		})
		hp := os.Getenv("EFE_RELAY_HEALTH_ADDR")
		if hp == "" {
			hp = "0.0.0.0:8080"
		}
		logger.Info("health server listening", "addr", hp)
		if err := http.ListenAndServe(hp, health); err != nil && !errors.Is(err, http.ErrServerClosed) {
			logger.Error("health server failed", "err", err)
		}
	}()

	ctx, cancel := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer cancel()

	go func() {
		<-ctx.Done()
		shutdownCtx, sc := context.WithTimeout(context.Background(), 15*time.Second)
		defer sc()
		_ = server.Shutdown(shutdownCtx)
	}()

	logger.Info("vsock-relayer listening", "vsock_cid", "host", "port", port)
	if err := server.Serve(listener); err != nil && !errors.Is(err, http.ErrServerClosed) {
		logger.Error("server failed", "err", err)
		os.Exit(1)
	}
}

// slogErrorLog adapts slog to the legacy log.Logger that http.Server expects.
func slogErrorLog(logger *slog.Logger) *slogStdLog {
	return &slogStdLog{l: logger}
}

type slogStdLog struct{ l *slog.Logger }

func (s *slogStdLog) Write(p []byte) (int, error) {
	s.l.Error("http server", "msg", string(p))
	return len(p), nil
}
