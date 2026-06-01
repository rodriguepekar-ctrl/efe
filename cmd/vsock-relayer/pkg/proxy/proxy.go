// Package proxy implements the HTTP forwarder from vsock to in-cluster
// upstream services with mTLS + JWT validation.
package proxy

import (
	"crypto/ecdsa"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httputil"
	"net/url"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/golang-jwt/jwt/v5"

	"github.com/efe-ai/efe/cmd/vsock-relayer/pkg/audit"
	"github.com/efe-ai/efe/cmd/vsock-relayer/pkg/ratelimit"
)

const (
	HeaderRunID  = "X-EFE-Run-Id"
	HeaderTenant = "X-EFE-Tenant"
	HeaderStep   = "X-EFE-Step"
)

// Upstream describes one allowed upstream service.
type Upstream struct {
	HostHeader  string  // value the agent uses in the Host header
	TargetURL   url.URL // resolved upstream URL (https://...)
	Class       string  // "llm" | "mcp"
	ClientCert  string  // path to mTLS client cert
	ClientKey   string  // path to mTLS client key
	CABundle    string  // path to upstream CA
	clientCache *http.Client
}

func (u *Upstream) HTTPClient() (*http.Client, error) {
	if u.clientCache != nil {
		return u.clientCache, nil
	}
	cert, err := tls.LoadX509KeyPair(u.ClientCert, u.ClientKey)
	if err != nil {
		return nil, fmt.Errorf("load mTLS cert : %w", err)
	}
	caPem, err := os.ReadFile(u.CABundle)
	if err != nil {
		return nil, fmt.Errorf("read CA bundle : %w", err)
	}
	caPool := x509.NewCertPool()
	if !caPool.AppendCertsFromPEM(caPem) {
		return nil, errors.New("CA bundle parse failed")
	}
	tr := &http.Transport{
		TLSClientConfig: &tls.Config{
			Certificates: []tls.Certificate{cert},
			RootCAs:      caPool,
			MinVersion:   tls.VersionTLS13,
		},
		MaxIdleConns:        100,
		MaxIdleConnsPerHost: 50,
		IdleConnTimeout:     90 * time.Second,
	}
	u.clientCache = &http.Client{
		Transport: tr,
		Timeout:   60 * time.Second,
	}
	return u.clientCache, nil
}

// Upstreams is a fixed allowlist keyed by Host header.
type Upstreams map[string]*Upstream

func LoadUpstreamsFromEnv(logger *slog.Logger) (Upstreams, error) {
	llm := getEnv("EFE_LLM_ROUTER_URL", "https://efe-llm-router.efe-system.svc.cluster.local:50051")
	mcp := getEnv("EFE_MCP_SERVER_URL", "https://efe-mcp-server.efe-system.svc.cluster.local:8080")

	llmTLS := getEnv("EFE_LLM_CLIENT_TLS_DIR", "/var/run/tls/llm-client")
	mcpTLS := getEnv("EFE_MCP_CLIENT_TLS_DIR", "/var/run/tls/mcp-client")
	caBundle := getEnv("EFE_RELAY_CA_BUNDLE", "/etc/ssl/certs/efe-ca-bundle.pem")

	llmURL, err := url.Parse(llm)
	if err != nil {
		return nil, fmt.Errorf("LLM URL : %w", err)
	}
	mcpURL, err := url.Parse(mcp)
	if err != nil {
		return nil, fmt.Errorf("MCP URL : %w", err)
	}

	upstreams := Upstreams{
		"efe-llm-router.local:50051": &Upstream{
			HostHeader: "efe-llm-router.local:50051",
			TargetURL:  *llmURL,
			Class:      "llm",
			ClientCert: llmTLS + "/tls.crt",
			ClientKey:  llmTLS + "/tls.key",
			CABundle:   caBundle,
		},
		"efe-mcp-server.local:8080": &Upstream{
			HostHeader: "efe-mcp-server.local:8080",
			TargetURL:  *mcpURL,
			Class:      "mcp",
			ClientCert: mcpTLS + "/tls.crt",
			ClientKey:  mcpTLS + "/tls.key",
			CABundle:   caBundle,
		},
	}
	logger.Info("upstreams loaded",
		"llm", llm, "mcp", mcp,
		"llm_tls_dir", llmTLS, "mcp_tls_dir", mcpTLS)
	return upstreams, nil
}

// Handler is the HTTP handler exposed on the vsock listener.
type Handler struct {
	upstreams Upstreams
	rl        *ratelimit.Limiter
	audit     *audit.Client
	jwtPub    *ecdsa.PublicKey
	logger    *slog.Logger
	ready     atomic.Bool
}

func NewHandler(
	upstreams Upstreams,
	rl *ratelimit.Limiter,
	auditClient *audit.Client,
	jwtPub *ecdsa.PublicKey,
	logger *slog.Logger,
) *Handler {
	h := &Handler{
		upstreams: upstreams,
		rl:        rl,
		audit:     auditClient,
		jwtPub:    jwtPub,
		logger:    logger,
	}
	h.ready.Store(true)
	return h
}

func (h *Handler) Ready() bool {
	return h.ready.Load()
}

// Claims is the JWT shape accepted by the relayer.
type Claims struct {
	jwt.RegisteredClaims
	Tenant         string   `json:"tenant"`
	RunID          string   `json:"run_id"`
	ToolsAllowed   []string `json:"tools_allowed,omitempty"`
}

func (h *Handler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	bearer := strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")
	if bearer == "" {
		http.Error(w, "missing bearer", http.StatusUnauthorized)
		return
	}

	tok, err := jwt.ParseWithClaims(bearer, &Claims{}, func(t *jwt.Token) (interface{}, error) {
		if _, ok := t.Method.(*jwt.SigningMethodECDSA); !ok {
			return nil, fmt.Errorf("unexpected algo : %v", t.Header["alg"])
		}
		return h.jwtPub, nil
	}, jwt.WithAudience("efe-agent-relay"))
	if err != nil {
		h.logger.Warn("JWT validation failed", "err", err)
		http.Error(w, "invalid token", http.StatusUnauthorized)
		return
	}
	claims, ok := tok.Claims.(*Claims)
	if !ok || !tok.Valid {
		http.Error(w, "invalid token", http.StatusUnauthorized)
		return
	}

	hostHeader := r.Header.Get("Host")
	if hostHeader == "" {
		hostHeader = r.Host
	}
	upstream, exists := h.upstreams[hostHeader]
	if !exists {
		h.logger.Warn("upstream not in allowlist", "host", hostHeader)
		http.Error(w, "upstream not allowed", http.StatusForbidden)
		return
	}

	endpointClass := classifyEndpoint(upstream.Class, r.URL.Path)
	if !h.rl.Allow(claims.RunID, endpointClass) {
		http.Error(w, "rate limited", http.StatusTooManyRequests)
		return
	}

	// Tool whitelist enforcement (4th layer).
	if upstream.Class == "mcp" && strings.HasPrefix(r.URL.Path, "/v1/tools/") {
		toolName := strings.TrimPrefix(r.URL.Path, "/v1/tools/")
		toolName = strings.SplitN(toolName, "/", 2)[0]
		if !sliceContains(claims.ToolsAllowed, toolName) {
			h.logger.Warn("tool not in JWT tools_allowed",
				"tool", toolName, "run_id", claims.RunID)
			http.Error(w, "tool not allowed", http.StatusForbidden)
			h.audit.FireAndForget(map[string]any{
				"event_type": "tool_call_rejected_relay",
				"run_id":     claims.RunID,
				"tool":       toolName,
			})
			return
		}
	}

	// Forward.
	client, err := upstream.HTTPClient()
	if err != nil {
		h.logger.Error("upstream client init failed", "err", err)
		http.Error(w, "upstream init failed", http.StatusBadGateway)
		return
	}

	target := upstream.TargetURL
	target.Path = r.URL.Path
	target.RawQuery = r.URL.RawQuery

	req, err := http.NewRequestWithContext(r.Context(), r.Method, target.String(), r.Body)
	if err != nil {
		http.Error(w, "build req failed", http.StatusInternalServerError)
		return
	}
	for k, v := range r.Header {
		// Strip the original Authorization (we use a separate one for upstream mTLS).
		if strings.EqualFold(k, "Authorization") {
			continue
		}
		req.Header[k] = v
	}
	req.Header.Set(HeaderRunID, claims.RunID)
	req.Header.Set(HeaderTenant, claims.Tenant)

	resp, err := client.Do(req)
	if err != nil {
		h.logger.Error("upstream call failed", "err", err, "host", hostHeader)
		http.Error(w, "upstream unavailable", http.StatusBadGateway)
		return
	}
	defer resp.Body.Close()

	for k, v := range resp.Header {
		w.Header()[k] = v
	}
	w.WriteHeader(resp.StatusCode)
	_, _ = io.Copy(w, resp.Body)

	h.audit.FireAndForget(map[string]any{
		"event_type": eventTypeFor(upstream.Class),
		"run_id":     claims.RunID,
		"tenant":     claims.Tenant,
		"path":       r.URL.Path,
		"upstream":   hostHeader,
		"status":     resp.StatusCode,
	})
}

func classifyEndpoint(class, path string) string {
	if class == "llm" {
		return "llm"
	}
	if class == "mcp" {
		switch {
		case strings.HasPrefix(path, "/v1/tools/efe_classify"),
			strings.HasPrefix(path, "/v1/tools/efe_query_events"),
			strings.HasPrefix(path, "/v1/tools/web_search"),
			strings.HasPrefix(path, "/v1/tools/fetch_url"):
			return "tool_read"
		case strings.HasPrefix(path, "/v1/tools/"):
			return "tool_write"
		case strings.HasPrefix(path, "/v1/audit/append"):
			return "audit"
		case strings.HasPrefix(path, "/v1/agent/checkpoint"):
			return "checkpoint"
		}
	}
	return "other"
}

func eventTypeFor(class string) string {
	switch class {
	case "llm":
		return "llm_call_completed"
	case "mcp":
		return "tool_call_completed"
	default:
		return "relay_call_completed"
	}
}

func sliceContains(haystack []string, needle string) bool {
	for _, s := range haystack {
		if s == needle {
			return true
		}
	}
	return false
}

// LoadJWTPublicKey loads the ECDSA public key from PEM.
func LoadJWTPublicKey() (*ecdsa.PublicKey, error) {
	pemPath := getEnv("EFE_RELAY_JWT_PUBLIC_KEY_PATH", "/var/run/relay-jwt/pub.pem")
	pemEnv := os.Getenv("EFE_RELAY_JWT_PUBLIC_KEY")
	var data []byte
	if pemEnv != "" {
		data = []byte(pemEnv)
	} else {
		var err error
		data, err = os.ReadFile(pemPath)
		if err != nil {
			return nil, fmt.Errorf("read jwt key : %w", err)
		}
	}
	pub, err := jwt.ParseECPublicKeyFromPEM(data)
	if err != nil {
		return nil, fmt.Errorf("parse jwt key : %w", err)
	}
	return pub, nil
}

func getEnv(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

// helper for httputil-style debug logging if needed in dev
var _ = httputil.DumpRequest
var _ = sync.Mutex{}
var _ = json.Marshal
