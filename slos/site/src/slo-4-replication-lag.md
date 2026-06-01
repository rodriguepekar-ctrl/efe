# SLO-4 — Replication lag p99

**Target** : in ≥ 99 % of 1-min windows, p99 replication lag ≤ 100 entries.

## Definition (AD16 — Raft-aware)

For each 1-min window :
- Compute p99 lag **per participant** (using their replication
  histogram).
- Take the **minimum** across participants.
- A bit = 1 if min ≤ 100, else 0.

The min (not max) is canonical because the most-up-to-date participant
defines the commitment index in Raft consensus.  Reporting on the worst
participant would be alarmist (single lagging node should not fail the
SLO).

## Badge

![](./badges/slo-4-replication-lag.svg)
