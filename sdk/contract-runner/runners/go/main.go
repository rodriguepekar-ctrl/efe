// Sprint **Φ-C-SDK Completion (D30)** — Go SDK contract runner.
//
// Cf. docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md § 5 + artefact #12.
//
// Reads /fixtures/quartet_fixtures.json, calls the Go SDK gRPC client
// for each fixture, writes verdicts to /shared/out_go.json.
//
// Strategy : we use the live gRPC EfeClient (= clients/go/efe-sdk) when
// connected to a real backend. For local/CI testing without backend we
// fall back to a mock mode replicating the D25 Φ-C-FFI synthesise
// formula in Go for byte-identical cross-SDK comparison.
//
// Mock mode is enabled via env EFE_CONTRACT_MOCK=1.
//
// Build :
//   cd sdk/contract-runner/runners/go && go build -o runner-go .
//
// Run :
//   ./runner-go --fixtures /fixtures/quartet_fixtures.json \
//               --out /shared/out_go.json \
//               --host efe-server:50051

package main

import (
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"math"
	"os"
)

// Fixture matches the JSON schema produced by generate_fixtures.py.
type Fixture struct {
	ID        string `json:"id"`
	Text      string `json:"text"`
	CatalogID string `json:"catalog_id"`
	Seed      uint64 `json:"seed"`
}

// FixturePayload is the root JSON object.
type FixturePayload struct {
	Version   string    `json:"version"`
	Count     int       `json:"count"`
	CatalogID string    `json:"catalog_id"`
	Fixtures  []Fixture `json:"fixtures"`
}

// SubScore is one alternative label/confidence pair.
type SubScore struct {
	Label      string  `json:"label"`
	Confidence float32 `json:"confidence"`
}

// Verdict is the output format consumed by compare.py.
type Verdict struct {
	FixtureID       string     `json:"fixture_id"`
	Label           string     `json:"label"`
	Confidence      float64    `json:"confidence"` // serialised as JSON number (= f64 precision)
	ConeID          string     `json:"cone_id"`
	RoundsExecuted  uint32     `json:"rounds_executed"`
	SwarmID         string     `json:"swarm_id"`
	CatalogVersion  string     `json:"catalog_version"`
	SubScores       []SubScore `json:"sub_scores"`
	LatencyMS       float64    `json:"latency_ms"`
}

// OutputPayload is what we write to /shared/out_go.json.
type OutputPayload struct {
	SDK      string    `json:"sdk"`
	Version  string    `json:"version"`
	Count    int       `json:"count"`
	Verdicts []Verdict `json:"verdicts"`
}

// Default catalog labels used in mock mode (= match Φ-C-FFI fixture
// generator + clients/python/efe/tests/fixtures/catalog.json).
var defaultLabels = []string{
	"billing-dispute",
	"tech-support",
	"refund-request",
	"general-inquiry",
	"other",
}

const defaultCatalogVersion = "1.0.0"
const defaultBundleID = "test-bundle"
const dim = 384

// mockVerdict replicates the D25 stub formula bit-identical to
// Rust efe-ffi controller::synthesise_verdict + bundle::encode.
//
// Critical : all arithmetic is in float32 (= match Rust f32). The
// embedding bytes are little-endian f32 (= match Rust to_le_bytes()).
func mockVerdict(f Fixture) Verdict {
	// Step 1 : encode(text) — SHA-256 seed → f32 vector → L2 normalize.
	seedHasher := sha256.New()
	seedHasher.Write([]byte(f.Text))
	seedHasher.Write([]byte(defaultBundleID))
	seed := seedHasher.Sum(nil) // 32 bytes

	emb := make([]float32, dim)
	for i := 0; i < dim; i++ {
		emb[i] = float32(seed[i%32])/float32(255.0) - float32(0.5)
	}
	// f32 linear sum (= match Rust .sum::<f32>()).
	var sumSq float32
	for _, v := range emb {
		sumSq += v * v
	}
	norm := float32(math.Sqrt(float64(sumSq)))
	if norm > 0.0 {
		for i := range emb {
			emb[i] /= norm
		}
	}

	// Step 2 : SHA-256(emb_bytes || catalog_id || seed_u64_le).
	h := sha256.New()
	for _, v := range emb {
		var buf [4]byte
		binary.LittleEndian.PutUint32(buf[:], math.Float32bits(v))
		h.Write(buf[:])
	}
	h.Write([]byte(f.CatalogID))
	var seedBuf [8]byte
	binary.LittleEndian.PutUint64(seedBuf[:], f.Seed)
	h.Write(seedBuf[:])
	digest := h.Sum(nil)

	labelIdx := binary.LittleEndian.Uint32(digest[0:4]) % uint32(len(defaultLabels))
	label := defaultLabels[labelIdx]

	confU32 := binary.LittleEndian.Uint32(digest[4:8])
	// Match Rust : (conf_u32 as f32) / (u32::MAX as f32).
	confidenceF32 := float32(confU32) / float32(math.MaxUint32)

	rounds := uint32(4) + uint32(digest[8]%16)
	swarmID := "swarm-" + hex.EncodeToString(digest[:8])

	return Verdict{
		FixtureID:      f.ID,
		Label:          label,
		Confidence:     float64(confidenceF32), // f32 → f64 exact cast
		ConeID:         "cone-" + f.CatalogID,
		RoundsExecuted: rounds,
		SwarmID:        swarmID,
		CatalogVersion: defaultCatalogVersion,
		SubScores:      []SubScore{},
		LatencyMS:      0.0,
	}
}

// realVerdict would call the live gRPC backend via the Go SDK.
// **Φ-C-SDK D30 stub** — currently a no-op that delegates to mock
// because clients/go/efe-sdk gRPC stubs aren't yet generated at the
// freeze date. DT-SDK-WIRE-1 (D31) will swap this to the real path.
func realVerdict(_ Fixture) (Verdict, error) {
	return Verdict{}, fmt.Errorf("real gRPC path not yet wired ; set EFE_CONTRACT_MOCK=1")
}

func main() {
	fixturesPath := flag.String("fixtures", "/fixtures/quartet_fixtures.json", "input fixtures JSON path")
	outPath := flag.String("out", "/shared/out_go.json", "output verdicts JSON path")
	host := flag.String("host", os.Getenv("EFE_HOST"), "gRPC host:port (default $EFE_HOST)")
	versionLabel := flag.String("version", "0.6.0", "SDK version label")
	flag.Parse()
	_ = host

	raw, err := os.ReadFile(*fixturesPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "FATAL: read fixtures: %v\n", err)
		os.Exit(2)
	}
	var payload FixturePayload
	if err := json.Unmarshal(raw, &payload); err != nil {
		fmt.Fprintf(os.Stderr, "FATAL: parse fixtures: %v\n", err)
		os.Exit(2)
	}

	mockMode := os.Getenv("EFE_CONTRACT_MOCK") == "1"
	verdicts := make([]Verdict, 0, len(payload.Fixtures))
	for _, f := range payload.Fixtures {
		var v Verdict
		if mockMode {
			v = mockVerdict(f)
		} else {
			v, err = realVerdict(f)
			if err != nil {
				fmt.Fprintf(os.Stderr, "FATAL: %s: %v\n", f.ID, err)
				os.Exit(2)
			}
		}
		verdicts = append(verdicts, v)
	}

	out := OutputPayload{
		SDK:      "go",
		Version:  *versionLabel,
		Count:    len(verdicts),
		Verdicts: verdicts,
	}
	outBytes, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		fmt.Fprintf(os.Stderr, "FATAL: marshal output: %v\n", err)
		os.Exit(1)
	}

	if err := os.MkdirAll(parentDir(*outPath), 0o755); err != nil {
		fmt.Fprintf(os.Stderr, "FATAL: mkdir: %v\n", err)
		os.Exit(1)
	}
	if err := os.WriteFile(*outPath, append(outBytes, '\n'), 0o644); err != nil {
		fmt.Fprintf(os.Stderr, "FATAL: write output: %v\n", err)
		os.Exit(1)
	}
	fmt.Fprintf(os.Stderr, "runner-go : wrote %d verdicts → %s\n",
		len(verdicts), *outPath)
}

func parentDir(p string) string {
	for i := len(p) - 1; i >= 0; i-- {
		if p[i] == '/' || p[i] == os.PathSeparator {
			return p[:i]
		}
	}
	return "."
}
