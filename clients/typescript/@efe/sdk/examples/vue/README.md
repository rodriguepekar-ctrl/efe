# `@efe/sdk` — Vue 3 + Vite example

Demonstrates the canonical EFE SDK integration in a [Vue 3](https://vuejs.org)
app via [Vite](https://vitejs.dev) + the `<script setup>` Composition API
+ `useEfeClient` + `useClassify` composables.

## Run

```bash
pnpm install
pnpm dev    # http://localhost:5174
```

## Layout

Separate Vite + Vue project (`vue-tsc` for type-check). The SDK is
consumed via `workspace:*`.

## See also

* [SDK README](../../README.md)
* [`react` example](../react/)
* [`express` example](../express/)
