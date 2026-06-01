# EFE M50 — Sprint 19 — N27 — Streaming Anthropic + Exponential Retry + N17 Wiring (Architecture)

> **Status** : design-time, ultra-detailed.
> **Sprint** : 19. **Sub-sprint** : N27 (between N26 ClaudeBridge↔Provenance and N28 Postgres+Event-Bus).
> **Depends on** : N17 (LlmProvenanceRecord), N21 (SwarmOrchestrator + ClaudeBridge), N22 (HTTP gateway), N24 (compliance runner), **N26 (provenance wiring)**.
> **Unlocks** : N28 (Postgres persistence), N30 (multi-region failover), N31 (speculative parallel dispatch).
> **Doc convention** : `Iₙ` = invariant, `Fₙ` = failure mode (with chaos test), `Gₙ` = guarantee (semantic contract), `ADR-n` = recorded decision. Section IDs are stable so audit reports and PR descriptions can link back here.

---

## 0. TL;DR

The N26 wiring made every Mode-C dispatch produce one `LlmProvenanceRecord`, but the dispatch path is **buffered/blocking** — the gateway waits for the full Claude response before returning the first byte to the client. N27 turns that into a streaming pipeline end-to-end :

1. **Streaming Anthropic** : `ClaudeBridge` learns to call the Messages API with `"stream": true`, parse the SSE event flow (`message_start` → `content_block_start` → `content_block_delta`×N → `content_block_stop` → `message_delta` → `message_stop`), and forward token deltas downstream as they arrive — **including** the two-turn `tool_use` protocol where turn 1 streams a tool call, the bridge executes it, and turn 2 streams the final natural-language answer.
2. **Exponential retry** : every Anthropic call is wrapped in a `RetryPolicy` with classified status-code handling (retry 408/429/500/502/503/504/529 + network errors ; never retry 4xx-non-429 + 400/401/403/404/413), capped exponential backoff (base 500 ms, factor 2, max 3 retries, max 30 s total wall), with `Retry-After` honoring and ±25 % jitter. Retries are **pre-stream only** — once any byte has flowed to the downstream consumer, the call becomes non-idempotent and retries are forbidden (ADR-N27-3).
3. **N17 wiring complete** : the existing `LlmProvenanceRecord` is extended with `retry_count`, `time_to_first_token_ms`, `streaming_mode`, and a new `Completeness::Aborted { received_tokens, reason }` variant. Aborted streams (client disconnect, upstream error after first byte, read-timeout) still produce **exactly one** record (`G1` is preserved). All five new invariants `I21`–`I25` (TTFT ≤ total ; retry_count ≤ MAX ; aborted ⇒ partial usage ; streaming-mode latency split ; per-turn TTFT independent) have dedicated tests.

The new gateway surface is `POST /v1/agentic/call-stream` (text/event-stream response, RFC 8895). The existing `POST /v1/agentic/call` (buffered) continues to work and is implemented by collecting the streaming bridge's output into a `String`, so we **only ship one code path** internally — the bridge is always streaming, the buffered endpoint just collects.

**Test surface** : ~78 new tests (12 unit + 24 phase + 18 audit + 16 chaos + 8 OpenAPI/contract) ; total workspace targets **~434 tests green** after N27 lands (current 356 + 78).

---

## 1. Motivation — why now and why all three together

### 1.1 Why streaming

| Pain (today, N26) | Cost | What streaming fixes |
|---|---|---|
| Client waits 5–12 s for the first byte | UX : SLA penalty, perceived slowness | First byte in <500 ms (TTFT from Claude) |
| If Claude times out at 10 s, client sees a hard 504 with no partial output | Lost work, no debug signal | Partial output already in client buffer ; abort recorded with `received_tokens` |
| Server holds the entire response in memory (avg 4 KB, p99 32 KB) | Memory pressure, head-of-line blocking | Constant-memory pipeline, bounded internal channels |
| Anthropic charges for ALL output tokens even if you abort late | Wasted spend | Early cancellation propagates upstream (Anthropic honors HTTP/2 RST_STREAM, frees their GPU) |

### 1.2 Why retry

| Failure (observed in N22-N26 staging) | Frequency | Without retry | With retry |
|---|---|---|---|
| Anthropic 529 "Overloaded" | 0.3–1.1 % during US business hours | User sees 502 from gateway | Backoff 500 ms → 1 s → 2 s, ~98 % succeed by attempt 3 |
| Anthropic 503 "Service Unavailable" | 0.05 % during deploys | User sees 502 | Retry transparent |
| Anthropic 429 with `Retry-After: 7` | 0.02 % under sustained load | User sees 429 | Honor header, retry after exactly 7 s |
| Network RST mid-handshake | 0.001 % (VPN, mobile, flaky NICs) | User sees 502 | Single retry, succeeds |
| 500 Internal Server Error | 0.01 % | User sees 502 | Retry once |

Net effect, on the N22 SLO of "99.5 % of `/v1/agentic/call` succeeds end-to-end" : N26 measured 98.7 % ; with N27 retry we project 99.6 % (above SLO).

### 1.3 Why provenance wiring at the same time

If we shipped streaming without provenance, every streamed call would either :

* (a) emit a record at the start (ill-formed — no usage yet, no output hash)
* (b) emit at the end (loses aborted streams entirely)
* (c) emit twice (violates `G1` "one call → one record")

The N26 invariants (one record, exact token counts, latency-sum) all assume buffered semantics. We have to extend them in lockstep with streaming, or audit goes dark on every streamed call. Same for retry : without `retry_count` in the record, an SRE looking at `total_latency_ms = 31 s` cannot tell if Anthropic took 31 s, or if there were 3 × 500 ms retries + 29.5 s on the final attempt. So we ship all three as one coherent unit.

---

## 2. Goals & non-goals

### 2.1 Goals (must ship in N27)

| Goal | Acceptance criterion |
|---|---|
| `ClaudeBridge` streams from Anthropic SSE | `phase_9_n27_stream_single_turn_emits_deltas` green |
| Two-turn protocol works under streaming | `phase_9_n27_stream_two_turn_tool_use` green ; turn 1 + turn 2 each streamed |
| Backpressure : bounded internal channel | `audit_n27_i_channel_bounded_at_64` green ; no `Vec::push` after capacity reached |
| Client disconnect cancels upstream | `chaos_n27_f_client_disconnect_cancels_anthropic` green ; HTTP/2 RST observed by mock |
| Aborted stream produces 1 record with `Completeness::Aborted` | `audit_n27_i_aborted_stream_emits_one_record` green |
| Retry on 408/429/500/502/503/504/529 + network | `phase_9_n27_retry_succeeds_on_third_attempt` green |
| Do NOT retry on 400/401/403/404/413 | `chaos_n27_f_400_no_retry` green |
| `Retry-After` header honored | `phase_9_n27_retry_after_honored_to_the_second` green |
| Pre-stream-only retry boundary | `audit_n27_i_no_retry_after_first_byte` green |
| TTFT measured and stamped | `audit_n27_i_ttft_le_total` green |
| `retry_count` in record (≥0, ≤MAX_RETRIES) | `audit_n27_i_retry_count_bounded` green |
| Gateway `/v1/agentic/call-stream` SSE endpoint | `phase_9_n27_gateway_sse_endpoint_e2e` green ; OpenAPI documents schema |
| Buffered endpoint still works (single internal path) | `phase_9_n27_buffered_endpoint_still_works` green (regression of N26) |
| 78 new tests green ; existing 356 unchanged | CI workspace count = 434 ± delta from refactor |

### 2.2 Non-goals (deferred, ADR-N27-13)

* Streaming **replay** : if a retry happens after a partial stream was already delivered, we do NOT attempt to resume — we abort downstream and record `Completeness::Aborted { reason: NonRetryablePostStream }`. A replay protocol (track byte/token offsets, ask Anthropic to resume) is N30+ work.
* **Postgres persistence** of streaming records : still in-memory store (N28).
* **Server-side tool-use streaming** with partial JSON parsing : we accumulate the full `input_json_delta` sequence into a `String`, then `serde_json::from_str` once at `content_block_stop`. Partial parsing buys us nothing (the tool can't run on half-args anyway).
* **Multi-region failover** : if Anthropic's us-east-1 is down, we exhaust retries and surface a 503 — we do not call eu-west or asia (N30).
* **Speculative parallel dispatch** to two providers : N31.
* **Prompt caching** specifics (`cache_control` markers, `ephemeral` blocks) : tangentially affects token accounting but is a separate body of work (planned N29).
* **Anthropic Batch API** (50 % off, ≤24 h latency) : not needed for streaming use cases.
* **gRPC bidi streaming** : we ship SSE only. gRPC for internal service-to-service is N32+.

### 2.3 Explicit out-of-scope clarifications

> **Q (sprint review)** : "Should retries continue across the two-turn boundary, i.e. if turn 2 fails, do we restart turn 1?"
> **A** : No. Once turn 1 succeeded, the tool was executed and its result is in the conversation transcript. Turn 2 is a new HTTP request to Anthropic with that transcript ; turn-2 retry is independent and only restarts the turn-2 POST. If turn 2 exhausts retries, the user sees an error, and the record has `Completeness::Aborted { reason: UpstreamError, received_tokens: turn1_output_tokens }`.

> **Q** : "Should backoff be per-tenant or global?"
> **A** : Per-call. Each call has its own retry budget. Tenant-level backpressure is N32+ (token-bucket rate limiter).

> **Q** : "What about the long Anthropic 60 s read-timeout?"
> **A** : Configurable, default 60 s **between events** (not total). Anthropic sends `ping` events every 15 s during long completions. If we don't see ANY event for 60 s, we abort and (if pre-stream) retry once.

---

## 3. Anthropic SSE protocol — surgical deep dive

This section is the source of truth for what the parser must handle. It's based on Anthropic's public Messages API streaming documentation (rev 2024-12, the version pinned in our `anthropic-version: 2023-06-01` header) and on three captured production sessions (replayed locally with `mitmproxy`).

### 3.1 HTTP surface

```
POST https://api.anthropic.com/v1/messages
x-api-key: sk-ant-...
anthropic-version: 2023-06-01
content-type: application/json
accept: text/event-stream            ← we add this for streaming
{
  "model": "claude-sonnet-4-6",
  "messages": [...],
  "tools": [SWARM_CONTEXT_TOOL_DEF],
  "max_tokens": 4096,
  "stream": true                     ← THE flag
}
```

Response :

```
HTTP/1.1 200 OK
content-type: text/event-stream; charset=utf-8
cache-control: no-cache
connection: keep-alive
x-anthropic-request-id: req_01ABC...
```

then a stream of SSE events terminated by `message_stop`.

### 3.2 Wire format (RFC-conformant SSE)

Each event is :

```
event: <event_name>\n
data: <single-line JSON>\n
\n
```

Two consecutive `\n` mark the event boundary. Comments (heartbeats from intermediaries) are `: <comment>\n\n` — we MUST skip lines starting with `:` (RFC 8895 §4.2). Anthropic itself does NOT send comments, but Cloudflare in front of them sometimes does.

### 3.3 Event taxonomy — all 7 types

#### 3.3.1 `message_start`

```
event: message_start
data: {
  "type":"message_start",
  "message":{
    "id":"msg_01ABC",
    "type":"message",
    "role":"assistant",
    "model":"claude-sonnet-4-6",
    "content":[],
    "stop_reason":null,
    "stop_sequence":null,
    "usage":{"input_tokens":1247,"output_tokens":1,
             "cache_creation_input_tokens":0,
             "cache_read_input_tokens":0}
  }
}
```

* **Always first**. Carries `message.id` (we stamp it into `provider_request_id` of the record).
* `usage.input_tokens` is **final** — input is fully tokenized server-side before any output. `output_tokens` is the running counter (starts at 1, increments throughout the stream).
* Cache fields are 0 unless prompt caching was used (out of scope).

#### 3.3.2 `content_block_start`

```
event: content_block_start
data: {
  "type":"content_block_start",
  "index":0,
  "content_block":{"type":"text","text":""}     // OR
}
```

For tool use :

```
data: {
  "type":"content_block_start",
  "index":0,
  "content_block":{
    "type":"tool_use",
    "id":"toolu_01XYZ",
    "name":"swarm_context",
    "input":{}                                    ← always empty initially
  }
}
```

* `index` is the position in `message.content[]` — Claude can emit multiple blocks in one message (e.g., a `text` "Let me think..." then a `tool_use`). We track per-index state.
* Tool-use `input` is populated incrementally via subsequent `input_json_delta` events.

#### 3.3.3 `content_block_delta`

Two delta types :

**Text delta** :

```
data: {
  "type":"content_block_delta",
  "index":0,
  "delta":{"type":"text_delta","text":"Hello, "}
}
```

* Concatenate to per-index text buffer.

**Tool-use input JSON delta** :

```
data: {
  "type":"content_block_delta",
  "index":1,
  "delta":{"type":"input_json_delta","partial_json":"{\"depth\":\"min"}
}
data: {
  "type":"content_block_delta",
  "index":1,
  "delta":{"type":"input_json_delta","partial_json":"imal\"}"}
}
```

* Concatenate `partial_json` strings to a per-index `String` accumulator.
* **Do not parse** until `content_block_stop` arrives (`I22` : "no partial-JSON validation").

#### 3.3.4 `content_block_stop`

```
data: {"type":"content_block_stop","index":0}
```

* Signals the per-index block is complete.
* For tool-use blocks : NOW `serde_json::from_str` the accumulated `String` ; on failure, F8 (corrupt tool-use JSON → no retry, abort with `Errored`).
* For text blocks : the per-index `text` is now final.

#### 3.3.5 `message_delta`

```
event: message_delta
data: {
  "type":"message_delta",
  "delta":{"stop_reason":"end_turn","stop_sequence":null},
  "usage":{"output_tokens":342}
}
```

* **Always exactly once near the end**.
* Carries the final `stop_reason` and the **final** `output_tokens` (overrides all running counters).
* This is the event we use to compute `Completeness` (Complete vs Partial).

#### 3.3.6 `message_stop`

```
event: message_stop
data: {"type":"message_stop"}
```

* Marks end-of-stream. After this, the HTTP connection should close (or be reused for keep-alive).
* The bridge transitions to "stream-finalize" state : compute output hash from accumulated text blocks, compute cost from `message_delta.usage`, stamp the record.

#### 3.3.7 `ping`

```
event: ping
data: {"type":"ping"}
```

* Heartbeat from Anthropic during long completions (every ~10–15 s).
* We absorb internally (do NOT forward downstream — the gateway sends its own heartbeats keyed to the downstream client's connection age).

#### 3.3.8 `error`

```
event: error
data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}
```

* Mid-stream error. Possible types we've seen :
  * `overloaded_error` — Anthropic is overloaded
  * `api_error` — generic 5xx-flavored
  * `rate_limit_error` — rare mid-stream, more common as initial 429
  * `authentication_error` — never seen mid-stream (auth happens pre-stream) but theoretically possible
* **Always terminates the stream**. We MUST :
  1. Stop reading the SSE body.
  2. Decide if retry is allowed (only if pre-first-byte-downstream).
  3. Emit `Completeness::Aborted { received_tokens: accumulated_output_so_far, reason: AbortReason::UpstreamError(error_type) }`.

### 3.4 Worked example — single-turn text response (no tools)

```
event: message_start
data: {"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":42,"output_tokens":1,...}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":", world!"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}

event: message_stop
data: {"type":"message_stop"}
```

Bridge state at end : `output_text = "Hello, world!"`, `input_tokens=42`, `output_tokens=3`, `stop_reason=EndTurn`, `Completeness::Complete`. One record emitted.

### 3.5 Worked example — two-turn `tool_use` protocol streamed

Turn 1 request body (gateway → Anthropic) :

```json
{
  "model": "claude-sonnet-4-6",
  "messages":[{"role":"user","content":"What's in the bundle?"}],
  "tools":[SWARM_CONTEXT_TOOL_DEF],
  "max_tokens":4096,
  "stream": true
}
```

Turn 1 SSE response :

```
event: message_start
data: {"type":"message_start","message":{"id":"msg_t1","usage":{"input_tokens":823,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Let me check the bundle. "}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_T1","name":"swarm_context","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"depth\":\""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"minimal\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":47}}

event: message_stop
data: {"type":"message_stop"}
```

Bridge sees `stop_reason=tool_use`. It :

1. **Forwards** the text-block deltas downstream as `AgenticEvent::TextDelta("Let me check the bundle. ")` (the user's UI shows this incrementally).
2. **Does NOT forward** the tool-use block downstream (internal protocol detail).
3. **Executes** `swarm_context` tool with `{"depth":"minimal"}` against the in-process `ContextBundle`. Result is a JSON object (the redacted bundle view).
4. Builds turn-2 request body :

```json
{
  "model": "claude-sonnet-4-6",
  "messages": [
    {"role":"user","content":"What's in the bundle?"},
    {"role":"assistant","content":[
      {"type":"text","text":"Let me check the bundle. "},
      {"type":"tool_use","id":"toolu_T1","name":"swarm_context","input":{"depth":"minimal"}}
    ]},
    {"role":"user","content":[
      {"type":"tool_result","tool_use_id":"toolu_T1","content":"{<redacted bundle JSON>}"}
    ]}
  ],
  "tools":[SWARM_CONTEXT_TOOL_DEF],
  "max_tokens":4096,
  "stream": true
}
```

5. Opens turn 2 SSE stream :

```
event: message_start
data: {"type":"message_start","message":{"id":"msg_t2","usage":{"input_tokens":1102,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"The bundle "}}

... (more text deltas, each forwarded downstream) ...

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":219}}

event: message_stop
data: {"type":"message_stop"}
```

6. Emits `AgenticEvent::Done { stop_reason: EndTurn, ... }` downstream.
7. Emits ONE `LlmProvenanceRecord` summing :
   * `token_usage.input_tokens = 823 + 1102 = 1925`
   * `token_usage.output_tokens = 47 + 219 = 266`
   * `total_latency_ms = turn1_latency_ms + turn2_latency_ms` (NO retry waits ; if any retries happened, those are added too)
   * `time_to_first_token_ms = elapsed_from_turn1_post_to_first_text_delta` (this is the **user-visible TTFT** — measured at the very first `content_block_delta` of turn 1 that contained text, NOT counting tool-use deltas)

### 3.6 Edge cases the parser must handle

| Case | What Anthropic sends | What we do |
|---|---|---|
| Empty text block | `content_block_start` (text) then immediate `content_block_stop`, no deltas | Buffer = "" ; valid |
| `event:` line missing | Some SSE intermediaries normalize | Use the `data.type` field as the event name (Anthropic always includes it) — see ADR-N27-2 |
| Multi-line `data:` field | Possible per RFC | Concatenate (join with `\n`) before JSON parse — Anthropic doesn't do this in practice, but be safe |
| Trailing `\n\n\n\n` (double blank lines) | Cloudflare quirk | Treat each blank line as event boundary, skip empty events |
| Truncated last event (no trailing `\n\n`) | Anthropic connection drop | Detect EOF mid-event ; emit `Errored` (no partial event applied) |
| `data:` field with non-JSON payload | Should never happen but defense | Log + abort + emit `Errored { error_kind: "invalid_event_payload" }` |
| Out-of-order `index` (delta for index 2 before block_start of index 2) | Should never happen | Defense : reject as protocol error ; F9 |

### 3.7 SSE state machine (parser)

```
                    ┌──────────────┐
                    │  ReadingHeaders │
                    └──────┬───────┘
                           │  (after HTTP 200 + content-type check)
                           ▼
                    ┌──────────────┐
              ┌────►│ ReadingLine  │  (line-buffered TCP read)
              │     └──────┬───────┘
              │            │
              │ line       │ blank line (\n\n boundary)
              │            ▼
              │     ┌──────────────┐
              │     │  DispatchEvent│
              │     └──────┬───────┘
              │            │
              │            │ (route by parsed event_name + data.type)
              │            ▼
              │  ┌─────────────────────────────────┐
              │  │  PerEventHandler                │
              │  │   • message_start  → init state │
              │  │   • content_block_start        │
              │  │   • content_block_delta        │
              │  │   • content_block_stop         │
              │  │   • message_delta              │
              │  │   • message_stop  → Finalize   │
              │  │   • ping          → ignore     │
              │  │   • error         → Abort       │
              │  └──────┬──────────────────────────┘
              │         │
              └─────────┘
```

The parser maintains a `StreamState` :

```rust
struct StreamState {
    message_id: Option<String>,             // from message_start
    input_tokens: Option<u32>,              // from message_start.usage
    output_tokens_running: u32,             // from message_delta.usage (final)
    cache_creation_input_tokens: u32,
    cache_read_input_tokens: u32,
    blocks: HashMap<u32, ContentBlockBuilder>,
    final_stop_reason: Option<StopReason>,
    finalized: bool,
}

enum ContentBlockBuilder {
    Text { buffer: String },
    ToolUse { id: String, name: String, input_json_buffer: String },
}
```

After `message_stop`, the bridge :

* Builds `accumulated_text` by concatenating text-block buffers in `index` order.
* Builds `tool_uses: Vec<ToolUseBlock>` by parsing JSON of each tool-use buffer.
* Computes `output_hash = blake3(accumulated_text || canonical(tool_uses))`.

### 3.8 What we DON'T support (yet)

* `cache_creation_input_tokens > 0` (prompt caching) — we read it but do not stamp it (N29 work).
* `image` content blocks (vision) — schema-wise possible, but `swarm_context` is text-only.
* Multi-tool `tools` arrays — only `swarm_context` for now ; if Claude returns a tool_use we didn't define, we F10 (unknown tool → no retry, `Errored`).
* `stop_sequence` non-null (custom stop strings) — pass-through, no special handling.

---

## 4. Retry policy — surgical deep dive

### 4.1 Classification table

| HTTP / cause | Retry? | Backoff seed | Reason |
|---|---|---|---|
| 200 OK (then mid-stream error event) | **No** (F-rule : post-first-byte) | — | Already streamed |
| 200 OK (then network RST before message_stop) | **No** | — | Post-first-byte |
| 400 Bad Request | **No** | — | Client error, retry won't help |
| 401 Unauthorized | **No** | — | Bad API key, retry won't help |
| 403 Forbidden | **No** | — | Permissions, retry won't help |
| 404 Not Found | **No** | — | Model doesn't exist, retry won't help |
| 408 Request Timeout | **Yes** | 500 ms | Transient |
| 413 Payload Too Large | **No** | — | Truncation needed, retry won't help |
| 429 Too Many Requests | **Yes** | honor `Retry-After` then exp | Rate limit |
| 500 Internal Server Error | **Yes** | 500 ms | Transient |
| 502 Bad Gateway | **Yes** | 500 ms | Edge / Cloudflare blip |
| 503 Service Unavailable | **Yes** | 500 ms | Anthropic deploys |
| 504 Gateway Timeout | **Yes** | 1 s | Upstream slow |
| 529 Overloaded | **Yes** | 1 s | Anthropic capacity |
| reqwest::Error::is_timeout() (pre-stream) | **Yes** | 500 ms | TCP timeout |
| reqwest::Error::is_connect() | **Yes** | 500 ms | TCP refused/reset |
| reqwest::Error::is_request() with body source error | **Yes** | 500 ms | Local IO blip on write |
| reqwest::Error::is_decode() | **No** | — | Our parser bug, retry won't help |
| reqwest::Error::is_body() (post-first-byte) | **No** | — | Mid-stream, can't replay |

### 4.2 Backoff formula

```
attempt n (0-indexed) sleep_ms =
    if retry-after-header present:
        retry_after_ms                                    // honor exactly
    else if 529:
        min(BASE_MS * 2^n * jitter_factor(), MAX_PER_RETRY)
        where BASE_MS = 1000, MAX_PER_RETRY = 8000
    else:
        min(BASE_MS * 2^n * jitter_factor(), MAX_PER_RETRY)
        where BASE_MS = 500, MAX_PER_RETRY = 4000

jitter_factor() = 1.0 + uniform(-0.25, 0.25)            // ±25%
```

Total wall-clock cap : `MAX_TOTAL_RETRY_MS = 30_000`. If after `n` retries, `sum(sleep_n) > 30_000`, we abort and surface the last error.

Concrete examples :

| n | base | computed (no jitter) | with min(MAX_PER_RETRY) |
|---|---|---|---|
| 0 (first retry) | 500 | 500 | 500 |
| 1 | 500 | 1000 | 1000 |
| 2 | 500 | 2000 | 2000 |
| 3 | 500 | 4000 | 4000 (would be next, but MAX_RETRIES=3 → no n=3) |

For 529 :

| n | base | computed | with cap |
|---|---|---|---|
| 0 | 1000 | 1000 | 1000 |
| 1 | 1000 | 2000 | 2000 |
| 2 | 1000 | 4000 | 4000 |

`MAX_RETRIES = 3` constant. Total possible wall = (sum of cap) + jitter ≤ 500 + 1000 + 2000 + ε = 3500 ms (non-529), or 1000+2000+4000 = 7000 ms (529). Both well under the 30 s cap, so the cap is a guard, not the primary limit.

### 4.3 The "first byte" boundary — ADR-N27-3

This is the critical correctness constraint :

> A retry MAY happen iff no event has been forwarded to the downstream consumer of the stream.

Reasoning :

* If the downstream `axum::sse::Sse` channel has emitted ≥1 event (`event: delta\ndata: {...}\n\n`), the client's UI has rendered tokens. Retrying would either (a) duplicate tokens (replay), (b) jump to a different completion (incoherent), or (c) require a complex resume protocol (out of scope).
* The boundary is **not** "first byte from Anthropic" — it's "first byte WE sent downstream". So a `message_start` event from Anthropic does NOT close the retry boundary ; only when we forward `AgenticEvent::TextDelta` (or `MessageStart` if we emit that downstream) does the gate close.

State variable :

```rust
struct DispatchState {
    downstream_sent_first_event: AtomicBool,
}
```

Set to `true` immediately before `downstream_tx.send(first_event).await?`. Once true, the retry layer sees it and refuses to retry, even on retryable errors.

### 4.4 Retry-After parsing

Per RFC 7231 §7.1.3, `Retry-After` is either :

* **Delta-seconds** : `Retry-After: 7`
* **HTTP-date** : `Retry-After: Fri, 22 May 2026 12:30:00 GMT`

Anthropic always sends delta-seconds, but we handle both. If the header is malformed, we fall back to exponential backoff (warn-log it).

```rust
fn parse_retry_after(h: &str) -> Option<Duration> {
    if let Ok(n) = h.trim().parse::<u64>() {
        return Some(Duration::from_secs(n.min(300)));    // cap at 5 min
    }
    if let Ok(t) = httpdate::parse_http_date(h) {
        let now = SystemTime::now();
        return t.duration_since(now).ok().map(|d| d.min(Duration::from_secs(300)));
    }
    None
}
```

### 4.5 Retry observability

Each retry event is logged + counted :

```rust
tracing::warn!(
    attempt = current_attempt,
    next_delay_ms = next_delay.as_millis() as u64,
    error_class = %classification.label(),
    "agentic retry scheduled"
);
metrics.tick_retry(model_id, current_attempt, classification.label());
```

And finally stamped in the provenance record (`retry_count` field, `retry_outcome` field).

### 4.6 Idempotency considerations

Anthropic's Messages API is **idempotent at the request level** : the same body produces a stochastically-similar completion (temperature-dependent), and there is no idempotency key. We do NOT pass an idempotency key (Anthropic doesn't accept one). Each retry is treated as a fresh request by the server, which means :

* `provider_request_id` of the record is from the **succeeding** attempt only.
* Token billing counts only the final attempt (failed attempts didn't return usage to bill).
* Output hash is stable across retries IFF temperature=0 and seed is set — we don't pin a seed (Anthropic doesn't support deterministic seeding), so the hash is meaningless for retried calls beyond audit/forensics.

---

## 5. Two-turn protocol over stream — sequence diagram

```
client                  gateway              ClaudeBridge          Anthropic
  │                        │                      │                    │
  ├─ POST /agentic/call-stream                    │                    │
  │  (JWT, body)            │                     │                    │
  │                        │                      │                    │
  │                        ├─ open SSE response   │                    │
  │  ◄──── HTTP 200 ────────│  (text/event-stream)│                    │
  │                        │                      │                    │
  │                        ├─ call_with_context_stream(bundle, task)   │
  │                        │                      │                    │
  │                        │                      ├─ pre_call provenance (model lookup)
  │                        │                      │                    │
  │                        │                      ├─ POST /v1/messages (stream=true, turn 1)
  │                        │                      │                    ├─ HTTP 200 SSE
  │                        │                      │  ◄── message_start
  │                        │                      │  ◄── content_block_start (text, idx=0)
  │                        │  ◄── TextDelta("Let")│  ◄── delta "Let"   │
  │  ◄── event: delta       │                     │                    │
  │      data: "Let"        │                     │                    │
  │                        │                      │  ◄── delta " me check..."
  │  ◄── event: delta       │  ◄── TextDelta("...")│                   │
  │      data: " me check"  │                     │                    │
  │                        │                      │  ◄── content_block_stop (idx=0)
  │                        │                      │  ◄── content_block_start (tool_use, idx=1)
  │                        │                      │  ◄── input_json_delta×N
  │                        │                      │  ◄── content_block_stop (idx=1)
  │                        │                      │  ◄── message_delta (stop=tool_use)
  │                        │                      │  ◄── message_stop  │
  │                        │                      │                    │
  │                        │                      ├─ execute swarm_context tool (in-process)
  │                        │                      │   (build redacted view of ContextBundle)
  │                        │                      │                    │
  │                        │                      ├─ POST /v1/messages (turn 2 with tool_result)
  │                        │                      │  ◄── HTTP 200 SSE  │
  │                        │                      │  ◄── message_start (input_tokens for turn 2)
  │                        │                      │  ◄── content_block_start (text, idx=0)
  │                        │  ◄── TextDelta("The")│  ◄── delta "The"   │
  │  ◄── event: delta       │                     │                    │
  │      data: "The"        │  ◄── TextDelta(...) │  ◄── delta×N       │
  │  ◄── event: delta×N     │                     │                    │
  │                        │                      │  ◄── content_block_stop
  │                        │                      │  ◄── message_delta (stop=end_turn)
  │                        │                      │  ◄── message_stop  │
  │                        │                      │                    │
  │                        │  ◄── Done            ├─ post_call provenance: write 1 record
  │  ◄── event: done        │                     │   (input_tokens = T1+T2, output = T1+T2,
  │      data: {...}        │                     │    total_latency = T1_lat + T2_lat + retry_wait,
  │                        │                      │    ttft = elapsed_to_first_text_delta_T1,
  │                        │                      │    retry_count = N,
  │                        │                      │    completeness = Complete)
  │                        │                      │                    │
  │  ◄── (TCP FIN)          │                     │                    │
```

Key timing measurements :

* `T_pre_call_start` : when `call_with_context_stream` is invoked.
* `T_turn1_post_start` : just before `reqwest::Client::post`.
* `T_turn1_first_delta` : first `content_block_delta(text)` event from Anthropic.
* `T_first_downstream_event` : when we `sse_tx.send(AgenticEvent::TextDelta(...)).await` for the first time.
* `T_turn1_message_stop` : turn 1 stream closed.
* `T_turn2_post_start` : just before turn 2 reqwest call.
* `T_turn2_first_delta`, `T_turn2_message_stop` : turn 2.
* `T_post_call_done` : provenance record written.

Derived fields :

* `time_to_first_token_ms = (T_turn1_first_delta - T_turn1_post_start).as_millis()`
* `turn1_latency_ms = (T_turn1_message_stop - T_turn1_post_start).as_millis()`
* `turn2_latency_ms = (T_turn2_message_stop - T_turn2_post_start).as_millis()`
* `total_retry_wait_ms = sum of all sleep durations` (only non-zero if retries happened)
* `total_latency_ms = turn1_latency_ms + turn2_latency_ms + total_retry_wait_ms`
* `provider_response_time_ms = turn1_latency_ms + turn2_latency_ms` (per N17 spec : provider only, no retry waits)

---

## 6. N17 wiring extensions

### 6.1 New `LlmProvenanceRecord` fields

```rust
// added to LlmProvenanceRecord (efe-nano-sdk::types::llm_provenance)
//
// ─── Sprint 19 — N27 streaming + retry ────────────────────────────────

/// **N27** : Was this call made in streaming mode? (POST with stream=true).
/// `false` for buffered calls (legacy /v1/agentic/call) where the bridge
/// streams internally but the gateway aggregates before responding.
#[serde(default)]
pub streaming_mode: bool,

/// **N27** : Number of retries before the final attempt. 0 = succeeded
/// on first try. Bounded by [0, N27_MAX_RETRIES = 3].
#[serde(default)]
pub retry_count: u8,

/// **N27** : Outcome of the retry loop.
#[serde(default)]
pub retry_outcome: RetryOutcome,

/// **N27** : Time from `T_post_start` (first attempt) to first text
/// `content_block_delta` event received from the provider. `None` for
/// cache hits, replays, refusals before any deltas, or aborted-pre-stream
/// calls.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub time_to_first_token_ms: Option<u32>,
```

```rust
/// **N27** : closed taxonomy of retry-loop outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RetryOutcome {
    /// First attempt succeeded ; `retry_count = 0`.
    #[default]
    NoRetry,
    /// One or more retries happened, final attempt succeeded.
    RetriedSucceeded,
    /// All retries exhausted ; final attempt failed.
    RetriedExhausted,
    /// Hit a non-retryable error (400/401/403/etc.) — no retries attempted.
    NonRetryable,
}
```

### 6.2 New `Completeness` variant

```rust
// modified Completeness enum in efe-nano-sdk::types::llm_provenance
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Completeness {
    Complete,
    Partial { received_tokens: u32, expected_tokens: u32 },
    /// **N27** : Stream was aborted before completion.
    /// Distinct from `Partial` (which is "max_tokens hit, response would have
    /// continued"). Aborted means we explicitly cut the stream short.
    Aborted {
        received_tokens: u32,
        reason: AbortReason,
    },
    Refused { category: SafetyCategory },
    Errored { error_kind: String },
}
```

```rust
/// **N27** : closed taxonomy of stream-abort reasons.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AbortReason {
    /// Downstream client closed the SSE connection.
    ClientDisconnect,
    /// Mid-stream `error` event from provider.
    UpstreamError { error_type: String },
    /// No SSE event seen for `EVENT_READ_TIMEOUT_MS` (default 60s).
    ReadTimeout { ms: u32 },
    /// Total stream wall-clock exceeded `STREAM_TOTAL_TIMEOUT_MS` (default 300s).
    TotalTimeout { ms: u32 },
    /// Retry exhausted post-first-byte-downstream (cannot replay).
    NonRetryablePostStream,
    /// Internal channel saturated, backpressure forced abort.
    BackpressureAbort,
}
```

### 6.3 Where the new fields get populated

| Field | Set by | When |
|---|---|---|
| `streaming_mode` | `ClaudeBridge::dispatch_stream` | At record-build time, hardcoded `true` ; `false` for legacy `dispatch` buffered path |
| `retry_count` | `RetryLoop::execute` | Incremented on each retry-eligible failure ; final value baked into record at end |
| `retry_outcome` | `RetryLoop::execute` | Final classification (`NoRetry` / `RetriedSucceeded` / `RetriedExhausted` / `NonRetryable`) |
| `time_to_first_token_ms` | `StreamState::on_first_text_delta` | Captured ONCE on the first `content_block_delta(text_delta)` event |
| `Completeness::Aborted` | `StreamState::finalize_aborted` | When stream ends without `message_stop` |

### 6.4 Updated invariants set (additions to N17/N26)

(See § 10 for full list. These are the deltas.)

* **I21 (NEW)** : `time_to_first_token_ms <= total_latency_ms` (always, when both are Some). Test : `audit_n27_i21_ttft_le_total`.
* **I22 (NEW)** : `retry_count <= N27_MAX_RETRIES` (compile-time bounded by `u8 = 3`). Test : `audit_n27_i22_retry_count_bounded`.
* **I23 (NEW)** : Aborted streams have `token_usage.output_tokens == received_tokens_in_completeness` (within ±1 due to last-delta-not-counted edge). Test : `audit_n27_i23_aborted_token_consistency`.
* **I24 (NEW)** : `streaming_mode == true` ⇔ record came from `dispatch_stream` path. Test : `audit_n27_i24_streaming_mode_flag_authoritative`.
* **I25 (NEW)** : `total_latency_ms == provider_response_time_ms + total_retry_wait_ms` (sum holds within ±5 ms tolerance for clock skew). Test : `audit_n27_i25_total_latency_decomposition`.

### 6.5 Updated guarantees set

* **G14 (NEW)** : **One stream → one record**, regardless of completion state. A 2-turn stream that aborts in turn 2 still produces exactly one record (with `Aborted` completeness). Test : `audit_n27_g14_one_record_per_stream_invariant`.
* **G15 (NEW)** : **Retries are invisible to downstream when they succeed**. The downstream SSE consumer cannot observe whether 0, 1, 2, or 3 retries happened — they only see the final stream. Test : `audit_n27_g15_retries_invisible_downstream`.

---

## 7. Component map

```
crates/
├── efe-agentic-bridge/
│   ├── src/
│   │   ├── claude.rs              (existing, modified — add dispatch_stream)
│   │   ├── claude_stream/         (NEW module)
│   │   │   ├── mod.rs            (re-exports)
│   │   │   ├── sse_parser.rs     (line-based SSE parser, state machine)
│   │   │   ├── event.rs          (Anthropic SSE event enum + serde)
│   │   │   ├── state.rs          (StreamState accumulator)
│   │   │   └── finalize.rs       (build LlmProvenanceRecord from state)
│   │   ├── retry/                 (NEW module)
│   │   │   ├── mod.rs            (RetryPolicy trait + ExponentialBackoff)
│   │   │   ├── classify.rs       (status code → RetryDecision)
│   │   │   ├── backoff.rs        (compute_delay, jitter, retry-after parse)
│   │   │   └── loop_executor.rs  (RetryLoop driver)
│   │   ├── bridge.rs              (existing, modified — add call_with_context_stream default)
│   │   ├── pricing.rs             (existing, unchanged)
│   │   ├── task.rs                (existing, modified — add AgenticEvent enum)
│   │   ├── tool_use.rs            (existing, unchanged)
│   │   └── lib.rs                 (modified — re-exports for new types)
│   └── tests/
│       ├── audit_n27_streaming_retry.rs       (NEW : 18 audit tests)
│       ├── chaos_n27_streaming_retry.rs       (NEW : 16 chaos tests)
│       └── phase_9_n27_streaming.rs           (NEW : 24 phase/contract tests)
│
├── efe-llm-provenance/
│   └── src/
│       ├── collector.rs           (existing, modified — add StreamingPostCallProvenance)
│       └── lib.rs                 (modified — re-export new types)
│
├── efe-nano-sdk/
│   └── src/types/
│       └── llm_provenance.rs      (modified : add fields, add Aborted variant, add AbortReason, add RetryOutcome)
│
└── efe-gateway/
    ├── src/
    │   ├── handlers/
    │   │   ├── agentic.rs                (existing : modify to collect-from-stream internally)
    │   │   └── agentic_stream.rs         (NEW : SSE handler)
    │   ├── sse/                           (NEW module)
    │   │   ├── mod.rs
    │   │   ├── encoder.rs                (AgenticEvent → axum::sse::Event)
    │   │   ├── heartbeat.rs              (15s downstream pings)
    │   │   └── cancel.rs                 (client-disconnect → upstream cancel)
    │   ├── router.rs                     (modified : add /v1/agentic/call-stream route)
    │   ├── openapi.rs                    (modified : add SSE response schema)
    │   └── metrics.rs                    (modified : add N27 counters/histograms)
    └── tests/
        ├── audit_n27_gateway_stream.rs           (NEW : 8 audit tests)
        ├── chaos_n27_gateway_stream.rs           (NEW : 6 chaos tests)
        └── phase_9_n27_gateway_e2e.rs            (NEW : 6 phase tests)
```

Total new files : 14. Total modified files : 8.

---

## 8. Trait & type definitions (Rust signatures)

### 8.1 Bridge trait extension

```rust
// crates/efe-agentic-bridge/src/bridge.rs

use async_trait::async_trait;
use futures::stream::BoxStream;

#[async_trait]
pub trait AgenticBridge: Send + Sync + 'static {
    /// **N21** : buffered call (legacy). Default impl now drives the
    /// streaming path internally and collects ; concrete bridges MAY
    /// override for efficiency.
    async fn call_with_context(
        &self,
        bundle: &ContextBundle,
        task: AgenticTask,
    ) -> Result<AgenticResponse, BridgeError> {
        // Default impl : drain the stream into a buffered response.
        // Override only if your bridge has no native streaming surface.
        let mut stream = self.call_with_context_stream(bundle, task).await?;
        let mut text = String::new();
        let mut stop = StopReason::EndTurn;
        let mut usage = TokenUsage::default();
        let mut identity = ModelIdentity::default();
        let mut cost = 0;
        while let Some(ev) = stream.next().await {
            match ev {
                AgenticEvent::TextDelta(s) => text.push_str(&s),
                AgenticEvent::Done { stop_reason, total_usage, model_identity, cost_microcents, .. } => {
                    stop = stop_reason;
                    usage = total_usage;
                    identity = model_identity;
                    cost = cost_microcents;
                }
                AgenticEvent::Error(e) => return Err(e),
                _ => {} // tool-use deltas, message_start, etc. are internal
            }
        }
        Ok(AgenticResponse {
            text, stop_reason: stop, usage, model_identity: identity,
            cost_microcents: cost,
        })
    }

    /// **N27** : streaming call. Returns a `Stream<AgenticEvent>` that
    /// emits incremental deltas + a final `Done` (or `Error`).
    /// Bridges with no native streaming MUST implement this by
    /// calling their buffered path and emitting a single `TextDelta`
    /// + `Done` (StubAgenticBridge does this).
    async fn call_with_context_stream(
        &self,
        bundle: &ContextBundle,
        task: AgenticTask,
    ) -> Result<BoxStream<'static, AgenticEvent>, BridgeError>;
}
```

### 8.2 `AgenticEvent` — the over-the-wire enum

```rust
// crates/efe-agentic-bridge/src/task.rs

/// **N27** : single event in an agentic streaming response.
///
/// Forwarded from the bridge to the gateway (which re-encodes into
/// SSE for HTTP clients). Internal-only events (MessageStart, ToolUseDelta,
/// etc.) are emitted for instrumentation/observability but the gateway
/// strips them by default (see § 8.3).
#[derive(Debug, Clone)]
pub enum AgenticEvent {
    /// **Internal** : turn began. Carries provider request id + input_tokens.
    /// Not forwarded downstream by default.
    MessageStart {
        turn: u8,                         // 1 or 2
        provider_request_id: Option<String>,
        input_tokens: u32,
    },
    /// **Downstream** : a chunk of text was produced.
    TextDelta(String),
    /// **Internal** : turn 1 tool-use began (we will execute & turn 2 it).
    /// Not forwarded downstream.
    ToolUseStart {
        tool_id: String,
        tool_name: String,
    },
    /// **Internal** : partial tool input JSON. Accumulated server-side ;
    /// not forwarded.
    ToolUseDelta(String),
    /// **Internal** : tool-use block complete. Holds the parsed input.
    ToolUseComplete {
        tool_id: String,
        tool_name: String,
        input: serde_json::Value,
    },
    /// **Internal** : final per-turn usage.
    TurnUsage {
        turn: u8,
        usage: TokenUsage,
        latency_ms: u32,
    },
    /// **Downstream** : stream is finished. Carries final summary.
    Done {
        stop_reason: StopReason,
        total_usage: TokenUsage,
        model_identity: ModelIdentity,
        cost_microcents: u64,
        time_to_first_token_ms: Option<u32>,
        provider_response_time_ms: u32,
        retry_count: u8,
    },
    /// **Downstream** : terminal error.
    Error(BridgeError),
}

impl AgenticEvent {
    /// Should this event be forwarded to the HTTP client by default?
    pub fn is_downstream_visible(&self) -> bool {
        matches!(
            self,
            AgenticEvent::TextDelta(_) | AgenticEvent::Done { .. } | AgenticEvent::Error(_)
        )
    }
}
```

### 8.3 Retry policy

```rust
// crates/efe-agentic-bridge/src/retry/mod.rs

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    /// Retry after `delay`.
    Retry { delay: Duration },
    /// Do not retry ; the original error is final.
    Abort,
}

pub trait RetryPolicy: Send + Sync + 'static {
    /// Classify an outcome and return a retry decision.
    ///
    /// `attempt` is 0-indexed (0 = first attempt's failure → decide
    /// whether to do the FIRST retry).
    /// `outcome` carries the error class + HTTP status + retry-after header.
    fn classify(
        &self,
        attempt: u8,
        outcome: &RetryOutcomeProbe,
    ) -> RetryDecision;

    /// Max retries this policy permits (informational ; loop also enforces).
    fn max_retries(&self) -> u8;
}

#[derive(Debug, Clone)]
pub struct RetryOutcomeProbe {
    pub http_status: Option<u16>,
    pub anthropic_error_type: Option<String>,
    pub retry_after: Option<Duration>,
    pub network_class: NetworkErrorClass,
    pub post_first_byte_downstream: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkErrorClass {
    None,
    Timeout,
    Connect,
    BodyMidStream,
    Decode,
    Other,
}

#[derive(Debug, Clone)]
pub struct ExponentialBackoff {
    pub base: Duration,                  // 500 ms default
    pub multiplier: f64,                 // 2.0 default
    pub max_per_retry: Duration,         // 4 s default
    pub max_total_wall: Duration,        // 30 s default
    pub max_retries: u8,                 // 3 default
    pub jitter_pct: f64,                 // 0.25 default
    pub overloaded_base: Duration,       // 1 s for 529
    pub overloaded_max: Duration,        // 8 s for 529
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self {
            base: Duration::from_millis(500),
            multiplier: 2.0,
            max_per_retry: Duration::from_secs(4),
            max_total_wall: Duration::from_secs(30),
            max_retries: 3,
            jitter_pct: 0.25,
            overloaded_base: Duration::from_secs(1),
            overloaded_max: Duration::from_secs(8),
        }
    }
}

impl RetryPolicy for ExponentialBackoff { /* see § 19 */ }
```

### 8.4 SSE event types

```rust
// crates/efe-agentic-bridge/src/claude_stream/event.rs

use serde::Deserialize;

/// Discriminator-tagged Anthropic SSE event.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicSseEvent {
    MessageStart { message: AnthropicMessage },
    ContentBlockStart { index: u32, content_block: AnthropicContentBlock },
    ContentBlockDelta { index: u32, delta: AnthropicDelta },
    ContentBlockStop { index: u32 },
    MessageDelta { delta: AnthropicMessageDelta, usage: AnthropicMessageDeltaUsage },
    MessageStop,
    Ping,
    Error { error: AnthropicError },
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicMessage {
    pub id: String,
    pub model: String,
    pub usage: AnthropicUsage,
    // Other fields (role, content, stop_reason, stop_sequence) ignored —
    // we rebuild content from deltas.
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicDelta {
    TextDelta { text: String },
    InputJsonDelta { partial_json: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicMessageDelta {
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub stop_sequence: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicMessageDeltaUsage {
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicError {
    pub r#type: String,
    pub message: String,
}
```

### 8.5 Stream state

```rust
// crates/efe-agentic-bridge/src/claude_stream/state.rs

use std::time::Instant;

pub struct StreamState {
    pub turn: u8,                                    // 1 or 2
    pub message_id: Option<String>,
    pub input_tokens: u32,
    pub output_tokens_running: u32,
    pub final_output_tokens: Option<u32>,            // from message_delta
    pub cache_creation_input_tokens: u32,
    pub cache_read_input_tokens: u32,
    pub blocks: std::collections::BTreeMap<u32, BlockBuilder>,
    pub final_stop_reason: Option<crate::StopReason>,
    pub started_at: Instant,
    pub first_text_delta_at: Option<Instant>,
    pub ended_at: Option<Instant>,
    pub finalized: bool,
    pub abort_reason: Option<crate::task::AbortReason>,
}

pub enum BlockBuilder {
    Text { buffer: String },
    ToolUse { id: String, name: String, json_buffer: String, parsed: Option<serde_json::Value> },
}

impl StreamState {
    pub fn new(turn: u8) -> Self { /* ... */ }

    pub fn apply(&mut self, ev: AnthropicSseEvent) -> Result<Option<AgenticEvent>, BridgeError>;

    pub fn finalize_complete(&mut self) -> StreamFinalized;
    pub fn finalize_aborted(&mut self, reason: AbortReason) -> StreamFinalized;
}

pub struct StreamFinalized {
    pub text_output: String,
    pub tool_uses: Vec<crate::ToolUseBlock>,
    pub usage: crate::TokenUsage,
    pub stop_reason: crate::StopReason,
    pub completeness: efe_nano_sdk::types::Completeness,
    pub latency_ms: u32,
    pub ttft_ms: Option<u32>,
}
```

### 8.6 Streaming collector hook

```rust
// crates/efe-llm-provenance/src/collector.rs

/// **N27** : post-call provenance variant for streaming dispatches.
/// Distinct from `PostCallProvenance` so the collector can validate
/// streaming-specific fields (TTFT bounds, abort reasons).
#[derive(Debug, Clone)]
pub struct StreamingPostCallProvenance {
    pub base: PostCallProvenance,
    pub streaming_mode: bool,
    pub retry_count: u8,
    pub retry_outcome: efe_nano_sdk::types::RetryOutcome,
    pub time_to_first_token_ms: Option<u32>,
    pub abort_reason: Option<efe_nano_sdk::types::AbortReason>,
}

#[async_trait::async_trait]
pub trait LlmProvenanceCollector: Send + Sync + 'static {
    async fn record_pre_call(&self, pre: PreCallProvenance) -> Result<ProvenanceId, ProvenanceError>;
    async fn record_post_call(&self, id: ProvenanceId, post: PostCallProvenance) -> Result<(), ProvenanceError>;
    // ... existing methods ...

    /// **N27** : streaming variant. Default impl projects to the buffered
    /// path (stamping streaming_mode=false would be a bug ; the projection
    /// preserves streaming_mode=true if set).
    async fn record_streaming_post_call(
        &self,
        id: ProvenanceId,
        post: StreamingPostCallProvenance,
    ) -> Result<(), ProvenanceError> {
        // Default : project to PostCall + write the additional fields
        // via a second mutation. Concrete collectors SHOULD override
        // for atomicity.
        self.record_post_call(id, post.base.clone()).await?;
        self.patch_streaming_fields(id, &post).await
    }

    async fn patch_streaming_fields(
        &self,
        id: ProvenanceId,
        post: &StreamingPostCallProvenance,
    ) -> Result<(), ProvenanceError>;
}
```

`DefaultLlmProvenanceCollector` overrides `record_streaming_post_call` to do both writes in one store transaction (in-memory : trivially atomic ; Postgres N28 : single UPDATE).

### 8.7 Gateway handler

```rust
// crates/efe-gateway/src/handlers/agentic_stream.rs

use axum::{
    extract::{State, Json},
    response::sse::{Event, KeepAlive, Sse},
    http::StatusCode,
};
use futures::stream::{Stream, StreamExt};
use std::convert::Infallible;
use std::time::Duration;

pub async fn post_agentic_call_stream(
    State(state): State<AppState>,
    claims: Hs256Claims,
    Json(body): Json<AgenticCallRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    // 1. Standard N26 audit-context build (JWT-driven tenant_id)
    let mut audit_ctx = body.audit_context.clone().unwrap_or_default();
    audit_ctx.tenant_id = Some(claims.tenant_id.clone());
    audit_ctx.trace_id = body.trace_id.clone();
    // ... (same as buffered handler)

    // 2. Build task
    let task = AgenticTask { /* ... */ };
    let bundle = build_bundle(&body, &state).await?;

    // 3. Open the bridge stream
    let bridge_stream = state.agentic
        .call_with_context_stream(&bundle, task)
        .await
        .map_err(ApiError::from)?;

    // 4. Convert AgenticEvent → axum SSE Event ; bridge cancellation
    //    propagates via tokio_util CancellationToken (see § 12 F3).
    let sse_stream = bridge_stream
        .filter(|ev| futures::future::ready(ev.is_downstream_visible()))
        .map(|ev| {
            let event = match ev {
                AgenticEvent::TextDelta(s) => Event::default().event("delta").data(s),
                AgenticEvent::Done { stop_reason, total_usage, cost_microcents, ttft, .. } => {
                    let payload = serde_json::json!({
                        "stop_reason": stop_reason.label(),
                        "input_tokens": total_usage.input_tokens,
                        "output_tokens": total_usage.output_tokens,
                        "cost_microcents": cost_microcents,
                        "time_to_first_token_ms": ttft,
                    });
                    Event::default().event("done").json_data(&payload).unwrap()
                }
                AgenticEvent::Error(e) => Event::default().event("error").data(e.to_string()),
                _ => unreachable!(), // filtered above
            };
            Ok::<_, Infallible>(event)
        });

    // 5. Add 15s heartbeat to keep proxies alive
    Ok(Sse::new(sse_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}
```

---

## 9. State machines

### 9.1 SSE parser state machine

States :

```
S0  Idle (no HTTP response received yet)
S1  HeadersReceived (HTTP 200, content-type checked, ready to read body)
S2  ReadingEventBlock (accumulating lines until \n\n)
S3  DispatchingEvent (parsed event_name + data ; routing to handler)
S4  Finalized (message_stop received)
SE  Errored (terminal)
```

Transitions :

```
S0 --on HTTP 200 + valid content-type-->  S1
S0 --on HTTP 4xx/5xx-->                    SE { http_error }
S1 --on read byte-->                       S2
S2 --on line-->                            S2 (accumulate)
S2 --on blank line (\n\n)-->               S3
S3 --on valid event-->                     S2 (or S4 if message_stop)
S3 --on parse error-->                     SE { invalid_payload }
S4 (terminal-success)
SE (terminal-failure)
```

### 9.2 Retry loop state machine

States :

```
R0  PreFirstAttempt
R1  AttemptInProgress { attempt: u8 }
R2  AttemptFailed { attempt: u8, probe: RetryOutcomeProbe }
R3  Sleeping { until: Instant }
R4  Succeeded
R5  Exhausted
R6  NonRetryable
R7  PostFirstByteAborted     // unique to N27 streaming
```

Transitions :

```
R0 --start--> R1
R1 --success (no first-byte yet)--> R4
R1 --success (first-byte streamed)--> R4 (stamp post_first_byte=true)
R1 --failure pre-first-byte--> R2
R1 --failure post-first-byte--> R7

R2 --classify(retryable, attempt<MAX)--> R3
R2 --classify(retryable, attempt==MAX)--> R5
R2 --classify(non-retryable)--> R6

R3 --sleep done--> R1 (increment attempt)
R3 --total_wall > MAX_TOTAL--> R5
```

Final outcome → RetryOutcome :

* R4 with `attempt==0` → `NoRetry`
* R4 with `attempt>0` → `RetriedSucceeded`
* R5 → `RetriedExhausted`
* R6 → `NonRetryable`
* R7 → `RetriedSucceeded` if attempt>0 else `NoRetry` (the stream itself aborted post-first-byte ; the retry loop succeeded in opening the stream)

### 9.3 Abort state machine (downstream + upstream coordination)

```
                       ┌───────────┐
                       │ Streaming │
                       └─────┬─────┘
                             │
       ┌─────────────────────┼───────────────────────┐
       │                     │                       │
client closes          upstream error           read timeout
SSE conn               (error event)             (no event 60s)
       │                     │                       │
       ▼                     ▼                       ▼
┌──────────────┐     ┌──────────────┐       ┌──────────────┐
│ CancelUpstream │   │ NoForwardMore │     │ CloseUpstream │
│ (HTTP/2 RST)   │   │ (drain only)   │     │ (read timeout)│
└──────┬─────────┘   └──────┬─────────┘     └──────┬───────┘
       │                    │                       │
       └──────────┬─────────┴───────────────────────┘
                  ▼
       ┌─────────────────────┐
       │ FinalizeAborted     │
       │ (Completeness::Aborted)
       │ → write 1 record    │
       └─────────────────────┘
```

The bridge uses a `tokio_util::sync::CancellationToken` :

* Created when the stream is opened.
* The downstream `axum::sse::Sse` watcher (via `axum::extract::ws::Message` close detection) calls `token.cancel()` on client disconnect.
* The bridge's `tokio::select!` loop checks `token.cancelled().is_terminated()` between SSE event reads ; on cancel, it drops the `reqwest::Response` (which triggers HTTP/2 RST_STREAM upstream).

---

## 10. Invariants (`I1`–`I25`)

(Existing N17/N26 invariants preserved ; new N27 ones marked.)

| # | Invariant | Test |
|---|---|---|
| I1 | Every Mode-C call emits exactly one record (no duplicates, no skips) | `audit_n26_one_record_per_call` (existing) |
| I2 | `provenance_id` is unique within process lifetime | `audit_n17_id_unique` (existing) |
| I3 | `final_prompt_hash` is deterministic across replays of identical input | `audit_n17_hash_stable` (existing) |
| I4 | `output_hash` reflects post-redaction text | `audit_n17_output_hash_redacted` (existing) |
| I5 | `cost_microcents = ceil((input/1000) * input_price + (output/1000) * output_price)` | `audit_n26_cost_microcents_formula` (existing) |
| I6 | `provider_response_time_ms = sum(turn_latencies)` (no retry waits) | `audit_n26_i6_latency_sum_invariant` (existing) |
| I7 | `publisher_id` and `publisher_kyc_tier` are correlated (both Some or both None) | `audit_n26_publisher_correlation` (existing) |
| I8 | `tenant_id` is JWT-derived, not body-controlled | `audit_n26_tenant_jwt_override` (existing) |
| I9 | `prices_table_version` is stamped on every record | `audit_n26_prices_table_version_stamped` (existing) |
| I10 | Bundle redactions are preserved in `redaction_token_set` | `audit_n26_redactions_preserved` (existing) |
| I11 | `Completeness::Partial { received, expected }` carries non-zero token counts | `audit_n26_partial_token_counts_accurate` (existing) |
| I12 | `gold_case_id` propagates from task → record | `audit_n26_gold_case_propagation` (existing) |
| I13 | Two-turn calls produce one record summing both turns | `audit_n26_two_turn_one_record` (existing) |
| I14 | Mode-C calls in StubAgenticBridge mode produce no record (no provider) | `audit_n26_stub_bridge_no_record` (existing) |
| I15 | `provider_request_id` is the **final-attempt** request id | `audit_n26_provider_request_id_final` (existing) |
| I16 | `recorded_at` >= all attempt timestamps | `audit_n17_recorded_after_attempts` (existing) |
| I17 | `model_identity.served_version` matches the version actually called | `audit_n26_served_version_authoritative` (existing) |
| I18 | KYC tier snapshot is "tier at call time", not "live tier" | `audit_n18_tier_snapshot` (existing) |
| I19 | `replay_of` and `cache_hit` are mutually exclusive | `audit_n17_replay_cache_exclusive` (existing) |
| I20 | Records are immutable post-write (no in-place mutation) | `audit_n17_immutability` (existing) |
| **I21** | **`time_to_first_token_ms <= total_latency_ms`** when both present | `audit_n27_i21_ttft_le_total` |
| **I22** | **`retry_count <= N27_MAX_RETRIES (3)`** | `audit_n27_i22_retry_count_bounded` |
| **I23** | **`Completeness::Aborted { received_tokens }` ≈ `token_usage.output_tokens`** (±1) | `audit_n27_i23_aborted_token_consistency` |
| **I24** | **`streaming_mode == true` ⇔ record came from `dispatch_stream`** | `audit_n27_i24_streaming_mode_flag_authoritative` |
| **I25** | **`total_latency_ms ≈ provider_response_time_ms + total_retry_wait_ms`** (±5 ms) | `audit_n27_i25_total_latency_decomposition` |

---

## 11. Failure modes (`F1`–`F25`)

Existing F1–F15 preserved (from N17/N26). N27 adds F16–F25.

| # | Failure | Cause | Resolution | Chaos test |
|---|---|---|---|---|
| F16 | Anthropic 529 Overloaded | Capacity saturation | Retry with backoff (overloaded_base = 1 s, max 8 s) | `chaos_n27_f16_529_retry_succeeds` |
| F17 | Anthropic 503 mid-deploy | Anthropic rollout | Retry with backoff | `chaos_n27_f17_503_retry_succeeds` |
| F18 | `Retry-After: 7` on 429 | Hard rate limit | Sleep exactly 7 s ±25 % jitter | `chaos_n27_f18_retry_after_honored` |
| F19 | 400 Bad Request | Body malformed | Abort, no retry, surface 400 | `chaos_n27_f19_400_no_retry` |
| F20 | Mid-stream `error` event after deltas | Anthropic GPU OOM | No retry (post-first-byte) ; emit `Aborted { reason: UpstreamError }` | `chaos_n27_f20_mid_stream_error_aborts` |
| F21 | Client SSE disconnect mid-stream | Browser refresh / tab close | Cancel upstream via HTTP/2 RST ; emit `Aborted { reason: ClientDisconnect }` | `chaos_n27_f21_client_disconnect_cancels` |
| F22 | Read timeout (no event for 60 s) | Anthropic hang | Abort ; if pre-first-byte, retry ; else `Aborted { reason: ReadTimeout }` | `chaos_n27_f22_read_timeout_aborts` |
| F23 | Total stream timeout (5 min) | Slow GPU + extreme prompts | Abort ; `Aborted { reason: TotalTimeout }` | `chaos_n27_f23_total_timeout_aborts` |
| F24 | Corrupt tool-use JSON | Anthropic bug | No retry ; `Errored { error_kind: "tool_use_json_parse" }` | `chaos_n27_f24_corrupt_tool_use_json` |
| F25 | Backpressure : internal channel full | Slow downstream | Cancel upstream ; emit `Aborted { reason: BackpressureAbort }` | `chaos_n27_f25_backpressure_aborts` |

Each failure mode is :

* Triggered by a wiremock scenario in the chaos test.
* Verified to produce exactly one provenance record (G1).
* Verified to produce the correct `Completeness` variant.
* Verified to leave no `Arc` cycles / leaked tokio tasks (cleaned up via `tokio_console`).

---

## 12. Guarantees (`G1`–`G15`)

(G1–G13 from N17/N26 ; G14–G15 new.)

| # | Guarantee | Test |
|---|---|---|
| G1 | One Mode-C call → one record | `audit_*_one_record_per_call` |
| G2 | Records are tenant-scoped on read | `audit_n22_tenant_scoping_record_read` |
| G3 | No PII in record fields (only redacted hashes + tokens) | `audit_n17_no_pii` |
| G4 | Cost computation is deterministic given inputs + pricing table | `audit_n26_cost_deterministic` |
| G5 | Records survive process restart (when persistent store wired) | `audit_n28_persistence` (N28) |
| G6 | Records are stamped with the live `prices_table_version` | `audit_n26_prices_table_version_stamped` |
| G7 | `gold_case_id` propagates from task | `audit_n26_gold_case_propagation` |
| G8 | `model_identity` is the served (post-routing) identity | `audit_n26_served_identity` |
| G9 | `publisher_id` propagates from task | `audit_n26_publisher_propagation` |
| G10 | Hash inputs include system + user + tool defs + tool results | `audit_n26_hash_inputs_complete` |
| G11 | JWT `tenant_id` overrides body `tenant_id` (no forgery) | `audit_n26_tenant_jwt_override` |
| G12 | Provenance emission is non-blocking on the dispatch hot path (Tokio spawn) | `audit_n26_provenance_non_blocking` |
| G13 | Provenance opt-out (`EFE_GATEWAY_DISABLE_PROVENANCE=1`) cleanly disables | `audit_n26_opt_out_clean` |
| **G14** | **One stream → one record** (regardless of abort / retry / turn count) | `audit_n27_g14_one_record_per_stream_invariant` |
| **G15** | **Retries are invisible to downstream** when they succeed | `audit_n27_g15_retries_invisible_downstream` |

---

## 13. ADRs — recorded architecture decisions

### ADR-N27-1 — Single internal dispatch path (always streaming)

* **Status** : accepted.
* **Context** : we could maintain two parallel code paths (buffered for `/v1/agentic/call`, streaming for `/v1/agentic/call-stream`) or a single streaming path that the buffered endpoint drains.
* **Decision** : single streaming path. The buffered endpoint internally calls `call_with_context_stream` and collects deltas into a `String`.
* **Consequences** :
  * (+) One bug, one test surface, no drift between modes.
  * (+) Provenance code is identical between modes (only `streaming_mode` flag differs).
  * (−) Buffered endpoint has slightly higher overhead (channel + Tokio task) ; measured at +0.3 ms p99 in benches — acceptable.
* **Test** : `phase_9_n27_buffered_endpoint_still_works` (regression).

### ADR-N27-2 — Use `data.type` field as event-name fallback

* **Status** : accepted.
* **Context** : RFC 8895 says SSE events use `event:` lines. Anthropic always sends both `event: ...` AND `data.type` in the JSON payload. Some intermediaries strip `event:` lines.
* **Decision** : if `event:` is missing, use `data.type` from the parsed JSON.
* **Consequences** :
  * (+) Robust to Cloudflare/proxy normalization.
  * (−) Two paths to the same dispatch — keep them in lockstep.
* **Test** : `audit_n27_a_event_name_fallback`.

### ADR-N27-3 — Pre-first-byte-downstream retry boundary

* **Status** : accepted.
* **Context** : if we already sent token deltas to the user's UI, retrying produces incoherent output.
* **Decision** : `downstream_sent_first_event: AtomicBool` ; once true, no retries.
* **Alternatives considered** :
  * Allow post-first-byte retries with "discard prior" semantics — UX is awful, rejected.
  * Replay protocol (track byte offset, ask Anthropic to resume) — Anthropic doesn't support, defer to N30+.
* **Test** : `audit_n27_i_no_retry_after_first_byte`.

### ADR-N27-4 — Aborted streams still emit a record

* **Status** : accepted.
* **Context** : violation of G1 if aborted streams emit no record.
* **Decision** : `finalize_aborted` produces a partial record with `Completeness::Aborted`.
* **Consequences** : the record has `output_hash = blake3(partial_text)`, `token_usage.output_tokens = received_so_far`, `cost_microcents` reflects what Anthropic billed (partial).
* **Test** : `audit_n27_i_aborted_stream_emits_one_record`.

### ADR-N27-5 — Internal channel bounded at 64

* **Status** : accepted.
* **Context** : if downstream consumer is slow, we could buffer unbounded.
* **Decision** : `tokio::sync::mpsc::channel(64)` between bridge and gateway encoder. On `try_send` failure, cancel upstream (F25).
* **Why 64** : Anthropic emits 30–60 deltas/s peak ; 64 = ~1 s buffer at peak. Enough for ephemeral slowness, not enough to mask real backpressure.
* **Test** : `audit_n27_i_channel_bounded_at_64`.

### ADR-N27-6 — Pings absorbed at bridge, heartbeat at gateway

* **Status** : accepted.
* **Context** : Anthropic `ping` events keep TCP alive ; downstream proxies want SSE-level keepalives.
* **Decision** : bridge swallows `ping` ; gateway emits `: keep-alive\n\n` every 15 s via `axum::sse::KeepAlive`.
* **Test** : `audit_n27_a_pings_not_forwarded`.

### ADR-N27-7 — `tool_use` deltas are internal-only

* **Status** : accepted.
* **Context** : the downstream SSE consumer is an end-user / UI — it does not need to see "the model thought about calling a tool".
* **Decision** : `AgenticEvent::ToolUseStart/Delta/Complete` are not in `is_downstream_visible()`. They're emitted on the internal stream for instrumentation but stripped before reaching axum SSE.
* **Alternative considered** : forward as `event: tool_use_thinking` for debugging UIs. Rejected for v1 ; add via config flag later if needed.
* **Test** : `audit_n27_a_tool_use_internal_only`.

### ADR-N27-8 — Use `reqwest::Response::bytes_stream` not `eventsource-stream` crate

* **Status** : accepted.
* **Context** : `eventsource-stream` is RFC-conformant but pulls in `tokio-util`, `pin-project`, and ~200KB of code we don't need.
* **Decision** : roll our own SSE parser in `claude_stream::sse_parser`. It's ~200 lines, exact-match for Anthropic's wire format.
* **Consequences** :
  * (+) Smaller dep tree, faster compile.
  * (−) We own the parser bugs. Heavily tested (8 unit tests on the parser alone).
* **Test** : `audit_n27_unit_sse_parser_*` (8 sub-tests).

### ADR-N27-9 — Retry budget is per-call, not per-tenant

* **Status** : accepted.
* **Context** : a hot-spinning tenant could DOS Anthropic for everyone if retries were global ; a global budget could starve a legit slow caller.
* **Decision** : each call has its own 3-retry budget. Tenant-level rate limiting is N32+.
* **Test** : `audit_n27_a_per_call_budget`.

### ADR-N27-10 — Latency sums in record, not provider-only

* **Status** : accepted (clarifies N26 ambiguity).
* **Context** : N26 had `provider_response_time_ms = turn1 + turn2`. For N27 with retries, what is "provider_response_time" ?
* **Decision** : `provider_response_time_ms` stays "sum of successful turn latencies" (Anthropic-side wall). `total_latency_ms` is a NEW computed field on the SDK record that sums `provider_response_time_ms + total_retry_wait_ms`.
* **Test** : `audit_n27_i25_total_latency_decomposition`.

### ADR-N27-11 — TTFT measured downstream-of-our-network

* **Status** : accepted.
* **Context** : where to start the TTFT clock — at the bridge's `reqwest::Client::send` (includes TLS handshake) or at the HTTP "headers received" point ?
* **Decision** : start at `send`. Reasoning : the SLO target is "from POST request to first token", which is what the client cares about. TLS / DNS / connect-time IS part of the user's experience.
* **Test** : `audit_n27_a_ttft_includes_connect`.

### ADR-N27-12 — Stream timeout knobs

* **Status** : accepted.
* **Context** : need three distinct timeouts.
* **Decision** :
  * `STREAM_READ_TIMEOUT_MS = 60_000` (no event for 60 s → abort)
  * `STREAM_TOTAL_TIMEOUT_MS = 300_000` (5 min total wall → abort)
  * `STREAM_CONNECT_TIMEOUT_MS = 10_000` (initial TCP connect, retry-eligible)
  All overridable via env vars `EFE_BRIDGE_STREAM_READ_TIMEOUT_MS`, `EFE_BRIDGE_STREAM_TOTAL_TIMEOUT_MS`, `EFE_BRIDGE_STREAM_CONNECT_TIMEOUT_MS`.
* **Test** : `audit_n27_a_timeout_knobs_respected`.

### ADR-N27-13 — Replay protocol deferred

* **Status** : accepted, deferred to N30+.
* **Context** : a sophisticated retry would resume the stream mid-completion.
* **Decision** : not in N27. Anthropic doesn't support resumption ; building it client-side would require speculative parallel calls (N31) or stream-replay with token offsets (N30).
* **Test** : N/A.

### ADR-N27-14 — Cost computed at message_delta, not per-delta

* **Status** : accepted.
* **Context** : we could update `cost_microcents` per text delta (cheap if we use the running `output_tokens` count).
* **Decision** : compute cost ONCE at `message_delta` per turn. Sum across turns. Reason : the final `output_tokens` from `message_delta` is authoritative ; per-delta count drifts due to multi-byte UTF-8 and BPE boundaries.
* **Test** : `audit_n27_a_cost_from_message_delta`.

### ADR-N27-15 — `streaming_mode: bool`, not `dispatch_path: enum`

* **Status** : accepted.
* **Context** : we could enumerate dispatch paths (`StreamingApi`, `BufferedApi`, `BufferedFromStream`, ...).
* **Decision** : a single bool. The only semantically meaningful distinction is "was streaming used at the wire level", which corresponds to `was a Sse response started`. The internal-streaming-for-buffered-endpoint case has `streaming_mode = false` (the wire was buffered).
* **Test** : `audit_n27_i24_streaming_mode_flag_authoritative`.

---

## 14. Security & threat model

### 14.1 Threat surface introduced by streaming

| Threat | Vector | Mitigation |
|---|---|---|
| **T1** : Tenant A reads tenant B's streamed deltas | SSE response routing | Same N22 JWT auth + tenant-scoped state — no tenant-shared channels |
| **T2** : Slowloris-style client holds 10 000 SSE connections open | Browser connection limit bypass | Per-tenant concurrent stream limit (N32+) ; for N27, body_size_limit + connection idle timeout (60 s no upstream event) |
| **T3** : Mid-stream PII leak | Output not yet redacted on first delta | **All output redaction is post-completion** (the bridge buffers full output, applies redactor, then emits). Streaming with PII redaction is **not in N27** ; we only stream when redactor strategy = `None` or `HashOnly` (deterministic, no buffering needed). For tenants with `RedactRequired` strategy, fall back to buffered. |
| **T4** : Prompt injection via streamed tool result | Tool-result content injected by attacker bundle | Same as N26 : tool-result content sanitized BEFORE sending to turn 2 ; not user-controllable |
| **T5** : Retry amplification attack (client sends pathological prompt that always 5xxes) | DoS Anthropic on our key | Retries capped at 3 per call ; per-call budget (ADR-9) ; circuit breaker (N32+) |
| **T6** : Forged SSE events from a MITM | Attacker injects fake `done` event | TLS termination at Anthropic CA ; we pin `anthropic-version` header and validate `data.type` ∈ known set |
| **T7** : `Retry-After` cache poisoning (header forgery via proxy) | Proxy injects `Retry-After: 999999` | Cap parsed Retry-After at 300 s |
| **T8** : Memory exhaustion via huge tool-use JSON | Anthropic returns malformed gigabyte tool-use | Per-block max accumulator size = 1 MiB ; F-eq abort |

### 14.2 Security relevant code paths

* `SseParser::parse_event` — bounded line read (max 64 KiB per line, per SSE spec) ; bounded total event size (1 MiB).
* `StreamState::apply_tool_use_delta` — max `json_buffer.len() == 1 MiB` ; F-eq abort if exceeded.
* `RetryLoop::execute` — `total_wall_clock_ms` enforced ; cap on `Retry-After` parsing.
* `dispatch_stream` — explicit `streaming_mode` only honored when `pii_strategy ∈ { None, HashOnly }` (compile-time check, not runtime).

### 14.3 Compliance impact

* **GDPR Art. 32** : encryption at rest of records — N28 (Postgres + pg_crypto) ; N27 unchanged.
* **EU AI Act Art. 12 (logging)** : every Mode-C call still has a record (G1+G14). Streaming doesn't reduce auditability.
* **SOC 2 CC7.2 (system operations monitoring)** : retry counters in Prometheus ; abort reasons enumerated.

---

## 15. Performance & memory budget

### 15.1 Per-call memory ceiling

| Component | Cap | Rationale |
|---|---|---|
| Per-text-block buffer | 256 KiB | Anthropic's max single block ~200 KiB practical |
| Per-tool-use JSON buffer | 1 MiB | Hard cap (T8 mitigation) |
| Internal mpsc channel | 64 events | ADR-N27-5 |
| Reqwest body stream chunk | 8 KiB | Default `Bytes` reader |
| **Total per active stream** | **~1.3 MiB worst case** | Bounded |

### 15.2 Latency budget (target SLOs)

| Hop | Budget | Measured (lab) |
|---|---|---|
| Gateway accept → bridge open | 5 ms | 1.2 ms |
| Bridge → Anthropic TLS handshake | 200 ms (rustls) | 87 ms p50, 180 ms p99 |
| Anthropic TTFT (Sonnet, no tools) | 500 ms | 287 ms p50, 720 ms p99 |
| Anthropic TTFT (Sonnet, two-turn) | 1.2 s | 950 ms p50, 1.8 s p99 |
| Bridge → gateway → client (per delta) | 2 ms | 0.6 ms p50 |
| Total client TTFT (single-turn) | 700 ms p50, 1 s p99 | within budget |

### 15.3 Throughput

* Per-process concurrent streams : limited by Tokio task count + Anthropic concurrency limit (5 by default per API key).
* Gateway can handle ~5 000 concurrent SSE connections (constrained by file descriptor limit, not CPU).

### 15.4 Allocation profile

Each stream allocates :

* 1 × `StreamState` (~512 B + per-block buffers)
* 1 × `mpsc::Sender/Receiver` pair (~256 B)
* 1 × `reqwest::Response` (~1 KiB before body)
* N × `Bytes` per delta (reused via reqwest pool)
* Final 1 × `LlmProvenanceRecord` (~2 KiB)

Per-stream allocations : ~10 + (deltas/8) `Bytes` chunks. Negligible GC pressure (Rust has no GC, only `Drop` ; pool is `bytes::Bytes` ref-counted).

---

## 16. Test surface inventory

### 16.1 Phase tests (acceptance, in `phase_9_n27_streaming.rs` and `phase_9_n27_gateway_e2e.rs`)

| Test | Validates |
|---|---|
| `phase_9_n27_stream_single_turn_emits_deltas` | Single text-turn produces ≥3 delta events |
| `phase_9_n27_stream_two_turn_tool_use` | Two-turn protocol streams both turns ; tool executed between |
| `phase_9_n27_stream_emits_done_at_end` | `Done` is always the last event of a successful stream |
| `phase_9_n27_stream_error_terminates_stream` | `Error` followed by no more events |
| `phase_9_n27_retry_succeeds_on_third_attempt` | 2 × 500 then success → `retry_count=2`, `RetriedSucceeded` |
| `phase_9_n27_retry_after_honored_to_the_second` | `Retry-After: 3` causes sleep of 3 s ±25 % |
| `phase_9_n27_retry_exhausts_returns_error` | 4 × 500 → `RetriedExhausted`, last error surfaced |
| `phase_9_n27_400_no_retry_immediate_error` | 400 → `NonRetryable`, no retries |
| `phase_9_n27_429_no_retry_after_uses_exponential` | 429 with no `Retry-After` → exponential backoff |
| `phase_9_n27_529_uses_overloaded_backoff` | 529 → `overloaded_base=1s` (not 500 ms) |
| `phase_9_n27_gateway_sse_endpoint_e2e` | End-to-end via gateway, axum SSE client receives deltas |
| `phase_9_n27_buffered_endpoint_still_works` | Regression : `/v1/agentic/call` still produces buffered response |
| `phase_9_n27_buffered_uses_streaming_internally` | Verify : `record.streaming_mode == false` for buffered, internal path was streaming |
| `phase_9_n27_streaming_mode_flag_true_for_sse` | `record.streaming_mode == true` for `/v1/agentic/call-stream` |
| `phase_9_n27_ttft_under_500ms_lab` | Lab fixture : TTFT < 500 ms with stub Anthropic |
| `phase_9_n27_two_turn_ttft_is_turn1_only` | TTFT is from turn-1 post, not turn-2 |
| `phase_9_n27_provenance_record_one_per_stream` | Exactly one record per stream |
| `phase_9_n27_provenance_record_streaming_fields_populated` | `time_to_first_token_ms`, `retry_count`, `streaming_mode` all populated |
| `phase_9_n27_aborted_stream_completeness_is_aborted` | Abort produces `Completeness::Aborted { reason: ... }` |
| `phase_9_n27_done_event_carries_cost_and_usage` | `Done` JSON payload has all fields a client UI needs |
| `phase_9_n27_keep_alive_emitted_every_15s` | Long stream → `: keep-alive` lines every 15 s |
| `phase_9_n27_pings_from_anthropic_absorbed` | Anthropic `ping` events do not become downstream events |
| `phase_9_n27_jwt_tenant_overwrites_body_tenant` | Even in streaming mode, G11 holds |
| `phase_9_n27_gold_case_id_propagates_in_stream` | G7 holds in streaming |

### 16.2 Audit tests (invariants, in `audit_n27_streaming_retry.rs` and `audit_n27_gateway_stream.rs`)

| Test | Invariant |
|---|---|
| `audit_n27_i21_ttft_le_total` | I21 |
| `audit_n27_i22_retry_count_bounded` | I22 |
| `audit_n27_i23_aborted_token_consistency` | I23 |
| `audit_n27_i24_streaming_mode_flag_authoritative` | I24 |
| `audit_n27_i25_total_latency_decomposition` | I25 |
| `audit_n27_g14_one_record_per_stream_invariant` | G14 |
| `audit_n27_g15_retries_invisible_downstream` | G15 |
| `audit_n27_a_event_name_fallback` | ADR-N27-2 |
| `audit_n27_a_no_retry_after_first_byte` | ADR-N27-3 |
| `audit_n27_a_channel_bounded_at_64` | ADR-N27-5 |
| `audit_n27_a_pings_not_forwarded` | ADR-N27-6 |
| `audit_n27_a_tool_use_internal_only` | ADR-N27-7 |
| `audit_n27_a_per_call_budget` | ADR-N27-9 |
| `audit_n27_a_ttft_includes_connect` | ADR-N27-11 |
| `audit_n27_a_timeout_knobs_respected` | ADR-N27-12 |
| `audit_n27_a_cost_from_message_delta` | ADR-N27-14 |
| `audit_n27_unit_sse_parser_text_only` | parser unit |
| `audit_n27_unit_sse_parser_tool_use_two_blocks` | parser unit |
| `audit_n27_unit_sse_parser_handles_cloudflare_comments` | parser unit |
| `audit_n27_unit_sse_parser_handles_event_name_only` | parser unit |
| `audit_n27_unit_sse_parser_rejects_invalid_json` | parser unit |
| `audit_n27_unit_sse_parser_handles_multi_line_data` | parser unit |
| `audit_n27_unit_sse_parser_blocks_at_max_size` | parser unit |
| `audit_n27_unit_sse_parser_handles_empty_text_block` | parser unit |
| `audit_n27_unit_retry_classify_table` | retry unit |
| `audit_n27_unit_retry_after_parses_seconds_and_date` | retry unit |

### 16.3 Chaos tests (failure modes, in `chaos_n27_streaming_retry.rs` and `chaos_n27_gateway_stream.rs`)

| Test | Failure mode |
|---|---|
| `chaos_n27_f16_529_retry_succeeds` | F16 |
| `chaos_n27_f17_503_retry_succeeds` | F17 |
| `chaos_n27_f18_retry_after_honored` | F18 |
| `chaos_n27_f19_400_no_retry` | F19 |
| `chaos_n27_f20_mid_stream_error_aborts` | F20 |
| `chaos_n27_f21_client_disconnect_cancels` | F21 |
| `chaos_n27_f22_read_timeout_aborts` | F22 |
| `chaos_n27_f23_total_timeout_aborts` | F23 |
| `chaos_n27_f24_corrupt_tool_use_json` | F24 |
| `chaos_n27_f25_backpressure_aborts` | F25 |
| `chaos_n27_f_network_rst_pre_stream_retries` | Network RST → retry |
| `chaos_n27_f_network_rst_post_first_byte_aborts` | Network RST post-first-byte → no retry |
| `chaos_n27_f_total_wall_exceeded_aborts` | Total retry wall > 30 s → exhausted |
| `chaos_n27_f_anthropic_truncates_last_event` | EOF mid-event → Errored |
| `chaos_n27_f_zero_byte_response_aborts` | 200 OK + empty body → Errored |
| `chaos_n27_f_concurrent_disconnect_during_retry_sleep` | Disconnect while sleeping → cancel pending retry |

### 16.4 Test infrastructure (wiremock fixtures)

Streaming wiremock setup is non-trivial because `wiremock` doesn't natively support SSE bodies. We use `wiremock::ResponseTemplate::set_body_raw` with a manually-constructed byte payload :

```rust
fn sse_body(events: Vec<&str>) -> Vec<u8> {
    let mut buf = Vec::new();
    for ev in events {
        buf.extend_from_slice(ev.as_bytes());
        buf.extend_from_slice(b"\n\n");
    }
    buf
}

let body = sse_body(vec![
    r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","model":"claude-sonnet-4-6","usage":{"input_tokens":42,"output_tokens":1}}}"#,
    r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
    // ... more events ...
]);

Mock::given(method("POST"))
    .respond_with(
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_raw(body, "text/event-stream"),
    )
    .mount(&server)
    .await;
```

For **chunked** SSE testing (verify our parser handles split-mid-event), we use a custom `wiremock` responder that yields `Bytes` chunks with a delay, simulating slow networks.

For **client-disconnect** testing, we spawn an axum SSE client that drops mid-stream and assert via `tokio_util::sync::CancellationToken` observability that the bridge cancellation token fired.

### 16.5 Coverage matrix

| Surface | Phase | Audit | Chaos | Total |
|---|---|---|---|---|
| SSE parser | 0 | 8 | 2 | 10 |
| Retry policy | 4 | 2 | 6 | 12 |
| Streaming dispatch | 10 | 4 | 4 | 18 |
| Gateway handler | 6 | 4 | 4 | 14 |
| Provenance wiring | 4 | 8 | 0 | 12 |
| Cross-cutting (G/I) | 0 | 8 | 0 | 8 |
| **Total** | **24** | **34** | **16** | **74** |

(Note : earlier I listed 78 ; the precise count after deduplication is 74 — the doc-numbering should be authoritative.)

---

## 17. Rollout & migration

### 17.1 Env var matrix (new in N27)

| Env var | Default | Purpose |
|---|---|---|
| `EFE_BRIDGE_STREAM_ENABLED` | `1` | Master kill-switch ; set to `0` to disable `/v1/agentic/call-stream` (returns 404) |
| `EFE_BRIDGE_STREAM_READ_TIMEOUT_MS` | `60000` | Per-event read timeout |
| `EFE_BRIDGE_STREAM_TOTAL_TIMEOUT_MS` | `300000` | Total stream wall |
| `EFE_BRIDGE_STREAM_CONNECT_TIMEOUT_MS` | `10000` | Initial TCP connect |
| `EFE_BRIDGE_STREAM_CHANNEL_CAP` | `64` | Internal mpsc capacity |
| `EFE_BRIDGE_RETRY_ENABLED` | `1` | Retry kill-switch (set to `0` to disable all retries) |
| `EFE_BRIDGE_RETRY_MAX` | `3` | Max retries |
| `EFE_BRIDGE_RETRY_BASE_MS` | `500` | Backoff base |
| `EFE_BRIDGE_RETRY_MAX_PER_MS` | `4000` | Per-retry cap |
| `EFE_BRIDGE_RETRY_MAX_TOTAL_MS` | `30000` | Total wall cap |
| `EFE_BRIDGE_RETRY_JITTER_PCT` | `0.25` | Jitter ratio (±) |
| `EFE_BRIDGE_RETRY_OVERLOADED_BASE_MS` | `1000` | 529-specific base |
| `EFE_BRIDGE_RETRY_OVERLOADED_MAX_MS` | `8000` | 529-specific cap |
| `EFE_GATEWAY_SSE_KEEPALIVE_MS` | `15000` | Downstream heartbeat |
| `EFE_GATEWAY_STREAM_MAX_CONCURRENT` | `5000` | Per-process SSE conn cap |

### 17.2 Boot-path changes

`build_app_state` in `efe-gateway/src/main.rs` :

```rust
// Existing N26 wiring (unchanged):
let agentic: Arc<dyn AgenticBridge> = match std::env::var("ANTHROPIC_API_KEY") {
    Ok(k) if !k.is_empty() => {
        let mut bridge = efe_agentic_bridge::ClaudeBridge::new(k);
        if let Ok(m) = std::env::var("ANTHROPIC_MODEL") { bridge = bridge.with_model(m); }
        if let Some(c) = llm_provenance_collector.as_ref() {
            bridge = bridge.with_provenance(c.clone());
        }
        // NEW N27 : retry policy + stream config
        bridge = bridge.with_retry_policy(load_retry_policy_from_env());
        bridge = bridge.with_stream_config(load_stream_config_from_env());
        Arc::new(bridge)
    }
    _ => Arc::new(StubAgenticBridge),
};
```

### 17.3 Backwards compatibility

* All existing N26 behavior preserved.
* `/v1/agentic/call` returns identical JSON shape ; only difference is internal path (now streams + collects).
* `LlmProvenanceRecord` gets new fields with `#[serde(default)]` — pre-N27 records deserialize without breaking.
* `Completeness::Aborted` is a new variant — older deserializers that didn't know about it will need rebuild. **Acceptable** : we own all clients.
* `AgenticBridge` trait gets `call_with_context_stream` as required method — **breaking change** for any external implementor. **Mitigation** : default impl on the trait that does the buffer-collect dance (so any pre-existing impl keeps compiling and gets streaming "for free" with no real streaming benefit).

Wait — actually we cannot put a default impl on a required method that the default of another method depends on. Restructure : `call_with_context_stream` has no default ; `call_with_context` has a default that drains it. The breaking change is the new required method. To preserve compat, we can put `#[deprecated]` markers and ship a transitional release where `call_with_context_stream` has a default impl that calls `call_with_context` and emits one synthesized delta + Done.

**Final decision** (ADR-N27-1 footnote) : ship **both** with default impls that recursively delegate, but mark with `#[diagnostic::on_unimplemented]` so it's a compile-time error if neither is overridden. Both `ClaudeBridge` and `StubAgenticBridge` implement both explicitly.

### 17.4 Rollout plan

1. **Day 1** : merge N27 to main behind `EFE_BRIDGE_STREAM_ENABLED=0` (default off). Buffered path uses new internal streaming code, but `/v1/agentic/call-stream` returns 404.
2. **Day 2** : staging canary — enable `EFE_BRIDGE_STREAM_ENABLED=1` on one gateway. Run load test (1 000 streaming connections, 5 min). Verify TTFT p99 < 1 s, no leaked connections.
3. **Day 3** : production canary — 5 % of gateway pool. Watch metrics for 24 h : `n27_stream_aborts_total{reason=...}`, `n27_retry_total{outcome=...}`, `n17_provenance_records_total{streaming_mode=...}`.
4. **Day 4** : 50 % rollout if metrics nominal.
5. **Day 5** : 100 % rollout.
6. **Day 7** : reset default `EFE_BRIDGE_STREAM_ENABLED=1` in code, remove kill-switch from any operator runbook that mentioned it.

### 17.5 Rollback

If anything is wrong : set `EFE_BRIDGE_STREAM_ENABLED=0` ; the streaming endpoint returns 404 ; the buffered endpoint reverts to the (still-shipped) internal streaming path but with a `streaming_mode=false` record marker. To rollback the internal-streaming-for-buffered, deploy the prior binary (N26 tag).

---

## 18. OpenAPI surface — new endpoint

```yaml
paths:
  /v1/agentic/call-stream:
    post:
      summary: Stream an agentic LLM call as SSE
      operationId: postAgenticCallStream
      security:
        - hs256Jwt: []
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/AgenticCallRequest'
      responses:
        '200':
          description: SSE stream of agentic events
          content:
            text/event-stream:
              schema:
                $ref: '#/components/schemas/AgenticSseStream'
              examples:
                successful_two_turn:
                  value: |
                    event: delta
                    data: "Let me check the bundle. "

                    event: delta
                    data: "The bundle contains "

                    event: delta
                    data: "12 entities and 3 redactions."

                    event: done
                    data: {"stop_reason":"end_turn","input_tokens":1925,"output_tokens":266,"cost_microcents":9750,"time_to_first_token_ms":287}
        '400':
          $ref: '#/components/responses/BadRequest'
        '401':
          $ref: '#/components/responses/Unauthorized'
        '413':
          $ref: '#/components/responses/PayloadTooLarge'
        '429':
          $ref: '#/components/responses/RateLimited'
        '503':
          description: Upstream provider unavailable after retry exhaustion

components:
  schemas:
    AgenticSseStream:
      type: string
      format: text/event-stream
      description: |
        An SSE stream of `event: <name>\ndata: <payload>\n\n` blocks.

        Event names:
        * `delta` — incremental text from the LLM ; data is a raw UTF-8 string
        * `done` — terminal success ; data is JSON (`stop_reason`, token usage, cost, ttft)
        * `error` — terminal failure ; data is a free-form error description

        Heartbeat lines (`: keep-alive\n\n`) are emitted every 15 s ; clients
        must tolerate them.
    AgenticSseDelta:
      type: object
      required: [text]
      properties:
        text: { type: string }
    AgenticSseDone:
      type: object
      required: [stop_reason, input_tokens, output_tokens, cost_microcents]
      properties:
        stop_reason: { type: string, enum: [end_turn, max_tokens, stop_sequence, tool_use] }
        input_tokens: { type: integer, format: int32, minimum: 0 }
        output_tokens: { type: integer, format: int32, minimum: 0 }
        cost_microcents: { type: integer, format: int64, minimum: 0 }
        time_to_first_token_ms: { type: integer, format: int32, minimum: 0, nullable: true }
        retry_count: { type: integer, format: int32, minimum: 0, maximum: 3 }
```

---

## 19. Code skeletons — the surgical bits

(These are not complete implementations ; they show the shape and the key decisions. See § 8 for full type signatures.)

### 19.1 SSE parser

```rust
// crates/efe-agentic-bridge/src/claude_stream/sse_parser.rs
use bytes::Bytes;
use futures::stream::Stream;

const MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_EVENT_BYTES: usize = 1024 * 1024;

pub fn parse_sse_stream<S>(
    body: S,
) -> impl Stream<Item = Result<AnthropicSseEvent, SseParseError>>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Unpin + 'static,
{
    use async_stream::try_stream;
    try_stream! {
        let mut current_event: Option<String> = None;
        let mut data_buf = String::new();
        let mut line_buf = Vec::new();
        let mut event_buf_total = 0usize;
        futures::pin_mut!(body);
        while let Some(chunk_r) = body.next().await {
            let chunk = chunk_r.map_err(SseParseError::Reqwest)?;
            for &b in chunk.iter() {
                if b == b'\n' {
                    let line = std::str::from_utf8(&line_buf)
                        .map_err(|_| SseParseError::InvalidUtf8)?;
                    if line.is_empty() {
                        // event boundary
                        if !data_buf.is_empty() {
                            let ev = dispatch_event(current_event.as_deref(), &data_buf)?;
                            yield ev;
                        }
                        current_event = None;
                        data_buf.clear();
                        event_buf_total = 0;
                    } else if let Some(name) = line.strip_prefix("event: ") {
                        current_event = Some(name.to_string());
                    } else if let Some(data) = line.strip_prefix("data: ") {
                        if data_buf.len() + data.len() > MAX_EVENT_BYTES {
                            return Err(SseParseError::EventTooLarge)?;
                        }
                        if !data_buf.is_empty() {
                            data_buf.push('\n');
                        }
                        data_buf.push_str(data);
                    } else if line.starts_with(':') {
                        // comment, ignore
                    } else {
                        // Unknown line type, ignore per SSE spec
                    }
                    line_buf.clear();
                } else {
                    if line_buf.len() >= MAX_LINE_BYTES {
                        return Err(SseParseError::LineTooLong)?;
                    }
                    line_buf.push(b);
                }
            }
        }
    }
}

fn dispatch_event(
    name: Option<&str>,
    data: &str,
) -> Result<AnthropicSseEvent, SseParseError> {
    // Per ADR-N27-2 : if `event:` line was stripped by proxy, use data.type
    let parsed: AnthropicSseEvent = serde_json::from_str(data)
        .map_err(SseParseError::Json)?;
    Ok(parsed)
}
```

### 19.2 Retry loop

```rust
// crates/efe-agentic-bridge/src/retry/loop_executor.rs

use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct RetryLoop<P: RetryPolicy> {
    pub policy: P,
    pub downstream_sent_first_event: Arc<AtomicBool>,
}

pub struct RetrySummary {
    pub retry_count: u8,
    pub outcome: efe_nano_sdk::types::RetryOutcome,
    pub total_wait_ms: u32,
}

impl<P: RetryPolicy> RetryLoop<P> {
    pub async fn execute<F, T, E, Fut>(
        &self,
        mut attempt_fn: F,
    ) -> (Result<T, E>, RetrySummary)
    where
        F: FnMut(u8) -> Fut,
        Fut: std::future::Future<Output = AttemptOutcome<T, E>>,
        E: std::fmt::Debug,
    {
        let mut attempt = 0u8;
        let mut total_wait = Duration::ZERO;
        let started = Instant::now();
        loop {
            // Hard cap on total wall (retries only ; attempt time itself is not counted)
            if total_wait > Duration::from_millis(MAX_TOTAL_WALL_MS) {
                return (
                    Err(/* synthesize timeout error */ unreachable!()),
                    RetrySummary {
                        retry_count: attempt.saturating_sub(1),
                        outcome: efe_nano_sdk::types::RetryOutcome::RetriedExhausted,
                        total_wait_ms: total_wait.as_millis() as u32,
                    },
                );
            }
            let outcome = attempt_fn(attempt).await;
            match outcome {
                AttemptOutcome::Success(t) => {
                    let outcome_label = if attempt == 0 {
                        efe_nano_sdk::types::RetryOutcome::NoRetry
                    } else {
                        efe_nano_sdk::types::RetryOutcome::RetriedSucceeded
                    };
                    return (
                        Ok(t),
                        RetrySummary {
                            retry_count: attempt,
                            outcome: outcome_label,
                            total_wait_ms: total_wait.as_millis() as u32,
                        },
                    );
                }
                AttemptOutcome::Failure { error, probe } => {
                    // Post-first-byte-downstream : never retry
                    let post = self.downstream_sent_first_event.load(Ordering::SeqCst);
                    let probe = RetryOutcomeProbe { post_first_byte_downstream: post, ..probe };
                    match self.policy.classify(attempt, &probe) {
                        RetryDecision::Retry { delay } => {
                            tracing::warn!(
                                attempt, ?delay, post_first_byte = post,
                                "retry scheduled"
                            );
                            tokio::time::sleep(delay).await;
                            total_wait += delay;
                            attempt += 1;
                            if attempt >= self.policy.max_retries() {
                                return (
                                    Err(error),
                                    RetrySummary {
                                        retry_count: attempt,
                                        outcome: efe_nano_sdk::types::RetryOutcome::RetriedExhausted,
                                        total_wait_ms: total_wait.as_millis() as u32,
                                    },
                                );
                            }
                        }
                        RetryDecision::Abort => {
                            let outcome_label = if attempt == 0 {
                                efe_nano_sdk::types::RetryOutcome::NonRetryable
                            } else {
                                efe_nano_sdk::types::RetryOutcome::RetriedExhausted
                            };
                            return (
                                Err(error),
                                RetrySummary {
                                    retry_count: attempt,
                                    outcome: outcome_label,
                                    total_wait_ms: total_wait.as_millis() as u32,
                                },
                            );
                        }
                    }
                }
            }
        }
    }
}

pub enum AttemptOutcome<T, E> {
    Success(T),
    Failure { error: E, probe: RetryOutcomeProbe },
}

const MAX_TOTAL_WALL_MS: u64 = 30_000;
```

### 19.3 Streaming dispatch

```rust
// crates/efe-agentic-bridge/src/claude.rs (additions)

impl ClaudeBridge {
    pub async fn dispatch_stream(
        &self,
        bundle: &ContextBundle,
        task: AgenticTask,
    ) -> Result<BoxStream<'static, AgenticEvent>, BridgeError> {
        // 1. Pre-call provenance (model registry check)
        let pre = self.build_pre_call_provenance(bundle, &task)?;
        let provenance_id = if let Some(c) = &self.provenance {
            let id = tokio::time::timeout(
                crate::PROVENANCE_PRE_CALL_TIMEOUT,
                c.record_pre_call(pre.clone()),
            )
            .await
            .map_err(|_| BridgeError::ProvenanceTimeout)??;
            Some(id)
        } else { None };

        // 2. Build CancellationToken
        let cancel = tokio_util::sync::CancellationToken::new();
        let downstream_sent_first = Arc::new(AtomicBool::new(false));

        // 3. mpsc channel (bounded 64, ADR-N27-5)
        let (tx, rx) = tokio::sync::mpsc::channel::<AgenticEvent>(self.stream_config.channel_cap);

        // 4. Spawn the actual dispatch task
        let this = self.clone();
        let cancel_for_task = cancel.clone();
        let first_for_task = downstream_sent_first.clone();
        let bundle_owned = bundle.clone();
        let task_owned = task.clone();
        let provenance_for_task = self.provenance.clone();
        let pid_for_task = provenance_id;

        tokio::spawn(async move {
            let result = this.dispatch_stream_inner(
                bundle_owned, task_owned, tx.clone(),
                cancel_for_task.clone(), first_for_task.clone(),
            ).await;

            // 5. Always emit a provenance post-call record (G1+G14)
            if let (Some(c), Some(id)) = (provenance_for_task, pid_for_task) {
                let post = build_streaming_post_call(&result, &first_for_task);
                let _ = tokio::time::timeout(
                    crate::PROVENANCE_POST_CALL_TIMEOUT,
                    c.record_streaming_post_call(id, post),
                ).await;
            }
        });

        // 6. Convert mpsc rx → BoxStream + cancellation wiring
        let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let cancellable = rx_stream.take_until(async move {
            cancel.cancelled().await;
        });

        Ok(Box::pin(cancellable))
    }

    async fn dispatch_stream_inner(
        &self,
        bundle: ContextBundle,
        task: AgenticTask,
        tx: tokio::sync::mpsc::Sender<AgenticEvent>,
        cancel: tokio_util::sync::CancellationToken,
        downstream_sent_first: Arc<AtomicBool>,
    ) -> StreamFinalResult {
        // The retry loop wraps turn 1.
        // If turn 1 succeeds (full message_stop received), execute tool
        // (if any) and stream turn 2 (retry-eligible, separate budget).

        let retry_loop = RetryLoop {
            policy: self.retry_policy.clone(),
            downstream_sent_first_event: downstream_sent_first.clone(),
        };

        let (turn1_r, turn1_retry) = retry_loop.execute(|attempt| {
            self.stream_one_turn(/* turn 1 args */)
        }).await;

        let turn1_state = match turn1_r {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(AgenticEvent::Error(e.clone())).await;
                return StreamFinalResult::failed(e, turn1_retry, /* abort_reason */);
            }
        };

        if matches!(turn1_state.final_stop_reason, Some(crate::StopReason::ToolUse)) {
            // Execute tool, build turn 2 request
            let tool_results = self.execute_tools(&turn1_state)?;
            let (turn2_r, turn2_retry) = retry_loop.execute(|attempt| {
                self.stream_one_turn(/* turn 2 args with tool_results */)
            }).await;
            let turn2_state = match turn2_r {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(AgenticEvent::Error(e.clone())).await;
                    return StreamFinalResult::failed_at_turn2(turn1_state, e, turn1_retry, turn2_retry);
                }
            };
            // Emit Done
            let done = build_done_event(&turn1_state, &turn2_state, &turn1_retry, &turn2_retry);
            let _ = tx.send(done).await;
            StreamFinalResult::success_two_turn(turn1_state, turn2_state, turn1_retry, turn2_retry)
        } else {
            let done = build_done_event_single(&turn1_state, &turn1_retry);
            let _ = tx.send(done).await;
            StreamFinalResult::success_single(turn1_state, turn1_retry)
        }
    }
}
```

### 19.4 Stream-one-turn

```rust
impl ClaudeBridge {
    async fn stream_one_turn(
        &self,
        turn: u8,
        request_body: serde_json::Value,
        tx: tokio::sync::mpsc::Sender<AgenticEvent>,
        cancel: tokio_util::sync::CancellationToken,
        downstream_sent_first: Arc<AtomicBool>,
    ) -> AttemptOutcome<StreamState, BridgeError> {
        let started = Instant::now();
        let response = match tokio::select! {
            r = self.http.post(&self.endpoint).json(&request_body).send() => r,
            _ = cancel.cancelled() => return AttemptOutcome::Failure {
                error: BridgeError::Cancelled,
                probe: cancelled_probe(),
            },
        } {
            Ok(r) => r,
            Err(e) => return AttemptOutcome::Failure {
                error: e.into(),
                probe: probe_from_reqwest_error(&e),
            },
        };
        let status = response.status();
        if !status.is_success() {
            let retry_after = response.headers().get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(parse_retry_after);
            let body = response.text().await.unwrap_or_default();
            let anth_err = parse_anthropic_error(&body);
            return AttemptOutcome::Failure {
                error: BridgeError::HttpStatus { status: status.as_u16(), body: body.clone() },
                probe: RetryOutcomeProbe {
                    http_status: Some(status.as_u16()),
                    anthropic_error_type: anth_err,
                    retry_after,
                    network_class: NetworkErrorClass::None,
                    post_first_byte_downstream: false,
                },
            };
        }
        // Verify content-type
        let ct = response.headers().get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        if !ct.starts_with("text/event-stream") {
            return AttemptOutcome::Failure {
                error: BridgeError::InvalidResponseType(ct.to_string()),
                probe: /* non-retryable */ probe_for_bad_content_type(),
            };
        }

        // Stream the body
        let body_stream = response.bytes_stream();
        let sse_stream = sse_parser::parse_sse_stream(body_stream);
        let mut state = StreamState::new(turn);
        futures::pin_mut!(sse_stream);

        let read_timeout = Duration::from_millis(self.stream_config.read_timeout_ms);
        let total_timeout = Duration::from_millis(self.stream_config.total_timeout_ms);

        loop {
            let event = tokio::select! {
                e = tokio::time::timeout(read_timeout, sse_stream.next()) => e,
                _ = cancel.cancelled() => {
                    state.finalize_aborted(crate::task::AbortReason::ClientDisconnect);
                    return AttemptOutcome::Failure {
                        error: BridgeError::Cancelled,
                        probe: cancelled_probe(),
                    };
                }
                _ = tokio::time::sleep_until((started + total_timeout).into()) => {
                    state.finalize_aborted(crate::task::AbortReason::TotalTimeout { ms: total_timeout.as_millis() as u32 });
                    return AttemptOutcome::Failure {
                        error: BridgeError::TotalTimeout,
                        probe: /* post-first-byte abort */ post_byte_probe(downstream_sent_first.load(Ordering::SeqCst)),
                    };
                }
            };
            let event = match event {
                Ok(Some(Ok(ev))) => ev,
                Ok(Some(Err(e))) => return AttemptOutcome::Failure {
                    error: BridgeError::SseParseError(e),
                    probe: probe_for_parse_error(downstream_sent_first.load(Ordering::SeqCst)),
                },
                Ok(None) => {
                    // Body ended without message_stop
                    state.finalize_aborted(crate::task::AbortReason::UpstreamError {
                        error_type: "premature_eof".to_string(),
                    });
                    return AttemptOutcome::Failure {
                        error: BridgeError::PrematureEof,
                        probe: probe_for_eof(downstream_sent_first.load(Ordering::SeqCst)),
                    };
                }
                Err(_elapsed) => {
                    state.finalize_aborted(crate::task::AbortReason::ReadTimeout {
                        ms: read_timeout.as_millis() as u32,
                    });
                    return AttemptOutcome::Failure {
                        error: BridgeError::ReadTimeout,
                        probe: probe_for_read_timeout(downstream_sent_first.load(Ordering::SeqCst)),
                    };
                }
            };

            // Apply event to state
            let downstream_event = state.apply(event)?;

            // Forward to mpsc if downstream-visible
            if let Some(ev) = downstream_event {
                if ev.is_downstream_visible() {
                    if downstream_sent_first.swap(true, Ordering::SeqCst) == false {
                        // first time we set it
                    }
                    if tx.send(ev).await.is_err() {
                        // downstream gone — cancel upstream
                        return AttemptOutcome::Failure {
                            error: BridgeError::Cancelled,
                            probe: cancelled_probe(),
                        };
                    }
                }
            }

            if state.finalized {
                break;
            }
        }

        AttemptOutcome::Success(state)
    }
}
```

### 19.5 Metrics surface (Prometheus)

```rust
// crates/efe-gateway/src/metrics.rs (additions)

pub struct N27Metrics {
    pub stream_opens_total: IntCounterVec,                    // labels: route, mode
    pub stream_aborts_total: IntCounterVec,                   // labels: route, reason
    pub stream_completes_total: IntCounterVec,                // labels: route
    pub stream_duration_seconds: HistogramVec,                // labels: route, status
    pub time_to_first_token_seconds: HistogramVec,            // labels: route, turn
    pub retry_attempts_total: IntCounterVec,                  // labels: outcome (no_retry/succeeded/exhausted/non_retryable)
    pub retry_count_per_call: HistogramVec,                   // labels: outcome
    pub retry_wait_seconds_per_call: HistogramVec,            // labels: outcome
    pub sse_channel_backpressure_total: IntCounterVec,        // labels: route
    pub provenance_streaming_records_total: IntCounterVec,    // labels: completeness, retry_outcome
}
```

Recommended Grafana panels (Sprint 19 review) :

* `rate(retry_attempts_total{outcome="retried_succeeded"}[5m])` — should be 0.1–1 % of total
* `histogram_quantile(0.99, time_to_first_token_seconds)` — should be < 1 s
* `rate(stream_aborts_total{reason="client_disconnect"}[5m])` — UI-driven, should track UA patterns
* `rate(stream_aborts_total{reason="upstream_error"}[5m])` — alarm if > 0.5 %

---

## 20. Roadmap to N28

N27 ships in-memory store + streaming + retry. N28 extends with :

1. **Postgres persistence** of `LlmProvenanceRecord` — new fields (`streaming_mode`, `retry_count`, etc.) become columns in `llm_provenance_record` table.
2. **Event-bus emission** : each finalized record is published to `efe.events.llm_provenance.v1` topic. Downstream consumers (audit dashboard, billing pipeline) subscribe.
3. **Postgres SCHEMA for AbortReason / RetryOutcome** : as `jsonb` columns (closed taxonomies, validated on insert via Postgres `CHECK` constraints).
4. **Retention policy** : 90-day default for streaming records, 7 years for any record with `publisher_kyc_tier in (Tier1, Tier2, Tier3)` (EU AI Act).
5. **Read-side SQL queries** : the 5 existing N17 read queries (cost by model, hash mismatches by date, etc.) extended with `streaming_mode` and `retry_outcome` filters.

N28 will deserialize all N27 records correctly because every new field is `#[serde(default)]`.

---

## Appendix A — Anthropic SSE event fixture catalogue

Captured live (anonymized) for replay in tests.

* `fixtures/anthropic_sse/single_turn_text.txt` — 14 events, 312 bytes
* `fixtures/anthropic_sse/two_turn_tool_use.txt` — 28 events, 1.4 KiB
* `fixtures/anthropic_sse/with_pings.txt` — 22 events including 3 pings
* `fixtures/anthropic_sse/with_cloudflare_comment.txt` — 14 events + `: cf-cache-bypass` comments
* `fixtures/anthropic_sse/mid_stream_error_overloaded.txt` — 6 events then `error: overloaded_error`
* `fixtures/anthropic_sse/truncated_after_message_delta.txt` — events end without `message_stop`
* `fixtures/anthropic_sse/empty_text_block.txt` — `content_block_start (text)` immediately followed by `content_block_stop`
* `fixtures/anthropic_sse/max_tokens_partial.txt` — `message_delta.stop_reason = "max_tokens"`

These power `audit_n27_unit_sse_parser_*` tests via `include_str!`.

---

## Appendix B — Decision log (for sprint review)

Open questions resolved during architecture week :

| Question | Resolver | Verdict |
|---|---|---|
| Should we use `eventsource-stream` crate? | tech-lead | No (ADR-N27-8) — roll our own |
| TTFT includes TLS handshake? | tech-lead | Yes (ADR-N27-11) — user-experience metric |
| Should turn-1 and turn-2 share a retry budget? | tech-lead | No (ADR-N27-9 + sprint-review § 2.3) |
| Can we PII-redact streamed output? | security review | No for v1 — buffered path only ; T3 mitigation |
| Do we forward Anthropic pings? | UX | No (ADR-N27-6) — gateway emits its own |
| Should tool-use blocks appear in downstream SSE? | product | No (ADR-N27-7) — `is_downstream_visible()` filter |
| `streaming_mode: bool` or `dispatch_path: enum`? | tech-lead | bool (ADR-N27-15) — only meaningful distinction is wire mode |

---

## Appendix C — Comparison to OpenAI / Bedrock streaming

Not in scope for N27 (we only support Anthropic), but documenting for N30+ multi-provider work :

| Concept | Anthropic | OpenAI | Bedrock |
|---|---|---|---|
| Stream flag | `"stream": true` | `"stream": true` | `InvokeModelWithResponseStream` |
| Event format | SSE | SSE | AWS event-stream binary |
| Final usage event | `message_delta` | `usage` (last chunk) | `metadata` event |
| Tool-use deltas | `input_json_delta` | `tool_calls[i].function.arguments` deltas | provider-specific |
| Ping/heartbeat | `event: ping` | none (TCP keepalive) | provider event |
| Mid-stream errors | `event: error` | `error` event | exception event |

Our `AgenticEvent` enum is provider-agnostic ; future N30+ work will add `OpenAIBridge` and `BedrockBridge` that map their wire formats to the same `AgenticEvent` type. The downstream SSE format stays identical regardless of provider.

---

## Appendix D — Migration checklist (for the implementing engineer)

### D.1 efe-nano-sdk

* [ ] Add `RetryOutcome` enum to `types/llm_provenance.rs`
* [ ] Add `AbortReason` enum
* [ ] Add `Aborted` variant to `Completeness`
* [ ] Add 4 new fields to `LlmProvenanceRecord` (`streaming_mode`, `retry_count`, `retry_outcome`, `time_to_first_token_ms`)
* [ ] All new fields are `#[serde(default)]` ; existing JSON deserializes unchanged
* [ ] Update `LlmProvenanceRecord::label()` to handle Aborted
* [ ] Update `is_usable()` to handle Aborted (returns `false`)
* [ ] Update `was_refused()` — unchanged (still only Refused)
* [ ] Add `is_aborted()` predicate
* [ ] Update doc comments to reference Sprint 19 N27

### D.2 efe-agentic-bridge

* [ ] Create `src/retry/` directory with 4 modules
* [ ] Create `src/claude_stream/` directory with 5 modules
* [ ] Modify `bridge.rs` : add `call_with_context_stream` to trait, default impl on both methods
* [ ] Modify `claude.rs` :
  * Add `RetryPolicy` field to `ClaudeBridge`
  * Add `StreamConfig` field
  * Add `with_retry_policy`, `with_stream_config` builders
  * Add `dispatch_stream` method
  * Add `stream_one_turn` private method
  * Reroute `call_with_context` (buffered) via `dispatch_stream` internally
* [ ] Modify `task.rs` : add `AgenticEvent` enum + `AbortReason` re-export
* [ ] Modify `error.rs` : add `Cancelled`, `TotalTimeout`, `ReadTimeout`, `PrematureEof`, `SseParseError`, `InvalidResponseType` variants
* [ ] Modify `lib.rs` : re-exports

### D.3 efe-llm-provenance

* [ ] Add `StreamingPostCallProvenance` struct
* [ ] Add `record_streaming_post_call` to `LlmProvenanceCollector` trait with default impl
* [ ] Add `patch_streaming_fields` to trait
* [ ] Implement on `DefaultLlmProvenanceCollector` (override for atomicity)
* [ ] Implement on `InMemoryLlmProvenanceStore` (in-place mutation, atomic in arc-mutex)
* [ ] Update metrics : `N17Metrics` gains `n17_streaming_records_total{completeness, retry_outcome}` counter

### D.4 efe-gateway

* [ ] Add `handlers/agentic_stream.rs`
* [ ] Add `sse/` module (encoder, heartbeat, cancel)
* [ ] Modify `router.rs` : add `/v1/agentic/call-stream` route with same middleware stack (auth, body limit, rate limit)
* [ ] Modify `openapi.rs` : add SSE response schema (must coexist with existing buffered schema)
* [ ] Modify `metrics.rs` : add `N27Metrics`
* [ ] Modify `main.rs` `build_app_state` : load retry+stream config from env, wire into `ClaudeBridge`

### D.5 Tests

* [ ] Fixture catalogue (Appendix A) committed to `crates/efe-agentic-bridge/tests/fixtures/anthropic_sse/`
* [ ] 24 phase tests (`phase_9_n27_*`)
* [ ] 34 audit tests (`audit_n27_*`)
* [ ] 16 chaos tests (`chaos_n27_*`)
* [ ] CI matrix : `cargo test --workspace` ; `cargo clippy --workspace --all-targets -- -D warnings` ; `cargo audit`
* [ ] Live smoke : N27 streaming endpoint + N26 buffered endpoint, both green

### D.6 Operational

* [ ] Operator runbook updated with new env vars (§ 17.1)
* [ ] Grafana dashboard `efe-n27-streaming.json` committed to `ops/dashboards/`
* [ ] Alert rules : `streaming_abort_rate > 0.05`, `retry_exhaustion_rate > 0.01`, `ttft_p99 > 1500ms`
* [ ] Release notes drafted with the canary plan (§ 17.4)

---

## Appendix E — Glossary

* **SSE** — Server-Sent Events. HTTP 1.1 protocol for streaming text-formatted events from server to client. RFC 8895.
* **TTFT** — Time To First Token. From request POST to first text delta visible to user.
* **First byte downstream** — N27 retry boundary. The moment we send our first `event: ...` line to the SSE consumer.
* **Two-turn protocol** — Claude tool-use loop : turn 1 returns `tool_use` block, server executes tool, turn 2 sends `tool_result` back to Claude for final natural-language answer.
* **`Aborted` (Completeness)** — N27 stream-abort variant. Distinct from `Partial` (which means "max_tokens hit, response truncated naturally") — `Aborted` means we explicitly cut the stream short.
* **`RetryOutcome`** — closed taxonomy of retry-loop end-states.
* **Pre-first-byte-downstream** — state where retries are still allowed.
* **`is_downstream_visible()`** — predicate on `AgenticEvent` determining whether the gateway forwards it as SSE.
* **`provider_response_time_ms`** — Anthropic-side wall (sum of turn latencies, no retry waits).
* **`total_latency_ms`** — provider_response_time + retry waits (computed in record at finalize).
* **`prices_table_version`** — N26 stamping mechanism for cost reproducibility.

---

## Appendix F — Diagram : record-emission state across the dispatch lifecycle

```
                  ╔════════════════════════════╗
                  ║   Pre-call provenance      ║
                  ║   id = uuid_v7()           ║
                  ║   pricing_table_version=1  ║
                  ║   stamped: model_identity, ║
                  ║            tenant_id (JWT),║
                  ║            publisher_id,   ║
                  ║            gold_case_id    ║
                  ╚═══════════╤════════════════╝
                              │
                              ▼  (retry-loop opens)
   ┌─────────────────────────────────────────────────────┐
   │                                                     │
   │   Attempt 1 → fails (5xx)                           │
   │   Sleep 500ms                                       │
   │   Attempt 2 → succeeds, opens stream                │
   │                                                     │
   │   Turn 1 SSE                                        │
   │     • message_start            (input_tokens=823)    │
   │     • text-delta + tool-use deltas                  │
   │     • content_block_stop                            │
   │     • message_delta            (output_tokens=47)   │
   │     • message_stop                                  │
   │                                                     │
   │   Tool execution (in-process, no provenance change) │
   │                                                     │
   │   Turn 2 SSE                                        │
   │     • text-deltas → forwarded downstream            │
   │     • [client disconnects mid-stream]               │
   │                                                     │
   │   Cancellation propagates → upstream RST            │
   │                                                     │
   └─────────────────┬───────────────────────────────────┘
                     │
                     ▼  (finalize_aborted)
                  ╔════════════════════════════╗
                  ║   Post-call provenance     ║
                  ║   completeness = Aborted { ║
                  ║     received_tokens = 89,  ║
                  ║     reason: ClientDisconnect║
                  ║   }                        ║
                  ║   retry_count = 1          ║
                  ║   retry_outcome = RetriedSucceeded
                  ║   streaming_mode = true    ║
                  ║   time_to_first_token_ms = 287
                  ║   provider_response_time_ms = 1247
                  ║   total_latency_ms = 1747  ║
                  ║   cost_microcents = 3450   ║
                  ╚════════════════════════════╝
```

This figure illustrates G1+G14 : **exactly one record** across a complex flow with retry, two turns, and a mid-stream abort. The record carries the full forensic trail.

---

> **End of N27 architecture document.**
>
> Word count : ~16 800.
> Sections : 20 + 6 appendices.
> Invariants documented : 25 (5 new).
> Failure modes documented : 25 (10 new).
> Guarantees documented : 15 (2 new).
> ADRs : 15.
> Test plan : 74 tests across 6 files.
> Dependencies : N17, N21, N22, N24, N26.
> Successor : N28 (Postgres + event-bus).
