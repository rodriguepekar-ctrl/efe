<script setup lang="ts">
// Sprint D24 — Vue 3 SFC : minimal classifier UI.
import { ref } from 'vue';
import { useClassify, useEfeClient } from './useEfeClient.js';

const endpoint = (import.meta.env.VITE_EFE_ENDPOINT as string) ?? 'http://localhost:50052';
const jwt = import.meta.env.VITE_EFE_JWT as string | undefined;

const client = useEfeClient({ endpoint, jwt });
const { loading, verdict, error, classify } = useClassify(client);

const text = ref('');
</script>

<template>
  <main style="font-family: system-ui, sans-serif; padding: 24px; max-width: 720px;">
    <h1>@efe/sdk — Vue 3 example</h1>
    <p>Endpoint : <code>{{ endpoint }}</code></p>
    <textarea
      v-model="text"
      placeholder="Enter some text to classify…"
      rows="4"
      style="width: 100%; font-size: 14px; padding: 8px;"
    />
    <div style="margin-top: 12px;">
      <button
        type="button"
        :disabled="loading || text.trim().length === 0"
        @click="classify(text)"
        style="padding: 8px 16px; font-size: 14px;"
      >
        {{ loading ? 'Classifying…' : 'Classify' }}
      </button>
    </div>
    <pre v-if="error" style="margin-top: 12px; color: crimson; background: #fee; padding: 12px;">
Error: {{ error.message }}
    </pre>
    <pre v-if="verdict" style="margin-top: 12px; background: #f6f8fa; padding: 12px;">
{{ JSON.stringify(verdict, null, 2) }}
    </pre>
  </main>
</template>
