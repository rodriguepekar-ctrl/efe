#!/usr/bin/env python3
"""
Sprint Φ-D-OPS (D97) — Alert router.

Reads a payload from Alertmanager webhook (= JSON), looks up the
matching service in `deploy/pagerduty/services.yaml`, and emits the
PagerDuty Events API v2 payload. Designed to run as an Alertmanager
`webhook` receiver.

Supports a `--dry-run` mode for CI golden tests.

Cf. `docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 10.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.request
from pathlib import Path
from typing import Any, Dict, List, Optional

try:
    import yaml  # type: ignore
except ImportError:
    yaml = None  # type: ignore


REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONFIG = REPO_ROOT / "deploy" / "pagerduty" / "services.yaml"


def _load_config(path: Path) -> Dict[str, Any]:
    if yaml is None:
        sys.exit("missing pyyaml")
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def _match_service(alert_labels: Dict[str, str],
                   routing_rules: List[Dict[str, Any]]) -> Optional[str]:
    for rule in routing_rules:
        match = rule.get("match", {})
        if all(alert_labels.get(k) == v for k, v in match.items()):
            return rule.get("service")
    return None


def _to_pagerduty_event(alert: Dict[str, Any], service: str,
                        routing_key: str) -> Dict[str, Any]:
    labels = alert.get("labels", {})
    annotations = alert.get("annotations", {})
    severity = labels.get("severity", "warning")
    pd_severity = {
        "critical": "critical",
        "major":    "error",
        "watch":    "warning",
        "ticket":   "info",
    }.get(severity, "warning")

    return {
        "routing_key": routing_key,
        "event_action": "trigger" if alert.get("status") == "firing" else "resolve",
        "dedup_key": alert.get("fingerprint", labels.get("alertname")),
        "payload": {
            "summary": annotations.get(
                "summary",
                f"{labels.get('alertname', 'alert')} on {labels.get('region', 'unknown')}",
            ),
            "severity": pd_severity,
            "source": labels.get("region", labels.get("instance", "efe")),
            "component": labels.get("ops_master_slo", "unknown"),
            "group": service,
            "class": labels.get("alertname", "alert"),
            "custom_details": {
                "runbook": annotations.get("runbook"),
                "dashboard": annotations.get("dashboard"),
                "labels": labels,
            },
        },
        "links": [
            {"href": annotations.get("runbook", ""),
             "text": "Runbook"},
            {"href": annotations.get("dashboard", ""),
             "text": "Dashboard"},
        ],
    }


def _routing_key_for(service: str) -> str:
    env_name = f"PD_ROUTING_KEY_{service.upper().replace('-', '_')}"
    return os.environ.get(env_name, "")


def _send_event(payload: Dict[str, Any]) -> None:
    req = urllib.request.Request(
        url="https://events.pagerduty.com/v2/enqueue",
        method="POST",
        headers={"Content-Type": "application/json"},
        data=json.dumps(payload).encode("utf-8"),
    )
    with urllib.request.urlopen(req, timeout=10) as resp:
        _ = resp.read()


def main(argv: Optional[List[str]] = None) -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--alertmanager-json", type=Path,
                   help="Path to Alertmanager webhook payload.")
    p.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    p.add_argument("--dry-run", action="store_true",
                   help="Print events to stdout instead of POSTing.")
    args = p.parse_args(argv)

    if not args.alertmanager_json:
        # Read from stdin (= Alertmanager piping).
        payload = json.loads(sys.stdin.read())
    else:
        payload = json.loads(args.alertmanager_json.read_text(encoding="utf-8"))

    cfg = _load_config(args.config)
    routing_rules = cfg.get("routing_rules", [])

    events: List[Dict[str, Any]] = []
    for alert in payload.get("alerts", []):
        service = _match_service(alert.get("labels", {}), routing_rules)
        if not service:
            print(f"  no service match for alert {alert.get('labels')}",
                  file=sys.stderr)
            continue
        key = _routing_key_for(service)
        if not key and not args.dry_run:
            print(f"  missing routing key env for service {service}",
                  file=sys.stderr)
            continue
        event = _to_pagerduty_event(alert, service, key or "DRY-RUN-KEY")
        events.append(event)

    if args.dry_run:
        print(json.dumps(events, indent=2))
        return 0
    for ev in events:
        _send_event(ev)
    print(f"sent {len(events)} event(s) to PagerDuty")
    return 0


if __name__ == "__main__":
    sys.exit(main())
