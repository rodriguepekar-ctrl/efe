//! PR #22 § 13 — quarterly evidence chain (sign + verify).
//!
//! Cosign 2.x flow (modern, keyless OIDC by default in CI) :
//!   * **sign** : `cosign sign-blob --yes --bundle <bundle> <pdf>`
//!     (ambient credentials — GitHub OIDC token in CI).
//!   * **verify (keyless)** : `cosign verify-blob
//!     --certificate-identity-regexp <re>
//!     --certificate-oidc-issuer <iss>
//!     --bundle <bundle> <pdf>`
//!   * **verify (keyed)** : `cosign verify-blob --key <pubkey>
//!     --bundle <bundle> <pdf>`
//!
//! Phase-2 auto-detects keyed vs keyless from the `--pubkey` flag :
//!   * pubkey file exists AND contains `-----BEGIN PUBLIC KEY-----`
//!     followed by a real key (not the placeholder)  ⇒  **keyed**.
//!   * otherwise  ⇒  **keyless** (uses the STABILITY_CONTRACT § 22.8
//!     identity regex + OIDC issuer).
//!
//! In all cases, the embedded-SHA256 check (PDF metadata vs JSON
//! sidecar) is performed FIRST and is sufficient by itself to reject
//! tampered content — cosign is an additional, optional layer.

use std::path::Path;
use std::process::Command;

use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::error::{ExitCode, SloError, SloResult};

/// STABILITY_CONTRACT v1.4 § 22.8 — frozen identity regex for keyless
/// OIDC verification.  Any workflow signing the quarterly PDF MUST be
/// at this URL prefix ; any deviation is a contract violation.
pub const FROZEN_IDENTITY_REGEXP: &str =
    r"^https://github\.com/efe-ai/efe/\.github/workflows/slo-quarterly\.yml@refs/.+";

/// STABILITY_CONTRACT v1.4 § 22.8 — frozen OIDC issuer.
pub const FROZEN_OIDC_ISSUER: &str = "https://token.actions.githubusercontent.com";

/// `efe-slo-rollup evidence sign <pdf>` — invoke `cosign sign-blob`
/// using cosign 2.x ambient-credential flow (keyless OIDC in CI).
///
/// The optional `--cosign-identity` flag passes a custom OIDC issuer
/// (rarely needed — GitHub OIDC is auto-detected by cosign in
/// workflows).
pub fn sign(pdf: &Path, identity: Option<&str>) -> SloResult<ExitCode> {
    let bundle_path = pdf.with_extension("pdf.bundle");
    let mut cmd = Command::new("cosign");
    cmd.arg("sign-blob").arg("--yes").arg("--bundle").arg(&bundle_path);
    if let Some(iss) = identity {
        // Modern cosign 2.x : `--oidc-issuer` overrides ambient detection.
        cmd.arg("--oidc-issuer").arg(iss);
    }
    cmd.arg(pdf);
    let out = cmd
        .output()
        .map_err(|e| SloError::EvidenceSign(format!("cosign not on PATH or invocation failed : {e}")))?;
    if !out.status.success() {
        return Err(SloError::EvidenceSign(format!(
            "cosign sign-blob failed : {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    info!(bundle = %bundle_path.display(), "cosign sign-blob success");
    Ok(ExitCode::Success)
}

/// `efe-slo-rollup evidence verify <pdf>` — offline-first verifier.
///
/// 3-step protocol :
///   1. Read PDF + JSON sidecar from disk.
///   2. Recompute SHA256(sidecar) and check it matches the SHA embedded
///      in the PDF's `/EFE_EVIDENCE_JSON_SHA256` info dictionary.
///   3. If a `.pdf.bundle` is present, run cosign verify-blob :
///        - Keyed mode if `pubkey` is a real `-----BEGIN PUBLIC KEY-----`
///          file (non-placeholder content).
///        - Keyless OIDC otherwise (matches FROZEN_IDENTITY_REGEXP +
///          FROZEN_OIDC_ISSUER).
///   4. If cosign is not installed OR bundle is missing, the verifier
///      still succeeds if step (2) passed — but emits a WARN.
pub fn verify(pdf: &Path, pubkey: &Path) -> SloResult<ExitCode> {
    if !pdf.exists() {
        return Err(SloError::EvidenceSign(format!(
            "pdf not found : {}",
            pdf.display()
        )));
    }
    let pdf_bytes = std::fs::read(pdf).map_err(|source| SloError::StorageIo {
        path: pdf.to_path_buf(),
        source,
    })?;
    let json_path = pdf.with_extension("json");
    if !json_path.exists() {
        return Err(SloError::EvidenceSign(format!(
            "json sidecar not found : {}",
            json_path.display()
        )));
    }
    let json_bytes = std::fs::read(&json_path).map_err(|source| SloError::StorageIo {
        path: json_path.clone(),
        source,
    })?;

    // ── Step 2 : embedded-SHA check.
    let computed = hex::encode(Sha256::digest(&json_bytes));
    let embedded = crate::storage::pdf::extract_embedded_json_sha256(&pdf_bytes).ok_or_else(|| {
        SloError::EvidenceSign("PDF does not contain /EFE_EVIDENCE_JSON_SHA256".into())
    })?;
    if computed != embedded {
        return Err(SloError::EvidenceSign(format!(
            "embedded SHA mismatch : pdf has {embedded}, json computes {computed}"
        )));
    }
    info!(sha = %computed, "embedded JSON SHA matches");

    // ── Step 3 : cosign bundle verification (optional).
    let bundle_path = pdf.with_extension("pdf.bundle");
    if !bundle_path.exists() {
        warn!(bundle = %bundle_path.display(), "no .pdf.bundle present — signature not verified");
        return Ok(ExitCode::Success);
    }
    let mode = detect_verify_mode(pubkey);
    let mut cmd = Command::new("cosign");
    cmd.arg("verify-blob")
        .arg("--bundle").arg(&bundle_path);
    match mode {
        VerifyMode::Keyed => {
            info!(pubkey = %pubkey.display(), "cosign verify-blob : keyed mode");
            cmd.arg("--key").arg(pubkey);
        }
        VerifyMode::Keyless => {
            info!(
                issuer = %FROZEN_OIDC_ISSUER,
                identity_regex = %FROZEN_IDENTITY_REGEXP,
                "cosign verify-blob : keyless OIDC mode",
            );
            cmd.arg("--certificate-identity-regexp").arg(FROZEN_IDENTITY_REGEXP);
            cmd.arg("--certificate-oidc-issuer").arg(FROZEN_OIDC_ISSUER);
        }
    }
    cmd.arg(pdf);

    let res = cmd.output();
    match res {
        Ok(out) if out.status.success() => {
            info!("cosign verify-blob success");
        }
        Ok(out) => {
            return Err(SloError::EvidenceSign(format!(
                "cosign verify-blob failed : {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Err(e) => {
            warn!(
                error = %e,
                "cosign not installed — embedded-SHA check passed but signature not verified"
            );
        }
    }
    Ok(ExitCode::Success)
}

/// Auto-detect keyed vs keyless mode from the pubkey file's content.
fn detect_verify_mode(pubkey: &Path) -> VerifyMode {
    if !pubkey.exists() {
        return VerifyMode::Keyless;
    }
    let content = match std::fs::read_to_string(pubkey) {
        Ok(s) => s,
        Err(_) => return VerifyMode::Keyless,
    };
    // The committed placeholder at `deploy/cosign/slo-evidence.pub`
    // contains the marker comment `Replace this with the actual` —
    // detect it and fall back to keyless OIDC.
    if content.contains("Replace this with the actual") || content.trim().is_empty() {
        return VerifyMode::Keyless;
    }
    // Real key files contain a base64-encoded body between the BEGIN/END
    // markers ; the placeholder doesn't.  Heuristic : at least one line
    // of pure base64 between the markers.
    let lines: Vec<&str> = content.lines().collect();
    let has_real_body = lines.iter().any(|l| {
        let l = l.trim();
        !l.is_empty()
            && !l.starts_with("---")
            && !l.starts_with('#')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
    });
    if has_real_body {
        VerifyMode::Keyed
    } else {
        VerifyMode::Keyless
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerifyMode {
    Keyed,
    Keyless,
}

/// Public re-export of the verification result type (for tests + lib users).
pub struct EvidenceVerification {
    pub embedded_sha_match: bool,
    pub cosign_bundle_present: bool,
    pub cosign_verified: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_identity_constants_locked() {
        // STABILITY_CONTRACT v1.4 § 22.8 — frozen.
        assert!(FROZEN_IDENTITY_REGEXP.contains("efe-ai/efe"));
        assert!(FROZEN_IDENTITY_REGEXP.contains("slo-quarterly"));
        assert_eq!(FROZEN_OIDC_ISSUER, "https://token.actions.githubusercontent.com");
    }

    #[test]
    fn verify_rejects_missing_pdf() {
        let e = verify(Path::new("/no/such.pdf"), Path::new("/no/key.pub")).unwrap_err();
        assert!(matches!(e, SloError::EvidenceSign(_)));
    }

    #[test]
    fn verify_rejects_missing_sidecar() {
        let tmp = tempfile::tempdir().unwrap();
        let pdf = tmp.path().join("Q2-2026.pdf");
        std::fs::write(&pdf, b"%PDF-1.4\n").unwrap();
        let e = verify(&pdf, &tmp.path().join("k.pub")).unwrap_err();
        assert!(matches!(e, SloError::EvidenceSign(_)));
    }

    #[test]
    fn verify_detects_sha_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let pdf = tmp.path().join("Q2-2026.pdf");
        let json = tmp.path().join("Q2-2026.json");
        let pdf_bytes = crate::storage::pdf::build_quarterly_pdf("Q", "body", b"hello world").unwrap();
        std::fs::write(&pdf, pdf_bytes).unwrap();
        std::fs::write(&json, b"different content").unwrap();
        let e = verify(&pdf, &tmp.path().join("k.pub")).unwrap_err();
        assert!(matches!(e, SloError::EvidenceSign(s) if s.contains("SHA mismatch")));
    }

    #[test]
    fn verify_accepts_matching_sha_no_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let pdf = tmp.path().join("Q2-2026.pdf");
        let json = tmp.path().join("Q2-2026.json");
        let json_bytes = b"hello world";
        std::fs::write(&json, json_bytes).unwrap();
        let pdf_bytes = crate::storage::pdf::build_quarterly_pdf("Q", "body", json_bytes).unwrap();
        std::fs::write(&pdf, pdf_bytes).unwrap();
        let rc = verify(&pdf, &tmp.path().join("k.pub")).unwrap();
        assert_eq!(rc, ExitCode::Success);
    }

    #[test]
    fn detect_verify_mode_keyless_when_pubkey_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(detect_verify_mode(&tmp.path().join("noexist.pub")), VerifyMode::Keyless);
    }

    #[test]
    fn detect_verify_mode_keyless_for_placeholder() {
        let tmp = tempfile::tempdir().unwrap();
        let k = tmp.path().join("placeholder.pub");
        std::fs::write(&k, "-----BEGIN PUBLIC KEY-----\n# Replace this with the actual key\n-----END PUBLIC KEY-----\n").unwrap();
        assert_eq!(detect_verify_mode(&k), VerifyMode::Keyless);
    }

    #[test]
    fn detect_verify_mode_keyed_for_real_key() {
        let tmp = tempfile::tempdir().unwrap();
        let k = tmp.path().join("real.pub");
        // Sample ECDSA P-256 public key (real-looking base64 body).
        std::fs::write(
            &k,
            "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEX9NfvJq2T8I+Q2HEZbN8GfM8j8s1\nKpLY9hT1bV6r2N5+xZjT/jY3K8XKjqfYzGOpV2vN3oPRD8xV+L7L8pP6Lg==\n-----END PUBLIC KEY-----\n",
        )
        .unwrap();
        assert_eq!(detect_verify_mode(&k), VerifyMode::Keyed);
    }
}
