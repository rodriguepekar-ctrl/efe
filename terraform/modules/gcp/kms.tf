resource "google_kms_key_ring" "main" {
  project  = var.project_id
  name     = "${local.prefix}-keyring"
  location = var.region
}

resource "google_kms_crypto_key" "byok" {
  count = var.enable_kms_byok ? 1 : 0

  name            = "${local.prefix}-byok"
  key_ring        = google_kms_key_ring.main.id
  rotation_period = "7776000s" # 90 days

  version_template {
    algorithm        = "GOOGLE_SYMMETRIC_ENCRYPTION"
    protection_level = "SOFTWARE"
  }

  labels = merge(local.common_labels, { purpose = "byok" })

  lifecycle {
    prevent_destroy = false
  }
}

resource "google_kms_crypto_key" "cloudsql" {
  count = var.enable_kms_byok ? 1 : 0

  name            = "${local.prefix}-cloudsql"
  key_ring        = google_kms_key_ring.main.id
  rotation_period = "7776000s"
}

resource "google_kms_crypto_key" "gke_etcd" {
  count = var.enable_kms_byok ? 1 : 0

  name            = "${local.prefix}-gke-etcd"
  key_ring        = google_kms_key_ring.main.id
  rotation_period = "7776000s"
}

resource "google_kms_crypto_key" "gcs_audit" {
  count = var.enable_kms_byok ? 1 : 0

  name            = "${local.prefix}-gcs-audit"
  key_ring        = google_kms_key_ring.main.id
  rotation_period = "7776000s"
}

# Grant Cloud SQL service account decrypt access on cloudsql key.
resource "google_kms_crypto_key_iam_member" "cloudsql_decrypter" {
  count = var.enable_kms_byok ? 1 : 0

  crypto_key_id = google_kms_crypto_key.cloudsql[0].id
  role          = "roles/cloudkms.cryptoKeyEncrypterDecrypter"
  member        = "serviceAccount:service-${data.google_project.current.number}@gcp-sa-cloud-sql.iam.gserviceaccount.com"
}

# Grant GCS service agent decrypt access on audit key.
resource "google_kms_crypto_key_iam_member" "gcs_decrypter" {
  count = var.enable_kms_byok ? 1 : 0

  crypto_key_id = google_kms_crypto_key.gcs_audit[0].id
  role          = "roles/cloudkms.cryptoKeyEncrypterDecrypter"
  member        = "serviceAccount:service-${data.google_project.current.number}@gs-project-accounts.iam.gserviceaccount.com"
}
