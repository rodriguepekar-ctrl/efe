// Sprint Φ-D-LATACT D92 — Signup page (= start of P2 activation funnel).

'use client';
import { useState } from 'react';
import { emitActivationEvent } from '@/lib/telemetry';

// L1 audit fix Φ-D-LATACT.L1-2 : the TTFC / AHA / abandon timers start
// at signup. The signup_ts MUST be persisted in sessionStorage so
// subsequent pages (= /sandbox, /poc) can compute time_since_signup_ms.
// Without this fix, all SLO-P2-TTFC-* and SLO-P2-AHA-* would measure
// 0 ms in production (= faussement-PASS).
export const SIGNUP_TS_KEY = 'efe_signup_ts';
export const SIGNUP_TENANT_KEY = 'efe_signup_tenant';

export default function SignupPage() {
  const [email, setEmail] = useState('');
  const [planTarget, setPlanTarget] = useState<'sandbox' | 'poc' | 'prod'>('sandbox');
  const [submitting, setSubmitting] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    try {
      const res = await fetch('/api/signup', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email, plan_target: planTarget }),
      });
      if (!res.ok) throw new Error(`Signup failed : ${res.status}`);
      const { tenant_id } = await res.json();

      // L1 fix L1-2 : persist signup_ts BEFORE emitting event + redirect.
      // The TTFC clock starts at the precise moment of server-side
      // tenant_id assignment (= signed_up_at in PG).
      const signupTs = Date.now();
      sessionStorage.setItem(SIGNUP_TS_KEY, String(signupTs));
      sessionStorage.setItem(SIGNUP_TENANT_KEY, tenant_id);

      // Emit funnel event (= start of TTFC timer).
      await emitActivationEvent({
        state: 'signup',
        tenant_id,
        plan_target: planTarget,
        ts: signupTs,
      });

      // Redirect to sandbox.
      window.location.href = `/sandbox?tenant=${encodeURIComponent(tenant_id)}`;
    } catch (err) {
      console.error(err);
      setSubmitting(false);
    }
  }

  return (
    <main className="signup">
      <h1>Try EFE</h1>
      <form onSubmit={handleSubmit}>
        <label>
          Email
          <input
            type="email"
            required
            value={email}
            onChange={(e) => setEmail(e.target.value)}
          />
        </label>
        <label>
          Plan
          <select
            value={planTarget}
            onChange={(e) => setPlanTarget(e.target.value as any)}
          >
            <option value="sandbox">Sandbox (free, 5 min setup)</option>
            <option value="poc">POC (30 min setup)</option>
            <option value="prod">Production (manual review)</option>
          </select>
        </label>
        <button type="submit" disabled={submitting}>
          {submitting ? 'Creating...' : 'Create account'}
        </button>
      </form>
    </main>
  );
}
