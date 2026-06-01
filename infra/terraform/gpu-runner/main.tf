// Sprint Φ-D-LATACT D90 — Self-hosted GPU runner for CI GPU benches.
// Cf. docs/EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md § 4.
//
// Provisions a pool of g4dn.xlarge spot instances (= NVIDIA T4, 16 GB
// VRAM) running the GitHub Actions self-hosted runner. Auto-scaling 0..3
// via the official github-actions-runner-controller.

terraform {
  required_version = ">= 1.5.0"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
  }
}

variable "region" {
  type    = string
  default = "us-east-1"
}

variable "runner_count" {
  type        = number
  default     = 2
  description = "Number of GPU runners to provision (= spare for spot termination)"
}

variable "github_org" {
  type    = string
  default = "efe-ai"
}

variable "github_token" {
  type      = string
  sensitive = true
}

provider "aws" {
  region = var.region
}

// AMI : AWS Deep Learning AMI (= pre-installed CUDA 12.4, NVIDIA driver, Rust toolchain).
data "aws_ami" "deep_learning_gpu" {
  most_recent = true
  owners      = ["amazon"]
  filter {
    name   = "name"
    values = ["Deep Learning AMI GPU PyTorch * (Ubuntu 22.04) *"]
  }
}

resource "aws_iam_role" "runner" {
  name = "efe-gpu-runner-role"
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect = "Allow"
      Principal = { Service = "ec2.amazonaws.com" }
      Action = "sts:AssumeRole"
    }]
  })
}

resource "aws_iam_instance_profile" "runner" {
  name = "efe-gpu-runner-profile"
  role = aws_iam_role.runner.name
}

resource "aws_security_group" "runner" {
  name        = "efe-gpu-runner-sg"
  description = "GPU runner egress for GitHub Actions"
  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}

resource "aws_spot_instance_request" "gpu_runner" {
  count = var.runner_count

  ami                    = data.aws_ami.deep_learning_gpu.id
  instance_type          = "g4dn.xlarge"  // NVIDIA T4 16 GB, 4 vCPU, 16 GB RAM
  spot_price             = "0.20"          // < on-demand 0.526
  spot_type              = "persistent"
  wait_for_fulfillment   = true
  vpc_security_group_ids = [aws_security_group.runner.id]
  iam_instance_profile   = aws_iam_instance_profile.runner.name

  user_data = templatefile("${path.module}/userdata.sh.tftpl", {
    github_org   = var.github_org
    github_token = var.github_token
    runner_name  = "efe-gpu-runner-${count.index}"
    runner_labels = "self-hosted,gpu-t4,linux,x64"
  })

  root_block_device {
    volume_size = 50    // GB ; cargo cache + CUDA caches
    volume_type = "gp3"
  }

  tags = {
    Name        = "efe-gpu-runner-${count.index}"
    Sprint      = "Phi-D-LATACT"
    Purpose     = "github-actions-gpu-bench"
    Provisioned = "terraform"
  }
}

output "runner_instance_ids" {
  value = [for r in aws_spot_instance_request.gpu_runner : r.spot_instance_id]
}

output "estimated_monthly_cost_usd" {
  value       = var.runner_count * 0.20 * 24 * 30 * 0.30  // 30 % utilization
  description = "Approx monthly cost ($0.20/h × runners × 30%% util)"
}
