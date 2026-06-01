resource "google_redis_instance" "efe" {
  project        = var.project_id
  name           = "${local.prefix}-redis"
  region         = var.region
  tier           = var.redis_tier
  memory_size_gb = var.redis_memory_size_gb

  authorized_network = google_compute_network.vpc.id
  connect_mode       = "PRIVATE_SERVICE_ACCESS"
  redis_version      = "REDIS_7_0"

  auth_enabled            = true
  transit_encryption_mode = "SERVER_AUTHENTICATION"

  persistence_config {
    persistence_mode    = "RDB"
    rdb_snapshot_period = "ONE_HOUR"
  }

  redis_configs = {
    "maxmemory-policy" = "noeviction"
  }

  labels = local.common_labels

  depends_on = [google_service_networking_connection.private_vpc_connection]
}

# NOTE : Memorystore Redis Standard does NOT support Redis Stack modules
# (vector search HNSW). Tenants enabling Mode-3 vector search MUST set
# `redis-stack.enabled=true` in Helm chart values to use the in-cluster
# StatefulSet instead. See README.md for the full constraint matrix.
