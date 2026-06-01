resource "google_storage_bucket" "audit" {
  project  = var.project_id
  name     = "${local.prefix}-audit"
  location = var.region

  storage_class               = "STANDARD"
  uniform_bucket_level_access = true
  public_access_prevention    = "enforced"

  versioning {
    enabled = true
  }

  dynamic "encryption" {
    for_each = var.enable_kms_byok ? [1] : []
    content {
      default_kms_key_name = google_kms_crypto_key.gcs_audit[0].id
    }
  }

  lifecycle_rule {
    condition {
      age = 90
    }
    action {
      type          = "SetStorageClass"
      storage_class = "NEARLINE"
    }
  }

  lifecycle_rule {
    condition {
      age = 365
    }
    action {
      type          = "SetStorageClass"
      storage_class = "COLDLINE"
    }
  }

  lifecycle_rule {
    condition {
      age = var.audit_bucket_retention_days
    }
    action {
      type = "Delete"
    }
  }

  retention_policy {
    retention_period = var.audit_bucket_retention_days * 86400
    is_locked        = false
  }

  logging {
    log_bucket        = google_storage_bucket.audit_access_log.name
    log_object_prefix = "audit-access/"
  }

  labels = merge(local.common_labels, { purpose = "audit-archive" })

  depends_on = [google_kms_crypto_key_iam_member.gcs_decrypter]
}

resource "google_storage_bucket" "audit_access_log" {
  project                     = var.project_id
  name                        = "${local.prefix}-audit-access-log"
  location                    = var.region
  storage_class               = "STANDARD"
  uniform_bucket_level_access = true
  public_access_prevention    = "enforced"

  lifecycle_rule {
    condition {
      age = 90
    }
    action {
      type = "Delete"
    }
  }

  labels = merge(local.common_labels, { purpose = "gcs-access-log" })
}
