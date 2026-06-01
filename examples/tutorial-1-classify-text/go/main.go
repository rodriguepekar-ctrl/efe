// Tutorial 1 — Classify a single text via the Go SDK.
//
// Sprint Φ-C-SDK Completion (D32) — artefact #25.
//
// Build :
//   cd examples/tutorial-1-classify-text/go && go build -o tutorial-1 .
//
// Run :
//   EFE_HOST=efe.example.com:443 ./tutorial-1

package main

import (
	"context"
	"fmt"
	"log"
	"os"
	"time"
	// Once the Go SDK is published, this becomes :
	// import efe "github.com/efe-ai/efe-go"
)

func main() {
	host := os.Getenv("EFE_HOST")
	if host == "" {
		host = "efe.example.com:443"
	}

	// Once Φ-C-SDK D31 wiring lands :
	//
	//   client, err := efe.NewClient(host)
	//   if err != nil {
	//       log.Fatalf("connect: %v", err)
	//   }
	//   defer client.Close()
	//
	//   ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	//   defer cancel()
	//
	//   verdict, err := client.Classify(ctx, &efe.ClassifyRequest{
	//       Text:      "I want to dispute the charge on my account",
	//       CatalogId: "router-v1",
	//   })
	//   if err != nil {
	//       log.Fatalf("classify: %v", err)
	//   }
	//
	//   fmt.Printf("label       : %s\n", verdict.Label)
	//   fmt.Printf("confidence  : %.4f\n", verdict.Confidence)
	//   fmt.Printf("rounds      : %d\n", verdict.RoundsExecuted)
	//   fmt.Printf("swarm_id    : %s\n", verdict.SwarmId)
	//   fmt.Printf("catalog_ver : %s\n", verdict.CatalogVersion)

	// Pre-D31 placeholder :
	_ = context.Background
	_ = time.Second
	log.Printf("tutorial-1 (Go) connecting to %s\n", host)
	fmt.Println("Once Φ-C-SDK D31 wires the Go SDK gRPC stubs, this tutorial")
	fmt.Println("will perform an end-to-end Classify call. For now it documents")
	fmt.Println("the idiomatic Go API shape per docs/sdk-auth-conventions.md.")
}
