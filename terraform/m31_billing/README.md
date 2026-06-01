# M31 billing Terraform module

Provisions :

- **Stripe products + prices** via the Stripe Terraform provider.
- **AWS SNS topics** for hard / soft budget alert delivery.
- Outputs the Stripe IDs to be mirrored into PG (`pricing_plan.stripe_product_id`,
  `pricing_plan_meter.stripe_price_id`).

## Usage

```hcl
module "m31_billing" {
  source                    = "./terraform/m31_billing"
  stripe_api_key            = var.stripe_secret_key  # from AWS Secrets Manager
  pagerduty_integration_url = var.pd_url
  soft_alert_email          = "finops@efe.ai"
  tags                      = { env = "prod" }
}

output "stripe_ids" {
  value = module.m31_billing
}
```

## Outputs

| Output | Description |
|---|---|
| `stripe_product_ids` | One per plan ; mirror in `pricing_plan.stripe_product_id`. |
| `stripe_price_ids` | One per (plan, meter) ; mirror in `pricing_plan_meter.stripe_price_id`. |
| `sns_topic_hard_arn` | PagerDuty pipe. |
| `sns_topic_soft_arn` | Email distro. |
