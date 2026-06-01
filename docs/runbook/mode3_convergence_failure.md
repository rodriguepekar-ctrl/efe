# Runbook — Mode-3 Convergence Failure

> Sprint **Φ-D-MODE3 Completion (D73)** — artefact #31.
> Cf. `docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md` § 6.

## Symptômes

| Indicateur | Implication |
|---|---|
| `SLO-MODE3-CONVERGENCE-RATE` fail (< 90 %) | Multi-turn loop not converging |
| `efe_mode3_convergence_status_total{status="FailedToConverge"}` rate up | Sessions running to max K=10 turns |
| `efe_mode3_oscillation_detected_total` rate up | Anchor flip-flopping pattern |
| Customer reports : "system asks too many follow-up questions" | Sessions reaching max turns regularly |

## 3 root causes typiques

### Cas 1 : Convergence threshold trop strict

`cosine ≥ 0.95 for 2 turns` peut être trop strict sur certains catalogs
nuancés. Symptôme : convergence rate ~ 75-85 % au lieu de ≥ 90 %.

Diagnostic :
```bash
# Per-catalog convergence breakdown.
curl -s "http://prometheus/api/v1/query?query=sum%20by%20(catalog)(rate(efe_mode3_convergence_status_total%7Bstatus%3D%22Converged%22%7D%5B24h%5D))" | jq

# Identify catalogs with low convergence.
```

Mitigation :
* Per-catalog threshold tuning (= 0.92-0.95 selon catalog difficulty).
* Calibrate via A/B test before rolling out.

### Cas 2 : Oscillation pattern

Anchor flip-flops between 2 attractors → never converges. Caused by :
* Contradictory user inputs (= adversarial)
* Ambiguous catalog labels (= overlapping descriptions)
* Prompt instability (= LLM temperature > 0)

Diagnostic :
```bash
python3 eval/scripts/mode3_oscillation_detector.py \
    --adversarial-sessions eval/mode3/adversarial_sessions/sessions.json
```

Mitigation :
* Detect → escalate to human review (= automatic in production).
* For chronic oscillation in a catalog : tighten label descriptions OR add disambiguating prompt.

### Cas 3 : Provider drift mid-session

LLM provider returns inconsistent outputs across turns (= cache miss + different completion).

Diagnostic :
```bash
# Check provider call patterns per session.
curl -s "http://prometheus/api/v1/query?query=count%20by%20(provider%2C%20session_id)(efe_mode3_anchor_refinement_seconds)" | jq | head -50
```

Mitigation :
* Pin `temperature=0` + `top_p=1` for Mode-3 calls.
* Pin model version per session start.

## Mitigation immediate

1. Si convergence rate ≥ 85 % : monitor, no action.
2. Si 80 % ≤ rate < 90 % : `#efe-mode3` Slack within 24h.
3. Si rate < 80 % : `#efe-incidents` PagerDuty within 4h.
4. Si rate < 50 % : SEV1 PagerDuty + customer-facing alert.

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| Single-catalog convergence < 90 % | self-investigate via per-catalog metrics | 24h |
| Aggregate convergence < 90 % | `#efe-mode3` Slack | 4h |
| Aggregate convergence < 80 % | PagerDuty | 30 min |
| Aggregate convergence < 50 % | PagerDuty SEV1 | 5 min |

## Liens

* Archi § 6 : `docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md`
* SLO : `SLO-MODE3-CONVERGENCE-RATE`, `SLO-MODE3-CONVERGENCE-COSINE`
* Tool : `eval/scripts/mode3_oscillation_detector.py`
