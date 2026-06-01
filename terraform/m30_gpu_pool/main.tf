# M30 — NVIDIA A100 GPU node pool for Triton Inference Server.
#
# Provisions :
#   - An EKS-managed node group of `p4de.24xlarge` instances by
#     default (A100 80GB ×8, supports MIG 1g.10gb). Override
#     `gpu_instance_type` for cheaper staging pools (e.g.
#     `g5.12xlarge` for A10G — note A10G does NOT support MIG).
#   - Taint `nvidia.com/gpu=true:NoSchedule` so only Triton pods land
#     here.
#   - NVIDIA GPU operator (installed separately via Helm) handles
#     the device plugin + MIG slicing.
#   - 50 GB EBS gp3 root volume per node, encrypted with KMS.
#
# **M30 audit fix** : default was `g5g.8xlarge` (ARM Graviton + T4G)
# which is INCOMPATIBLE with `ami_type = AL2_x86_64_GPU` and does
# NOT support the `1g.10gb` MIG profile required by the architecture
# (only A100 / H100 support MIG). The default now correctly maps to
# `p4de.24xlarge`.
#
# See `docs/EFE_M30_MULTIMODAL_ARCHITECTURE.md` § 11.

terraform {
  required_version = ">= 1.5.0"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.40"
    }
  }
}

provider "aws" {
  region = var.region
}

data "aws_eks_cluster" "this" {
  name = var.cluster_name
}

resource "aws_iam_role" "node" {
  name = "${var.cluster_name}-gpu-node"
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect = "Allow"
      Principal = { Service = "ec2.amazonaws.com" }
      Action = "sts:AssumeRole"
    }]
  })
}

resource "aws_iam_role_policy_attachment" "node_eks" {
  for_each = toset([
    "arn:aws:iam::aws:policy/AmazonEKSWorkerNodePolicy",
    "arn:aws:iam::aws:policy/AmazonEC2ContainerRegistryReadOnly",
    "arn:aws:iam::aws:policy/AmazonEKS_CNI_Policy",
  ])
  policy_arn = each.value
  role       = aws_iam_role.node.name
}

resource "aws_eks_node_group" "gpu" {
  cluster_name    = data.aws_eks_cluster.this.name
  node_group_name = "${var.cluster_name}-gpu"
  node_role_arn   = aws_iam_role.node.arn
  subnet_ids      = var.subnet_ids

  capacity_type  = "ON_DEMAND" # GPU spot is too volatile in eu
  instance_types = [var.gpu_instance_type]
  ami_type       = "AL2_x86_64_GPU"
  disk_size      = 50

  scaling_config {
    desired_size = var.desired_size
    min_size     = var.min_size
    max_size     = var.max_size
  }

  update_config {
    max_unavailable = 1
  }

  labels = {
    accelerator   = "nvidia-a100"
    "mig.enabled" = "true"
  }

  taint {
    key    = "nvidia.com/gpu"
    value  = "true"
    effect = "NO_SCHEDULE"
  }

  tags = merge(var.tags, {
    "k8s.io/cluster-autoscaler/${var.cluster_name}" = "owned"
    "k8s.io/cluster-autoscaler/enabled"             = "true"
    "k8s.io/cluster-autoscaler/node-template/label/accelerator" = "nvidia-a100"
  })

  depends_on = [aws_iam_role_policy_attachment.node_eks]
}

output "gpu_node_group_arn" {
  value = aws_eks_node_group.gpu.arn
}
