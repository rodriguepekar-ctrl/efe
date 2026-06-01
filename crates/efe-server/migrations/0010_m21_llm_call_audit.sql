-- 0010_m21_llm_call_audit.sql — M21 Phase E
--
-- Per-tenant HMAC-chained audit log for every Mode-2 LLM call.
-- Extends the D14-bis pattern : each row's `hmac` covers
-- `prev_hmac` + payload, so tampering is detectable.
--
-- Privacy : only `sentence_hash` (sha256 of redacted sentence) is
-- stored, NEVER the raw sentence. Full reasoning trace (label,
-- confidence, anchors_used, provider, model, tokens, cost) is
-- preserved for GDPR Art. 22 right-to-explanation.

CREATE TABLE IF NOT EXISTS llm_call_audit (
    id              BIGSERIAL PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    catalog         TEXT NOT NULL,
    bundle_version  TEXT NOT NULL,
    sentence_hash   TEXT NOT NULL,
    mode            TEXT NOT NULL CHECK (mode IN ('MODE_2', 'MODE_AUTO_ESC')),
    provider_used   TEXT NOT NULL,
    model_used      TEXT NOT NULL,
    tokens_in       INTEGER NOT NULL CHECK (tokens_in >= 0),
    tokens_out      INTEGER NOT NULL CHECK (tokens_out >= 0),
    cost_usd        NUMERIC(10,6) NOT NULL CHECK (cost_usd >= 0),
    cache_hit       BOOLEAN NOT NULL DEFAULT FALSE,
    duration_ms     INTEGER NOT NULL CHECK (duration_ms >= 0),
    label           TEXT,
    confidence      REAL CHECK (confidence IS NULL OR confidence BETWEEN 0 AND 1),
    anchors_used    TEXT[] NOT NULL DEFAULT '{}',
    seq             BIGINT NOT NULL,
    prev_hmac       TEXT NOT NULL,
    hmac            TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, seq)
);

CREATE INDEX IF NOT EXISTS ix_llm_call_audit_tenant_time
    ON llm_call_audit(tenant_id, created_at DESC);

CREATE INDEX IF NOT EXISTS ix_llm_call_audit_provider
    ON llm_call_audit(provider_used);

COMMENT ON TABLE llm_call_audit IS
    'M21 — HMAC-chained audit log for every Mode-2 LLM call. Extends D14-bis.';
COMMENT ON COLUMN llm_call_audit.sentence_hash IS
    'sha256 of redacted sentence. Raw sentence NEVER stored.';
COMMENT ON COLUMN llm_call_audit.seq IS
    'Per-tenant monotonic sequence (UNIQUE constraint enforces no gaps).';
COMMENT ON COLUMN llm_call_audit.hmac IS
    'HMAC-SHA256 over (payload || prev_hmac). M14-bis tamper-evident chain.';
