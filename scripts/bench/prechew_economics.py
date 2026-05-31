#!/usr/bin/env python3
# SPDX-License-Identifier: UNLICENSED
"""
prechew_economics.py — Roadmap H0.1 : mesure A/B de l'économie réelle du
pré-mâchage contre un VRAI LLM (Claude / OpenAI), pas le mock.

Le premier sweep (mock) ne mesurait que la réduction de tokens + la latence SLM.
Ici on mesure le **bénéfice côté LLM premium** : coût $ et latence réellement
économisés, et le verdict net (le SLM amont en vaut-il la peine ?).

═══════════════════════════════════════════════════════════════════════════
  SÉCURITÉ — clés API
═══════════════════════════════════════════════════════════════════════════
Ce script ne touche JAMAIS aux clés API. C'est TOI qui lances deux bridges
`real-llm` avec tes clés en environnement (côté serveur). Le script ne tape que
le HTTP du bridge ; il ne voit jamais les clés et n'en logge aucune.

═══════════════════════════════════════════════════════════════════════════
  PROTOCOLE A/B — lancer DEUX bridges real-llm (clés Claude/OpenAI en env)
═══════════════════════════════════════════════════════════════════════════
  # Terminal 1 — BASELINE (pré-mâchage OFF → le LLM reçoit le prompt BRUT)
  EFE_LIVE_SLM=0 EFE_PREVIEW_ADDR=127.0.0.1:8700 \
    cargo run -p efe-preview-server --features real-llm

  # Terminal 2 — CHEWED (pré-mâchage ON → le LLM reçoit le prompt COMPRESSÉ)
  #   (les serveurs SLM doivent tourner : ./scripts/slm/start.sh)
  EFE_LIVE_SLM=1 EFE_PREVIEW_ADDR=127.0.0.1:8701 \
    cargo run -p efe-preview-server --features real-llm

  # Terminal 3 — le sweep
  python3 scripts/bench/prechew_economics.py \
    --model claude-3-5-sonnet --repeats 3 --confirm

NB : sans `--confirm`, le script s'arrête après l'estimation de dépense.
NB : les deux bridges DOIVENT être en `--features real-llm` SANS
     EFE_PREVIEW_MOCK (sinon c'est le mock → coût/latence faux ; le script le
     détecte via le provider `mock-*` et avertit).
"""
from __future__ import annotations

import argparse
import json
import math
import statistics
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field, asdict
from datetime import datetime, timezone

# ─────────────────────────────────────────────────────────────────────────
# Table de pricing — $ par MILLION de tokens (input, output).
# ⚠ PRIX INDICATIFS (≈2024) — VÉRIFIER + METTRE À JOUR avant tout run réel.
#   Pour un modèle absent : passer --price-in / --price-out (en $/Mtok).
# ─────────────────────────────────────────────────────────────────────────
PRICING_USD_PER_MTOK: dict[str, tuple[float, float]] = {
    "claude-3-5-sonnet": (3.00, 15.00),
    "claude-3-5-haiku": (0.80, 4.00),
    "claude-3-opus": (15.00, 75.00),
    "gpt-4o": (2.50, 10.00),
    "gpt-4o-mini": (0.15, 0.60),
}

# ─────────────────────────────────────────────────────────────────────────
# Jeu de prompts — RÉALISTES (le LLM produit une vraie réponse jugeable),
# au-dessus du gate (~250 t ≈ 1000 car) pour que la compression tourne.
# `expect` (optionnel) = mots-clés attendus dans la réponse (contrôle factuel).
# `kind` : "redundant" (best-case compression) ou "varied" (réaliste).
# ─────────────────────────────────────────────────────────────────────────
_FILLER = (
    "Alors voilà, honnêtement je ne sais plus trop par où commencer, j'ai lu "
    "énormément de choses un peu partout — des forums, des articles, des vidéos — "
    "et chacun raconte un peu sa version, donc c'est difficile de s'y retrouver. "
)


def _sized(question: str, target_chars: int) -> str:
    """Préfixe `question` de filler redondant jusqu'à ~target_chars."""
    body = ""
    while len(body) + len(question) < target_chars:
        body += _FILLER
    return body + question


@dataclass
class Prompt:
    id: str
    kind: str
    text: str
    expect: list[str] = field(default_factory=list)


PROMPTS: list[Prompt] = [
    Prompt(
        "fr_capital_1k",
        "redundant",
        _sized("Bref, ma question : quelle est la capitale de la France ? "
               "Réponds en une phrase.", 1100),
        expect=["Paris"],
    ),
    Prompt(
        "syncasync_1k5",
        "redundant",
        _sized("Bref, ma vraie question : quelle est la différence concrète "
               "entre la programmation synchrone et asynchrone, et quand "
               "préférer l'une à l'autre ?", 1500),
    ),
    Prompt(
        "rest_graphql_2k5",
        "redundant",
        _sized("Au final ma question : quelles sont les différences "
               "fondamentales entre une API REST et une API GraphQL, et dans "
               "quels cas choisir l'une plutôt que l'autre ?", 2500),
    ),
    Prompt(
        "varied_backend",
        "varied",
        # Texte VARIÉ (non-redondant, ~1700 car) → savings plus réalistes / bas,
        # mais au-dessus du gate pour que la compression tourne.
        ("Je conçois le backend d'une application de réservation de créneaux. Les "
         "utilisateurs recherchent des disponibilités, réservent, modifient et "
         "annulent ; le trafic est très inégal, avec des pics le lundi matin et "
         "le vendredi soir, et des creux la nuit. Le modèle de données couvre les "
         "utilisateurs, les ressources réservables, les créneaux horaires, les "
         "réservations, les paiements et les notifications. Côté cohérence, je "
         "redoute les doubles réservations sur un même créneau sous forte "
         "concurrence : faut-il un verrouillage pessimiste au niveau de la ligne "
         "créneau, ou une approche optimiste avec un numéro de version et un retry "
         "applicatif ? Pour les paiements, le client peut renvoyer la même requête "
         "après un timeout réseau ; comment garantir l'idempotence sans "
         "double-débit — une clé d'idempotence stockée côté serveur suffit-elle ? "
         "Les notifications (email, SMS, push) ne doivent pas ralentir la "
         "confirmation : vaut-il mieux les sortir du chemin critique via une file "
         "de messages, quitte à accepter une livraison au-mieux ? Pour la lecture "
         "des disponibilités à fort trafic, un cache en mémoire avec invalidation "
         "à l'écriture tient-il la charge, ou faut-il précalculer une vue "
         "matérialisée rafraîchie périodiquement ? Enfin, pour l'observabilité, "
         "quelles métriques suivre en priorité pour détecter une dégradation avant "
         "les utilisateurs ? Ma question la plus concrète, pour commencer : entre "
         "verrouillage pessimiste et verrouillage optimiste pour réserver un "
         "créneau unique sous forte concurrence, lequel choisir, et pourquoi ?"),
    ),
]

TERMINAL = {"SWARM_FINISHED", "SWARM_FAILED"}


# ─────────────────────────────────────────────────────────────────────────
# Client SSE — une mission, un flux d'events parsés.
# ─────────────────────────────────────────────────────────────────────────
@dataclass
class MissionResult:
    ok: bool
    provider: str = ""
    answer: str = ""
    output_tokens: int = 0
    llm_latency_ms: int = 0
    llm_cost_cents: int = 0           # garde-fou (arrondi)
    input_tokens_raw: int = 0
    input_tokens_compressed: int = 0
    slm_latency_ms: int = 0
    failed_components: list[str] = field(default_factory=list)
    error: str = ""


def run_mission(url: str, prompt: str, model, max_output: int,
                timeout: int = 120) -> MissionResult:
    # `model=None` → on n'épingle pas : le bridge utilise son `default_llms`
    # (l'unique provider configuré). Sinon on épingle l'id provider exact.
    dto: dict = {"user_text": prompt}
    if model:
        dto["available_llms"] = [model]
    req = urllib.request.Request(
        f"{url}/v1/missions/a",
        data=json.dumps(dto).encode("utf-8"),
        headers={"content-type": "application/json"},
        method="POST",
    )
    tokens: dict[int, str] = {}
    res = MissionResult(ok=False)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            for raw in r:
                line = raw.decode("utf-8", "replace").strip()
                if not line.startswith("data:"):
                    continue
                try:
                    ev = json.loads(line[5:].strip())
                except Exception:
                    continue
                t = ev.get("type")
                if t == "LLM_TOKEN_STREAMED":
                    tokens[ev.get("index", len(tokens))] = ev.get("token", "")
                elif t == "LLM_FINISHED":
                    res.provider = ev.get("provider", "")
                    res.output_tokens = ev.get("tokens_generated", 0)
                    res.llm_latency_ms = ev.get("latency_ms", 0)
                    res.llm_cost_cents = ev.get("cost_cents", 0)
                elif t == "PRE_CHEWING_METRICS":
                    m = ev.get("metrics", {})
                    res.input_tokens_raw = m.get("input_tokens_raw", 0)
                    res.input_tokens_compressed = m.get("input_tokens_compressed", 0)
                    res.slm_latency_ms = m.get("total_slm_latency_ms", 0)
                    res.failed_components = m.get("failed_components", []) or []
        res.answer = "".join(tokens[i] for i in sorted(tokens))
        res.ok = True
        return res
    except urllib.error.HTTPError as e:
        res.error = f"HTTP {e.code}: {e.read().decode('utf-8', 'replace')[:200]}"
        return res
    except Exception as e:  # noqa: BLE001
        res.error = f"{type(e).__name__}: {e}"
        return res


# ─────────────────────────────────────────────────────────────────────────
# Coût (calculé depuis tokens × pricing ; cost_cents arrondit → on ne s'y fie pas)
# ─────────────────────────────────────────────────────────────────────────
def cost_usd(input_tok: int, output_tok: int, price_in: float, price_out: float) -> float:
    return (input_tok / 1e6) * price_in + (output_tok / 1e6) * price_out


# ─────────────────────────────────────────────────────────────────────────
# Qualité — divergence baseline↔chewed (sans LLM-juge : proxy honnête + texte
# brut sauvegardé pour revue humaine).
# ─────────────────────────────────────────────────────────────────────────
def _words(s: str) -> set[str]:
    return {w for w in "".join(c.lower() if c.isalnum() else " " for c in s).split() if w}


def jaccard(a: str, b: str) -> float:
    wa, wb = _words(a), _words(b)
    if not wa and not wb:
        return 1.0
    inter = len(wa & wb)
    union = len(wa | wb)
    return inter / union if union else 0.0


def keyword_hit(answer: str, expect: list[str]) -> float | None:
    if not expect:
        return None
    low = answer.lower()
    return sum(1 for k in expect if k.lower() in low) / len(expect)


# ─────────────────────────────────────────────────────────────────────────
# Orchestration A/B
# ─────────────────────────────────────────────────────────────────────────
def post_with_backoff(url, prompt, model, max_output, pace, attempts=4):
    delay = 2.0
    for i in range(attempts):
        r = run_mission(url, prompt, model, max_output)
        if r.ok and not r.error:
            time.sleep(pace)
            return r
        # backoff sur erreur (429/5xx/transport)
        if i < attempts - 1:
            time.sleep(delay)
            delay *= 2
    return r  # dernier essai (échoué)


def aggregate(values: list[float]) -> tuple[float, float]:
    if not values:
        return (0.0, 0.0)
    return (statistics.mean(values),
            statistics.pstdev(values) if len(values) > 1 else 0.0)


def main() -> int:
    ap = argparse.ArgumentParser(description="H0.1 — économie réelle du pré-mâchage (A/B vrai LLM)")
    ap.add_argument("--baseline-url", default="http://127.0.0.1:8700",
                    help="Bridge real-llm EFE_LIVE_SLM=0 (prompt brut)")
    ap.add_argument("--chewed-url", default="http://127.0.0.1:8701",
                    help="Bridge real-llm EFE_LIVE_SLM=1 (prompt compressé)")
    ap.add_argument("--model", default=None,
                    help="Id provider exact à épingler (ex. claude_claude-sonnet-4-6, "
                         "gpt_gpt-4o). OMETTRE → le bridge utilise son unique provider "
                         "configuré (recommandé si une seule clé). Avec un id non "
                         "standard, fournir --price-in/--price-out.")
    ap.add_argument("--price-in", type=float, help="Override prix input $/Mtok")
    ap.add_argument("--price-out", type=float, help="Override prix output $/Mtok")
    ap.add_argument("--repeats", type=int, default=3, help="Répétitions par (prompt×condition)")
    ap.add_argument("--max-output-tokens", type=int, default=400)
    ap.add_argument("--pace", type=float, default=1.0, help="Pause (s) entre appels")
    ap.add_argument("--out-dir", default="docs")
    ap.add_argument("--confirm", action="store_true", help="Confirme la dépense réelle")
    args = ap.parse_args()

    # Pricing résolu (override prioritaire ; sinon table ; sinon erreur claire)
    if args.price_in is not None and args.price_out is not None:
        price_in, price_out = args.price_in, args.price_out
    elif args.model and args.model in PRICING_USD_PER_MTOK:
        price_in, price_out = PRICING_USD_PER_MTOK[args.model]
    else:
        print(f"❌ Pricing introuvable pour '{args.model or '(défaut du bridge)'}'. "
              f"Passe --price-in ET --price-out (en $/Mtok), "
              f"ou --model parmi {list(PRICING_USD_PER_MTOK)}.", file=sys.stderr)
        return 2

    n_calls = len(PROMPTS) * args.repeats * 2
    print("═" * 70)
    print(" H0.1 — Économie réelle du pré-mâchage (A/B vrai LLM)")
    print("═" * 70)
    print(f" Modèle      : {args.model or '(défaut du bridge)'}  (in ${price_in}/Mtok · out ${price_out}/Mtok)")
    print(f" Baseline    : {args.baseline_url}  (EFE_LIVE_SLM=0)")
    print(f" Chewed      : {args.chewed_url}  (EFE_LIVE_SLM=1)")
    print(f" Prompts     : {len(PROMPTS)} × {args.repeats} rép × 2 conditions = {n_calls} appels LLM réels")
    # Estimation grossière de dépense (≈ in 800 t + out 300 t par appel)
    est = n_calls * cost_usd(800, 300, price_in, price_out)
    print(f" Dépense est.: ~${est:.3f} (ordre de grandeur ; dépend des réponses)")
    print("═" * 70)

    if not args.confirm:
        print("\nℹ️  Estimation seule (les bridges n'ont pas besoin d'être lancés). "
              "Relance avec --confirm pour la dépense réelle.")
        return 0

    # Pré-flight : les 2 bridges répondent ? (seulement quand on va dépenser)
    for label, url in [("baseline", args.baseline_url), ("chewed", args.chewed_url)]:
        try:
            with urllib.request.urlopen(f"{url}/healthz", timeout=5) as h:
                if h.read().decode().strip() != "ok":
                    print(f"⚠ {label} {url}/healthz inattendu", file=sys.stderr)
        except Exception as e:  # noqa: BLE001
            print(f"❌ {label} bridge injoignable ({url}) : {e}", file=sys.stderr)
            return 2

    # ── Smoke 1 appel : détecte le provider réel (ou un repli mock) ──
    smoke = run_mission(args.chewed_url, "Test.", args.model, 16)
    if not smoke.ok or not smoke.provider:
        print(f"❌ Smoke KO sur {args.chewed_url} : {smoke.error or 'pas de provider'}.",
              file=sys.stderr)
        return 2
    if smoke.provider.startswith("mock-"):
        print(f"❌ Provider MOCK détecté ('{smoke.provider}') → AUCUNE clé API "
              f"chargée. Exporte ANTHROPIC_API_KEY et/ou OPENAI_API_KEY AVANT de "
              f"lancer les bridges (en `--features real-llm`, sans EFE_PREVIEW_MOCK).",
              file=sys.stderr)
        return 2
    print(f"   provider réel détecté : {smoke.provider}")

    # ── Run A/B ──
    rows = []
    for p in PROMPTS:
        print(f"\n▶ {p.id} ({p.kind}, {len(p.text)} car)")
        reps = []
        for k in range(args.repeats):
            b = post_with_backoff(args.baseline_url, p.text, args.model, args.max_output_tokens, args.pace)
            c = post_with_backoff(args.chewed_url, p.text, args.model, args.max_output_tokens, args.pace)
            if not (b.ok and c.ok):
                print(f"   rép {k}: ÉCHEC (baseline={b.error or 'ok'} chewed={c.error or 'ok'})")
                continue
            cost_b = cost_usd(b.input_tokens_raw, b.output_tokens, price_in, price_out)
            cost_c = cost_usd(c.input_tokens_compressed, c.output_tokens, price_in, price_out)
            net_latency_ms = (c.slm_latency_ms + c.llm_latency_ms) - b.llm_latency_ms
            reps.append({
                "in_raw": b.input_tokens_raw, "in_comp": c.input_tokens_compressed,
                "out_b": b.output_tokens, "out_c": c.output_tokens,
                "cost_b": cost_b, "cost_c": cost_c, "dcost": cost_b - cost_c,
                "llm_lat_b": b.llm_latency_ms, "llm_lat_c": c.llm_latency_ms,
                "slm_lat_c": c.slm_latency_ms, "net_latency_ms": net_latency_ms,
                "jaccard": jaccard(b.answer, c.answer),
                "kw_b": keyword_hit(b.answer, p.expect), "kw_c": keyword_hit(c.answer, p.expect),
                "answer_b": b.answer, "answer_c": c.answer,
                "failed_c": c.failed_components,
            })
            print(f"   rép {k}: Δcost=${cost_b - cost_c:+.5f}  net_latency={net_latency_ms:+d}ms  "
                  f"jaccard={jaccard(b.answer, c.answer):.2f}")
        if not reps:
            continue
        agg = {
            "id": p.id, "kind": p.kind, "chars": len(p.text), "n": len(reps),
            "in_raw": round(statistics.mean(r["in_raw"] for r in reps)),
            "in_comp": round(statistics.mean(r["in_comp"] for r in reps)),
            "savings_ratio": 1 - statistics.mean(r["in_comp"] for r in reps) /
                                 max(1, statistics.mean(r["in_raw"] for r in reps)),
            "dcost_usd": aggregate([r["dcost"] for r in reps]),
            "net_latency_ms": aggregate([float(r["net_latency_ms"]) for r in reps]),
            "slm_lat_ms": aggregate([float(r["slm_lat_c"]) for r in reps]),
            "jaccard": aggregate([r["jaccard"] for r in reps]),
            "kw_b": aggregate([r["kw_b"] for r in reps if r["kw_b"] is not None]),
            "kw_c": aggregate([r["kw_c"] for r in reps if r["kw_c"] is not None]),
            "reps": reps,
        }
        rows.append(agg)

    if not rows:
        print("\n❌ Aucune mesure exploitable.", file=sys.stderr)
        return 1

    # ── Écriture JSON brut (ré-analyse sans re-dépenser) + rapport markdown ──
    stamp = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    base = f"{args.out_dir}/EFE_PRECHEWING_ECONOMICS_{stamp}"
    payload = {
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "model": args.model or smoke.provider, "price_in_usd_mtok": price_in, "price_out_usd_mtok": price_out,
        "baseline_url": args.baseline_url, "chewed_url": args.chewed_url,
        "repeats": args.repeats, "rows": rows,
    }
    with open(f"{base}.json", "w", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False, indent=2)
    write_report(f"{base}.md", payload)
    print(f"\n✅ Rapport : {base}.md   ·   données brutes : {base}.json")
    return 0


def write_report(path: str, p: dict) -> None:
    rows = p["rows"]

    def m(agg):  # (mean, std)
        return f"{agg[0]:+.5f} ± {agg[1]:.5f}"

    def mi(agg):
        return f"{agg[0]:+.0f} ± {agg[1]:.0f}"

    lines = [
        f"# EFE — Économie réelle du pré-mâchage (vrai LLM) · {p['model']}",
        "",
        f"> Roadmap **H0.1**. A/B contrôlé (baseline brut vs chewed compressé) sur "
        f"modèle épinglé, coût **calculé** depuis tokens × pricing "
        f"(in ${p['price_in_usd_mtok']}/Mtok · out ${p['price_out_usd_mtok']}/Mtok). "
        f"Généré : {p['generated_utc']}. Données brutes : le `.json` à côté.",
        "",
        "## Verdict par prompt",
        "",
        "| Prompt | type | savings tokens | Δcoût (USD) | latence NETTE (ms) | overhead SLM (ms) | qualité (jaccard) | factuel base/chewed |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for r in rows:
        kw = ("—" if not r["kw_b"] or r["kw_b"][0] is None
              else f"{r['kw_b'][0]:.2f}/{r['kw_c'][0]:.2f}")
        lines.append(
            f"| {r['id']} | {r['kind']} | {r['savings_ratio']:+.0%} "
            f"({r['in_raw']}→{r['in_comp']} t) | {m(r['dcost_usd'])} | "
            f"{mi(r['net_latency_ms'])} | {mi(r['slm_lat_ms'])} | "
            f"{r['jaccard'][0]:.2f} ± {r['jaccard'][1]:.2f} | {kw} |"
        )
    # Lecture
    lines += [
        "",
        "## Comment lire",
        "",
        "- **Δcoût > 0** → le pré-mâchage **économise** sur le coût LLM (le coût "
        "SLM local ≈ nul). **Δcoût < 0** → l'overhead l'emporte (rare : dérive de "
        "longueur de sortie).",
        "- **latence NETTE = (SLM + LLM_chewed) − LLM_baseline**. **< 0** → chewed "
        "est PLUS RAPIDE bout-en-bout (win latence). **> 0** → chewed est plus LENT "
        "(le SLM amont coûte plus que le temps LLM gagné).",
        "- **qualité (jaccard)** : recouvrement de mots baseline↔chewed. **~1** = "
        "réponses concordantes ; **bas** = la compression a changé la réponse → "
        "inspecter `answer_b`/`answer_c` dans le `.json`.",
        "- **factuel** : fraction de mots-clés attendus présents (base/chewed) ; "
        "doit rester égal pour valider la qualité.",
        "",
        "## Caveats méthodologiques",
        "",
        "- **Température non réglable via le DTO** (`MissionADto` n'expose pas "
        "`temperature`) → la variance LLM est contrôlée par les répétitions "
        "(moyenne ± σ), pas par `temperature=0`.",
        "- **Tokens d'entrée estimés** (`chars/4`, via `PRE_CHEWING_METRICS`), pas "
        "le compte exact du tokenizer provider ; `cost_cents` du flux sert de "
        "garde-fou (mais arrondit). Le coût du tableau est calculé sur l'estimation.",
        "- **Répétitions à prompt identique** → le cache de prompt SLM "
        "(`cache_prompt=true`) peut accélérer les répétitions 2-3 → l'overhead SLM "
        "mesuré est plutôt un **plancher** (biais favorable au chewed, mais "
        "réaliste en prod).",
        "- **Coût SLM compute non monétisé** : supposé ≈ nul (self-hosted CPU/GPU "
        "local). Si SLM facturé, soustraire son coût du Δcoût.",
        "",
        "## Décision (à compléter)",
        "",
        "> Phrase-titre attendue (débloque H0.3 — positionnement) :",
        "> « Le pré-mâchage paie sur le **COÛT** dès N tokens (−X%) ; sur la "
        "**LATENCE** [oui/non] aux vitesses actuelles ; **qualité** préservée "
        "[oui/sous conditions]. »",
        "",
        "⚠️ Rappel : ces prompts incluent des `redundant` (best-case compression "
        "→ savings **optimistes**) ; les `varied` bornent le réalisme (cf. H1.1).",
        "",
    ]
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))


if __name__ == "__main__":
    raise SystemExit(main())
