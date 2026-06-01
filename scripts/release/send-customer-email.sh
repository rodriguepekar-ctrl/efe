#!/usr/bin/env bash
# send-customer-email.sh — Trigger Mailgun batch send for release announce.
#
# Reads template + tenants list ; substitutes placeholders ; POST batch
# to Mailgun. Stub-compatible if MAILGUN_API_KEY is missing.
#
# Usage :
#   ./scripts/release/send-customer-email.sh \
#     docs/marketing/CUSTOMER_EMAIL_TEMPLATE.md v1.0.0

set -euo pipefail

TEMPLATE_FILE="${1:?usage: $0 <template-file> <version>}"
VERSION="${2:?usage: $0 <template-file> <version>}"

if [[ -z "${MAILGUN_API_KEY:-}" ]]; then
  echo "⚠ MAILGUN_API_KEY not set ; skipping customer email."
  exit 0
fi

if [[ ! -f "$TEMPLATE_FILE" ]]; then
  echo "❌ template file not found : $TEMPLATE_FILE" >&2
  exit 1
fi

MAILGUN_DOMAIN="${MAILGUN_DOMAIN:-mg.efe.example.com}"
TENANTS_LIST_URL="${TENANTS_LIST_URL:-https://efe.example.com/api/internal/customer-list.json}"
SUBJECT="🚀 EFE ${VERSION} GA — what's new for your tenant"

# Body (Markdown). Mailgun supports both text and HTML ; we send both.
BODY_MD=$(cat "$TEMPLATE_FILE")

# Convert Markdown to HTML (best-effort using pandoc if available, else plain).
if command -v pandoc >/dev/null 2>&1; then
  BODY_HTML=$(echo "$BODY_MD" | pandoc -f markdown -t html)
else
  # Minimal fallback : wrap in <pre>, escaping `<` and `&` so the
  # Markdown source renders verbatim (no accidental HTML execution).
  ESCAPED="${BODY_MD//&/&amp;}"
  ESCAPED="${ESCAPED//</&lt;}"
  BODY_HTML="<pre>${ESCAPED}</pre>"
fi

# Fetch tenants list (a private endpoint).
echo "▶ Fetching customer list ..."
TENANTS_JSON=$(curl -fsSL "$TENANTS_LIST_URL" \
  -H "Authorization: Bearer ${EFE_INTERNAL_TOKEN:-}")

TOTAL=$(echo "$TENANTS_JSON" | jq 'length')
echo "  → $TOTAL customers"

# Mailgun batch send (recipient-variables for personalization).
echo "▶ Sending via Mailgun ..."
RECIPIENT_VARIABLES=$(echo "$TENANTS_JSON" | jq -r '
  map({(.email): {tenant_name: .tenant_name, tenant_slug: .tenant_slug, contact_name: .contact_name}}) | add
')

# Mailgun supports up to 1000 recipients per batch.
TO=$(echo "$TENANTS_JSON" | jq -r 'map(.email) | join(",")')

curl -fsSL --user "api:${MAILGUN_API_KEY}" \
  "https://api.mailgun.net/v3/${MAILGUN_DOMAIN}/messages" \
  -F from="EFE Team <release@${MAILGUN_DOMAIN}>" \
  -F to="$TO" \
  -F subject="$SUBJECT" \
  -F text="$BODY_MD" \
  -F html="$BODY_HTML" \
  -F recipient-variables="$RECIPIENT_VARIABLES" \
  -F o:tag="release-${VERSION}" \
  -F o:tracking="yes" \
  >/dev/null

echo "✅ Customer email sent to $TOTAL recipients"
