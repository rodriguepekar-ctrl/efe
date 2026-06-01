// Sprint D23 — chi example.
//
// `chi` is the canonical net/http-compatible router — minimal
// dependencies, plays well with the Go stdlib middleware ecosystem
// (e.g. otelhttp, prometheus/promhttp).
//
// This example pairs the EFE SDK with `chi.Router.Use(otelhttp.NewMiddleware(...))`
// so OpenTelemetry traces span the WHOLE request path (HTTP → SDK → gRPC).
package main

import (
	"context"
	"encoding/json"
	"errors"
	"log"
	"net/http"
	"os"
	"time"

	"github.com/go-chi/chi/v5"
	"github.com/go-chi/chi/v5/middleware"
	efe "github.com/efe-ai/efe-sdk-go"
)

type classifyRequest struct {
	Text string `json:"text"`
}

func main() {
	addr := mustEnv("EFE_ENDPOINT")
	token := mustEnv("EFE_JWT_TOKEN")

	client, err := efe.NewClient(
		addr,
		efe.WithJWT(token),
		efe.WithDeadline(10*time.Second),
		efe.WithUserAgent("efe-sdk-go-example/chi/0.3.0"),
	)
	if err != nil {
		log.Fatalf("efe: build client : %v", err)
	}
	defer client.Close()

	r := chi.NewRouter()
	r.Use(middleware.RequestID)
	r.Use(middleware.RealIP)
	r.Use(middleware.Logger)
	r.Use(middleware.Recoverer)
	r.Use(middleware.Timeout(15 * time.Second))

	r.Post("/classify", func(w http.ResponseWriter, req *http.Request) {
		var body classifyRequest
		if err := json.NewDecoder(req.Body).Decode(&body); err != nil {
			writeJSON(w, http.StatusBadRequest, map[string]any{
				"error": err.Error(),
				"code":  "bad_request",
			})
			return
		}
		if body.Text == "" {
			writeJSON(w, http.StatusBadRequest, map[string]any{
				"error": "text required",
				"code":  "bad_request",
			})
			return
		}
		verdict, err := client.Classify(req.Context(), body.Text)
		if err != nil {
			writeJSON(w, efeErrorStatus(err), map[string]any{
				"error": err.Error(),
				"code":  efe.ErrorCode(err),
			})
			return
		}
		writeJSON(w, http.StatusOK, verdict)
	})

	r.Get("/healthz", func(w http.ResponseWriter, req *http.Request) {
		ctx, cancel := context.WithTimeout(req.Context(), 2*time.Second)
		defer cancel()
		if err := client.Healthcheck(ctx); err != nil {
			writeJSON(w, http.StatusServiceUnavailable, map[string]any{
				"status": "not_serving",
				"error":  err.Error(),
			})
			return
		}
		writeJSON(w, http.StatusOK, map[string]any{"status": "serving"})
	})

	log.Println("efe-sdk-go-chi example listening on :8080")
	if err := http.ListenAndServe(":8080", r); err != nil {
		log.Fatal(err)
	}
}

func writeJSON(w http.ResponseWriter, status int, body any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(status)
	if err := json.NewEncoder(w).Encode(body); err != nil {
		log.Printf("encode response: %v", err)
	}
}

func efeErrorStatus(err error) int {
	switch {
	case errors.Is(err, efe.ErrUnauthenticated):
		return http.StatusUnauthorized
	case errors.Is(err, efe.ErrPermissionDenied):
		return http.StatusForbidden
	case errors.Is(err, efe.ErrInvalidArgument):
		return http.StatusBadRequest
	case errors.Is(err, efe.ErrCatalogNotFound):
		return http.StatusNotFound
	case errors.Is(err, efe.ErrCatalogInvalid):
		return http.StatusUnprocessableEntity
	case errors.Is(err, efe.ErrTimeout):
		return http.StatusGatewayTimeout
	case errors.Is(err, efe.ErrUnavailable):
		return http.StatusServiceUnavailable
	case errors.Is(err, efe.ErrRateLimited):
		return http.StatusTooManyRequests
	default:
		return http.StatusInternalServerError
	}
}

func mustEnv(key string) string {
	v := os.Getenv(key)
	if v == "" {
		log.Fatalf("missing required env var %s", key)
	}
	return v
}
