"""EFE Mode 1 — Hybrid Decoder (Plan B) example.

Demonstrates the recommended way to activate the hybrid decoder
introduced as Plan B in Sprint D7.6 and formally adopted in D8.0.

Use the hybrid decoder when :

* You need higher accuracy on inherently ambiguous inputs (the
  swarm sometimes converges to the wrong answer with high
  confidence ; raw cosine acts as a defensive cross-check).
* You want **calibrated confidence** — the hybrid result exposes
  both the raw cosine score and the swarm consensus, so the
  caller can apply application-specific decision policies.
* You're running on a catalog where the H3c residual still bites
  (cf. `docs/audits/d7_calibration_report.md` §6.4).

When NOT to use the hybrid :

* When the swarm-only path already saturates accuracy (≥ 0.95) :
  the extra raw-cosine call adds no signal but adds latency.
* When you cannot afford the doubled embedder cost (one for the
  catalog anchors, one for the request) — though the embedder is
  fast enough (< 50 ms typical).

Prerequisites :

    pip install -e clients/python    # exposes the efe_tool package
    cargo build --release -p efe-server
    ./target/release/efe-server &    # starts on 127.0.0.1:50051
"""
from __future__ import annotations

import os

from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder
from efe_tool.integrations.hybrid_decoder import HybridClassifier, HybridResult


def main() -> None:
    # ------------------------------------------------------------------
    # 1. Set up the embedder + catalog + client.
    # ------------------------------------------------------------------
    embedder = LocalSTEmbedder()  # paraphrase-multilingual-MiniLM-L12-v2 by default
    catalog = AnchorCatalog.load(
        "catalogs/support_triage.yaml",
        embedder=embedder,
    )
    os.environ.setdefault("EFE_ENDPOINT", "127.0.0.1:50051")
    client = EfeClient.from_env()

    # ------------------------------------------------------------------
    # 2. Build the hybrid classifier. Thresholds are tuned for the
    #    `support_triage` catalog ; tighten them for higher-stakes
    #    classifications (fraud, medical) so the hybrid prefers
    #    "uncertain" over a wrong label.
    # ------------------------------------------------------------------
    hybrid = HybridClassifier(
        client=client,
        catalog=catalog,
        # Defaults shown for clarity ; override only if you have
        # measured the impact (cf. `eval/scripts/compare_decoders.py`).
        raw_high_margin=0.55,
        swarm_high_confidence=0.85,
        swarm_low_confidence=0.70,
        raw_top_gap=0.04,
    )

    # ------------------------------------------------------------------
    # 3. Classify a few inputs. The HybridResult gives full visibility
    #    into the decision : raw vs swarm labels, the chosen branch,
    #    and the confidence used to decide.
    # ------------------------------------------------------------------
    examples = [
        # Easy fraud — both agree.
        "Someone charged my card without my consent.",
        # Mixed — billing-adjacent question (H3c-α territory).
        "Where do I find a record of my past invoices?",
        # Bug report — clear domain.
        "L'application plante quand j'ouvre le menu paramètres.",
        # Ambiguous — phishing or fraud question.
        "I got an email asking for my password — is this legitimate?",
    ]

    print(f"{'Text':<55} → {'Label':<12} {'Conf':<6} {'Branch':<24} "
          f"{'Raw label':<12} {'Swarm label':<12}")
    print("-" * 130)
    for text in examples:
        try:
            result: HybridResult = hybrid.classify(text)
        except Exception as e:
            print(f"{text[:53]:<55} → ERROR : {e}")
            continue
        print(
            f"{text[:53]:<55} → {result.label:<12} "
            f"{result.confidence:<6.2f} {result.path:<24} "
            f"{result.raw_label:<12} {result.swarm_label:<12}"
        )

    # ------------------------------------------------------------------
    # 4. Application-specific decision policy on the result.
    #    The HybridResult.path lets you act on the agreement signal :
    # ------------------------------------------------------------------
    print()
    print("Application policy on a tricky case :")
    tricky = "Where do I find a record of my past invoices?"
    result = hybrid.classify(tricky)

    if result.path == "agree":
        action = f"AUTO-ROUTE to `{result.label}` (high agreement)"
    elif result.path == "raw_overrides_swarm":
        action = f"ROUTE to `{result.label}` (raw decisive, log for review)"
    elif result.path == "swarm_overrides_raw":
        action = f"ROUTE to `{result.label}` (swarm high-confidence)"
    elif result.path == "reject":
        action = "ASK CLARIFYING QUESTION (signals disagree)"
    else:
        action = "FALLBACK (unexpected branch)"
    print(f"  Input  : {tricky}")
    print(f"  Path   : {result.path}")
    print(f"  Action : {action}")

    client.close()


if __name__ == "__main__":
    main()
