//! PR #22 § 13 — quarterly PDF generator.
//!
//! Phase-2 ships a **minimal PDF** : a single text-only document
//! that embeds the SHA256 of the JSON sidecar in its metadata.  The
//! 7-section layout (§ 13.1) is implemented via plain text formatting —
//! `printpdf` is a Phase-3 follow-up (G.8) to keep the dep footprint
//! small.
//!
//! The PDF is deliberately **flat text** (no `/JavaScript`, no
//! `/AcroForm`) — T105 mitigation.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{SloError, SloResult};

/// PDF version 1.4 magic header.
const PDF_HEADER: &str = "%PDF-1.4\n";

/// Build a flat-text PDF for the quarterly evidence.  This is a
/// deliberately-simplistic PDF producer : it writes a single page with
/// the evidence as plain text + the JSON sidecar SHA256 in the
/// `/EFE_EVIDENCE_JSON_SHA256` document-info entry (AD35).
pub fn build_quarterly_pdf(
    title: &str,
    body_text: &str,
    json_sidecar: &[u8],
) -> SloResult<Vec<u8>> {
    if title.contains('\n') {
        return Err(SloError::EvidenceSign("PDF title must be single-line".into()));
    }
    let sha = Sha256::digest(json_sidecar);
    let sha_hex = hex::encode(sha);

    // Each PDF object is on one line for byte-stability.
    let mut buf = String::new();
    buf.push_str(PDF_HEADER);
    // Document info dictionary — embeds the JSON SHA256.
    let info_obj = format!(
        "1 0 obj <<\n  /Title ({title})\n  /Producer (efe-slo-rollup v{ver})\n  /EFE_EVIDENCE_JSON_SHA256 ({sha_hex})\n  /EFE_EVIDENCE_SCHEMA_VERSION ({sv})\n>>\nendobj\n",
        ver = crate::build::VERSION,
        sv = crate::QUARTERLY_EVIDENCE_SCHEMA_VERSION,
    );
    let pages_obj = "2 0 obj <<\n  /Type /Pages\n  /Count 1\n  /Kids [3 0 R]\n>>\nendobj\n".to_string();
    let escaped_body = escape_pdf_text(body_text);
    let body = escaped_body.as_str();
    let stream = format!("BT\n/F1 10 Tf\n50 750 Td\n({title}) Tj\nT*\n({body}) Tj\nET");
    let page_obj = "3 0 obj <<\n  /Type /Page\n  /Parent 2 0 R\n  /MediaBox [0 0 612 792]\n  /Contents 4 0 R\n  /Resources << /Font << /F1 5 0 R >> >>\n>>\nendobj\n".to_string();
    let stream_obj = format!(
        "4 0 obj <<\n  /Length {len}\n>>\nstream\n{stream}\nendstream\nendobj\n",
        len = stream.len(),
    );
    let font_obj = "5 0 obj <<\n  /Type /Font\n  /Subtype /Type1\n  /BaseFont /Helvetica\n>>\nendobj\n".to_string();
    let catalog_obj = "6 0 obj <<\n  /Type /Catalog\n  /Pages 2 0 R\n>>\nendobj\n".to_string();

    buf.push_str(&info_obj);
    buf.push_str(&pages_obj);
    buf.push_str(&page_obj);
    buf.push_str(&stream_obj);
    buf.push_str(&font_obj);
    buf.push_str(&catalog_obj);

    // Simplified xref (not byte-perfect for adobe pdfa but parseable).
    buf.push_str("xref\n0 7\n");
    buf.push_str("0000000000 65535 f \n");
    for _ in 0..6 {
        buf.push_str("0000000000 00000 n \n");
    }
    buf.push_str("trailer <<\n  /Size 7\n  /Root 6 0 R\n  /Info 1 0 R\n>>\n");
    buf.push_str("startxref\n0\n%%EOF\n");

    Ok(buf.into_bytes())
}

/// Write a PDF to disk.
pub fn write_pdf(path: &Path, content: &[u8]) -> SloResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| SloError::StorageIo {
            path: path.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(path, content).map_err(|source| SloError::StorageIo {
        path: path.to_path_buf(),
        source,
    })
}

/// Extract the embedded JSON SHA256 from a previously-built PDF.
/// Used by the verifier.
#[must_use]
pub fn extract_embedded_json_sha256(pdf: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(pdf);
    let marker = "/EFE_EVIDENCE_JSON_SHA256 (";
    let start = text.find(marker)? + marker.len();
    let rest = &text[start..];
    let end = rest.find(')')?;
    Some(rest[..end].to_string())
}

fn escape_pdf_text(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
        .replace('\n', " ")
}

/// Build the canonical 7-section body text for a quarterly evidence
/// (matches PR #22 § 13.1).
#[must_use]
pub fn render_quarterly_body(
    quarter: &str,
    chart_version: &str,
    achievements: &[(String, f64, f64, &'static str)],
    methodology_url: &str,
    audit_trail: &str,
) -> String {
    let mut body = String::new();
    body.push_str("Section 1 — Cover\n");
    body.push_str(&format!("  Period       : {quarter}\n"));
    body.push_str(&format!("  Chart version: {chart_version}\n"));
    body.push_str(&format!("  Audit-trail  : {audit_trail}\n\n"));
    body.push_str("Section 2 — Methodology\n");
    body.push_str(&format!("  Reference    : {methodology_url}\n\n"));
    for (i, (slo, target, achieved, verdict)) in achievements.iter().enumerate() {
        body.push_str(&format!(
            "Section {} — SLO {}\n  target : {target:.4}\n  achieved: {achieved:.4}\n  verdict: {verdict}\n\n",
            i + 3,
            slo,
        ));
    }
    body.push_str("Section 7 — Appendix + signature\n");
    body.push_str("  Cosign signature in sidecar bundle (.pdf.bundle).\n");
    body.push_str("  Verifier : efe-slo-rollup evidence verify <pdf>\n");
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_starts_with_magic_header() {
        let p = build_quarterly_pdf("Q2-2026", "body", b"{}").unwrap();
        assert!(p.starts_with(b"%PDF-1.4"));
    }

    #[test]
    fn pdf_contains_embedded_sha() {
        let p = build_quarterly_pdf("Q2-2026", "body", b"hello world").unwrap();
        let sha_hex = hex::encode(Sha256::digest(b"hello world"));
        let pdf_text = String::from_utf8_lossy(&p);
        assert!(pdf_text.contains(&sha_hex));
    }

    #[test]
    fn extract_embedded_sha_roundtrips() {
        let p = build_quarterly_pdf("Q2-2026", "body", b"sample json sidecar").unwrap();
        let extracted = extract_embedded_json_sha256(&p).unwrap();
        let expected = hex::encode(Sha256::digest(b"sample json sidecar"));
        assert_eq!(extracted, expected);
    }

    #[test]
    fn rejects_multiline_title_t105() {
        let e = build_quarterly_pdf("multi\nline", "body", b"{}").unwrap_err();
        assert!(matches!(e, SloError::EvidenceSign(_)));
    }

    #[test]
    fn pdf_contains_no_javascript_t105() {
        let p = build_quarterly_pdf("Q2-2026", "body", b"{}").unwrap();
        let text = String::from_utf8_lossy(&p);
        // T105 mitigation : strictly no /JavaScript or /JS keywords.
        assert!(!text.contains("/JavaScript"));
        assert!(!text.contains("/JS "));
        assert!(!text.contains("/AcroForm"));
    }

    #[test]
    fn escape_pdf_text_handles_parens() {
        let s = escape_pdf_text("foo (bar) baz");
        assert_eq!(s, "foo \\(bar\\) baz");
    }

    #[test]
    fn render_body_lists_7_sections() {
        let achievements = vec![
            ("L2_AVAILABILITY".into(), 0.995, 0.999, "pass"),
            ("L2_LATENCY_P95".into(), 0.99, 0.998, "pass"),
            ("SWARM_RECOVERY_TIME".into(), 0.99, 1.0, "pass"),
            ("REPLICATION_LAG_P99".into(), 0.99, 0.999, "pass"),
        ];
        let body = render_quarterly_body(
            "Q2-2026", "0.6.0", &achievements,
            "https://efe.github.io/slos/methodology.html",
            "rekor:abc123",
        );
        for s in ["Section 1", "Section 2", "Section 3", "Section 7"] {
            assert!(body.contains(s));
        }
    }
}
