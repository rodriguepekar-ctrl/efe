//! PR #24 § 14 + AD35 — PR #23 smoke badge embed.
//!
//! Embedded in the overview header via `<img src>` — no fetch, no JS.

use crate::publish::crosslink::SMOKE_BADGE_URL;

/// Render the smoke badge HTML fragment for the overview header.
#[must_use]
pub fn render_header_html() -> String {
    format!(
        "<a href=\"https://efe.github.io/smoke/\"><img src=\"{SMOKE_BADGE_URL}\" alt=\"smoke\"></a>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_html_references_pr23_frozen_url() {
        let html = render_header_html();
        assert!(html.contains("https://efe.github.io/smoke/badge.svg"));
        assert!(html.contains("<img"));
    }

    #[test]
    fn header_html_has_no_javascript_t100() {
        let html = render_header_html().to_lowercase();
        assert!(!html.contains("<script"));
        assert!(!html.contains("onclick"));
        assert!(!html.contains("javascript:"));
    }
}
