#!/usr/bin/env bash
# send-yank-notification.sh — Notify customers of a release yank.
#
# Usage :
#   ./scripts/release/send-yank-notification.sh v1.0.1 P1 "fix breaking schema"

set -euo pipefail

VERSION="${1:?usage: $0 <version> <severity> <reason>}"
SEVERITY="${2:?usage: $0 <version> <severity> <reason>}"
REASON="${3:?usage: $0 <version> <severity> <reason>}"

if [[ -z "${MAILGUN_API_KEY:-}" ]]; then
  echo "⚠ MAILGUN_API_KEY not set ; skipping yank email."
  exit 0
fi

MAILGUN_DOMAIN="${MAILGUN_DOMAIN:-mg.efe.example.com}"
SUBJECT="🚫 EFE ${VERSION} yanked — ${SEVERITY} : action required"

BODY_TEXT=$(cat <<EOF
EFE ${VERSION} has been yanked due to a ${SEVERITY} issue.

Reason :
${REASON}

Action required :
- If you have v${VERSION#v} deployed, pin to the next patch release as soon as one is available.
- Cosign signatures and SBOM remain available (Rekor is append-only) but
  the release is flagged as deprecated.
- Subscribe to the security feed : https://efe.example.com/security/feed.xml

Helm rollback example :
  helm rollback efe-stack --namespace efe

Status page :
https://status.efe.example.com

Support :
support@efe.example.com

— The EFE Team
EOF
)

TO_LIST="${EFE_CUSTOMER_LIST:-customers@efe.example.com}"

echo "▶ Sending yank notification to ${TO_LIST}"
curl -fsSL --user "api:${MAILGUN_API_KEY}" \
  "https://api.mailgun.net/v3/${MAILGUN_DOMAIN}/messages" \
  -F from="EFE Security <security@${MAILGUN_DOMAIN}>" \
  -F to="$TO_LIST" \
  -F subject="$SUBJECT" \
  -F text="$BODY_TEXT" \
  -F o:tag="yank-${VERSION}" \
  -F o:priority="high" \
  >/dev/null

echo "✅ Yank notification sent"
