//! Sprint Φ-D-LATACT D93 — Pre-built saga steps for P2 tenant deploy + GDPR delete.

use async_trait::async_trait;
use super::{SagaContext, SagaError, SagaStep};

/// Step 1 : Provision the tenant Postgres database (= cluster + schema).
pub struct ProvisionPostgresStep;

#[async_trait]
impl SagaStep for ProvisionPostgresStep {
    fn name(&self) -> &'static str { "provision_postgres" }
    async fn execute(&self, _ctx: &SagaContext) -> Result<(), SagaError> {
        // Production : invoke Terraform or PG admin RPC. Stub returns OK.
        Ok(())
    }
    async fn compensate(&self, _ctx: &SagaContext) -> Result<(), SagaError> {
        // Production : DROP DATABASE / Terraform destroy.
        Ok(())
    }
}

/// Step 2 : Migrate schema (= apply all sqlx migrations).
pub struct MigrateSchemaStep;

#[async_trait]
impl SagaStep for MigrateSchemaStep {
    fn name(&self) -> &'static str { "migrate_schema" }
    async fn execute(&self, _ctx: &SagaContext) -> Result<(), SagaError> { Ok(()) }
    async fn compensate(&self, _ctx: &SagaContext) -> Result<(), SagaError> {
        // Production : revert migrations.
        Ok(())
    }
}

/// Step 3 : Seed sample fixtures (= one demo catalog).
pub struct SeedFixturesStep;

#[async_trait]
impl SagaStep for SeedFixturesStep {
    fn name(&self) -> &'static str { "seed_fixtures" }
    async fn execute(&self, _ctx: &SagaContext) -> Result<(), SagaError> { Ok(()) }
    async fn compensate(&self, _ctx: &SagaContext) -> Result<(), SagaError> { Ok(()) }
}

/// Step 4 : Notify tenant via email.
pub struct NotifyTenantStep;

#[async_trait]
impl SagaStep for NotifyTenantStep {
    fn name(&self) -> &'static str { "notify_tenant" }
    async fn execute(&self, _ctx: &SagaContext) -> Result<(), SagaError> { Ok(()) }
    async fn compensate(&self, _ctx: &SagaContext) -> Result<(), SagaError> {
        // Cancel-email if welcome was sent.
        Ok(())
    }
}

/// Step 5 : Mark tenant active.
pub struct MarkActiveStep;

#[async_trait]
impl SagaStep for MarkActiveStep {
    fn name(&self) -> &'static str { "mark_active" }
    async fn execute(&self, _ctx: &SagaContext) -> Result<(), SagaError> { Ok(()) }
    async fn compensate(&self, _ctx: &SagaContext) -> Result<(), SagaError> { Ok(()) }
}

// ─────────────────────────────────────────────────────────────────────
// GDPR delete cascade steps (D-LATACT-8).
// ─────────────────────────────────────────────────────────────────────

pub struct GdprAuditStartStep;
pub struct GdprDropSessionsStep;
pub struct GdprTombstoneAuditStep;
pub struct GdprPurgeRedisStep;
pub struct GdprPurgeS3Step;
pub struct GdprAuditCompleteStep;

macro_rules! gdpr_step {
    ($name:ident, $tag:expr) => {
        #[async_trait]
        impl SagaStep for $name {
            fn name(&self) -> &'static str { $tag }
            async fn execute(&self, _ctx: &SagaContext) -> Result<(), SagaError> { Ok(()) }
            async fn compensate(&self, _ctx: &SagaContext) -> Result<(), SagaError> {
                // GDPR cascade compensation : tombstone audit + rollback rare.
                Ok(())
            }
        }
    };
}

gdpr_step!(GdprAuditStartStep,       "gdpr_audit_start");
gdpr_step!(GdprDropSessionsStep,     "gdpr_drop_sessions");
gdpr_step!(GdprTombstoneAuditStep,   "gdpr_tombstone_audit");
gdpr_step!(GdprPurgeRedisStep,       "gdpr_purge_redis");
gdpr_step!(GdprPurgeS3Step,          "gdpr_purge_s3");
gdpr_step!(GdprAuditCompleteStep,    "gdpr_audit_complete");
