#!/usr/bin/env python3
"""Sprint D13 — audit-stream export to compliance archive.

Pulls audit events from Loki via the LogQL query API for a
configurable time window, validates each event against the D13
audit JSON-Schema, and writes a gzipped NDJSON file for archival.

Use case : 7-year compliance retention required by the security
model (cf. ``docs/explanation/security-model.md``).

Usage::

    python scripts/observability/export_audit_archive.py \\
        --loki-url http://loki:3100 \\
        --tenant tenant-acme \\
        --since 2026-04-01T00:00:00Z \\
        --until 2026-05-01T00:00:00Z \\
        --out /var/audit/2026-04-acme.ndjson.gz

Exit codes :

* 0 — N events exported, all valid.
* 1 — at least one event failed schema validation (export still
       completes ; check the report).
* 2 — usage / fetch error.
"""

from __future__ import annotations

import argparse
import gzip
import json
import sys
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path


def parse_iso(s: str) -> datetime:
    """Accept e.g. 2026-04-27T14:30:00Z or +00:00 ; return aware UTC."""
    s = s.replace("Z", "+00:00")
    dt = datetime.fromisoformat(s)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt.astimezone(timezone.utc)


def to_unix_ns(dt: datetime) -> int:
    return int(dt.timestamp() * 1_000_000_000)


def fetch_logs(
    loki_url: str,
    tenant: str,
    since: datetime,
    until: datetime,
    limit_per_chunk: int = 5000,
) -> list[dict]:
    """Pull `{stream="audit", tenant_id="<tenant>"}` from Loki.

    Loki's range API caps the result count per call (default 5000) ;
    we paginate via the `direction` + chunk windowing.
    """
    base = loki_url.rstrip("/")
    path = "/loki/api/v1/query_range"

    query = '{stream="audit", tenant_id="' + tenant + '"} | json'
    events: list[dict] = []

    cur_start = since
    while cur_start < until:
        params = {
            "query": query,
            "start": str(to_unix_ns(cur_start)),
            "end":   str(to_unix_ns(until)),
            "limit": str(limit_per_chunk),
            "direction": "forward",
        }
        url = f"{base}{path}?{urllib.parse.urlencode(params)}"
        try:
            with urllib.request.urlopen(url, timeout=30.0) as resp:
                body = json.loads(resp.read().decode("utf-8"))
        except (urllib.error.URLError, TimeoutError) as e:
            raise SystemExit(f"FATAL: Loki query failed: {e}") from e

        chunk: list[dict] = []
        for stream in body.get("data", {}).get("result", []):
            for ts, line in stream.get("values", []):
                try:
                    chunk.append(json.loads(line))
                except json.JSONDecodeError:
                    # Lines that aren't JSON are silently skipped.
                    pass
        events.extend(chunk)
        if len(chunk) < limit_per_chunk:
            break
        # Advance the start cursor past the last event ; Loki ts in ns.
        last_ts = chunk[-1].get("timestamp")
        if not last_ts:
            break
        try:
            cur_start = parse_iso(last_ts)
        except Exception:
            break

    return events


def validate_audit_event(ev: dict) -> str | None:
    """Best-effort schema validation without `jsonschema` dep.

    Returns None when the event passes, or an error message string.
    """
    required = ("schema_version", "stream", "timestamp", "level",
                "service", "service_version", "tenant_id", "principal",
                "action", "resource_type", "resource_key", "op_tag")
    for k in required:
        if k not in ev:
            return f"missing field {k!r}"
    if ev.get("stream") != "audit":
        return f"stream != 'audit' (got {ev.get('stream')!r})"
    if ev.get("action") not in ("register", "evolve", "delete"):
        return f"action {ev.get('action')!r} not in canonical set"
    if ev.get("op_tag") not in (
        "register", "append_class", "remove_class",
        "update_thresholds", "replace_class_anchors",
        "recompile", "delete",
    ):
        return f"op_tag {ev.get('op_tag')!r} not in canonical set"
    return None


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 2)[0])
    parser.add_argument("--loki-url", required=True)
    parser.add_argument("--tenant",   required=True)
    parser.add_argument("--since",    required=True,
                        help="ISO 8601 inclusive lower bound")
    parser.add_argument("--until",    required=True,
                        help="ISO 8601 exclusive upper bound")
    parser.add_argument("--out",      required=True,
                        help="Output NDJSON.gz path")
    parser.add_argument("--no-validate", action="store_true",
                        help="Skip schema validation")
    args = parser.parse_args(argv)

    since = parse_iso(args.since)
    until = parse_iso(args.until)
    if not since < until:
        print(f"FATAL: --since must be < --until", file=sys.stderr)
        return 2

    print(f"[1/3] Pulling audit events for {args.tenant!r} "
          f"in [{since.isoformat()}, {until.isoformat()}]…")
    events = fetch_logs(args.loki_url, args.tenant, since, until)
    print(f"      pulled {len(events)} events")

    invalid: list[tuple[int, str]] = []
    if not args.no_validate:
        print(f"[2/3] Validating against the D13 audit schema…")
        for i, ev in enumerate(events):
            err = validate_audit_event(ev)
            if err:
                invalid.append((i, err))
        print(f"      {len(invalid)} invalid events")

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    print(f"[3/3] Writing {out_path}…")
    with gzip.open(out_path, "wt", encoding="utf-8") as f:
        for ev in events:
            f.write(json.dumps(ev, separators=(",", ":")))
            f.write("\n")
    print(f"      ✓ wrote {len(events)} events ({out_path.stat().st_size} bytes gz)")

    if invalid:
        print(f"\n      WARNING : {len(invalid)} events failed validation",
              file=sys.stderr)
        for i, err in invalid[:10]:
            print(f"        [{i}] {err}", file=sys.stderr)
        if len(invalid) > 10:
            print(f"        ... ({len(invalid) - 10} more)", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
