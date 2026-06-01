# SLO-2 — L2 latency p95

**Target** : in ≥ 99 % of 5-min windows, p95 latency ≤ 50 ms.

## Definition

Per-window compliance bit ; SLI = mean of bits over 30-day rolling
window (excluding low-coverage windows).

## Why "compliance of windows" and not "raw p95"

The Google SRE Workbook (ch.4) argues that user experience is driven by
the **stability** of the latency target across short windows, not by an
aggregate quantile.  This SLO matches the canonical pattern.

## Error budget

86 non-compliant 5-min windows per 30-day month.

## Badge

![](./badges/slo-2-latency-p95.svg)
