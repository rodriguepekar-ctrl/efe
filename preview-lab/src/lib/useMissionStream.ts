// SPDX-License-Identifier: UNLICENSED
"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { postMissionStream, postResilienceStream } from "./api";
import type {
  MissionDto,
  MissionType,
  ResilienceScenario,
  StreamEvent,
} from "./events";
import { reduceMissionState, type MissionState } from "./reduce";

export type StreamStatus = "idle" | "connecting" | "streaming" | "done" | "error";

export interface UseMissionStream {
  events: StreamEvent[];
  state: MissionState;
  status: StreamStatus;
  error: string | null;
  start: (type: MissionType, dto: MissionDto) => Promise<void>;
  startResilience: (scenario: ResilienceScenario) => Promise<void>;
  reset: () => void;
}

/**
 * Pilote une mission live et expose les events + l'état dérivé.
 *
 * Transport : `fetch(POST)` + `ReadableStream` (un seul aller-retour, pas de
 * course — le body de requête porte la mission, le stream démarre sur la même
 * connexion). Parse les frames SSE `data: {json}\n\n` au fil de l'eau.
 */
export function useMissionStream(): UseMissionStream {
  const [events, setEvents] = useState<StreamEvent[]>([]);
  const [status, setStatus] = useState<StreamStatus>("idle");
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  const reset = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    setEvents([]);
    setStatus("idle");
    setError(null);
  }, []);

  // Cœur : ouvre un POST (via `opener`) et consomme le flux SSE.
  const beginStream = useCallback(
    async (opener: (signal: AbortSignal) => Promise<Response>) => {
      abortRef.current?.abort();
      const ac = new AbortController();
      abortRef.current = ac;
      setEvents([]);
      setError(null);
      setStatus("connecting");

      try {
        const resp = await opener(ac.signal);
        if (!resp.ok) {
          const text = await resp.text().catch(() => "");
          setStatus("error");
          setError(`HTTP ${resp.status}${text ? ` — ${text}` : ""}`);
          return;
        }
        if (!resp.body) {
          setStatus("error");
          setError("réponse sans body (streaming non supporté par ce navigateur)");
          return;
        }

        setStatus("streaming");
        const reader = resp.body.getReader();
        const decoder = new TextDecoder();
        let buf = "";

        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          buf += decoder.decode(value, { stream: true });

          let sep = buf.indexOf("\n\n");
          while (sep >= 0) {
            const frame = buf.slice(0, sep);
            buf = buf.slice(sep + 2);
            const data = frame
              .split("\n")
              .filter((l) => l.startsWith("data:"))
              .map((l) => l.slice(5).replace(/^ /, ""))
              .join("\n");
            if (data) {
              try {
                const ev = JSON.parse(data) as StreamEvent;
                setEvents((prev) => [...prev, ev]);
              } catch {
                // frame partielle/non-JSON — ignorée.
              }
            }
            sep = buf.indexOf("\n\n");
          }
        }
        setStatus("done");
      } catch (e) {
        if (ac.signal.aborted) return; // reset()/unmount → silencieux
        setStatus("error");
        setError(String(e));
      }
    },
    [],
  );

  const start = useCallback(
    (type: MissionType, dto: MissionDto) =>
      beginStream((signal) => postMissionStream(type, dto, signal)),
    [beginStream],
  );

  const startResilience = useCallback(
    (scenario: ResilienceScenario) =>
      beginStream((signal) => postResilienceStream(scenario, signal)),
    [beginStream],
  );

  // Abort en cours au démontage.
  useEffect(() => () => abortRef.current?.abort(), []);

  const state = useMemo(() => reduceMissionState(events), [events]);

  return { events, state, status, error, start, startResilience, reset };
}
