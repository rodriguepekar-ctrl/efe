// Sprint D23 (Φ-B) — Classify + ClassifyStream RPCs.
//
// Classify is the SDK's PRIMARY entry point — what 99% of consumers
// will call. It wraps the gRPC unary RPC `efe.v1.SwarmService.Execute`
// behind an idiomatic Go signature.
//
// ClassifyStream is the batch variant : a channel of texts → a
// channel of (verdict, error) pairs. Order-preserved via Index.
//
// Both paths run through the canonical interceptor chain (auth +
// otel + prom + retry) — no per-RPC overrides.

package efe

import (
	"context"
	"fmt"
	"strings"
	"sync"
	"time"

	pb "github.com/efe-ai/efe-sdk-go/internal/proto/efe/v1"
)

// ─────────────────────────────────────────────────────────────────
// Classify — unary
// ─────────────────────────────────────────────────────────────────

// Classify sends one classification request to the EFE server.
//
// On success, returns a *Verdict — fields filled from the gRPC
// response. On error, returns nil and a wrapped sentinel error
// (use errors.Is to discriminate, see [ErrUnauthenticated],
// [ErrTimeout], etc.).
//
// Context handling :
//   - If ctx has a deadline, it wins.
//   - Else, the SDK applies [WithDeadline] (default 10 s).
//   - If neither, the call may block indefinitely on a hung server
//     (ANTI-PATTERN — always pass a deadline-bearing ctx in prod).
//
// Concurrency : safe to call from multiple goroutines.
func (c *EfeClient) Classify(ctx context.Context, text string) (*Verdict, error) {
	if err := c.guardClosed(); err != nil {
		return nil, err
	}
	if strings.TrimSpace(text) == "" {
		return nil, fmt.Errorf("%w: empty text", ErrInvalidArgument)
	}

	ctx, cancel := c.applyDeadline(ctx)
	defer cancel()

	start := time.Now()
	req := &pb.ExecuteRequest{
		RawPrompt: text,
		// SDK does not synthesise a SwarmSpec — the server applies
		// the catalog default. Advanced callers can subclass in
		// post-D23 follow-ups.
	}
	resp, err := c.swarm.Execute(ctx, req)
	elapsed := time.Since(start)
	if err != nil {
		return nil, fromGRPC(err)
	}
	v := verdictFromExecuteResponse(resp, elapsed)
	c.opts.logger.Debug(
		"efe-sdk-go: Classify ok",
		"label", v.Label,
		"confidence", v.Confidence,
		"rounds", v.Rounds,
		"elapsed_ms", v.ElapsedMs,
	)
	return v, nil
}

// verdictFromExecuteResponse projects the wire-shape ExecuteResponse
// into the public Verdict struct.
//
// Field mapping :
//   - Label           ← Output.Text (proto-side label string ; for
//                       MVP the canonical "label" comes from the
//                       output.text since LETTER_E §5 simplified
//                       the wire shape).
//   - Confidence      ← Confidence.
//   - Rounds          ← RoundsExecuted.
//   - AlignmentDelta  ← AlignmentDelta.
//   - SwarmID         ← SwarmId.
//   - RequestID       ← RequestId.
//   - Reject          ← derived from Verdict enum (DIVERGENCE / MODE_COLLAPSE).
//   - Catalog, Zone   ← derived from server-side metadata. For
//                       D23 baseline these may be empty until the
//                       server populates them — pin tested.
//   - ElapsedMs       ← computed client-side (start..return).
func verdictFromExecuteResponse(resp *pb.ExecuteResponse, elapsed time.Duration) *Verdict {
	if resp == nil {
		return &Verdict{ElapsedMs: elapsed.Milliseconds()}
	}
	v := &Verdict{
		Confidence:     float64(resp.Confidence),
		Rounds:         int(resp.RoundsExecuted),
		AlignmentDelta: float64(resp.AlignmentDelta),
		SwarmID:        resp.SwarmId,
		RequestID:      resp.RequestId,
		ElapsedMs:      elapsed.Milliseconds(),
	}
	if resp.Output != nil {
		v.Label = resp.Output.Text
	}
	switch resp.Verdict {
	case pb.Verdict_VERDICT_DIVERGENCE, pb.Verdict_VERDICT_MODE_COLLAPSE:
		v.Reject = true
		v.Zone = "reject"
	case pb.Verdict_VERDICT_CONVERGED:
		v.Zone = deriveZone(v.Confidence)
	case pb.Verdict_VERDICT_CONTINUE:
		// Server timed out without converging — surface as warn.
		v.Zone = "warn"
	default:
		v.Zone = "warn"
	}
	return v
}

// deriveZone applies the canonical thresholds (LETTER_D §5.3).
// Without server-pushed thresholds (current D23 baseline), we use
// 0.80 / 0.50 as soft default.
func deriveZone(confidence float64) string {
	switch {
	case confidence >= 0.80:
		return "trusted"
	case confidence >= 0.50:
		return "warn"
	default:
		return "reject"
	}
}

// ─────────────────────────────────────────────────────────────────
// ClassifyStream — concurrent batch
// ─────────────────────────────────────────────────────────────────

// ClassifyStream takes a channel of input texts and returns a
// channel of [StreamItem] verdicts. Ordering is preserved via the
// Index field — Index reflects the position of the text in the input
// channel (0 for the first text sent, 1 for the second, ...).
//
// Architecture : a single DISPATCHER goroutine reads from `texts`
// and writes (idx, text) pairs to a fan-out channel. A pool of
// worker goroutines pulls from the fan-out channel, calls Classify,
// and emits StreamItems on the output channel.
//
// The dispatcher pattern is intentional — **audit fix M9-A3** : a
// previous implementation had each worker pull from `texts` directly
// and increment a shared mutex-protected counter. That allowed a
// race where Worker A reads text 0 from the channel but is preempted
// before grabbing the counter ; Worker B reads text 1 AND grabs
// counter 0 → text 0 received Index 1, text 1 received Index 0.
//
// Closing the input channel cleanly shuts down the pool ; the output
// channel is closed once all in-flight work drains.
//
// Concurrency : the worker pool size is fixed at 8. Callers who
// need higher / lower can run multiple ClassifyStream instances
// side-by-side.
func (c *EfeClient) ClassifyStream(
	ctx context.Context,
	texts <-chan string,
) (<-chan StreamItem, error) {
	if err := c.guardClosed(); err != nil {
		return nil, err
	}
	out := make(chan StreamItem, 64)

	const concurrency = 8

	// indexedText pairs an input text with its DISPATCHER-assigned
	// index. The dispatcher assigns indices in strict input-order ;
	// workers preserve them.
	type indexedText struct {
		idx  int
		text string
	}
	fanout := make(chan indexedText, concurrency*2)

	// Dispatcher : single goroutine, monotonic counter, no mutex.
	go func() {
		defer close(fanout)
		var idx int
		for {
			select {
			case <-ctx.Done():
				return
			case text, ok := <-texts:
				if !ok {
					return
				}
				select {
				case <-ctx.Done():
					return
				case fanout <- indexedText{idx: idx, text: text}:
					idx++
				}
			}
		}
	}()

	// Worker pool : N goroutines consume from fanout.
	go func() {
		defer close(out)
		var wg sync.WaitGroup
		for w := 0; w < concurrency; w++ {
			wg.Add(1)
			go func() {
				defer wg.Done()
				for {
					select {
					case <-ctx.Done():
						return
					case it, ok := <-fanout:
						if !ok {
							return
						}
						verdict, err := c.Classify(ctx, it.text)
						select {
						case <-ctx.Done():
							return
						case out <- StreamItem{Index: it.idx, Verdict: verdict, Err: err}:
						}
					}
				}
			}()
		}
		wg.Wait()
	}()
	return out, nil
}
