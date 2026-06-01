# 02 — Multi-tenant isolation

> **Time** : 10 minutes
> **Pre-requisite** : tutorial 01 completed

## Goal

See EFE's D11 zero-trust multi-tenant isolation in action : tenant `acme` cannot
read or modify tenant `beta-corp`'s catalogs, and vice versa.

## Step 1 — Set up acme + load support-routing

If you haven't already :
1. Sign in as **Acme Corp**
2. Load the **support-routing** sample

## Step 2 — Switch to beta-corp

1. Click **Sign in** in the sidebar
2. Click **Sign out** at the bottom
3. Click the **Beta Corporation** card

You're now signed in as `beta-corp`. Notice the tenant pill in the top-right.

## Step 3 — Verify isolation : beta-corp doesn't see acme's catalog

1. Click **Catalogs** in the sidebar
2. Result : empty — "No catalogs yet"

This proves D11 isolation : `beta-corp` cannot enumerate `acme`'s catalogs.

## Step 4 — beta-corp creates a different catalog

Same procedure as tutorial 01 :
1. Load the **content-moderation** sample (instead of support-routing)
2. Click **🎯 Classify** on the loaded sample
3. Try : `"Buy crypto NOW guaranteed 1000% returns!"` → top class : `spam`

## Step 5 — Switch back to acme

1. Sign out
2. Sign in as **Acme Corp**
3. Click **Catalogs**

You should still see `support-routing` (acme's catalog from tutorial 01).
You do NOT see `content-moderation` (which belongs to beta-corp).

## Step 6 — Verify via audit log

1. Click **Audit log**
2. You see only `acme` events

Switch back to beta-corp and check :
- You see only `beta-corp` events

The audit log is **tenant-scoped** : every read is filtered by `tenant_id` claim.

## Step 7 — Try a third tenant : demo-fr (multilingual)

1. Sign out, sign in as **Démo Français**
2. Load the **multilingual-intents** sample
3. Try classifying :
   - `"Bonjour, comment allez-vous?"` → `greeting`
   - `"Merci beaucoup pour votre aide"` → `thanks`
   - `"Estoy muy decepcionado con el servicio"` → `complaint`
   - `"Auf Wiedersehen"` → `farewell`

The same model handles English / French / Spanish / German / Italian / Portuguese.
That's the M13 D26 production multilingual bundle pattern.

## What just happened technically

Per request, EFE :

1. Validates the JWT bearer token (D15 OIDC)
2. Extracts the `tenant_id` claim
3. Scopes ALL data access (catalogs, audit log) by `tenant_id` (D11)
4. Audits the action with the tenant_id (D14-bis)

In production :
- The OIDC issuer is real (Auth0 / Keycloak / Azure AD)
- The data store is Postgres with row-level security per tenant_id
- NetworkPolicy strict (M12 D21) prevents pod-level cross-tenant network access

## Threat model : what if a tenant tries to forge a token?

In the playground, demo tokens are NOT cryptographically signed (anyone can craft
a `playground|acme|admin|...` base64 string). This is **only safe for offline
playground use**.

In production :
- Tokens are JWTs signed by the OIDC issuer's private key
- EFE verifies the signature against the issuer's JWKS
- Tampered tokens fail verification → 401 Unauthorized

See `docs/security/threat_model.md` §3.1 (Spoofing threats) for the full
threat list.

## Recap

You've seen :
- Per-tenant catalog isolation (D11)
- Per-tenant audit log isolation
- Multi-language classification (M13 D26 pattern)
- The trust boundary between OIDC issuer and EFE server

## Next : [03 — Drift detection](03_drift_detection.md)
