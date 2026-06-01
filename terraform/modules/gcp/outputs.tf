output "kubeconfig_command" {
  description = "Command to update local kubeconfig for the GKE cluster."
  value       = "gcloud container clusters get-credentials ${google_container_cluster.primary.name} --region ${var.region} --project ${var.project_id}"
}

output "gke_cluster_name" {
  value = google_container_cluster.primary.name
}

output "gke_cluster_endpoint" {
  value     = google_container_cluster.primary.endpoint
  sensitive = true
}

output "gke_workload_identity_pool" {
  value = "${var.project_id}.svc.id.goog"
}

output "cloudsql_instance_name" {
  value = google_sql_database_instance.efe.name
}

output "cloudsql_private_ip" {
  value = google_sql_database_instance.efe.private_ip_address
}

output "cloudsql_secret_id" {
  description = "Secret Manager secret ID containing Cloud SQL credentials."
  value       = google_secret_manager_secret.cloudsql_credentials.secret_id
}

output "redis_host" {
  value = google_redis_instance.efe.host
}

output "redis_secret_id" {
  value = google_secret_manager_secret.redis_auth.secret_id
}

output "byok_kms_key" {
  description = "KMS BYOK key ID (null when disabled)."
  value       = var.enable_kms_byok ? google_kms_crypto_key.byok[0].id : null
}

output "audit_bucket_name" {
  value = google_storage_bucket.audit.name
}

output "efe_server_workload_identity_sa" {
  value = google_service_account.efe_server.email
}

output "audit_hmac_secret_id" {
  value = google_secret_manager_secret.audit_hmac_seed.secret_id
}

output "vpc_self_link" {
  value = google_compute_network.vpc.self_link
}
