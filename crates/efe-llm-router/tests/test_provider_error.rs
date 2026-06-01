//! Pin tests for ProviderError mapping + retry semantics.

use efe_llm_router::provider::{ProviderError, FinishReason};

#[test]
fn test_timeout_is_retryable() {
    let err = ProviderError::Timeout("anthropic:claude".into(), 5_000);
    assert!(err.is_retryable());
}

#[test]
fn test_rate_limited_is_retryable() {
    let err = ProviderError::RateLimited("openai:gpt-5".into());
    assert!(err.is_retryable());
}

#[test]
fn test_unavailable_is_retryable() {
    let err = ProviderError::Unavailable("vertex".into(), "503".into());
    assert!(err.is_retryable());
}

#[test]
fn test_auth_error_not_retryable() {
    let err = ProviderError::AuthError("anthropic".into());
    assert!(!err.is_retryable());
}

#[test]
fn test_invalid_response_not_retryable() {
    let err = ProviderError::InvalidResponse("openai".into(), "bad json".into());
    assert!(!err.is_retryable());
}

#[test]
fn test_safety_block_not_retryable() {
    let err = ProviderError::SafetyBlock("anthropic".into());
    assert!(!err.is_retryable());
}

#[test]
fn test_finish_reason_eq() {
    assert_eq!(FinishReason::Stop, FinishReason::Stop);
    assert_ne!(FinishReason::Stop, FinishReason::Length);
}

#[test]
fn test_provider_error_display_includes_name() {
    let err = ProviderError::Timeout("anthropic".into(), 5_000);
    let s = err.to_string();
    assert!(s.contains("anthropic"));
    assert!(s.contains("5000"));
}
