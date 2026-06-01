//! PR #22 § 25 — quarterly evidence sign+verify+tamper tests.

use efe_slo_rollup::evidence;
use efe_slo_rollup::storage::pdf;

#[test]
fn verify_accepts_matching_pdf_and_sidecar() {
    let tmp = tempfile::tempdir().unwrap();
    let pdf_path = tmp.path().join("Q2-2026.pdf");
    let json_path = tmp.path().join("Q2-2026.json");
    let json_bytes = br#"{"period":"Q2-2026"}"#;
    std::fs::write(&json_path, json_bytes).unwrap();
    let pdf_bytes = pdf::build_quarterly_pdf("Q2 evidence", "body", json_bytes).unwrap();
    std::fs::write(&pdf_path, pdf_bytes).unwrap();

    // Verify : no .pdf.bundle, but embedded-SHA check must pass.
    let rc = evidence::verify(&pdf_path, &tmp.path().join("noexist.pub")).unwrap();
    assert_eq!(rc, efe_slo_rollup::ExitCode::Success);
}

#[test]
fn verify_rejects_tampered_sidecar() {
    let tmp = tempfile::tempdir().unwrap();
    let pdf_path = tmp.path().join("Q2-2026.pdf");
    let json_path = tmp.path().join("Q2-2026.json");
    let original = br#"{"period":"Q2-2026","slos":{}}"#;
    std::fs::write(&json_path, original).unwrap();
    let pdf_bytes = pdf::build_quarterly_pdf("Q2", "body", original).unwrap();
    std::fs::write(&pdf_path, pdf_bytes).unwrap();
    // Tamper : 1 byte changed.
    std::fs::write(&json_path, br#"{"period":"Q2-2027","slos":{}}"#).unwrap();
    let e = evidence::verify(&pdf_path, &tmp.path().join("k.pub")).unwrap_err();
    assert!(matches!(e, efe_slo_rollup::SloError::EvidenceSign(s) if s.contains("SHA mismatch")));
}

#[test]
fn pdf_metadata_embeds_sha256() {
    let pdf = pdf::build_quarterly_pdf("title", "body", b"hello").unwrap();
    let sha = pdf::extract_embedded_json_sha256(&pdf).unwrap();
    let expected = hex::encode(sha2::Sha256::digest(b"hello"));
    assert_eq!(sha, expected);
}

#[test]
fn pdf_has_no_javascript_t105() {
    let pdf = pdf::build_quarterly_pdf("title", "body", b"hello").unwrap();
    let text = String::from_utf8_lossy(&pdf);
    assert!(!text.contains("/JavaScript"));
    assert!(!text.contains("/AcroForm"));
}

use sha2::Digest;
