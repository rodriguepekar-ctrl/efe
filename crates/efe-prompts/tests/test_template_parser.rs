//! Pin tests for PromptTemplate parser (M21 Phase A).

use efe_prompts::{PromptTemplate, TemplateError};
use std::collections::HashMap;

const VALID: &str = r#"---
name: test_v1
version: 1
license: proprietary
provider_compat: [anthropic, openai]
min_context_tokens: 4000
max_response_tokens: 200
eval_pass: true
eval_accuracy: 0.94
notes: |
  Test template
---

# System
You are an expert in $catalog classification.

# User
Sentence : "$sentence"
Candidates : $candidates_json
"#;

#[test]
fn test_parse_valid_template() {
    let t = PromptTemplate::parse(VALID).expect("parse must succeed");
    assert_eq!(t.name, "test_v1");
    assert_eq!(t.version, 1);
    assert_eq!(t.license, "proprietary");
    assert_eq!(t.provider_compat, vec!["anthropic", "openai"]);
    assert!(t.eval_pass);
    assert_eq!(t.eval_accuracy, Some(0.94));
    assert_eq!(t.min_context_tokens, 4000);
    assert_eq!(t.max_response_tokens, 200);
    assert!(t.system_body.contains("$catalog"));
    assert!(t.user_body.contains("$sentence"));
}

#[test]
fn test_parse_missing_front_matter() {
    let raw = "# System\nfoo\n# User\nbar";
    let err = PromptTemplate::parse(raw).unwrap_err();
    assert!(matches!(err, TemplateError::MissingFrontMatter));
}

#[test]
fn test_parse_missing_user_section() {
    let raw = r#"---
name: test_v1
version: 1
---

# System
only system here
"#;
    let err = PromptTemplate::parse(raw).unwrap_err();
    assert!(matches!(err, TemplateError::MissingUserSection));
}

#[test]
fn test_parse_invalid_eval_accuracy() {
    let raw = r#"---
name: test_v1
version: 1
eval_accuracy: 1.5
---

# System
x
# User
y
"#;
    let err = PromptTemplate::parse(raw).unwrap_err();
    assert!(matches!(err, TemplateError::InvalidField(_)));
}

#[test]
fn test_render_substitutes_simple_var() {
    let t = PromptTemplate::parse(VALID).unwrap();
    let mut vars = HashMap::new();
    vars.insert("catalog".to_string(), "support".to_string());
    vars.insert("sentence".to_string(), "hello".to_string());
    vars.insert("candidates_json".to_string(), "[a,b]".to_string());

    let (sys, usr) = t.render(&vars);
    assert!(sys.contains("expert in support"));
    assert!(usr.contains(r#""hello""#));
    assert!(usr.contains("[a,b]"));
}

#[test]
fn test_render_safe_substitute_missing_var_kept() {
    let t = PromptTemplate::parse(VALID).unwrap();
    let mut vars = HashMap::new();
    vars.insert("catalog".to_string(), "support".to_string());
    // sentence + candidates_json missing on purpose
    let (_sys, usr) = t.render(&vars);
    assert!(usr.contains("$sentence")); // kept as literal
    assert!(usr.contains("$candidates_json"));
}

#[test]
fn test_render_braced_var_substituted() {
    let raw = r#"---
name: test
version: 1
---

# System
sys

# User
hello ${name}, you have ${count} messages
"#;
    let t = PromptTemplate::parse(raw).unwrap();
    let mut vars = HashMap::new();
    vars.insert("name".to_string(), "Alice".to_string());
    vars.insert("count".to_string(), "5".to_string());
    let (_, usr) = t.render(&vars);
    assert_eq!(usr, "hello Alice, you have 5 messages");
}

#[test]
fn test_is_compatible_with_provider() {
    let t = PromptTemplate::parse(VALID).unwrap();
    assert!(t.is_compatible_with("anthropic"));
    assert!(t.is_compatible_with("openai"));
    assert!(!t.is_compatible_with("nonexistent"));
}

#[test]
fn test_default_min_max_tokens() {
    let raw = r#"---
name: t
version: 1
---

# System
s
# User
u
"#;
    let t = PromptTemplate::parse(raw).unwrap();
    assert_eq!(t.min_context_tokens, 4_000);
    assert_eq!(t.max_response_tokens, 1_024);
}
