#!/usr/bin/env python3
"""
Validate-grammars : envoie 1000 prompts variés à chaque robot et vérifie
que 100 % des réponses sont du JSON valide qui passe son JSON Schema.

Requiert :
- llama-server tournant aux ports 8090-8093 (cf. EFE_PROJET/03_a_construire/scripts/start-nano-robots.sh)
- jsonschema Python package (`pip install jsonschema`)

Usage :
    pip install jsonschema requests
    cd /path/to/efe-nano-app-coordinator
    python3 scripts/validate-grammars.py            # 1000 tirages
    python3 scripts/validate-grammars.py --n 100    # 100 tirages
    python3 scripts/validate-grammars.py --robot analyzer

Sortie : CSV dans `target/grammar_validation_report.csv`.
"""
from __future__ import annotations

import argparse
import csv
import json
import random
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

try:
    import jsonschema
    import requests
except ImportError as e:
    print(f"❌ Missing dependency: {e}", file=sys.stderr)
    print("   Run: pip install jsonschema requests", file=sys.stderr)
    sys.exit(1)

CRATE_DIR = Path(__file__).resolve().parent.parent
SCHEMAS_DIR = CRATE_DIR / "assets" / "schemas"
GRAMMARS_DIR = CRATE_DIR / "assets" / "grammars"
PROMPTS_DIR = CRATE_DIR / "assets" / "prompts"

ROBOTS = {
    "analyzer":      {"port": 8090, "schema": "analyzer_v1.json",
                      "grammar": "analyzer_v1.gbnf", "prompt": "analyzer.txt",
                      "max_tokens": 512},
    "reformulator":  {"port": 8091, "schema": "reformulator_v1.json",
                      "grammar": "reformulator_v1.gbnf", "prompt": "reformulator.txt",
                      "max_tokens": 1024},
    "compressor":    {"port": 8092, "schema": "compressor_v1.json",
                      "grammar": "compressor_v1.gbnf", "prompt": "compressor.txt",
                      "max_tokens": 1536},
    "verifier":      {"port": 8093, "schema": "verifier_v1.json",
                      "grammar": "verifier_v1.gbnf", "prompt": "verifier.txt",
                      "max_tokens": 768},
}

# Corpus de test : ~30 variations courtes pour chaque robot, on les cycle.
ANALYZER_CORPUS = [
    "Apple reported $90B revenue in Q3 2026.",
    "Le Crédit Agricole a annoncé une hausse de 12% de ses bénéfices.",
    "The Federal Reserve maintained rates at 5.25-5.50% on March 20.",
    "L'inflation européenne est tombée à 2.3% en avril.",
    "Microsoft acquired a small AI startup for $2B last week.",
    "Bordeaux a connu un été particulièrement chaud cette année.",
    "Tesla shares dropped 8% after the earnings call.",
    "Le président français a rencontré son homologue allemand à Paris.",
    "Climate change is accelerating faster than predicted.",
    "Pfizer announced a new vaccine trial starting in June 2026.",
    "Did Lionel Messi score in the last match against Brazil?",
    "Should I invest in tech stocks now or wait for a correction?",
    "How does inflation affect mortgage rates over a 5-year horizon?",
    "Explain the difference between supervised and unsupervised learning.",
    "Compare the GDP growth of France vs Germany over the last decade.",
    "Trois ans après le COVID, la santé mentale des jeunes reste fragile.",
    "今日の東京の天気は晴れです。",
    "I am very disappointed with the customer service.",
    "Excellent meal at the new restaurant, highly recommend it!",
    "The traffic on Highway 101 was completely blocked this morning.",
    "Quel est l'impact de l'IA sur l'emploi ?",
    "Compare Bitcoin and Ethereum on long-term sustainability.",
    "Yesterday's match was so boring, nothing happened.",
    "The discovery of penicillin revolutionized modern medicine.",
    "Net Asset Value of the fund increased by 3.2% in Q4.",
    "How can I improve my Python code for performance?",
    "AstraZeneca trial enrolled 500 patients in 12 countries.",
    "Tokyo Olympics 2020 were postponed due to the pandemic.",
    "Read the FOMC minutes for clues about the next rate decision.",
    "Steve Jobs co-founded Apple Computer in 1976 with Steve Wozniak.",
]

REFORMULATOR_CORPUS = [
    json.dumps({
        "raw_text": '<ref id="src-a1">Federal Reserve maintained rates at 5.25%</ref>',
        "source_dialect": "CLAUDE_XML",
        "target_schema_id": "citation_v1",
    }),
    json.dumps({
        "raw_text": '{"source": "src-b2", "quote": "GDP grew 3.3%"}',
        "source_dialect": "GPT4_JSON",
        "target_schema_id": "citation_v1",
    }),
    json.dumps({
        "raw_text": "Fed maintains rate at 5.25-5.50%[^src-c3]",
        "source_dialect": "GEMINI_MARKDOWN",
        "target_schema_id": "citation_v1",
    }),
    json.dumps({
        "raw_text": "Subject: Apple, Predicate: reported revenue, Object: $90B in Q3",
        "source_dialect": "GPT4_JSON",
        "target_schema_id": "fact_v1",
    }),
] * 8  # 32 variants

COMPRESSOR_CORPUS = [
    json.dumps({
        "text": (
            "La Réserve fédérale américaine a annoncé mercredi qu'elle "
            "maintenait son taux directeur dans la fourchette de 5,25 à "
            "5,50 %, comme attendu par les marchés. Cette décision marque "
            "la cinquième pause consécutive, le comité fédéral citant la "
            "nécessité d'observer davantage de données économiques avant "
            "tout changement de politique monétaire. Les marchés ont peu "
            "réagi à l'annonce. Le président de la Fed, Jerome Powell, a "
            "indiqué lors de la conférence de presse que des baisses de "
            "taux restent possibles plus tard en 2026 si l'inflation "
            "continue de ralentir vers la cible de 2 %."
        ),
        "target_words": 50,
        "must_preserve": ["Réserve fédérale", "5,25", "5,50", "Jerome Powell"],
        "style": "EXTRACTIVE",
    }),
] * 30

VERIFIER_CORPUS = [
    json.dumps({
        "claim": "The Fed maintained rates at 5.25-5.50%",
        "candidate_sources": [
            {"id": "src-fed-2026-03-20",
             "text": "The FOMC voted unanimously to maintain the federal "
                     "funds rate target range at 5.25 to 5.50 percent."}
        ],
        "strictness": "STANDARD",
    }),
    json.dumps({
        "claim": "Apple's Q3 revenue was $200 billion",
        "candidate_sources": [
            {"id": "src-apple-q3",
             "text": "Apple reported Q3 revenue of $90 billion."}
        ],
        "strictness": "STANDARD",
    }),
] * 15


def load_asset(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def call_robot(robot: str, prompt: str, port: int, grammar: str,
               system: str, max_tokens: int, timeout: int = 30) -> dict:
    """Call llama-server /completion with grammar enforcement."""
    body = {
        "prompt": f"{system}\n\n{prompt}",
        "n_predict": max_tokens,
        "temperature": 0.1,
        "cache_prompt": True,
        "grammar": grammar,
        "stop": ["\n\n\n"],
    }
    r = requests.post(
        f"http://127.0.0.1:{port}/completion",
        json=body,
        timeout=timeout,
    )
    r.raise_for_status()
    return r.json()


def validate_one(robot_name: str, robot_cfg: dict, prompt: str,
                  schema_obj: dict, system: str, grammar: str) -> tuple[bool, str | None, float]:
    """
    Returns (success, error_msg, latency_ms).
    """
    started = time.monotonic()
    try:
        resp = call_robot(robot_name, prompt, robot_cfg["port"],
                          grammar, system, robot_cfg["max_tokens"])
    except Exception as e:
        return False, f"transport: {e}", (time.monotonic() - started) * 1000
    elapsed_ms = (time.monotonic() - started) * 1000

    content = resp.get("content", "").strip()
    if not content:
        return False, "empty content", elapsed_ms

    # Parse JSON.
    try:
        parsed = json.loads(content)
    except json.JSONDecodeError as e:
        return False, f"json parse: {e}", elapsed_ms

    # Validate against schema.
    try:
        jsonschema.validate(parsed, schema_obj)
    except jsonschema.ValidationError as e:
        return False, f"schema: {e.message[:200]}", elapsed_ms

    return True, None, elapsed_ms


def run_robot(robot_name: str, robot_cfg: dict, corpus: list[str],
              n: int, concurrency: int) -> dict:
    """Validate one robot with N prompts."""
    schema_path = SCHEMAS_DIR / robot_cfg["schema"]
    grammar_path = GRAMMARS_DIR / robot_cfg["grammar"]
    prompt_path = PROMPTS_DIR / robot_cfg["prompt"]

    if not (schema_path.exists() and grammar_path.exists() and prompt_path.exists()):
        return {
            "robot": robot_name,
            "error": f"missing asset(s) in {SCHEMAS_DIR}",
        }

    schema_obj = json.loads(schema_path.read_text())
    grammar = grammar_path.read_text()
    system = prompt_path.read_text()

    # Health check first.
    try:
        h = requests.get(f"http://127.0.0.1:{robot_cfg['port']}/health",
                          timeout=3)
        if h.status_code != 200:
            return {
                "robot": robot_name,
                "error": f"health {h.status_code} — robot down?",
            }
    except Exception as e:
        return {
            "robot": robot_name,
            "error": f"unreachable: {e}",
        }

    # Pick N prompts (cycle through corpus if N > len).
    rng = random.Random(42)
    prompts = [rng.choice(corpus) for _ in range(n)]

    results = []
    with ThreadPoolExecutor(max_workers=concurrency) as ex:
        futures = {
            ex.submit(validate_one, robot_name, robot_cfg, p, schema_obj,
                      system, grammar): i
            for i, p in enumerate(prompts)
        }
        for fut in as_completed(futures):
            results.append(fut.result())

    successes = sum(1 for s, _, _ in results if s)
    failures = [(e, lat) for s, e, lat in results if not s]
    latencies = sorted([lat for _, _, lat in results])

    return {
        "robot": robot_name,
        "n": n,
        "successes": successes,
        "failure_count": len(failures),
        "success_rate": successes / n * 100,
        "p50_ms": latencies[n // 2] if latencies else 0,
        "p95_ms": latencies[int(n * 0.95)] if latencies else 0,
        "p99_ms": latencies[int(n * 0.99)] if latencies else 0,
        "first_failures": failures[:5],
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=1000,
                    help="Number of tirages per robot")
    ap.add_argument("--robot",
                    choices=list(ROBOTS.keys()) + ["all"], default="all")
    ap.add_argument("--concurrency", type=int, default=4)
    args = ap.parse_args()

    robots_to_run = [args.robot] if args.robot != "all" else list(ROBOTS.keys())

    corpora = {
        "analyzer": ANALYZER_CORPUS,
        "reformulator": REFORMULATOR_CORPUS,
        "compressor": COMPRESSOR_CORPUS,
        "verifier": VERIFIER_CORPUS,
    }

    all_results = []
    for rb in robots_to_run:
        print(f"\n=== {rb} ({args.n} tirages, concurrency={args.concurrency}) ===")
        res = run_robot(rb, ROBOTS[rb], corpora[rb], args.n, args.concurrency)
        all_results.append(res)
        if "error" in res:
            print(f"  ❌ {res['error']}")
            continue
        print(f"  ✓ success rate : {res['success_rate']:.1f}%")
        print(f"  ✓ failures     : {res['failure_count']}")
        print(f"  ✓ p50 / p95 / p99 : {res['p50_ms']:.1f} / {res['p95_ms']:.1f} / {res['p99_ms']:.1f} ms")
        if res["first_failures"]:
            print(f"  ⚠ first failures (max 5):")
            for err, lat in res["first_failures"]:
                print(f"     - {err[:80]} ({lat:.0f}ms)")

    # Write CSV.
    output_csv = CRATE_DIR.parent.parent / "target" / "grammar_validation_report.csv"
    output_csv.parent.mkdir(exist_ok=True)
    with open(output_csv, "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["robot", "n", "successes", "failure_count",
                         "success_rate", "p50_ms", "p95_ms", "p99_ms"])
        for r in all_results:
            if "error" in r:
                writer.writerow([r["robot"], 0, 0, 0, 0, 0, 0, 0])
            else:
                writer.writerow([
                    r["robot"], r["n"], r["successes"], r["failure_count"],
                    f"{r['success_rate']:.1f}", f"{r['p50_ms']:.1f}",
                    f"{r['p95_ms']:.1f}", f"{r['p99_ms']:.1f}",
                ])
    print(f"\nCSV report: {output_csv}")

    # Gate : si un robot < 99 % success rate, retourner code != 0
    fail = any(("error" in r) or r.get("success_rate", 100) < 99.0
               for r in all_results)
    return 1 if fail else 0


if __name__ == "__main__":
    sys.exit(main())
