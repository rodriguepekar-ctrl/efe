# Scenario: support_triage_bilingual

**Version**: 1.0.0
**Model**: `claude-sonnet-4-6`
**Catalogs**: `support_triage`
**Languages**: French, English

## What this scenario does

Routes inbound customer-support messages into one of four intents
(`fraud`, `bug`, `question`, `billing`) using the `efe_classify`
tool, then produces a short, language-matched acknowledgement to the
user. Behaviour branches on confidence thresholds (62% / 82%) defined
by the system prompt — the decision policy lives **in the prompt**,
not in the client code.

## How to run

```bash
# Single live run (requires ANTHROPIC_API_KEY + a running efe-server):
efe-admin scenario run support_triage_bilingual \
    --user "Ma carte a été débitée sans mon consentement"

# Bench the 8 golden cases (mock mode in tests, real API if opt-in):
efe-admin scenario bench support_triage_bilingual
```

## Golden cases

Eight cases split FR / EN across the four labels. Used to gate
prompt-edit PRs: a change that drops accuracy below the current
baseline must include a documented trade-off.

## Notes

- The prompt explicitly forbids leaking confidence values to the
  end user. A diff that starts saying "I'm 67% sure this is a bug"
  is a breaking change.
- The `efe_classify` tool schema restricts the `catalog` field to
  `support_triage` at run time — a prompt that tries to pick a
  different catalog will receive a `tool_result` with
  `is_error: true`.
