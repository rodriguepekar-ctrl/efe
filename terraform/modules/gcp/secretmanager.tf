resource "google_secret_manager_secret" "cloudsql_credentials" {
  project   = var.project_id
  secret_id = "${local.prefix}-cloudsql-credentials"

  replication {
    auto {
      dynamic "customer_managed_encryption" {
        for_each = var.enable_kms_byok ? [1] : []
        content {
          kms_key_name = google_kms_crypto_key.byok[0].id
        }
      }
    }
  }

  labels = local.common_labels
}

resource "google_secret_manager_secret_version" "cloudsql_credentials" {
  secret = google_secret_manager_secret.cloudsql_credentials.id
  secret_data = jsonencode({
    username          = google_sql_user.admin.name
    password          = random_password.cloudsql.result
    host              = google_sql_database_instance.efe.private_ip_address
    port              = 5432
    dbname            = "efe"
    connection_string = "postgresql://${google_sql_user.admin.name}:${urlencode(random_password.cloudsql.result)}@${google_sql_database_instance.efe.private_ip_address}:5432/efe?sslmode=require"
  })
}

resource "random_password" "audit_hmac_seed" {
  length  = 64
  special = false
}

resource "google_secret_manager_secret" "audit_hmac_seed" {
  project   = var.project_id
  secret_id = "${local.prefix}-audit-hmac-seed"

  replication {
    auto {}
  }

  labels = merge(local.common_labels, {
    purpose     = "audit-chain-hmac"
    criticality = "p0"
  })
}

resource "google_secret_manager_secret_version" "audit_hmac_seed" {
  secret      = google_secret_manager_secret.audit_hmac_seed.id
  secret_data = random_password.audit_hmac_seed.result

  lifecycle {
    ignore_changes = [secret_data]
  }
}

resource "google_secret_manager_secret" "redis_auth" {
  project   = var.project_id
  secret_id = "${local.prefix}-redis-auth"

  replication {
    auto {}
  }

  labels = local.common_labels
}

resource "google_secret_manager_secret_version" "redis_auth" {
  secret = google_secret_manager_secret.redis_auth.id
  secret_data = jsonencode({
    auth_token        = google_redis_instance.efe.auth_string
    host              = google_redis_instance.efe.host
    port              = google_redis_instance.efe.port
    connection_string = "redis://:${urlencode(google_redis_instance.efe.auth_string)}@${google_redis_instance.efe.host}:${google_redis_instance.efe.port}"
  })
}
