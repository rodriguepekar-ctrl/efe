#!/usr/bin/env python3
"""Generate the 100 quartet fixtures for the cross-SDK contract test.

Sprint **Φ-C-SDK Completion (D30)** — artefact #11.
Cf. ``docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md`` § 5.4.

## Mission

Produit ``fixtures/quartet_fixtures.json`` contenant 100 entrées
``(id, text, catalog_id, seed)``. Pas d'expected verdict (= le ground
truth est calculé par le serveur, comparé entre SDKs uniquement).

## Doctrine

Les fixtures couvrent :
* 30 % English (= primary language)
* 20 % French, Spanish, German, Italian, Portuguese
* 15 % Asian languages (CJK)
* 15 % edge cases (long, short, Unicode, special chars)
* 20 % stress (= numbers, code-like, mixed-script)

Les seeds varient (42 + index) pour exercer la matrice swarm_id.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

SAMPLE_TEXTS = [
    # English (30)
    "I want to dispute the charge on my account",
    "Where is my package?",
    "Cancel my subscription please",
    "I need a refund for order #12345",
    "Help me understand the bill",
    "The product arrived damaged",
    "When will my order be delivered?",
    "Can I change my shipping address?",
    "I want to upgrade my plan",
    "How do I cancel?",
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

    # French (20)
    "Quand est-ce que ma commande arrive ?",
    "Comment fonctionne votre garantie ?",
    "Je voudrais résilier mon abonnement",
    "Le produit est arrivé endommagé",
    "Je n'ai pas reçu ma facture",
    "Pourriez-vous m'aider avec mon compte ?",
    "Je voudrais un remboursement",
    "Mon paiement a échoué",
    "Comment changer mon adresse de livraison ?",
    "Je ne peux pas me connecter à mon compte",
    "Avez-vous des promotions en cours ?",
    "Quels sont vos délais de livraison ?",
    "Je voudrais parler à un agent humain",
    "Mon code de vérification ne fonctionne pas",
    "Comment supprimer mon compte ?",
    "Mes données personnelles sont-elles protégées ?",
    "Je voudrais signaler un problème de sécurité",
    "Comment configurer la double authentification ?",
    "Le site Web ne charge pas correctement",
    "Pouvez-vous m'envoyer une nouvelle facture ?",

    # Spanish (10) — accents + ñ
    "¿Cómo puedo cambiar mi contraseña?",
    "Quiero solicitar un reembolso",
    "Mi pedido no ha llegado",
    "El producto está dañado",
    "Necesito hablar con un agente",
    "¿Cuánto tarda el envío?",
    "He recibido el artículo equivocado",
    "Mi tarjeta fue rechazada",
    "No recuerdo mi contraseña",
    "La aplicación se cierra constantemente",

    # German (5)
    "Wie kann ich mein Konto kündigen?",
    "Ich möchte mein Geld zurück",
    "Die Lieferung ist beschädigt angekommen",
    "Mein Passwort funktioniert nicht",
    "Wann wird meine Bestellung geliefert?",

    # Asian (CJK 10)
    "プライバシーポリシーを教えてください",
    "我想要退款",
    "안녕하세요, 환불 부탁드립니다",
    "私の注文はいつ届きますか",
    "您能告诉我物流信息吗",
    "비밀번호를 잊어버렸습니다",
    "請問你們的退貨政策",
    "アカウントを削除したい",
    "주문 취소를 원합니다",
    "我的包裹送错了地址",

    # Portuguese (5)
    "Há algum desconto disponível?",
    "Preciso cancelar meu pedido",
    "Quando chegará minha encomenda?",
    "Quero falar com um atendente",
    "Minha senha não funciona",

    # Edge cases (20)
    "Hi",
    "Help",
    "Refund",
    "OK",
    "?",
    "!!",
    "...",
    "$$$",
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
    "Issue with billing — please resolve this matter ASAP. I have already contacted support twice and have not received a response in over a week. This is unacceptable customer service.",
]

assert len(SAMPLE_TEXTS) == 100, f"expected 100 samples, got {len(SAMPLE_TEXTS)}"


def generate(count: int = 100, catalog_id: str = "router-v1", seed_base: int = 42) -> dict:
    fixtures = [
        {
            "id": f"fixture-{i:03d}",
            "text": text,
            "catalog_id": catalog_id,
            "seed": seed_base + i,
        }
        for i, text in enumerate(SAMPLE_TEXTS[:count])
    ]
    return {
        "version": "v0.6.0",
        "count": len(fixtures),
        "catalog_id": catalog_id,
        "fixtures": fixtures,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description="Generate quartet fixtures")
    ap.add_argument(
        "--out", type=Path,
        default=Path(__file__).parent / "fixtures" / "quartet_fixtures.json",
    )
    ap.add_argument("--count", type=int, default=100)
    ap.add_argument("--catalog-id", default="router-v1")
    args = ap.parse_args()

    payload = generate(count=args.count, catalog_id=args.catalog_id)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n")
    print(f"wrote {payload['count']} fixtures → {args.out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
