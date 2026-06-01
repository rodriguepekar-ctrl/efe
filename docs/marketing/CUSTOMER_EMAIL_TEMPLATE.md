# Customer Email — EFE v1.0.0 GA Announcement

This template is consumed by `scripts/release/send-customer-email.sh` and
sent via the **Mailgun batch API**. Handlebars-style placeholders
(`{{tenant_name}}`, `{{tenant_slug}}`, `{{contact_name}}`) are
substituted per-recipient by the script's batch resolver.

---

## Subject

```
EFE v1.0.0 is now GA — what this means for {{tenant_name}}
```

## From

```
EFE Team <release@efe.example.com>
```

## Reply-to

```
support@efe.example.com
```

## Tags (Mailgun)

```
release-announce, v1.0.0, ga
```

## Priority

```
o:priority=normal
```

---

## Plain-text body

```
Hi {{contact_name}},

We're excited to share that EFE v1.0.0 (General Availability) is
shipping today, 2026-05-09.

What this means for {{tenant_name}} :

1. Your tenant ('{{tenant_slug}}') has been auto-pinned to v1.0.0 on
   the staging cluster. The production cluster will be migrated during
   your next planned maintenance window.

2. Compliance evidence has been regenerated for {{tenant_name}}. Your
   compliance pack ZIP is available at :
       https://portal.efe.example.com/{{tenant_slug}}/compliance

3. All EFE artifacts are now Sigstore-signed and ship with SLSA-3
   provenance. Verify any artifact with :
       cosign verify ghcr.io/efe-ai/efe-server:1.0.0 \
         --certificate-identity-regexp "^https://github\.com/efe-ai/efe/.*" \
         --certificate-oidc-issuer "https://token.actions.githubusercontent.com"
   Full guide : https://efe.example.com/verify

What's new :

- 4 production-grade Docker images (efe-server, mcp-server,
  portal-admin, efe-server-migrate).
- Helm umbrella chart (efe-stack) with 6 sub-charts.
- 3 Terraform modules : AWS, GCP, Azure.
- 6 CLI binaries : darwin/linux/windows × amd64/arm64.
- 4 SDKs (Python, Go, TypeScript, Java).
- Per-tenant compliance evidence package (SOC 2, GDPR Art. 30, AI Act
  Art. 11, ISO 27001).

Action required from {{tenant_name}} :

- Review your compliance pack at the URL above.
- (Optional) Verify the artifacts you deploy in CI using cosign.
- (Optional) Update your Terraform module pin to "v1.0.0".

No breaking changes from your last RC (v0.34). Migration is fully
backward-compatible.

Resources :

- Release notes :
      https://github.com/efe-ai/efe/blob/v1.0.0/docs/release/RELEASE_NOTES_v1.0.0.md
- Verification guide :
      https://github.com/efe-ai/efe/blob/v1.0.0/docs/release/VERIFY_RELEASE.md
- Security disclosure policy :
      https://github.com/efe-ai/efe/blob/v1.0.0/docs/release/SECURITY_DISCLOSURE_POLICY.md
- RSS feed :
      https://efe.example.com/security/feed.xml
- Status page :
      https://status.efe.example.com

Questions ? Reply to this email or reach the team at
support@efe.example.com.

We're proud of where v1.0.0 lands and grateful for your partnership in
building EFE. Onward !

— The EFE Team
```

---

## HTML body

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <title>EFE v1.0.0 is now GA</title>
  <style>
    body { font-family: -apple-system, "Segoe UI", Helvetica, Arial, sans-serif;
           color: #1a1a1a; line-height: 1.5; max-width: 640px; margin: 0 auto;
           padding: 24px; }
    h1 { font-size: 22px; margin-top: 0; }
    h2 { font-size: 16px; color: #2a4d8f; margin-top: 24px; }
    code { background: #f4f4f5; padding: 2px 6px; border-radius: 3px;
           font-size: 13px; }
    pre  { background: #f4f4f5; padding: 12px; border-radius: 6px;
           overflow-x: auto; font-size: 12px; }
    .cta { display: inline-block; padding: 10px 20px; margin-top: 12px;
           background: #2a4d8f; color: #fff; text-decoration: none;
           border-radius: 4px; }
    .footer { margin-top: 32px; font-size: 12px; color: #555; }
  </style>
</head>
<body>
  <h1>EFE v1.0.0 is now GA — what this means for {{tenant_name}}</h1>

  <p>Hi {{contact_name}},</p>

  <p>We're excited to share that <strong>EFE v1.0.0 (General Availability)</strong>
     is shipping today, <strong>2026-05-09</strong>.</p>

  <h2>What this means for {{tenant_name}}</h2>
  <ol>
    <li>Your tenant (<code>{{tenant_slug}}</code>) has been auto-pinned
        to v1.0.0 on the staging cluster.</li>
    <li>Your compliance pack has been regenerated and is available at
        <a href="https://portal.efe.example.com/{{tenant_slug}}/compliance">
          portal.efe.example.com/{{tenant_slug}}/compliance</a>.</li>
    <li>All artifacts are <strong>Sigstore-signed</strong> with
        <strong>SLSA-3 provenance</strong>.</li>
  </ol>

  <h2>Verify your install</h2>
  <pre><code>cosign verify ghcr.io/efe-ai/efe-server:1.0.0 \
  --certificate-identity-regexp "^https://github\.com/efe-ai/efe/.*" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com"</code></pre>

  <p>
    <a class="cta" href="https://efe.example.com/verify">Full verification guide →</a>
  </p>

  <h2>Resources</h2>
  <ul>
    <li><a href="https://github.com/efe-ai/efe/blob/v1.0.0/docs/release/RELEASE_NOTES_v1.0.0.md">Release notes</a></li>
    <li><a href="https://github.com/efe-ai/efe/blob/v1.0.0/docs/release/VERIFY_RELEASE.md">Verification guide</a></li>
    <li><a href="https://github.com/efe-ai/efe/blob/v1.0.0/docs/release/SECURITY_DISCLOSURE_POLICY.md">Security disclosure policy</a></li>
    <li><a href="https://efe.example.com/security/feed.xml">Security RSS feed</a></li>
  </ul>

  <p>Questions? Reply to this email or reach us at
     <a href="mailto:support@efe.example.com">support@efe.example.com</a>.</p>

  <p>— The EFE Team</p>

  <div class="footer">
    EFE-AI Inc. · 14 rue de la Paix · 75002 Paris, France<br />
    You received this email because <code>{{tenant_name}}</code> is on
    the EFE security mailing list.
    <a href="https://efe.example.com/unsubscribe?tenant={{tenant_slug}}">Unsubscribe</a>.
  </div>
</body>
</html>
```

---

## Send list resolution

The send list is built by joining :

- The active-tenant table (`SELECT id, name, primary_contact_email FROM
  tenants WHERE status = 'active'`).
- Excluded if `notification_preferences.releases = false`.
- Up to 5 contacts per tenant (primary + admins + opted-in operators).

The resolver lives in `scripts/release/resolve-customer-list.py`
(invoked by `send-customer-email.sh`).

---

## Send window

| Severity                   | Send window      |
|----------------------------|------------------|
| GA / minor release         | T+2 h post-pipeline (release-announce phase) |
| Patch release (non-security) | T+1 day        |
| Security yank (P0/P1)      | Immediate (< 1 h) |
| Security yank (P2)         | < 24 h           |

---

## Compliance metadata

- Mailgun message-id is logged in `audit_log` with action `release_announce_email_sent`.
- The send is included in the next compliance pack regeneration.
- Bounce / unsubscribe events are honored within 1 business day.
