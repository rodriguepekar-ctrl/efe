//! PR #24 § 12 — cosign keyless OIDC wrapper.
//!
//! Phase-2 shells out to `cosign sign-blob` / `cosign verify-blob`.
//! The CI workflow `bench-publish-quarterly.yml` provides ambient
//! OIDC credentials.  Local dev runs use `--cosign-disabled`.

use std::path::Path;
use std::process::Command;

use tracing::{info, warn};

use crate::error::{BenchError, BenchResult};

/// Sign a PDF with `cosign sign-blob` (modern cosign 2.x).  Writes the
/// signature bundle to `<pdf>.bundle`.
pub fn sign(pdf: &Path, identity: Option<&str>) -> BenchResult<()> {
    let bundle = pdf.with_extension("pdf.bundle");
    let mut cmd = Command::new("cosign");
    cmd.arg("sign-blob")
        .arg("--yes")
        .arg("--bundle")
        .arg(&bundle);
    if let Some(iss) = identity {
        cmd.arg("--oidc-issuer").arg(iss);
    }
    cmd.arg(pdf);
    let out = cmd
        .output()
        .map_err(|e| BenchError::Internal(format!("cosign not on PATH : {e}")))?;
    if !out.status.success() {
        return Err(BenchError::Internal(format!(
            "cosign sign-blob failed : {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    info!(bundle = %bundle.display(), "cosign sign-blob ok");
    Ok(())
}

/// Verify a PDF using cosign keyless OIDC (modern cosign 2.x).  Uses
/// the **shared** identity regex with PR #22 (§ 12.1) so a single
/// verifier command works for both chains.
pub fn verify(pdf: &Path) -> BenchResult<()> {
    let pdf_bytes = std::fs::read(pdf).map_err(|source| BenchError::Result {
        path: pdf.to_path_buf(),
        source,
    })?;
    let json_path = pdf.with_extension("json");
    let json_bytes = std::fs::read(&json_path).map_err(|source| BenchError::Result {
        path: json_path.clone(),
        source,
    })?;

    // 1. Embedded-SHA check.
    let expected = hex::encode(sha2::Sha256::digest(&json_bytes));
    let embedded =
        crate::publish::quarterly::pdf::extract_embedded_json_sha256(&pdf_bytes).ok_or_else(|| {
            BenchError::Internal("PDF missing /EFE_BENCH_EVIDENCE_JSON_SHA256".into())
        })?;
    if expected != embedded {
        return Err(BenchError::Internal(format!(
            "embedded SHA mismatch : pdf={embedded}, json={expected}"
        )));
    }
    info!(sha = %expected, "embedded JSON SHA matches");

    // 2. Cosign signature verification.
    //
    // **D-M fix** — honest verification : when a `.bundle` exists, the
    // operator clearly INTENDED to publish a signed artefact ; failing
    // to invoke cosign must NOT be silently downgraded to "ok".  Three
    // cases :
    //
    //   (a) no `.bundle`     → SHA-only verification, warn + Ok.
    //   (b) `.bundle` exists, cosign on PATH, sig valid    → Ok.
    //   (c) `.bundle` exists, cosign on PATH, sig invalid  → ERROR.
    //   (d) `.bundle` exists, cosign NOT on PATH           → ERROR.
    //
    // Previously case (d) returned Ok with a `warn!` — meaning an
    // operator running `evidence verify` on a system without cosign
    // would see "verified" despite the signature never being checked.
    // That is a security regression : auditors would be deceived.
    let bundle = pdf.with_extension("pdf.bundle");
    if !bundle.exists() {
        warn!(bundle = %bundle.display(), "no .pdf.bundle present — SHA verified, signature skipped (unsigned artefact)");
        return Ok(());
    }
    let out = Command::new("cosign")
        .arg("verify-blob")
        .arg("--bundle")
        .arg(&bundle)
        .arg("--certificate-identity-regexp")
        .arg(super::SHARED_COSIGN_IDENTITY_REGEXP)
        .arg("--certificate-oidc-issuer")
        .arg(super::COSIGN_OIDC_ISSUER)
        .arg(pdf)
        .output();
    match out {
        Ok(o) if o.status.success() => info!("cosign verify-blob ok"),
        Ok(o) => {
            return Err(BenchError::Internal(format!(
                "cosign verify-blob failed : {}",
                String::from_utf8_lossy(&o.stderr)
            )));
        }
        Err(e) => {
            return Err(BenchError::Internal(format!(
                "cosign required to verify .bundle ({}) but not available : {e} — install cosign or remove the bundle to fall back to SHA-only",
                bundle.display()
            )));
        }
    }
    Ok(())
}

use sha2::Digest;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_rejects_missing_pdf() {
        let e = verify(Path::new("/no/such.pdf")).unwrap_err();
        match e {
            BenchError::Result { .. } => {}
            other => panic!("expected Result, got {other:?}"),
        }
    }

    #[test]
    fn verify_rejects_missing_json_sidecar() {
        let tmp = tempfile::tempdir().unwrap();
        let pdf = tmp.path().join("Q2-2026.pdf");
        let pdf_bytes = crate::publish::quarterly::pdf::build_quarterly_pdf("Q2", "body", b"{}").unwrap();
        std::fs::write(&pdf, pdf_bytes).unwrap();
        let e = verify(&pdf).unwrap_err();
        match e {
            BenchError::Result { .. } => {}
            other => panic!("expected Result, got {other:?}"),
        }
    }

    #[test]
    fn verify_accepts_matching_sha_no_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let pdf = tmp.path().join("Q2-2026.pdf");
        let json = tmp.path().join("Q2-2026.json");
        let json_bytes = b"hello bench";
        std::fs::write(&json, json_bytes).unwrap();
        let pdf_bytes = crate::publish::quarterly::pdf::build_quarterly_pdf("Q2", "body", json_bytes).unwrap();
        std::fs::write(&pdf, pdf_bytes).unwrap();
        verify(&pdf).unwrap(); // no bundle file → still passes (warns)
    }

    #[test]
    fn verify_detects_tampered_sidecar() {
        let tmp = tempfile::tempdir().unwrap();
        let pdf = tmp.path().join("Q2-2026.pdf");
        let json = tmp.path().join("Q2-2026.json");
        let pdf_bytes = crate::publish::quarterly::pdf::build_quarterly_pdf("Q2", "body", b"original").unwrap();
        std::fs::write(&pdf, pdf_bytes).unwrap();
        std::fs::write(&json, b"tampered").unwrap();
        let e = verify(&pdf).unwrap_err();
        match e {
            BenchError::Internal(s) => assert!(s.contains("SHA mismatch")),
            other => panic!("expected SHA mismatch, got {other:?}"),
        }
    }
}
