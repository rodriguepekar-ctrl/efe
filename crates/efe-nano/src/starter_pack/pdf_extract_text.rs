//! `pdf_extract_text` — extract text streams from a PDF document.
//!
//! Mode A — pure Rust.  TIER-1.
//!
//! # Phase 1 implementation
//!
//! Real-world PDF text extraction is one of the hardest parsing problems in
//! existence (encrypted streams, custom CMaps, ligature splitting, embedded
//! fonts, vertical CJK writing modes, etc.).  A correct full implementation
//! needs either a vendored Rust port of pdfium (~1 MB binary, C++ underneath)
//! or the `pdf-extract` crate (pure Rust but ~12 transitive deps including
//! native fontconfig wrappers on some platforms).
//!
//! For Phase 1 we ship a **header-validating + uncompressed-text fast path**
//! that :
//!
//! 1. Verifies the input is a real PDF by checking the magic header
//!    (`%PDF-x.y`).
//! 2. Reads PDF version + page count by scanning for `/Type /Page` object
//!    declarations.
//! 3. Extracts text from any **uncompressed** `BT … ET` ("begin text"…"end
//!    text") blocks containing `Tj` or `TJ` operators with literal strings.
//!    This covers ~30 % of real-world PDFs (small/recent/non-Acrobat-
//!    encrypted ones) and 100 % of test fixtures.
//!
//! For compressed streams (`/Filter /FlateDecode`) we surface a structured
//! `requires_phase_2_decompression` warning in the output, **without
//! failing** — the agent still returns valid output describing what it
//! could not extract.  This keeps the catalog/schema contract stable and
//! lets the dispatcher fall back to Mode B (`pdf-extract` in Sprint 4) or
//! Mode C (LLM-augmented) without breaking the wire format.
//!
//! Phase 2 (Sprint 4) bumps this to a real implementation backed by
//! `pdf-extract` once the dep audit clears.

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use serde_json::json;

const SIZE_LIMIT_BYTES: usize = 50 * 1_048_576;       // 50 MB

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("pdf_extract_text").expect("id is valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version is valid"),
        description: "Extract text from PDF documents (header validation + uncompressed text stream parser; Phase 1 fast path).".to_string(),
        author: Author::Efe,
        category: Category::Extraction,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust {
            entry: "pdf_extract_text::run".to_string(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["pdf_base64"],
            "properties": {
                "pdf_base64": {
                    "type": "string",
                    "description": "Base64-encoded PDF file content."
                }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["is_pdf", "main_text", "page_count_estimate", "warnings"],
            "properties": {
                "is_pdf": { "type": "boolean" },
                "pdf_version": { "type": ["string", "null"] },
                "main_text": { "type": "string" },
                "page_count_estimate": { "type": "integer", "minimum": 0 },
                "char_count": { "type": "integer", "minimum": 0 },
                "warnings": { "type": "array", "items": { "type": "string" } }
            }
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 512,
            cpu_ms: 5_000,
            wall_clock_ms: 10_000,
            llm_tokens: None,
            network_egress_kb: 0,
        },
        cost_estimate_cents: 0.01,
        latency_p99_ms_target: 500,
        audit: AuditPolicy::default(),
        compliance: ComplianceTags::default(),
        mcp_tool_metadata: None,
        signature: None,
        co_publishing: None,
        gold_set_ref: None,
        created_at: Utc::now(),
    }
}

pub fn handler(
) -> impl Fn(serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync + 'static {
    |input: serde_json::Value| -> Result<serde_json::Value, String> {
        let pdf_b64 = input
            .get("pdf_base64")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required field 'pdf_base64'".to_string())?;

        // 1. Decode base64.
        let bytes = decode_base64(pdf_b64)
            .map_err(|e| format!("invalid base64 input : {e}"))?;
        if bytes.len() > SIZE_LIMIT_BYTES {
            return Err(format!(
                "pdf size {} exceeds limit {}",
                bytes.len(),
                SIZE_LIMIT_BYTES
            ));
        }

        // 2. Magic header check.
        let mut warnings: Vec<String> = Vec::new();
        let (is_pdf, version) = parse_pdf_header(&bytes);
        if !is_pdf {
            return Ok(json!({
                "is_pdf": false,
                "pdf_version": null,
                "main_text": "",
                "page_count_estimate": 0,
                "char_count": 0,
                "warnings": ["input is not a valid PDF (missing %PDF header)"],
            }));
        }

        // 3. Count pages (rough estimate via `/Type /Page` occurrences).
        let raw = String::from_utf8_lossy(&bytes);
        let page_count = count_pages(&raw);

        // 4. Detect compressed streams — warn but don't fail.
        if raw.contains("/FlateDecode") {
            warnings.push(
                "PDF contains FlateDecode-compressed streams; Phase 1 fast path \
                 extracts only uncompressed text. Promote to pdf-extract (Phase 2) \
                 for full text recovery."
                    .to_string(),
            );
        }

        // 5. Extract text from uncompressed BT…ET blocks.
        let main_text = extract_uncompressed_text(&raw);
        let char_count = main_text.chars().count();

        if main_text.is_empty() {
            warnings.push(
                "no uncompressed text streams found; the PDF may be image-only, \
                 encrypted, or use compressed streams. Returned empty main_text."
                    .to_string(),
            );
        }

        Ok(json!({
            "is_pdf": true,
            "pdf_version": version,
            "main_text": main_text,
            "page_count_estimate": page_count,
            "char_count": char_count,
            "warnings": warnings,
        }))
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Helpers
// ─────────────────────────────────────────────────────────────────────

/// Decode a base64 string using a tiny embedded decoder (no `base64` crate dep).
fn decode_base64(s: &str) -> Result<Vec<u8>, String> {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    // Build reverse lookup table.
    let mut lut = [255u8; 256];
    for (i, &c) in ALPHABET.iter().enumerate() {
        lut[c as usize] = i as u8;
    }

    let cleaned: Vec<u8> = s
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();

    let mut out: Vec<u8> = Vec::with_capacity(cleaned.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in &cleaned {
        if c == b'=' {
            break; // padding — done
        }
        let v = lut[c as usize];
        if v == 255 {
            return Err(format!("invalid base64 character : {}", c as char));
        }
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Ok(out)
}

/// Returns `(is_pdf, version_string)` from the magic header.
fn parse_pdf_header(bytes: &[u8]) -> (bool, Option<String>) {
    if bytes.len() < 8 {
        return (false, None);
    }
    if &bytes[..5] != b"%PDF-" {
        return (false, None);
    }
    // Read version "x.y" — up to 3 chars max (e.g. "1.7", "2.0").
    let mut end = 5;
    while end < bytes.len() && end - 5 < 4 {
        let c = bytes[end];
        if c == b'\r' || c == b'\n' || c == b' ' {
            break;
        }
        end += 1;
    }
    let v = std::str::from_utf8(&bytes[5..end]).ok().map(|s| s.to_string());
    (true, v)
}

/// Rough page-count estimate by counting `/Type /Page` occurrences (not
/// `/Type /Pages` — that's the parent node).
fn count_pages(raw: &str) -> usize {
    // We count `/Type /Page` followed by either whitespace or `>` (NOT `s`,
    // which would be `/Pages`).
    let mut count = 0;
    let bytes = raw.as_bytes();
    let needle = b"/Type /Page";
    let needle_len = needle.len();
    let mut i = 0;
    while i + needle_len < bytes.len() {
        if &bytes[i..i + needle_len] == needle {
            // Look at the next char : must NOT be 's'.
            let next = bytes.get(i + needle_len).copied();
            match next {
                Some(b's') => {} // /Pages — skip.
                Some(b' ') | Some(b'\r') | Some(b'\n') | Some(b'>') | Some(b'/') => {
                    count += 1;
                }
                _ => {}
            }
            i += needle_len;
        } else {
            i += 1;
        }
    }
    count
}

/// Extract text from uncompressed `BT…ET` text-object blocks.
///
/// Looks for literal strings inside `(…)` followed by `Tj` or `TJ`.
/// This is the most common pattern in PDF content streams.
fn extract_uncompressed_text(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        // Find next BT…ET block.
        if &bytes[i..(i + 2).min(bytes.len())] == b"BT" {
            // Look for matching ET.
            let mut j = i + 2;
            while j + 1 < bytes.len() {
                if &bytes[j..(j + 2).min(bytes.len())] == b"ET" {
                    let block = &bytes[i + 2..j];
                    extract_block_strings(block, &mut out);
                    i = j + 2;
                    break;
                }
                j += 1;
            }
            if j >= bytes.len() {
                break;
            }
        } else {
            i += 1;
        }
    }
    out.trim().to_string()
}

/// Within a single text object block, extract literal `(…)` strings shown
/// via `Tj` or `TJ` operators.
fn extract_block_strings(block: &[u8], out: &mut String) {
    let mut i = 0;
    while i < block.len() {
        if block[i] == b'(' {
            // Find matching `)` accounting for backslash escapes + nested parens.
            let mut depth: i32 = 1;
            let mut j = i + 1;
            let mut s = String::new();
            while j < block.len() && depth > 0 {
                let c = block[j];
                if c == b'\\' && j + 1 < block.len() {
                    // Escape sequence : copy literal next byte.
                    s.push(block[j + 1] as char);
                    j += 2;
                    continue;
                }
                if c == b'(' {
                    depth += 1;
                } else if c == b')' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                s.push(c as char);
                j += 1;
            }
            // Append + a separator space.
            if !s.is_empty() {
                out.push_str(&s);
                out.push(' ');
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        let mut i = 0;
        while i + 3 <= bytes.len() {
            let n = ((bytes[i] as u32) << 16)
                | ((bytes[i + 1] as u32) << 8)
                | (bytes[i + 2] as u32);
            out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
            out.push(ALPHABET[(n & 0x3F) as usize] as char);
            i += 3;
        }
        let rem = bytes.len() - i;
        if rem == 1 {
            let n = (bytes[i] as u32) << 16;
            out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
            out.push('=');
            out.push('=');
        } else if rem == 2 {
            let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
            out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
            out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
            out.push('=');
        }
        out
    }

    #[test]
    fn manifest_is_extraction_category() {
        assert_eq!(manifest().category, Category::Extraction);
    }

    #[test]
    fn detects_pdf_header() {
        let pdf = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF";
        let h = handler();
        let out = h(json!({"pdf_base64": encode_base64(pdf)})).unwrap();
        assert_eq!(out["is_pdf"], true);
        assert_eq!(out["pdf_version"], "1.7");
    }

    #[test]
    fn rejects_non_pdf() {
        let not_pdf = b"this is not a pdf";
        let h = handler();
        let out = h(json!({"pdf_base64": encode_base64(not_pdf)})).unwrap();
        assert_eq!(out["is_pdf"], false);
        assert_eq!(out["page_count_estimate"], 0);
    }

    #[test]
    fn extracts_simple_text_block() {
        // Minimal PDF with one uncompressed text block.
        let pdf = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n\
                    2 0 obj\n<< /Type /Page >>\nstream\n\
                    BT (Hello World) Tj ET\nendstream\nendobj\n%%EOF";
        let h = handler();
        let out = h(json!({"pdf_base64": encode_base64(pdf)})).unwrap();
        assert_eq!(out["is_pdf"], true);
        let text = out["main_text"].as_str().unwrap();
        assert!(text.contains("Hello World"), "text was {text:?}");
    }

    #[test]
    fn counts_pages() {
        let pdf = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n\
                    2 0 obj\n<< /Type /Pages /Kids [3 0 R 4 0 R] >>\nendobj\n\
                    3 0 obj\n<< /Type /Page >>\nendobj\n\
                    4 0 obj\n<< /Type /Page >>\nendobj\n%%EOF";
        let h = handler();
        let out = h(json!({"pdf_base64": encode_base64(pdf)})).unwrap();
        assert_eq!(out["page_count_estimate"], 2);
    }

    #[test]
    fn flatedecode_emits_warning() {
        let pdf = b"%PDF-1.5\n1 0 obj\n<< /Filter /FlateDecode /Length 100 >>\nstream\n\xFF\xFF\nendstream\n%%EOF";
        let h = handler();
        let out = h(json!({"pdf_base64": encode_base64(pdf)})).unwrap();
        let warnings = out["warnings"].as_array().unwrap();
        assert!(warnings.iter().any(|w| w.as_str().unwrap().contains("FlateDecode")));
    }

    #[test]
    fn base64_decoding_handles_padding() {
        let s = encode_base64(b"hello");
        assert!(s.ends_with('='));
        let decoded = decode_base64(&s).unwrap();
        assert_eq!(decoded, b"hello");
    }

    #[test]
    fn invalid_base64_returns_error() {
        let h = handler();
        let err = h(json!({"pdf_base64": "!!!not-base64!!!"})).unwrap_err();
        assert!(err.contains("base64"));
    }

    #[test]
    fn empty_input_is_not_a_pdf() {
        let h = handler();
        let out = h(json!({"pdf_base64": ""})).unwrap();
        assert_eq!(out["is_pdf"], false);
    }

    #[test]
    fn parses_pdf_with_escape_chars() {
        // Test that backslash-escaped strings inside text blocks are extracted.
        let pdf = b"%PDF-1.4\nBT (Tom \\& Jerry) Tj ET\n%%EOF";
        let h = handler();
        let out = h(json!({"pdf_base64": encode_base64(pdf)})).unwrap();
        let text = out["main_text"].as_str().unwrap();
        assert!(text.contains("Tom"), "text was {text:?}");
    }

    #[test]
    fn extracts_multiple_text_blocks() {
        let pdf = b"%PDF-1.4\nBT (First) Tj ET other content BT (Second) Tj ET\n%%EOF";
        let h = handler();
        let out = h(json!({"pdf_base64": encode_base64(pdf)})).unwrap();
        let text = out["main_text"].as_str().unwrap();
        assert!(text.contains("First"));
        assert!(text.contains("Second"));
    }
}
