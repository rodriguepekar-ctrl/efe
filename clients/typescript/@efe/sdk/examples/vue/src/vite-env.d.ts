// Sprint D24 — Vite-injected env typing for the Vue example.
//
// **Audit fix M10-A11** — pre-fix `import.meta.env.VITE_*` was typed
// as `any`, defeating TS strict mode. This triple-slash directive
// pulls Vite's canonical types so `import.meta.env.VITE_EFE_ENDPOINT`
// is `string | undefined` (correct).

/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_EFE_ENDPOINT?: string;
  readonly VITE_EFE_JWT?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
