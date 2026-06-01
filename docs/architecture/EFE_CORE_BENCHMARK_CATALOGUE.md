# EFE Core Benchmark Catalogue — Meta-Swarm-independent benchmarks

**Status** : DRAFT (catalogue review pending bench-infra leadership 2026-06-01)
**Phase** : 3 (Q4-2026 critical-path quarter)
**Type** : Process + benchmark suite (zero-LOC change to existing subsystem code, ~3 000 LOC of new bench scenarios + harness + tests)
**Owner** : `@efe-ai/bench-infra` (primary) + `@efe-ai/release-eng` (publish chain integration)
**Authors** : bench-infra + the engineering leads of each subsystem owner
**Reviewers** : 3 minimum — bench-infra lead + one subsystem owner per family + release-eng lead
**Date** : 2026-05-17

**Companion docs** :
- Existing scenario catalogue (Meta-Swarm-dependent, S01-S10) : [`bench/scenarios/`](../../bench/scenarios/)
- Stability contract : [`docs/STABILITY_CONTRACT.md`](../STABILITY_CONTRACT.md) § 27 (new, this catalogue)
- Sibling architecture doc (process discipline) : [`EFE_A1_PHASE3_PR25_ARCHITECTURE.md`](EFE_A1_PHASE3_PR25_ARCHITECTURE.md)
- Sibling architecture doc (post-mortem) : [`EFE_A1_PHASE3_PR26_ARCHITECTURE.md`](EFE_A1_PHASE3_PR26_ARCHITECTURE.md)

---

## 0. TL;DR

This document introduces the **Core Benchmark Catalogue** — a set of
**thirty-five** benchmarks that measure the EFE platform's behaviour
without activating the Meta-Swarm coordination layer. The catalogue
complements the existing ten S-prefixed scenarios (S01-S10) which
all target the Meta-Swarm controller via gRPC and which therefore
cannot run on a deployment where Meta-Swarm is not user-activated.

The Core Benchmark Catalogue covers thirty subsystem-level benchmarks
(CB-01 through CB-30) and five meta-benchmarks (CB-A1 through CB-A5).
Together they exercise the platform's non-coordination crates *and*
its top-level subsystems (the PyO3 FFI bridge, the MCP server, the
SLOs rollup, the quarterly-PDF publisher, the eval harness, the
agent-core lifecycle, the soak workload, the DR-drill simulator, the
Helm chart, the Docker image) in a way that produces reproducible,
signed, audit-grade evidence that the platform's non-coordination
surfaces meet their declared service-level indicators.

> **Note on the 15 → 35 expansion.** The catalogue was originally
> shipped with fifteen benchmarks. A deep-coverage audit performed on
> 2026-05-17 (audit-trail row in STABILITY § 27.5) surfaced ten
> previously-uncovered crates, eight previously-uncovered top-level
> subsystems and two previously-uncovered packaging artefacts. The
> twenty new benchmarks (CB-13..CB-30 and CB-A4..CB-A5) were added
> under the same freeze discipline as the original fifteen. Two new
> families were introduced to host them : **integration** for
> benchmarks that cross the Rust↔external boundary (PyO3 FFI, MCP
> server, eval harness, agent-core lifecycle) and **long-run** for
> benchmarks whose defining property is duration rather than
> subsystem identity (soak workload, DR drill). Several passages in
> this document still use the figure "fifteen" or the phrase "the
> twelve subsystem benchmarks" — these are preserved verbatim as
> historical narrative and supplemented with deep-coverage callouts
> where the count matters operationally.

Why does this catalogue exist ? Three reasons. First, a customer who
deploys EFE *without* activating Meta-Swarm currently has no way
to validate that the rest of the platform is working as advertised ;
the existing S-prefixed scenarios all fail closed because they
target a Meta-Swarm endpoint. Second, the engineering team needs a
benchmark suite that runs in continuous integration on every
recorded change without requiring a kind cluster spun up with
Meta-Swarm — the existing scenarios run only on the bench-publish
schedule because they require expensive cluster provisioning.
Third, the audit narrative we make to industrial customers about
the platform's reliability needs to be defensible at the subsystem
level, not just at the coordination-layer level ; an auditor
asking "what is your evidence that the vector store performs
within budget" deserves a concrete answer.

The catalogue is frozen under STABILITY § 27 for the same
six-chart-minor horizon as § 25 and § 26. The freeze covers the
benchmark identifiers (CB-01 through CB-A3), the tolerance budgets
declared per benchmark, the metric set each benchmark emits, and
the YAML schema each benchmark uses. The freeze does NOT cover
the underlying subsystem implementations — those are versioned
independently and may improve their actual performance as long as
they do not regress beyond the tolerance budgets.

**Success criterion** : within ninety days of this catalogue
landing, ninety percent of the thirty-five benchmarks run in CI on
every PR that modifies a crate they depend on, and the bench
portal publishes a CB-suite summary alongside the S-suite summary
on every quarterly publish.  Of the thirty-five, the cheap five
(CB-A1 CLI cold-start, CB-A2 workspace tests, CB-06 server API,
CB-A4 Helm install dry-run, CB-A5 Docker image build) run on every
PR ; the remainder run nightly.

---

## 1. Why this catalogue exists

### 1.1 The gap the catalogue closes

The platform's existing benchmark surface, comprising the ten
S-prefixed scenarios in `bench/scenarios/`, was designed during
PR #21 (the public-bench harness release) and PR #24 (the public
bench results release). Every one of those scenarios targets the
Meta-Swarm controller : their `loadgen.target.endpoints` field
points at `grpc://efe-meta-swarm-controller.efe.svc:50051`, and
they will not produce results against a deployment where
Meta-Swarm is not present.

This was a defensible design choice when EFE was conceived as a
coordination platform whose only purpose was Meta-Swarm. It has
become a problem as the platform's surface has grown : industrial
customers increasingly deploy EFE for the data plane, the
inference router, the saga orchestrator, the vector store, the
billing rollup, and the compliance pipeline, *without* activating
Meta-Swarm. For these customers, the existing benchmark suite
returns either inconclusive or outright failing verdicts at every
scheduled run, which makes the bench portal's outputs uninformative
about their deployment.

The team has dealt with this gap by writing ad-hoc benchmarks
inside individual crates as needed — there are
`crates/efe-core/tests/bench_smoke_test.rs`,
`crates/efe-server/tests/bench_realism_check.rs`, and similar
files scattered across the workspace. The problem with the ad-hoc
approach is that each ad-hoc benchmark emits its own metric
shape, runs on its own cadence, and produces no signed evidence.
There is no consolidated answer to the question "is the platform's
non-coordination behaviour within budget".

The Core Benchmark Catalogue closes this gap by introducing fifteen
canonical, signed-evidence-producing benchmarks that the team
commits to maintaining for at least the next six chart-minor
versions of the platform.

### 1.2 Why fifteen — and why we later widened to thirty-five

The original choice of fifteen benchmarks was deliberate. A larger
catalogue (one per crate) would have diluted the maintenance
discipline ; a smaller catalogue (one per architectural plane) would
have lost too much resolution to be useful. Fifteen sat at the
inflection point where each benchmark had a single owning team,
exercised a single hot path, and produced a single tolerance
verdict.

The original grouping into twelve subsystem benchmarks plus three
meta-benchmarks reflected how the team thought about the platform's
surface at the time of first authorship. The subsystem benchmarks
each exercised one crate's hot path under representative load. The
meta-benchmarks exercised properties of the platform as a whole —
its CLI cold-start time, its full test-suite duration, its
cluster-provisioner startup time — that no individual crate owns.

**The deep-coverage audit (2026-05-17).** A subsequent audit pass
walked every crate under `crates/` and every top-level directory of
the workspace asking, for each one, "is there a benchmark that would
detect a tolerance-class regression in this surface ?". The answer
was *no* for ten previously-uncovered crates (efe-retrieval,
efe-multimodal, efe-curation, efe-active-learning, efe-backup,
efe-admin, efe-agent-runner, efe-kernels, efe-core, efe-metering
standalone), for eight top-level subsystems whose code lives outside
the crates directory (the PyO3 FFI bridge, the MCP server, the SLOs
rollup, the quarterly-PDF publisher, the eval harness, the
agent-core lifecycle, the soak workload, the DR-drill simulator),
and for two packaging artefacts (the Helm chart, the Docker image).

The audit added one benchmark per uncovered surface, taking the
catalogue from fifteen to thirty-five. The new benchmarks slot
naturally into the existing families where they exercise a familiar
plane (CB-13 retrieval into data-plane, CB-14 multimodal into
inference, CB-A4/A5 into meta-bench, etc.) and into two newly-frozen
families where they do not :

- **integration** (CB-23, CB-24, CB-27, CB-28) for benchmarks that
  cross the Rust↔external boundary. The PyO3 FFI call overhead, an
  MCP server tool invocation, the eval harness's
  external-evaluator-roundtrip, and the agent-core lifecycle
  (Python-side agent boot + Rust-side runtime handshake) are all
  shaped by behaviour on both sides of an FFI or RPC seam, not by a
  single subsystem's behaviour. The "integration" family signals
  that to scenario readers.

- **long-run** (CB-29, CB-30) for benchmarks whose defining property
  is duration. A six-hour soak workload and a forty-five-minute DR
  drill are not characterised by which subsystem they exercise (they
  exercise many) ; they are characterised by sustained behaviour
  under time pressure. The "long-run" family makes that explicit.

Adding an eighth family is a STABILITY-bump PR. Removing or
renaming either of the two new families is also a STABILITY-bump
PR.

### 1.3 The relationship to the S-suite

The CB-suite does not replace the S-suite ; it complements it. The
two suites have explicit and non-overlapping responsibilities :

The S-suite exercises Meta-Swarm coordination under chaos. Its ten
scenarios introduce participant loss, network partitions,
certificate expiry, reload storms, multi-region replay, and
sustained soak load. Its tolerance budgets are calibrated against
the coordination layer's behaviour. Its outputs feed the
quarterly evidence chain that customers running Meta-Swarm
consume.

The CB-suite exercises the platform's other twenty-six crates
under representative load *without* Meta-Swarm in the loop. Its
fifteen benchmarks measure subsystem-level latency, throughput,
memory, and correctness properties. Its tolerance budgets are
calibrated against each subsystem's individual service-level
indicator. Its outputs feed a *separate* CB-suite summary that
customers running EFE without Meta-Swarm consume.

When both suites pass on the same quarterly run, the platform's
end-to-end correctness is demonstrated at both granularities.
When one passes and the other fails, the failure pinpoints
whether the regression is in coordination or in subsystem
behaviour. This is significantly more useful to the team than a
single combined verdict would be.

---

## 2. Scope

### 2.1 In scope

The catalogue defines and exercises **thirty-five** benchmarks
across **seven** families :

- **Data plane** (six benchmarks) : event-sourcing append
  throughput (CB-01), telemetry ingestion rate (CB-02), vector-store
  query latency (CB-03), knowledge-graph query latency (CB-04),
  cache hit-rate and operations per second (CB-05), hybrid retrieval
  blended-rank latency (CB-13), curation-pipeline document
  throughput (CB-15), standalone metering ingestion (CB-22).
- **Control plane** (six benchmarks) : server-API latency under
  fixed load (CB-06), region-routing decision latency (CB-07), saga
  step-latency and throughput (CB-08), backup/DR orchestration
  step-time (CB-17), admin tenant-onboarding workflow (CB-18),
  agent-runner Firecracker cold-start (CB-19), core orchestration
  primitives (CB-21).
- **Inference path** (four benchmarks) : language-model router
  decision latency (CB-09), prompt-rendering throughput (CB-10),
  multimodal fusion latency (CB-14), active-learning sampling
  throughput (CB-16), CUDA kernel throughput (CB-20).
- **Compliance** (four benchmarks) : PII redaction throughput
  (CB-11), billing rollup computation throughput (CB-12), SLOs
  rollup throughput (CB-25), quarterly-PDF publish wall-clock
  (CB-26).
- **Integration** (four benchmarks) : PyO3 FFI call overhead
  (CB-23), MCP server tool invocation (CB-24), eval-harness
  throughput (CB-27), agent-core lifecycle (CB-28).
- **Long-run** (two benchmarks) : sustained-load soak (CB-29), DR
  drill simulator (CB-30).
- **Meta-benchmarks** (five benchmarks) : harness CLI cold-start
  (CB-A1), workspace test-suite duration (CB-A2),
  cluster-provisioner kind cold-start (CB-A3), Helm chart install
  time (CB-A4), Docker image build time (CB-A5).

Each benchmark has a YAML scenario file under `bench/scenarios/`
following the same schema as the existing S-prefixed scenarios,
a tolerance budget calibrated against the subsystem's declared
SLI, and a canonical reference dataset against which CI runs are
compared.

### 2.2 Explicitly out of scope

The catalogue does not introduce any new subsystem code. It does
not modify any frozen surface. It does not change any existing
S-suite scenario.

The catalogue does not benchmark the Meta-Swarm coordination
layer or any crate prefixed `efe-meta-swarm-` ; those are the
explicit responsibility of the S-suite.

The catalogue does not introduce a new tolerance heuristic for
the existing S-suite. The S-suite's tolerance discipline is
frozen under STABILITY § 24.3 and remains so.

The catalogue does not impose a new CI cadence on the existing
S-suite. The S-suite continues to run on the quarterly publish
schedule. The CB-suite runs on a separate (cheaper, more
frequent) cadence described in § 6.

### 2.3 Deferred to follow-up PRs

Three items are deliberately deferred. First, the full Rust
criterion implementation of each benchmark — the catalogue ships
the YAML specifications and the orchestrator, with one reference
criterion implementation included for the pattern. The remaining
fourteen implementations are scheduled for a follow-up PR. Second,
the integration of CB-suite outputs into the quarterly evidence
chain — the CB-suite initially emits standalone summaries, which
will be consolidated into the quarterly chain after one full
calibration quarter. Third, the historical baselines for each
benchmark — these are populated by the first month of CB-suite
runs, after which the tolerance budgets become enforcing rather
than advisory.

---

## 3. Vocabulary

Throughout this catalogue, the following terms have the following
precise meanings.

**The CB-suite.** The set of fifteen Core Benchmarks introduced by
this document. Identified by the `CB-` filename prefix. Frozen
under STABILITY § 27.

**The S-suite.** The pre-existing set of ten chaos-and-coordination
scenarios identified by the `S` prefix. Frozen under STABILITY
§ 24.3.

**A benchmark.** A single named entry in the CB-suite. Each
benchmark has a YAML scenario file, a tolerance budget, a
reference dataset, and an owning team.

**A tolerance budget.** The maximum permitted drift of each
metric the benchmark emits, expressed as either a delta percentage
against the reference baseline or an absolute threshold. Drift
beyond the budget produces a WARN or FAIL verdict per § 5.

**A subsystem.** One of the twenty-six non-coordination crates in
the workspace. Each subsystem has a primary CB benchmark and may
have an additional crate-internal criterion benchmark.

**A meta-benchmark.** A benchmark that exercises a property of the
platform as a whole rather than a single subsystem. There are
five : CB-A1 (harness CLI cold-start), CB-A2 (workspace test-suite
duration), CB-A3 (cluster-provisioner cold-start), CB-A4 (Helm
chart install time), CB-A5 (Docker image build time).

**The reference dataset.** The canonical input the benchmark runs
against. For data-plane benchmarks, this is a static fixture
file ; for control-plane benchmarks, this is a synthesised request
trace.

**The reference baseline.** The expected output of the benchmark
against the reference dataset. Captured at calibration time,
revised at each STABILITY-bump. Lives under
`bench/baselines/CB-NN.json`.

---

## 4. The fifteen benchmarks

This section describes each benchmark : its identifier, the
subsystem it exercises, the workload it applies, the metrics it
emits, the tolerance budget against the reference baseline, and
the owning team. The descriptions are deliberately concrete
because the freeze at STABILITY § 27 covers them in full.

### 4.1 Data plane

#### CB-01 — Event-sourcing append throughput

**Subsystem** : `efe-events`.
**Owner** : data-plane team.

The benchmark drives sustained append traffic against the event
store. The workload is a constant rate of synthesised events of
typical size (approximately 512 bytes each) for sixty seconds.
The benchmark measures the maximum sustainable rate without
storage backpressure, the per-event append latency at p50 and
p99, and the storage growth rate.

**Workload** : 10 000 events per second, mixed event types,
constant rate, 60-second duration.

**Metrics emitted** : `append_rate_per_sec`, `append_p50_ms`,
`append_p99_ms`, `storage_growth_mb_per_sec`, `error_rate_pct`.

**Tolerance budget** : `append_rate_per_sec` no more than 10 %
below baseline ; `append_p99_ms` no more than 20 % above baseline ;
`storage_growth_mb_per_sec` within 5 % of baseline ;
`error_rate_pct` strictly zero.

#### CB-02 — Telemetry ingestion rate

**Subsystem** : `efe-telemetry`.
**Owner** : data-plane team.

The benchmark feeds OpenTelemetry-compatible spans and metrics
into the telemetry pipeline at increasing rates until the pipeline
reports backpressure. The benchmark measures the maximum
sustainable ingestion rate, the per-span overhead, and the
end-to-end propagation latency from emission to storage.

**Workload** : ramp from 1 000 to 100 000 spans per second over
60 seconds, mixed span types.

**Metrics emitted** : `max_sustained_spans_per_sec`,
`per_span_overhead_us`, `propagation_p99_ms`, `drop_rate_pct`.

**Tolerance budget** : `max_sustained_spans_per_sec` no more than
10 % below baseline ; `propagation_p99_ms` no more than 20 %
above baseline ; `drop_rate_pct` strictly zero up to the maximum
sustained rate.

#### CB-03 — Vector store query latency

**Subsystem** : `efe-vector`.
**Owner** : data-plane team.

The benchmark exercises the vector store under realistic query
patterns : a mix of exact-match lookups, approximate-nearest-
neighbour queries with varying k, and bulk-insert operations.
The benchmark measures per-query latency at p50, p95, p99, and
the throughput under saturating load.

**Workload** : a deterministic seed produces 10 000 queries with
60 % approximate-nearest-neighbour (k=10), 30 % exact lookup,
10 % bulk insert (10 vectors per insert).

**Metrics emitted** : `query_p50_ms`, `query_p99_ms`,
`bulk_insert_p99_ms`, `qps_at_saturation`, `index_size_mb`.

**Tolerance budget** : `query_p99_ms` no more than 15 % above
baseline ; `qps_at_saturation` no more than 10 % below baseline ;
`index_size_mb` within 5 % of baseline.

#### CB-04 — Knowledge graph query latency

**Subsystem** : `efe-kg` and `efe-graphrag`.
**Owner** : data-plane team.

The benchmark issues representative read queries against the
knowledge graph : single-hop neighbour lookups, multi-hop
traversals up to depth four, and GraphRAG-style retrieval-
augmented queries that combine vector and graph operations. The
benchmark measures per-query latency at p50 and p99, and the
query-plan stability across multiple runs against the same
input.

**Workload** : 5 000 mixed queries (50 % single-hop, 30 %
multi-hop, 20 % GraphRAG) over 30 seconds.

**Metrics emitted** : `single_hop_p99_ms`, `multi_hop_p99_ms`,
`graphrag_p99_ms`, `plan_stability_pct`, `qps_at_saturation`.

**Tolerance budget** : every `_p99_ms` metric no more than 20 %
above baseline ; `plan_stability_pct` exactly 100 % ;
`qps_at_saturation` no more than 10 % below baseline.

#### CB-05 — Cache hit-rate and operations per second

**Subsystem** : `efe-cache`.
**Owner** : data-plane team.

The benchmark exercises the cache under a realistic access
pattern : a Zipfian distribution with skew matching the
production trace, with a working set sized to 80 % of the cache
capacity (forcing some eviction). The benchmark measures hit
rate, operations per second, eviction rate, and per-operation
latency.

**Workload** : Zipfian distribution alpha=1.0, working-set
1 million keys, cache capacity 800 000 keys, 100 000 operations
per second for 30 seconds.

**Metrics emitted** : `hit_rate_pct`, `ops_per_sec`,
`eviction_rate_per_sec`, `get_p99_us`, `put_p99_us`.

**Tolerance budget** : `hit_rate_pct` no more than 2 percentage
points below baseline ; `ops_per_sec` no more than 10 % below
baseline ; `get_p99_us` and `put_p99_us` no more than 25 % above
baseline.

### 4.2 Control plane

#### CB-06 — Server API latency under fixed load

**Subsystem** : `efe-server`.
**Owner** : control-plane team.

The benchmark issues synthetic HTTP and gRPC requests against
the server's public surface (excluding any Meta-Swarm-bound
endpoint) at a fixed rate. The benchmark measures per-request
latency at p50, p95, p99, error rate, and the server's CPU and
memory utilisation under the load.

**Workload** : 1 000 requests per second for 60 seconds, mixed
endpoint distribution (40 % read, 40 % write, 20 % admin).

**Metrics emitted** : `request_p99_ms`, `error_rate_pct`,
`cpu_pct_p99`, `memory_mb_p99`.

**Tolerance budget** : `request_p99_ms` no more than 15 % above
baseline ; `error_rate_pct` below 0.1 % ; `cpu_pct_p99` no more
than 5 percentage points above baseline ; `memory_mb_p99` within
10 % of baseline.

#### CB-07 — Region-routing decision latency

**Subsystem** : `efe-region`.
**Owner** : control-plane team.

The benchmark exercises the multi-region routing layer under a
synthesised stream of requests that span the recognised regions.
The benchmark measures per-decision latency, the routing-table
lookup correctness against a reference table, and the cache hit
rate on the routing layer's own internal cache.

**Workload** : 50 000 routing decisions over 10 seconds, four
recognised regions, request origin distribution matching a
production trace.

**Metrics emitted** : `decision_p99_us`,
`routing_correctness_pct`, `internal_cache_hit_rate_pct`.

**Tolerance budget** : `decision_p99_us` no more than 25 % above
baseline ; `routing_correctness_pct` exactly 100 % ;
`internal_cache_hit_rate_pct` no more than 3 percentage points
below baseline.

#### CB-08 — Saga step-latency and throughput

**Subsystem** : `efe-saga`.
**Owner** : control-plane team.

The benchmark drives the saga orchestrator through a
representative workload of multi-step transactions, including
both successful completions and forced rollbacks. The benchmark
measures per-step latency, per-saga end-to-end latency, and the
throughput of completed sagas per second.

**Workload** : 1 000 sagas of varying step-count (3 to 12 steps),
80 % success rate, 20 % forced rollback, 30-second duration.

**Metrics emitted** : `step_p99_ms`, `saga_p99_ms`,
`completed_sagas_per_sec`, `rollback_success_pct`.

**Tolerance budget** : `saga_p99_ms` no more than 20 % above
baseline ; `completed_sagas_per_sec` no more than 10 % below
baseline ; `rollback_success_pct` exactly 100 %.

### 4.3 Inference path

#### CB-09 — Language-model router decision latency

**Subsystem** : `efe-llm-router`.
**Owner** : inference team.

The benchmark exercises the language-model router under a
synthesised stream of requests with varying classification
criteria : context length, latency tolerance, cost ceiling,
modality requirement. The benchmark measures per-decision
latency and the routing-decision correctness against a reference
oracle.

**Workload** : 10 000 routing decisions, no actual model
invocations (the router itself is the measured surface), 10-second
duration.

**Metrics emitted** : `decision_p99_us`, `oracle_match_pct`,
`per_decision_cpu_us`.

**Tolerance budget** : `decision_p99_us` no more than 25 % above
baseline ; `oracle_match_pct` exactly 100 % ;
`per_decision_cpu_us` no more than 15 % above baseline.

#### CB-10 — Prompt rendering throughput

**Subsystem** : `efe-prompts`.
**Owner** : inference team.

The benchmark exercises the prompt-templating engine under a
realistic mix of template sizes and substitution counts. The
benchmark measures the number of rendered prompts per second
and the per-render latency at p99.

**Workload** : 100 000 renders over 10 seconds, mix of template
sizes (small, medium, large), substitution counts between 1 and
50 per render.

**Metrics emitted** : `renders_per_sec`, `per_render_p99_us`,
`template_cache_hit_rate_pct`.

**Tolerance budget** : `renders_per_sec` no more than 10 % below
baseline ; `per_render_p99_us` no more than 20 % above baseline ;
`template_cache_hit_rate_pct` no more than 2 percentage points
below baseline.

### 4.4 Compliance

#### CB-11 — PII redaction throughput

**Subsystem** : `efe-pii`.
**Owner** : compliance team.

The benchmark exercises the PII-redaction engine against a
synthetic dataset containing known PII patterns at known rates.
The benchmark measures the throughput of redacted documents per
second, the per-document latency, the recall of the redaction
(percentage of seeded PII actually redacted), and the precision
(percentage of redactions that were correct).

**Workload** : 10 000 synthetic documents with seeded PII at
controlled density (5 % of tokens are PII), 30-second duration.

**Metrics emitted** : `docs_per_sec`, `per_doc_p99_ms`,
`recall_pct`, `precision_pct`.

**Tolerance budget** : `docs_per_sec` no more than 10 % below
baseline ; `recall_pct` no less than 99.5 % absolute ;
`precision_pct` no less than 98 % absolute ; `per_doc_p99_ms` no
more than 20 % above baseline.

#### CB-12 — Billing rollup computation

**Subsystem** : `efe-billing` and `efe-metering`.
**Owner** : compliance team.

The benchmark drives the billing rollup against a synthesised
metering trace covering a calendar month of activity for a
representative tenant. The benchmark measures the time to compute
the rollup, the correctness of the rollup against a reference
oracle, and the memory peak during the computation.

**Workload** : 30 days of synthetic metering events at production
density (approximately one million events for a single tenant
month).

**Metrics emitted** : `rollup_duration_sec`, `oracle_match_pct`,
`peak_memory_mb`, `events_per_sec_processed`.

**Tolerance budget** : `rollup_duration_sec` no more than 15 %
above baseline ; `oracle_match_pct` exactly 100 % ;
`peak_memory_mb` no more than 20 % above baseline.

### 4.5 Meta-benchmarks

#### CB-A1 — Harness CLI cold-start

**Subsystem** : `efe-bench` (the harness itself).
**Owner** : bench-infra.

The benchmark measures the wall-clock time from invocation of
the bench harness CLI binary to the moment the first measurement
is emitted. The measurement excludes any cluster setup and
focuses on the binary's own startup behaviour.

**Workload** : fifty consecutive invocations of
`efe-bench-harness validate` against a fixed scenario.

**Metrics emitted** : `cold_start_median_ms`,
`cold_start_p99_ms`.

**Tolerance budget** : `cold_start_median_ms` no more than 200
milliseconds ; `cold_start_p99_ms` no more than 500 milliseconds.

#### CB-A2 — Workspace test-suite duration

**Subsystem** : the workspace as a whole.
**Owner** : release-eng.

The benchmark measures the total wall-clock duration of
`cargo test --workspace --all-features` against a clean target
directory on the standard CI runner. This is the time the team
pays on every PR's CI run.

**Workload** : a single full test-suite run from a clean state.

**Metrics emitted** : `total_duration_sec`, `compile_duration_sec`,
`test_duration_sec`.

**Tolerance budget** : `total_duration_sec` no more than 600
seconds (10 minutes) ; `compile_duration_sec` no more than 480
seconds.

#### CB-A3 — Cluster-provisioner kind cold-start

**Subsystem** : `efe-cluster-provisioner`.
**Owner** : bench-infra.

The benchmark measures the wall-clock time from a
`provision --kind` invocation to a fully-ready kind cluster, on
the standard CI runner. The kind cluster is the lightest weight
cluster target the platform supports and is the most-used cluster
type in CI.

**Workload** : five cold starts from a clean state, with the kind
image pre-pulled.

**Metrics emitted** : `provision_median_sec`,
`provision_p99_sec`.

**Tolerance budget** : `provision_median_sec` no more than 90
seconds ; `provision_p99_sec` no more than 120 seconds.

#### CB-A4 — Helm chart install time

**Subsystem** : `charts/` (the Helm chart shipped to customers).
**Owner** : release-eng.

The benchmark measures the wall-clock time from `helm install` to
the chart's `--wait` semantics returning success, against a kind
cluster. This captures regressions in chart templating speed, in
the number of resources the chart renders, and in the readiness
probes of the rendered Pods.

**Workload** : ten consecutive cold installs of the chart against
a kind cluster pre-provisioned by CB-A3.

**Metrics emitted** : `install_median_sec`, `install_p99_sec`,
`rendered_resources_count`.

**Tolerance budget** : `install_median_sec` no more than 120
seconds ; `install_p99_sec` no more than 180 seconds ;
`rendered_resources_count` exactly within ±2 of the baseline (a
hard-coded resource count is a useful tripwire for an inadvertent
template change).

#### CB-A5 — Docker image build time

**Subsystem** : `docker/` (the multi-stage Dockerfile shipped to
customers).
**Owner** : release-eng.

The benchmark measures the wall-clock time of `docker buildx
build` against a cold layer cache. This captures regressions in
build-stage organisation, in the size of the build context, and
in the speed of the Rust compilation step (which dominates).

**Workload** : five cold-cache builds of the production
Dockerfile against the standard CI runner with BuildKit enabled.

**Metrics emitted** : `build_median_sec`, `build_p99_sec`,
`final_image_size_mb`.

**Tolerance budget** : `build_median_sec` no more than 900
seconds (15 minutes) ; `final_image_size_mb` no more than 1 200
megabytes.

### 4.6 Deep-coverage subsystem benchmarks (CB-13 through CB-22)

The ten benchmarks below each cover a crate that the original
fifteen-benchmark catalogue did not exercise. They were added on
2026-05-17 by the deep-coverage audit. Each follows the same
schema as the original fifteen.

#### CB-13 — Retrieval hybrid blended-rank latency

**Subsystem** : `efe-retrieval`.
**Owner** : retrieval team.

The benchmark drives a hybrid (vector + lexical + recency) ranker
against a synthesised corpus and measures the per-query blended
score computation time. The crate's hot path is the blender, not
the underlying vector store (CB-03) or the lexical index.

**Metrics emitted** : `query_p50_ms`, `query_p99_ms`,
`queries_per_sec`, `rank_stability_pct` (Spearman vs. baseline).

**Tolerance budget** : `query_p50_ms` ≤ 50 ms ; `query_p99_ms`
≤ 200 ms ; `rank_stability_pct` ≥ 95 %.

#### CB-14 — Multimodal fusion latency

**Subsystem** : `efe-multimodal`.
**Owner** : inference team.

The benchmark exercises the fusion path that combines text + image
embeddings for a representative multimodal query, against a
synthesised corpus of three modalities.

**Metrics emitted** : `fusion_p50_ms`, `fusion_p99_ms`,
`fusion_per_sec`.

**Tolerance budget** : `fusion_p50_ms` ≤ 80 ms ; `fusion_p99_ms`
≤ 350 ms.

#### CB-15 — Curation pipeline throughput

**Subsystem** : `efe-curation`.
**Owner** : data team.

The benchmark drives the curation pipeline against a synthesised
batch of documents (mixed quality, mixed PII, mixed length) and
measures end-to-end throughput.

**Metrics emitted** : `docs_per_sec`, `acceptance_rate_pct`,
`per_doc_p99_ms`.

**Tolerance budget** : `docs_per_sec` no more than 10 % below
baseline ; `acceptance_rate_pct` within ±1 % of baseline ;
`per_doc_p99_ms` no more than 20 % above baseline.

#### CB-16 — Active-learning sampling throughput

**Subsystem** : `efe-active-learning`.
**Owner** : ML-ops team.

The benchmark exercises the uncertainty-sampling loop that selects
the next batch of training examples from a candidate pool.

**Metrics emitted** : `samples_selected_per_sec`,
`selection_p99_ms`, `diversity_score`.

**Tolerance budget** : `samples_selected_per_sec` no more than
15 % below baseline ; `selection_p99_ms` ≤ 250 ms ;
`diversity_score` ≥ 0.7.

#### CB-17 — Backup / DR orchestration step-time

**Subsystem** : `efe-backup`.
**Owner** : SRE team.

The benchmark drives the backup orchestrator through a full
backup → verify → restore cycle against a synthesised state
snapshot and measures per-step latency.

**Metrics emitted** : `backup_step_p99_ms`, `restore_step_p99_ms`,
`verify_step_p99_ms`, `cycle_total_sec`.

**Tolerance budget** : each step's `_p99_ms` no more than 20 %
above baseline ; `cycle_total_sec` no more than 600 seconds.

#### CB-18 — Admin tenant-onboarding workflow

**Subsystem** : `efe-admin`.
**Owner** : platform team.

The benchmark exercises the tenant-onboarding workflow (create
tenant → provision quotas → seed defaults → emit audit) and
measures end-to-end wall-clock time + per-step latency.

**Metrics emitted** : `onboard_p50_ms`, `onboard_p99_ms`,
`onboards_per_sec`.

**Tolerance budget** : `onboard_p50_ms` ≤ 500 ms ;
`onboard_p99_ms` ≤ 2 000 ms.

#### CB-19 — Agent-runner Firecracker cold-start

**Subsystem** : `efe-agent-runner`.
**Owner** : runtime team.

The benchmark measures cold-start latency for a Firecracker
microVM hosting an agent runtime, against a pre-warmed image
cache.

**Metrics emitted** : `cold_start_median_ms`,
`cold_start_p99_ms`, `cold_starts_per_sec`.

**Tolerance budget** : `cold_start_median_ms` ≤ 800 ms ;
`cold_start_p99_ms` ≤ 1 500 ms.

#### CB-20 — Kernels CUDA throughput

**Subsystem** : `efe-kernels`.
**Owner** : inference team.

The benchmark exercises the CUDA-kernel hot paths (attention,
softmax, GEMM bias-add fusion) on a fixed GPU instance type.

**Metrics emitted** : `tokens_per_sec_attention`,
`tokens_per_sec_softmax`, `gemm_gflops`.

**Tolerance budget** : each metric no more than 10 % below
baseline. Run only on GPU-equipped runners ; INCONCLUSIVE on CPU.

#### CB-21 — Core orchestration primitives

**Subsystem** : `efe-core`.
**Owner** : core team.

The benchmark drives the core's primitive orchestration loop
(span emission, error wrapping, retry policy evaluation, deadline
propagation) under a synthetic workload.

**Metrics emitted** : `primitive_ops_per_sec`,
`primitive_p99_ns`.

**Tolerance budget** : `primitive_ops_per_sec` no more than 10 %
below baseline ; `primitive_p99_ns` no more than 20 % above
baseline.

#### CB-22 — Metering ingestion (standalone)

**Subsystem** : `efe-metering` standalone (not co-driven with
billing rollup as in CB-12).
**Owner** : compliance team.

The benchmark exercises metering-event ingestion alone, isolating
the per-event cost from the billing rollup.

**Metrics emitted** : `events_per_sec`, `ingest_p99_ms`,
`dropped_events_pct`.

**Tolerance budget** : `events_per_sec` no more than 10 % below
baseline ; `dropped_events_pct` exactly 0 % ; `ingest_p99_ms`
≤ 50 ms.

### 4.7 Integration-family benchmarks (CB-23, CB-24, CB-27, CB-28)

The four benchmarks below cross the Rust↔external boundary. Their
defining property is that performance is shaped by behaviour on
*both* sides of the boundary, not by a single subsystem in
isolation.

#### CB-23 — PyO3 FFI call overhead

**Subsystem** : `efe-ffi` (the PyO3 binding).
**Owner** : FFI team.

The benchmark measures the per-call overhead of crossing the
Rust → Python boundary for representative call shapes (numeric
scalar, small dict, large bytes buffer).

**Metrics emitted** : `call_overhead_p50_ns`,
`call_overhead_p99_ns`, `calls_per_sec`.

**Tolerance budget** : `call_overhead_p50_ns` ≤ 2 000 ns ;
`call_overhead_p99_ns` ≤ 10 000 ns.

#### CB-24 — MCP server tool invocation

**Subsystem** : `mcp-server` (the top-level MCP shim).
**Owner** : agent-runtime team.

The benchmark drives an end-to-end tool-invocation roundtrip
(client → MCP server → tool handler → client) against a fixed
tool catalogue.

**Metrics emitted** : `invocation_p50_ms`, `invocation_p99_ms`,
`invocations_per_sec`.

**Tolerance budget** : `invocation_p50_ms` ≤ 30 ms ;
`invocation_p99_ms` ≤ 150 ms.

#### CB-27 — Eval harness throughput

**Subsystem** : `eval/` (the top-level eval harness).
**Owner** : eval team.

The benchmark exercises the eval harness against a synthesised
batch of evaluator invocations and measures end-to-end throughput
+ per-eval latency.

**Metrics emitted** : `evals_per_sec`, `eval_p99_ms`,
`harness_overhead_ms_pct`.

**Tolerance budget** : `evals_per_sec` no more than 15 % below
baseline ; `harness_overhead_ms_pct` ≤ 20 %.

#### CB-28 — Agent-core lifecycle

**Subsystem** : `agent-core` (the Python-side agent core paired
with the Rust runtime).
**Owner** : agent-runtime team.

The benchmark drives the full agent lifecycle (boot → handshake
with Rust runtime → first action → graceful shutdown) and
measures per-phase wall-clock time.

**Metrics emitted** : `boot_p99_ms`, `handshake_p99_ms`,
`first_action_p99_ms`, `shutdown_p99_ms`.

**Tolerance budget** : each phase no more than 20 % above
baseline ; total lifecycle ≤ 5 seconds.

### 4.8 Long-run benchmarks (CB-29, CB-30)

The two benchmarks below are characterised by duration rather
than subsystem. They run on a separate weekly cadence and feed a
distinct long-run summary.

#### CB-29 — Sustained-load soak

**Subsystem** : `bench/soak` (the top-level soak driver).
**Owner** : SRE team.

The benchmark drives a representative production-shaped workload
at fixed RPS for six hours and measures the drift of key metrics
over the window.

**Metrics emitted** : `p99_drift_pct_per_hour`,
`error_rate_drift_pct_per_hour`, `memory_drift_mb_per_hour`,
`final_verdict_drift_classification`.

**Tolerance budget** : `p99_drift_pct_per_hour` ≤ 2 % per hour ;
`memory_drift_mb_per_hour` ≤ 50 MB per hour (heap-growth
tripwire).

#### CB-30 — DR drill simulator

**Subsystem** : `dr/` (the top-level DR drill harness).
**Owner** : SRE team.

The benchmark simulates a region-loss + restore cycle against a
synthesised cluster state and measures recovery-time-objective +
recovery-point-objective.

**Metrics emitted** : `rto_minutes`, `rpo_seconds`,
`drill_total_minutes`, `restore_correctness_pct`.

**Tolerance budget** : `rto_minutes` ≤ 30 ; `rpo_seconds` ≤ 60 ;
`restore_correctness_pct` exactly 100 %.

---

## 5. Verdict semantics

Each benchmark produces one of four verdicts at the end of its
run, drawn from the same closed taxonomy as the S-suite :

**PASS** — every metric within its tolerance budget ; the
benchmark's behaviour is consistent with the reference baseline.

**WARN** — at least one metric outside its tolerance budget but
the drift is small enough that the benchmark's run still produced
a usable measurement.

**FAIL** — at least one metric outside its tolerance budget by a
margin larger than the WARN threshold ; the benchmark's run
indicates a regression.

**INCONCLUSIVE** — the benchmark could not produce a complete
measurement (CI runner crashed, network failure, dependency
unavailable). The run is re-attempted once ; if the re-attempt
also returns INCONCLUSIVE, the benchmark is flagged for human
investigation.

The verdicts compose under the same worst-case aggregator as the
S-suite : the CB-suite's overall verdict is the worst per-
benchmark verdict, with INCONCLUSIVE ranking above FAIL.

---

## 6. Cadence

The CB-suite runs at four distinct cadences (one cadence was
added when the catalogue widened from fifteen to thirty-five so
that the long-run benchmarks have somewhere to live).

**On every PR** — five benchmarks run on every PR : CB-A1
(harness CLI cold-start), CB-A2 (workspace test-suite duration),
CB-A4 (Helm chart install time) and CB-A5 (Docker image build
time) are mechanically cheap enough to run on every PR without
inflating CI time, and CB-06 (server API latency) is sensitive
enough to catch most subsystem-level regressions introduced by
typical changes. The remaining benchmarks do not run on every PR ;
they run on the daily or weekly cadence below.

**Daily on main** — the full nightly CB suite (all benchmarks
*except* the two long-run ones) runs at 03:00 UTC every business
day against the latest commit on main. The daily run takes
approximately ninety minutes total wall-clock time on the
standard CI runner after the expansion (up from forty minutes in
the original fifteen-benchmark catalogue). The verdict of each
daily run is posted to the sticky tracker issue labeled
`cb-suite-status` (distinct from the post-mortem tracker
introduced in PR #26).

**Weekly long-run cadence** — CB-29 (six-hour sustained soak) and
CB-30 (forty-five-minute DR drill) run on the weekly cadence,
starting Sunday 22:00 UTC, against a dedicated long-run runner.
Their verdicts post to a separate sticky tracker issue labeled
`cb-suite-longrun-status`. Long-run benchmarks must not block
PR merges.

**Quarterly publish** — the CB-suite contributes to the
quarterly evidence chain alongside the S-suite. Each quarterly
publish produces two summary documents : the existing
S-suite summary covering Meta-Swarm coordination, and a new
CB-suite summary covering subsystem behaviour. Both summaries
are signed under the same cosign keyless OIDC identity and both
are recorded in the same Rekor transparency log.

---

## 7. Reference baselines

Each benchmark has a reference baseline captured at calibration
time and stored at `bench/baselines/CB-NN.json`. The baseline file
contains the expected value of each metric the benchmark emits,
plus the standard deviation of the metric across the calibration
runs, plus the date and commit at which the baseline was captured.

Baselines are revised at every STABILITY-bump that touches the
relevant subsystem, with the revision recorded in the audit-trail
addendum of STABILITY § 27.5. Baselines are NOT revised silently ;
a silent baseline revision would mask a regression, defeating the
benchmark's purpose.

When a benchmark's PASS / WARN / FAIL verdict is computed, the
comparison is against the *most recent* baseline that is
applicable to the benchmark's input dataset. If the input dataset
has changed since the baseline was captured, the benchmark
returns INCONCLUSIVE until a new baseline is captured against
the new dataset.

---

## 8. STABILITY § 27 additions

The CB-suite is frozen under a new STABILITY § 27 covering the
following sub-sections :

- § 27.1 — The thirty-five benchmark identifiers (CB-01 through
  CB-30 and CB-A1 through CB-A5). Frozen as a closed set ; adding
  new benchmarks requires a STABILITY-bump PR.  Originally fifteen
  identifiers ; widened to thirty-five on 2026-05-17 by the
  deep-coverage audit (see § 27.5 audit-trail).
- § 27.2 — The metric emitted by each benchmark. Frozen per
  benchmark ; adding new metrics requires a STABILITY-bump PR.
- § 27.3 — The YAML schema of each scenario file. Frozen ;
  shape changes require a STABILITY-bump PR.
- § 27.4 — The verdict taxonomy (PASS / WARN / FAIL /
  INCONCLUSIVE). Identical to the S-suite taxonomy ; frozen by
  reference to § 24.3.
- § 27.5 — Audit-trail addendum capturing the calibration date
  and commit for each baseline. Appended to as baselines are
  revised.

The freezing is enforced by `bench/harness/tests/cb_catalogue.rs`,
a new integration test file that asserts the presence and shape
of each YAML file and the existence of each baseline file.

---

## 9. Implementation rollout

The catalogue lands in three steps :

**Step 1 — Catalogue + scenarios (this PR)**. The fifteen YAML
files ship together with the catalogue document, the STABILITY
contract amendment, and the validator that asserts the YAML
files conform to the schema. The bench harness is extended to
recognise the CB- prefix and route CB- scenarios to a
non-Meta-Swarm execution path.

**Step 2 — Per-benchmark implementation (follow-up PR)**. The
fifteen benchmarks' actual workload-driving code lands across
fifteen separate sub-PRs (or one large PR, at the team's
discretion). Each implementation is a criterion-style benchmark
or an external workload generator targeting the relevant
subsystem's public surface.

**Step 3 — Quarterly chain integration (follow-up PR)**. The
CB-suite outputs are consolidated into the quarterly evidence
chain as a second signed summary document, alongside the
existing S-suite summary.

Step 1 takes approximately one engineer-week of focused work.
Step 2 takes approximately four engineer-weeks distributed across
the subsystem owners. Step 3 takes approximately one engineer-
week. The total work is six engineer-weeks, distributed across
two calendar months.

---

## 10. Audit budget

In keeping with the I.6 invariant from PR #25, this PR ships with
the following explicit audit budget :

- **Implementation** : 5 calendar days (catalogue + scenarios +
  validator + STABILITY amendment).
- **Audit (per I.1)** : 2 calendar days.
- **Reserve** : 0.5 calendar day.
- **Total** : 7.5 calendar days.

The audit will follow the chirurgical multi-pass discipline
established in PR #25, with passes covering : schema layer
integrity, scenario-file conformance, STABILITY contract
consistency, test coverage, and documentation completeness.

---

## 11. Acceptance criteria

The PR is acceptance-ready when each of the following criteria is
met :

1. Thirty-five YAML scenario files exist under `bench/scenarios/`
   with names matching `CB-NN_*.yaml` for the thirty subsystem
   benchmarks (CB-01..CB-30) and `CB-AN_*.yaml` for the five
   meta-benchmarks (CB-A1..CB-A5).
2. Each YAML file conforms to the schema asserted by
   `bench/harness/tests/cb_catalogue.rs`.
3. STABILITY § 27 is present in `docs/STABILITY_CONTRACT.md` with
   the five sub-sections specified in § 8 above.
4. The freeze tests for § 27.1 through § 27.5 exist in
   `bench/harness/tests/stability_contract_v17.rs` and pass.
5. The orchestrator script `scripts/run-core-benchmarks.sh`
   exists, is executable, and lists each of the thirty-five
   benchmarks in its `--list` output.
6. The reference criterion implementation
   (`crates/efe-events/benches/cb_01_event_append.rs`) exists
   and compiles cleanly.
7. The catalogue document is referenced from the master incident-
   response runbook's section on benchmark-related incidents.
8. The PR's lessons file at `docs/lessons/PR-NN-lessons.md`
   passes the audit-prompt validator.

---

## 12. References

- Sibling architecture doc (PR #25 process invariants) :
  `EFE_A1_PHASE3_PR25_ARCHITECTURE.md`.
- Sibling architecture doc (PR #26 post-mortem) :
  `EFE_A1_PHASE3_PR26_ARCHITECTURE.md`.
- Existing scenario catalogue (S-suite, Meta-Swarm-dependent) :
  `bench/scenarios/S01_*.yaml` through `S10_*.yaml`.
- Existing bench harness :
  `bench/harness/src/`.
- STABILITY contract :
  `docs/STABILITY_CONTRACT.md` § 24 (the S-suite freeze) and
  § 27 (the new CB-suite freeze).

---

_Document author : bench-infra leadership._
_Last revised : 2026-05-17._
_Status : DRAFT, pending bench-infra leadership review on 2026-06-01._
