// Package test contains terratest integration tests for the GCP module.
//
// Provisions REAL GCP resources and incurs cost. Runs only in CI on a
// sandbox project, with all resources tagged `efe_io_terratest=true`
// for cleanup safety.
package test

import (
	"fmt"
	"strings"
	"testing"

	"github.com/gruntwork-io/terratest/modules/random"
	"github.com/gruntwork-io/terratest/modules/terraform"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func commonOptions(t *testing.T) (*terraform.Options, string) {
	tenant := fmt.Sprintf("tt%s", strings.ToLower(random.UniqueId()))
	projectID := getEnv(t, "GCP_TERRATEST_PROJECT")
	region := "europe-west1"

	return &terraform.Options{
		TerraformDir: "../examples/basic",
		Vars: map[string]interface{}{
			"project_id":                   projectID,
			"region":                       region,
			"tenant":                       tenant,
			"data_residency":               "EU_ONLY",
			"cloudsql_deletion_protection": false,
		},
		EnvVars: map[string]string{
			"GOOGLE_PROJECT": projectID,
		},
		NoColor: true,
	}, tenant
}

func getEnv(t *testing.T, key string) string {
	v := terraform.EnvVar(t, key)
	require.NotEmpty(t, v, "env var %s required", key)
	return v
}

func TestEfeGcpBasic(t *testing.T) {
	t.Parallel()

	opts, _ := commonOptions(t)
	defer terraform.Destroy(t, opts)
	terraform.InitAndApply(t, opts)

	require.NotEmpty(t, terraform.Output(t, opts, "kubeconfig_command"))
	require.NotEmpty(t, terraform.Output(t, opts, "gke_cluster_name"))
	require.NotEmpty(t, terraform.Output(t, opts, "audit_bucket_name"))
}

func TestEfeGcpResidencyEnforcement(t *testing.T) {
	t.Parallel()

	tenant := fmt.Sprintf("tt%s", strings.ToLower(random.UniqueId()))
	opts := &terraform.Options{
		TerraformDir: "../examples/basic",
		Vars: map[string]interface{}{
			"project_id":     getEnv(t, "GCP_TERRATEST_PROJECT"),
			"region":         "us-central1",
			"tenant":         tenant,
			"data_residency": "EU_ONLY",
		},
		NoColor: true,
	}

	_, err := terraform.InitAndPlanE(t, opts)
	require.Error(t, err)
	assert.Contains(t, err.Error(), "data_residency")
}
