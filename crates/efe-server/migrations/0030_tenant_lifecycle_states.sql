-- 0030_tenant_lifecycle_states.sql — Sprint Φ-D-OPS (D96)
--
-- **Φ-D-OPS post-audit fix B1** : initially numbered `0014_` and
-- collided with the pre-existing `0014_taxonomy_curation.sql` (M23).
-- `sqlx::migrate!()` panics at boot on duplicate numeric prefixes,
-- so the migration was renumbered to `0030_` (= the next free slot
-- after `0029_bundle_releases`). Saga driver reference updated in
-- `crates/efe-saga/src/tenant_lifecycle.rs`.
--
-- Materialises the 12-state tenant lifecycle FSM described in
-- `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 7.
--
-- States :
--    1. signup_intent
--    2. provisioning_pg
--    3. provisioning_redis
--    4. migrating_schema
--    5. seeding_fixtures
--    6. compliance_pending
--    7. active
--    8. throttled
--    9. suspended
--   10. erasure_pending
--   11. erasure_in_progress
--   12. erased
--
-- Each transition is gated by a CHECK + a trigger that audits the
-- jump into `ui_audit_log` (HMAC-chained). The saga driver in
-- `crates/efe-saga/src/tenant_lifecycle.rs` is the only client.

-- ─── Enum + table extensions ────────────────────────────────────────
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'tenant_state') THEN
        CREATE TYPE tenant_state AS ENUM (
            'signup_intent',
            'provisioning_pg',
            'provisioning_redis',
            'migrating_schema',
            'seeding_fixtures',
            'compliance_pending',
            'active',
            'throttled',
            'suspended',
            'erasure_pending',
            'erasure_in_progress',
            'erased'
        );
    END IF;
END $$;

-- ALTER tenants table : add lifecycle columns idempotently.
ALTER TABLE IF EXISTS tenants
    ADD COLUMN IF NOT EXISTS lifecycle_state tenant_state
        NOT NULL DEFAULT 'signup_intent',
    ADD COLUMN IF NOT EXISTS lifecycle_entered_at TIMESTAMPTZ
        NOT NULL DEFAULT now(),
    ADD COLUMN IF NOT EXISTS lifecycle_attempts INTEGER
        NOT NULL DEFAULT 0 CHECK (lifecycle_attempts >= 0),
    ADD COLUMN IF NOT EXISTS lifecycle_last_error TEXT,
    ADD COLUMN IF NOT EXISTS lifecycle_saga_id UUID;

CREATE INDEX IF NOT EXISTS tenants_lifecycle_state_idx
    ON tenants(lifecycle_state);

CREATE INDEX IF NOT EXISTS tenants_lifecycle_saga_id_idx
    ON tenants(lifecycle_saga_id)
    WHERE lifecycle_saga_id IS NOT NULL;

-- ─── Valid transition matrix ────────────────────────────────────────
-- Maintained as a table for runtime introspection (the saga reads it
-- to know whether a step it's about to take is legal).

CREATE TABLE IF NOT EXISTS tenant_lifecycle_transitions (
    from_state    tenant_state NOT NULL,
    to_state      tenant_state NOT NULL,
    direction     TEXT NOT NULL CHECK (direction IN ('forward','compensate','admin')),
    description   TEXT NOT NULL,
    PRIMARY KEY (from_state, to_state)
);

INSERT INTO tenant_lifecycle_transitions(from_state, to_state, direction, description)
VALUES
    -- Happy-path forward edges
    ('signup_intent',       'provisioning_pg',     'forward',    'Saga step 1'),
    ('provisioning_pg',     'provisioning_redis',  'forward',    'Saga step 2'),
    ('provisioning_redis',  'migrating_schema',    'forward',    'Saga step 3'),
    ('migrating_schema',    'seeding_fixtures',    'forward',    'Saga step 4'),
    ('seeding_fixtures',    'compliance_pending',  'forward',    'Saga step 5'),
    ('compliance_pending',  'active',              'forward',    'Attest signed → live'),
    -- Operational transitions
    ('active',              'throttled',           'admin',      'Quota burst exceeded'),
    ('throttled',           'active',              'admin',      'Throttle lifted'),
    ('active',              'suspended',           'admin',      'Admin suspend'),
    ('suspended',           'active',              'admin',      'Admin resume'),
    -- Erasure cascade
    ('active',              'erasure_pending',     'forward',    'Art. 17 request received'),
    ('throttled',           'erasure_pending',     'forward',    'Art. 17 from throttled'),
    ('suspended',           'erasure_pending',     'forward',    'Art. 17 from suspended'),
    ('erasure_pending',     'erasure_in_progress', 'forward',    'GDPR saga running'),
    ('erasure_in_progress', 'erased',              'forward',    'GDPR saga complete'),
    -- Compensation edges (saga rollback)
    ('provisioning_pg',     'signup_intent',       'compensate', 'PG provision failed'),
    ('provisioning_redis',  'provisioning_pg',     'compensate', 'Redis provision failed'),
    ('migrating_schema',    'provisioning_redis',  'compensate', 'Schema migration failed'),
    ('seeding_fixtures',    'migrating_schema',    'compensate', 'Seed fixtures failed'),
    ('compliance_pending',  'seeding_fixtures',    'compensate', 'Attest rejected, retry')
ON CONFLICT DO NOTHING;

-- ─── Trigger : audit every state transition ─────────────────────────
CREATE OR REPLACE FUNCTION fn_tenant_lifecycle_audit()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    is_valid BOOLEAN;
BEGIN
    IF NEW.lifecycle_state = OLD.lifecycle_state THEN
        RETURN NEW;
    END IF;

    SELECT EXISTS (
        SELECT 1 FROM tenant_lifecycle_transitions
        WHERE from_state = OLD.lifecycle_state
          AND to_state   = NEW.lifecycle_state
    ) INTO is_valid;

    IF NOT is_valid THEN
        -- Bump the invalid-transition counter so the alert fires.
        PERFORM pg_notify(
            'tenant_invalid_transition',
            jsonb_build_object(
                'tenant_id', NEW.tenant_id,
                'from',      OLD.lifecycle_state::text,
                'to',        NEW.lifecycle_state::text,
                'saga_id',   NEW.lifecycle_saga_id
            )::text
        );
        RAISE EXCEPTION
            'invalid tenant transition: % → % for tenant %',
            OLD.lifecycle_state, NEW.lifecycle_state, NEW.tenant_id;
    END IF;

    NEW.lifecycle_entered_at := now();
    NEW.lifecycle_attempts   := NEW.lifecycle_attempts + 1;

    INSERT INTO ui_audit_log(
        tenant_id,
        actor,
        action,
        payload_json,
        request_id
    ) VALUES (
        NEW.tenant_id,
        coalesce(current_setting('efe.actor', true), 'system'),
        'tenant_lifecycle_transition',
        jsonb_build_object(
            'from', OLD.lifecycle_state::text,
            'to',   NEW.lifecycle_state::text,
            'saga_id', NEW.lifecycle_saga_id,
            'attempt', NEW.lifecycle_attempts
        ),
        coalesce(current_setting('efe.request_id', true), gen_random_uuid()::text)
    );
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS tg_tenants_lifecycle_audit ON tenants;
CREATE TRIGGER tg_tenants_lifecycle_audit
    BEFORE UPDATE OF lifecycle_state ON tenants
    FOR EACH ROW
    EXECUTE FUNCTION fn_tenant_lifecycle_audit();

-- ─── Convenience view ───────────────────────────────────────────────
CREATE OR REPLACE VIEW v_tenant_lifecycle_age AS
SELECT
    tenant_id,
    lifecycle_state,
    EXTRACT(EPOCH FROM now() - lifecycle_entered_at)::INTEGER AS age_seconds,
    lifecycle_attempts,
    lifecycle_last_error,
    lifecycle_saga_id
FROM tenants;

COMMENT ON VIEW v_tenant_lifecycle_age IS
'Per-tenant time-in-state, consumed by the SLO `SLO-OPS-PROVISION-P95`.';

-- End of 0014.
