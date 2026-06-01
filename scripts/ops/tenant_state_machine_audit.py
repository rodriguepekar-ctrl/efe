#!/usr/bin/env python3
"""
Sprint Φ-D-OPS (D96) — Tenant FSM audit bench.

Walks the SQL transition matrix declared in
`crates/efe-server/migrations/0014_tenant_lifecycle_states.sql` and
asserts the topological invariants expected by the OPS architecture :

  1. Every state is reachable from `signup_intent`.
  2. `erased` is a sink (no outgoing edges).
  3. No cycle except for the explicit (active ↔ throttled) +
     (active ↔ suspended) admin loops.
  4. Compensation edges form a valid reverse path for each forward
     subgraph.

Emits a JSON measurements payload `{slo_id: value}` and exits non-zero
on invariant violation.

Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 7.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import defaultdict, deque
from pathlib import Path
from typing import Dict, List, Optional, Set, Tuple


_RE_TRIPLE = re.compile(
    r"\(\s*'([a-z_]+)'\s*,\s*'([a-z_]+)'\s*,\s*'([a-z]+)'\s*,",
    re.IGNORECASE,
)


def _parse_transitions(sql: str) -> List[Tuple[str, str, str]]:
    """Extract every (from_state, to_state, direction) row from the
    INSERT block of the migration file."""
    return [
        (m.group(1), m.group(2), m.group(3).lower())
        for m in _RE_TRIPLE.finditer(sql)
    ]


def _build_graph(rows: List[Tuple[str, str, str]],
                 directions: Set[str]) -> Dict[str, List[str]]:
    g: Dict[str, List[str]] = defaultdict(list)
    for f, t, d in rows:
        if d in directions:
            g[f].append(t)
    return g


def _reachable(graph: Dict[str, List[str]], start: str) -> Set[str]:
    seen: Set[str] = {start}
    q = deque([start])
    while q:
        cur = q.popleft()
        for nxt in graph.get(cur, []):
            if nxt not in seen:
                seen.add(nxt)
                q.append(nxt)
    return seen


def _has_cycle_ignoring(graph: Dict[str, List[str]],
                        ignore_edges: Set[Tuple[str, str]]) -> Optional[List[str]]:
    """Standard DFS cycle detection. Returns first cycle path found or None."""
    WHITE, GRAY, BLACK = 0, 1, 2
    color: Dict[str, int] = defaultdict(int)
    parent: Dict[str, Optional[str]] = {}
    for start in list(graph.keys()):
        if color[start] != WHITE:
            continue
        stack = [(start, iter(graph.get(start, [])))]
        color[start] = GRAY
        parent[start] = None
        while stack:
            node, it = stack[-1]
            try:
                nxt = next(it)
                if (node, nxt) in ignore_edges:
                    continue
                if color[nxt] == GRAY:
                    # Cycle.
                    cycle = [nxt]
                    cur: Optional[str] = node
                    while cur and cur != nxt:
                        cycle.append(cur)
                        cur = parent.get(cur)
                    cycle.append(nxt)
                    return list(reversed(cycle))
                if color[nxt] == WHITE:
                    color[nxt] = GRAY
                    parent[nxt] = node
                    stack.append((nxt, iter(graph.get(nxt, []))))
            except StopIteration:
                color[node] = BLACK
                stack.pop()
    return None


EXPECTED_STATES = {
    "signup_intent", "provisioning_pg", "provisioning_redis",
    "migrating_schema", "seeding_fixtures", "compliance_pending",
    "active", "throttled", "suspended",
    "erasure_pending", "erasure_in_progress", "erased",
}

# Admin loops we allow to be cyclic.
TOLERATED_CYCLE_EDGES = {
    ("active", "throttled"),
    ("throttled", "active"),
    ("active", "suspended"),
    ("suspended", "active"),
}


def main(argv: Optional[List[str]] = None) -> int:
    p = argparse.ArgumentParser(description="Tenant FSM audit")
    p.add_argument("--migration",
                   default="crates/efe-server/migrations/0014_tenant_lifecycle_states.sql")
    p.add_argument("--out", type=Path,
                   default=Path("target/ops/measures.fsm.json"))
    args = p.parse_args(argv)

    sql = Path(args.migration).read_text(encoding="utf-8")
    rows = _parse_transitions(sql)
    if not rows:
        print(f"FATAL : no transitions parsed from {args.migration}", file=sys.stderr)
        return 1
    states = {s for f, t, _ in rows for s in (f, t)}
    missing = EXPECTED_STATES - states
    extra = states - EXPECTED_STATES
    if missing or extra:
        print(f"FATAL : state set mismatch missing={missing} extra={extra}",
              file=sys.stderr)
        return 2

    forward = _build_graph(rows, {"forward", "admin"})
    reachable = _reachable(forward, "signup_intent")
    unreachable = EXPECTED_STATES - reachable
    if unreachable:
        print(f"FATAL : unreachable states from signup_intent: {unreachable}",
              file=sys.stderr)
        return 3

    if forward.get("erased"):
        print(f"FATAL : `erased` should be a sink, found edges {forward['erased']}",
              file=sys.stderr)
        return 4

    cycle = _has_cycle_ignoring(forward, TOLERATED_CYCLE_EDGES)
    if cycle:
        print(f"FATAL : unexpected cycle found: {' → '.join(cycle)}",
              file=sys.stderr)
        return 5

    # Every compensate edge must reverse a forward edge.
    forward_edges = {(f, t) for f, t, d in rows if d == "forward"}
    compensate_edges = {(f, t) for f, t, d in rows if d == "compensate"}
    invalid_compensations = [
        (f, t) for f, t in compensate_edges if (t, f) not in forward_edges
    ]
    if invalid_compensations:
        print(f"FATAL : compensation edges with no forward inverse: "
              f"{invalid_compensations}", file=sys.stderr)
        return 6

    payload = {
        "SLO-OPS-LIFECYCLE-INVARIANTS": 0,
        "_fsm_audit_meta": {
            "states_total":          len(states),
            "states_reachable":      len(reachable),
            "transitions_total":     len(rows),
            "forward_edges":         len(forward_edges),
            "compensate_edges":      len(compensate_edges),
            "cycles":                0,
        },
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    print(f"FSM audit OK → {args.out}")
    print(f"  states={len(states)}  forward={len(forward_edges)}  "
          f"compensate={len(compensate_edges)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
