// Sprint D23 (Φ-B) — mTLS configuration helper.
//
// Loads X.509 cert + private key + CA pool from disk, builds a
// `*tls.Config` honouring TLS 1.3 minimum (matches Lettre E D22
// server-side enforcement).
//
// Trust boundary :
//   - All file reads happen at NewClient time. Errors surface at
//     construction (vs. at first RPC), so the operator catches
//     bad paths early.
//   - The private key is held in memory inside `tls.Certificate` ;
//     we don't redact it from logs because the logger never sees
//     `tls.Certificate` (only the `*mtlsConfig` opaque struct,
//     stringified to "<MTLS>").

package efe

import (
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"os"

	"google.golang.org/grpc/credentials"
)

// mtlsConfig holds the parsed mTLS material. Internal — exposed
// to the gRPC layer via toCredentials().
type mtlsConfig struct {
	cert       tls.Certificate
	rootCAs    *x509.CertPool
	serverName string
}

// loadMTLS parses the 3 PEM files and returns a ready-to-use
// mtlsConfig.
//
// Path layout (canonical Lettre E deployment) :
//   - certPath = /etc/efe/client/tls.crt
//   - keyPath  = /etc/efe/client/tls.key
//   - caPath   = /etc/efe/client/ca.crt
func loadMTLS(certPath, keyPath, caPath string) (*mtlsConfig, error) {
	if certPath == "" || keyPath == "" || caPath == "" {
		return nil, fmt.Errorf("certPath / keyPath / caPath must all be non-empty")
	}
	cert, err := tls.LoadX509KeyPair(certPath, keyPath)
	if err != nil {
		return nil, fmt.Errorf("load cert+key from %s + %s : %w", certPath, keyPath, err)
	}
	caPEM, err := os.ReadFile(caPath)
	if err != nil {
		return nil, fmt.Errorf("read CA from %s : %w", caPath, err)
	}
	pool := x509.NewCertPool()
	if !pool.AppendCertsFromPEM(caPEM) {
		return nil, fmt.Errorf("CA file %s contains no valid certificates", caPath)
	}
	return &mtlsConfig{cert: cert, rootCAs: pool}, nil
}

// String returns "<MTLS>" — placeholder used by the slog handler
// when the mtlsConfig is logged accidentally. Never returns
// the cert / key bytes.
func (m *mtlsConfig) String() string { return "<MTLS>" }

// toCredentials builds the gRPC TransportCredentials for this
// mTLS config. Enforces TLS 1.3 (matches D22 server enforcement).
func (m *mtlsConfig) toCredentials() credentials.TransportCredentials {
	return credentials.NewTLS(&tls.Config{
		Certificates: []tls.Certificate{m.cert},
		RootCAs:      m.rootCAs,
		ServerName:   m.serverName,
		MinVersion:   tls.VersionTLS13,
	})
}

// resolveTLSCredentials picks the canonical TransportCredentials
// for an EfeClient based on the option set :
//
//   1. Explicit `tlsConfig` (from WithTLS) wins.
//   2. mTLS config (from WithMTLS) — TLS 1.3.
//   3. `insecureSkipVerify` (DEV ONLY) — disables verification.
//   4. Default : TLS with system roots, MinVersion 1.2.
//
// Caller passes the address so we can heuristic-detect plain h2c
// (`grpc://` / `:50051`) vs TLS (`https://` / `:443`).
func resolveTLSCredentials(o *options) credentials.TransportCredentials {
	if o.tlsConfig != nil {
		return credentials.NewTLS(o.tlsConfig)
	}
	if o.mtls != nil {
		// Apply the SNI override if WithTLS-style server name was
		// also set.
		if o.tlsServerName != "" {
			o.mtls.serverName = o.tlsServerName
		}
		return o.mtls.toCredentials()
	}
	if o.insecureSkipVerify {
		// DEV ONLY — emit a UserAgent suffix so server logs surface it.
		return credentials.NewTLS(&tls.Config{InsecureSkipVerify: true}) //nolint:gosec
	}
	// Default — TLS with system root pool, MinVersion 1.2.
	return credentials.NewTLS(&tls.Config{
		MinVersion: tls.VersionTLS12,
		ServerName: o.tlsServerName,
	})
}
