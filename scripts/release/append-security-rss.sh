#!/usr/bin/env bash
# append-security-rss.sh — Append a security event to the RSS feed.
#
# Usage :
#   ./scripts/release/append-security-rss.sh v1.0.1 yank "fix breaking schema"
#   ./scripts/release/append-security-rss.sh v1.0.0 ga "initial baseline"

set -euo pipefail

VERSION="${1:?usage: $0 <version> <event:yank|ga|cve|fix> <reason>}"
EVENT="${2:?usage: $0 <version> <event> <reason>}"
REASON="${3:?usage: $0 <version> <event> <reason>}"

FEED_FILE="docs/security/feed.xml"
DATE=$(date -u +"%a, %d %b %Y %H:%M:%S GMT")

if [[ ! -f "$FEED_FILE" ]]; then
  ./scripts/release/update-security-feed.sh "$VERSION"
fi

python3 - "$FEED_FILE" "$VERSION" "$EVENT" "$REASON" "$DATE" <<'PY'
import sys
import xml.etree.ElementTree as ET

feed, version, event, reason, date = sys.argv[1:6]
titles = {
    "yank":  f"⚠ EFE {version} yanked",
    "ga":    f"EFE {version} GA security baseline",
    "cve":   f"EFE {version} CVE assessment",
    "fix":   f"EFE {version} security fix",
}
title = titles.get(event, f"EFE {version} security event ({event})")

tree = ET.parse(feed)
root = tree.getroot()
channel = root.find("channel")

new_item = ET.Element("item")
ET.SubElement(new_item, "title").text = title
ET.SubElement(new_item, "link").text = f"https://efe.example.com/security/{version}"
ET.SubElement(new_item, "description").text = reason
ET.SubElement(new_item, "pubDate").text = date
guid = ET.SubElement(new_item, "guid", {"isPermaLink": "false"})
guid.text = f"{version}-{event}"

# Insert at top (after the description element).
desc_idx = list(channel).index(channel.find("description"))
channel.insert(desc_idx + 1, new_item)

last_build = channel.find("lastBuildDate")
if last_build is not None:
    last_build.text = date

tree.write(feed, encoding="UTF-8", xml_declaration=True)
print(f"✓ Appended {event} entry for {version}")
PY
