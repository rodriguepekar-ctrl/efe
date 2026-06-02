#!/usr/bin/env python3
# SPDX-License-Identifier: UNLICENSED
"""
prechew_economics.py — mesure A/B de l'economie reelle du pre-machage contre un
VRAI LLM (Claude / OpenAI), pas le mock.

Roadmap :
  - H0.1 : premier verdict A/B (baseline brut vs chewed compresse) sur ~4 prompts.
  - H1.1 (CE FICHIER, etendu) : CORPUS STRATIFIE par FAMILLE de prompt, avec
    decomposition du cout ENTREE vs SORTIE (le vrai levier), rollup par famille,
    qualite honnete (rubric deterministe + juge LLM optionnel), et generation
    d'une REGLE DE ROUTAGE (chewer / ne pas chewer / conditionnel).

Ce que H1.1 ajoute a H0.1 (sans casser la retro-compat) :
  M1  loader de corpus externe (--corpus PATH ; fallback sur 4 prompts embarques) ;
  M2  decomposition du cout : dcost_in vs dcost_out + part_sortie + reduction_sortie ;
  M3  rollup par famille (moyenne +/- sigma) : F1 redondant, F2 propre, F3 court, F4 long ;
  M4  qualite : rubric `expect`-coverage (gratuit) + --judge (LLM-juge, paye) ;
      le jaccard est CONSERVE mais RETROGRADE au rang de "divergence de style".
  M5  rapport stratifie : verdict par famille + decomposition sortie + regle de routage.

═══════════════════════════════════════════════════════════════════════════
  SECURITE — cles API
═══════════════════════════════════════════════════════════════════════════
Ce script ne touche JAMAIS aux cles API. C'est TOI qui lances deux bridges
`real-llm` avec tes cles en environnement (cote serveur). Le script ne tape que
le HTTP du bridge ; il ne voit jamais les cles et n'en logge aucune.

═══════════════════════════════════════════════════════════════════════════
  PROTOCOLE — passe 1 KEYLESS (gratuit) PUIS passe 2 LLM REEL (chiffree)
═══════════════════════════════════════════════════════════════════════════
  # PASSE 1 — keyless : valide toute la plomberie a 0$ (mock LLM + SLM reel).
  EFE_PREVIEW_MOCK=1 EFE_LIVE_SLM=0 EFE_PREVIEW_ADDR=127.0.0.1:8700 \
    cargo run -p efe-preview-server --features real-llm
  EFE_PREVIEW_MOCK=1 EFE_LIVE_SLM=1 EFE_PREVIEW_ADDR=127.0.0.1:8701 \
    cargo run -p efe-preview-server --features real-llm        # SLM via scripts/slm/start.sh
  python3 scripts/bench/prechew_economics.py \
    --corpus scripts/bench/corpus/prechew_corpus_v1.json \
    --price-in 3 --price-out 15 --confirm        # cout 0$, verifie loader/rollup/rubric

  # PASSE 2 — LLM reel : memes commandes SANS EFE_PREVIEW_MOCK, cles en env.
  #   (le smoke detecte un repli mock via le provider `mock-*` et avertit.)
  python3 scripts/bench/prechew_economics.py \
    --corpus scripts/bench/corpus/prechew_corpus_v1.json \
    --price-in 3 --price-out 15 [--judge] --confirm

NB : sans `--confirm`, le script s'arrete apres l'estimation de depense.
NB : sans `--corpus`, comportement H0.1 identique (4 prompts embarques).
"""
from __future__ import annotations

import argparse
import json
import os
import re
import statistics
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from datetime import datetime, timezone

# ─────────────────────────────────────────────────────────────────────────
# Table de pricing — $ par MILLION de tokens (input, output).
# ⚠ PRIX INDICATIFS (≈2024) — VERIFIER + METTRE A JOUR avant tout run reel.
#   Pour un modele absent : passer --price-in / --price-out (en $/Mtok).
# ─────────────────────────────────────────────────────────────────────────
PRICING_USD_PER_MTOK: dict[str, tuple[float, float]] = {
    "claude-3-5-sonnet": (3.00, 15.00),
    "claude-3-5-haiku": (0.80, 4.00),
    "claude-3-opus": (15.00, 75.00),
    "gpt-4o": (2.50, 10.00),
    "gpt-4o-mini": (0.15, 0.60),
}

# ─────────────────────────────────────────────────────────────────────────
# Remplissage redondant — utilise par le corpus (famille redundant_verbose) et
# par les 4 prompts embarques de fallback (retro-compat H0.1).
# ─────────────────────────────────────────────────────────────────────────
_FILLER = (
    "Alors voila, honnetement je ne sais plus trop par ou commencer, j'ai lu "
    "enormement de choses un peu partout — des forums, des articles, des videos — "
    "et chacun raconte un peu sa version, donc c'est difficile de s'y retrouver. "
)


def _sized(question: str, target_chars: int) -> str:
    """Prefixe `question` de filler redondant jusqu'a ~target_chars."""
    body = ""
    while len(body) + len(question) < target_chars:
        body += _FILLER
    return body + question


# ─────────────────────────────────────────────────────────────────────────
# Familles de prompt — chacune teste une hypothese economique distincte.
# ─────────────────────────────────────────────────────────────────────────
VALID_FAMILIES = (
    "redundant_verbose",        # F1 — best-case : CHEWER
    "clean_concise_long",       # F2 — propre/dense : NE PAS chewer (non-nuisance)
    "short_below_gate",         # F3 — sous le gate : BYPASS (NO-OP)
    "long_technical_multipart",  # F4 — long realiste : CONDITIONNEL
)
FAMILY_ORDER = list(VALID_FAMILIES)
FAMILY_LABEL = {
    "redundant_verbose": "F1 redondant/verbeux",
    "clean_concise_long": "F2 propre/concis long",
    "short_below_gate": "F3 court (sous-gate)",
    "long_technical_multipart": "F4 long technique",
}
# Regle de routage attendue PAR DESIGN (le measured tag, lui, vient des chiffres).
FAMILY_DESIGN_RULE = {
    "redundant_verbose": "CHEWER",
    "clean_concise_long": "NE PAS chewer",
    "short_below_gate": "BYPASS (gate)",
    "long_technical_multipart": "CONDITIONNEL",
}

# En-dessous de ce |Δcout| (USD), la part_sortie = Δcout_out/Δcout_total est
# numeriquement instable (division par ~0) → on l'affiche "—" plutot qu'un
# pourcentage trompeur (+100%/+0%). Cas typique : prompt NON compresse (sous-gate).
NEGLIGIBLE_DCOST_USD = 2e-4


def _share_cell(dcost_mean: float, share_mean: float) -> str:
    """Part-sortie affichable : '—' si le Δcout est negligeable (sinon trompeur)."""
    if abs(dcost_mean) < NEGLIGIBLE_DCOST_USD:
        return "—"
    return f"{share_mean:+.0%}"


@dataclass
class Prompt:
    id: str
    family: str
    text: str                                   # envoye aux bridges (peut etre padde)
    expect: list[str] = field(default_factory=list)
    core: str = ""                              # question essentielle (pour le juge)
    notes: str = ""

    def __post_init__(self) -> None:
        if not self.core:
            self.core = self.text
        if self.family not in VALID_FAMILIES:
            raise ValueError(
                f"prompt '{self.id}' : famille inconnue '{self.family}' "
                f"(attendu : {', '.join(VALID_FAMILIES)})")


# ─────────────────────────────────────────────────────────────────────────
# M1 — Corpus loader (externe) avec FALLBACK embarque (retro-compat H0.1).
# ─────────────────────────────────────────────────────────────────────────
_EMBEDDED_PROMPTS: list[Prompt] = [
    Prompt("fr_capital_1k", "redundant_verbose",
           _sized("Bref, ma question : quelle est la capitale de la France ? "
                  "Reponds en une phrase.", 1100),
           expect=["paris"]),
    Prompt("syncasync_1k5", "redundant_verbose",
           _sized("Bref, ma vraie question : quelle est la difference concrete "
                  "entre la programmation synchrone et asynchrone, et quand "
                  "preferer l'une a l'autre ?", 1500),
           expect=["asynchron"]),
    Prompt("rest_graphql_2k5", "redundant_verbose",
           _sized("Au final ma question : quelles sont les differences "
                  "fondamentales entre une API REST et une API GraphQL, et dans "
                  "quels cas choisir l'une plutot que l'autre ?", 2500),
           expect=["graphql", "rest"]),
    Prompt("varied_backend", "long_technical_multipart",
           ("Je concois le backend d'une application de reservation de creneaux. "
            "Les utilisateurs recherchent des disponibilites, reservent, modifient "
            "et annulent ; le trafic est tres inegal. Cote coherence, je redoute "
            "les doubles reservations sur un meme creneau sous forte concurrence : "
            "faut-il un verrouillage pessimiste au niveau de la ligne creneau, ou "
            "une approche optimiste avec un numero de version et un retry "
            "applicatif ? Pour les paiements, le client peut renvoyer la meme "
            "requete apres un timeout reseau ; comment garantir l'idempotence sans "
            "double-debit ? Ma question la plus concrete : entre verrouillage "
            "pessimiste et optimiste pour reserver un creneau unique sous forte "
            "concurrence, lequel choisir, et pourquoi ?"),
           core=("Entre verrouillage pessimiste et optimiste pour reserver un "
                 "creneau unique sous forte concurrence, lequel choisir et pourquoi ?"),
           expect=["verrouillage", "concurrence"]),
]


def load_corpus(path: str | None) -> tuple[list[Prompt], str]:
    """Charge le corpus depuis `path` (JSON) ; sinon (None ou absent) renvoie le
    fallback embarque. Retourne (prompts, source_label).
    Format attendu : {"prompts": [{id, family, (text | pad_to+question),
    expect?, core?, notes?}, ...]}.
    """
    if not path:
        return list(_EMBEDDED_PROMPTS), "embarque (H0.1, 4 prompts)"
    if not os.path.exists(path):
        print(f"⚠ Corpus introuvable '{path}' → fallback embarque (4 prompts).",
              file=sys.stderr)
        return list(_EMBEDDED_PROMPTS), f"fallback embarque (corpus absent: {path})"
    with open(path, "r", encoding="utf-8") as f:
        data = json.load(f)
    raw = data.get("prompts")
    if not isinstance(raw, list) or not raw:
        raise ValueError(f"Corpus '{path}' : champ 'prompts' vide ou absent.")
    prompts: list[Prompt] = []
    seen: set[str] = set()
    for i, e in enumerate(raw):
        pid = e.get("id") or f"prompt_{i}"
        if pid in seen:
            raise ValueError(f"Corpus '{path}' : id duplique '{pid}'.")
        seen.add(pid)
        fam = e.get("family")
        if "pad_to" in e and "question" in e:
            text = _sized(str(e["question"]), int(e["pad_to"]))
            core = str(e["question"])
        elif "text" in e:
            text = str(e["text"])
            core = str(e.get("core") or text)
        else:
            raise ValueError(
                f"Corpus '{path}' : prompt '{pid}' sans 'text' ni 'pad_to'+'question'.")
        prompts.append(Prompt(
            id=pid, family=fam, text=text,
            expect=[str(x) for x in e.get("expect", [])],
            core=core, notes=str(e.get("notes", "")),
        ))
    ver = data.get("version", "?")
    return prompts, f"{path} (version {ver}, {len(prompts)} prompts)"


TERMINAL = {"SWARM_FINISHED", "SWARM_FAILED"}


# ─────────────────────────────────────────────────────────────────────────
# Client SSE — une mission, un flux d'events parses.
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


def run_mission(url: str, prompt: str, model, timeout: int = 120) -> MissionResult:
    # `model=None` → on n'epingle pas : le bridge utilise son `default_llms`
    # (l'unique provider configure). Sinon on epingle l'id provider exact.
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
# Cout (calcule depuis tokens × pricing ; cost_cents arrondit → on ne s'y fie pas)
# ─────────────────────────────────────────────────────────────────────────
def cost_usd(input_tok: int, output_tok: int, price_in: float, price_out: float) -> float:
    return (input_tok / 1e6) * price_in + (output_tok / 1e6) * price_out


# ─────────────────────────────────────────────────────────────────────────
# Qualite.
#   - jaccard : RETROGRADE → "divergence de style" (PAS un signal de qualite).
#   - rubric_coverage : couverture des concepts attendus (deterministe, gratuit).
#   - run_judge : LLM-juge optionnel (justesse + completude), paye.
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


def rubric_coverage(answer: str, expect: list[str]) -> float | None:
    """Fraction des concepts `expect` (sous-chaines, casse ignoree) presents."""
    if not expect:
        return None
    low = answer.lower()
    return sum(1 for k in expect if k.lower() in low) / len(expect)


_JUDGE_TMPL = (
    "Tu es un evaluateur strict et concis. Voici une QUESTION, puis deux reponses "
    "A (reference) et B (a evaluer).\n\n"
    "QUESTION:\n{q}\n\n"
    "REPONSE A (reference):\n{a}\n\n"
    "REPONSE B (a evaluer):\n{b}\n\n"
    "Evalue UNIQUEMENT la reponse B vis-a-vis de la QUESTION. Reponds STRICTEMENT "
    "en JSON sur une seule ligne, sans aucun autre texte :\n"
    '{{"correct": <entier 1-5>, "complete": <entier 1-5>}}\n'
    "ou correct = justesse factuelle de B, complete = completude de B."
)


def run_judge(judge_url: str, question: str, answer_b: str, answer_c: str,
              model) -> dict | None:
    """Juge la reponse chewed (B) via le bridge baseline (brut, non chewe).
    Retourne {"correct": 1-5, "complete": 1-5} ou None si parsing impossible."""
    prompt = _JUDGE_TMPL.format(
        q=question[:2000], a=answer_b[:2000], b=answer_c[:2000])
    r = run_mission(judge_url, prompt, model)
    if not r.ok or not r.answer:
        return None
    m = re.search(r"\{[^{}]*\}", r.answer)
    if not m:
        return None
    try:
        obj = json.loads(m.group(0))
        c = float(obj.get("correct"))
        p = float(obj.get("complete"))
        if not (1.0 <= c <= 5.0 and 1.0 <= p <= 5.0):
            return None
        return {"correct": c, "complete": p}
    except Exception:  # noqa: BLE001
        return None


# ─────────────────────────────────────────────────────────────────────────
# Orchestration A/B
# ─────────────────────────────────────────────────────────────────────────
def post_with_backoff(url, prompt, model, pace, attempts=4):
    delay = 2.0
    r = MissionResult(ok=False, error="non execute")
    for i in range(attempts):
        r = run_mission(url, prompt, model)
        if r.ok and not r.error:
            time.sleep(pace)
            return r
        if i < attempts - 1:                  # backoff sur erreur (429/5xx/transport)
            time.sleep(delay)
            delay *= 2
    return r  # dernier essai (echoue)


def aggregate(values: list[float]) -> tuple[float, float]:
    if not values:
        return (0.0, 0.0)
    return (statistics.mean(values),
            statistics.pstdev(values) if len(values) > 1 else 0.0)


def _mean0(agg_or_scalar) -> float:
    """Renvoie la moyenne, que la valeur soit un tuple (mean,std) ou un scalaire."""
    if isinstance(agg_or_scalar, (list, tuple)):
        return float(agg_or_scalar[0])
    return float(agg_or_scalar)


# ─────────────────────────────────────────────────────────────────────────
# M3 — Rollup par famille : moyenne +/- sigma des moyennes par-prompt.
# ─────────────────────────────────────────────────────────────────────────
def aggregate_by_family(rows: list[dict]) -> dict[str, dict]:
    fams: dict[str, list[dict]] = {}
    for r in rows:
        fams.setdefault(r["family"], []).append(r)

    out: dict[str, dict] = {}
    for fam, rs in fams.items():
        def col(key, only=None):
            vals = [_mean0(r[key]) for r in rs
                    if only is None or only(r)]
            return aggregate(vals)

        has_rubric = lambda r: r.get("rubric_n", 0) > 0       # noqa: E731
        has_judge = lambda r: r.get("judged_n", 0) > 0        # noqa: E731
        out[fam] = {
            "n_prompts": len(rs),
            "savings_ratio": col("savings_ratio"),
            "out_reduction": col("out_reduction"),
            "dcost_usd": col("dcost_usd"),
            "dcost_in": col("dcost_in"),
            "dcost_out": col("dcost_out"),
            "share_output": col("share_output"),
            "net_latency_ms": col("net_latency_ms"),
            "slm_lat_ms": col("slm_lat_ms"),
            "jaccard": col("jaccard"),
            "quality_delta": col("quality_delta", only=has_rubric),
            "quality_n": sum(1 for r in rs if has_rubric(r)),
            "judge_correct": col("judge_correct", only=has_judge),
            "judge_complete": col("judge_complete", only=has_judge),
            "judge_n": sum(1 for r in rs if has_judge(r)),
        }
    return out


def routing_verdict(family: str, stats: dict) -> tuple[str, str]:
    """(tag mesure depuis les chiffres, regle suggeree par design)."""
    dcost = stats["dcost_usd"][0]
    netlat = stats["net_latency_ms"][0]
    if dcost > 5e-4:
        cost_tag = "cout +"
    elif dcost < -5e-4:
        cost_tag = "cout -"
    else:
        cost_tag = "cout ~0"
    if netlat < -300:
        lat_tag = "plus rapide"
    elif netlat > 300:
        lat_tag = "plus lent"
    else:
        lat_tag = "latence neutre"
    measured = f"{cost_tag} · {lat_tag}"
    return measured, FAMILY_DESIGN_RULE.get(family, "?")


# ─────────────────────────────────────────────────────────────────────────
def main() -> int:
    ap = argparse.ArgumentParser(
        description="H1.1 — economie stratifiee du pre-machage (A/B vrai LLM)")
    ap.add_argument("--baseline-url", default="http://127.0.0.1:8700",
                    help="Bridge real-llm EFE_LIVE_SLM=0 (prompt brut)")
    ap.add_argument("--chewed-url", default="http://127.0.0.1:8701",
                    help="Bridge real-llm EFE_LIVE_SLM=1 (prompt compresse)")
    ap.add_argument("--corpus", default=None,
                    help="Chemin du corpus JSON (defaut : 4 prompts embarques H0.1). "
                         "Recommande : scripts/bench/corpus/prechew_corpus_v1.json")
    ap.add_argument("--families", default=None,
                    help="Filtre CSV de familles a mesurer (ex. "
                         "'clean_concise_long,long_technical_multipart' pour un re-run "
                         "cible). Defaut : toutes. Familles : " + ", ".join(VALID_FAMILIES))
    ap.add_argument("--model", default=None,
                    help="Id provider exact a epingler (ex. claude_claude-sonnet-4-6). "
                         "OMETTRE → le bridge utilise son unique provider configure.")
    ap.add_argument("--price-in", type=float, help="Override prix input $/Mtok")
    ap.add_argument("--price-out", type=float, help="Override prix output $/Mtok")
    ap.add_argument("--repeats", type=int, default=3, help="Repetitions par (prompt×condition)")
    ap.add_argument("--pace", type=float, default=1.0, help="Pause (s) entre appels")
    # M4 — juge LLM optionnel.
    ap.add_argument("--judge", action="store_true",
                    help="Active le LLM-juge (justesse+completude de la reponse chewed). PAYE.")
    ap.add_argument("--judge-url", default=None,
                    help="Bridge pour le juge (defaut : --baseline-url, brut/non chewe).")
    ap.add_argument("--judge-reps", type=int, default=1,
                    help="Nb de reps juges par prompt (defaut 1 → borne le cout).")
    ap.add_argument("--out-dir", default="docs")
    ap.add_argument("--confirm", action="store_true", help="Confirme la depense reelle")
    args = ap.parse_args()

    # Corpus (M1)
    try:
        prompts, corpus_src = load_corpus(args.corpus)
    except Exception as e:  # noqa: BLE001
        print(f"❌ Corpus invalide : {e}", file=sys.stderr)
        return 2

    # Filtre familles (re-run cible) — ex. ne rejouer que F2+F4 apres correction corpus.
    if args.families:
        wanted = {f.strip() for f in args.families.split(",") if f.strip()}
        unknown = wanted - set(VALID_FAMILIES)
        if unknown:
            print(f"❌ Famille(s) inconnue(s) : {sorted(unknown)} "
                  f"(attendu : {', '.join(VALID_FAMILIES)}).", file=sys.stderr)
            return 2
        prompts = [p for p in prompts if p.family in wanted]
        if not prompts:
            print(f"❌ Aucun prompt pour les familles {sorted(wanted)}.", file=sys.stderr)
            return 2
        corpus_src += f"  [filtre familles : {', '.join(sorted(wanted))}]"
    judge_url = args.judge_url or args.baseline_url

    # Pricing resolu (override prioritaire ; sinon table ; sinon erreur claire)
    if args.price_in is not None and args.price_out is not None:
        price_in, price_out = args.price_in, args.price_out
    elif args.model and args.model in PRICING_USD_PER_MTOK:
        price_in, price_out = PRICING_USD_PER_MTOK[args.model]
    else:
        print(f"❌ Pricing introuvable pour '{args.model or '(defaut du bridge)'}'. "
              f"Passe --price-in ET --price-out (en $/Mtok), "
              f"ou --model parmi {list(PRICING_USD_PER_MTOK)}.", file=sys.stderr)
        return 2

    # Repartition par famille (visibilite)
    by_fam: dict[str, int] = {}
    for p in prompts:
        by_fam[p.family] = by_fam.get(p.family, 0) + 1

    n_ab = len(prompts) * args.repeats * 2
    n_judge = (len(prompts) * min(args.repeats, args.judge_reps)) if args.judge else 0
    print("═" * 74)
    print(" H1.1 — Economie stratifiee du pre-machage (A/B vrai LLM)")
    print("═" * 74)
    print(f" Corpus      : {corpus_src}")
    for fam in FAMILY_ORDER:
        if by_fam.get(fam):
            print(f"               · {FAMILY_LABEL[fam]:<26} : {by_fam[fam]} prompts")
    print(f" Modele      : {args.model or '(defaut du bridge)'}  "
          f"(in ${price_in}/Mtok · out ${price_out}/Mtok)")
    print(f" Baseline    : {args.baseline_url}  (EFE_LIVE_SLM=0)")
    print(f" Chewed      : {args.chewed_url}  (EFE_LIVE_SLM=1)")
    print(f" Juge        : {'ON → ' + judge_url if args.judge else 'OFF'}"
          f"{f'  ({args.judge_reps} rep/prompt)' if args.judge else ''}")
    print(f" Appels      : {n_ab} A/B" + (f" + {n_judge} juge" if n_judge else "")
          + f" = {n_ab + n_judge} appels LLM reels")
    est = n_ab * cost_usd(800, 300, price_in, price_out)
    if n_judge:
        est += n_judge * cost_usd(900, 60, price_in, price_out)
    print(f" Depense est.: ~${est:.3f} (ordre de grandeur ; depend des reponses)")
    print("═" * 74)

    if not args.confirm:
        print("\nℹ️  Estimation seule (bridges non requis). "
              "Relance avec --confirm pour la depense reelle.")
        return 0

    # Pre-flight : les 2 bridges repondent ? (seulement quand on va depenser)
    for label, url in [("baseline", args.baseline_url), ("chewed", args.chewed_url)]:
        try:
            with urllib.request.urlopen(f"{url}/healthz", timeout=5) as h:
                if h.read().decode().strip() != "ok":
                    print(f"⚠ {label} {url}/healthz inattendu", file=sys.stderr)
        except Exception as e:  # noqa: BLE001
            print(f"❌ {label} bridge injoignable ({url}) : {e}", file=sys.stderr)
            return 2

    # ── Smoke 1 appel : detecte le provider reel (ou un repli mock) ──
    smoke = run_mission(args.chewed_url, "Test.", args.model)
    if not smoke.ok or not smoke.provider:
        print(f"❌ Smoke KO sur {args.chewed_url} : {smoke.error or 'pas de provider'}.",
              file=sys.stderr)
        return 2
    if smoke.provider.startswith("mock-"):
        print(f"⚠️  Provider MOCK detecte ('{smoke.provider}'). En PASSE 1 keyless "
              f"c'est ATTENDU (validation gratuite). En passe reelle, exporte "
              f"ANTHROPIC_API_KEY/OPENAI_API_KEY AVANT de lancer les bridges "
              f"(--features real-llm, sans EFE_PREVIEW_MOCK).", file=sys.stderr)
    else:
        print(f"   provider reel detecte : {smoke.provider}")

    # ── Run A/B ──
    rows = []
    for p in prompts:
        print(f"\n▶ {p.id} [{FAMILY_LABEL.get(p.family, p.family)}] ({len(p.text)} car)")
        reps = []
        for k in range(args.repeats):
            b = post_with_backoff(args.baseline_url, p.text, args.model, args.pace)
            c = post_with_backoff(args.chewed_url, p.text, args.model, args.pace)
            if not (b.ok and c.ok):
                print(f"   rep {k}: ECHEC (baseline={b.error or 'ok'} chewed={c.error or 'ok'})")
                continue

            # M2 — decomposition du cout entree vs SORTIE
            cost_b = cost_usd(b.input_tokens_raw, b.output_tokens, price_in, price_out)
            cost_c = cost_usd(c.input_tokens_compressed, c.output_tokens, price_in, price_out)
            dcost_total = cost_b - cost_c
            dcost_in = (b.input_tokens_raw - c.input_tokens_compressed) / 1e6 * price_in
            dcost_out = (b.output_tokens - c.output_tokens) / 1e6 * price_out
            share_output = (dcost_out / dcost_total) if dcost_total != 0 else 0.0
            out_reduction = 1.0 - c.output_tokens / max(1, b.output_tokens)
            net_latency_ms = (c.slm_latency_ms + c.llm_latency_ms) - b.llm_latency_ms

            # M4 — qualite : rubric (gratuit) + juge optionnel (paye)
            rb = rubric_coverage(b.answer, p.expect)
            rc = rubric_coverage(c.answer, p.expect)
            jc = jp = None
            if args.judge and k < args.judge_reps:
                jv = run_judge(judge_url, p.core, b.answer, c.answer, args.model)
                if jv:
                    jc, jp = jv["correct"], jv["complete"]
                time.sleep(args.pace)

            reps.append({
                "in_raw": b.input_tokens_raw, "in_comp": c.input_tokens_compressed,
                "out_b": b.output_tokens, "out_c": c.output_tokens,
                "cost_b": cost_b, "cost_c": cost_c,
                "dcost": dcost_total, "dcost_in": dcost_in, "dcost_out": dcost_out,
                "share_output": share_output, "out_reduction": out_reduction,
                "llm_lat_b": b.llm_latency_ms, "llm_lat_c": c.llm_latency_ms,
                "slm_lat_c": c.slm_latency_ms, "net_latency_ms": net_latency_ms,
                "jaccard": jaccard(b.answer, c.answer),
                "rubric_b": rb, "rubric_c": rc,
                "judge_correct": jc, "judge_complete": jp,
                "answer_b": b.answer, "answer_c": c.answer,
                "failed_c": c.failed_components,
            })
            qd = (f"  qΔrubric={rc - rb:+.2f}" if (rb is not None and rc is not None) else "")
            jd = (f"  juge c/c={jc:.0f}/{jp:.0f}" if jc is not None else "")
            share_disp = _share_cell(dcost_total, share_output)
            print(f"   rep {k}: Δcost=${dcost_total:+.5f} (sortie {share_disp})  "
                  f"net_lat={net_latency_ms:+d}ms  out_red={out_reduction:+.0%}{qd}{jd}")

        if not reps:
            continue

        rub_pairs = [(r["rubric_b"], r["rubric_c"]) for r in reps
                     if r["rubric_b"] is not None and r["rubric_c"] is not None]
        judged = [r for r in reps if r["judge_correct"] is not None]
        agg = {
            "id": p.id, "family": p.family, "chars": len(p.text), "n": len(reps),
            "notes": p.notes,
            "in_raw": round(statistics.mean(r["in_raw"] for r in reps)),
            "in_comp": round(statistics.mean(r["in_comp"] for r in reps)),
            "out_b": round(statistics.mean(r["out_b"] for r in reps)),
            "out_c": round(statistics.mean(r["out_c"] for r in reps)),
            "savings_ratio": 1 - statistics.mean(r["in_comp"] for r in reps) /
                                 max(1, statistics.mean(r["in_raw"] for r in reps)),
            "out_reduction": aggregate([r["out_reduction"] for r in reps]),
            "dcost_usd": aggregate([r["dcost"] for r in reps]),
            "dcost_in": aggregate([r["dcost_in"] for r in reps]),
            "dcost_out": aggregate([r["dcost_out"] for r in reps]),
            "share_output": aggregate([r["share_output"] for r in reps]),
            "net_latency_ms": aggregate([float(r["net_latency_ms"]) for r in reps]),
            "slm_lat_ms": aggregate([float(r["slm_lat_c"]) for r in reps]),
            "jaccard": aggregate([r["jaccard"] for r in reps]),
            "rubric_b": aggregate([rb for rb, _ in rub_pairs]),
            "rubric_c": aggregate([rc for _, rc in rub_pairs]),
            "quality_delta": aggregate([rc - rb for rb, rc in rub_pairs]),
            "rubric_n": len(rub_pairs),
            "judge_correct": aggregate([r["judge_correct"] for r in judged]),
            "judge_complete": aggregate([r["judge_complete"] for r in judged]),
            "judged_n": len(judged),
            "reps": reps,
        }
        rows.append(agg)

    if not rows:
        print("\n❌ Aucune mesure exploitable.", file=sys.stderr)
        return 1

    # ── Ecriture JSON brut (re-analyse sans re-depenser) + rapport markdown ──
    stamp = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    base = f"{args.out_dir}/EFE_PRECHEWING_ECONOMICS_H1a_{stamp}"
    fam_rollup = aggregate_by_family(rows)
    payload = {
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "roadmap": "H1.1",
        "corpus_source": corpus_src,
        "model": args.model or smoke.provider,
        "price_in_usd_mtok": price_in, "price_out_usd_mtok": price_out,
        "baseline_url": args.baseline_url, "chewed_url": args.chewed_url,
        "judge": bool(args.judge), "judge_url": judge_url if args.judge else None,
        "repeats": args.repeats,
        "family_rollup": fam_rollup,
        "rows": rows,
    }
    with open(f"{base}.json", "w", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False, indent=2)
    write_report(f"{base}.md", payload)
    print(f"\n✅ Rapport : {base}.md   ·   donnees brutes : {base}.json")
    return 0


# ─────────────────────────────────────────────────────────────────────────
# M5 — Rapport stratifie
# ─────────────────────────────────────────────────────────────────────────
def write_report(path: str, p: dict) -> None:
    rows = p["rows"]
    fam_rollup = p["family_rollup"]
    judged = p.get("judge", False)

    def m5(agg):   # cout (mean ± std), 5 decimales
        return f"{agg[0]:+.5f} ± {agg[1]:.5f}"

    def mi(agg):   # entier signe (ms)
        return f"{agg[0]:+.0f} ± {agg[1]:.0f}"

    def pct(agg):
        return f"{agg[0]:+.0%}"

    lines = [
        f"# EFE — Economie stratifiee du pre-machage (vrai LLM) · {p['model']}",
        "",
        f"> Roadmap **H1.1**. A/B controle (baseline brut vs chewed compresse), "
        f"corpus **stratifie par famille**, cout **calcule** depuis tokens × pricing "
        f"(in ${p['price_in_usd_mtok']}/Mtok · out ${p['price_out_usd_mtok']}/Mtok). "
        f"Corpus : {p['corpus_source']}. Juge LLM : {'ON' if judged else 'OFF'}. "
        f"Genere : {p['generated_utc']}. Donnees brutes : le `.json` a cote.",
        "",
        "## 1. Verdict par famille (le rollup)",
        "",
        "| Famille | n | savings IN | reduction SORTIE | part sortie | Δcout (USD) | "
        "latence NETTE (ms) | qualite (Δrubric) |"
        + (" juge c/compl |" if judged else "")
        + " mesure | regle |",
        "|---|---|---|---|---|---|---|---|"
        + ("---|" if judged else "") + "---|---|",
    ]
    for fam in FAMILY_ORDER:
        s = fam_rollup.get(fam)
        if not s:
            continue
        measured, rule = routing_verdict(fam, s)
        ql = ("—" if s["quality_n"] == 0
              else f"{s['quality_delta'][0]:+.2f} (n={s['quality_n']})")
        jcol = ""
        if judged:
            jcol = (f" {s['judge_correct'][0]:.1f}/{s['judge_complete'][0]:.1f} "
                    f"(n={s['judge_n']}) |" if s["judge_n"] else " — |")
        lines.append(
            f"| {FAMILY_LABEL[fam]} | {s['n_prompts']} | {pct(s['savings_ratio'])} | "
            f"{pct(s['out_reduction'])} | {_share_cell(s['dcost_usd'][0], s['share_output'][0])} | "
            f"{m5(s['dcost_usd'])} | {mi(s['net_latency_ms'])} | {ql} |"
            + jcol
            + f" {measured} | **{rule}** |"
        )

    # ── Section 2 : decomposition du cout (ou vit le gain) ──
    lines += [
        "",
        "## 2. Decomposition du cout — ou vit le gain (entree vs SORTIE)",
        "",
        "> Le verdict H0.1 affirmait que ~88 % du gain vient de la **sortie** "
        "(induction de brievete), pas de l'entree compressee. Cette section le "
        "**chiffre** par famille. `part sortie` = Δcout_out / Δcout_total.",
        "",
        "| Famille | Δcout entree (USD) | Δcout SORTIE (USD) | part sortie | reduction sortie |",
        "|---|---|---|---|---|",
    ]
    for fam in FAMILY_ORDER:
        s = fam_rollup.get(fam)
        if not s:
            continue
        lines.append(
            f"| {FAMILY_LABEL[fam]} | {m5(s['dcost_in'])} | {m5(s['dcost_out'])} | "
            f"{_share_cell(s['dcost_usd'][0], s['share_output'][0])} | {pct(s['out_reduction'])} |"
        )

    # ── Section 3 : detail par prompt ──
    lines += [
        "",
        "## 3. Detail par prompt",
        "",
        "| Prompt | famille | savings IN | red. sortie | Δcout (USD) | "
        "latence NETTE (ms) | overhead SLM (ms) | rubric b/c | style (jaccard) |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    for r in rows:
        rub = ("—" if r["rubric_n"] == 0
               else f"{r['rubric_b'][0]:.2f}/{r['rubric_c'][0]:.2f}")
        lines.append(
            f"| {r['id']} | {r['family']} | {r['savings_ratio']:+.0%} "
            f"({r['in_raw']}→{r['in_comp']} t) | {pct(r['out_reduction'])} "
            f"({r['out_b']}→{r['out_c']} t) | {m5(r['dcost_usd'])} | "
            f"{mi(r['net_latency_ms'])} | {mi(r['slm_lat_ms'])} | {rub} | "
            f"{r['jaccard'][0]:.2f} |"
        )

    # ── Section 4 : qualite ──
    lines += [
        "",
        "## 4. Qualite (honnete)",
        "",
        "- **rubric b/c** : fraction des concepts attendus (`expect`) presents dans "
        "la reponse baseline (b) puis chewed (c). **Critere de garde : rubric_c ≥ "
        "rubric_b** (la brievete ne doit pas perdre de concept). `—` = prompt sans "
        "`expect` (le juge prend le relais).",
    ]
    if judged:
        lines += [
            "- **juge c/compl** : LLM-juge (1-5) sur la reponse CHEWED — justesse "
            "(correct) et completude (complete). C'est le **seul** signal capable de "
            "valider « correct mais concis » (ce que le jaccard ne peut pas voir).",
        ]
    else:
        lines += [
            "- **juge LLM** : DESACTIVE (`--judge` off). Pour le verdict rigoureux "
            "« correct mais concis », relancer avec `--judge` (ajoute ~1 appel/prompt).",
        ]
    lines += [
        "- **style (jaccard)** : recouvrement de mots baseline↔chewed. **RETROGRADE** "
        "→ mesure une **divergence de STYLE**, PAS la qualite. Un jaccard bas (~0,25) "
        "= reponse plus concise, pas une erreur (cf. verdict H0.1).",
    ]

    # ── Section 5 : regle de routage (le livrable) ──
    lines += [
        "",
        "## 5. Regle de routage (LIVRABLE — entree d'une future politique de routeur)",
        "",
        "| Famille | chewer ? | justification (mesuree) |",
        "|---|---|---|",
    ]
    for fam in FAMILY_ORDER:
        s = fam_rollup.get(fam)
        if not s:
            continue
        measured, rule = routing_verdict(fam, s)
        lines.append(f"| {FAMILY_LABEL[fam]} | **{rule}** | {measured} "
                     f"(Δcout {s['dcost_usd'][0]:+.5f} $, "
                     f"net_lat {s['net_latency_ms'][0]:+.0f} ms) |")
    lines += [
        "",
        "**Logique de routage** : sous le gate (F3) → BYPASS (compression NO-OP, "
        "overhead analyzer pur) ; redondant/verbeux (F1) → CHEWER (gain coût+latence "
        "via reduction de sortie) ; propre/concis (F2) → NE PAS chewer (overhead SLM "
        "non couvert) ; long technique (F4) → CONDITIONNEL (chewer si Q&A, pas si "
        "exhaustivite requise — arbitrer avec la colonne juge).",
    ]

    # ── Comment lire / caveats / DoD ──
    lines += [
        "",
        "## Comment lire",
        "",
        "- **Δcout > 0** → le pre-machage **economise** (cout SLM local ≈ nul). "
        "**part sortie** ≈ fraction du gain venant de la reduction de SORTIE.",
        "- **latence NETTE = (SLM + LLM_chewed) − LLM_baseline**. **< 0** → chewed "
        "PLUS RAPIDE bout-en-bout.",
        "- **rollup par famille** = moyenne ± σ des moyennes par-prompt de la famille.",
        "",
        "## Caveats methodologiques",
        "",
        "- **Tokens d'entree estimes** (`chars/4`, via `PRE_CHEWING_METRICS`), pas le "
        "compte exact du tokenizer provider ; `cost_cents` du flux sert de garde-fou.",
        "- **Temperature non reglable via le DTO** → variance geree par les "
        "repetitions (moyenne ± σ), pas par `temperature=0`.",
        "- **Famille redundant_verbose = best-case** (padding) → savings optimistes ; "
        "les familles propre/court/long bornent le realisme.",
        "- **Cout compute SLM non monetise** (self-host local ≈ nul). Si SLM facture, "
        "soustraire son cout du Δcout.",
        "- **Juge LLM** (si ON) : un evaluateur est lui-meme bruite ; il sert d'**aide "
        "a la decision** sur F4, pas de verite absolue.",
        "",
        "## Decision (a completer apres lecture)",
        "",
        "> Phrase-titre attendue : « Le pre-machage se route par FAMILLE : CHEWER les "
        "prompts redondants (−X% cout, −Y ms), BYPASSER les courts, NE PAS chewer les "
        "propres, et CONDITIONNER les longs techniques a [Q&A / exhaustivite]. »",
        "",
    ]
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))


if __name__ == "__main__":
    raise SystemExit(main())
