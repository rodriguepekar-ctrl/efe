//! Sprint **D12-bis** — proto ⇄ trait conversion helpers.
//!
//! Translates between :
//!
//!   * `crate::proto::v1::catalog::*` (tonic-generated, wire layer)
//!   * `crate::catalog::*` (domain trait types — D12)
//!
//! Validation happens **at the traversal** : every helper returns
//! `Result<T, tonic::Status>` and rejects malformed proto with a
//! `Status::invalid_argument` *before* the trait sees them. The
//! trait does its own validation via `CatalogPolicy::validate_*`,
//! but pre-validating here :
//!
//!   * surfaces the error with the right gRPC code (`InvalidArgument`)
//!     even if the trait method panics (defensive depth).
//!   * Saves a registry call when the input is grossly malformed.
//!   * Lets the histogram observer record `result="invalid_argument"`
//!     on the wire latency, useful for SLO debugging.
//!
//! All conversions are pure functions ; they don't allocate
//! observability events or touch global state. Tests live next to
//! the helpers and verify round-trip fidelity.

use std::collections::BTreeMap;

use tonic::Status;

use crate::catalog::{
    CatalogClass, CatalogContent, CatalogKey, CatalogMetadata, CompiledClass, ConePolicy,
    ContentDigest, EmbedderSpec, EvolveOp, RegisteredCatalog, ThresholdPolicy,
};
use crate::proto::v1::catalog as pb;

// =====================================================================
// proto → trait
// =====================================================================

/// Convert a wire-form `CatalogContent` into its domain shape.
///
/// **Validation invariants enforced here** :
///
///   * `content` is `Some(...)` (proto3 messages are optional on the
///     wire, but the application-level contract requires presence).
///   * `embedder` is present and `dim > 0`.
///   * `classes.len() >= 2` (matches `CatalogPolicy` MIN constant).
///   * `cone_policy` is present (audit fix M1 — silently defaulting to
///     `0.7` when absent surfaced no validation error to the operator
///     and could mask integration mistakes).
///   * `thresholds` is present (audit fix M1 — silently defaulting to
///     `(0.0, 0.0)` produced an opaque downstream error
///     "confidence_medium must be in (0,1) (got 0)" because the
///     registry's `validate_thresholds` requires
///     `confidence_medium > 0`. Returning a clear "thresholds is
///     required" InvalidArgument here keeps the error blame at the
///     handler boundary).
pub fn proto_content_to_trait(p: Option<pb::CatalogContent>) -> Result<CatalogContent, Status> {
    let p = p.ok_or_else(|| Status::invalid_argument("CatalogContent is required"))?;

    let embedder = p
        .embedder
        .ok_or_else(|| Status::invalid_argument("CatalogContent.embedder is required"))?;
    if embedder.dim == 0 {
        return Err(Status::invalid_argument("embedder.dim must be > 0"));
    }
    if embedder.family.is_empty() || embedder.model.is_empty() {
        return Err(Status::invalid_argument(
            "embedder.family and embedder.model must be non-empty",
        ));
    }
    if p.classes.len() < 2 {
        return Err(Status::invalid_argument(format!(
            "content.classes must have ≥ 2 entries, got {}",
            p.classes.len(),
        )));
    }

    let classes: Vec<CatalogClass> = p
        .classes
        .into_iter()
        .map(proto_class_to_trait)
        .collect::<Result<Vec<_>, _>>()?;

    // Audit fix M1 — require explicit ConePolicy + ThresholdPolicy.
    let cone_policy_pb = p.cone_policy.ok_or_else(|| {
        Status::invalid_argument(
            "CatalogContent.cone_policy is required \
         (set max_projection_distance ∈ (0, 1])",
        )
    })?;
    if !(cone_policy_pb.max_projection_distance > 0.0
        && cone_policy_pb.max_projection_distance <= 1.0)
    {
        return Err(Status::invalid_argument(format!(
            "cone_policy.max_projection_distance must be in (0, 1], got {}",
            cone_policy_pb.max_projection_distance,
        )));
    }

    let thresholds_pb = p.thresholds.ok_or_else(|| {
        Status::invalid_argument(
            "CatalogContent.thresholds is required \
         (set confidence_high ∈ (0, 1] and confidence_medium ∈ (0, 1) \
         with confidence_high ≥ confidence_medium)",
        )
    })?;

    Ok(CatalogContent {
        embedder: EmbedderSpec {
            family: embedder.family,
            model: embedder.model,
            dim: embedder.dim,
            normalize: embedder.normalize,
        },
        classes,
        cone_policy: ConePolicy {
            max_projection_distance: cone_policy_pb.max_projection_distance,
        },
        thresholds: ThresholdPolicy {
            confidence_high: thresholds_pb.confidence_high,
            confidence_medium: thresholds_pb.confidence_medium,
        },
    })
}

/// Convert a single class.
///
/// **Validation invariants enforced here** :
///
///   * `name` is non-empty after trim.
///   * `canonical_sentences` has ≥ 1 entry.
pub fn proto_class_to_trait(c: pb::CatalogClass) -> Result<CatalogClass, Status> {
    let name = c.name.trim().to_owned();
    if name.is_empty() {
        return Err(Status::invalid_argument("class.name must be non-empty"));
    }
    if c.canonical_sentences.is_empty() {
        return Err(Status::invalid_argument(format!(
            "class {name:?}.canonical_sentences must have ≥ 1 entry",
        )));
    }
    Ok(CatalogClass {
        name,
        description: c.description,
        canonical_sentences: c.canonical_sentences,
        color: c.color,
        deprecated: c.deprecated,
    })
}

/// Convert a wire-form `CatalogEvolutionOp` (oneof) into the
/// discriminated `EvolveOp` enum.
pub fn proto_op_to_trait(p: pb::CatalogEvolutionOp) -> Result<EvolveOp, Status> {
    use pb::catalog_evolution_op::Op as OneOf;
    let inner = p.op.ok_or_else(|| {
        Status::invalid_argument(
            "evolve op is required (one of append_class, remove_class, \
         update_thresholds, replace_class_anchors, recompile)",
        )
    })?;
    Ok(match inner {
        OneOf::AppendClass(o) => {
            let cls = o
                .cls
                .ok_or_else(|| Status::invalid_argument("AppendClassOp.cls is required"))?;
            EvolveOp::AppendClass {
                class: proto_class_to_trait(cls)?,
            }
        }
        OneOf::RemoveClass(o) => {
            let class_name = o.class_name.trim().to_owned();
            if class_name.is_empty() {
                return Err(Status::invalid_argument(
                    "RemoveClassOp.class_name is required",
                ));
            }
            EvolveOp::RemoveClass { class_name }
        }
        OneOf::UpdateThresholds(o) => {
            let thresholds = o.thresholds.ok_or_else(|| {
                Status::invalid_argument("UpdateThresholdsOp.thresholds is required")
            })?;
            EvolveOp::UpdateThresholds {
                thresholds: ThresholdPolicy {
                    confidence_high: thresholds.confidence_high,
                    confidence_medium: thresholds.confidence_medium,
                },
            }
        }
        OneOf::ReplaceClassAnchors(o) => {
            let class_name = o.class_name.trim().to_owned();
            if class_name.is_empty() {
                return Err(Status::invalid_argument(
                    "ReplaceClassAnchorsOp.class_name is required",
                ));
            }
            if o.canonical_sentences.is_empty() {
                return Err(Status::invalid_argument(format!(
                    "ReplaceClassAnchorsOp for {class_name:?} requires \
                     a non-empty canonical_sentences list",
                )));
            }
            EvolveOp::ReplaceClassAnchors {
                class_name,
                canonical_sentences: o.canonical_sentences,
            }
        }
        OneOf::Recompile(o) => {
            let new_embedder = o
                .new_embedder
                .ok_or_else(|| Status::invalid_argument("RecompileOp.new_embedder is required"))?;
            if new_embedder.dim == 0 {
                return Err(Status::invalid_argument(
                    "RecompileOp.new_embedder.dim must be > 0",
                ));
            }
            EvolveOp::Recompile {
                new_embedder: EmbedderSpec {
                    family: new_embedder.family,
                    model: new_embedder.model,
                    dim: new_embedder.dim,
                    normalize: new_embedder.normalize,
                },
            }
        }
    })
}

// =====================================================================
// trait → proto
// =====================================================================

/// Convert a trait `CatalogContent` back to its wire form.
pub fn trait_content_to_proto(c: CatalogContent) -> pb::CatalogContent {
    pb::CatalogContent {
        embedder: Some(pb::EmbedderSpec {
            family: c.embedder.family,
            model: c.embedder.model,
            dim: c.embedder.dim,
            normalize: c.embedder.normalize,
        }),
        classes: c
            .classes
            .into_iter()
            .map(|cls| pb::CatalogClass {
                name: cls.name,
                description: cls.description,
                canonical_sentences: cls.canonical_sentences,
                color: cls.color,
                deprecated: cls.deprecated,
            })
            .collect(),
        cone_policy: Some(pb::ConePolicy {
            max_projection_distance: c.cone_policy.max_projection_distance,
        }),
        thresholds: Some(pb::ThresholdPolicy {
            confidence_high: c.thresholds.confidence_high,
            confidence_medium: c.thresholds.confidence_medium,
        }),
    }
}

/// Convert a `CatalogMetadata` to its proto form, including the
/// `google.protobuf.Timestamp` for `created_at`.
pub fn trait_metadata_to_proto(m: CatalogMetadata) -> pb::CatalogMetadata {
    let secs = m.created_at_unix_ms / 1_000;
    let nanos = ((m.created_at_unix_ms % 1_000) * 1_000_000) as i32;
    let labels: std::collections::HashMap<String, String> = m.labels.into_iter().collect();
    pb::CatalogMetadata {
        created_at: Some(prost_types::Timestamp {
            seconds: secs,
            nanos,
        }),
        created_by: m.created_by,
        description: m.description,
        parent_key: m.parent_key,
        evolution_op: m.evolution_op,
        labels,
        content_sha256: m.content_sha256.0,
    }
}

/// Build the proto `CompiledClass` from a domain `CompiledClass`.
fn trait_compiled_class_to_proto(c: CompiledClass) -> pb::CompiledClass {
    pb::CompiledClass {
        class_name: c.class_name,
        anchors_blob: c.anchors_blob,
        num_anchors: c.num_anchors,
        embedder_fingerprint: c.embedder_fingerprint,
    }
}

/// Convert a trait `RegisteredCatalog` to a `CanonicalCatalog`
/// (used by `GetCatalog` response).
pub fn registered_to_canonical_proto(r: RegisteredCatalog) -> pb::CanonicalCatalog {
    let compiled = r
        .compiled
        .unwrap_or_default()
        .into_iter()
        .map(trait_compiled_class_to_proto)
        .collect();
    pb::CanonicalCatalog {
        content: Some(trait_content_to_proto(r.content)),
        metadata: Some(trait_metadata_to_proto(r.metadata)),
        compiled,
    }
}

/// Convert a trait `RegisteredCatalog` to a `CatalogHandle`
/// (used by `RegisterCatalog` and `EvolveCatalog` responses).
pub fn registered_to_handle(r: RegisteredCatalog) -> pb::CatalogHandle {
    let key = r.key.to_wire();
    pb::CatalogHandle {
        key,
        catalog: Some(registered_to_canonical_proto(r)),
    }
}

// =====================================================================
// Construction helpers (handler-side glue)
// =====================================================================

/// Build the initial `CatalogMetadata` stamped at register time.
///
/// `evolution_op`, `parent_key`, `content_sha256` are left blank ;
/// the trait's `register()` overwrites them. This is documented in
/// `crates/efe-server/src/catalog/memory.rs` and pinned by the
/// metadata round-trip tests.
pub fn build_initial_metadata(
    principal: &str,
    description: &str,
    labels: &std::collections::HashMap<String, String>,
) -> CatalogMetadata {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let label_map: BTreeMap<String, String> =
        labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    CatalogMetadata {
        created_at_unix_ms: now_ms,
        created_by: principal.to_owned(),
        description: description.to_owned(),
        parent_key: String::new(),
        evolution_op: String::new(),
        labels: label_map,
        content_sha256: ContentDigest(String::new()),
    }
}

/// Build a metadata override for `EvolveCatalog` from the proto
/// override field. Returns a sensible default when the proto field
/// is absent (the trait will overwrite the controlled fields anyway).
pub fn build_evolve_metadata(
    principal: &str,
    override_proto: Option<pb::CatalogMetadata>,
) -> CatalogMetadata {
    match override_proto {
        Some(md) => CatalogMetadata {
            created_at_unix_ms: 0, // overwritten
            created_by: if !md.created_by.is_empty() {
                md.created_by
            } else {
                principal.to_owned()
            },
            description: md.description,
            parent_key: String::new(),
            evolution_op: String::new(),
            labels: md.labels.into_iter().collect(),
            content_sha256: ContentDigest(String::new()),
        },
        None => build_initial_metadata(principal, "", &Default::default()),
    }
}

/// Convert a `CatalogKey` parsed from a wire string into the trait
/// type. Mirrors `CatalogKey::parse`.
pub fn parse_canonical_key(wire: &str) -> Result<CatalogKey, Status> {
    CatalogKey::parse(wire)
        .map_err(|e| Status::invalid_argument(format!("invalid canonical key {wire:?}: {e}",)))
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn sample_proto_class() -> pb::CatalogClass {
        pb::CatalogClass {
            name: "billing".to_owned(),
            description: "Billing issues".to_owned(),
            canonical_sentences: vec![
                "I want a refund".to_owned(),
                "my card was charged twice".to_owned(),
            ],
            color: "#ef4444".to_owned(),
            deprecated: false,
        }
    }

    fn sample_proto_content() -> pb::CatalogContent {
        pb::CatalogContent {
            embedder: Some(pb::EmbedderSpec {
                family: "sentence-transformers".to_owned(),
                model: "paraphrase-multilingual-MiniLM-L12-v2".to_owned(),
                dim: 384,
                normalize: "l2".to_owned(),
            }),
            classes: vec![
                sample_proto_class(),
                pb::CatalogClass {
                    name: "login".to_owned(),
                    description: "".to_owned(),
                    canonical_sentences: vec!["I cannot log in".to_owned()],
                    color: "".to_owned(),
                    deprecated: false,
                },
            ],
            cone_policy: Some(pb::ConePolicy {
                max_projection_distance: 0.7,
            }),
            thresholds: Some(pb::ThresholdPolicy {
                confidence_high: 0.8,
                confidence_medium: 0.5,
            }),
        }
    }

    // ── proto → trait ──────────────────────────────────────────────

    #[test]
    fn proto_content_to_trait_happy_path() {
        let c = proto_content_to_trait(Some(sample_proto_content())).unwrap();
        assert_eq!(c.classes.len(), 2);
        assert_eq!(c.classes[0].name, "billing");
        assert_eq!(c.embedder.dim, 384);
    }

    #[test]
    fn proto_content_to_trait_rejects_none() {
        let err = proto_content_to_trait(None).unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn proto_content_to_trait_rejects_missing_embedder() {
        let mut p = sample_proto_content();
        p.embedder = None;
        let err = proto_content_to_trait(Some(p)).unwrap_err();
        assert!(err.message().contains("embedder"));
    }

    #[test]
    fn proto_content_to_trait_rejects_dim_zero() {
        let mut p = sample_proto_content();
        p.embedder.as_mut().unwrap().dim = 0;
        let err = proto_content_to_trait(Some(p)).unwrap_err();
        assert!(err.message().contains("dim"));
    }

    #[test]
    fn proto_content_to_trait_rejects_one_class() {
        let mut p = sample_proto_content();
        p.classes.pop();
        let err = proto_content_to_trait(Some(p)).unwrap_err();
        assert!(err.message().contains("≥ 2"));
    }

    #[test]
    fn proto_content_to_trait_rejects_empty_embedder_family() {
        let mut p = sample_proto_content();
        p.embedder.as_mut().unwrap().family = String::new();
        let err = proto_content_to_trait(Some(p)).unwrap_err();
        assert!(err.message().contains("family"));
    }

    #[test]
    fn proto_class_rejects_empty_name() {
        let mut c = sample_proto_class();
        c.name = "  ".to_owned();
        let err = proto_class_to_trait(c).unwrap_err();
        assert!(err.message().contains("name"));
    }

    #[test]
    fn proto_class_rejects_empty_sentences() {
        let mut c = sample_proto_class();
        c.canonical_sentences.clear();
        let err = proto_class_to_trait(c).unwrap_err();
        assert!(err.message().contains("canonical_sentences"));
    }

    // Audit D12-bis fix M1 — explicit cone_policy + thresholds.

    #[test]
    fn proto_content_rejects_missing_cone_policy() {
        let mut p = sample_proto_content();
        p.cone_policy = None;
        let err = proto_content_to_trait(Some(p)).unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("cone_policy"));
    }

    #[test]
    fn proto_content_rejects_invalid_cone_policy_distance() {
        let mut p = sample_proto_content();
        p.cone_policy = Some(pb::ConePolicy {
            max_projection_distance: 0.0,
        });
        let err = proto_content_to_trait(Some(p)).unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("max_projection_distance"));
    }

    #[test]
    fn proto_content_rejects_cone_policy_distance_over_one() {
        let mut p = sample_proto_content();
        p.cone_policy = Some(pb::ConePolicy {
            max_projection_distance: 1.5,
        });
        let err = proto_content_to_trait(Some(p)).unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn proto_content_rejects_missing_thresholds() {
        let mut p = sample_proto_content();
        p.thresholds = None;
        let err = proto_content_to_trait(Some(p)).unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().contains("thresholds"),
            "expected thresholds in error message, got {:?}",
            err.message()
        );
    }

    // ── round-trip ─────────────────────────────────────────────────

    #[test]
    fn round_trip_content_preserves_classes() {
        let original = sample_proto_content();
        let trait_form = proto_content_to_trait(Some(original.clone())).unwrap();
        let proto_back = trait_content_to_proto(trait_form);
        assert_eq!(original.classes.len(), proto_back.classes.len());
        assert_eq!(original.classes[0].name, proto_back.classes[0].name);
        assert_eq!(
            original.classes[0].canonical_sentences,
            proto_back.classes[0].canonical_sentences,
        );
    }

    #[test]
    fn round_trip_content_preserves_embedder() {
        let original = sample_proto_content();
        let trait_form = proto_content_to_trait(Some(original.clone())).unwrap();
        let proto_back = trait_content_to_proto(trait_form);
        assert_eq!(
            original.embedder.unwrap().model,
            proto_back.embedder.unwrap().model
        );
    }

    // ── EvolveOp ───────────────────────────────────────────────────

    #[test]
    fn proto_op_append_class_round_trips() {
        let p = pb::CatalogEvolutionOp {
            op: Some(pb::catalog_evolution_op::Op::AppendClass(
                pb::AppendClassOp {
                    cls: Some(sample_proto_class()),
                },
            )),
        };
        let t = proto_op_to_trait(p).unwrap();
        match t {
            EvolveOp::AppendClass { class } => assert_eq!(class.name, "billing"),
            _ => panic!("expected AppendClass"),
        }
    }

    #[test]
    fn proto_op_remove_class_round_trips() {
        let p = pb::CatalogEvolutionOp {
            op: Some(pb::catalog_evolution_op::Op::RemoveClass(
                pb::RemoveClassOp {
                    class_name: "spam".to_owned(),
                },
            )),
        };
        let t = proto_op_to_trait(p).unwrap();
        match t {
            EvolveOp::RemoveClass { class_name } => assert_eq!(class_name, "spam"),
            _ => panic!("expected RemoveClass"),
        }
    }

    #[test]
    fn proto_op_remove_class_rejects_empty_name() {
        let p = pb::CatalogEvolutionOp {
            op: Some(pb::catalog_evolution_op::Op::RemoveClass(
                pb::RemoveClassOp {
                    class_name: " ".to_owned(),
                },
            )),
        };
        let err = proto_op_to_trait(p).unwrap_err();
        assert!(err.message().contains("class_name"));
    }

    #[test]
    fn proto_op_update_thresholds_round_trips() {
        let p = pb::CatalogEvolutionOp {
            op: Some(pb::catalog_evolution_op::Op::UpdateThresholds(
                pb::UpdateThresholdsOp {
                    thresholds: Some(pb::ThresholdPolicy {
                        confidence_high: 0.9,
                        confidence_medium: 0.6,
                    }),
                },
            )),
        };
        let t = proto_op_to_trait(p).unwrap();
        match t {
            EvolveOp::UpdateThresholds { thresholds } => {
                assert!((thresholds.confidence_high - 0.9).abs() < 1e-6);
                assert!((thresholds.confidence_medium - 0.6).abs() < 1e-6);
            }
            _ => panic!("expected UpdateThresholds"),
        }
    }

    #[test]
    fn proto_op_replace_class_anchors_round_trips() {
        let p = pb::CatalogEvolutionOp {
            op: Some(pb::catalog_evolution_op::Op::ReplaceClassAnchors(
                pb::ReplaceClassAnchorsOp {
                    class_name: "billing".to_owned(),
                    canonical_sentences: vec!["new".to_owned()],
                },
            )),
        };
        let t = proto_op_to_trait(p).unwrap();
        match t {
            EvolveOp::ReplaceClassAnchors {
                class_name,
                canonical_sentences,
            } => {
                assert_eq!(class_name, "billing");
                assert_eq!(canonical_sentences.len(), 1);
            }
            _ => panic!("expected ReplaceClassAnchors"),
        }
    }

    #[test]
    fn proto_op_replace_class_anchors_rejects_empty_sentences() {
        let p = pb::CatalogEvolutionOp {
            op: Some(pb::catalog_evolution_op::Op::ReplaceClassAnchors(
                pb::ReplaceClassAnchorsOp {
                    class_name: "billing".to_owned(),
                    canonical_sentences: vec![],
                },
            )),
        };
        let err = proto_op_to_trait(p).unwrap_err();
        assert!(err.message().contains("canonical_sentences"));
    }

    #[test]
    fn proto_op_recompile_round_trips() {
        let p = pb::CatalogEvolutionOp {
            op: Some(pb::catalog_evolution_op::Op::Recompile(pb::RecompileOp {
                new_embedder: Some(pb::EmbedderSpec {
                    family: "st".to_owned(),
                    model: "mpnet".to_owned(),
                    dim: 768,
                    normalize: "l2".to_owned(),
                }),
            })),
        };
        let t = proto_op_to_trait(p).unwrap();
        match t {
            EvolveOp::Recompile { new_embedder } => {
                assert_eq!(new_embedder.dim, 768);
                assert_eq!(new_embedder.model, "mpnet");
            }
            _ => panic!("expected Recompile"),
        }
    }

    #[test]
    fn proto_op_recompile_rejects_dim_zero() {
        let p = pb::CatalogEvolutionOp {
            op: Some(pb::catalog_evolution_op::Op::Recompile(pb::RecompileOp {
                new_embedder: Some(pb::EmbedderSpec {
                    family: "st".to_owned(),
                    model: "mpnet".to_owned(),
                    dim: 0,
                    normalize: "l2".to_owned(),
                }),
            })),
        };
        let err = proto_op_to_trait(p).unwrap_err();
        assert!(err.message().contains("dim"));
    }

    #[test]
    fn proto_op_rejects_missing_oneof() {
        let p = pb::CatalogEvolutionOp { op: None };
        let err = proto_op_to_trait(p).unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    // ── canonical key parsing ──────────────────────────────────────

    #[test]
    fn parse_canonical_key_happy_path() {
        let k = parse_canonical_key("acme/billing@v3").unwrap();
        assert_eq!(k.tenant, "acme");
        assert_eq!(k.name, "billing");
        assert_eq!(k.version, 3);
    }

    #[test]
    fn parse_canonical_key_rejects_malformed() {
        let err = parse_canonical_key("not-a-key").unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    // ── metadata helpers ───────────────────────────────────────────

    #[test]
    fn build_initial_metadata_stamps_principal() {
        let m = build_initial_metadata("alice@acme", "test", &HashMap::new());
        assert_eq!(m.created_by, "alice@acme");
        assert_eq!(m.description, "test");
        assert_eq!(m.parent_key, "");
        assert_eq!(m.evolution_op, "");
        assert_eq!(m.content_sha256.0, "");
        assert!(m.created_at_unix_ms > 0);
    }

    #[test]
    fn build_evolve_metadata_falls_back_to_principal_when_override_empty() {
        let m = build_evolve_metadata("bob@acme", None);
        assert_eq!(m.created_by, "bob@acme");
    }

    #[test]
    fn build_evolve_metadata_uses_override_when_present() {
        let mut labels = std::collections::HashMap::new();
        labels.insert("env".to_owned(), "prod".to_owned());
        let override_proto = pb::CatalogMetadata {
            created_at: None,
            created_by: "alice@acme".to_owned(),
            description: "manual override".to_owned(),
            parent_key: "".to_owned(),
            evolution_op: "".to_owned(),
            labels,
            content_sha256: "".to_owned(),
        };
        let m = build_evolve_metadata("bob@acme", Some(override_proto));
        assert_eq!(m.created_by, "alice@acme");
        assert_eq!(m.description, "manual override");
        assert_eq!(m.labels.get("env"), Some(&"prod".to_owned()));
    }

    // ── CompiledClass + CanonicalCatalog round-trip ────────────────

    #[test]
    fn registered_to_handle_includes_key_and_catalog() {
        let r = RegisteredCatalog {
            key: CatalogKey {
                tenant: "acme".to_owned(),
                name: "billing".to_owned(),
                version: 1,
            },
            content: proto_content_to_trait(Some(sample_proto_content())).unwrap(),
            metadata: build_initial_metadata("alice", "init", &HashMap::new()),
            compiled: None,
        };
        let h = registered_to_handle(r);
        assert_eq!(h.key, "acme/billing@v1");
        assert!(h.catalog.is_some());
        let canonical = h.catalog.unwrap();
        assert!(canonical.metadata.is_some());
        assert_eq!(canonical.compiled.len(), 0);
    }

    #[test]
    fn registered_to_handle_includes_compiled_when_present() {
        let r = RegisteredCatalog {
            key: CatalogKey {
                tenant: "acme".to_owned(),
                name: "billing".to_owned(),
                version: 1,
            },
            content: proto_content_to_trait(Some(sample_proto_content())).unwrap(),
            metadata: build_initial_metadata("alice", "init", &HashMap::new()),
            compiled: Some(vec![CompiledClass {
                class_name: "billing".to_owned(),
                anchors_blob: vec![1, 2, 3, 4],
                num_anchors: 1,
                embedder_fingerprint: "fp".to_owned(),
            }]),
        };
        let h = registered_to_handle(r);
        let canonical = h.catalog.unwrap();
        assert_eq!(canonical.compiled.len(), 1);
        assert_eq!(canonical.compiled[0].class_name, "billing");
        assert_eq!(canonical.compiled[0].anchors_blob, vec![1, 2, 3, 4]);
    }
}
