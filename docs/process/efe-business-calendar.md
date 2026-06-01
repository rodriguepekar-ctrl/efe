# EFE business calendar

**Status** : live calendar (append-only by year)
**Process invariant** : G.15 deadline algorithm (PR #26 § 5.3.2)
**Frozen at** : STABILITY_CONTRACT v1.7 § 26.4
**Owner** : `@efe-ai/release-eng` (writes entries) ; SRE leadership (reviews annually)
**Cadence** : appended each calendar year ; reviewed once per Q4 for next year

This file records the public holidays the post-mortem deadline
algorithm treats as non-business days. A deadline that lands on a
listed holiday is shifted to the next business day (per § 26.4).

The initial entries cover Swedish national holidays (the geography
of the founding team) plus the cross-European Christmas and Easter
clusters. Additional national lists may be added by future PRs as
the team's geographic footprint grows.

> **Format** : each entry is a YYYY-MM-DD date in UTC, paired with
> a short human-readable name. The deadline algorithm reads only
> the dates ; the names are documentation.

---

## 1. 2026

### 1.1 Swedish national holidays

| Date | Name |
|------|------|
| 2026-01-01 | New Year's Day |
| 2026-01-06 | Epiphany |
| 2026-04-03 | Good Friday |
| 2026-04-06 | Easter Monday |
| 2026-05-01 | Labour Day |
| 2026-05-14 | Ascension Day |
| 2026-06-06 | National Day of Sweden |
| 2026-06-19 | Midsummer's Eve (half-day, but treated as a holiday) |
| 2026-06-20 | Midsummer's Day |
| 2026-12-24 | Christmas Eve (half-day, but treated as a holiday) |
| 2026-12-25 | Christmas Day |
| 2026-12-26 | Boxing Day (Second Day of Christmas) |
| 2026-12-31 | New Year's Eve (half-day, but treated as a holiday) |

### 1.2 European Christmas cluster

The week of 22 December through 1 January is treated as a single
non-business cluster, on the empirical observation that the team's
effective output drops sharply in this window even outside the
specific public-holiday dates. Deadlines that would otherwise fall
within this window are shifted to the first business day on or
after 2 January.

### 1.3 European Easter cluster

The four days from Maundy Thursday through Easter Monday inclusive
are treated as a single non-business cluster.

## 2. 2027

### 2.1 Swedish national holidays

| Date | Name |
|------|------|
| 2027-01-01 | New Year's Day |
| 2027-01-06 | Epiphany |
| 2027-03-26 | Good Friday |
| 2027-03-29 | Easter Monday |
| 2027-05-01 | Labour Day |
| 2027-05-06 | Ascension Day |
| 2027-06-06 | National Day of Sweden |
| 2027-06-25 | Midsummer's Eve |
| 2027-06-26 | Midsummer's Day |
| 2027-12-24 | Christmas Eve |
| 2027-12-25 | Christmas Day |
| 2027-12-26 | Boxing Day |
| 2027-12-31 | New Year's Eve |

### 2.2 Clusters

As 2026.

---

## 3. How to extend this file

Adding a new year is a regular PR ; it does NOT require a
STABILITY-bump because the per-year entries are content, not
schema. The schema (the table shape, the cluster definitions) is
frozen under § 26.4.

Adding a new national list (e.g. for a new geography the team
enters) is a regular PR that adds a new subsection at the same
level as § 1.1 ; the deadline algorithm interprets every dated
entry in this file as a non-business day regardless of which
subsection it appears in.

The Q4 annual review verifies that the next year's dates are
present before the year begins.

---

_Frozen under STABILITY § 26.4 ; the schema (table shape, cluster
definitions, year-section structure) is frozen for six chart-minor
versions. The per-year date entries are content and may be added,
corrected, or revised in regular PRs._
