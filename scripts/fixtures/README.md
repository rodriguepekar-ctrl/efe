# Bench fixtures

Realistic `SwarmSpec` fixtures used by the Sprint Bench Foundation
benches and the swarm E2E tests.

## Files

| Fixture | dim | N agents | Anchors | Rounds | Latent footprint | SLO target p95 |
|---------|----:|---------:|--------:|-------:|-----------------:|---------------:|
| `realistic_small.json` | 128 | 8 | 4 | 8 | 4 KB | **< 50 ms** |
| `realistic_medium.json` | 256 | 32 | 8 | 8 | 32 KB | **< 200 ms** |
| `realistic_large.json` | 256 | 128 | 16 | 8 | 128 KB | **< 1 s** |

## Schema

```json
{
  "spec": {
    "dim_latent": <u32>,
    "problem_embedding": [<f32>; dim_latent],
    "population": [
      { "template_code": "na-sem"|"na-log"|"na-crit", "count": <u32> }
    ],
    "max_rounds": <u32>,
    "alignment_cone": {
      "anchors": [{ "values": [<f32>; dim_latent] }],
      "max_projection_distance": <f32>
    }
  }
}
```

The outer `"spec"` wrapper matches the legacy `scripts/fixtures/spec.json`
format used by the smoke test, so the fixtures are usable with both
the `scripts/smoke.sh` script and the bench harness.

## Anchors layout

All anchors are **one-hot orthogonal unit vectors** placed at evenly-
spaced indices `[0, dim/N, 2·dim/N, …]`. This guarantees pairwise
cosine similarity = 0 — well below the I5 anchor-deduplication
threshold (default 0.99) enforced by `alignment::policy`.

## Regeneration

```bash
python3 scripts/gen_fixtures.py
```

The generator script is the source of truth. **Edit the script, not
the JSON files** — running the script overwrites the JSON.

## Why these sizes?

* **small** is the smallest spec we expect to publish in production
  (smaller specs do not make business sense — overhead dominates).
* **medium** is the spec of an "average" Mode 1 query (multi-template
  reflection over a moderate latent).
* **large** is a stress / industrial scenario; it sets the upper
  bound of what a single CPU node can serve.

## Loading from Rust

```rust
let raw = std::fs::read_to_string("scripts/fixtures/realistic_small.json")?;
let wrapper: serde_json::Value = serde_json::from_str(&raw)?;
let spec: SwarmSpec = serde_json::from_value(
    wrapper.get("spec").cloned().unwrap()
)?;
spec.validate()?;
```
