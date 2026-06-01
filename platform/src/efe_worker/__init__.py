"""EFE Training Worker — M04.

The worker is the *right hand* of the fine-tuning service: M03
decides **what** to run and **where**, and this package executes the
actual training loop inside a Kubernetes Job. It consumes a snapshot
from M01, applies one of the five supported fine-tuning methods, saves
checkpoints along the way, evaluates on validation set, then uploads
the final artifact to the registry consumed by M05/M06.

Relation to the rest of the platform:

    efe_ft        — M01 Feature Store + M02 Labeling
    efe_orch      — M03 Training Orchestrator (dispatcher)
    efe_worker    — **M04 Training Worker (this package)**

Five training methods live under :mod:`efe_worker.methods`:

    A. ``infonce``  — Contrastive Pair Learning (InfoNCE)
    B. ``triplet``  — Triplet Loss with hard-negative mining
    C. ``classif``  — Classification Cross-Entropy
    D. ``lora``     — LoRA PEFT adapters
    E. ``distill``  — Knowledge Distillation (teacher → student)

Design principles:

* The outer plumbing (transport, data, driver, observability) is pure
  Python and is testable without a GPU. torch is imported **lazily**
  inside the method classes so the package can be loaded on a machine
  that does not have torch installed (e.g., CI runners, lightweight
  checks). Code paths that actually exercise torch raise a clear
  :class:`WorkerError` if torch is missing.
* Every external side-effect (S3, gRPC, filesystem, clock) sits behind
  a protocol with a deterministic fake. Tests build fakes and assert
  on their recorded calls.
* The worker's lifecycle is short (boot → terminal) and fully driven
  by the run's state machine on the orchestrator side.

Public surface (stable):

    efe_worker.errors           — exception hierarchy
    efe_worker.config           — WorkerSettings (pydantic, frozen)
    efe_worker.context          — RunContext DI bundle
    efe_worker.transport        — gRPC + S3 + heartbeat loop
    efe_worker.data             — snapshot + samplers + tokenizer
    efe_worker.methods          — 5 training methods
    efe_worker.training         — driver + checkpoints + eval
    efe_worker.observability    — metrics, structured logging
"""

from __future__ import annotations

from efe_worker.version import VERSION, __version__

__all__ = ["VERSION", "__version__"]
