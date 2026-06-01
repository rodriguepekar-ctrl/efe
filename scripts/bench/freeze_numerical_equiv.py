#!/usr/bin/env python3
"""Freeze 100 numerical equivalence fixtures for the FFI Φ-C-FFI suite.

Sprint **Φ-C-FFI Completion (D28)** — artefact #22.
Cf. `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md` § 7.

## Mission

Produit `crates/efe-ffi/python/efe/tests/fixtures/numerical_equiv.json`
contenant 100 entrées (text, catalog_id, seed, expected_verdict). Le
test `test_numerical_equiv.py` les consume pour gating SLO-FFI-EQUIVALENCE.

## Modes

* **--mode replicate** (default) : génère les expected verdicts en
  ré-implémentant la formule SHA-256 du stub D25
  (`controller::synthesise_verdict` + `bundle::InnerBundle::encode`).
  Utile pré-D29 (= avant que le real swarm soit câblé) pour avoir
  une fixture stable + déterministe.
* **--mode runtime** : charge `efe._native` et capture les verdicts
  réels via `efe.classify_zero_copy()`. Utile post-D29 pour figer
  les vrais valeurs.

## Usage

```bash
# Pré-D29 — replicate Python-side
python scripts/bench/freeze_numerical_equiv.py \\
    --mode replicate \\
    --out crates/efe-ffi/python/efe/tests/fixtures/numerical_equiv.json \\
    --count 100 \\
    --catalog-id router-v1

# Post-D29 — capture runtime
python scripts/bench/freeze_numerical_equiv.py \\
    --mode runtime \\
    --out crates/efe-ffi/python/efe/tests/fixtures/numerical_equiv.json \\
    --count 100 \\
    --catalog-path crates/efe-ffi/python/efe/tests/fixtures/catalog.json
```

## Réplication de la formule stub D25

Source : `crates/efe-ffi/src/bundle.rs::InnerBundle::encode` +
`crates/efe-ffi/src/controller.rs::synthesise_verdict`.

```text
# bundle.encode(text)
seed = SHA256(text || bundle_id)         # 32 bytes
emb[i] = (seed[i % 32] / 255.0) - 0.5    # for i in [0, dim)
emb = emb / ||emb||_2                    # L2-normalize

# controller.synthesise_verdict(emb, catalog_id, version, labels, seed_u64)
h = SHA256(le_bytes(emb) || catalog_id || le_bytes(seed_u64))
label_idx = u32_le(h[0..4]) % len(labels)
label = labels[label_idx]
confidence = u32_le(h[4..8]) / u32::MAX
rounds = 4 + (h[8] % 16)
swarm_id = "swarm-" + hex(h[0..8])
```

Le script reproduit fidèlement cette formule en Python. Le pin test
`scripts/tests/test_freeze_numerical_equiv.py` vérifie que la
réplication match la sortie Rust.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import struct
import sys
from pathlib import Path
from typing import Any

# **Audit fix Φ-C-FFI.B2** — numpy est requis pour la réplication
# f32 fidèle de la formule Rust. Sans numpy, l'arithmétique Python
# se fait en f64 → norm divergent de Rust f32 → 396/1536 bytes
# diffèrent dans l'embedding → SHA-256 totalement différent →
# label/swarm_id/confidence tous mismatches → 0/100 fixtures PASS.
try:
    import numpy as np
except ImportError:  # pragma: no cover
    sys.stderr.write(
        "FATAL : numpy is required for f32-fidelity Python replication "
        "(audit fix Φ-C-FFI.B2). Install with `pip install numpy`.\n"
    )
    sys.exit(2)

# ────────────────────────────────────────────────────────────────────
# Constants — must match `crates/efe-ffi/src/bundle.rs` D25 stub.
# ────────────────────────────────────────────────────────────────────

DIM = 384
BUNDLE_ID = "test-bundle"  # matches synthetic_for_test
CATALOG_VERSION = "1.0.0"

# Sample catalog matching `controller::synthetic_catalog_from_path`
# fallback. The test fixtures use this catalog so the labels are
# deterministic across runs.
SAMPLE_LABELS = [
    "billing-dispute",
    "tech-support",
    "refund-request",
    "general-inquiry",
    "other",
]

# 100 input texts — mix multilingual + edge cases (long, short,
# unicode). Stable order = stable fixture order.
SAMPLE_TEXTS = [
    "I want to dispute the charge on my account",
    "Quand est-ce que ma commande arrive ?",
    "¿Cómo puedo cambiar mi contraseña?",
    "Wie kann ich mein Konto kündigen?",
    "プライバシーポリシーを教えてください",
    "我想要退款",
    "안녕하세요, 환불 부탁드립니다",
    "Há algum desconto disponível?",
    "Where is my package?",
    "Cancel my subscription please",
    "I need a refund for order #12345",
    "Comment fonctionne votre garantie ?",
    "Could you reset my account?",
    "Help me understand the bill",
    "The product arrived damaged",
    "I have a billing question",
    "When will my order be delivered?",
    "Can I change my shipping address?",
    "I want to upgrade my plan",
    "How do I cancel?",
    # 20 → 40 : variation lexicale
    "Issue with payment processing",
    "My card was charged twice",
    "I did not authorize this transaction",
    "Please contact support immediately",
    "I lost access to my account",
    "The app keeps crashing",
    "The website is down",
    "I cannot log in",
    "Two-factor authentication is failing",
    "I want to delete my data",
    "How long does shipping take?",
    "What is your return policy?",
    "Do you offer student discounts?",
    "Can I get a quote for bulk orders?",
    "I need an invoice for tax purposes",
    "The size I ordered is wrong",
    "I am missing items from my order",
    "Why was my order cancelled?",
    "Track my shipment please",
    "Update my email address",
    # 40 → 70 : variation longueur
    "Hi",
    "Refund",
    "Help",
    "Issue with billing — please resolve this matter as soon as possible. I have already contacted support twice and have not received a response in over a week. This is unacceptable customer service and I expect a full refund plus compensation for the inconvenience caused.",
    "Order #98765 has been delayed for 3 weeks, I want a status update",
    "I would like to cancel my subscription effective immediately and receive a prorated refund for the remaining days",
    "How do I export my data before deleting my account?",
    "The chat support agent disconnected before resolving my issue",
    "I'm being charged for a service I cancelled last month",
    "There's a discrepancy between my invoice and the amount charged to my credit card — please investigate",
    "Hello world",
    "OK",
    "Yes",
    "No",
    "Please",
    "Thank you",
    "...",
    "?",
    "!!",
    "$$",
    "I love your product so much, can you tell me about your enterprise pricing?",
    "I would not recommend your service based on my last experience",
    "Your product changed my workflow for the better",
    "The new feature is buggy on mobile",
    "Mobile app version 2.3 has a critical bug",
    "Desktop app shows wrong currency",
    "The pricing page is misleading",
    "I want to speak with a human agent",
    "Connect me to billing",
    "Transfer to technical support",
    # 70 → 100 : variation casse + unicode
    "URGENT: data loss bug",
    "urgent: data loss bug",
    "Urgent: Data Loss Bug",
    "URGENT DATA LOSS BUG",
    "Émojis et accents : café à 5€ ☕",
    "Mixing English and français in same query",
    "Test with newlines\nand tabs\there",
    "Special chars: <script>alert('xss')</script>",
    "SQL injection ' OR '1'='1",
    "Numbers: 42 1024 -7 3.14159 0.0",
    "Currency: $100 €50 ¥10000 £75 ₹500",
    "Time: 2026-05-13T12:00:00Z",
    "URL: https://example.com/path?query=value",
    "Email: user@example.com",
    "Phone: +1-555-123-4567",
    "Address: 123 Main St, Anytown, USA 12345",
    "Negative number ratio",
    "Whitespace   matters     too",
    "MixedCASEandLowercaseAndUPPERCASE",
    "snake_case_variable_name",
    "kebab-case-variable-name",
    "camelCaseVariableName",
    "PascalCaseVariableName",
    "Special chars: ~!@#$%^&*()_+`",
    "Brackets: () [] {} <> ⟨⟩",
    "Quotes: 'single' \"double\" «french» „german\"",
    "Math: α + β = γ ; ∀x∃y",
    "Long compound: hyperdimensional-vector-embedding-classification",
    "Repeated text " * 5,
    "Final fixture entry",
]

assert len(SAMPLE_TEXTS) == 100, f"expected 100 sample texts, got {len(SAMPLE_TEXTS)}"


# ────────────────────────────────────────────────────────────────────
# Python replication of the Rust D25 stub formula.
# ────────────────────────────────────────────────────────────────────


def encode_text_d25(text: str, bundle_id: str = BUNDLE_ID, dim: int = DIM) -> "np.ndarray":
    """Replicate `InnerBundle::encode(text)` from bundle.rs **bit-identical**.

    **Audit fix Φ-C-FFI.B2** — la version pre-fix utilisait f64
    Python tout du long, puis packait f32 en sortie. Le problème :
    le calcul de norm en f64 vs f32 diverge ; sur dim=384, on
    accumule ~1e-5 d'erreur sur le norm, ce qui décale 396/1536
    bytes de l'embedding f32-packed → SHA-256 totalement différent
    vs Rust. La version post-fix force f32 sur tous les opérations
    + linear sum (= match Rust `iter().map(...).sum::<f32>()`).

    Returns a numpy ndarray of dtype float32, shape (dim,), L2-norm = 1.
    """
    seed = hashlib.sha256(text.encode("utf-8") + bundle_id.encode("utf-8")).digest()
    # Étape 1 : bytes → f32 array — Rust : `(byte as f32) / 255.0 - 0.5`
    # Chaque opération en f32 ; les valeurs sont identiques à
    # `1.0_f32 / 255.0_f32` bit-pour-bit (cf. IEEE 754 round-to-nearest).
    raw = np.array([seed[i % 32] for i in range(dim)], dtype=np.uint8)
    emb = raw.astype(np.float32) / np.float32(255.0) - np.float32(0.5)
    # Étape 2 : norm en f32 via linear sum (= match Rust
    # `iter().map(|v| v * v).sum::<f32>().sqrt()`).
    # IMPORTANT : numpy `np.sum()` peut utiliser pairwise summation
    # qui diverge de Rust linear sum. On force linear sum.
    sum_of_squares = np.float32(0.0)
    for v in emb:
        # f32 multiply puis f32 add — match Rust closure exactly.
        sq = np.float32(v) * np.float32(v)
        sum_of_squares = np.float32(sum_of_squares + sq)
    norm = np.float32(math.sqrt(float(sum_of_squares)))
    if float(norm) > 0.0:
        # f32 division — match Rust `*v /= norm`.
        emb = emb / norm
    return emb


def f32_le_bytes(values: "np.ndarray") -> bytes:
    """Pack a numpy float32 array as little-endian f32 bytes.

    Equivalent à Rust `for v in &emb { hasher.update(v.to_le_bytes()); }`.
    `numpy.ndarray.tobytes()` produit le buffer native-endian ; sur
    toute plateforme supportée (x86_64, aarch64) c'est little-endian.
    """
    if not isinstance(values, np.ndarray):
        # Defensive : accept Python list for the runtime mode that
        # captures from efe.classify_zero_copy() directly.
        values = np.array(values, dtype=np.float32)
    if values.dtype != np.float32:
        values = values.astype(np.float32)
    return values.tobytes()


def synthesise_verdict_d25(
    embedding: "np.ndarray | list[float]",
    catalog_id: str,
    catalog_version: str,
    labels: list[str],
    seed: int,
) -> dict[str, Any]:
    """Replicate `controller::synthesise_verdict` from controller.rs."""
    h = hashlib.sha256()
    h.update(f32_le_bytes(embedding))
    h.update(catalog_id.encode("utf-8"))
    h.update(struct.pack("<Q", seed))  # u64 little-endian
    digest = h.digest()  # 32 bytes

    label_idx_raw = struct.unpack("<I", digest[0:4])[0]  # u32 LE
    label_idx = label_idx_raw % max(len(labels), 1)
    label = labels[label_idx] if labels else "unknown"

    # **Audit fix Φ-C-FFI.B2 (volet 2)** — Rust calcule la confidence
    # en f32 puis l'expose comme f32 field de PyVerdict. PyO3 le
    # convertit en Python float (= f64) au retour. Pour reproduire
    # bit-identique, on doit faire la division en f32 puis cast en
    # f64 — pas une division f64 directe.
    #
    # Détail subtil : `u32::MAX as f32` n'est PAS 4294967295 (= pas
    # représentable f32 ; le plus proche f32 est 4294967296 = 2^32).
    # Donc Rust effectue `(conf_u32 as f32) / 4294967296.0_f32`, ce
    # qui diverge de `conf_u32 / 0xFFFFFFFF` (f64 par défaut Python).
    conf_u32 = struct.unpack("<I", digest[4:8])[0]
    # Reproduit Rust : `(conf_u32 as f32) / (u32::MAX as f32)`
    confidence_f32 = np.float32(conf_u32) / np.float32(0xFFFF_FFFF)
    confidence = float(confidence_f32)  # f32 → f64 exact (= ce que PyO3 expose)

    rounds = 4 + (digest[8] % 16)
    swarm_id = f"swarm-{digest[:8].hex()}"

    return {
        "label": label,
        "confidence": confidence,
        "cone_id": f"cone-{catalog_id}",
        "rounds_executed": rounds,
        "swarm_id": swarm_id,
        "catalog_version": catalog_version,
    }


# ────────────────────────────────────────────────────────────────────
# Fixture generator.
# ────────────────────────────────────────────────────────────────────


def generate_fixtures(
    count: int,
    catalog_id: str,
    catalog_version: str,
    labels: list[str],
    seed_base: int = 42,
) -> list[dict[str, Any]]:
    """Build `count` fixtures by replicating the D25 stub Python-side."""
    fixtures = []
    texts = SAMPLE_TEXTS[:count] if count <= len(SAMPLE_TEXTS) else SAMPLE_TEXTS * (count // len(SAMPLE_TEXTS) + 1)
    texts = texts[:count]
    for i, text in enumerate(texts):
        # Seed varies per fixture so verdicts are spread.
        seed = seed_base + i
        embedding = encode_text_d25(text)
        expected = synthesise_verdict_d25(
            embedding=embedding,
            catalog_id=catalog_id,
            catalog_version=catalog_version,
            labels=labels,
            seed=seed,
        )
        fixtures.append(
            {
                "id": f"fixture-{i:03d}",
                "text": text,
                "catalog_id": catalog_id,
                "seed": seed,
                "expected": expected,
            }
        )
    return fixtures


# ────────────────────────────────────────────────────────────────────
# Runtime mode — capture from the real efe._native (post-D29 path).
# ────────────────────────────────────────────────────────────────────


def capture_runtime_fixtures(
    count: int,
    catalog_path: Path,
    bundle_path: Path | None,
) -> list[dict[str, Any]]:
    """Capture fixtures by actually calling `efe.classify_zero_copy`.

    Requires `efe._native` to be importable (= `maturin develop` ran).
    """
    try:
        import efe  # type: ignore
    except ImportError as e:
        sys.stderr.write(
            f"FATAL: efe._native not importable ({e}). Run `maturin develop` first.\n"
        )
        sys.exit(2)

    fixtures = []
    texts = SAMPLE_TEXTS[:count]
    bp = str(bundle_path) if bundle_path else "/etc/efe/bundles/active"
    for i, text in enumerate(texts):
        seed = 42 + i
        try:
            verdict = efe.classify_zero_copy(
                text,
                catalog_id="router-v1",
                bundle_path=bp,
                catalog_path=str(catalog_path),
                seed=seed,
            )
        except Exception as e:
            sys.stderr.write(f"fixture {i} failed: {e}\n")
            continue
        fixtures.append(
            {
                "id": f"fixture-{i:03d}",
                "text": text,
                "catalog_id": "router-v1",
                "seed": seed,
                "expected": {
                    "label": verdict.label,
                    "confidence": verdict.confidence,
                    "cone_id": verdict.cone_id,
                    "rounds_executed": verdict.rounds_executed,
                    "swarm_id": verdict.swarm_id,
                    "catalog_version": verdict.catalog_version,
                },
            }
        )
    return fixtures


# ────────────────────────────────────────────────────────────────────
# CLI.
# ────────────────────────────────────────────────────────────────────


def main() -> int:
    ap = argparse.ArgumentParser(description="Freeze FFI numerical equivalence fixtures")
    ap.add_argument(
        "--mode",
        choices=["replicate", "runtime"],
        default="replicate",
        help="`replicate` reproduces the D25 stub formula in Python ; "
        "`runtime` calls the real efe._native (post-D29).",
    )
    ap.add_argument(
        "--out",
        type=Path,
        required=True,
        help="Output JSON path",
    )
    ap.add_argument(
        "--count",
        type=int,
        default=100,
        help="Number of fixtures (default 100)",
    )
    ap.add_argument(
        "--catalog-id",
        default="router-v1",
        help="Catalog id label to use in each fixture",
    )
    ap.add_argument(
        "--catalog-version",
        default=CATALOG_VERSION,
        help="Catalog version label",
    )
    ap.add_argument(
        "--catalog-path",
        type=Path,
        help="Catalog JSON path (runtime mode only)",
    )
    ap.add_argument(
        "--bundle-path",
        type=Path,
        help="Bundle path (runtime mode only ; defaults to /etc/efe/bundles/active)",
    )
    args = ap.parse_args()

    if args.mode == "replicate":
        fixtures = generate_fixtures(
            count=args.count,
            catalog_id=args.catalog_id,
            catalog_version=args.catalog_version,
            labels=SAMPLE_LABELS,
        )
    else:
        if args.catalog_path is None:
            ap.error("--catalog-path is required in runtime mode")
        fixtures = capture_runtime_fixtures(
            count=args.count,
            catalog_path=args.catalog_path,
            bundle_path=args.bundle_path,
        )

    payload = {
        "version": "v0.6.0",
        "mode": args.mode,
        "catalog_id": args.catalog_id,
        "catalog_version": args.catalog_version,
        "labels": SAMPLE_LABELS,
        "count": len(fixtures),
        "fixtures": fixtures,
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n")
    print(f"Wrote {len(fixtures)} fixtures → {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
