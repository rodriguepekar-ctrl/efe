// Sprint D23 — fiber example.
//
// Same handler shape as the gin example, on top of fiber's
// fasthttp-based engine. Highlights :
//   - `fiber.Ctx` is short-lived ; `c.Context()` returns the
//     stdlib context.Context for SDK consumption.
//   - JSON encoding goes through fiber's pre-allocated buffer
//     pool (zero alloc on hot path).
package main

import (
	"context"
	"errors"
	"log"
	"net/http"
	"os"
	"time"

	"github.com/gofiber/fiber/v2"
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
		efe.WithUserAgent("efe-sdk-go-example/fiber/0.3.0"),
	)
	if err != nil {
		log.Fatalf("efe: build client : %v", err)
	}
	defer client.Close()

	app := fiber.New(fiber.Config{
		// Disable the default startup banner — keeps stdout
		// pristine for log aggregation.
		DisableStartupMessage: true,
	})

	app.Post("/classify", func(c *fiber.Ctx) error {
		var req classifyRequest
		if err := c.BodyParser(&req); err != nil {
			return c.Status(http.StatusBadRequest).JSON(fiber.Map{
				"error": err.Error(),
				"code":  "bad_request",
			})
		}
		if req.Text == "" {
			return c.Status(http.StatusBadRequest).JSON(fiber.Map{
				"error": "text required",
				"code":  "bad_request",
			})
		}
		verdict, err := client.Classify(c.Context(), req.Text)
		if err != nil {
			return c.Status(efeErrorStatus(err)).JSON(fiber.Map{
				"error": err.Error(),
				"code":  efe.ErrorCode(err),
			})
		}
		return c.JSON(verdict)
	})

	app.Get("/healthz", func(c *fiber.Ctx) error {
		ctx, cancel := context.WithTimeout(c.Context(), 2*time.Second)
		defer cancel()
		if err := client.Healthcheck(ctx); err != nil {
			return c.Status(http.StatusServiceUnavailable).JSON(fiber.Map{
				"status": "not_serving",
				"error":  err.Error(),
			})
		}
		return c.JSON(fiber.Map{"status": "serving"})
	})

	log.Println("efe-sdk-go-fiber example listening on :8080")
	if err := app.Listen(":8080"); err != nil {
		log.Fatal(err)
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
