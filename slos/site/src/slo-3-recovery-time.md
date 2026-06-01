# SLO-3 — Swarm recovery time

**Target** : ≥ 99 % of chaos-induced recoveries complete within 60 s.

## Definition

Event-driven SLO.  Each recovery (participant loss → all participants
ready) contributes one compliance bit ; SLI = fraction within budget.

## Low-event guard

If fewer than 10 recovery events occur over 30 d, the SLI is reported
as **indeterminate** rather than passing by default (`PASS` from no
chaos has no operational meaning).

## Badge

![](./badges/slo-3-recovery-time.svg)
