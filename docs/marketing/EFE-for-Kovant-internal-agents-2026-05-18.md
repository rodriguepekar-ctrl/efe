# EFE for Kovant — Internal Agent Deployment Plan

**Date drafted** : 2026-05-18
**Author**       : Rodrigue Pekar
**Audience**     : Kovant — for the pitch on Thursday 21 May 2026
**Purpose**      : Beyond the investment thesis, demonstrate concrete value by
                   proposing EFE as the infrastructure layer for Kovant's
                   own internal AI agents. The "double-down" proposition :
                   Kovant invests AND becomes anchor customer + design partner.

---

## The proposition in one paragraph

> **Kovant deploys 5-7 internal AI agents on EFE. We co-design them
> together over a 6-month free pilot. Kovant gets full-stack
> investment-firm-grade automation (deal flow, DD, portfolio
> monitoring, LP reporting) with by-construction audit trail.  EFE
> gets its first vertical-specific reference customer in the Swiss
> investment firm segment + a case study for the next 50 Swiss
> PE / VC / Family Office prospects.**

This is not a free service offer. This is a structured design partnership
with a defined commercial path after the pilot.

---

## Why Kovant specifically

Most VCs use AI tools the same way (notebooks + ChatGPT + maybe Harvey
for legal). The problem is that **investment firms have unique compliance
constraints that off-the-shelf tools fail** :

| Compliance constraint | Off-the-shelf gap | EFE-native |
|----------------------|-------------------|------------|
| LP confidentiality (per-LP segregation) | ChatGPT cross-contaminates contexts | Multi-tenant Firecracker microVM per LP/portfolio |
| Investment decision reproducibility (audit by FINMA/AMLA) | No audit trail in Notion or Linear | HMAC audit chain on every agent decision |
| GDPR for EU LPs | Cloud LLMs send data to US providers | EU-resident LLM routing + DPIA gating |
| Conflict of interest screening | Manual today | Step approval workflow with human-in-the-loop |
| Quarterly LP reporting reproducibility | Re-generated from scratch, drift | Signed quarterly evidence (cosign + Rekor) |
| Cost control on LLM usage | No cap, surprise bills | Per-agent + per-tenant LLM cost caps |

These are not theoretical — they are exactly what **DORA (entered in force
17 January 2025)** demands from regulated financial entities and what
**FINMA** will scrutinize in 2026-2027 audits of investment firms using AI.

## The 7 agents proposed for Kovant

### Agent 1 — Deal flow intake

**What it does** : every incoming pitch deck, email, intro request is
ingested. The agent extracts structured fields (founder, sector, stage,
ask, valuation) into the Kovant CRM and produces a 1-paragraph
"first-look" summary for the partner team.

**Why EFE matters** :
- Multi-tenant : each founder's data is isolated; cannot leak to
  another deal
- Audit trail : when Kovant says "we rejected at first look", the
  founder can request the reasoning (GDPR Art. 22 — automated
  decisions)
- Step approval : never auto-rejects without partner sign-off

**LLM cost estimate** : ~ 0.50 € / deck × 500 decks per quarter = 1,000 €/quarter.

### Agent 2 — Due diligence reader

**What it does** : reads the data room (financials, contracts, term
sheets, board minutes) and produces a structured DD memo. Identifies
red flags (cap-table issues, IP risk, governance) and cross-references
against Kovant's existing portfolio for conflicts.

**Why EFE matters** :
- Strict data room isolation per deal (Firecracker microVM)
- Cost cap on LLM usage (a thorough DD can otherwise cost 200-500 € in tokens)
- Audit trail for the DD process (defensible to LPs if a deal goes bad)
- Step approval : the final DD memo requires partner approval before
  going to IC

**LLM cost estimate** : ~ 50-200 € / deal × 20-40 deep DDs per quarter = ~ 2-8k €/quarter.

### Agent 3 — Portfolio monitoring

**What it does** : monitors all 30-50 portfolio companies daily.
Reads incoming reports, news, social signals. Produces a weekly
"red/amber/green" rollup for the partner team. Flags companies that
need attention (burn rate spike, key hire leaving, lawsuit filed).

**Why EFE matters** :
- Meta-swarm : one agent per portfolio company, running in parallel,
  isolated per company (no cross-contamination of insights)
- Cost cap : prevents runaway costs when news cycles spike
- Audit trail : every flag has a traced reason (so a flagged founder
  can be told why honestly)
- DPIA gating : sensitive data (HR, founder personal info) is
  redacted before LLM exposure (PII redaction module M20)

**LLM cost estimate** : ~ 5-20 €/company/week × 40 companies × 13 weeks = ~ 2.6-10k €/quarter.

### Agent 4 — LP quarterly reporting

**What it does** : auto-generates the LP quarterly report from
portfolio data + IRR calcs + market commentary. Produces draft
narrative sections for each portfolio company.

**Why EFE matters** :
- Signed quarterly evidence : the report carries a cosign signature
  + Rekor entry, so LPs can verify the report was not tampered with
  post-publication
- Multi-LP segregation : different LP cohorts (different funds,
  jurisdictions) get tailored reports without cross-leakage
- Reproducibility : the report from Q3 2026 can be re-generated
  byte-identical in 2032 if LPs request audit
- Compliance gating : DPIA enforced on LP data, AI Act high-risk
  classification respected

**LLM cost estimate** : ~ 500-2000 €/quarter × 4 = 2-8k €/year (low compute, high value).

### Agent 5 — Market intelligence

**What it does** : continuous research on 5-10 sectors of strategic
interest. Produces a weekly briefing for the partner team. Identifies
emerging companies before they hit the standard radar.

**Why EFE matters** :
- Cost cap : research agents are notorious for runaway costs
- Closed-loop active learning (M24) : the agent learns which sectors
  the partners actually act on and prioritizes accordingly
- Audit trail : when Kovant says "we'd been tracking this sector for
  6 months", there is provable evidence

**LLM cost estimate** : ~ 100-500 €/week = 5-25k €/year.

### Agent 6 — Conflict of interest screening

**What it does** : every new deal, every co-investor, every board
appointment is screened against the existing portfolio + Kovant
partners' personal holdings + known relationships. Produces a
conflict report.

**Why EFE matters** :
- This is high-stakes (a missed conflict = LP litigation)
- Audit trail : when Kovant signs an LP saying "we have no conflict",
  the agent's screening output is evidence
- Step approval : any conflict above a threshold blocks the deal
  until partner override (which is itself audited)
- Multi-tenant : partners' personal holdings are isolated from
  the deal-team agents

**LLM cost estimate** : low compute, high importance. ~ 200 €/quarter.

### Agent 7 — Compliance + LP onboarding (KYC/AML)

**What it does** : KYC/AML checks on new LPs. Reads onboarding
documents, cross-references sanctions lists, generates a structured
compliance memo for the compliance officer.

**Why EFE matters** :
- This is FINMA-regulated territory : audit trail is mandatory
- DPIA gating : LP personal data handled with explicit consent
  documentation
- Step approval : final decision always human
- Reproducibility : 7-year retention obligation under Swiss AMLA

**LLM cost estimate** : ~ 100 € / LP onboarding × 20 LPs per year = 2k €/year.

---

## How it deploys at Kovant

### Phase 0 — Initial setup (weeks 1-2)

- Helm chart deployment to Kovant's chosen cluster (Swiss cloud :
  exoscale, Swisscom, or on-prem)
- Per-tenant identity provisioning (one tenant = one portfolio
  company + Kovant operations tenant + LP tenant)
- LLM provider integration (Kovant chooses : Anthropic API, Azure
  OpenAI EU, or local self-hosted)
- Initial cost caps + step approval workflows configured

### Phase 1 — Pilot agents (months 1-3)

Deploy 3 of the 7 agents in production-like mode :
- Agent 1 (Deal flow intake) — high volume, low risk
- Agent 3 (Portfolio monitoring) — proves multi-tenant value
- Agent 5 (Market intelligence) — proves cost governance

Measure : time saved per partner per week, LLM cost actual vs cap,
audit-trail completeness.

### Phase 2 — High-stakes agents (months 4-6)

Deploy the 4 remaining agents :
- Agent 2 (Due diligence) — high-value, partner-facing
- Agent 4 (LP reporting) — compliance-critical
- Agent 6 (Conflict screening) — audit-critical
- Agent 7 (KYC/AML) — FINMA-relevant

Measure : full quarter end-to-end with all agents running, LP
report signed, conflict screening clean, KYC/AML defensible.

### Phase 3 — Commercial transition (month 7+)

After 6 months of free pilot, transition to commercial license :
- Enterprise tier : EUR 500 k - 1.5 M / year depending on usage
- Kovant becomes the reference customer for the Swiss investment
  firm vertical
- EFE uses Kovant case study (with permission) for next 50 prospects

---

## The economics for Kovant

### Without EFE (status quo)

| Cost item | Annual cost |
|-----------|-------------|
| Manual deal screening (1 associate × 50 % time) | ~ 75 k CHF |
| Manual DD reading (1 senior × 30 % time) | ~ 90 k CHF |
| Manual portfolio monitoring (1 associate × 100 %) | ~ 120 k CHF |
| Manual LP reporting (1 finance × 25 % time) | ~ 30 k CHF |
| Manual market intel (1 associate × 25 % time) | ~ 45 k CHF |
| Manual KYC/AML | ~ 30 k CHF |
| LP litigation provision (1 missed conflict = 1-5 M CHF) | risk-weighted ~ 50 k CHF |
| **Sub-total people + risk** | **~ 440 k CHF / year** |

### With EFE (post-pilot, commercial)

| Cost item | Annual cost |
|-----------|-------------|
| EFE Enterprise license | ~ 600 k CHF |
| Reduced manual time (50 % saved across 5 roles) | ~ 200 k CHF saved |
| LLM API costs (with cost cap) | ~ 30-50 k CHF |
| Reduced litigation risk (full audit trail) | ~ 30 k CHF saved |
| **Net cost** | **~ 400-450 k CHF / year** |
| **vs status quo** | **Break-even on direct cost, big upside on compliance posture** |

The dollar math is break-even.  **The real value is** :
1. **Compliance posture** — defensible to FINMA + LPs in 2026-2030
2. **Speed** — agent decisions in seconds, not days
3. **Quality** — never miss a portfolio company red flag
4. **Defensibility** — every decision auditable for 7+ years

---

## The pilot offer (concrete terms)

### What Kovant gets

1. **6 months free** deployment + integration support
2. **7 agents** deployed end-to-end (3 in Phase 1, 4 in Phase 2)
3. **Direct access to the founder** (me) for setup, training, escalations
4. **Co-design of agent prompts + workflows** specific to Kovant operations
5. **Helm chart + documentation** for self-managed evolution post-pilot
6. **No commitment to commercial subscription** during pilot

### What EFE gets

1. **Design partner status** — Kovant validates the Swiss investment
   firm vertical
2. **Case study** (with permission, post-pilot) for next 50 prospects
3. **Feedback loop** to harden the product for Swiss PE/VC/Family Office
4. **Pre-seed lead investment** from Kovant (if Kovant decides to
   invest after the pitch)
5. **Reference call** rights — Kovant agrees to take 2-3 reference
   calls per quarter from EFE prospects

### What changes after 6 months

Two paths :

**Path A — Continue commercially** :
- Enterprise license (EUR 500 k - 1.5 M / year depending on agent
  surface, tenant count, LLM volume)
- Multi-year commitment discount available

**Path B — Discontinue** :
- Kovant exits at end of pilot, no penalty
- Helm chart + docs remain for Kovant's free use (open core)
- EFE gains the case study + validation regardless

---

## Risk mitigation for Kovant

| Risk | Mitigation |
|------|------------|
| EFE is solo founder — bus factor | Helm chart + open code architecture means Kovant can self-host indefinitely even if EFE dissolves |
| LLM cost overrun during pilot | Hard cost caps configured upfront; Kovant approves any cap increase |
| Sensitive deal data exposed | Firecracker microVM isolation + on-prem deployment option + DPIA gating |
| Pilot disrupts Kovant operations | Each agent deployed as net-additive (not replacing humans); humans can override / disable any agent in 1 click |
| Agent makes a bad decision that affects investment outcome | All agents have step approval workflows on high-stakes outputs; human partner always sees + can override |

---

## The strategic narrative for the pitch

**Phrase d'accroche (slide bonus or Q&A response)** :

> *"You're not just an investor — you're also exactly the type of
> customer EFE is built for : a Swiss financial entity with
> compliance constraints, LP obligations, and AI-curious leadership.
>
> I'd like to propose that Kovant becomes EFE's first design partner.
> 6 months free pilot. 7 internal agents deployed end-to-end. If it
> works, you become our anchor customer and reference. If it
> doesn't, you've still gained the helm chart + a 6-month head
> start on internal AI infrastructure.
>
> The investment thesis and the customer thesis are the same : EFE
> wins the Swiss investment-firm vertical, Kovant gets a competitive
> edge that 95 % of Swiss VCs / family offices don't have yet."*

This positioning does 3 things at once :

1. **Demonstrates value** with a use case Kovant understands intimately
   (their own operations)
2. **Shows founder thoughtfulness** — you've thought about them as a
   customer, not just a funder
3. **De-risks the investment** — if Kovant invests, they get
   immediate value, not just a 5-7 year wait for exit

---

## Why this works for Kovant specifically (vs other VCs)

| Argument | Kovant fit |
|----------|------------|
| Swiss-based investor + Swiss-based founder | Local trust, regulatory alignment, cultural fit |
| Kovant's ticket size (typically pre-seed/seed) | EFE's stage matches perfectly |
| Kovant's portfolio companies (likely in regulated sectors) | Each portfolio company becomes a potential EFE customer too |
| Kovant's LP base (European institutionals) | The same LPs that demand DORA-compliant investments |
| Kovant's investment thesis (tech enabling compliance/efficiency) | EFE is exactly that thesis |

---

## Bottom line

This is not a "free trial to win the deal" gimmick.  It is a
structured 6-month design partnership with :
- **Defined deliverables** (7 agents in production)
- **Defined success metrics** (time saved, cost vs budget, audit completeness)
- **Defined commercial path** (commercial license after pilot)
- **Defined exit** (Kovant can walk away with no penalty)

It demonstrates exactly the kind of thinking a regulated industry
buyer needs : structured, defensible, no surprises.

If Kovant accepts, we have **simultaneously** :
1. Closed the pre-seed investment
2. Signed our first design partner
3. Validated the Swiss investment firm vertical
4. Created a case study for the next 50 prospects

If Kovant declines the pilot but invests anyway : great, we still
have the capital to pursue Pictet / Lombard Odier / Kudelski / Roche
as design partners.

If Kovant declines both : we've still demonstrated thoughtfulness +
shown a concrete use case + given Kovant a reference frame for
future discussions.

**All three outcomes are positive.**

---

_End of proposal._

_For the pitch on 21 May 2026 — this can be referenced in the Q&A
section ("would you be interested in piloting EFE internally ?") or
incorporated as a 13th slide in the deck just before the Ask slide._
