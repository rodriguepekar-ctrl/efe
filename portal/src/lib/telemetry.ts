// Sprint Φ-D-LATACT D92 — Funnel telemetry types + event emitter.
// Cf. docs/EFE_P1_LATENCE_P2_ACTIVATION_COMPLETION_ARCHITECTURE.md § 7.3.

/**
 * 6 activation funnel states (= D-LATACT-9 granular telemetry).
 */
export type ActivationState =
  | 'signup'
  | 'sandbox_created'
  | 'first_classify'
  | 'first_correct'      // = Aha moment
  | 'poc_requested'
  | 'poc_active'
  | 'abandoned';

/** Strongly-typed event payload per state. */
export type ActivationEvent =
  | {
      state: 'signup';
      tenant_id: string;
      plan_target: 'sandbox' | 'poc' | 'prod';
      ts: number;
    }
  | {
      state: 'sandbox_created';
      tenant_id: string;
      time_since_signup_ms: number;
      ts: number;
    }
  | {
      state: 'first_classify';
      tenant_id: string;
      time_since_signup_ms: number;
      classify_result: string;
      ts: number;
    }
  | {
      state: 'first_correct';
      tenant_id: string;
      time_since_signup_ms: number;
      ts: number;
    }
  | {
      state: 'poc_requested';
      tenant_id: string;
      ts: number;
    }
  | {
      state: 'poc_active';
      tenant_id: string;
      time_since_signup_ms: number;
      cloud: 'gcp' | 'aws' | 'azure';
      ts: number;
    }
  | {
      state: 'abandoned';
      tenant_id: string;
      last_state: ActivationState;
      ts: number;
    };

/**
 * Emit an activation event. Server-side: writes to `audit_events` with HMAC chain.
 * Client-side: POSTs to /api/activation/event.
 *
 * Anti-Φ-D.A1 : event emission MUST happen BEFORE UI updates so that
 * a client crash doesn't lose the audit trail (= D-LATACT-9 atomic).
 */
export async function emitActivationEvent(event: ActivationEvent): Promise<void> {
  const isServer = typeof window === 'undefined';
  if (isServer) {
    // Server : direct PG write + audit chain (= future server-action).
    throw new Error('server-side emit not yet implemented (cf. /api/activation/event)');
  }
  // Client : POST to API route.
  const res = await fetch('/api/activation/event', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(event),
  });
  if (!res.ok) {
    // Best-effort : log but don't block UI.
    console.error('activation event failed:', res.status, await res.text());
  }
}

/**
 * Compute current state from PG row (= reverse map for resume after refresh).
 */
export function deriveCurrentState(row: {
  signed_up_at?: string | null;
  sandbox_created_at?: string | null;
  first_classify_at?: string | null;
  first_correct_at?: string | null;
  poc_requested_at?: string | null;
  poc_active_at?: string | null;
  abandoned_at?: string | null;
}): ActivationState {
  if (row.abandoned_at) return 'abandoned';
  if (row.poc_active_at) return 'poc_active';
  if (row.poc_requested_at) return 'poc_requested';
  if (row.first_correct_at) return 'first_correct';
  if (row.first_classify_at) return 'first_classify';
  if (row.sandbox_created_at) return 'sandbox_created';
  if (row.signed_up_at) return 'signup';
  throw new Error('no signed_up_at — row not in funnel');
}
