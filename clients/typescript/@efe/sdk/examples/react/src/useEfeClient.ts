// Sprint D24 — React hook : EFE client + classify state machine.
//
// Composes 2 hooks :
//   - useEfeClient(opts)  : memoized EfeClient ; closes on unmount.
//   - useClassify(client) : { loading, verdict, error, classify(text) } state.

import { useEffect, useMemo, useState } from 'react';
import { EfeClient, type Verdict } from '@efe/sdk/web';

export function useEfeClient(opts: { endpoint: string; jwt?: string }): EfeClient {
  const client = useMemo(
    () =>
      new EfeClient({
        endpoint: opts.endpoint,
        ...(opts.jwt ? { auth: { kind: 'jwt' as const, token: opts.jwt } } : {}),
      }),
    [opts.endpoint, opts.jwt],
  );
  useEffect(
    () => () => {
      void client.close();
    },
    [client],
  );
  return client;
}

interface ClassifyState {
  loading: boolean;
  verdict?: Verdict;
  error?: Error;
}

export function useClassify(client: EfeClient): ClassifyState & {
  classify: (text: string) => Promise<void>;
} {
  const [state, setState] = useState<ClassifyState>({ loading: false });

  async function classify(text: string): Promise<void> {
    setState({ loading: true });
    try {
      const verdict = await client.classify(text);
      setState({ loading: false, verdict });
    } catch (error) {
      setState({ loading: false, error: error as Error });
    }
  }

  return { ...state, classify };
}
