//! Input validation helpers.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::error::AdminError;

static TENANT_SLUG: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$").expect("compile"));

static EMAIL_LOOSE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}$").expect("compile")
});

static SEMVER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[A-Za-z0-9-.]+)?$").expect("compile")
});

pub fn tenant_slug(s: &str) -> Result<&str, AdminError> {
    if TENANT_SLUG.is_match(s) {
        Ok(s)
    } else {
        Err(AdminError::invalid(format!(
            "tenant slug '{s}' must match [a-z0-9][a-z0-9-]{{1,30}}[a-z0-9]"
        )))
    }
}

pub fn email(s: &str) -> Result<&str, AdminError> {
    if EMAIL_LOOSE.is_match(s) {
        Ok(s)
    } else {
        Err(AdminError::invalid(format!("'{s}' does not look like an email")))
    }
}

pub fn semver(s: &str) -> Result<&str, AdminError> {
    if SEMVER.is_match(s) {
        Ok(s)
    } else {
        Err(AdminError::invalid(format!(
            "'{s}' is not a semver (expected vX.Y.Z)"
        )))
    }
}

pub fn duration_or_never(s: &str) -> Result<Option<chrono::Duration>, AdminError> {
    if s.eq_ignore_ascii_case("never") {
        return Ok(None);
    }
    let std_dur = humantime::parse_duration(s)
        .map_err(|e| AdminError::invalid(format!("duration '{s}': {e}")))?;
    let secs = i64::try_from(std_dur.as_secs())
        .map_err(|_| AdminError::invalid("duration too large"))?;
    Ok(Some(chrono::Duration::seconds(secs)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_valid() {
        assert!(tenant_slug("acme").is_ok());
        assert!(tenant_slug("acme-corp").is_ok());
        assert!(tenant_slug("a1b2c3").is_ok());
    }

    #[test]
    fn slug_rejects_short() {
        assert!(tenant_slug("ab").is_err());
    }

    #[test]
    fn slug_rejects_uppercase() {
        assert!(tenant_slug("Acme").is_err());
    }

    #[test]
    fn slug_rejects_leading_or_trailing_hyphen() {
        assert!(tenant_slug("-acme").is_err());
        assert!(tenant_slug("acme-").is_err());
    }

    #[test]
    fn email_simple() {
        assert!(email("jane.doe@acme.com").is_ok());
    }

    #[test]
    fn email_rejects_no_dot() {
        assert!(email("jane@acme").is_err());
    }

    #[test]
    fn semver_valid() {
        assert!(semver("v1.0.0").is_ok());
        assert!(semver("1.2.3").is_ok());
        assert!(semver("v1.2.3-rc.1").is_ok());
    }

    #[test]
    fn semver_invalid() {
        assert!(semver("1.2").is_err());
        assert!(semver("vX.Y.Z").is_err());
    }

    #[test]
    fn duration_never() {
        assert!(duration_or_never("never").unwrap().is_none());
        assert!(duration_or_never("Never").unwrap().is_none());
    }

    #[test]
    fn duration_30d() {
        let d = duration_or_never("30d").unwrap().unwrap();
        assert_eq!(d.num_days(), 30);
    }

    #[test]
    fn duration_invalid() {
        assert!(duration_or_never("forever").is_err());
    }
}
