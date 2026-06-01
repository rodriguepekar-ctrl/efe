variable "stripe_api_key" {
  description = "Stripe Secret API key (sk_live_... or sk_test_...). Loaded from secret store."
  type        = string
  sensitive   = true
}

variable "aws_region" {
  description = "AWS region for SNS topics."
  type        = string
  default     = "eu-west-1"
}

variable "pagerduty_integration_url" {
  description = "PagerDuty integration URL for hard-cap alerts."
  type        = string
  sensitive   = true
}

variable "soft_alert_email" {
  description = "Distribution list for soft-cap budget alerts."
  type        = string
  default     = "finops@efe.ai"
}

variable "tags" {
  description = "Common tags."
  type        = map(string)
  default     = {}
}
