//! Prompt template parser : YAML front-matter + body with variable
//! substitution.

use serde::Deserialize;
use std::collections::HashMap;
use thiserror::Error;

/// Parsed prompt template ready for substitution.
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    /// Stable identifier (e.g. `mode2_anchor_gen_v1`).
    pub name: String,
    /// Version integer (bumps on breaking change).
    pub version: u32,
    /// License declaration (typically `proprietary`).
    pub license: String,
    /// Providers this template was tested against.
    pub provider_compat: Vec<String>,
    /// Whether eval_pass against gold dataset.
    pub eval_pass: bool,
    /// Eval accuracy on the configured dataset, in [0, 1].
    pub eval_accuracy: Option<f64>,
    /// Min context tokens required.
    pub min_context_tokens: u32,
    /// Hard cap on response tokens (forwarded to provider).
    pub max_response_tokens: u32,
    /// System-prompt section (rendered with `$var` substitution).
    pub system_body: String,
    /// User-prompt section (rendered with `$var` substitution).
    pub user_body: String,
    /// Optional notes (not exposed to LLM).
    pub notes: String,
}

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("template missing YAML front-matter")]
    MissingFrontMatter,

    #[error("template malformed front-matter delimiter (expected `---`)")]
    MalformedDelimiter,

    #[error("template missing `# User` section")]
    MissingUserSection,

    #[error("invalid YAML header: {0}")]
    InvalidYaml(#[from] serde_yaml::Error),

    #[error("invalid header field: {0}")]
    InvalidField(String),
}

#[derive(Debug, Deserialize)]
struct RawHeader {
    name: String,
    version: u32,
    #[serde(default = "default_license")]
    license: String,
    #[serde(default)]
    provider_compat: Vec<String>,
    #[serde(default)]
    eval_pass: bool,
    eval_accuracy: Option<f64>,
    #[serde(default = "default_min_ctx")]
    min_context_tokens: u32,
    #[serde(default = "default_max_resp")]
    max_response_tokens: u32,
    #[serde(default)]
    notes: String,
}

fn default_license() -> String {
    "proprietary".into()
}
fn default_min_ctx() -> u32 {
    4_000
}
fn default_max_resp() -> u32 {
    1_024
}

impl PromptTemplate {
    /// Parse a Markdown template with YAML front-matter.
    pub fn parse(raw: &str) -> Result<Self, TemplateError> {
        if !raw.starts_with("---") {
            return Err(TemplateError::MissingFrontMatter);
        }

        let mut parts = raw.splitn(3, "---");
        let _ = parts.next(); // empty before first ---
        let header_yaml = parts.next().ok_or(TemplateError::MalformedDelimiter)?;
        let body = parts.next().ok_or(TemplateError::MalformedDelimiter)?;

        let header: RawHeader = serde_yaml::from_str(header_yaml)?;

        if let Some(acc) = header.eval_accuracy {
            if !(0.0..=1.0).contains(&acc) {
                return Err(TemplateError::InvalidField(format!(
                    "eval_accuracy must be in [0, 1], got {acc}"
                )));
            }
        }

        let (system_body, user_body) = split_sections(body)?;

        Ok(PromptTemplate {
            name: header.name,
            version: header.version,
            license: header.license,
            provider_compat: header.provider_compat,
            eval_pass: header.eval_pass,
            eval_accuracy: header.eval_accuracy,
            min_context_tokens: header.min_context_tokens,
            max_response_tokens: header.max_response_tokens,
            system_body,
            user_body,
            notes: header.notes,
        })
    }

    /// Render `(system_prompt, user_prompt)` substituting `$var`.
    ///
    /// Missing variables are kept as-is (`safe_substitute` semantics) so
    /// callers can spot un-bound vars at runtime instead of crashing.
    pub fn render(&self, vars: &HashMap<String, String>) -> (String, String) {
        (substitute(&self.system_body, vars), substitute(&self.user_body, vars))
    }

    /// Returns true if this template was tested against `provider_name`.
    pub fn is_compatible_with(&self, provider_name: &str) -> bool {
        self.provider_compat.iter().any(|p| p == provider_name)
    }
}

fn split_sections(body: &str) -> Result<(String, String), TemplateError> {
    let mut system = String::new();
    let mut user = String::new();
    enum Section {
        None,
        System,
        User,
    }
    let mut current = Section::None;

    for line in body.lines() {
        let lower = line.trim().to_ascii_lowercase();
        if lower.starts_with("# system") {
            current = Section::System;
            continue;
        }
        if lower.starts_with("# user") {
            current = Section::User;
            continue;
        }
        match current {
            Section::None => {}
            Section::System => {
                system.push_str(line);
                system.push('\n');
            }
            Section::User => {
                user.push_str(line);
                user.push('\n');
            }
        }
    }

    let user = user.trim().to_string();
    if user.is_empty() {
        return Err(TemplateError::MissingUserSection);
    }
    Ok((system.trim().to_string(), user))
}

/// `string.Template`-style `$var` / `${var}` substitution (safe — missing
/// vars left as-is).
pub(crate) fn substitute(input: &str, vars: &HashMap<String, String>) -> String {
    // Scan for `$identifier` or `${identifier}` and substitute.
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'$' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'{' {
                // Find closing `}`.
                if let Some(end) = bytes[i + 2..].iter().position(|&b| b == b'}') {
                    let key = std::str::from_utf8(&bytes[i + 2..i + 2 + end]).unwrap_or("");
                    if let Some(val) = vars.get(key) {
                        out.push_str(val);
                        i += 2 + end + 1;
                        continue;
                    }
                }
            } else if next.is_ascii_alphabetic() || next == b'_' {
                let mut j = i + 1;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                let key = std::str::from_utf8(&bytes[i + 1..j]).unwrap_or("");
                if let Some(val) = vars.get(key) {
                    out.push_str(val);
                    i = j;
                    continue;
                }
            }
        }
        // Default : copy byte through.
        out.push(c as char);
        i += 1;
    }
    out
}
