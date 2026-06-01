//! Span-based text redactor.
//!
//! Given an input string + a list of [`DetectedSpan`] (with byte
//! offsets into the input), produces a new string where each span is
//! replaced by `[REDACTED:TYPE]`. Spans must be sorted by start ; we
//! sort defensively.
//!
//! Multi-byte UTF-8 safety : every span boundary is verified to be on a
//! char boundary before splicing.

use crate::policy::DetectedSpan;

/// Replace every detected span in `input` with `[REDACTED:type]`.
///
/// Spans are sorted by `start` then applied left-to-right. Overlapping
/// spans are merged (the second one is dropped) to avoid double-redaction.
pub fn redact(input: &str, spans: &[DetectedSpan]) -> String {
    if spans.is_empty() {
        return input.to_string();
    }
    let mut sorted: Vec<DetectedSpan> = spans.to_vec();
    sorted.sort_by_key(|s| s.start);

    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut cursor = 0usize;
    let mut prev_end = 0usize;

    for span in &sorted {
        // Overlap : skip if this span starts before prev ends.
        if span.start < prev_end {
            continue;
        }
        // Sanity : ensure boundaries on char boundaries.
        if span.start > bytes.len() || span.end > bytes.len() || span.start >= span.end {
            continue;
        }
        if !input.is_char_boundary(span.start) || !input.is_char_boundary(span.end) {
            // Drop the span rather than corrupt UTF-8.
            continue;
        }
        // Emit the run from `cursor` up to `span.start` verbatim.
        out.push_str(&input[cursor..span.start]);
        // Emit the placeholder.
        out.push_str("[REDACTED:");
        out.push_str(&span.entity);
        out.push(']');
        cursor = span.end;
        prev_end = span.end;
    }
    // Trailing slice.
    out.push_str(&input[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: usize, end: usize, entity: &str) -> DetectedSpan {
        DetectedSpan {
            start,
            end,
            entity: entity.into(),
            score: 0.9,
        }
    }

    #[test]
    fn no_spans_returns_input() {
        assert_eq!(redact("hello", &[]), "hello");
    }

    #[test]
    fn single_span() {
        let s = "Hello, john@example.com !";
        let spans = vec![span(7, 23, "EMAIL_ADDRESS")];
        assert_eq!(redact(s, &spans), "Hello, [REDACTED:EMAIL_ADDRESS] !");
    }

    #[test]
    fn multiple_spans_sorted() {
        let s = "Call John at +33 1 23 45 67 89.";
        let spans = vec![
            span(5, 9, "PERSON"),
            span(13, 30, "PHONE_NUMBER"),
        ];
        assert_eq!(
            redact(s, &spans),
            "Call [REDACTED:PERSON] at [REDACTED:PHONE_NUMBER].",
        );
    }

    #[test]
    fn multiple_spans_unsorted_input() {
        let s = "Call John at +33 1 23 45 67 89.";
        // Provide spans out of order :
        let spans = vec![
            span(13, 30, "PHONE_NUMBER"),
            span(5, 9, "PERSON"),
        ];
        assert_eq!(
            redact(s, &spans),
            "Call [REDACTED:PERSON] at [REDACTED:PHONE_NUMBER].",
        );
    }

    #[test]
    fn overlapping_spans_second_dropped() {
        let s = "abcdefghij";
        let spans = vec![span(0, 5, "A"), span(3, 8, "B")];
        // Second overlaps : dropped.
        assert_eq!(redact(s, &spans), "[REDACTED:A]fghij");
    }

    #[test]
    fn invalid_boundaries_dropped() {
        // "café" : é is 2 bytes (0xC3 0xA9). Cutting at byte 4 is mid-char.
        let s = "café-user";
        let bad = span(0, 4, "PERSON"); // 4 = mid-é
        assert_eq!(redact(s, &[bad]), s, "invalid boundary span dropped");
    }

    #[test]
    fn span_at_end_of_string() {
        let s = "Contact admin@example.com";
        let spans = vec![span(8, 25, "EMAIL_ADDRESS")];
        assert_eq!(redact(s, &spans), "Contact [REDACTED:EMAIL_ADDRESS]");
    }

    #[test]
    fn span_at_start_of_string() {
        let s = "john@example.com is the contact";
        let spans = vec![span(0, 16, "EMAIL_ADDRESS")];
        assert_eq!(redact(s, &spans), "[REDACTED:EMAIL_ADDRESS] is the contact");
    }

    #[test]
    fn unicode_safe() {
        let s = "Hello café";
        let spans = vec![span(6, 11, "PERSON")];   // 6..11 = "café"
        // Span ends at byte 11 (after é). Must respect char boundary.
        if !s.is_char_boundary(11) {
            assert_eq!(redact(s, &spans), s); // dropped
        } else {
            assert_eq!(redact(s, &spans), "Hello [REDACTED:PERSON]");
        }
    }
}
