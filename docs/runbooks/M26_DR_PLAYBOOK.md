# M26 Disaster Recovery Playbook

**Audience** : SRE on-call lead.
**Game day cadence** : 4/year (one per quarter).
**Last drill** : `<YYYY-Qq>` see [Drill log](#drill-log).

---

## 1. DB region failover (Q1 game day)

**Scenario** : Postgres primary in region X becomes unreachable.

**RTO** : ≤ 5 min. **RPO** : ≤ 30 s.

**Steps** :

1. Alertmanager fires `DatabaseUnreachable` (P0).
2. Verify primary unreachable : `psql -h <primary> -c 'SELECT 1'` → fails.
3. Promote standby (cloud-managed Postgres : trigger via console or
   Terraform).
4. Update DNS CNAME `efe-db.example.com` → standby endpoint.
5. Restart `efe-server` pods to pick up new connection : `kubectl rollout restart -n efe-system deployment/efe-server`.
6. Same for `efe-agent-runner` controller pods.
7. Verify : `efe-admin health check --tenant <s>`.

**Post-mortem** : audit chain MUST verify intact. Run `efe-admin audit verify <s> --module=agent_runtime` for the largest tenant.

## 2. Firecracker fleet loss (Q2 game day)

**Scenario** : All KVM-capable worker nodes unavailable simultaneously
(e.g. AZ outage).

**RTO** : ≤ 15 min (auto-scale + cordon).
**RPO** : agent_state checkpoints survive ; in-flight runs lose runtime
state and must restart from last checkpoint.

**Steps** :

1. Alertmanager fires `AgentMicrovmProvisioningDown` (P0).
2. Cordon affected nodes : `kubectl cordon <node>`.
3. Spin up replacement KVM-capable nodes (provider-specific : AWS launch
   template, GCP instance group, etc.).
4. Wait for vsock-relayer DaemonSet pods to schedule on new nodes.
5. Resume any paused runs : `efe-admin agent resume <run_id>` for runs
   in `PAUSED` state.
6. For runs in `RUNNING` state when the fleet went down, the controller
   marks them `FAILED` after 5 min of unreachable VM. The tenant
   re-creates from the last checkpoint.

## 3. KMS key rotation (Q3 game day)

**Scenario** : KMS key compromise (real or simulated).

**RTO** : ≤ 1 h (rotate + re-encrypt active checkpoints).
**RPO** : 0 (no data loss).

**Steps** :

1. Open security incident (P0).
2. Disable old KMS key (kept for decryption only).
3. Create new KMS key version.
4. Update `EFE_KMS_KEY_PER_TENANT_<slug>` env var on controller.
5. Restart controller pods.
6. New `agent_state` writes use new key. Old reads continue with old
   key (KMS handles version selection).
7. Re-encrypt active checkpoints :
   ```bash
   efe-admin agent state reencrypt --all --confirm
   ```
8. After all checkpoints re-encrypted, mark old key version for deletion
   (≥ 30 d retention for forensics).

## 4. Audit chain HMAC re-key (Q4 game day, part 1)

**Scenario** : `EFE_AUDIT_HMAC_KEY` compromised.

**RTO** : ≤ 24 h. **RPO** : 0.

**Steps** :

1. Open security incident (P0).
2. **Freeze writes** : flip `EFE_FEATURE_AGENT_RUNTIME_ENABLED=false`.
3. Verify all current chains :
   ```bash
   for tenant in $(efe-admin tenant list --json | jq -r '.[]|.slug'); do
     efe-admin audit verify $tenant --module=agent_runtime || echo "FAIL: $tenant"
   done
   ```
4. Generate new HMAC key, store in Vault.
5. Run the dual-key window migration :
   ```bash
   # For each tenant : compute new HMACs for ALL existing rows
   # using the new key, write to a `hmac_v2` shadow column.
   efe-admin audit rekey --confirm
   ```
6. Cut over : drop `hmac` column, rename `hmac_v2` → `hmac`.
7. Restore writes : `EFE_FEATURE_AGENT_RUNTIME_ENABLED=true`.

## 5. cosign key rotation (Q4 game day, part 2)

**Scenario** : cosign signing key compromise.

**RTO** : ≤ 6 h. **RPO** : n/a.

**Steps** :

1. Open security incident (P0).
2. Disable old cosign key in KMS keyring.
3. Generate new cosign key + add to keyring.
4. Re-sign current `agent-core` and `efe-agent-runner` images with new
   key + push new Rekor entries.
5. Update `EFE_M25_IMAGE_WHITELIST` env var with new digest entries.
6. Update `EFE_M25_COSIGN_KEY_REF` to point to new key version.
7. Restart controller pods → cosign verify uses new key.
8. Old digests remain Rekor-verifiable (transparency log is append-only)
   but no new images are signed with old key.

---

## Drill log

| Date | Quarter | Scenario | Lead | Outcome | Lessons |
|---|---|---|---|---|---|
| `<YYYY-MM-DD>` | Q1 | DB failover | `<name>` | OK / partial | … |
| `<YYYY-MM-DD>` | Q2 | FC fleet loss | `<name>` | … | … |
| `<YYYY-MM-DD>` | Q3 | KMS rotation | `<name>` | … | … |
| `<YYYY-MM-DD>` | Q4 | HMAC + cosign | `<name>` | … | … |

---

## Roles + responsibilities

| Role | During drill | Post-drill |
|---|---|---|
| SRE on-call lead | Executes the runbook | Documents in drill log |
| Eng lead | Supports + observes | Files lessons in `docs/runbooks/M26_DR_LESSONS.md` |
| DPO | Observes (compliance) | Files annual report |
| Security | Observes + spot-checks audit chain integrity | Validates findings |
| Product | Notifies tenants of drill timing | Coordinates follow-up |
