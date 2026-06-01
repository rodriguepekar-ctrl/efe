//! efe-prompts — Versioned prompt templates + A/B test framework (M21).
//!
//! Used by [`efe-llm-router`] to render Mode-2 anchor-generation prompts
//! and (in M22+) Mode-3 multi-turn templates. Templates are :
//!
//! 1. **Versioned** — `<name>_v<N>.md` filename convention. A bump from
//!    v1 → v2 is a breaking change in semantics ; both versions can
//!    co-exist transiently for A/B comparison.
//! 2. **Eval-attested** — YAML front-matter declares `eval_pass: true`
//!    and `eval_accuracy: <float>` measured against a frozen gold
//!    dataset.
//! 3. **Provider-compat** — `provider_compat: [anthropic, openai, ...]`
//!    declares which providers were tested. The router validates this
//!    list at provider selection time.
//! 4. **A/B testable** — the [`AbAssignmentTable`] deterministically
//!    routes a fraction of tenants to a candidate template version.
//!
//! Variable substitution uses the `$var` / `${var}` `string.Template`
//! syntax (NOT Jinja2) for two reasons :
//!   - Minimal dependency surface.
//!   - Cannot accidentally execute arbitrary code from a malformed
//!     template (Jinja2 has expression evaluation paths that have been
//!     CVE-vulnerable in the past).
//!
//! ## Pin tests : `tests/`.
#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod ab_test;
pub mod registry;
pub mod template;
pub mod version;

pub use ab_test::{AbAssignmentTable, AbCohort};
pub use registry::{PromptRegistry, RegistryError};
pub use template::{PromptTemplate, TemplateError};
pub use version::SemverPin;
