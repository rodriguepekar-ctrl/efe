# Frozen bench baselines

This directory holds **frozen** bench reports used as references for
regression detection. Each subdirectory `v{X.Y.Z}/` contains:

- `baseline.json` — output of `scripts/parse_criterion_report.py`
  for a clean bench run, captured at release time.

## Why versioned

The criterion JSON tree at `target/criterion/<bench>/v{X.Y.Z}/` is
**not** versioned (it lives under `target/` which is `.gitignore`d).
Versioning the consolidated `bench-report-*.json` separately gives:

- A small, human-readable, diff-friendly artefact.
- A canonical reference for `bench.yml` to compare PR runs against.
- Easy historical browsing without GB of binary criterion data.

## Process

To **freeze a new baseline** at release time:

```bash
# 1. Clean build, ensure repo is on the release tag
cargo clean
git checkout v0.2.0

# 2. Run the bench and parse the report
scripts/bench_runner.sh

# 3. Promote the latest report to a frozen baseline
mkdir -p bench_baselines/v0.2.0
cp target/bench-report-*-$(git rev-parse --short HEAD).json \
   bench_baselines/v0.2.0/baseline.json

# 4. Commit
git add bench_baselines/v0.2.0/
git commit -m "chore(bench): freeze v0.2.0 baseline"
```

## Comparing to a baseline

The CI workflow `.github/workflows/bench.yml` parses the latest
report and compares each measurement to `bench_baselines/v0.1.0/baseline.json`.
Locally:

```bash
scripts/bench_runner.sh
python3 scripts/compare_baselines.py \
    --reference bench_baselines/v0.1.0/baseline.json \
    --candidate target/bench-report-*.json
```

(`compare_baselines.py` is a Sprint follow-up — for v0.1.0, manual
comparison is sufficient.)
