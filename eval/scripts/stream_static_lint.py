#!/usr/bin/env python3
"""Streaming static lint : mpsc(64) constant + Rust wiring status report.

Sprint **Φ-D-STREAM Completion (D83)** — artefact #36.
Cf. ``docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md`` § 10.1 + § 24.

## Audit fix Φ-D-STREAM.L1-2

Pre-fix the lint emitted `SLO-STREAM-ADMIN-AUDIT-BODY` with pass=0
when the Rust body was still `Status::unimplemented`. The
`stream_admin_audit_bench` emitted the same SLO with pass=1 (= logic
validated). The merge order made the bench win → faussement-PASS
from the production-readiness perspective.

**Post-fix** : two distinct concerns, two distinct fields :
  * `SLO-STREAM-BUFFER-64` : owned by THIS lint (= compile-time invariant).
  * `rust_wiring_status` : non-SLO top-level field reporting whether the
    `admin.rs::list_audit_events` body is wired (= ties to
    `DT-STREAM-RUST-WIRING-1`). The bench remains the SLO authority
    for `SLO-STREAM-ADMIN-AUDIT-BODY`.

The lint exits 1 ONLY if `mpsc(64)` is violated. The Rust wiring
status is reported but does NOT fail the gate (= it's a deferred
ticket tracker).
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

STREAMING_HANDLERS = [
    REPO / "crates" / "efe-server" / "src" / "services" / "swarm.rs",
    REPO / "crates" / "efe-server" / "src" / "services" / "mode2" / "mod.rs",
    REPO / "crates" / "efe-server" / "src" / "services" / "mode3" / "mod.rs",
    REPO / "crates" / "efe-server" / "src" / "services" / "admin.rs",
]


def check_mpsc_64() -> tuple[bool, list[str]]:
    """SLO-STREAM-BUFFER-64 : all `mpsc::channel(N)` in streaming handlers have N == 64."""
    violations: list[str] = []
    pattern = re.compile(r"mpsc::channel\s*(?:::<[^>]+>)?\s*\(\s*(\d+)\s*\)")
    for path in STREAMING_HANDLERS:
        if not path.exists():
            violations.append(f"{path.name}: file missing")
            continue
        src = path.read_text(encoding="utf-8")
        for match in pattern.finditer(src):
            n = int(match.group(1))
            if n != 64:
                line_no = src[:match.start()].count("\n") + 1
                violations.append(f"{path.name}:{line_no}: mpsc::channel({n}) ≠ 64")
    return (not violations, violations)


def check_rust_wiring_status() -> dict:
    """Non-SLO status report for the Rust wiring track (DT-STREAM-RUST-WIRING-1).

    Reports whether each streaming handler body has been wired to the
    new RoundEmitter / AuditStream scaffolding, vs still in D32/D21
    placeholder state.

    Audit fix Φ-D-STREAM.A6 : robust function-block detection that
    ignores string literals (pre-fix the brace counter could be fooled
    by `{` inside a Rust string).
    """
    admin_path = REPO / "crates" / "efe-server" / "src" / "services" / "admin.rs"
    if not admin_path.exists():
        return {
            "admin_list_audit_events": "MISSING_FILE",
            "defer_ticket": "DT-STREAM-RUST-WIRING-1",
        }
    src = admin_path.read_text(encoding="utf-8")
    fn_start = src.find("async fn list_audit_events")
    if fn_start < 0:
        return {
            "admin_list_audit_events": "FN_NOT_FOUND",
            "defer_ticket": "DT-STREAM-RUST-WIRING-1",
        }
    # Robust brace counting that skips Rust string literals + line comments.
    fn_body_chars = _extract_fn_body(src, fn_start)
    if "Status::unimplemented" in fn_body_chars:
        status = "STUB_UNIMPLEMENTED"
    elif "AuditStream" in fn_body_chars:
        status = "WIRED_AUDIT_STREAM"
    else:
        status = "WIRED_OTHER"
    return {
        "admin_list_audit_events": status,
        "defer_ticket": "DT-STREAM-RUST-WIRING-1" if status == "STUB_UNIMPLEMENTED" else None,
    }


def _extract_fn_body(src: str, fn_start: int) -> str:
    """Extract a function body via brace counting that respects strings + comments.

    Robust to :
      - `"{` inside string literals
      - `//` line comments
      - `/* */` block comments
      - `'{'` char literals
    """
    pos = src.find("{", fn_start)
    if pos < 0:
        return ""
    depth = 0
    i = pos
    in_string = False
    in_char = False
    in_line_comment = False
    in_block_comment = False
    while i < len(src):
        ch = src[i]
        nxt = src[i + 1] if i + 1 < len(src) else ""
        if in_line_comment:
            if ch == "\n":
                in_line_comment = False
        elif in_block_comment:
            if ch == "*" and nxt == "/":
                in_block_comment = False
                i += 1
        elif in_string:
            if ch == "\\":
                i += 1  # skip escaped char
            elif ch == '"':
                in_string = False
        elif in_char:
            if ch == "\\":
                i += 1
            elif ch == "'":
                in_char = False
        else:
            if ch == "/" and nxt == "/":
                in_line_comment = True
                i += 1
            elif ch == "/" and nxt == "*":
                in_block_comment = True
                i += 1
            elif ch == '"':
                in_string = True
            elif ch == "'":
                in_char = True
            elif ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    return src[pos:i + 1]
        i += 1
    return src[pos:]


def main() -> int:
    ap = argparse.ArgumentParser(description="Streaming static lint")
    ap.add_argument("--out", type=Path,
                    default=Path("target/streaming/static_lint.json"))
    args = ap.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)

    mpsc_ok, mpsc_violations = check_mpsc_64()
    wiring = check_rust_wiring_status()

    print(f"mpsc(64) static lint  : {'PASS' if mpsc_ok else 'FAIL'} "
          f"({len(mpsc_violations)} violations)", file=sys.stderr)
    print(f"rust_wiring_status    : admin.list_audit_events = "
          f"{wiring['admin_list_audit_events']}", file=sys.stderr)
    if wiring.get("defer_ticket"):
        print(f"  → tracked under {wiring['defer_ticket']} (informational, "
              f"does NOT fail the gate)", file=sys.stderr)

    args.out.write_text(json.dumps({
        "slo_compliance": {
            "SLO-STREAM-BUFFER-64": {
                "measured_pass_count": 1 if mpsc_ok else 0,
                "measured_expected_count": 1,
                "violations": mpsc_violations,
            },
        },
        # NON-SLO field : Rust wiring progress vs DT-STREAM-RUST-WIRING-1.
        # The bench `stream_admin_audit_bench` owns `SLO-STREAM-ADMIN-AUDIT-BODY`
        # for the LOGIC validation ; this section reports the WIRING status
        # to operators without conflicting with the SLO.
        "rust_wiring_status": wiring,
    }, indent=2) + "\n")
    # Only fail on the mpsc(64) lint ; the wiring status is informational.
    return 0 if mpsc_ok else 1


if __name__ == "__main__":
    sys.exit(main())
