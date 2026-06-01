# Error budget

For an SLO target $T$ over a rolling 30-day window, the error budget is

```
B = (1 - T) × TotalEvents_{30d}
```

The fraction consumed is

```
C/B  where C is the count of bad events in the window
```

The fraction **remaining** is reported per SLO in
[current.json](./current.json) under `error_budget_remaining_pct`.

## Worked example — SLO-1 over May 2026

```
TotalEvents  = 4 842 331
BadEvents    =    18 414
T            = 0.995

B  = 0.005 × 4 842 331 ≈ 24 212
C  = 18 414
R% = (24 212 - 18 414) / 24 212 × 100 = 23.95 %
```

⇒ 24 % of the May error budget remained at end of month.

## Burn rate

Burn rate = (1 - SLI) / (1 - T).  A burn rate of 1 means we're
consuming budget exactly as fast as it accrues (sustainable
indefinitely).  Burn ≥ 14.4 means budget exhausted in 2 days.  Burn ≥ 6
means exhausted in 5 days.  Google SRE 2-window alerts fire on both.
