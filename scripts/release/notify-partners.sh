#!/usr/bin/env bash
# notify-partners.sh — Slack webhook notification for release.
#
# Usage :
#   ./scripts/release/notify-partners.sh v1.0.0

set -euo pipefail

VERSION="${1:?usage: $0 <version>}"

if [[ -z "${SLACK_WEBHOOK_URL:-}" ]]; then
  echo "⚠ SLACK_WEBHOOK_URL not set ; skipping Slack notify."
  exit 0
fi

PAYLOAD=$(jq -n --arg version "$VERSION" '
{
  blocks: [
    {
      type: "header",
      text: { type: "plain_text", text: ("🚀 EFE " + $version + " is now GA") }
    },
    {
      type: "section",
      text: {
        type: "mrkdwn",
        text: ("*Release notes :* https://github.com/efe-ai/efe/releases/" + $version + "\n*Blog :* https://efe.example.com/blog/" + ($version | sub("\\."; "-"; "g")) + "-ga\n*Helm chart :* `oci://ghcr.io/efe-ai/charts/efe-stack:" + ($version | ltrimstr("v")) + "`")
      }
    },
    {
      type: "actions",
      elements: [
        {
          type: "button",
          text: { type: "plain_text", text: "View release" },
          url: ("https://github.com/efe-ai/efe/releases/" + $version)
        },
        {
          type: "button",
          text: { type: "plain_text", text: "Verify signature" },
          url: "https://github.com/efe-ai/efe/blob/main/docs/release/VERIFY_RELEASE.md"
        }
      ]
    }
  ]
}')

echo "▶ Sending Slack notification ..."
curl -fsSL -X POST "$SLACK_WEBHOOK_URL" \
  -H "Content-Type: application/json" \
  -d "$PAYLOAD" >/dev/null

echo "✅ Partners notified on Slack"
