// Sprint D23 — gin example.
//
// Demonstrates the canonical EFE SDK integration in a gin handler :
//   - Boot an [efe.EfeClient] from env (EFE_ENDPOINT + EFE_JWT_TOKEN).
//   - Wire a POST /classify endpoint that forwards the input text.
//   - Wire a GET /healthz endpoint that probes the gRPC health.
//   - Translate SDK sentinel errors into HTTP status codes.
//
// Run :
//
//	cd clients/go/efe-sdk/examples/gin
//	EFE_ENDPOINT=staging.efe.ai:50051 \
//	EFE_JWT_TOKEN=<token> \
//	go run main.go
//
// Test :
//
//	curl -X POST http://localhost:8080/classify \
//	    -H 'Content-Type: application/json' \
//	    -d '{"text": "I want a refund"}'
//	curl http://localhost:8080/healthz
package main

import (
	"context"
	"errors"
	"log"
	"net/http"
	"os"
	"time"

	"github.com/gin-gonic/gin"
	efe "github.com/efe-ai/efe-sdk-go"
)

type classifyRequest struct {
	Text string `json:"text" binding:"required"`
}

func main() {
	addr := mustEnv("EFE_ENDPOINT")
	token := mustEnv("EFE_JWT_TOKEN")

	client, err := efe.NewClient(
		addr,
		efe.WithJWT(token),
		efe.WithDeadline(10*time.Second),
		efe.WithUserAgent("efe-sdk-go-example/gin/0.3.0"),
	)
	if err != nil {
		log.Fatalf("efe: build client : %v", err)
	}
	defer client.Close()

	r := gin.Default()

	// POST /classify — forward `text` to EFE, return Verdict as JSON.
	r.POST("/classify", func(c *gin.Context) {
		var req classifyRequest
		if err := c.ShouldBindJSON(&req); err != nil {
			c.JSON(http.StatusBadRequest, gin.H{
				"error": err.Error(),
				"code":  "bad_request",
			})
			return
		}
		verdict, err := client.Classify(c.Request.Context(), req.Text)
		if err != nil {
			c.JSON(efeErrorStatus(err), gin.H{
				"error": err.Error(),
				"code":  efe.ErrorCode(err),
			})
			return
		}
		c.JSON(http.StatusOK, verdict)
	})

	// GET /healthz — probe the EFE server.
	r.GET("/healthz", func(c *gin.Context) {
		ctx, cancel := context.WithTimeout(c.Request.Context(), 2*time.Second)
		defer cancel()
		if err := client.Healthcheck(ctx); err != nil {
			c.JSON(http.StatusServiceUnavailable, gin.H{
				"status": "not_serving",
				"error":  err.Error(),
			})
			return
		}
		c.JSON(http.StatusOK, gin.H{"status": "serving"})
	})

	log.Println("efe-sdk-go-gin example listening on :8080")
	if err := r.Run(":8080"); err != nil {
		log.Fatal(err)
	}
}

// efeErrorStatus maps SDK sentinel errors → HTTP status codes.
//
// Stable mapping :
//   ErrUnauthenticated  → 401
//   ErrPermissionDenied → 403
//   ErrInvalidArgument  → 400
//   ErrCatalogNotFound  → 404
//   ErrCatalogInvalid   → 422
//   ErrTimeout          → 504
//   ErrUnavailable      → 503
//   ErrRateLimited      → 429
//   ErrInternal         → 500
//   default             → 500
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
