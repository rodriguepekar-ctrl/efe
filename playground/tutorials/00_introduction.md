# 00 — Introduction to EFE

> **Time** : 5 minutes reading
> **Pre-requisite** : none

## What is EFE ?

EFE (Ephemeral Fluid Swarm) is a **multi-tenant semantic classification gRPC service**.
You define a "catalog" — a set of classes with example sentences — and EFE classifies
new texts against your catalog using **embedding-based nearest-cluster matching**.

## When to use EFE ?

EFE is best for :

| Use case | Examples |
|---|---|
| Customer-support ticket routing | Auto-route tickets to billing / tech / sales / cancellation queues |
| Content moderation triage | Flag spam / toxic / pii content for human review |
| Intent recognition | Backend for chatbots / voice assistants |
| Product feature classification | E-commerce category assignment |
| Multi-tenant intent recognition | Per-customer routing rules |
| Cross-lingual classification | Same classes across 50+ languages |

EFE is **NOT** designed for :

- Generative tasks (text/image/audio generation) — use an LLM
- Biometric identification — refused per AI Act Art. 5
- Predictive policing / social scoring — refused per AI Act Art. 5
- Critical infrastructure safety functions
- Real-time biometric identification in public spaces

## How it works (in 30 seconds)

1. **Operator** registers a `catalog` :
   ```
   {
     "name": "support-routing",
     "classes": [
       { "name": "billing",   "examples": ["I was charged twice", "Where's my invoice"] },
       { "name": "technical", "examples": ["The app crashes", "I get a 500 error"] }
     ]
   }
   ```
2. EFE computes the **centroid** of each class :
   - Each example is embedded into a 384-dim vector
   - Centroid = L2-normalized mean of class examples
3. **End user** sends a new sentence (`"My credit card was rejected"`)
4. EFE :
   - Embeds the sentence
   - Computes cosine similarity vs each class centroid
   - Returns top-K classes with confidence scores

## Why multi-tenant ?

EFE is built for SaaS/B2B from day 1 :
- **Per-tenant catalogs** : tenant A and tenant B can have completely different routing rules
- **Zero-trust isolation** : tenant A cannot read or modify tenant B's catalogs
- **OIDC authentication** : each request authenticated via JWT bearer token
- **Per-tenant rate limit + audit** : independent SLOs per tenant

## What you'll do in the playground

The playground bundles a working EFE-like backend (in-memory + simplified) so you
can try EFE locally without K8s.

You'll go through these 4 scenarios :

1. **First catalog** : build a support-ticket router from scratch
2. **Multi-tenant** : switch between 3 demo tenants and see isolation
3. **Drift detection** : how to spot when your catalog needs re-fitting
4. **Production deployment** : from playground to real K8s cluster

## Next : [01 — Your first catalog](01_first_catalog.md)
