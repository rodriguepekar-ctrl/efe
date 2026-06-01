// Sprint D24 — Vue 3 composable : EFE client + classify state.

import { onUnmounted, ref, type Ref } from 'vue';

import { EfeClient, type Verdict } from '@efe/sdk/web';

export function useEfeClient(opts: { endpoint: string; jwt?: string }): EfeClient {
  const client = new EfeClient({
    endpoint: opts.endpoint,
    ...(opts.jwt ? { auth: { kind: 'jwt' as const, token: opts.jwt } } : {}),
  });
  onUnmounted(() => {
    void client.close();
  });
  return client;
}

interface UseClassify {
  loading: Ref<boolean>;
  verdict: Ref<Verdict | undefined>;
  error: Ref<Error | undefined>;
  classify: (text: string) => Promise<void>;
}

export function useClassify(client: EfeClient): UseClassify {
  const loading = ref(false);
  const verdict = ref<Verdict | undefined>(undefined);
  const error = ref<Error | undefined>(undefined);

  async function classify(text: string): Promise<void> {
    loading.value = true;
    error.value = undefined;
    verdict.value = undefined;
    try {
      verdict.value = await client.classify(text);
    } catch (e) {
      error.value = e as Error;
    } finally {
      loading.value = false;
    }
  }

  return { loading, verdict, error, classify };
}
