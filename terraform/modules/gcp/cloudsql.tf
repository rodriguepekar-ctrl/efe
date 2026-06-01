resource "random_password" "cloudsql" {
  length  = 48
  special = true
}

resource "google_sql_database_instance" "efe" {
  project          = var.project_id
  name             = "${local.prefix}-postgres"
  database_version = "POSTGRES_16"
  region           = var.region

  depends_on = [google_service_networking_connection.private_vpc_connection]

  settings {
    tier              = var.cloudsql_tier
    availability_type = "REGIONAL" # HA — primary + standby in different zones.
    disk_size         = var.cloudsql_disk_size_gb
    disk_type         = "PD_SSD"
    disk_autoresize   = true

    backup_configuration {
      enabled                        = true
      start_time                     = "02:00"
      point_in_time_recovery_enabled = true
      transaction_log_retention_days = 7
      backup_retention_settings {
        retained_backups = 14
        retention_unit   = "COUNT"
      }
    }

    ip_configuration {
      ipv4_enabled                                  = false
      private_network                               = google_compute_network.vpc.id
      enable_private_path_for_google_cloud_services = true
      # Force TLS for all client connections (no plaintext fallback).
      ssl_mode = "ENCRYPTED_ONLY"
    }

    insights_config {
      query_insights_enabled  = true
      query_string_length     = 1024
      record_application_tags = true
      record_client_address   = false
    }

    database_flags {
      name  = "log_min_duration_statement"
      value = "1000"
    }
    database_flags {
      name  = "log_statement"
      value = "ddl"
    }
    database_flags {
      name  = "log_connections"
      value = "on"
    }
    # Audit + ops visibility flags (tfsec recommended).
    database_flags {
      name  = "log_temp_files"
      value = "0"
    }
    database_flags {
      name  = "log_lock_waits"
      value = "on"
    }
    database_flags {
      name  = "log_disconnections"
      value = "on"
    }
    database_flags {
      name  = "log_checkpoints"
      value = "on"
    }

    user_labels = local.common_labels
  }

  encryption_key_name = var.enable_kms_byok ? google_kms_crypto_key.cloudsql[0].id : null

  deletion_protection = var.cloudsql_deletion_protection
}

resource "google_sql_database" "efe" {
  project  = var.project_id
  name     = "efe"
  instance = google_sql_database_instance.efe.name
}

resource "google_sql_user" "admin" {
  project  = var.project_id
  name     = "efe_admin"
  instance = google_sql_database_instance.efe.name
  password = random_password.cloudsql.result
}
