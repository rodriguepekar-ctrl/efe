#!/usr/bin/env python3
# SPDX-License-Identifier: UNLICENSED
"""poc/demo.py — driver de démo HEADLESS du PoC EFE (Scout & Synthesize).

POSTe les 3 prompts scénarisés (CHEWER / BYPASS / compromis) à UN bridge
`efe-preview-server` en marche, parse le flux SSE de chaque mission, et imprime
un tableau de contraste : tokens entrée→sortie, ratio de compression, verdict
(CHEWER / BYPASS), provider, latence, extrait de réponse.

Marche en **keyless** (LLM mock, gratuit) ET en **réel**. La compression
EFFECTIVE (raw > compressed sur le prompt redondant) n'apparaît qu'avec les SLM
réels (`EFE_LIVE_SLM=1` + compressor `:8092` UP) ; en keyless sans SLM, le moteur
émet la métrique mais raw == compressed (pas de compression) — la mécanique SSE
reste démontrée.

Usage :
  ./poc.sh demo                                  # via l'orchestrateur (recommandé)
  python3 scripts/poc/demo.py --url http://127.0.0.1:8700
  python3 scripts/poc/demo.py --url ... --json   # sortie JSON brute (CI)

Les 3 prompts sont la version canonique de `poc-demo-prompts.md`.
"""
from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request

# Remplissage redondant (best-case compression), aligné sur le harness éco.
_FILLER = (
    "Alors voila, honnetement je ne sais plus trop par ou commencer, j'ai lu "
    "enormement de choses un peu partout — des forums, des articles, des videos — "
    "et chacun raconte un peu sa version, donc c'est difficile de s'y retrouver. "
)


def _sized(question: str, target_chars: int) -> str:
    body = ""
    while len(body) + len(question) < target_chars:
        body += _FILLER
    return body + question


# (label, prompt, attendu) — l'attendu sert juste à annoter le tableau.
PROMPTS = [
    (
        "1. CHEWER (redondant)",
        _sized(
            "Bref, ma question : quelle est la difference concrete entre la "
            "programmation synchrone et asynchrone, et quand preferer l'une a "
            "l'autre ? Reponds simplement.",
            1300,
        ),
        "CHEWER",
    ),
    (
        "2. BYPASS (court)",
        "Quelle est la capitale de la France ? Reponds en une phrase.",
        "BYPASS",
    ),
    (
        "3. COMPROMIS (detaille)",
        # Long (>1200 car ≈ >300 tokens) → AU-DESSUS du gate (250). Choisi parce que
        # le compresseur greedy le compresse de façon FIABLE (mesuré H1·a : 326→130
        # ×5, ratio ~0,4). Démontre le compromis brièveté↔complétude (verdict H1·a :
        # justesse ~4/5 mais complétude ~2/5 — visible seulement en mode --real, le
        # mock ignorant le contenu). NB : tous les prompts denses ne chewent pas — le
        # 1B en dégénère certains (ex. idempotence : no-op déterministe).
        "Je dois concevoir la limitation de debit de notre passerelle d'API et je "
        "veux comparer rigoureusement les algorithmes token bucket et leaky bucket "
        "avant de trancher. Le besoin : autoriser des rafales courtes legitimes, par "
        "exemple un client qui synchronise plusieurs ressources d'un coup au reveil "
        "de l'application, tout en bornant le debit moyen sur la duree et en "
        "protegeant le backend d'une surcharge. Pour le token bucket, je voudrais "
        "comprendre comment le seau se remplit a taux constant jusqu'a une capacite "
        "maximale, comment chaque requete consomme un jeton, et pourquoi cette "
        "capacite accumulee pendant les periodes calmes permet d'absorber une rafale "
        "tant que des jetons restent disponibles. Pour le leaky bucket, je veux "
        "saisir comment il impose au contraire un debit de sortie parfaitement "
        "regulier en faisant fuir les requetes a taux fixe, et pourquoi il lisse les "
        "rafales au prix d'une latence d'attente ou d'un rejet quand la file est "
        "pleine. Je veux aussi comparer l'etat a maintenir par client, le "
        "comportement exact aux limites, et l'interaction avec plusieurs instances "
        "de passerelle partageant un compteur central. Au final, indique lequel des "
        "deux tu choisirais pour autoriser des pics courts tout en garantissant un "
        "debit moyen borne, et le raisonnement complet derriere ce choix.",
        "CHEWER (mais complétude à surveiller)",
    ),
]


def run_mission(url: str, prompt: str, timeout: int = 60) -> dict:
    """POST /v1/missions/a + parse SSE → métriques + réponse. Lève en cas d'échec."""
    dto = {"user_text": prompt}
    req = urllib.request.Request(
        f"{url}/v1/missions/a",
        data=json.dumps(dto).encode("utf-8"),
        headers={"content-type": "application/json"},
        method="POST",
    )
    raw = comp = ratio = None
    provider = ""
    latency = 0
    tokens: dict[int, str] = {}
    with urllib.request.urlopen(req, timeout=timeout) as r:
        for line in r:
            s = line.decode("utf-8", "replace").strip()
            if not s.startswith("data:"):
                continue
            try:
                ev = json.loads(s[5:].strip())
            except Exception:
                continue
            t = ev.get("type")
            if t == "PRE_CHEWING_METRICS":
                m = ev.get("metrics", {})
                raw = m.get("input_tokens_raw")
                comp = m.get("input_tokens_compressed")
                ratio = m.get("compression_ratio")
            elif t == "LLM_TOKEN_STREAMED":
                tokens[ev.get("index", len(tokens))] = ev.get("token", "")
            elif t == "LLM_FINISHED":
                provider = ev.get("provider", "")
                latency = ev.get("latency_ms", 0)
    answer = "".join(tokens[i] for i in sorted(tokens))
    return {
        "in_raw": raw, "in_comp": comp, "ratio": ratio,
        "provider": provider, "latency_ms": latency, "answer": answer,
    }


def verdict(raw, comp) -> str:
    if raw is None:
        return "?"
    if comp is not None and comp < raw:
        return "CHEWER"
    return "BYPASS/no-op"


def main() -> int:
    ap = argparse.ArgumentParser(description="PoC EFE — démo headless des 3 scénarios")
    ap.add_argument("--url", default="http://127.0.0.1:8700", help="URL du bridge")
    ap.add_argument("--json", action="store_true", help="Sortie JSON brute (CI)")
    ap.add_argument("--timeout", type=int, default=60)
    args = ap.parse_args()

    # Pré-flight : le bridge répond ?
    try:
        with urllib.request.urlopen(f"{args.url}/healthz", timeout=5) as h:
            if h.read().decode().strip() != "ok":
                print(f"⚠ {args.url}/healthz inattendu", file=sys.stderr)
    except Exception as e:  # noqa: BLE001
        print(f"❌ Bridge injoignable ({args.url}) : {e}\n"
              f"   Lance d'abord : ./poc.sh up", file=sys.stderr)
        return 2

    results = []
    for label, prompt, _exp in PROMPTS:
        try:
            r = run_mission(args.url, prompt, args.timeout)
        except urllib.error.HTTPError as e:
            print(f"❌ {label} : HTTP {e.code}", file=sys.stderr)
            return 1
        except Exception as e:  # noqa: BLE001
            print(f"❌ {label} : {type(e).__name__}: {e}", file=sys.stderr)
            return 1
        r["label"] = label
        r["chars"] = len(prompt)
        r["verdict"] = verdict(r["in_raw"], r["in_comp"])
        results.append(r)

    if args.json:
        print(json.dumps(results, ensure_ascii=False, indent=2))
        return 0

    # Tableau de contraste.
    print("═" * 78)
    print(" PoC EFE — démo des 3 scénarios (Mission A live)")
    print("═" * 78)
    print(f" Bridge : {args.url}")
    any_chew = any(r["verdict"] == "CHEWER" for r in results)
    if not any_chew:
        print(" ⚠ Aucune compression effective (raw == compressed) — SLM probablement"
              " inactif.\n   Pour voir le CHEWER : ./poc.sh up (SLM réels) puis ./poc.sh demo.")
    print("─" * 78)
    print(f" {'Scénario':<26} {'car':>5} {'in→out tok':>12} {'ratio':>6} {'verdict':>12} {'lat(ms)':>8}")
    print("─" * 78)
    for r in results:
        raw = r["in_raw"] if r["in_raw"] is not None else "?"
        comp = r["in_comp"] if r["in_comp"] is not None else "?"
        ratio = f"{r['ratio']:.2f}" if isinstance(r.get("ratio"), (int, float)) else "—"
        print(f" {r['label']:<26} {r['chars']:>5} {f'{raw}→{comp}':>12} "
              f"{ratio:>6} {r['verdict']:>12} {r['latency_ms']:>8}")
    print("─" * 78)
    for r in results:
        snippet = " ".join(r["answer"].split())[:72]
        print(f" {r['label']}\n   provider={r['provider']}  →  « {snippet}{'…' if len(r['answer']) > 72 else ''} »")
    print("═" * 78)
    print(" Lecture : CHEWER = compression effective (in>out) ; BYPASS/no-op = sous le")
    print("           gate ou SLM inactif. Verdict mesuré : cf. poc-demo-prompts.md.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
