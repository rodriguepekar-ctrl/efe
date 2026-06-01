// Package audit forwards relay-side audit events to the controller's
// audit append endpoint. Fire-and-forget : never blocks the proxy hot
// path, drops on backpressure.
package audit

import (
	"bytes"
	"context"
	"encoding/json"
	"log/slog"
	"net/http"
	"os"
	"time"
)

type Client struct {
	endpoint string
	queue    chan map[string]any
	http     *http.Client
	logger   *slog.Logger
}

func NewClient(logger *slog.Logger) *Client {
	endpoint := os.Getenv("EFE_RELAY_AUDIT_ENDPOINT")
	if endpoint == "" {
		endpoint = "http://efe-mcp-server.efe-system.svc.cluster.local:8080/v1/audit/append"
	}
	c := &Client{
		endpoint: endpoint,
		queue:    make(chan map[string]any, 1024),
		http:     &http.Client{Timeout: 5 * time.Second},
		logger:   logger,
	}
	go c.worker()
	return c
}

// FireAndForget pushes an event to the async queue. Drops if the queue
// is full (backpressure ; we never block the proxy).
func (c *Client) FireAndForget(payload map[string]any) {
	select {
	case c.queue <- payload:
	default:
		c.logger.Warn("audit queue full, dropping event",
			"event", payload["event_type"])
	}
}

func (c *Client) worker() {
	for ev := range c.queue {
		body, err := json.Marshal(ev)
		if err != nil {
			c.logger.Error("audit marshal failed", "err", err)
			continue
		}
		req, err := http.NewRequestWithContext(
			context.Background(),
			http.MethodPost,
			c.endpoint,
			bytes.NewReader(body),
		)
		if err != nil {
			c.logger.Error("audit req build failed", "err", err)
			continue
		}
		req.Header.Set("Content-Type", "application/json")
		resp, err := c.http.Do(req)
		if err != nil {
			c.logger.Warn("audit upstream failed", "err", err)
			continue
		}
		_ = resp.Body.Close()
	}
}
