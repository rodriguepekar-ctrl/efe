// Sprint Φ-C-SDK Completion (D30) — minimal go.mod for the contract runner.
//
// The runner has zero external dependencies in mock mode (= stdlib only :
// crypto/sha256, encoding/{json,binary,hex}, math, os, flag). The live
// gRPC path will require google.golang.org/grpc once DT-SDK-WIRE-1 lands.

module github.com/efe-ai/efe/sdk/contract-runner/runners/go

go 1.22
