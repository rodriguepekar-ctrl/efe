# SLO-1 — L2 availability

**Target** : ≥ 99.5 % of L2 requests succeed.

## Definition

```
SLI_1 = Σ success / Σ (success + server_error + transport_error)
```

4xx (client_error) is **excluded** from both numerator and denominator
(T96 mitigation — anti-abuse).

## Compliance windows

- 5-min : burn-rate fast detection
- 30-min : burn-rate slow detection
- 1-hour : recording rule
- 30-day : rolling compliance window

## Error budget

For target 99.5 % over 30 days, the error budget is 0.5 % × 30 d ≈
3.6 hours of non-success time.

## Live status

Latest value : [current.json](./current.json) → `slos.L2_AVAILABILITY`.

## Badge

![](./badges/slo-1-availability.svg)
