<!-- SPDX-License-Identifier: UNLICENSED -->
# EFE · Preview Lab (frontend SSE) — Phase 4 / P4.S4

Frontend Next.js qui visualise le **pré-mâchage EFE en direct**. Consomme le
bridge [`efe-preview-server`](../crates/efe-preview-server) via SSE.

## Scènes

- **Live Mission** (`/`) — composer (A/B/Speculative) + Pipeline (Analyzer →
  Router → Compressor → LLM → Verifier) + TokenStream + SavingsGauge (triplet
  latence/coût µ¢/tokens) + Blackboard + OverlayToasts.
- **Savings Report** (`/savings/`) — le livrable R3.S9 : hero « cents 0 % vs µ¢ »
  + tableau par mode.
- **A/B Lab** (`/ab/`) — lance N missions A/B (`POST /v1/ab/run`) et compare
  control vs experiment (coût/qualité/latence + deltas).
- **Resilience Theater** (`/resilience/`) — déclenche de **vraies** dégradations
  (LLM fallback, drafter bypass, noise rejection) et observe la récupération
  gracieuse.

Garde-fou de non-dérive : au boot, `checkSchema()` compare `GET /v1/schema`
(version + 19 tags) à `src/lib/events.ts`.

## Single-binary (le bridge sert l'UI)

```bash
./preview-lab/run-preview.sh         # build l'export + lance le bridge sur :8700
# ou Docker :
docker compose -f preview-lab/docker-compose.yml up --build
```
Le bridge sert le build statique (`out/`, export Next.js) via `ServeDir`
(`EFE_PREVIEW_STATIC_DIR`), aux côtés de l'API SSE.

## Démarrer

```bash
# 1) le bridge (depuis la racine du workspace), mode mock gratuit :
EFE_PREVIEW_MOCK=1 cargo run -p efe-preview-server   # → http://127.0.0.1:8700

# 2) le frontend :
cd preview-lab
npm install        # ou pnpm install
npm run dev        # → http://localhost:3000
```

Variables (`.env.local`) :
- `NEXT_PUBLIC_EFE_PREVIEW_URL` (défaut `http://localhost:8700`)
- `NEXT_PUBLIC_EFE_PREVIEW_TOKEN` (si le bridge exige un bearer)

## Tests

```bash
npm run typecheck   # tsc --noEmit
npm run test        # vitest : T7 (contrat tags) + T8 (réducteur)
```

## Architecture

- `src/lib/events.ts` — union TS miroir de `StreamEvent` (19 variants) + DTOs.
- `src/lib/reduce.ts` — réducteur **pur** `StreamEvent[] → MissionState` (T8).
- `src/lib/useMissionStream.ts` — hook `fetch(POST)` + `ReadableStream` → events.
- `src/lib/api.ts` — accès bridge + `checkSchema` (non-dérive).
- `src/components/*` — Pipeline, RobotNode, TokenStream, SavingsGauge,
  BlackboardPanel, OverlayToasts, MissionComposer.
- `src/app/*` — shell + scène Live Mission.

Scènes Savings / A/B / Resilience : à venir (P4.S5 / P4.S6).
