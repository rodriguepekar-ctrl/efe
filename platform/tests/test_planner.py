"""Tests for the Plan / Execute / Verify planner."""

from __future__ import annotations

import pytest

from efe_platform.bot.planner import extract_recap
from efe_platform.errors import VerifyError

# ---------------------------------------------------------------------------
#  extract_recap
# ---------------------------------------------------------------------------


def test_extract_recap_happy_path() -> None:
    text = (
        "Réponse utilisateur.\n"
        '<json>{"verdict":"routed","confidence":0.9,"notes":["a"]}</json>'
    )
    user, recap = extract_recap(text)
    assert user == "Réponse utilisateur."
    assert recap["verdict"] == "routed"
    assert recap["confidence"] == 0.9
    assert recap["notes"] == ["a"]


def test_extract_recap_missing_json_is_permissive() -> None:
    user, recap = extract_recap("Juste un texte, pas de JSON.")
    assert user == "Juste un texte, pas de JSON."
    assert recap["verdict"] == "unknown"
    assert recap["confidence"] == 0.0


def test_extract_recap_invalid_json_raises() -> None:
    text = "préambule <json>this is not json</json>"
    with pytest.raises(VerifyError, match="invalid JSON"):
        extract_recap(text)


def test_extract_recap_clamps_confidence() -> None:
    text = '<json>{"verdict":"x","confidence":5.0}</json>'
    _, recap = extract_recap(text)
    assert recap["confidence"] == 1.0


def test_extract_recap_negative_confidence_floor_zero() -> None:
    text = '<json>{"verdict":"x","confidence":-0.5}</json>'
    _, recap = extract_recap(text)
    assert recap["confidence"] == 0.0


def test_extract_recap_notes_must_be_list() -> None:
    text = '<json>{"verdict":"x","notes":"oops"}</json>'
    with pytest.raises(VerifyError, match="notes must be a list"):
        extract_recap(text)


def test_extract_recap_truncates_notes_list() -> None:
    big_notes = [f"n{i}" for i in range(50)]
    import json

    payload = json.dumps({"verdict": "x", "notes": big_notes})
    _, recap = extract_recap(f"<json>{payload}</json>")
    assert len(recap["notes"]) == 20


def test_extract_recap_requires_object() -> None:
    text = "<json>[1,2,3]</json>"
    with pytest.raises(VerifyError, match="must be an object"):
        extract_recap(text)


# ---------------------------------------------------------------------------
#  End-to-end bot flow using the mock provider (uses `bot` fixture)
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_bot_conversational_passthrough(bot) -> None:  # type: ignore[no-untyped-def]
    reply = await bot.answer("Bonjour, qui êtes-vous ?")
    assert reply.ok is True
    assert len(reply.trace) == 1
    assert reply.trace[0].kind == "plan"
    assert reply.recap["verdict"] == "conversational"


@pytest.mark.asyncio
async def test_bot_tool_flow(bot) -> None:  # type: ignore[no-untyped-def]
    prompt = (
        "plan: swarm.classify "
        "tenant=default task=support cone_version=1 "
        "problem_embedding=[0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8,0.9,1.0,0.1,0.2,0.3,0.4,0.5,0.6] "
        "dim_latent=16"
    )
    reply = await bot.answer(prompt)
    assert reply.ok is True
    kinds = [e.kind for e in reply.trace]
    assert kinds == ["plan", "execute", "verify"]
    assert reply.ledger["tool_calls_made"] == 1


@pytest.mark.asyncio
async def test_bot_refuses_empty_prompt(bot) -> None:  # type: ignore[no-untyped-def]
    from efe_platform.errors import BotError

    with pytest.raises(BotError, match="must not be empty"):
        await bot.answer("   ")
