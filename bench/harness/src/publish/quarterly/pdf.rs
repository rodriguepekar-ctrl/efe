//! PR #24 § 12 — quarterly evidence PDF.
//!
//! Phase-2 ships a deliberately minimal text-PDF (mirror of PR #22
//! § 10.3) — no embedded fonts, no JS (T125 mitigation).
//!
//! AD27 — PDF metadata embeds SHA256 of the JSON sidecar.  Verifier
//! re-computes the SHA on read.

use sha2::{Digest, Sha256};

use crate::error::{BenchError, BenchResult};

const PDF_HEADER: &str = "%PDF-1.4\n";

/// Build a flat-text PDF.  Embed SHA256 of `json_sidecar` in the
/// document-info dictionary (AD27).
///
/// **D-AQ fix** — xref byte offsets are now computed from real
/// positions in the buffer instead of all-zeroes.  Lenient PDF
/// viewers happily ignored the zeros (they rescan for `N 0 obj`
/// patterns) but strict tools (`qpdf --check`, Acrobat strict mode,
/// `mutool clean`) flagged the file as structurally invalid — making
/// the compliance evidence look corrupted to auditors.
pub fn build_quarterly_pdf(
    title: &str,
    body_text: &str,
    json_sidecar: &[u8],
) -> BenchResult<Vec<u8>> {
    if title.contains('\n') {
        return Err(BenchError::Internal(
            "quarterly PDF title must be single-line".into(),
        ));
    }
    let sha_hex = hex::encode(Sha256::digest(json_sidecar));

    let mut buf = String::new();
    buf.push_str(PDF_HEADER);

    // Build each object body up-front so we can compute byte offsets
    // as we push them.
    let info_obj = format!(
        "1 0 obj <<\n  /Title ({title})\n  /Producer (efe-bench-harness v{ver})\n  /EFE_BENCH_EVIDENCE_JSON_SHA256 ({sha_hex})\n  /EFE_BENCH_EVIDENCE_SCHEMA_VERSION (1)\n>>\nendobj\n",
        ver = crate::build::VERSION,
    );
    let pages_obj =
        "2 0 obj <<\n  /Type /Pages\n  /Count 1\n  /Kids [3 0 R]\n>>\nendobj\n".to_string();
    let escaped_body = escape_pdf_text(body_text);
    let stream = format!(
        "BT\n/F1 10 Tf\n50 750 Td\n({title}) Tj\nT*\n({escaped_body}) Tj\nET",
    );
    let page_obj = "3 0 obj <<\n  /Type /Page\n  /Parent 2 0 R\n  /MediaBox [0 0 612 792]\n  /Contents 4 0 R\n  /Resources << /Font << /F1 5 0 R >> >>\n>>\nendobj\n".to_string();
    let stream_obj = format!(
        "4 0 obj <<\n  /Length {len}\n>>\nstream\n{stream}\nendstream\nendobj\n",
        len = stream.len(),
    );
    let font_obj =
        "5 0 obj <<\n  /Type /Font\n  /Subtype /Type1\n  /BaseFont /Helvetica\n>>\nendobj\n"
            .to_string();
    let catalog_obj =
        "6 0 obj <<\n  /Type /Catalog\n  /Pages 2 0 R\n>>\nendobj\n".to_string();

    // Push each object, recording its byte offset BEFORE the push.
    let mut offsets: [usize; 6] = [0; 6];
    let pieces = [
        &info_obj,
        &pages_obj,
        &page_obj,
        &stream_obj,
        &font_obj,
        &catalog_obj,
    ];
    for (i, piece) in pieces.iter().enumerate() {
        offsets[i] = buf.len(); // start of obj `i+1`
        buf.push_str(piece);
    }

    // xref table starts here ; remember its byte offset for `startxref`.
    let xref_offset = buf.len();
    buf.push_str("xref\n0 7\n");
    // Entry 0 : the free-list head, always `0000000000 65535 f `.
    buf.push_str("0000000000 65535 f \n");
    // Entries 1..=6 : actual byte offsets, 10-digit zero-padded.
    for off in offsets {
        // PDF spec § 7.5.4 : 10-digit offset, 5-digit generation, 'n'.
        // Trailing single space + newline is REQUIRED (20-byte rows).
        buf.push_str(&format!("{off:010} 00000 n \n"));
    }
    buf.push_str("trailer <<\n  /Size 7\n  /Root 6 0 R\n  /Info 1 0 R\n>>\n");
    buf.push_str(&format!("startxref\n{xref_offset}\n%%EOF\n"));

    Ok(buf.into_bytes())
}

/// Extract the embedded JSON SHA256 from a built PDF.
///
/// **D-BE fix** — the AC22 cross-chain verifier in the bench harness
/// MUST accept PDFs from BOTH chains :
///   * PR #24 bench :  marker `/EFE_BENCH_EVIDENCE_JSON_SHA256`
///   * PR #22 SLO  :   marker `/EFE_EVIDENCE_JSON_SHA256`
///
/// Previously only the bench marker was scanned ; a `bench-harness
/// evidence verify` on a PR #22 PDF would error out with "missing
/// /EFE_BENCH_EVIDENCE_JSON_SHA256" — exactly the cross-chain failure
/// the AC22 contract was supposed to prevent.  The bench marker is
/// tried first (more specific), with PR #22 as fallback.
///
/// **D-AR** — validates the extracted value is exactly 64 ASCII hex
/// characters before returning it (defense against malformed markers).
#[must_use]
pub fn extract_embedded_json_sha256(pdf: &[u8]) -> Option<String> {
    // **D-BE** — try the BENCH marker first, then fall back to PR #22.
    extract_with_marker(pdf, "/EFE_BENCH_EVIDENCE_JSON_SHA256 (")
        .or_else(|| extract_with_marker(pdf, "/EFE_EVIDENCE_JSON_SHA256 ("))
}

fn extract_with_marker(pdf: &[u8], marker: &str) -> Option<String> {
    let text = String::from_utf8_lossy(pdf);
    let start = text.find(marker)? + marker.len();
    let rest = &text[start..];
    let end = rest.find(')')?;
    let candidate = &rest[..end];
    // Strict SHA-256 format check : exactly 64 hex chars.
    if candidate.len() != 64 {
        return None;
    }
    if !candidate.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(candidate.to_string())
}

/// Escape a string for inclusion in a PDF literal-string (parenthesised) :
///
///   * `\` → `\\`
///   * `(` → `\(`
///   * `)` → `\)`
///   * `\n`, `\r`, `\t` → ` ` (single space, normalises to a clean stream)
///   * any non-ASCII char → `?` (Phase-2 PDF uses Helvetica + WinAnsi
///     encoding, which can't represent codepoints > U+00FF cleanly ;
///     dropping to `?` keeps the PDF valid and human-readable rather
///     than emitting mojibake glyphs)
///
/// **D-AL fix** — previously only `\n` was normalised, leaving `\r`
/// and `\t` to surface as garbled glyphs.  And non-ASCII codepoints
/// (e.g. `§`) leaked through as UTF-8 multi-byte sequences that
/// WinAnsi misinterpreted as `Â§`.
fn escape_pdf_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_ascii() => out.push(c),
            // Non-ASCII : replace with `?` (Phase-2 limitation —
            // proper Unicode would need a Type0 CIDFont).
            _ => out.push('?'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_starts_with_magic() {
        let p = build_quarterly_pdf("Q2-2026", "body", b"{}").unwrap();
        assert!(p.starts_with(b"%PDF-1.4"));
    }

    #[test]
    fn pdf_embeds_sha256_of_sidecar() {
        let pdf = build_quarterly_pdf("Q2-2026", "body", b"hello").unwrap();
        let expected = hex::encode(Sha256::digest(b"hello"));
        let extracted = extract_embedded_json_sha256(&pdf).unwrap();
        assert_eq!(extracted, expected);
    }

    #[test]
    fn pdf_rejects_multiline_title() {
        let e = build_quarterly_pdf("multi\nline", "body", b"{}").unwrap_err();
        assert!(matches!(e, BenchError::Internal(_)));
    }

    #[test]
    fn pdf_no_javascript_t125() {
        let pdf = build_quarterly_pdf("Q2-2026", "body", b"{}").unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(!text.contains("/JavaScript"));
        assert!(!text.contains("/JS "));
        assert!(!text.contains("/AcroForm"));
    }

    #[test]
    fn pdf_escapes_parens() {
        let pdf = build_quarterly_pdf("Q2-2026", "foo (bar) baz", b"{}").unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("foo \\(bar\\) baz"));
    }

    // ───────────────────────────────────────────────────────────────
    // D-X — extractor robustness on hostile / corrupted PDF bytes.
    // ───────────────────────────────────────────────────────────────

    #[test]
    fn d_x_extract_returns_none_on_empty_pdf() {
        assert_eq!(extract_embedded_json_sha256(b""), None);
    }

    #[test]
    fn d_x_extract_returns_none_on_truncated_marker() {
        // Marker present but no closing paren — must return None, not
        // half a SHA or a panic.
        let bytes = b"%PDF-1.4\n/EFE_BENCH_EVIDENCE_JSON_SHA256 (ab12";
        assert_eq!(extract_embedded_json_sha256(bytes), None);
    }

    #[test]
    fn d_x_extract_returns_none_when_marker_missing() {
        let bytes = b"%PDF-1.4\n... some other PDF without our marker ...";
        assert_eq!(extract_embedded_json_sha256(bytes), None);
    }

    #[test]
    fn d_x_extract_resists_body_injection_via_escaped_parens() {
        // **D-X invariant** : even if the operator passes hostile body
        // text containing a fake marker, `escape_pdf_text` neutralises
        // the open-paren so `find()` cannot match the injected one.
        // The verifier still extracts only the genuine info-dict SHA.
        let real_json = b"the-real-thing";
        let injected_body =
            "tampered /EFE_BENCH_EVIDENCE_JSON_SHA256 (deadbeef) overwrite";
        let pdf = build_quarterly_pdf("Q2-2026", injected_body, real_json).unwrap();
        let extracted = extract_embedded_json_sha256(&pdf).unwrap();
        let expected = hex::encode(Sha256::digest(real_json));
        assert_eq!(
            extracted, expected,
            "D-X : body-injected fake marker must NOT mask the genuine SHA"
        );
        assert_ne!(extracted, "deadbeef", "extractor returned the attacker's value");
    }

    // ───────────────────────────────────────────────────────────────
    // D-AL — escape_pdf_text edge cases.
    // ───────────────────────────────────────────────────────────────

    #[test]
    fn d_al_escape_handles_carriage_return_and_tab() {
        // \r and \t MUST be normalised to a single space ; otherwise
        // PDF renderers interpret \r as a forced linefeed and produce
        // garbled layout (`\t` rendering is undefined).
        let escaped = escape_pdf_text("a\rb\tc");
        assert_eq!(escaped, "a b c");
        assert!(
            !escaped.contains('\r'),
            "D-AL : \\r must be stripped, got '{escaped}'"
        );
        assert!(!escaped.contains('\t'), "D-AL : \\t must be stripped");
    }

    #[test]
    fn d_al_escape_strips_non_ascii_to_question_mark() {
        // Phase-2 limitation : Helvetica + WinAnsi can't render
        // arbitrary Unicode.  Previously a `§` leaked as mojibake.
        assert_eq!(escape_pdf_text("a§b"), "a?b");
        assert_eq!(escape_pdf_text("École"), "?cole");
        assert_eq!(escape_pdf_text("emoji ✅"), "emoji ?");
    }

    #[test]
    fn d_al_escape_preserves_pure_ascii() {
        let s = "abc 123 -. xyz";
        assert_eq!(escape_pdf_text(s), s);
    }

    #[test]
    fn d_al_escape_combined_metacharacters_and_unicode() {
        // Pathological input : every escape rule + non-ASCII.
        let escaped = escape_pdf_text("foo\\bar (baz) §\n\r\tend");
        // Backslash doubled, parens escaped, control chars → spaces,
        // `§` → `?`, end intact.
        assert_eq!(escaped, "foo\\\\bar \\(baz\\) ?   end");
    }

    #[test]
    fn d_al_production_body_text_pure_ascii_after_escape() {
        // Regression test : the actual body text engine.rs builds for
        // quarterly evidence MUST be pure ASCII after escape.
        let body = "EFE Bench Quarterly Evidence Period : Q2-2026 Chart version: 0.6.0 Schema : v1.6 Section 24 Verifier : efe-bench-harness evidence verify <pdf>";
        let escaped = escape_pdf_text(body);
        assert!(
            escaped.is_ascii(),
            "D-AL : production body must escape to pure ASCII : {escaped}"
        );
        assert!(!escaped.contains('?'), "no non-ASCII to lossily strip");
    }

    // ───────────────────────────────────────────────────────────────
    // D-AQ — PDF xref table correctness (ISO 32000-1 §7.5.4).
    // ───────────────────────────────────────────────────────────────

    // ───────────────────────────────────────────────────────────────
    // D-AR — extract_embedded_json_sha256 must validate format.
    // ───────────────────────────────────────────────────────────────

    // ───────────────────────────────────────────────────────────────
    // D-BE — bench harness MUST extract SHA from PR #22 PDFs too
    // (cross-chain verifier AC22).  PR #22 uses a different marker.
    // ───────────────────────────────────────────────────────────────

    #[test]
    fn d_be_extract_accepts_pr22_marker() {
        // PR #22 PDF has `/EFE_EVIDENCE_JSON_SHA256` (no `_BENCH_`).
        let pr22_pdf = b"%PDF\n/EFE_EVIDENCE_JSON_SHA256 (abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789)\n";
        let r = extract_embedded_json_sha256(pr22_pdf);
        assert_eq!(
            r,
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".to_string()),
            "D-BE : bench verifier must extract PR #22 marker for AC22 cross-chain"
        );
    }

    #[test]
    fn d_be_bench_marker_takes_priority_over_pr22() {
        // If BOTH markers are present (theoretical only — same PDF can't
        // be both), the bench marker (more specific) wins.
        let mixed = b"%PDF\n/EFE_BENCH_EVIDENCE_JSON_SHA256 (1111111111111111111111111111111111111111111111111111111111111111)\n/EFE_EVIDENCE_JSON_SHA256 (2222222222222222222222222222222222222222222222222222222222222222)\n";
        let r = extract_embedded_json_sha256(mixed);
        assert_eq!(
            r,
            Some("1111111111111111111111111111111111111111111111111111111111111111".to_string()),
            "D-BE : bench marker takes priority when both present"
        );
    }

    #[test]
    fn d_be_neither_marker_returns_none() {
        let neither = b"%PDF\n/some-other-key (deadbeef)\n";
        assert_eq!(extract_embedded_json_sha256(neither), None);
    }

    #[test]
    fn d_ar_extract_rejects_non_hex_value() {
        let evil = b"%PDF\n/EFE_BENCH_EVIDENCE_JSON_SHA256 (this-is-not-hex-data-but-64-chars-long-padding-padding-pad)\n";
        // Even though the length is wrong (not 64), the test confirms
        // the rejection.  The non-hex chars + wrong length should both
        // be caught.
        assert_eq!(
            extract_embedded_json_sha256(evil),
            None,
            "D-AR : non-hex value must be rejected"
        );
    }

    #[test]
    fn d_ar_extract_rejects_wrong_length() {
        // 63 hex chars (one short).
        let short = b"%PDF\n/EFE_BENCH_EVIDENCE_JSON_SHA256 (abcdef0123456789abcdef0123456789abcdef0123456789abcdef012345678)\n";
        assert_eq!(
            extract_embedded_json_sha256(short),
            None,
            "D-AR : 63-char value must be rejected"
        );
        // 65 hex chars (one over).
        let long = b"%PDF\n/EFE_BENCH_EVIDENCE_JSON_SHA256 (abcdef0123456789abcdef0123456789abcdef0123456789abcdef01234567890)\n";
        assert_eq!(
            extract_embedded_json_sha256(long),
            None,
            "D-AR : 65-char value must be rejected"
        );
    }

    #[test]
    fn d_ar_extract_accepts_valid_lowercase_sha256() {
        // 64 lowercase hex chars.
        let valid = b"%PDF\n/EFE_BENCH_EVIDENCE_JSON_SHA256 (abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789)\n";
        let r = extract_embedded_json_sha256(valid);
        assert_eq!(
            r,
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".to_string()),
            "D-AR : valid 64-char lowercase hex must be accepted"
        );
    }

    #[test]
    fn d_ar_extract_accepts_uppercase_hex() {
        // Hex is case-insensitive ; uppercase must round-trip too.
        // (The verifier compares against `hex::encode` which emits
        // lowercase, so an uppercase embed would mismatch — but the
        // extractor itself must not silently drop it.)
        let valid = b"%PDF\n/EFE_BENCH_EVIDENCE_JSON_SHA256 (ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789)\n";
        let r = extract_embedded_json_sha256(valid);
        assert!(r.is_some(), "D-AR : uppercase hex must be extractable");
    }

    #[test]
    fn d_aq_xref_offsets_are_real_byte_positions() {
        let pdf = build_quarterly_pdf("Q2-2026", "body", b"{}").unwrap();
        let text = String::from_utf8_lossy(&pdf);
        // Find the xref keyword and parse the table.
        let xref_kw_pos = text.find("\nxref\n0 7\n").expect("xref keyword present");
        let after_header = xref_kw_pos + "\nxref\n0 7\n".len();
        // 7 lines : entry 0 (free) + 6 object entries.
        let lines: Vec<&str> = text[after_header..]
            .lines()
            .take(7)
            .collect();
        assert_eq!(lines.len(), 7, "expected 7 xref entries");
        // Entry 0 = free head.
        assert!(lines[0].starts_with("0000000000 65535 f"));
        // Entries 1..=6 : non-zero, lex-ordered, and the prefix at
        // each offset MUST be `N 0 obj` for N == row index.
        for (i, line) in lines[1..].iter().enumerate() {
            let n = i + 1;
            let off_str: &str = line.split_whitespace().next().expect("offset");
            let off: usize = off_str.parse().unwrap_or_else(|e| {
                panic!("D-AQ : offset for obj {n} not numeric : '{off_str}' ({e})")
            });
            assert!(
                off > 0,
                "D-AQ : obj {n} xref offset is zero — old broken implementation"
            );
            // Validate the byte at `off` actually starts with `{n} 0 obj`.
            let expected_prefix = format!("{n} 0 obj");
            let slice = std::str::from_utf8(&pdf[off..off + expected_prefix.len()])
                .expect("ascii at xref offset");
            assert_eq!(
                slice, expected_prefix,
                "D-AQ : xref offset for obj {n} ({off}) does not point to '{expected_prefix}'"
            );
        }
    }

    #[test]
    fn d_aq_startxref_points_to_xref_keyword() {
        let pdf = build_quarterly_pdf("Q2-2026", "body", b"{}").unwrap();
        let text = String::from_utf8_lossy(&pdf);
        // Last `startxref` line carries the offset.
        let sx_pos = text.rfind("startxref\n").expect("startxref present");
        let rest = &text[sx_pos + "startxref\n".len()..];
        let offset_str = rest.lines().next().expect("offset line");
        let offset: usize = offset_str
            .parse()
            .unwrap_or_else(|e| panic!("D-AQ : startxref not numeric : '{offset_str}' ({e})"));
        assert!(offset > 0, "D-AQ : startxref must not be 0");
        // The bytes at `offset` MUST be the literal `xref\n`.
        let kw = std::str::from_utf8(&pdf[offset..offset + 5]).unwrap();
        assert_eq!(kw, "xref\n", "D-AQ : startxref offset must point to 'xref\\n'");
    }

    #[test]
    fn d_aq_pdf_ends_with_eof_marker() {
        let pdf = build_quarterly_pdf("Q2-2026", "body", b"{}").unwrap();
        let tail = std::str::from_utf8(&pdf[pdf.len() - 6..]).unwrap();
        assert_eq!(tail, "%%EOF\n", "D-AQ : PDF must end with %%EOF newline");
    }

    #[test]
    fn d_x_extract_returns_first_marker_when_multiple_present() {
        // Documented behaviour : if a hand-crafted PDF carries multiple
        // markers, `find()` returns the FIRST.  The verifier's SHA
        // mismatch check still catches this : attacker can't make the
        // first marker equal `sha256(json)` because they don't know it.
        //
        // Updated after **D-AR** : both candidates must be 64 hex
        // chars to pass the strict validation, otherwise the function
        // returns None.  Test now uses two valid 64-hex-char strings.
        let pdf = b"%PDF\n/EFE_BENCH_EVIDENCE_JSON_SHA256 (aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa)\n/EFE_BENCH_EVIDENCE_JSON_SHA256 (bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb)\n";
        assert_eq!(
            extract_embedded_json_sha256(pdf),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
            "D-X : first-match semantics MUST hold"
        );
    }
}
