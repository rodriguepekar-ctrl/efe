//! Environment-variable parsing for both binaries.
//!
//! Policy : every env var is read **once** at startup, parsed into a
//! strongly-typed struct, and then immutable for the life of the
//! process.  The two binaries share the helpers in this module ;
//! role-specific configs live in submodules.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use crate::error::ServerError;

pub mod controller;
pub mod participant;

/// Shared Mutex for env-var tests across submodules.
///
/// Rust tests run in parallel ; env vars are process-global.  Any
/// test that touches the env MUST hold this lock for its duration,
/// or it risks reading polluted state from a concurrent test in a
/// sibling module.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Read an env var, parse it to type `T`, or return the default if
/// the var is unset.
pub(crate) fn env_or<T: FromStr>(name: &str, default: T) -> Result<T, ServerError>
where
    T::Err: std::fmt::Display,
{
    match std::env::var(name) {
        Ok(s) => s
            .parse()
            .map_err(|e| ServerError::Config(format!("env var {name} = {s:?} : {e}"))),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(e) => Err(ServerError::Config(format!("env var {name} : {e}"))),
    }
}

/// Read an env var as a `SocketAddr`, with a default string.
pub(crate) fn env_socket_addr(name: &str, default: &str) -> Result<SocketAddr, ServerError> {
    let raw = std::env::var(name).unwrap_or_else(|_| default.to_string());
    raw.parse()
        .map_err(|e| ServerError::Config(format!("env var {name} = {raw:?} : {e}")))
}

/// Read an env var as a `Duration` in seconds, with a default.
pub(crate) fn env_duration_s(name: &str, default_s: u64) -> Result<Duration, ServerError> {
    let secs: u64 = env_or(name, default_s)?;
    Ok(Duration::from_secs(secs))
}

/// Read an env var as `Option<PathBuf>`.  Returns `None` if unset.
pub(crate) fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

/// Read an env var as `bool`.  Accepts the usual strings :
/// `true` / `false` / `1` / `0` / `yes` / `no`.  Case-insensitive.
pub(crate) fn env_bool(name: &str, default: bool) -> Result<bool, ServerError> {
    match std::env::var(name).ok().as_deref() {
        None => Ok(default),
        Some(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(true),
            "false" | "0" | "no" => Ok(false),
            other => Err(ServerError::Config(format!(
                "env var {name} = {other:?} : expected bool",
            ))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Use a unique env var name per test to avoid global-state
    /// contamination across parallel tests.

    #[test]
    fn env_or_returns_default_when_unset() {
        std::env::remove_var("EFE_TEST_ENV_OR_DEFAULT");
        let v: u64 = env_or("EFE_TEST_ENV_OR_DEFAULT", 42).unwrap();
        assert_eq!(v, 42);
    }

    #[test]
    fn env_or_parses_when_set() {
        std::env::set_var("EFE_TEST_ENV_OR_SET", "100");
        let v: u64 = env_or("EFE_TEST_ENV_OR_SET", 0).unwrap();
        assert_eq!(v, 100);
        std::env::remove_var("EFE_TEST_ENV_OR_SET");
    }

    #[test]
    fn env_or_errors_on_bad_value() {
        std::env::set_var("EFE_TEST_ENV_OR_BAD", "not-a-number");
        let r: Result<u64, _> = env_or("EFE_TEST_ENV_OR_BAD", 0);
        assert!(matches!(r, Err(ServerError::Config(_))));
        std::env::remove_var("EFE_TEST_ENV_OR_BAD");
    }

    #[test]
    fn env_bool_yes_true_1_all_true() {
        for v in ["yes", "true", "1", "TRUE", "Yes"] {
            std::env::set_var("EFE_TEST_ENV_BOOL", v);
            assert!(env_bool("EFE_TEST_ENV_BOOL", false).unwrap());
        }
        std::env::remove_var("EFE_TEST_ENV_BOOL");
    }

    #[test]
    fn env_bool_rejects_garbage() {
        std::env::set_var("EFE_TEST_ENV_BOOL_BAD", "maybe");
        assert!(env_bool("EFE_TEST_ENV_BOOL_BAD", false).is_err());
        std::env::remove_var("EFE_TEST_ENV_BOOL_BAD");
    }
}
