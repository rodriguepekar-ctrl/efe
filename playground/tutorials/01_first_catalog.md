# 01 — Your first catalog : a support-ticket router

> **Time** : 10 minutes
> **Pre-requisite** : playground running (`bash playground/scripts/quickstart.sh`)

## Goal

Build a working support-ticket router that classifies customer messages
into 5 queues : billing / technical / sales-inquiry / cancellation / feature-request.

## Step 1 — Sign in

1. Open http://localhost:8765/
2. Click **Sign in** in the sidebar
3. Click the **Acme Corp** card
4. Notice the tenant pill in the top-right turn purple — you're now signed in as `acme`.

## Step 2 — Load the sample (fast path)

1. Click **Samples** in the sidebar
2. Click **Load into acme** on the **support-routing** card
3. Notice the toast : `Loaded support-routing v1`
4. The UI auto-redirects to the Classify view

## Step 2-bis — OR build manually (recommended for learning)

If you want to build the catalog yourself instead of loading the sample :

1. Click **Catalogs** in the sidebar
2. Click **＋ New catalog**
3. Set name : `support-routing-manual`
4. Replace the 2 default classes with these 5 :
   - `billing` : "I was charged twice", "Where's my invoice", "Refund please"
   - `technical` : "The app crashes", "500 error", "Login broken"
   - `sales-inquiry` : "Tell me about pricing", "Schedule a demo"
   - `cancellation` : "Cancel my subscription", "Close my account"
   - `feature-request` : "Please add dark mode", "Slack integration please"
5. Click **💾 Register catalog**
6. Notice the response shows `version: 1` and a 64-char fingerprint.

## Step 3 — Classify your first text

1. Click **Classify** in the sidebar
2. Catalog dropdown : pick `support-routing` (or `support-routing-manual`)
3. Text : paste `"My credit card was charged twice last month"`
4. Top-K : 3
5. Click **🎯 Classify**

You should see :

```
billing         ████████████████████░░░░  0.7234
sales-inquiry   ██████░░░░░░░░░░░░░░░░░░  0.2153
cancellation    █████░░░░░░░░░░░░░░░░░░░  0.1890
```

(Exact scores vary by embedder backend — `sentence-transformers` is more
accurate ; the `hash` fallback is structurally valid but not semantically meaningful.)

## Step 4 — Try edge cases

Try these inputs and observe :

| Input | Expected top class |
|---|---|
| `"The mobile app crashed during login"` | technical |
| `"Send me your enterprise pricing"` | sales-inquiry |
| `"Please delete my account and all data"` | cancellation |
| `"Could you add Excel export?"` | feature-request |
| `"I refuse to pay the invoice"` | billing or cancellation |

The last one is interesting : it's between billing + cancellation. EFE returns
both with elevated scores ; you can tune `confidence_threshold` per class to
escalate to a human reviewer when scores are tied.

## Step 5 — Inspect the audit log

1. Click **Audit log** in the sidebar
2. Observe events :
   - `auth.issue_token`
   - `samples.load` (if you loaded the sample)
   - `classify` (one per classify call)
3. Each event has an HMAC chain — click **🔒 Verify chain integrity**
4. Result : `✓ Chain OK — N events, all HMAC links valid.`

This is the same tamper-evident audit log pattern that production EFE uses
(D14-bis HMAC chain, see `docs/bench/d14bis_hmac_audit_chain_architecture.md`).

## Step 6 — View the dashboard

1. Click **Dashboard** in the sidebar
2. See live metrics :
   - Total catalogs (1 if you loaded the sample)
   - Total audit events
   - Chain integrity status
   - Action breakdown bars

## Recap

You've :
- Created a tenant + got a token
- Registered (or loaded) a catalog with 5 classes
- Classified text with confidence scores
- Verified the audit log integrity
- Watched live metrics

This is **the same flow** as production EFE — just running locally with simpler backend.

## Next : [02 — Multi-tenant isolation](02_multi_tenant.md)
