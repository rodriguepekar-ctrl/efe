#!/usr/bin/env bash
# scripts/post-terraform-helm-install.sh
#
# Post-Terraform helper that wires up Kubernetes secrets from AWS
# Secrets Manager and runs `helm install` with the right values.
#
# Usage :
#   ./scripts/post-terraform-helm-install.sh \
#     --region eu-west-1 \
#     --tenant acme \
#     --domain efe.acme.com \
#     --terraform-dir terraform/aws

set -euo pipefail

usage() {
  cat <<EOF
Usage: $0 --region REGION --tenant TENANT --domain DOMAIN [--terraform-dir DIR]

Required :
  --region        AWS region (e.g. eu-west-1)
  --tenant        Tenant slug (e.g. acme)
  --domain        Public domain (e.g. efe.acme.com)

Optional :
  --terraform-dir Path to terraform stack root (default: terraform/aws)
  --release       Helm release name (default: efe-stack)
  --namespace     K8s namespace (default: efe)
EOF
  exit 1
}

REGION=""
TENANT=""
DOMAIN=""
TF_DIR="terraform/aws"
RELEASE="efe-stack"
NAMESPACE="efe"

while [[ $# -gt 0 ]]; do
  case $1 in
    --region)         REGION="$2"; shift 2;;
    --tenant)         TENANT="$2"; shift 2;;
    --domain)         DOMAIN="$2"; shift 2;;
    --terraform-dir)  TF_DIR="$2"; shift 2;;
    --release)        RELEASE="$2"; shift 2;;
    --namespace)      NAMESPACE="$2"; shift 2;;
    -h|--help)        usage;;
    *) echo "Unknown option: $1"; usage;;
  esac
done

[[ -n "$REGION" && -n "$TENANT" && -n "$DOMAIN" ]] || usage

echo "🔐 Reading Terraform outputs from $TF_DIR..."
DB_SECRET_ARN=$(cd "$TF_DIR" && terraform output -raw rds_secret_arn)
REDIS_SECRET_ARN=$(cd "$TF_DIR" && terraform output -raw redis_secret_arn)
BYOK_KEY_ARN=$(cd "$TF_DIR" && terraform output -raw byok_kms_key_arn 2>/dev/null || echo "")
IRSA_ARN=$(cd "$TF_DIR" && terraform output -raw efe_server_irsa_role_arn)
EKS_CLUSTER=$(cd "$TF_DIR" && terraform output -raw eks_cluster_name)
HMAC_SECRET_ARN=$(cd "$TF_DIR" && terraform output -raw audit_hmac_secret_arn)
# audit_bucket_name is exported by Terraform but consumed via IAM IRSA
# policy (not via Helm value) ; logged for reference only.
AUDIT_BUCKET=$(cd "$TF_DIR" && terraform output -raw audit_bucket_name)
echo "  EKS cluster   : $EKS_CLUSTER"
echo "  Audit bucket  : $AUDIT_BUCKET"
echo "  BYOK key      : ${BYOK_KEY_ARN:-(disabled)}"

echo "🔧 Configuring kubectl..."
aws eks update-kubeconfig --region "$REGION" --name "$EKS_CLUSTER"

echo "📁 Ensuring namespace $NAMESPACE..."
kubectl create namespace "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f -

echo "🔑 Syncing secrets from Secrets Manager..."

# Postgres
DB_CONN=$(aws secretsmanager get-secret-value \
  --secret-id "$DB_SECRET_ARN" \
  --query SecretString --output text \
  | jq -r .connection_string)
kubectl create secret generic efe-postgres-external \
  --from-literal=connection_string="$DB_CONN" \
  -n "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f -

# Redis
REDIS_CONN=$(aws secretsmanager get-secret-value \
  --secret-id "$REDIS_SECRET_ARN" \
  --query SecretString --output text \
  | jq -r .connection_string)
REDIS_TOKEN=$(aws secretsmanager get-secret-value \
  --secret-id "$REDIS_SECRET_ARN" \
  --query SecretString --output text \
  | jq -r .auth_token)
kubectl create secret generic efe-redis-external-auth \
  --from-literal=token="$REDIS_TOKEN" \
  --from-literal=connection_string="$REDIS_CONN" \
  -n "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f -

# HMAC seed
HMAC_SEED=$(aws secretsmanager get-secret-value \
  --secret-id "$HMAC_SECRET_ARN" \
  --query SecretString --output text)
kubectl create secret generic efe-audit-hmac \
  --from-literal=seed="$HMAC_SEED" \
  -n "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f -

echo "📝 Generating values-${TENANT}.yaml..."
VALUES_FILE="/tmp/values-${TENANT}.yaml"
cat > "$VALUES_FILE" <<EOF
global:
  domain: ${DOMAIN}
  tenant: ${TENANT}
  environment: prod
  externalPostgres:
    enabled: true
    secretRef:
      name: efe-postgres-external
      key: connection_string
efe-server:
  byok:
    backend: aws-kms
    awsKmsKeyId: ${BYOK_KEY_ARN}
  serviceAccount:
    create: true
    annotations:
      eks.amazonaws.com/role-arn: ${IRSA_ARN}
postgres-operator:
  enabled: false
redis-stack:
  enabled: false
externalRedis:
  enabled: true
  authTokenSecretRef:
    name: efe-redis-external-auth
    key: token
EOF

echo "📦 Installing efe-stack..."
helm upgrade --install "$RELEASE" ./charts/efe-stack \
  --values "$VALUES_FILE" \
  --namespace "$NAMESPACE" \
  --wait --timeout 10m

echo "✅ Install OK for tenant ${TENANT}"
echo
echo "Verification :"
echo "  kubectl get pods -n $NAMESPACE"
echo "  helm test $RELEASE -n $NAMESPACE"
