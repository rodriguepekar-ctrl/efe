# `@efe/sdk` — React 18 + Vite example

Demonstrates the canonical EFE SDK integration in a [React 18](https://react.dev)
app via [Vite](https://vitejs.dev) + the `useEfeClient` + `useClassify`
hook pattern.

## Run

```bash
pnpm install      # from the repo root, hoists workspace deps
pnpm dev          # http://localhost:5173

# In a separate shell, run the gRPC mock + Envoy translator :
docker run --rm --network host -v $PWD/../../tests/e2e/fixtures:/cfg \
  envoyproxy/envoy:v1.29-latest \
  -c /cfg/envoy.yaml --log-level info
```

Or point at a real EFE staging endpoint :

```bash
VITE_EFE_ENDPOINT=https://api-staging.efe.ai \
VITE_EFE_JWT=<token> \
pnpm dev
```

## Endpoints exercised

* `POST /efe.v1.SwarmService/Execute` (gRPC-Web) — via `client.classify(...)`

## Layout

This is a **separate Vite project** (own `vite.config.ts` + `tsconfig.json`)
that consumes the SDK via `workspace:*`. Adding the SDK to your project
does NOT pull React.

## See also

* [SDK README](../../README.md)
* [`vue` example](../vue/) — same handler, Vue 3 composables.
* [`express` example](../express/) — server-side BFF.
