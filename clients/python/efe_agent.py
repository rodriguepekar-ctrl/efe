"""efe_agent.py — intégration EFE + Claude prête à l'emploi.

Ce fichier expose DEUX fonctions simples, utilisables depuis n'importe
quel projet Python :

    - classify_simple(text) → ClassifyResult
          Classification pure par l'essaim. Rapide (~300-600 ms).
          Pas d'appel à Claude, donc zéro token consommé côté Anthropic.
          Retourne un objet avec label, confidence, reject, sub_scores…

    - handle_with_claude(text) → str
          Conversation complète : Claude reçoit la question, invoque
          l'essaim via le tool efe_classify, puis rédige une réponse
          finale adaptée au label + à la confiance.
          Coûte 1 à 3 centimes d'euro par appel Claude.

Installation (une fois) :
    pip install -e "/Users/rodrigue.pekar/Downloads/jjjj/efe/clients/python[dev]"
    pip install anthropic sentence-transformers

Prérequis au runtime :
    1. Le serveur EFE doit tourner quelque part (par défaut 127.0.0.1:50051).
       Voir /Users/rodrigue.pekar/Downloads/jjjj/efe/target/release/efe-server
    2. Le catalog YAML doit avoir été compilé au moins une fois
       (`efe-admin compile-catalog /path/to/support_triage.yaml`).
    3. Les variables d'environnement ci-dessous doivent être définies.

Variables d'environnement :
    ANTHROPIC_API_KEY   (requis pour handle_with_claude)  — ta clé sk-ant-…
    EFE_ENDPOINT        (défaut "127.0.0.1:50051")
    EFE_TIMEOUT_S       (défaut "60")        — timeout gRPC client
    EFE_CATALOG_PATH    (défaut : support_triage.yaml du repo)
    EFE_CLAUDE_MODEL    (défaut "claude-sonnet-4-6")
    EFE_BUDGET_MS       (défaut "60000")     — budget wall-clock essaim
    EFE_MAX_ROUNDS      (défaut "16")        — max fusion rounds

Usage minimal :
    from efe_agent import classify_simple, handle_with_claude

    # Rapide, pas d'API Claude.
    r = classify_simple("mon compte a été débité sans autorisation")
    print(r.label, round(r.confidence, 3), r.reject)
    # → fraud 0.575 False   (si les seuils du catalog sont assez bas)

    # Conversation complète.
    reply = handle_with_claude("Mon app bug depuis ce matin")
    print(reply)
    # → "Je comprends, ça doit être frustrant. Je vais ouvrir un ticket…"

Usage avancé (debug) :
    from efe_agent import conversation_with_claude, reset_state
    reply, transcript = conversation_with_claude("…")
    for msg in transcript:
        print(msg)

    reset_state()   # force le re-chargement du catalog et du client
"""

from __future__ import annotations

import os
import sys
from pathlib import Path
from typing import Any

# --------------------------------------------------------------------
# Imports EFE-tool — on tolère l'absence pour donner un message clair.
# --------------------------------------------------------------------
try:
    from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder
    from efe_tool.bridges.claude import (
        extract_final_text,
        extract_tool_result,
        run_conversation_with_tool,
    )
    from efe_tool.types import ClassifyResult
except ImportError as e:  # pragma: no cover
    # Use ``raise`` rather than ``sys.exit`` so that importing this
    # module from a notebook or a long-lived service does not kill the
    # host process. Callers that want fail-fast semantics can wrap the
    # import in their own try/except.
    raise ImportError(
        "[efe_agent] Impossible d'importer efe_tool. Installe-le d'abord :\n"
        "   pip install -e '/Users/rodrigue.pekar/Downloads/jjjj/efe/clients/python[dev]'\n"
        f"Erreur d'origine : {e}"
    ) from e


# --------------------------------------------------------------------
# Configuration — lue depuis l'environnement une seule fois.
# --------------------------------------------------------------------
_DEFAULT_CATALOG = str(
    Path("/Users/rodrigue.pekar/Downloads/jjjj/efe/catalogs/support_triage.yaml")
)


def _cfg_str(name: str, default: str) -> str:
    v = os.environ.get(name)
    return v if v else default


def _cfg_int(name: str, default: int) -> int:
    v = os.environ.get(name)
    return int(v) if v else default


CATALOG_PATH = _cfg_str("EFE_CATALOG_PATH", _DEFAULT_CATALOG)
CLAUDE_MODEL = _cfg_str("EFE_CLAUDE_MODEL", "claude-sonnet-4-6")
BUDGET_MS = _cfg_int("EFE_BUDGET_MS", 60_000)
MAX_ROUNDS = _cfg_int("EFE_MAX_ROUNDS", 16)


# --------------------------------------------------------------------
# Prompt système utilisé par Claude quand il orchestre l'essaim.
# Tu peux le remplacer par le tien si tu veux un autre ton ou d'autres
# règles de décision.
# --------------------------------------------------------------------
SYSTEM_PROMPT = """\
Tu es un assistant de support client qui route chaque message utilisateur
vers la bonne équipe en utilisant l'outil `efe_classify` (catalog = 'support_triage').

Règles strictes :

1. POUR CHAQUE message utilisateur reçu, commence par appeler
   `efe_classify(text=<message>, catalog='support_triage')`.
   Ne devine JAMAIS la catégorie toi-même.

2. L'outil renvoie `{label, confidence, alignment_delta, reject}`.
   Interprète ainsi :
      - reject=true OU confidence < 0.55 → ne devine rien. Pose UNE
        question de clarification courte et empathique à l'utilisateur.
      - confidence entre 0.55 et 0.70 → agis sur le label MAIS mentionne
        l'incertitude à l'utilisateur et propose de transférer à un humain
        s'il le souhaite.
      - confidence ≥ 0.70 → agis directement sur le label, ton confiant.

3. Catégories possibles (ne jamais en inventer d'autres) :
      - fraud    : suspicion de fraude, débit non autorisé, compte piraté.
      - bug      : problème technique, erreur, panne.
      - question : question fonctionnelle (comment faire X).
      - billing  : facturation, abonnement, remboursement.

4. Réponds TOUJOURS en français, en ton chaleureux et direct. Une réponse
   utile = 2 à 5 phrases, pas plus.

5. Ne répète JAMAIS les scores techniques à l'utilisateur (confidence,
   alignment_delta, rounds) — ce sont des détails internes.
"""


# --------------------------------------------------------------------
# État partagé (singletons lazy). Ces objets sont créés à la première
# utilisation, puis réutilisés par les appels suivants — important pour
# la latence (pas de rechargement du modèle sbert, pas de réouverture
# de la connexion gRPC, pas de re-compilation du catalog).
# --------------------------------------------------------------------
_catalog: AnchorCatalog | None = None
_efe_client: EfeClient | None = None
_anthropic_client: Any = None  # type: anthropic.Anthropic (lazy import)


def _get_catalog() -> AnchorCatalog:
    global _catalog
    if _catalog is None:
        if not Path(CATALOG_PATH).exists():
            raise FileNotFoundError(
                f"Catalog introuvable : {CATALOG_PATH}. Défini EFE_CATALOG_PATH "
                "ou compile le YAML avec `efe-admin compile-catalog ...`."
            )
        _catalog = AnchorCatalog.load(CATALOG_PATH, embedder=LocalSTEmbedder())
    return _catalog


def _get_efe_client() -> EfeClient:
    global _efe_client
    if _efe_client is None:
        # from_env() lit EFE_ENDPOINT et EFE_TIMEOUT_S.
        os.environ.setdefault("EFE_ENDPOINT", "127.0.0.1:50051")
        os.environ.setdefault("EFE_TIMEOUT_S", "60")
        _efe_client = EfeClient.from_env()
    return _efe_client


def _get_anthropic() -> Any:
    global _anthropic_client
    if _anthropic_client is None:
        try:
            from anthropic import Anthropic
        except ImportError as e:
            raise ImportError(
                "anthropic n'est pas installé. Fais :\n"
                "   pip install anthropic"
            ) from e
        if not os.environ.get("ANTHROPIC_API_KEY"):
            raise RuntimeError(
                "ANTHROPIC_API_KEY n'est pas exportée dans l'environnement. "
                "Fais : `export ANTHROPIC_API_KEY=sk-ant-…`"
            )
        _anthropic_client = Anthropic()
    return _anthropic_client


def reset_state() -> None:
    """Force le rechargement du catalog, du client gRPC et de Claude.

    Utile si tu modifies le catalog YAML à chaud, ou si tu veux purger
    le cache LRU du bridge.
    """
    global _catalog, _efe_client, _anthropic_client
    if _efe_client is not None:
        try:
            _efe_client.close()
        except Exception:
            pass
    _catalog = None
    _efe_client = None
    _anthropic_client = None


# --------------------------------------------------------------------
# API publique
# --------------------------------------------------------------------


def classify_simple(text: str) -> ClassifyResult:
    """Classifie un texte via l'essaim, sans passer par Claude.

    Très rapide (~300-600 ms en CPU), zéro coût monétaire.

    Args:
        text: La phrase à classifier.

    Returns:
        Un ClassifyResult avec les attributs `label`, `confidence`,
        `alignment_delta`, `reject`, `rounds`, `top_cosine`, `sub_scores`.
        Si la confiance est en-dessous du `confidence_medium` du catalog,
        `label` vaut "uncertain" et `reject` est True.

    Raises:
        BridgeError, NetworkError : si le serveur EFE est injoignable ou
        si l'embedder n'arrive pas à encoder.
    """
    catalog = _get_catalog()
    client = _get_efe_client()
    return client.classify(
        text,
        catalog,
        budget_wall_clock_ms=BUDGET_MS,
        max_rounds=MAX_ROUNDS,
        return_sub_scores=True,
    )


def handle_with_claude(text: str) -> str:
    """Conversation complète : Claude invoque EFE puis rédige la réponse.

    Une seule question utilisateur → une seule réponse assistant.
    Pour du multi-tour (contexte conversationnel), voir
    :func:`conversation_with_claude` plus bas.

    Args:
        text: Le message de l'utilisateur.

    Returns:
        La réponse finale de Claude, en français, adaptée au label de
        l'essaim. Texte simple, prêt à être affiché dans une UI.

    Raises:
        RuntimeError : si ANTHROPIC_API_KEY n'est pas défini.
        ImportError  : si `anthropic` n'est pas installé.
        BridgeError  : si l'appel à l'essaim échoue.
    """
    reply, _ = conversation_with_claude(text)
    return reply


def conversation_with_claude(text: str) -> tuple[str, list[dict[str, Any]]]:
    """Même chose que `handle_with_claude` mais retourne aussi le transcript.

    Pratique pour débugger ou afficher les étapes intermédiaires (tool_use,
    tool_result) dans une UI de développement.

    Returns:
        Un tuple `(reply, transcript)` où `reply` est la dernière réponse
        assistant en texte brut, et `transcript` est la liste complète
        des messages de la conversation (format Anthropic).
    """
    catalog = _get_catalog()
    client = _get_efe_client()
    anthropic = _get_anthropic()
    transcript = run_conversation_with_tool(
        anthropic,
        efe_client=client,
        catalogs={catalog.name: catalog},
        model=CLAUDE_MODEL,
        system=SYSTEM_PROMPT,
        user=text,
    )
    reply = extract_final_text(transcript) or "(pas de réponse textuelle finale)"
    return reply, transcript


# --------------------------------------------------------------------
# Mode REPL : `python efe_agent.py` lance une conversation interactive.
# --------------------------------------------------------------------
def _repl() -> None:
    print("=== EFE + Claude — REPL ===")
    print(f"Catalog       : {CATALOG_PATH}")
    print(f"Endpoint      : {os.environ.get('EFE_ENDPOINT', '127.0.0.1:50051')}")
    print(f"Modèle Claude : {CLAUDE_MODEL}")
    print("Tape ton message puis Entrée. 'quit' pour sortir.\n")
    while True:
        try:
            text = input("› Toi : ").strip()
        except (EOFError, KeyboardInterrupt):
            print()
            break
        if not text:
            continue
        if text.lower() in {"quit", "exit", "q"}:
            break
        try:
            # Un seul appel Claude → un seul appel essaim (via le
            # tool). On extrait le résultat de l'essaim directement
            # depuis le transcript pour l'afficher, sans relancer
            # l'essaim une deuxième fois. Coût = 1 essaim + 1 Claude,
            # au lieu de 2 essaims + 1 Claude.
            reply, transcript = conversation_with_claude(text)
            tool_payload = extract_tool_result(transcript)
            if tool_payload and "error" not in tool_payload:
                print(
                    f"   [essaim] label={tool_payload.get('label')} "
                    f"conf={float(tool_payload.get('confidence', 0.0)):.3f} "
                    f"reject={tool_payload.get('reject')}"
                )
            elif tool_payload and "error" in tool_payload:
                print(f"   [essaim] error={tool_payload['error']}")
            else:
                print("   [essaim] tool non invoqué par Claude")
            print(f"› Assistant : {reply}\n")
        except Exception as e:
            print(f"✗ Erreur : {type(e).__name__}: {e}\n")
    # Fermeture propre du canal gRPC.
    reset_state()


if __name__ == "__main__":
    _repl()
