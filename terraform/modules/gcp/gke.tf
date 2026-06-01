resource "google_container_cluster" "primary" {
  project  = var.project_id
  name     = "${local.prefix}-gke"
  location = var.region

  release_channel {
    channel = var.gke_release_channel
  }

  network    = google_compute_network.vpc.id
  subnetwork = google_compute_subnetwork.private.id

  ip_allocation_policy {
    cluster_secondary_range_name  = "pods"
    services_secondary_range_name = "services"
  }

  private_cluster_config {
    enable_private_nodes    = true
    enable_private_endpoint = false
    master_ipv4_cidr_block  = "172.16.0.0/28"
  }

  master_authorized_networks_config {
    dynamic "cidr_blocks" {
      for_each = var.gke_master_authorized_networks
      content {
        cidr_block   = cidr_blocks.value
        display_name = "authorized-${index(var.gke_master_authorized_networks, cidr_blocks.value)}"
      }
    }
  }

  workload_identity_config {
    workload_pool = "${var.project_id}.svc.id.goog"
  }

  addons_config {
    horizontal_pod_autoscaling { disabled = false }
    http_load_balancing { disabled = false }
    network_policy_config { disabled = false }
  }

  network_policy {
    enabled  = true
    provider = "CALICO"
  }

  database_encryption {
    state    = var.enable_kms_byok ? "ENCRYPTED" : "DECRYPTED"
    key_name = var.enable_kms_byok ? google_kms_crypto_key.gke_etcd[0].id : null
  }

  logging_service    = "logging.googleapis.com/kubernetes"
  monitoring_service = "monitoring.googleapis.com/kubernetes"

  binary_authorization {
    evaluation_mode = "DISABLED"
  }

  remove_default_node_pool = true
  initial_node_count       = 1

  resource_labels = local.common_labels

  deletion_protection = var.environment == "prod"

  depends_on = [
    google_service_networking_connection.private_vpc_connection,
  ]
}

resource "google_container_node_pool" "primary" {
  project  = var.project_id
  name     = "${local.prefix}-primary"
  location = var.region
  cluster  = google_container_cluster.primary.name

  autoscaling {
    min_node_count  = 3
    max_node_count  = 12
    location_policy = "BALANCED"
  }

  management {
    auto_repair  = true
    auto_upgrade = true
  }

  upgrade_settings {
    max_surge       = 1
    max_unavailable = 0
  }

  node_config {
    machine_type = var.gke_node_machine_type
    disk_size_gb = 100
    disk_type    = "pd-balanced"

    service_account = google_service_account.gke_node.email
    oauth_scopes    = ["https://www.googleapis.com/auth/cloud-platform"]

    # Disable legacy metadata endpoints (tfsec aws-gke-metadata-endpoints-disabled).
    metadata = {
      disable-legacy-endpoints = "true"
    }

    workload_metadata_config {
      mode = "GKE_METADATA"
    }

    shielded_instance_config {
      enable_secure_boot          = true
      enable_integrity_monitoring = true
    }

    labels = local.common_labels
  }
}
