"""Sprint D7 — automated calibration sweep for the Mode-1 β/α schedule.

Goal
====

Find `(β₀*, λ_β*, α₀*)` in `Mode1Config.fusion` that maximises the
aggregate `macro_f1` on N (catalog, golden) pairs without overfitting
any single catalog. The chosen tuple is emitted as JSON so the
operator can hardcode it in
`crates/efe-server/src/mode1.rs::build_mode1_fusion_config()` and
freeze the corresponding `bench_baselines/v0.1.7/baseline.json`.

How it works
============

1. **Build** a release binary of `efe-server` once (or rely on a
   pre-built one if `--prebuilt` is set).
2. **For each tuple** in the sweep grid :
   a. Spawn an `efe-server` instance with the relevant
      `EFE_FUSION_*` env vars set.
   b. Wait for the process to bind (poll the gRPC port).
   c. Run `eval/scripts/run_eval.py --multi-catalog` against it.
   d. Parse the JSON report.
   e. Kill the server, free the port.
3. **Rank** all tuples by `aggregate_macro_f1` (tie-break by
   `min_per_catalog_macro_f1`, then by `mean_conf_correct`).
4. **Emit** the winner + the full grid CSV to
   `target/calibration-d7/{winner.json, grid.csv, full.json}`.

Sub-grid vs full grid
=====================

The architecture (annexe B) defines an 80-point full grid. By
default the script runs a 27-point sub-grid centred on the
analytical starting point (β₀=0.45, λ_β=0.05, α₀=0.30) — which
covers the regime described in §5 of the architecture document.
Pass `--full-grid` to run the 80-point sweep.

CLI
===

::

    python eval/scripts/calibrate_mode1.py \\
        --catalog catalogs/support_triage.yaml \\
        --golden  eval/golden/support_triage.jsonl \\
        --catalog catalogs/support_triage.yaml \\
        --golden  eval/golden/fraud_alert.jsonl \\
        --catalog catalogs/support_triage.yaml \\
        --golden  eval/golden/billing_escalation.jsonl \\
        --output-dir target/calibration-d7 \\
        [--full-grid]              # 80 points instead of 27
        [--prebuilt path/to/efe-server]
        [--port-base 60100]        # spawned server uses 60100, 60101, …
        [--quiet]
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import shutil
import socket
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
RUN_EVAL = HERE / "run_eval.py"


# ---------------------------------------------------------------------------
# Sweep grids
# ---------------------------------------------------------------------------

# Sub-grid (27 points) — centred on the analytical D7 starting point.
SUB_GRID_BETA_INIT = [0.40, 0.45, 0.50]
SUB_GRID_BETA_DECAY = [0.03, 0.05, 0.08]
SUB_GRID_ALPHA_INIT = [0.25, 0.30, 0.35]

# Full grid (80 points) — exhaustive sweep over the architecture's
# annexe B.
FULL_GRID_BETA_INIT = [0.30, 0.40, 0.45, 0.50, 0.60]
FULL_GRID_BETA_DECAY = [0.02, 0.05, 0.08, 0.12]
FULL_GRID_ALPHA_INIT = [0.20, 0.25, 0.30, 0.40]


@dataclass(frozen=True)
class GridPoint:
    """One coordinate of the sweep grid."""

    beta_init: float
    beta_decay: float
    alpha_init: float

    def as_env(self) -> dict[str, str]:
        return {
            "EFE_FUSION_BETA_INIT": f"{self.beta_init}",
            "EFE_FUSION_BETA_DECAY": f"{self.beta_decay}",
            "EFE_FUSION_ALPHA_INIT": f"{self.alpha_init}",
        }

    def label(self) -> str:
        return (
            f"b{self.beta_init:.2f}_d{self.beta_decay:.2f}_a{self.alpha_init:.2f}"
        )


@dataclass
class GridResult:
    """The metrics produced by one (β₀, λ_β, α₀) evaluation."""

    point: GridPoint
    aggregate_macro_f1: float
    aggregate_accuracy_top1: float
    min_per_catalog_macro_f1: float
    min_per_catalog_accuracy: float
    per_catalog: list[dict[str, Any]] = field(default_factory=list)
    error: str | None = None

    def rank_key(self) -> tuple[float, float, float]:
        """Sort key — higher is better, hence we negate for `sorted`."""
        return (
            self.aggregate_macro_f1,
            self.min_per_catalog_macro_f1,
            self.aggregate_accuracy_top1,
        )


# ---------------------------------------------------------------------------
# Build / spawn helpers
# ---------------------------------------------------------------------------


def cargo_build_release() -> Path:
    """Compile `efe-server --release` and return the resulting binary
    path. Raises if cargo fails."""
    print(">>> cargo build --release -p efe-server")
    rc = subprocess.run(
        ["cargo", "build", "--release", "-p", "efe-server"],
        cwd=ROOT,
        check=False,
    ).returncode
    if rc != 0:
        raise SystemExit(f"cargo build failed (rc={rc})")
    bin_path = ROOT / "target" / "release" / "efe-server"
    if not bin_path.is_file():
        raise SystemExit(f"expected binary at {bin_path} but it does not exist")
    return bin_path


def find_free_port(start: int) -> int:
    """Probe `start, start+1, …` for the first free TCP port. We need
    this in case the operator's machine already has a server bound to
    `--port-base`."""
    for port in range(start, start + 1000):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            try:
                s.bind(("127.0.0.1", port))
                return port
            except OSError:
                continue
    raise SystemExit(f"no free port in [{start}, {start + 1000})")


class BindTimeoutError(RuntimeError):
    """Raised by `wait_for_port` when the spawned server fails to bind
    within the deadline. Caught by `evaluate_point` so the sweep can
    skip this grid point and continue, rather than terminating the
    whole campaign on the first slow start."""


def wait_for_port(host: str, port: int, timeout_s: float = 30.0) -> None:
    """Block until the given TCP port accepts connections, or raise
    [`BindTimeoutError`] on timeout.

    Note : we deliberately raise a regular `Exception` (not
    `SystemExit`) because the calibration loop must keep iterating
    even if one tuple's server fails to start — see `evaluate_point`.
    """
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            s.settimeout(0.5)
            try:
                s.connect((host, port))
                return
            except OSError:
                time.sleep(0.2)
    raise BindTimeoutError(
        f"server on {host}:{port} did not accept within {timeout_s}s"
    )


@dataclass
class SpawnedServer:
    """Bag holding the live process + the log file handle so the
    caller can close BOTH on tear-down. Without this, the log file
    descriptor leaks across grid points (80 sweeps → 80 leaked FDs →
    `Too many open files` on macOS)."""
    proc: subprocess.Popen
    log_fh: Any


def spawn_server(
    binary: Path,
    point: GridPoint,
    port: int,
    log_file: Path,
) -> SpawnedServer:
    """Spawn an efe-server with the right env vars, redirecting output
    to a per-tuple log file so the calibration log stays terse."""
    env = os.environ.copy()
    env.update(point.as_env())
    env["EFE_LISTEN_ADDR"] = f"127.0.0.1:{port}"
    # Force the metrics endpoint to a different port to avoid collision
    # with another instance.
    env["EFE_METRICS_ADDR"] = f"127.0.0.1:{port + 1000}"
    # We never want the calibration sweep to interact with auth or
    # rate-limiting — both layers are off in this binary anyway, but
    # we make it explicit.
    env["EFE_AUTH_ENABLED"] = "false"
    env["EFE_RATELIMIT_ENABLED"] = "false"
    log_file.parent.mkdir(parents=True, exist_ok=True)
    fh = log_file.open("wb")
    proc = subprocess.Popen(
        [str(binary)],
        stdout=fh,
        stderr=subprocess.STDOUT,
        env=env,
        cwd=ROOT,
    )
    return SpawnedServer(proc=proc, log_fh=fh)


def kill_server(server: SpawnedServer) -> None:
    """Send SIGTERM, reap, then close the log file handle.

    The handle close is the critical anti-FD-leak step : without it,
    a full 80-point sweep accumulates 80 open file descriptors and
    eventually trips the per-process FD limit on macOS (default 256).
    """
    proc = server.proc
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=10.0)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=2.0)
    # Always close the log fh (idempotent — Python guards against
    # double-close on file objects).
    try:
        server.log_fh.close()
    except OSError:
        pass


# ---------------------------------------------------------------------------
# Eval per grid point
# ---------------------------------------------------------------------------


def run_eval_multi_catalog(
    catalogs: list[str],
    goldens: list[str],
    endpoint: str,
    output_path: Path,
    quiet: bool,
) -> dict[str, Any]:
    """Invoke `run_eval.py --multi-catalog` against the running server
    and return the parsed JSON. Raises on any non-zero rc."""
    cmd: list[str] = [
        sys.executable,
        str(RUN_EVAL),
        "--multi-catalog",
        "--endpoint",
        endpoint,
        "--output",
        str(output_path),
    ]
    for cat, gold in zip(catalogs, goldens):
        cmd.extend(["--catalog", cat, "--golden", gold])

    stdout_target: Any = subprocess.DEVNULL if quiet else None
    res = subprocess.run(cmd, cwd=ROOT, stdout=stdout_target, stderr=stdout_target)
    if res.returncode not in (0, 1):
        # rc=1 is "gates failed" — we still want to read the metrics,
        # because a sub-optimal point in the sweep is expected to fail
        # the gate. Anything else (≥ 2) is a hard error.
        raise SystemExit(
            f"run_eval.py crashed for endpoint={endpoint} "
            f"(rc={res.returncode})"
        )
    return json.loads(output_path.read_text())


def evaluate_point(
    point: GridPoint,
    catalogs: list[str],
    goldens: list[str],
    binary: Path,
    port: int,
    output_dir: Path,
    quiet: bool,
) -> GridResult:
    """End-to-end : spawn server with `point`, run eval, kill server.

    Returns a `GridResult` even on failure (with `error` populated)
    so the sweep can continue past a bad tuple. This is a key
    robustness contract — a 1 h sweep that crashes on an early
    tuple wastes the whole campaign.
    """
    log_file = output_dir / "logs" / f"{point.label()}.log"
    eval_out = output_dir / "evals" / f"{point.label()}.json"
    eval_out.parent.mkdir(parents=True, exist_ok=True)
    server = spawn_server(binary, point, port, log_file)
    report: dict[str, Any] | None = None
    error: str | None = None
    try:
        try:
            wait_for_port("127.0.0.1", port, timeout_s=30.0)
        except BindTimeoutError as e:
            error = f"bind timeout : {e}"
            return GridResult(
                point=point,
                aggregate_macro_f1=0.0,
                aggregate_accuracy_top1=0.0,
                min_per_catalog_macro_f1=0.0,
                min_per_catalog_accuracy=0.0,
                error=error,
            )
        try:
            report = run_eval_multi_catalog(
                catalogs=catalogs,
                goldens=goldens,
                endpoint=f"127.0.0.1:{port}",
                output_path=eval_out,
                quiet=quiet,
            )
        except (SystemExit, json.JSONDecodeError, OSError) as e:
            error = f"run_eval failure : {e}"
            return GridResult(
                point=point,
                aggregate_macro_f1=0.0,
                aggregate_accuracy_top1=0.0,
                min_per_catalog_macro_f1=0.0,
                min_per_catalog_accuracy=0.0,
                error=error,
            )
    finally:
        kill_server(server)

    if report is None:
        # Defensive — should be unreachable since both error paths
        # return early above, but the type checker can't see that.
        return GridResult(
            point=point,
            aggregate_macro_f1=0.0,
            aggregate_accuracy_top1=0.0,
            min_per_catalog_macro_f1=0.0,
            min_per_catalog_accuracy=0.0,
            error=error or "no report produced",
        )
    aggregate = report["aggregate"]
    return GridResult(
        point=point,
        aggregate_macro_f1=float(aggregate["aggregate_macro_f1"]),
        aggregate_accuracy_top1=float(aggregate["aggregate_accuracy_top1"]),
        min_per_catalog_macro_f1=float(aggregate["min_per_catalog_macro_f1"]),
        min_per_catalog_accuracy=float(aggregate["min_per_catalog_accuracy"]),
        per_catalog=list(aggregate["per_catalog"]),
    )


# ---------------------------------------------------------------------------
# Grid construction + ranking
# ---------------------------------------------------------------------------


def build_grid(full: bool) -> list[GridPoint]:
    bs = FULL_GRID_BETA_INIT if full else SUB_GRID_BETA_INIT
    ds = FULL_GRID_BETA_DECAY if full else SUB_GRID_BETA_DECAY
    as_ = FULL_GRID_ALPHA_INIT if full else SUB_GRID_ALPHA_INIT
    return [GridPoint(b, d, a) for b in bs for d in ds for a in as_]


def rank_results(results: Iterable[GridResult]) -> list[GridResult]:
    """Sort descending by `rank_key` ; failures sink to the bottom."""
    valid = [r for r in results if r.error is None]
    failed = [r for r in results if r.error is not None]
    valid.sort(key=GridResult.rank_key, reverse=True)
    return valid + failed


# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------


def write_outputs(
    output_dir: Path,
    results: list[GridResult],
    catalogs: list[str],
    goldens: list[str],
) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)

    # winner.json — first ranked result + extra metadata.
    if not results:
        return
    winner = results[0]
    winner_payload = {
        "sprint": "D7",
        "winner": {
            "problem_anchor_init": winner.point.beta_init,
            "problem_anchor_decay": winner.point.beta_decay,
            "alignment_strength_init": winner.point.alpha_init,
            "aggregate_macro_f1": winner.aggregate_macro_f1,
            "aggregate_accuracy_top1": winner.aggregate_accuracy_top1,
            "min_per_catalog_macro_f1": winner.min_per_catalog_macro_f1,
            "min_per_catalog_accuracy": winner.min_per_catalog_accuracy,
            "per_catalog": winner.per_catalog,
        },
        "catalogs": catalogs,
        "goldens": goldens,
        "n_grid_points": len(results),
        "valid_grid_points": sum(1 for r in results if r.error is None),
    }
    (output_dir / "winner.json").write_text(
        json.dumps(winner_payload, indent=2, ensure_ascii=False)
    )

    # full.json — every result with the per-catalog breakdown.
    full_payload = [
        {
            **{
                "beta_init": r.point.beta_init,
                "beta_decay": r.point.beta_decay,
                "alpha_init": r.point.alpha_init,
            },
            "aggregate_macro_f1": r.aggregate_macro_f1,
            "aggregate_accuracy_top1": r.aggregate_accuracy_top1,
            "min_per_catalog_macro_f1": r.min_per_catalog_macro_f1,
            "min_per_catalog_accuracy": r.min_per_catalog_accuracy,
            "per_catalog": r.per_catalog,
            "error": r.error,
        }
        for r in results
    ]
    (output_dir / "full.json").write_text(
        json.dumps(full_payload, indent=2, ensure_ascii=False)
    )

    # grid.csv — operator-friendly tabular form for spreadsheet review.
    with (output_dir / "grid.csv").open("w", newline="") as fh:
        writer = csv.writer(fh)
        writer.writerow(
            [
                "rank",
                "beta_init",
                "beta_decay",
                "alpha_init",
                "aggregate_macro_f1",
                "aggregate_accuracy_top1",
                "min_per_catalog_macro_f1",
                "min_per_catalog_accuracy",
                "error",
            ]
        )
        for rank, r in enumerate(results, start=1):
            writer.writerow(
                [
                    rank,
                    r.point.beta_init,
                    r.point.beta_decay,
                    r.point.alpha_init,
                    f"{r.aggregate_macro_f1:.4f}",
                    f"{r.aggregate_accuracy_top1:.4f}",
                    f"{r.min_per_catalog_macro_f1:.4f}",
                    f"{r.min_per_catalog_accuracy:.4f}",
                    r.error or "",
                ]
            )


# ---------------------------------------------------------------------------
# CLI / main
# ---------------------------------------------------------------------------


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Sprint D7 — calibrate Mode-1 β/α via empirical sweep."
    )
    parser.add_argument("--catalog", action="append", required=True)
    parser.add_argument("--golden", action="append", required=True)
    parser.add_argument(
        "--output-dir",
        default="target/calibration-d7",
        help="Directory to write winner.json / full.json / grid.csv",
    )
    parser.add_argument(
        "--full-grid",
        action="store_true",
        help="Sweep the full 80-point grid (annexe B). "
        "Default : the 27-point sub-grid centred on the analytical D7 point.",
    )
    parser.add_argument(
        "--prebuilt",
        default=None,
        help="Path to a pre-built efe-server binary. If absent, the script "
        "runs `cargo build --release -p efe-server` first.",
    )
    parser.add_argument(
        "--port-base",
        type=int,
        default=60100,
        help="The first server instance binds at this port. Subsequent "
        "instances try `port_base + k` until a free port is found.",
    )
    parser.add_argument("--quiet", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if len(args.catalog) != len(args.golden):
        print(
            f"FATAL : --catalog ({len(args.catalog)}) and --golden "
            f"({len(args.golden)}) counts must match.",
            file=sys.stderr,
        )
        return 2
    if not args.catalog:
        print("FATAL : at least one --catalog / --golden pair required.", file=sys.stderr)
        return 2

    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    if (output_dir / "logs").is_dir():
        # Clean previous logs to keep the output dir tidy across re-runs.
        shutil.rmtree(output_dir / "logs")
    if (output_dir / "evals").is_dir():
        shutil.rmtree(output_dir / "evals")

    binary = Path(args.prebuilt) if args.prebuilt else cargo_build_release()
    if not binary.is_file():
        print(f"FATAL : binary {binary} does not exist", file=sys.stderr)
        return 2

    grid = build_grid(full=args.full_grid)
    print(f"Sweep grid : {len(grid)} points "
          f"({'full' if args.full_grid else 'sub'}-grid)")

    results: list[GridResult] = []
    for idx, point in enumerate(grid, start=1):
        port = find_free_port(args.port_base + (idx - 1) * 2)
        print(f"[{idx}/{len(grid)}] {point.label()} on port {port} ...")
        result = evaluate_point(
            point=point,
            catalogs=args.catalog,
            goldens=args.golden,
            binary=binary,
            port=port,
            output_dir=output_dir,
            quiet=args.quiet,
        )
        if result.error:
            print(f"    ✗ ERROR : {result.error}")
        else:
            print(
                f"    ✓ macro_f1={result.aggregate_macro_f1:.4f}  "
                f"acc={result.aggregate_accuracy_top1:.4f}  "
                f"min_f1={result.min_per_catalog_macro_f1:.4f}"
            )
        results.append(result)

    ranked = rank_results(results)
    write_outputs(output_dir, ranked, args.catalog, args.golden)
    if ranked and ranked[0].error is None:
        w = ranked[0]
        print(
            "\n=== WINNER ===\n"
            f"  β₀ = {w.point.beta_init}\n"
            f"  λ_β = {w.point.beta_decay}\n"
            f"  α₀ = {w.point.alpha_init}\n"
            f"  aggregate_macro_f1 = {w.aggregate_macro_f1:.4f}\n"
            f"  aggregate_accuracy = {w.aggregate_accuracy_top1:.4f}\n"
            f"  min_per_catalog_f1 = {w.min_per_catalog_macro_f1:.4f}\n"
            f"  min_per_catalog_acc = {w.min_per_catalog_accuracy:.4f}\n"
            f"\n  outputs : {output_dir.resolve()}"
        )
        return 0
    print("\nNo valid sweep result. Inspect logs in "
          f"{output_dir / 'logs'}.", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
