// SPDX-License-Identifier: UNLICENSED
//
// Accès au bridge `efe-preview-server`.

import {
  ALL_EVENT_TAGS,
  EVENT_SCHEMA_VERSION,
  type MissionDto,
  type MissionType,
  type ResilienceScenario,
  type SchemaResponse,
} from "./events";
import type { AbRunDto, AbRunResult, SavingsReport } from "./report";

/** Base URL du bridge (défaut localhost:8700, overridable au build). */
export const BASE_URL =
  process.env.NEXT_PUBLIC_EFE_PREVIEW_URL ?? "http://localhost:8700";

const TOKEN = process.env.NEXT_PUBLIC_EFE_PREVIEW_TOKEN;

function headers(json: boolean): HeadersInit {
  const h: Record<string, string> = {};
  if (json) h["content-type"] = "application/json";
  if (TOKEN) h["authorization"] = `Bearer ${TOKEN}`;
  return h;
}

/** Lance une mission et retourne la `Response` (body en `text/event-stream`). */
export function postMissionStream(
  type: MissionType,
  dto: MissionDto,
  signal?: AbortSignal,
): Promise<Response> {
  return fetch(`${BASE_URL}/v1/missions/${type}`, {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify(dto),
    signal,
  });
}

/** Déclenche un scénario de résilience et retourne la `Response` SSE (P4.S6). */
export function postResilienceStream(
  scenario: ResilienceScenario,
  signal?: AbortSignal,
): Promise<Response> {
  return fetch(`${BASE_URL}/v1/resilience/run`, {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify({ scenario }),
    signal,
  });
}

export async function fetchSchema(): Promise<SchemaResponse> {
  const r = await fetch(`${BASE_URL}/v1/schema`, { headers: headers(false) });
  if (!r.ok) throw new Error(`schema HTTP ${r.status}`);
  return (await r.json()) as SchemaResponse;
}

/** Récupère le rapport savings v2. `null` si pas encore généré (404). */
export async function fetchSavingsReport(): Promise<SavingsReport | null> {
  const r = await fetch(`${BASE_URL}/v1/savings-report`, { headers: headers(false) });
  if (r.status === 404) return null;
  if (!r.ok) throw new Error(`savings-report HTTP ${r.status}`);
  return (await r.json()) as SavingsReport;
}

/** Lance un A/B test (`n` missions) et retourne la comparaison ségrégée. */
export async function runAbTest(dto: AbRunDto): Promise<AbRunResult> {
  const r = await fetch(`${BASE_URL}/v1/ab/run`, {
    method: "POST",
    headers: headers(true),
    body: JSON.stringify(dto),
  });
  if (!r.ok) {
    const text = await r.text().catch(() => "");
    throw new Error(`ab/run HTTP ${r.status}${text ? ` — ${text}` : ""}`);
  }
  return (await r.json()) as AbRunResult;
}

export interface SchemaCheck {
  ok: boolean;
  message?: string;
}

/**
 * Garde-fou de non-dérive : compare la version + les tags du serveur à ceux
 * compilés dans `events.ts`. Renvoie `ok:false` avec un message si divergence.
 */
export async function checkSchema(): Promise<SchemaCheck> {
  let server: SchemaResponse;
  try {
    server = await fetchSchema();
  } catch (e) {
    return { ok: false, message: `bridge injoignable (${String(e)})` };
  }
  if (server.event_schema_version !== EVENT_SCHEMA_VERSION) {
    return {
      ok: false,
      message: `version protocole : serveur ${server.event_schema_version} ≠ client ${EVENT_SCHEMA_VERSION}`,
    };
  }
  const local = new Set<string>(ALL_EVENT_TAGS);
  const remote = new Set<string>(server.variants);
  const missingLocal = [...remote].filter((t) => !local.has(t));
  const missingRemote = [...local].filter((t) => !remote.has(t));
  if (missingLocal.length || missingRemote.length) {
    return {
      ok: false,
      message: `drift de variants — serveur-seul: [${missingLocal.join(", ")}] · client-seul: [${missingRemote.join(", ")}]`,
    };
  }
  return { ok: true };
}
