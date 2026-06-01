// Package test contains terratest integration tests for the Azure module.
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
	return &terraform.Options{
		TerraformDir: "../examples/basic",
		Vars: map[string]interface{}{
			"location":       "westeurope",
			"tenant":         tenant,
			"data_residency": "EU_ONLY",
		},
		NoColor: true,
	}, tenant
}

func TestEfeAzureBasic(t *testing.T) {
	t.Parallel()

	opts, _ := commonOptions(t)
	defer terraform.Destroy(t, opts)
	terraform.InitAndApply(t, opts)

	require.NotEmpty(t, terraform.Output(t, opts, "kubeconfig_command"))
	require.NotEmpty(t, terraform.Output(t, opts, "aks_cluster_name"))
	require.NotEmpty(t, terraform.Output(t, opts, "audit_storage_account"))
}

func TestEfeAzureResidencyEnforcement(t *testing.T) {
	t.Parallel()

	tenant := fmt.Sprintf("tt%s", strings.ToLower(random.UniqueId()))
	opts := &terraform.Options{
		TerraformDir: "../examples/basic",
		Vars: map[string]interface{}{
			"location":       "eastus",
			"tenant":         tenant,
			"data_residency": "EU_ONLY",
		},
		NoColor: true,
	}

	_, err := terraform.InitAndPlanE(t, opts)
	require.Error(t, err)
	assert.Contains(t, err.Error(), "data_residency")
}
