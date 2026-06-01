//! S3 URL parsing + allow-list validation.
//!
//! We accept only `s3://bucket/path/to/object[.parquet|.csv]` URLs.
//! Bucket must be in `tenant_s3_allowlist` for the submitting tenant
//! (loaded once per job submission, cached for the worker run).

use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum S3UrlError {
    #[error("URL must use `s3://` scheme")]
    BadScheme,
    #[error("URL must include a bucket name")]
    MissingBucket,
    #[error("URL must include an object key (path after bucket)")]
    MissingKey,
    #[error("bucket name '{0}' is malformed (RFC 1123 violation)")]
    InvalidBucketName(String),
    #[error("bucket '{0}' not in tenant_s3_allowlist for this tenant")]
    BucketNotAllowed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct S3Path {
    pub bucket: String,
    pub key: String,
}

/// Parse `s3://bucket/key/with/slashes.parquet` into typed components.
/// No network IO ; pure validation.
pub fn parse_s3_url(s: &str) -> Result<S3Path, S3UrlError> {
    let rest = s.strip_prefix("s3://").ok_or(S3UrlError::BadScheme)?;
    let mut split = rest.splitn(2, '/');
    let bucket = split.next().ok_or(S3UrlError::MissingBucket)?;
    if bucket.is_empty() {
        return Err(S3UrlError::MissingBucket);
    }
    if !is_valid_bucket_name(bucket) {
        return Err(S3UrlError::InvalidBucketName(bucket.to_string()));
    }
    let key = split.next().ok_or(S3UrlError::MissingKey)?;
    if key.is_empty() {
        return Err(S3UrlError::MissingKey);
    }
    Ok(S3Path {
        bucket: bucket.to_string(),
        key: key.to_string(),
    })
}

/// RFC 1123 + AWS bucket-naming rules subset :
/// - 3-63 chars, lowercase + digits + hyphens (no dots — we forbid
///   `bucket.with.dots` to dodge virtual-host SSL surprises).
fn is_valid_bucket_name(name: &str) -> bool {
    if name.len() < 3 || name.len() > 63 {
        return false;
    }
    if !name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        return false;
    }
    if !name.ends_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Check the parsed S3 path against a tenant allow-list of buckets.
pub fn check_against_allowlist(path: &S3Path, allowlist: &[String]) -> Result<(), S3UrlError> {
    if allowlist.iter().any(|b| b == &path.bucket) {
        Ok(())
    } else {
        Err(S3UrlError::BucketNotAllowed(path.bucket.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_url() {
        let p = parse_s3_url("s3://my-bucket/path/to/data.parquet").unwrap();
        assert_eq!(p.bucket, "my-bucket");
        assert_eq!(p.key, "path/to/data.parquet");
    }

    #[test]
    fn rejects_https_scheme() {
        assert_eq!(
            parse_s3_url("https://my-bucket/data.parquet"),
            Err(S3UrlError::BadScheme)
        );
    }

    #[test]
    fn rejects_missing_key() {
        assert_eq!(parse_s3_url("s3://my-bucket"), Err(S3UrlError::MissingKey));
    }

    #[test]
    fn rejects_missing_bucket() {
        assert_eq!(parse_s3_url("s3:///data"), Err(S3UrlError::MissingBucket));
    }

    #[test]
    fn rejects_dotted_bucket() {
        let r = parse_s3_url("s3://my.bucket/key");
        assert!(matches!(r, Err(S3UrlError::InvalidBucketName(_))));
    }

    #[test]
    fn rejects_uppercase_bucket() {
        let r = parse_s3_url("s3://MyBucket/key");
        assert!(matches!(r, Err(S3UrlError::InvalidBucketName(_))));
    }

    #[test]
    fn allowlist_check() {
        let p = parse_s3_url("s3://acme-data/file.parquet").unwrap();
        let allow = vec!["acme-data".to_string(), "acme-archive".to_string()];
        assert!(check_against_allowlist(&p, &allow).is_ok());

        let deny = vec!["other-bucket".to_string()];
        assert!(matches!(
            check_against_allowlist(&p, &deny),
            Err(S3UrlError::BucketNotAllowed(_))
        ));
    }
}
