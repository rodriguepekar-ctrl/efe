# M31 — Stripe products + prices managed via Terraform.
#
# We use the Stripe Terraform provider so the catalogue stays in
# version control. Plan rows in PG reference these by ID.

terraform {
  required_version = ">= 1.5.0"
  required_providers {
    stripe = {
      source  = "lukasaron/stripe"
      version = "~> 2.3"
    }
  }
}

provider "stripe" {
  api_key = var.stripe_api_key
}

# ── Products ────────────────────────────────────────────────────────
resource "stripe_product" "starter" {
  name = "EFE Starter"
  type = "service"
  metadata = {
    plan_name = "starter"
  }
}

resource "stripe_product" "pro" {
  name = "EFE Pro"
  type = "service"
  metadata = {
    plan_name = "pro"
  }
}

resource "stripe_product" "enterprise" {
  name = "EFE Enterprise"
  type = "service"
  metadata = {
    plan_name = "enterprise"
  }
}

# ── Base subscription prices (recurring) ────────────────────────────
resource "stripe_price" "starter_base" {
  product     = stripe_product.starter.id
  currency    = "usd"
  unit_amount = 4900
  recurring {
    interval = "month"
  }
  metadata = {
    plan_name = "starter"
    component = "base"
  }
}

resource "stripe_price" "pro_base" {
  product     = stripe_product.pro.id
  currency    = "usd"
  unit_amount = 49900
  recurring {
    interval = "month"
  }
  metadata = {
    plan_name = "pro"
    component = "base"
  }
}

# ── Usage-based meter prices (Stripe billing meters API) ────────────
# These are one-shot prices used by `InvoiceItem.create` at month-end.
# We do NOT use Stripe-Native metered billing because our metering is
# done outside Stripe (Redis + PG rollups).
resource "stripe_price" "starter_classify" {
  product     = stripe_product.starter.id
  currency    = "usd"
  unit_amount = 1 # $0.001 / 100 classify requests would be 100 cents ; we sell as $0.001/req hence amount 1 cent w/ tiers
  metadata = {
    plan_name = "starter"
    meter     = "classify_requests"
  }
}

resource "stripe_price" "starter_agent_run_seconds" {
  product     = stripe_product.starter.id
  currency    = "usd"
  unit_amount = 1
  metadata = {
    plan_name = "starter"
    meter     = "agent_run_seconds"
  }
}

resource "stripe_price" "starter_multimodal_vision" {
  product     = stripe_product.starter.id
  currency    = "usd"
  unit_amount = 5
  metadata = {
    plan_name = "starter"
    meter     = "multimodal_vision_requests"
  }
}

output "stripe_product_ids" {
  description = "Stripe Product IDs for each plan (mirror in pricing_plan.stripe_product_id)."
  value = {
    starter    = stripe_product.starter.id
    pro        = stripe_product.pro.id
    enterprise = stripe_product.enterprise.id
  }
}

output "stripe_price_ids" {
  description = "Stripe Price IDs for usage-based meters."
  value = {
    starter_base               = stripe_price.starter_base.id
    starter_classify           = stripe_price.starter_classify.id
    starter_agent_run_seconds  = stripe_price.starter_agent_run_seconds.id
    starter_multimodal_vision  = stripe_price.starter_multimodal_vision.id
    pro_base                   = stripe_price.pro_base.id
  }
}
