"""Benchmark harness for the EFE platform.

Goal: produce **measured** comparative numbers (accuracy, latency,
robustness, calibration) between the swarm-backed classifier and a
naïve cosine-similarity baseline. Everything here is deterministic
and reproducible — seeded RNGs, fixed dataset, no hidden randomness.

Modules:

* :mod:`efe_platform_bench.dataset`  — reproducible synthetic dataset
  of short customer-support queries with easy + hard variants.
* :mod:`efe_platform_bench.baseline` — naïve "no-swarm" classifier.
* :mod:`efe_platform_bench.swarm_sim` — pure-Python reference
  implementation of the Rust core's ephemeral-swarm kernel.
* :mod:`efe_platform_bench.evaluate` — metrics harness.
* :mod:`efe_platform_bench.run_bench` — entry point that produces
  ``benchmark_results.json``.
"""
