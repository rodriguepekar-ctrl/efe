# M31 — High-severity budget alerts pipe to PagerDuty via SNS.

provider "aws" {
  region = var.aws_region
}

resource "aws_sns_topic" "budget_hard" {
  name                        = "efe-budget-hard-alerts"
  kms_master_key_id           = "alias/aws/sns"
  delivery_policy             = file("${path.module}/sns_delivery_policy.json")

  tags = merge(var.tags, {
    "efe:milestone" = "m31"
    "efe:severity"  = "hard"
  })
}

resource "aws_sns_topic_subscription" "pagerduty" {
  topic_arn = aws_sns_topic.budget_hard.arn
  protocol  = "https"
  endpoint  = var.pagerduty_integration_url
}

# Soft alerts go to an email distro instead of paging.
resource "aws_sns_topic" "budget_soft" {
  name = "efe-budget-soft-alerts"
  tags = merge(var.tags, {
    "efe:milestone" = "m31"
    "efe:severity"  = "soft"
  })
}

resource "aws_sns_topic_subscription" "soft_email" {
  topic_arn = aws_sns_topic.budget_soft.arn
  protocol  = "email"
  endpoint  = var.soft_alert_email
}

output "sns_topic_hard_arn" {
  value = aws_sns_topic.budget_hard.arn
}
output "sns_topic_soft_arn" {
  value = aws_sns_topic.budget_soft.arn
}
