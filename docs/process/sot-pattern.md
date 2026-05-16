# SOT pattern — single source of truth for parallel const lists

**Status** : canonical guide
**Process invariant** : I.2 (PR #25 § 4)
**Frozen at** : STABILITY_CONTRACT v1.7 § 25.2
**Last updated** : 2026-05-16

This document explains the **Single Source of Truth (SOT) pattern**
mandated by process invariant I.2 of PR #25.  It exists because the
PR #24 audit found three high-visibility parallel-list defects that
would have been impossible if SOT had been enforced from day 0.

> **TL;DR** : if a list of strings (paths / filenames / flags /
> endpoints / scenario IDs / …) with ≥ 5 elements appears in 2+
> places in the codebase, that's drift surface.  Consolidate to one
> canonical const ; consumers reference it.

---

## 1. Why the pattern exists — three cautionary tales

### 1.1 D-U — six of ten scenario page URLs were 404'd

The aggregator built `scenarios.json::page` URLs from a slug heuristic :
`format!("{}-{}", parts[0], parts[1])`.  The placeholder writer used a
hardcoded list :

```rust
for f in [
    "s01-baseline.html",
    "s02-canary.html",
    "s03-ramp.html",
    "s04-chaos-participant-loss.html",   // ← real
    // ...
] {
```

The two diverged for 6 of 10 scenarios :

| Real filename | Aggregator's slug | Match? |
|---|---|---|
| `s04-chaos-participant-loss.html` | `s04-chaos` | ❌ |
| `s05-chaos-network-partition.html` | `s05-chaos` | ❌ |
| `s10-slo-validation.html` | `s10-slo` | ❌ |

Every customer click-through from a scenario badge hit a 404.  Caught
on pass 5 of 10.

**Fix** : `pub const SCENARIO_PAGE_FILENAMES: &[(&str, &str)]` as the
SINGLE canonical mapping.  Aggregator looks up by ID ; placeholder
writer iterates ; ratchet iterates ; deploy verify iterates.

### 1.2 D-AS — ratchet duplicated BADGE_FILENAMES

The ratchet's `REQUIRED_BADGES` const was a parallel hardcoded copy of
`BADGE_FILENAMES`.  If someone renamed a badge in the canonical const,
the ratchet would have silently undercounted the output without
failing.

**Fix** : `pre_deploy_ratchet` now `use crate::publish::badge::BADGE_FILENAMES;`
and iterates the canonical const directly.  Same pattern for
`SCENARIO_PAGE_FILENAMES`.

### 1.3 D-T + D-AB — cosign regex documented as "shared" but duplicated

The doc said "the cosign keyless OIDC identity regex is SHARED with
PR #22 ; update only in lock-step."  In reality the regex lived in two
crates with INDEPENDENT copies that could drift :

- `bench/harness/src/publish/quarterly/mod.rs::SHARED_COSIGN_IDENTITY_REGEXP`
- `crates/efe-slos/rollup/src/evidence/mod.rs::FROZEN_IDENTITY_REGEXP`

Worse, PR #22's regex was missing the trailing `$` anchor, making it
strictly broader than PR #24's.  The "shared" label was a doc lie.

**Fix** : mirror PR #22's regex as a `const PR22_FROZEN_IDENTITY_REGEXP`
in a stability test, plus a cross-chain regex test that asserts every
PR #22 OIDC subject is accepted by the bench's broader regex.  Full
cross-crate consolidation deferred to Phase-4.

### 1.4 What these three have in common

All three are :
- A list / pattern of strings duplicated in 2+ places.
- Each copy maintained independently by a different author.
- No CI lint or test caught the drift until human audit.
- The defects had real customer-visible impact (404, false audit
  signal, asymmetric cross-chain verification).

The SOT pattern below is the structural response.

---

## 2. The decision tree

When you introduce or modify a list of strings, walk this tree :

```
Is the list ≥ 5 elements?
├── No  → No SOT required.  But still : avoid duplicating it.
└── Yes → Continue.

Will any consumer outside the declaring module iterate over the list?
├── No  → No SOT required (module-internal helper const).
└── Yes → Continue.

Is the list a STRUCTURAL contract (frozen by STABILITY_CONTRACT)?
├── Yes → SOT required.  Declare `pub const X: &[T] = &[...]` in the
│         canonical module ; every consumer imports it.  Add a
│         stability test that locks the count + content.
└── No  → Continue.

Is the list expected to grow / change as new features land?
├── Yes → SOT required.  Same shape.  Add a comment explaining the
│         growth policy (e.g. "append-only ; never re-order").
└── No  → SOT recommended but not enforced ; if you skip, add a
          comment explaining why.
```

**Examples** :

| Scenario | SOT required? | Where? |
|---|---|---|
| 11 badge filenames (frozen, all crates consume) | **Yes** | `bench/harness/src/publish/badge/mod.rs::BADGE_FILENAMES` |
| 10 scenario ID/filename mappings (frozen, 4 consumers) | **Yes** | `bench/harness/src/publish/mod.rs::SCENARIO_PAGE_FILENAMES` |
| 5 JSON API endpoint filenames (frozen) | **Yes** | `bench/harness/src/publish/api/mod.rs::API_ENDPOINT_FILENAMES` (post-PR #25) |
| 4 SLO badge URLs (cross-link with PR #22) | **Yes** | `bench/harness/src/publish/crosslink/mod.rs::SLO_BADGE_FILENAMES` |
| Test fixture : 10 scenario verdicts | No (test-local only) | inline in test file |
| 6 chaos faults (data, in YAML) | N/A | YAML files, not code |
| `xml_escape`'s 5 metacharacters | No (function-internal, escape semantics not enumerable) | match expression |

---

## 3. The canonical pattern (code)

### 3.1 Declaration

The canonical const lives in the **most general** module that
declares the concept.  Visibility is `pub` to allow cross-crate
consumption.

```rust
// bench/harness/src/publish/badge/mod.rs

/// STABILITY_CONTRACT v1.6 § 24.1 — 11 frozen badge filenames.
///
/// Append-only ; never re-order.  Adding a 12th entry requires a
/// STABILITY major bump.
pub const BADGE_FILENAMES: &[&str] = &[
    "scenario-S01.svg",
    "scenario-S02.svg",
    // ...
    "scenario-S10.svg",
    "overall.svg",
];
```

### 3.2 Consumption

Consumers `use` the canonical name and iterate ; they do NOT
re-declare or copy.

```rust
// bench/harness/src/publish/deploy/ratchet.rs

use crate::publish::badge::BADGE_FILENAMES;

pub fn run(output_root: &Path) -> BenchResult<DeployManifest> {
    let badge_count = BADGE_FILENAMES
        .iter()
        .filter(|f| output_root.join("badges").join(f).exists())
        .count();
    // ...
}
```

### 3.3 Stability test

A stability test pins the count + content.  Located in
`tests/stability_contract_v<N>.rs`.

```rust
// bench/harness/tests/stability_contract_v16.rs

use efe_bench_harness::publish::badge::BADGE_FILENAMES;

#[test]
fn v1_6_section_24_1_eleven_badges() {
    assert_eq!(BADGE_FILENAMES.len(), 11);
    assert_eq!(BADGE_FILENAMES, &[
        "scenario-S01.svg",
        "scenario-S02.svg",
        "scenario-S03.svg",
        "scenario-S04.svg",
        "scenario-S05.svg",
        "scenario-S06.svg",
        "scenario-S07.svg",
        "scenario-S08.svg",
        "scenario-S09.svg",
        "scenario-S10.svg",
        "overall.svg",
    ]);
}
```

### 3.4 Pair / tuple lists (when 1-D doesn't suffice)

When you need (id, filename) mappings (or N-tuples), use a slice of
tuples :

```rust
pub const SCENARIO_PAGE_FILENAMES: &[(&str, &str)] = &[
    ("S01_baseline_100qps_5min",    "s01-baseline.html"),
    ("S02_canary_1000qps_15min",    "s02-canary.html"),
    // ...
];

// Consumer (aggregator) :
fn scenario_page_slug(scenario_id: &str) -> String {
    if let Some((_, slug)) = SCENARIO_PAGE_FILENAMES
        .iter()
        .find(|(sid, _)| *sid == scenario_id)
    {
        return slug.trim_end_matches(".html").to_string();
    }
    // forward-compat fallback...
}
```

This was the D-U fix : the heuristic-based slug producer was replaced
by a table lookup against the canonical SOT.

---

## 4. The CI enforcement

Two layers :

### 4.1 The `process_invariants.rs` Rust test

Located in `bench/harness/tests/process_invariants.rs` (introduced in
PR #25 day 3).  Walks the workspace looking for parallel const lists ;
fails the build if any are found.

```rust
#[test]
fn i_2_no_parallel_badge_filenames_list() {
    let canonical: std::collections::HashSet<_> =
        BADGE_FILENAMES.iter().copied().collect();

    let suspicious = scan_workspace_for_string_arrays();
    for (file, line, array) in suspicious {
        if file.ends_with("badge/mod.rs") { continue; }     // canonical
        if file.ends_with("stability_contract_v16.rs") { continue; }  // test mirror, allowed
        // ... whitelist check ...

        let overlap = array.iter().filter(|s| canonical.contains(*s)).count();
        let ratio = overlap as f64 / canonical.len() as f64;
        assert!(
            ratio < 0.8,
            "I.2 violation : {file:?}:{line} contains a list of strings \
             with {ratio:.0}% overlap with BADGE_FILENAMES.  \
             Reference the canonical const instead, or add a whitelist \
             entry with justification."
        );
    }
}
```

Threshold : **80 % overlap**.  This catches genuine duplication
(D-AS had 100 %) while letting test fixtures that intentionally mirror
the canonical list to validate it (≥ 1 entry differs).

### 4.2 The `sot-audit.sh` shell scanner

Located at `scripts/sot-audit.sh`.  Human-readable + JSON-capable
companion.

```bash
# Informational scan.
scripts/sot-audit.sh

# Pre-PR check (gate mode).
scripts/sot-audit.sh --strict --threshold 80

# Machine-readable for tooling.
scripts/sot-audit.sh --json | jq '.findings[].overlap_pct'
```

Use `--strict` before opening a PR ; CI uses non-strict (rust test is
the actual gate).

---

## 5. Whitelisting legitimate duplications

There ARE legitimate cases where a list appears in multiple files :

1. **Test fixtures** that re-state the canonical list specifically to
   ASSERT that the canonical hasn't drifted.  Example :
   `stability_contract_v16.rs::v1_6_section_24_1_eleven_badges`.
2. **Cross-crate stability mirrors** (e.g. PR #25's D-AB pattern :
   the bench harness mirrors PR #22's regex in a stability test).
3. **Generated code** (e.g. `build.rs` outputs) that happens to match
   a canonical const by content.

For these, add an explicit whitelist entry in
`tests/process_invariants.rs` :

```rust
const ALLOWED_PARALLEL_DUPLICATIONS: &[(&str, &str)] = &[
    // (file_path_suffix, canonical_const_name)
    // — each row must carry a comment explaining the legitimacy.

    // Stability test mirror : asserts BADGE_FILENAMES hasn't drifted.
    // Without this mirror, a re-order of the canonical const wouldn't
    // be caught.
    ("stability_contract_v16.rs", "BADGE_FILENAMES"),

    // PR #22 cross-chain mirror : the bench harness mirrors PR #22's
    // narrower regex to detect cross-PR drift (D-AB).
    ("stability_contract_v16.rs", "PR22_FROZEN_IDENTITY_REGEXP"),
];
```

> **Hard rule** : whitelist entries require a reviewer release-eng
> approval beyond the author.  The list is reviewed quarterly to
> detect new patterns that suggest the rule should change.

---

## 6. Common mistakes

### 6.1 "I'll consolidate later"

When you add a 6th item to a 5-item list that's already duplicated,
the urge is to "just add it in both places, consolidate later."

**Don't.**  PR #24 had 6 of 10 scenario URLs broken precisely because
"later" never came.  Consolidate WHEN you add the 6th item, not later.

### 6.2 "These two lists look similar but they're for different consumers"

If consumer A iterates list X and consumer B iterates list Y, and X
and Y are 90 % identical, that's still drift surface — because some
future PR will need to add to BOTH, and the author will likely add to
only one.

**Counter-example** : `BADGE_FILENAMES` (output filenames, kebab-case)
and `SCENARIO_IDs` (scenario identifiers, SCREAMING_SNAKE_CASE).  These
are RELATED but DIFFERENT representations.  Use `SCENARIO_PAGE_FILENAMES`
as the tuple SOT, or build a helper that derives one from the other.

### 6.3 "It's just 5 strings"

5 is the threshold deliberately ; smaller lists are unlikely to drift.

**Counter-pattern** : a 4-element list of `(host, port, scheme, path)`
endpoint pieces.  Even at length 4, if it's STRUCTURED (semantic
fields), it's SOT-worthy.  Use a struct.

### 6.4 "The duplication is in a test file"

Test files CAN legitimately duplicate canonical consts to assert
stability.  BUT only in `tests/stability_contract_v*.rs` files and
with a whitelist entry.  Other tests should reference the canonical.

---

## 7. Defect taxonomy reference

For the lessons file § 3 categorisation, the 6 canonical classes :

| Class | Definition | PR #24 examples |
|---|---|---|
| **Doc/code drift** | Documentation claims behaviour the code does not exhibit | D-L (fake exit codes), D-N (smoke embed hardcoded), D-BE (cross-chain lie) |
| **Coverage gap** | Test coverage missing for a path the code supports | D-AM (byte-stable Pass-only), D-AF (uniqueness untested) |
| **Schema drift** | Two representations of the same conceptual data exist and disagree | D-P (verdict string), D-W (forward-compat partial) |
| **Security / integrity** | A failure that silently degrades security or evidence integrity | D-M (cosign silent pass), D-Z (concurrent race), D-AJ (path traversal) |
| **Spec violation** | The code violates an external spec (ISO, RFC, IETF, etc.) | D-AQ (PDF xref ISO 32000-1), D-AL (PDF font encoding), D-U (slug heuristic) |
| **Process gap** | A workflow / CI / runbook step is missing or wrong | D-AP (workflow invariants), D-BK (main workflow), D-BD (happy path untested) |

A defect that fits NONE of these proposes a new category.  Document
it in your lessons file § 5 "Process changes proposed" and start the
discussion at the next release-eng sync.

---

## 8. Migration playbook (consolidating an existing duplication)

If you find a parallel-list duplication that PR #25 missed :

1. **Identify the canonical location.**  Where does the concept most
   naturally live?  Usually the module that DEFINES the data domain
   (not the module that CONSUMES it).
2. **Hoist the const.**  Move the declaration to the canonical
   module ; make it `pub`.
3. **Re-route consumers.**  Every duplicate site replaces its local
   const with `use crate::path::canonical_const`.  Iterate, don't
   copy.
4. **Add a stability test.**  Lock the count + content in
   `tests/stability_contract_v<N>.rs`.
5. **Run `scripts/sot-audit.sh --strict`** to confirm no other site
   has been missed.
6. **Document the migration** in your lessons file § 5 (cite the
   defect codes from PR #24 if applicable).

---

## 9. References

- PR #25 architecture : [`docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md`](../architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md) § 4
- PR #24 lessons : [`docs/lessons/pr24-lessons-learned.md`](../lessons/pr24-lessons-learned.md) § 3.1 + § 6 (D-U, D-AS, D-T, D-AB)
- STABILITY_CONTRACT v1.7 § 25.2 : [`docs/STABILITY_CONTRACT.md`](../STABILITY_CONTRACT.md)
- Scanner : `scripts/sot-audit.sh --help`
- Rust enforcer : `bench/harness/tests/process_invariants.rs` (Day 3 of PR #25)
- Audit prompt (calls SOT out as defect class) : [`docs/process/audit-prompt.md`](audit-prompt.md)
