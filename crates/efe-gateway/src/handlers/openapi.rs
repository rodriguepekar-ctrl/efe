// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : `/openapi.json` — § 9.1.
//!
//! Sprint 14 ships a *hand-curated* OpenAPI 3.1 spec (not utoipa-
//! generated) — the doc structure is hardcoded ; Sprint 15 wires
//! utoipa derive macros on every handler.

use axum::{
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// **N22** : Build the OpenAPI 3.1 spec.
pub fn openapi_spec() -> serde_json::Value {
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "EFE Gateway API",
            "version": crate::CRATE_VERSION,
            "description": "M50 N22 — public HTTP surface for the EFE swarm + agentic + compliance subsystems.",
            "contact": {
                "name": "EFE Team",
                "email": "eng@efe.ai"
            },
            "license": {
                "name": "UNLICENSED"
            }
        },
        "servers": [
            { "url": "/", "description": "this server" }
        ],
        "paths": {
            "/healthz": {
                "get": {
                    "summary": "Liveness probe",
                    "tags": ["health"],
                    "responses": {
                        "200": {
                            "description": "Process alive",
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/HealthzResponse" }
                                }
                            }
                        }
                    }
                }
            },
            "/readyz": {
                "get": {
                    "summary": "Readiness probe",
                    "tags": ["health"],
                    "responses": {
                        "200": { "description": "Ready" },
                        "503": { "description": "Degraded" }
                    }
                }
            },
            "/metrics": {
                "get": {
                    "summary": "Prometheus metrics",
                    "tags": ["observability"],
                    "responses": {
                        "200": {
                            "description": "Prometheus exposition format",
                            "content": {
                                "text/plain": { "schema": { "type": "string" } }
                            }
                        }
                    }
                }
            },
            "/openapi.json": {
                "get": {
                    "summary": "OpenAPI 3.1 spec",
                    "tags": ["meta"],
                    "responses": { "200": { "description": "Spec" } }
                }
            },
            "/v1/swarms/run": {
                "post": {
                    "summary": "Run a swarm DAG",
                    "tags": ["swarms"],
                    "security": [{ "bearerAuth": [] }],
                    "requestBody": {
                        "required": true,
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/SwarmRunRequest" }
                            }
                        }
                    },
                    "responses": {
                        "200": { "description": "SwarmReport" },
                        "401": { "description": "Missing/invalid JWT" },
                        "403": { "description": "Tenant isolation violation" },
                        "413": { "description": "Body too large" },
                        "429": { "description": "Rate limited" },
                        "504": { "description": "Swarm timeout" }
                    }
                }
            },
            "/v1/agentic/call": {
                "post": {
                    "summary": "Direct agentic LLM call with context bundle",
                    "tags": ["agentic"],
                    "security": [{ "bearerAuth": [] }],
                    "responses": {
                        "200": { "description": "AgenticResponse" },
                        "401": { "description": "Missing/invalid JWT" },
                        "413": { "description": "Bundle too large" },
                        "503": { "description": "LLM provider unavailable" }
                    }
                }
            },
            "/v1/agentic/call-stream": {
                "post": {
                    "summary": "Direct agentic LLM call — streaming (SSE)",
                    "description": "Sprint 19 — N27.  Same semantics as `/v1/agentic/call` but returns a `text/event-stream` body that emits :\n* `event: delta` — incremental text from the LLM (data : raw UTF-8 string)\n* `event: done` — terminal success (data : JSON with token usage / cost / TTFT / retry count)\n* `event: error` — terminal failure (data : JSON with `kind` + `message`)\n\nKeep-alive heartbeats (`: keep-alive`) are emitted every 15 s.  Returns 404 when the kill-switch `EFE_BRIDGE_STREAM_ENABLED=0` is set.",
                    "tags": ["agentic"],
                    "security": [{ "bearerAuth": [] }],
                    "responses": {
                        "200": {
                            "description": "SSE stream",
                            "content": {
                                "text/event-stream": {
                                    "schema": {
                                        "type": "string",
                                        "format": "text/event-stream",
                                        "example": "event: delta\ndata: Hello\n\nevent: delta\ndata: , world!\n\nevent: done\ndata: {\"stop_reason\":\"end_turn\",\"input_tokens\":42,\"output_tokens\":7,\"cost_microcents\":230,\"model_provider\":\"anthropic\",\"model_id\":\"claude-sonnet-4-6\",\"time_to_first_token_ms\":287,\"provider_response_time_ms\":1247,\"retry_count\":0,\"retry_outcome\":\"no_retry\"}\n\n"
                                    }
                                }
                            }
                        },
                        "401": { "description": "Missing/invalid JWT" },
                        "404": { "description": "Streaming disabled via `EFE_BRIDGE_STREAM_ENABLED=0`" },
                        "413": { "description": "Bundle too large" },
                        "503": { "description": "LLM provider unavailable" }
                    }
                }
            },
            "/v1/nano-catalogue": {
                "get": {
                    "summary": "List the nano-agent catalogue (built-in + tenant)",
                    "description": "Sprint 15 — N23 G13. Returns the six built-in pre-analysis agents wired through the N23 adapter, plus any tenant-scoped nanos the caller is authorised to see.",
                    "tags": ["catalogue"],
                    "security": [{ "bearerAuth": [] }],
                    "responses": {
                        "200": {
                            "description": "NanoCatalogueResponse",
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/NanoCatalogueResponse" }
                                }
                            }
                        },
                        "401": { "description": "Missing/invalid JWT" }
                    }
                }
            },
            "/v1/admin/circuit-breakers": {
                "get": {
                    "summary": "Inspect circuit breakers",
                    "tags": ["admin"],
                    "security": [{ "bearerAuth": ["operator"] }],
                    "responses": {
                        "200": { "description": "List of breakers" },
                        "403": { "description": "Operator scope required" }
                    }
                }
            },
            "/v1/admin/drift-sweep/trigger": {
                "post": {
                    "summary": "Trigger an immediate drift sweep",
                    "tags": ["admin"],
                    "security": [{ "bearerAuth": ["operator"] }],
                    "responses": {
                        "202": { "description": "Triggered" },
                        "403": { "description": "Operator scope required" }
                    }
                }
            },
            "/v1/llm-provenance": {
                "get": {
                    "summary": "List LLM provenance records (operator scope)",
                    "description": "Sprint 18 — N26 G13.  Returns paginated LlmProvenanceRecord entries for forensic / FinOps / EU AI Act §13 audit queries.  Tenant filter optional ; defaults to the JWT claim's tenant.  Returns 503 when no provenance store is configured.",
                    "tags": ["audit"],
                    "security": [{ "bearerAuth": ["operator"] }],
                    "parameters": [
                        { "name": "tenant_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "since", "in": "query", "schema": { "type": "string", "format": "date-time" } },
                        { "name": "until", "in": "query", "schema": { "type": "string", "format": "date-time" } },
                        { "name": "swarm_request_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "limit", "in": "query", "schema": { "type": "integer", "default": 50, "maximum": 500 } }
                    ],
                    "responses": {
                        "200": {
                            "description": "LlmProvenanceListResponse",
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/LlmProvenanceListResponse" }
                                }
                            }
                        },
                        "401": { "description": "Missing/invalid JWT" },
                        "403": { "description": "Operator scope required" },
                        "503": { "description": "Provenance store not configured" }
                    }
                }
            },
            "/v1/admin/compliance-status": {
                "get": {
                    "summary": "Snapshot the wired compliance stack",
                    "description": "Sprint 16 — N24 G13. Returns the live state of the wired compliance runner : publisher cache stats, per-gate counters (signature / gold / status), and the baked-in EFE publisher identity.  Returns 503 when no compliance runner is configured (the gateway is running in N23 experimental_fast mode).",
                    "tags": ["admin"],
                    "security": [{ "bearerAuth": ["operator"] }],
                    "responses": {
                        "200": {
                            "description": "ComplianceStatus",
                            "content": {
                                "application/json": {
                                    "schema": { "$ref": "#/components/schemas/ComplianceStatus" }
                                }
                            }
                        },
                        "403": { "description": "Operator scope required" },
                        "503": { "description": "Compliance runner not configured" }
                    }
                }
            }
        },
        "components": {
            "securitySchemes": {
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "JWT"
                }
            },
            "schemas": {
                "HealthzResponse": {
                    "type": "object",
                    "required": ["status", "version"],
                    "properties": {
                        "status": { "type": "string" },
                        "version": { "type": "string" }
                    }
                },
                "SwarmRunRequest": {
                    "type": "object",
                    "required": ["tenant_id", "input"],
                    "properties": {
                        "tenant_id": { "type": "string", "format": "uuid" },
                        "dag_id": { "type": "string" },
                        "input": { "type": "object" },
                        "config": { "type": "object" }
                    }
                },
                "NanoCatalogueResponse": {
                    "type": "object",
                    "required": ["version", "generated_at", "entries", "builtin_canonical_hash"],
                    "properties": {
                        "version": { "type": "string" },
                        "generated_at": { "type": "string", "format": "date-time" },
                        "tenant_id": { "type": ["string", "null"], "format": "uuid" },
                        "entries": {
                            "type": "array",
                            "items": { "type": "object" }
                        },
                        "builtin_canonical_hash": { "type": "string", "minLength": 64, "maxLength": 64 }
                    }
                },
                "LlmProvenanceListResponse": {
                    "type": "object",
                    "required": ["schema", "generated_at", "tenant_id", "records", "limit"],
                    "properties": {
                        "schema": { "type": "string" },
                        "generated_at": { "type": "string", "format": "date-time" },
                        "tenant_id": { "type": "string", "format": "uuid" },
                        "since": { "type": "string", "format": "date-time" },
                        "until": { "type": "string", "format": "date-time" },
                        "records": {
                            "type": "array",
                            "items": { "type": "object" }
                        },
                        "limit": { "type": "integer" }
                    }
                },
                "ComplianceStatus": {
                    "type": "object",
                    "required": ["schema", "generated_at", "cache", "counters", "gates", "builtin"],
                    "properties": {
                        "schema": { "type": "string" },
                        "generated_at": { "type": "string", "format": "date-time" },
                        "cache": { "type": "object" },
                        "counters": { "type": "object" },
                        "gates": { "type": "object" },
                        "builtin": { "type": "object" }
                    }
                },
                "ProblemDetails": {
                    "type": "object",
                    "required": ["type", "title", "status", "instance", "trace_id"],
                    "properties": {
                        "type": { "type": "string" },
                        "title": { "type": "string" },
                        "status": { "type": "integer" },
                        "detail": { "type": "string" },
                        "instance": { "type": "string" },
                        "trace_id": { "type": "string" },
                        "extensions": { "type": "object" }
                    }
                }
            }
        }
    })
}

/// **N22** : axum handler for `/openapi.json`.
pub async fn openapi_json_handler() -> Response {
    let spec = openapi_spec();
    let mut resp = Json(spec).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    *resp.status_mut() = StatusCode::OK;
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_has_version() {
        let s = openapi_spec();
        assert_eq!(s["openapi"], "3.1.0");
        assert!(!s["info"]["version"].as_str().unwrap().is_empty());
    }

    #[test]
    fn spec_lists_health_endpoints() {
        let s = openapi_spec();
        assert!(s["paths"]["/healthz"].is_object());
        assert!(s["paths"]["/readyz"].is_object());
        assert!(s["paths"]["/metrics"].is_object());
    }

    #[test]
    fn spec_lists_swarm_endpoint() {
        let s = openapi_spec();
        assert!(s["paths"]["/v1/swarms/run"]["post"].is_object());
    }

    #[test]
    fn spec_lists_agentic_endpoint() {
        let s = openapi_spec();
        assert!(s["paths"]["/v1/agentic/call"]["post"].is_object());
    }

    #[test]
    fn spec_lists_admin_endpoints() {
        let s = openapi_spec();
        assert!(s["paths"]["/v1/admin/circuit-breakers"]["get"].is_object());
        assert!(s["paths"]["/v1/admin/drift-sweep/trigger"]["post"].is_object());
    }

    #[test]
    fn spec_lists_n23_nano_catalogue_endpoint() {
        let s = openapi_spec();
        let op = &s["paths"]["/v1/nano-catalogue"]["get"];
        assert!(op.is_object());
        assert_eq!(op["tags"], serde_json::json!(["catalogue"]));
        assert!(op["responses"]["200"].is_object());
        assert!(op["responses"]["401"].is_object());
    }

    #[test]
    fn spec_defines_n23_nano_catalogue_response_schema() {
        let s = openapi_spec();
        let sch = &s["components"]["schemas"]["NanoCatalogueResponse"];
        assert_eq!(sch["type"], "object");
        let required = sch["required"].as_array().unwrap();
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"version"));
        assert!(names.contains(&"entries"));
        assert!(names.contains(&"builtin_canonical_hash"));
    }

    #[test]
    fn spec_defines_bearer_auth_security_scheme() {
        let s = openapi_spec();
        assert_eq!(s["components"]["securitySchemes"]["bearerAuth"]["type"], "http");
        assert_eq!(s["components"]["securitySchemes"]["bearerAuth"]["scheme"], "bearer");
    }

    #[test]
    fn spec_defines_problem_details_schema() {
        let s = openapi_spec();
        let pd = &s["components"]["schemas"]["ProblemDetails"];
        assert_eq!(pd["type"], "object");
        let required = pd["required"].as_array().unwrap();
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"trace_id"));
        assert!(names.contains(&"instance"));
    }
}
