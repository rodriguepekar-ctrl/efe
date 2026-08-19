//! Field-level PII filter consumed by [`crate::logs::JsonLayer`] before
//! serialisation.

use std::collections::HashSet;

use serde_json::Value;

use crate::redaction::{is_blocked, REDACTED_MARKER};

/// Applied to each field name / value pair during log serialisation.
#[derive(Clone, Debug, Default)]
pub struct FieldFilter {
    /// When `true`, blocked fields are emitted verbatim.
    pub allow_prompts: bool,
    /// Additional field names to always redact. Useful for product-
    /// specific secrets that are not in the default blocklist.
    pub extra_blocked: HashSet<String>,
}

impl FieldFilter {
    /// Construct a filter from `log_prompts` toggle and custom blocklist.
    #[must_use]
    pub fn new(allow_prompts: bool) -> Self {
        Self {
            allow_prompts,
            extra_blocked: HashSet::new(),
        }
    }

    /// Register an extra field name to redact (e.g. a tenant-specific
    /// header).
    pub fn block(&mut self, field: impl Into<String>) {
        self.extra_blocked.insert(field.into());
    }

    /// Return the value after optional redaction. A value is redacted
    /// when its field is on the default blocklist OR the extra set,
    /// unless `allow_prompts` is `true`.
    #[must_use]
    pub fn apply(&self, field: &str, value: Value) -> Value {
        if self.allow_prompts {
            return value;
        }
        if is_blocked(field) || self.extra_blocked.contains(field) {
            Value::String(REDACTED_MARKER.to_string())
        } else {
            value
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_filter_redacts_blocked_fields() {
        let f = FieldFilter::default();
        let out = f.apply("raw_prompt", Value::String("secret".into()));
        assert_eq!(out, Value::String(REDACTED_MARKER.into()));
    }

    #[test]
    fn allow_prompts_keeps_verbatim() {
        let f = FieldFilter::new(true);
        let out = f.apply("raw_prompt", Value::String("secret".into()));
        assert_eq!(out, Value::String("secret".into()));
    }

    #[test]
    fn non_blocked_field_passes_through() {
        let f = FieldFilter::default();
        let out = f.apply("swarm_id", Value::String("abc".into()));
        assert_eq!(out, Value::String("abc".into()));
    }

    #[test]
    fn extra_blocked_is_honored() {
        let mut f = FieldFilter::default();
        f.block("secret_header");
        let out = f.apply("secret_header", Value::String("v".into()));
        assert_eq!(out, Value::String(REDACTED_MARKER.into()));
    }

    #[test]
    fn authorization_is_redacted_by_default() {
        let f = FieldFilter::default();
        let out = f.apply("authorization", Value::String("Bearer xxx".into()));
        assert_eq!(out, Value::String(REDACTED_MARKER.into()));
    }
}
