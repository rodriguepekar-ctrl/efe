# Partner Notification — EFE v1.0.0 GA

This template is consumed by `scripts/release/notify-partners.sh` and
posted to **Slack channels** via incoming webhook (one per partner
workspace) and to **partner Discord servers** (where opted in).

> "Partners" = integration partners, OEM resellers, technology
> alliance members. Customers receive a separate email
> (`CUSTOMER_EMAIL_TEMPLATE.md`).

---

## Slack message — header block

```json
{
  "blocks": [
    {
      "type": "header",
      "text": {
        "type": "plain_text",
        "text": "🚀 EFE v1.0.0 is now GA",
        "emoji": true
      }
    },
    {
      "type": "section",
      "text": {
        "type": "mrkdwn",
        "text": "*Released* : 2026-05-09\n*Tag* : `v1.0.0` (+ 6 sub-tags)\n*Pipeline* : <https://github.com/efe-ai/efe/actions/runs/{{run_id}}|GitHub Actions>"
      }
    },
    {
      "type": "section",
      "text": {
        "type": "mrkdwn",
        "text": "*What's new* :\n• 4 production Docker images, all Sigstore-signed + SLSA-3 attested\n• `efe-stack` Helm umbrella chart (6 sub-charts, multi-cloud)\n• Terraform modules for AWS / GCP / Azure\n• 6 CLI binaries (linux/darwin/windows × amd64/arm64)\n• 4 SDKs : Python, Go, TypeScript, Java\n• Per-tenant compliance evidence pack (SOC 2, GDPR, AI Act, ISO 27001)"
      }
    },
    {
      "type": "section",
      "fields": [
        { "type": "mrkdwn", "text": "*Distribution channels*\nOCI · GH Releases · Homebrew · npm · Terraform · Cargo" },
        { "type": "mrkdwn", "text": "*Verification*\n`cosign verify ghcr.io/efe-ai/efe-server:1.0.0 ...`" },
        { "type": "mrkdwn", "text": "*Compliance*\n`efe-admin compliance pack <tenant>`" },
        { "type": "mrkdwn", "text": "*Yank policy*\nRekor signatures permanent ; deprecation via annotations" }
      ]
    },
    {
      "type": "actions",
      "elements": [
        {
          "type": "button",
          "text": { "type": "plain_text", "text": "Release notes", "emoji": true },
          "url": "https://github.com/efe-ai/efe/blob/v1.0.0/docs/release/RELEASE_NOTES_v1.0.0.md"
        },
        {
          "type": "button",
          "text": { "type": "plain_text", "text": "Verify guide", "emoji": true },
          "url": "https://github.com/efe-ai/efe/blob/v1.0.0/docs/release/VERIFY_RELEASE.md"
        },
        {
          "type": "button",
          "text": { "type": "plain_text", "text": "Distribution channels", "emoji": true },
          "url": "https://github.com/efe-ai/efe/blob/v1.0.0/docs/release/DISTRIBUTION_CHANNELS.md"
        },
        {
          "type": "button",
          "text": { "type": "plain_text", "text": "Blog post", "emoji": true },
          "url": "https://efe.example.com/blog/efe-v1-0-0-ga"
        }
      ]
    },
    { "type": "divider" },
    {
      "type": "context",
      "elements": [
        { "type": "mrkdwn", "text": ":lock: All artifacts signed by `release-v1-ga.yml@refs/tags/v1.0.0` (Fulcio + Rekor). Identity verifiable via `cosign verify`." }
      ]
    }
  ]
}
```

## Discord message (Markdown variant)

```
🚀 **EFE v1.0.0 is now GA** — 2026-05-09

**What's new**
• 4 Docker images (Sigstore-signed, SLSA-3)
• `efe-stack` Helm umbrella chart (multi-cloud)
• Terraform modules : AWS, GCP, Azure
• CLI binaries (6 platforms)
• SDKs : Python, Go, TypeScript, Java
• Compliance pack : SOC 2, GDPR Art. 30, AI Act Art. 11, ISO 27001

**Verify**
```
cosign verify ghcr.io/efe-ai/efe-server:1.0.0 \
  --certificate-identity-regexp "^https://github\.com/efe-ai/efe/.*" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com"
```

**Resources**
- Release notes : https://github.com/efe-ai/efe/blob/v1.0.0/docs/release/RELEASE_NOTES_v1.0.0.md
- Verify guide  : https://github.com/efe-ai/efe/blob/v1.0.0/docs/release/VERIFY_RELEASE.md
- Blog          : https://efe.example.com/blog/efe-v1-0-0-ga
- RSS / VEX     : https://efe.example.com/security

🔒 All artifacts signed by `release-v1-ga.yml@refs/tags/v1.0.0`.
```

---

## Distribution

| Workspace                 | Channel              | Webhook env var                         |
|---------------------------|----------------------|------------------------------------------|
| EFE internal              | `#release-feed`      | `SLACK_WEBHOOK_RELEASE_INTERNAL`         |
| Tenant-X (sample partner) | `#partner-x-efe`     | `SLACK_WEBHOOK_PARTNER_X`                |
| Tenant-Y                  | `#efe-announcements` | `SLACK_WEBHOOK_PARTNER_Y`                |
| Discord — EFE community   | `#announcements`     | `DISCORD_WEBHOOK_COMMUNITY`              |
| Tech alliance group       | `#efe-alliance`      | `SLACK_WEBHOOK_ALLIANCE`                 |

The full webhook map lives in 1Password vault `EFE / Release Webhooks`.

---

## Send window

- **GA / minor** : T+2 h after pipeline complete (announce phase).
- **Patch (non-security)** : T+1 business day.
- **Yank** : immediately (uses a different template,
  `partner-yank-notification.md`).

---

## Audit trail

Each post is logged in `audit_log` with action `partner_release_post`,
including the channel and message timestamp. For compliance traceability,
post receipts are included in the next per-tenant compliance pack
regeneration cycle.

---

## References

- Send script : `scripts/release/notify-partners.sh`
- Webhook documentation : Slack Block Kit / Discord webhook v1
- Yank notification : `scripts/release/send-yank-notification.sh` (separate flow)
