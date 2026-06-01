// Sprint D24 — React example : minimal classifier UI.
//
// **Audit fix M10-A12** — pre-fix had `: JSX.Element` return type
// which assumed a global `JSX` namespace. With `react-jsx` runtime
// (TS 5 + React 18), the global `JSX` may or may not be present
// depending on `@types/react` version. Drop the annotation — TS
// infers `React.ReactElement` automatically. Also use `import.meta.env.X`
// (now typed via `vite-env.d.ts`) instead of bracket-access casts.
import type { ReactElement } from 'react';
import { useState } from 'react';

import { useClassify, useEfeClient } from './useEfeClient.js';

export function App(): ReactElement {
  const endpoint = import.meta.env.VITE_EFE_ENDPOINT ?? 'http://localhost:50052';
  const jwt = import.meta.env.VITE_EFE_JWT;

  const client = useEfeClient({ endpoint, jwt });
  const { loading, verdict, error, classify } = useClassify(client);
  const [text, setText] = useState('');

  return (
    <main style={{ fontFamily: 'system-ui, sans-serif', padding: 24, maxWidth: 720 }}>
      <h1>@efe/sdk — React example</h1>
      <p>
        Endpoint : <code>{endpoint}</code>
      </p>
      <textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder="Enter some text to classify…"
        rows={4}
        style={{ width: '100%', fontSize: 14, padding: 8 }}
      />
      <div style={{ marginTop: 12 }}>
        <button
          onClick={() => void classify(text)}
          disabled={loading || text.trim().length === 0}
          type="button"
          style={{ padding: '8px 16px', fontSize: 14 }}
        >
          {loading ? 'Classifying…' : 'Classify'}
        </button>
      </div>
      {error && (
        <pre style={{ marginTop: 12, color: 'crimson', background: '#fee', padding: 12 }}>
          Error: {error.message}
        </pre>
      )}
      {verdict && (
        <pre style={{ marginTop: 12, background: '#f6f8fa', padding: 12 }}>
          {JSON.stringify(verdict, null, 2)}
        </pre>
      )}
    </main>
  );
}
