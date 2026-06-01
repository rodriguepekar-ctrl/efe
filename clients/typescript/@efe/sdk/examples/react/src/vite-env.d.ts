// Sprint D24 — Vite-injected env typing for the React example.
//
// **Audit fix M10-A11** — pre-fix `import.meta.env.VITE_*` was typed
// as `any`. This triple-slash directive pulls Vite's canonical types.

/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_EFE_ENDPOINT?: string;
  readonly VITE_EFE_JWT?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
