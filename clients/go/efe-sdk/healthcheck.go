// Sprint D23 (Φ-B) — Healthcheck via gRPC standard `Health` service.
//
// Probes the server's `grpc.health.v1.Health/Check` RPC, mapping
// the response status to a Go error. Used by HTTP framework
// integrations to build `/healthz` endpoints (cf. examples/).
//
// Idempotent + retry-eligible (uses the canonical retry interceptor).

package efe

import (
	"context"
	"fmt"

	"google.golang.org/grpc/health/grpc_health_v1"
)

// Healthcheck probes the server's gRPC Health endpoint.
//
// Returns nil when the server reports SERVING ; non-nil error in
// every other case (NOT_SERVING, UNKNOWN, transport error, timeout).
//
// The returned error wraps [ErrUnavailable] (server unreachable),
// [ErrTimeout] (deadline reached), or a generic descriptor when the
// server returns NOT_SERVING.
//
// Use it in HTTP frameworks :
//
//	http.HandleFunc("/healthz", func(w http.ResponseWriter, r *http.Request) {
//	    if err := client.Healthcheck(r.Context()); err != nil {
//	        http.Error(w, err.Error(), http.StatusServiceUnavailable)
//	        return
//	    }
//	    w.Write([]byte("ok"))
//	})
func (c *EfeClient) Healthcheck(ctx context.Context) error {
	if err := c.guardClosed(); err != nil {
		return err
	}
	ctx, cancel := c.applyDeadline(ctx)
	defer cancel()

	healthClient := grpc_health_v1.NewHealthClient(c.conn)
	req := &grpc_health_v1.HealthCheckRequest{
		// Empty Service = the server's overall health (canonical
		// gRPC convention). Pass "efe.v1.SwarmService" to probe
		// only that specific service's health.
		Service: "",
	}
	resp, err := healthClient.Check(ctx, req)
	if err != nil {
		return fromGRPC(err)
	}
	if resp.Status != grpc_health_v1.HealthCheckResponse_SERVING {
		return fmt.Errorf(
			"%w: server reports status=%s",
			ErrUnavailable, resp.Status.String(),
		)
	}
	return nil
}
