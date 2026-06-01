// Package ratelimit implements per-(run_id, endpoint_class) token-bucket
// rate limiting with golang.org/x/time/rate.
package ratelimit

import (
	"log/slog"
	"sync"
	"time"

	"golang.org/x/time/rate"
)

// Budgets per minute.
const (
	BudgetLLM        = 30
	BudgetToolRead   = 60
	BudgetToolWrite  = 10
	BudgetAudit      = 200
	BudgetCheckpoint = 5
	BudgetOther      = 60
)

type bucketKey struct {
	runID         string
	endpointClass string
}

// Limiter holds a per-key token bucket. Buckets evict 30 min after the
// last access to keep memory bounded.
type Limiter struct {
	mu      sync.Mutex
	buckets map[bucketKey]*entry
	logger  *slog.Logger
}

type entry struct {
	limiter *rate.Limiter
	last    time.Time
}

func New(logger *slog.Logger) *Limiter {
	l := &Limiter{
		buckets: make(map[bucketKey]*entry),
		logger:  logger,
	}
	go l.evictLoop()
	return l
}

// Allow returns true if a token was available. False = rate-limited.
func (l *Limiter) Allow(runID, endpointClass string) bool {
	l.mu.Lock()
	defer l.mu.Unlock()
	key := bucketKey{runID: runID, endpointClass: endpointClass}
	e, ok := l.buckets[key]
	if !ok {
		budget := budgetFor(endpointClass)
		e = &entry{
			limiter: rate.NewLimiter(rate.Limit(float64(budget)/60.0), budget),
			last:    time.Now(),
		}
		l.buckets[key] = e
	}
	e.last = time.Now()
	return e.limiter.Allow()
}

func budgetFor(class string) int {
	switch class {
	case "llm":
		return BudgetLLM
	case "tool_read":
		return BudgetToolRead
	case "tool_write":
		return BudgetToolWrite
	case "audit":
		return BudgetAudit
	case "checkpoint":
		return BudgetCheckpoint
	default:
		return BudgetOther
	}
}

func (l *Limiter) evictLoop() {
	t := time.NewTicker(5 * time.Minute)
	defer t.Stop()
	for range t.C {
		cutoff := time.Now().Add(-30 * time.Minute)
		l.mu.Lock()
		for k, e := range l.buckets {
			if e.last.Before(cutoff) {
				delete(l.buckets, k)
			}
		}
		count := len(l.buckets)
		l.mu.Unlock()
		l.logger.Debug("ratelimit eviction sweep done", "active_buckets", count)
	}
}
