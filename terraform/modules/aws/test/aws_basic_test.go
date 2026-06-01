// Package test contains terratest integration tests for the AWS module.
//
// These tests provision REAL AWS resources and incur cost. They are
// expected to run only in CI on a sandbox AWS account, with all
// resources tagged `efe.io/terratest=true` for cleanup safety.
//
// To run locally :
//
//	cd terraform/modules/aws/test
//	go test -v -timeout 60m ./...
package test

import (
	"fmt"
	"strings"
	"testing"

	"github.com/aws/aws-sdk-go/aws"
	"github.com/aws/aws-sdk-go/aws/session"
	"github.com/aws/aws-sdk-go/service/eks"
	"github.com/aws/aws-sdk-go/service/rds"
	"github.com/aws/aws-sdk-go/service/s3"
	"github.com/gruntwork-io/terratest/modules/random"
	"github.com/gruntwork-io/terratest/modules/terraform"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

// awsSession returns a configured AWS SDK session.
func awsSession(t *testing.T, region string) *session.Session {
	sess, err := session.NewSession(&aws.Config{Region: aws.String(region)})
	require.NoError(t, err)
	return sess
}

// commonOptions returns terraform options shared by every test.
func commonOptions(region, tenant string) *terraform.Options {
	return &terraform.Options{
		TerraformDir: "../examples/basic",
		Vars: map[string]interface{}{
			"region":                   region,
			"tenant":                   tenant,
			"data_residency":           "EU_ONLY",
			"rds_deletion_protection":  false,
			"rds_multi_az":             false,
			"rds_allocated_storage":    20,
			"redis_num_cache_clusters": 2,
		},
		EnvVars: map[string]string{
			"AWS_REGION": region,
		},
		NoColor: true,
	}
}

// TestEfeAwsBasic provisions the full stack and verifies basic outputs.
func TestEfeAwsBasic(t *testing.T) {
	t.Parallel()

	region := "eu-west-1"
	tenant := fmt.Sprintf("tt%s", strings.ToLower(random.UniqueId()))

	opts := commonOptions(region, tenant)
	defer terraform.Destroy(t, opts)
	terraform.InitAndApply(t, opts)

	require.NotEmpty(t, terraform.Output(t, opts, "kubeconfig_command"))
	require.NotEmpty(t, terraform.Output(t, opts, "eks_cluster_name"))
	require.NotEmpty(t, terraform.Output(t, opts, "rds_endpoint"))
	require.NotEmpty(t, terraform.Output(t, opts, "audit_bucket_name"))

	sess := awsSession(t, region)
	clusterName := terraform.Output(t, opts, "eks_cluster_name")

	eksClient := eks.New(sess)
	cluster, err := eksClient.DescribeCluster(&eks.DescribeClusterInput{
		Name: &clusterName,
	})
	require.NoError(t, err)
	assert.Equal(t, "ACTIVE", *cluster.Cluster.Status)
	require.NotNil(t, cluster.Cluster.Identity.Oidc)
	assert.Contains(t, *cluster.Cluster.Identity.Oidc.Issuer, "oidc.eks")
}

// TestEfeAwsResidencyEnforcement verifies that EU_ONLY residency
// rejects non-EU regions at plan time.
func TestEfeAwsResidencyEnforcement(t *testing.T) {
	t.Parallel()

	tenant := fmt.Sprintf("tt%s", strings.ToLower(random.UniqueId()))
	opts := &terraform.Options{
		TerraformDir: "../examples/basic",
		Vars: map[string]interface{}{
			"region":         "us-east-1",
			"tenant":         tenant,
			"data_residency": "EU_ONLY",
		},
		NoColor: true,
	}
	_, err := terraform.InitAndPlanE(t, opts)
	require.Error(t, err)
	assert.Contains(t, err.Error(), "data_residency")
}

// TestEfeAwsAuditBucketEncrypted verifies SSE-KMS on the audit bucket.
func TestEfeAwsAuditBucketEncrypted(t *testing.T) {
	t.Parallel()

	region := "eu-west-1"
	tenant := fmt.Sprintf("tt%s", strings.ToLower(random.UniqueId()))

	opts := commonOptions(region, tenant)
	defer terraform.Destroy(t, opts)
	terraform.InitAndApply(t, opts)

	bucketName := terraform.Output(t, opts, "audit_bucket_name")
	sess := awsSession(t, region)
	s3Client := s3.New(sess)

	enc, err := s3Client.GetBucketEncryption(&s3.GetBucketEncryptionInput{
		Bucket: aws.String(bucketName),
	})
	require.NoError(t, err)
	require.NotEmpty(t, enc.ServerSideEncryptionConfiguration.Rules)
	algo := enc.ServerSideEncryptionConfiguration.Rules[0].
		ApplyServerSideEncryptionByDefault.SSEAlgorithm
	assert.Equal(t, "aws:kms", *algo)

	// Public access block.
	pab, err := s3Client.GetPublicAccessBlock(&s3.GetPublicAccessBlockInput{
		Bucket: aws.String(bucketName),
	})
	require.NoError(t, err)
	assert.True(t, *pab.PublicAccessBlockConfiguration.BlockPublicAcls)
	assert.True(t, *pab.PublicAccessBlockConfiguration.BlockPublicPolicy)
	assert.True(t, *pab.PublicAccessBlockConfiguration.IgnorePublicAcls)
	assert.True(t, *pab.PublicAccessBlockConfiguration.RestrictPublicBuckets)

	// Versioning ON.
	ver, err := s3Client.GetBucketVersioning(&s3.GetBucketVersioningInput{
		Bucket: aws.String(bucketName),
	})
	require.NoError(t, err)
	assert.Equal(t, "Enabled", *ver.Status)
}

// TestEfeAwsRdsEncrypted verifies KMS encryption on the RDS instance.
func TestEfeAwsRdsEncrypted(t *testing.T) {
	t.Parallel()

	region := "eu-west-1"
	tenant := fmt.Sprintf("tt%s", strings.ToLower(random.UniqueId()))

	opts := commonOptions(region, tenant)
	defer terraform.Destroy(t, opts)
	terraform.InitAndApply(t, opts)

	sess := awsSession(t, region)
	rdsClient := rds.New(sess)
	dbId := fmt.Sprintf("efe-%s-postgres", tenant)

	out, err := rdsClient.DescribeDBInstances(&rds.DescribeDBInstancesInput{
		DBInstanceIdentifier: aws.String(dbId),
	})
	require.NoError(t, err)
	require.Len(t, out.DBInstances, 1)
	inst := out.DBInstances[0]
	assert.True(t, *inst.StorageEncrypted)
	assert.NotNil(t, inst.KmsKeyId)
	assert.Equal(t, "available", *inst.DBInstanceStatus)
}

// TestEfeAwsByokDisabled verifies that with enable_kms_byok=false,
// no BYOK key is created and the output is null.
func TestEfeAwsByokDisabled(t *testing.T) {
	t.Parallel()

	region := "eu-west-1"
	tenant := fmt.Sprintf("tt%s", strings.ToLower(random.UniqueId()))

	opts := commonOptions(region, tenant)
	defer terraform.Destroy(t, opts)
	terraform.InitAndApply(t, opts)

	val := terraform.OutputE
	out, err := val(t, opts, "byok_kms_key_arn")
	require.NoError(t, err)
	assert.Empty(t, out)
}
