// Sprint Φ-D-LATACT D92 — Sandbox page (= one-click spin-up + first classify).

'use client';
import { useState, useEffect } from 'react';
import { emitActivationEvent } from '@/lib/telemetry';

const SIGNUP_TS_KEY = 'efe_signup_ts';

export default function SandboxPage() {
  const [classifyResult, setClassifyResult] = useState<string | null>(null);
  const [classifying, setClassifying] = useState(false);
  const [tenantId, setTenantId] = useState<string | null>(null);

  useEffect(() => {
    const url = new URL(window.location.href);
    const tid = url.searchParams.get('tenant');
    if (tid) {
      setTenantId(tid);
      // Mark sandbox_created event (= one-click spin-up complete).
      const signupTs = parseInt(sessionStorage.getItem(SIGNUP_TS_KEY) || `${Date.now()}`);
      emitActivationEvent({
        state: 'sandbox_created',
        tenant_id: tid,
        time_since_signup_ms: Date.now() - signupTs,
        ts: Date.now(),
      });
    }
  }, []);

  async function handleClassify() {
    if (!tenantId) return;
    setClassifying(true);
    try {
      const res = await fetch('/api/sandbox/classify', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          tenant_id: tenantId,
          query: 'Where is my package?',
        }),
      });
      const { label } = await res.json();
      setClassifyResult(label);

      const signupTs = parseInt(sessionStorage.getItem(SIGNUP_TS_KEY) || `${Date.now()}`);
      await emitActivationEvent({
        state: 'first_classify',
        tenant_id: tenantId,
        time_since_signup_ms: Date.now() - signupTs,
        classify_result: label,
        ts: Date.now(),
      });
    } finally {
      setClassifying(false);
    }
  }

  async function handleConfirmCorrect() {
    if (!tenantId) return;
    const signupTs = parseInt(sessionStorage.getItem(SIGNUP_TS_KEY) || `${Date.now()}`);
    await emitActivationEvent({
      state: 'first_correct',
      tenant_id: tenantId,
      time_since_signup_ms: Date.now() - signupTs,
      ts: Date.now(),
    });
    // Aha moment ! Trigger nudge to POC.
    window.location.href = '/poc/request';
  }

  return (
    <main className="sandbox">
      <h1>Sandbox — Try a classify</h1>
      <p>Tenant : {tenantId ?? 'loading...'}</p>
      <button onClick={handleClassify} disabled={classifying || !tenantId}>
        {classifying ? 'Classifying...' : 'Run classify on sample query'}
      </button>
      {classifyResult && (
        <div className="result">
          <p>Result : <strong>{classifyResult}</strong></p>
          <button onClick={handleConfirmCorrect}>This is correct ✓</button>
        </div>
      )}
    </main>
  );
}
