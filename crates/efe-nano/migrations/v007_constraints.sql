-- Migration v007 — M50 Sprint 6 — Cross-tenant chain integrity guard.
--
-- Trigger that rejects any INSERT where chain_parent_id references a
-- trace belonging to a DIFFERENT tenant.  This is a defense-in-depth
-- layer beyond row-level security : even if RLS is misconfigured, the
-- chain itself cannot be poisoned across tenants.

CREATE OR REPLACE FUNCTION enforce_chain_parent_tenant()
RETURNS TRIGGER AS $$
DECLARE
    parent_tenant UUID;
BEGIN
    IF NEW.chain_parent_id IS NULL THEN
        RETURN NEW;
    END IF;
    SELECT tenant_id INTO parent_tenant
      FROM nano_agent_traces
     WHERE trace_id = NEW.chain_parent_id;
    IF parent_tenant IS NULL THEN
        RAISE EXCEPTION 'chain_parent_id % does not exist', NEW.chain_parent_id;
    END IF;
    IF parent_tenant != NEW.tenant_id THEN
        RAISE EXCEPTION 'chain_parent_id from different tenant : % vs %',
            parent_tenant, NEW.tenant_id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS enforce_chain_parent_tenant_trigger ON nano_agent_traces;
CREATE TRIGGER enforce_chain_parent_tenant_trigger
    BEFORE INSERT ON nano_agent_traces
    FOR EACH ROW
    EXECUTE FUNCTION enforce_chain_parent_tenant();
