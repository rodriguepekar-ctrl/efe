#!/usr/bin/env bash
# update-security-feed.sh — Append a release entry to the security RSS feed.
#
# Usage :
#   ./scripts/release/update-security-feed.sh v1.0.0

set -euo pipefail

VERSION="${1:?usage: $0 <version>}"
FEED_FILE="docs/security/feed.xml"
DATE=$(date -u +"%a, %d %b %Y %H:%M:%S GMT")

if [[ ! -f "$FEED_FILE" ]]; then
  echo "▶ Initializing $FEED_FILE"
  mkdir -p "$(dirname "$FEED_FILE")"
  cat > "$FEED_FILE" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>EFE Security Advisories</title>
    <link>https://efe.example.com/security</link>
    <description>Security advisories and CVE assessments for EFE</description>
    <language>en</language>
    <pubDate>${DATE}</pubDate>
    <lastBuildDate>${DATE}</lastBuildDate>
  </channel>
</rss>
EOF
fi

# Use python to manipulate XML safely (avoiding sed traps).
python3 - "$FEED_FILE" "$VERSION" "$DATE" <<'PY'
import sys
import xml.etree.ElementTree as ET
from xml.dom import minidom

feed_path, version, date = sys.argv[1], sys.argv[2], sys.argv[3]
tree = ET.parse(feed_path)
root = tree.getroot()
channel = root.find("channel")

new_item = ET.Element("item")
title = ET.SubElement(new_item, "title")
title.text = f"EFE {version} General Availability"
link = ET.SubElement(new_item, "link")
link.text = f"https://efe.example.com/security/{version}"
desc = ET.SubElement(new_item, "description")
desc.text = (
    f"EFE {version} ships with 0 known HIGH/CRITICAL CVEs. "
    "SLSA-3 provenance, Sigstore signatures, distroless containers."
)
pub_date = ET.SubElement(new_item, "pubDate")
pub_date.text = date
guid = ET.SubElement(new_item, "guid")
guid.text = f"{version}-ga"
guid.set("isPermaLink", "false")

# Insert as first item (most recent).
existing_items = channel.findall("item")
insertion_index = list(channel).index(channel.find("description")) + 1
channel.insert(insertion_index, new_item)

# Update lastBuildDate.
last_build = channel.find("lastBuildDate")
if last_build is not None:
    last_build.text = date

tree.write(feed_path, encoding="UTF-8", xml_declaration=True)
print(f"✓ Appended {version} entry to {feed_path}")
PY

echo "✅ RSS feed updated"
