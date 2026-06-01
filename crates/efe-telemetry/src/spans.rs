//! Sprint **D13** — span name constants for OpenTelemetry traces.
//!
//! The architecture document
//! (`docs/bench/d13_observability_dashboards_architecture.md` § 7.2)
//! pins the span hierarchy to the names below. Handler code uses
//! these constants exclusively — never inlines string literals — so
//! that renames are enforceable via the type system.
//!
//! # Naming convention
//!
//! `efe.<component>.<action>` — three-segment dot-separated, all
//! lowercase, snake_case for the action.
//!
//! # Span attributes (architecture § 7.2 + threat model T-13-3)
//!
//! Whitelist enforced at the OTel layer :
//!
//! | Span               | Allowed attributes |
//! |--------------------|--------------------|
//! | `efe.swarm.*`      | tenant, catalog_key, n_classes, round_idx, temperature |
//! | `efe.embedder.*`   | model, n_tokens, dim |
//! | `efe.cone.*`       | n_anchors, dim, class_name, distance |
//! | `efe.fusion.*`     | round_idx, temperature |
//! | `efe.catalog.*`    | tenant, name, version, op |
//! | `efe.auth.*`       | subject (sub claim), tenant, scopes (sorted, comma-joined) |
//! | `efe.ratelimit.*`  | tenant, bucket, outcome |
//!
//! NEVER attached to a span :
//!   * user input text (PII risk)
//!   * email / phone / address
//!   * JWT tokens
//!   * raw embeddings
//!   * full anchor floats

#![allow(missing_docs)]

// =====================================================================
// Swarm
// =====================================================================

pub const SWARM_CLASSIFY:    &str = "efe.swarm.classify";
pub const SWARM_FUSE:        &str = "efe.swarm.fuse";

// =====================================================================
// Embedder
// =====================================================================

pub const EMBEDDER_ENCODE:   &str = "efe.embedder.encode";
pub const EMBEDDER_LOAD:     &str = "efe.embedder.load";

// =====================================================================
// Cone
// =====================================================================

pub const CONE_FIT:          &str = "efe.cone.fit";
pub const CONE_CLASSIFY:     &str = "efe.cone.classify";
pub const CONE_PROJECT:      &str = "efe.cone.project";

// =====================================================================
// Fusion
// =====================================================================

pub const FUSION_ROUND:      &str = "efe.fusion.round";

// =====================================================================
// Catalog (D12-bis emit-sites)
// =====================================================================

pub const CATALOG_REGISTER:        &str = "efe.catalog.register";
pub const CATALOG_EVOLVE:          &str = "efe.catalog.evolve";
pub const CATALOG_GET:             &str = "efe.catalog.get";
pub const CATALOG_LIST:            &str = "efe.catalog.list";
pub const CATALOG_LIST_VERSIONS:   &str = "efe.catalog.list_versions";
pub const CATALOG_DELETE:          &str = "efe.catalog.delete";

// =====================================================================
// Sprint **D14** — Postgres backend spans
// =====================================================================
//
// One span per trait method (mirroring `CATALOG_*`) ; the `efe.pg.*`
// namespace lets operators filter PG-specific spans in Tempo / Jaeger
// independently from the high-level handler spans.

pub const PG_REGISTER:             &str = "efe.pg.register";
pub const PG_EVOLVE:               &str = "efe.pg.evolve";
pub const PG_GET:                  &str = "efe.pg.get";
pub const PG_LIST_CATALOGS:        &str = "efe.pg.list_catalogs";
pub const PG_LIST_VERSIONS:        &str = "efe.pg.list_versions";
pub const PG_DELETE:               &str = "efe.pg.delete";
pub const PG_MIGRATE:              &str = "efe.pg.migrate";

// =====================================================================
// Sprint **D14-bis** — HMAC-chained audit signing spans
// =====================================================================
//
// `efe.audit.sign` covers one HmacChainedAuditSink emit (begin tx,
// advisory lock, hydrate prev_hmac, compute HMAC, INSERT, commit).
// `efe.audit.verify` covers one verify_chain replay pass.

pub const AUDIT_SIGN:              &str = "efe.audit.sign";
pub const AUDIT_VERIFY:            &str = "efe.audit.verify";

// =====================================================================
// Sprint **D16** — Embedder bundling spans
// =====================================================================
//
// `efe.bundle.load` covers one `load_bundle_dir` invocation
// (read manifest → verify sig → verify components → instantiate
// runtime). Sub-spans `efe.bundle.signature`, `efe.bundle.verify_files`,
// `efe.bundle.runtime_init` give per-step latency.
//
// `efe.embedder.encode` covers one `Bundle::encode_one` call (gRPC
// hot path). `efe.embedder.tokenize` and `efe.embedder.forward`
// are the underlying sub-steps, useful for debugging tokenizer
// vs. forward-pass tail latency.

pub const BUNDLE_LOAD:             &str = "efe.bundle.load";
pub const BUNDLE_SIGNATURE:        &str = "efe.bundle.signature";
pub const BUNDLE_VERIFY_FILES:     &str = "efe.bundle.verify_files";
pub const BUNDLE_RUNTIME_INIT:     &str = "efe.bundle.runtime_init";
pub const EMBEDDER_TOKENIZE:       &str = "efe.embedder.tokenize";
pub const EMBEDDER_FORWARD:        &str = "efe.embedder.forward";
pub const EMBEDDER_POOL:           &str = "efe.embedder.pool";

// =====================================================================
// Sprint **D15** — OIDC + tenant onboarding spans
// =====================================================================
//
// `efe.oidc.validate` covers one `OidcValidator::validate` call
// (header parsing, kid lookup, jsonwebtoken::decode, claims/audience
// validation, tenant resolution).
// `efe.jwks.fetch` covers one `JwksCache::refresh` HTTP fetch.
// `efe.tenant.onboard` covers one `TenantStore` mutation (activate,
// suspend) emitted by the `efe-onboard` CLI.

pub const OIDC_VALIDATE:           &str = "efe.oidc.validate";
pub const JWKS_FETCH:              &str = "efe.jwks.fetch";
pub const TENANT_ONBOARD:          &str = "efe.tenant.onboard";

// =====================================================================
// Auth + ratelimit
// =====================================================================

pub const AUTH_VERIFY:        &str = "efe.auth.verify";
pub const RATELIMIT_ACQUIRE:  &str = "efe.ratelimit.acquire";

// =====================================================================
// Span attribute name constants (whitelist)
// =====================================================================
//
// Use these constants when annotating spans so the attribute names
// stay consistent across components.

pub const ATTR_TENANT:        &str = "tenant";
pub const ATTR_CATALOG_KEY:   &str = "catalog_key";
pub const ATTR_N_CLASSES:     &str = "n_classes";
pub const ATTR_ROUND_IDX:     &str = "round_idx";
pub const ATTR_TEMPERATURE:   &str = "temperature";
pub const ATTR_MODEL:         &str = "model";
pub const ATTR_N_TOKENS:      &str = "n_tokens";
pub const ATTR_DIM:           &str = "dim";
pub const ATTR_N_ANCHORS:     &str = "n_anchors";
pub const ATTR_CLASS_NAME:    &str = "class_name";
pub const ATTR_DISTANCE:      &str = "distance";
pub const ATTR_NAME:          &str = "name";
pub const ATTR_VERSION:       &str = "version";
pub const ATTR_OP:            &str = "op";
pub const ATTR_SUBJECT:       &str = "subject";
pub const ATTR_SCOPES:        &str = "scopes";
pub const ATTR_BUCKET:        &str = "bucket";
pub const ATTR_OUTCOME:       &str = "outcome";

// =====================================================================
// Forbidden attribute names (defense in depth)
// =====================================================================

/// Names we explicitly forbid on spans because they have :
///   * unbounded cardinality (e.g. `request_id`)
///   * PII risk (e.g. `email`, `text`)
///   * security risk (e.g. `jwt`, `token`)
///
/// The OTel layer rejects any span attribute whose key matches one of
/// these patterns. Test : `test_d13_span_forbidden_attribute_set`.
pub const FORBIDDEN_ATTR_PREFIXES: &[&str] = &[
    "request_id",
    "trace_id",     // (kept on the span itself, not as an attribute)
    "span_id",
    "text",
    "input",
    "raw_input",
    "email",
    "phone",
    "address",
    "jwt",
    "token",
    "password",
    "secret",
    "api_key",
    "customer_email",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_name_format_is_three_segment_lowercase() {
        let all_spans = [
            SWARM_CLASSIFY, SWARM_FUSE,
            EMBEDDER_ENCODE, EMBEDDER_LOAD,
            CONE_FIT, CONE_CLASSIFY, CONE_PROJECT,
            FUSION_ROUND,
            CATALOG_REGISTER, CATALOG_EVOLVE, CATALOG_GET,
            CATALOG_LIST, CATALOG_LIST_VERSIONS, CATALOG_DELETE,
            AUTH_VERIFY, RATELIMIT_ACQUIRE,
        ];
        for s in &all_spans {
            assert!(s.starts_with("efe."), "span {s:?} must start with efe.");
            assert_eq!(
                s.matches('.').count(),
                2,
                "span {s:?} must have exactly 2 dots (efe.<component>.<action>)"
            );
            assert_eq!(s.to_lowercase(), *s, "span {s:?} must be all-lowercase");
        }
    }

    #[test]
    fn no_span_constant_collides_with_forbidden_attribute() {
        for span in &[
            SWARM_CLASSIFY, EMBEDDER_ENCODE, CATALOG_REGISTER,
            AUTH_VERIFY,
        ] {
            for forbidden in FORBIDDEN_ATTR_PREFIXES {
                assert!(
                    !span.contains(forbidden),
                    "span {span:?} contains forbidden token {forbidden:?}"
                );
            }
        }
    }

    #[test]
    fn forbidden_attr_list_includes_pii_categories() {
        // Sanity : the forbidden list must include the three high-
        // risk categories : PII, secrets, unbounded-cardinality.
        let all = FORBIDDEN_ATTR_PREFIXES.join(",");
        assert!(all.contains("email"));
        assert!(all.contains("token"));
        assert!(all.contains("request_id"));
        assert!(all.contains("text"));
    }

    #[test]
    fn allowed_attribute_constants_are_concise() {
        // Audit hygiene : attribute names ≤ 16 chars to keep span
        // payloads small.
        for &attr in &[
            ATTR_TENANT, ATTR_CATALOG_KEY, ATTR_N_CLASSES, ATTR_ROUND_IDX,
            ATTR_TEMPERATURE, ATTR_MODEL, ATTR_N_TOKENS, ATTR_DIM,
            ATTR_N_ANCHORS, ATTR_CLASS_NAME, ATTR_DISTANCE, ATTR_NAME,
            ATTR_VERSION, ATTR_OP, ATTR_SUBJECT, ATTR_SCOPES, ATTR_BUCKET,
            ATTR_OUTCOME,
        ] {
            assert!(attr.len() <= 16, "attribute {attr:?} too long");
        }
    }

    #[test]
    fn six_catalog_rpc_spans_present() {
        // Architecture § 7.2 promises 6 catalog RPC spans matching
        // the 6 RPCs in catalog.proto.
        let catalog = [
            CATALOG_REGISTER, CATALOG_EVOLVE, CATALOG_GET,
            CATALOG_LIST, CATALOG_LIST_VERSIONS, CATALOG_DELETE,
        ];
        assert_eq!(catalog.len(), 6);
        // All distinct.
        let unique: std::collections::HashSet<_> = catalog.iter().collect();
        assert_eq!(unique.len(), 6);
    }
}
