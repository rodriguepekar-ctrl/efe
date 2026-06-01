// Sprint D23 — gin example for `efe-sdk-go`.
//
// Separate sub-module : adding the SDK to your project does NOT
// pull `gin` as a transitive dep. Each example is built/published
// independently.
module github.com/efe-ai/efe-sdk-go/examples/gin

go 1.22

require (
	github.com/efe-ai/efe-sdk-go v0.3.0
	github.com/gin-gonic/gin v1.9.1
)

// During development, replace with the local path so `go run`
// from the repo doesn't require the SDK to be published.
replace github.com/efe-ai/efe-sdk-go => ../..
