#!/usr/bin/env bash
# publish-blog.sh — Publish blog post to CMS via API.
#
# Reads a Markdown file, converts to JSON payload, POSTs to CMS API.
# Stub-compatible : skips gracefully if CMS_API_TOKEN is missing.
#
# Usage :
#   ./scripts/release/publish-blog.sh docs/marketing/BLOG_POST_v1_0_0_GA.md v1.0.0

set -euo pipefail

POST_FILE="${1:?usage: $0 <path-to-markdown> <version>}"
VERSION="${2:?usage: $0 <path-to-markdown> <version>}"

if [[ -z "${CMS_API_TOKEN:-}" ]]; then
  echo "⚠ CMS_API_TOKEN not set ; skipping blog publish."
  exit 0
fi

if [[ ! -f "$POST_FILE" ]]; then
  echo "❌ blog post file not found : $POST_FILE" >&2
  exit 1
fi

CMS_API_URL="${CMS_API_URL:-https://efe.example.com/api/cms}"

# Extract title from first H1 line (# Title).
TITLE=$(grep -m 1 "^# " "$POST_FILE" | sed 's/^# //')
TITLE="${TITLE:-EFE ${VERSION} GA}"

# Build payload (Markdown body wrapped in JSON-safe escaping).
BODY_JSON=$(jq -Rs . < "$POST_FILE")
# Slug derivation : strip leading `v`, replace `.` with `-`, append `-ga`.
# (v1.0.0 → 1-0-0-ga). Earlier `tr 'v' '-'` was too aggressive — it
# replaced ALL `v` chars and produced a leading dash.
SLUG_BASE="${VERSION#v}"
SLUG="$(echo "$SLUG_BASE" | tr '.' '-')-ga"

PAYLOAD=$(jq -n \
  --arg title "$TITLE" \
  --argjson body "$BODY_JSON" \
  --arg slug "$SLUG" \
  --arg version "$VERSION" \
  '{
     title: $title,
     slug: $slug,
     body_markdown: $body,
     tags: ["release", "ga", $version],
     published: true
   }')

echo "▶ Publishing blog post '$TITLE' to $CMS_API_URL"
RESPONSE=$(curl -fsSL -X POST "$CMS_API_URL/posts" \
  -H "Authorization: Bearer ${CMS_API_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "$PAYLOAD")

URL=$(echo "$RESPONSE" | jq -r '.url // .post_url // ""')
if [[ -n "$URL" ]]; then
  echo "✅ Published : $URL"
else
  echo "⚠ CMS responded but no URL extracted. Raw response :"
  echo "$RESPONSE" | head -c 500
fi
