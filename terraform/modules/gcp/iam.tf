# GKE node default service account (least privilege).
resource "google_service_account" "gke_node" {
  project      = var.project_id
  account_id   = "${local.prefix}-gke-node"
  display_name = "GKE node SA for ${local.prefix}"
}

resource "google_project_iam_member" "gke_node_logging" {
  project = var.project_id
  role    = "roles/logging.logWriter"
  member  = "serviceAccount:${google_service_account.gke_node.email}"
}

resource "google_project_iam_member" "gke_node_metrics" {
  project = var.project_id
  role    = "roles/monitoring.metricWriter"
  member  = "serviceAccount:${google_service_account.gke_node.email}"
}

resource "google_project_iam_member" "gke_node_trace" {
  project = var.project_id
  role    = "roles/cloudtrace.agent"
  member  = "serviceAccount:${google_service_account.gke_node.email}"
}

# efe-server Workload Identity SA (mapped to K8s SA "efe:efe-server").
resource "google_service_account" "efe_server" {
  project      = var.project_id
  account_id   = "${local.prefix}-efe-server"
  display_name = "efe-server WI SA for ${local.prefix}"
}

resource "google_service_account_iam_member" "efe_server_wi" {
  service_account_id = google_service_account.efe_server.id
  role               = "roles/iam.workloadIdentityUser"
  member             = "serviceAccount:${var.project_id}.svc.id.goog[efe/efe-server]"
}

# efe-server can use BYOK key.
resource "google_kms_crypto_key_iam_member" "efe_server_byok" {
  count = var.enable_kms_byok ? 1 : 0

  crypto_key_id = google_kms_crypto_key.byok[0].id
  role          = "roles/cloudkms.cryptoKeyEncrypterDecrypter"
  member        = "serviceAccount:${google_service_account.efe_server.email}"
}

# efe-server can write to GCS audit bucket.
resource "google_storage_bucket_iam_member" "efe_server_audit_writer" {
  bucket = google_storage_bucket.audit.name
  role   = "roles/storage.objectCreator"
  member = "serviceAccount:${google_service_account.efe_server.email}"
}

# efe-server can read its own Secret Manager entries.
resource "google_secret_manager_secret_iam_member" "efe_server_db_secret_reader" {
  project   = var.project_id
  secret_id = google_secret_manager_secret.cloudsql_credentials.secret_id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.efe_server.email}"
}

resource "google_secret_manager_secret_iam_member" "efe_server_hmac_seed_reader" {
  project   = var.project_id
  secret_id = google_secret_manager_secret.audit_hmac_seed.secret_id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.efe_server.email}"
}
