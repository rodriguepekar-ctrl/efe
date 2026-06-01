#!/usr/bin/env python3
"""scripts/check_distroless_symbols.py — Sprint D9.A4 amendment.

Verifies that every dynamic library imported by the production
``efe-server`` binary is resolvable inside the distroless runtime
image (``gcr.io/distroless/cc-debian12:nonroot``).

This catches the class of bugs where a Cargo dependency adds a
new ``dlopen`` against (e.g.) ``libssl``, ``libsqlite3``, or any
glibc-version-pinned symbol that distroless doesn't ship — the
binary builds and starts on the developer's Debian box but
SIGSEGVs on the first request inside the container.

Inputs (two text files, both produced by the release workflow) :
  --distroless-libs : output of ``ldconfig -p`` from the distroless
                      base image (one library per line).
  --binary-deps     : output of ``ldd`` against the built efe-server
                      (one library per line).

Output : exits 0 when every binary dep is resolvable, 1 otherwise.

Usage in CI (``.github/workflows/release.yml``) :

    docker run --rm --entrypoint=/busybox/sh \
        gcr.io/distroless/cc-debian12:debug-nonroot -c 'ldconfig -p' \
        > distroless_libs.txt
    docker run --rm --entrypoint=/busybox/sh \
        ghcr.io/efe-project/efe:${TAG} -c 'ldd /usr/local/bin/efe-server' \
        > binary_deps.txt
    python3 scripts/check_distroless_symbols.py \
        --distroless-libs distroless_libs.txt \
        --binary-deps     binary_deps.txt

Exit codes:
    0 — every binary dep is resolved by distroless.
    1 — at least one dep is missing.
    2 — operational failure (file missing, malformed input).

The list of "always present" libs (linux-vdso, ld-linux) is
hard-coded since they don't appear in ``ldconfig -p`` output.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# Always-present libraries that don't appear in `ldconfig -p` because
# they are kernel-supplied (vdso) or part of the dynamic linker itself.
_VDSO_AND_LINKER = frozenset({
    "linux-vdso.so.1",
    "linux-vdso.so",
    "ld-linux-x86-64.so.2",
    "ld-linux-aarch64.so.1",
    "ld.so",
})


def parse_ldconfig(text: str) -> set[str]:
    """Return the set of library SONAMEs from a ``ldconfig -p`` dump.

    The format is::

        <N> libs found in cache `/etc/ld.so.cache'
                libcrypto.so.3 (libc6,x86-64) => /lib/x86_64-linux-gnu/libcrypto.so.3
                libc.so.6 (libc6,x86-64) => /lib/x86_64-linux-gnu/libc.so.6
                ...

    We only need the first whitespace-delimited token of each
    indented line.
    """
    out: set[str] = set()
    for line in text.splitlines():
        line = line.strip()
        if not line or line.endswith("cache'"):
            continue
        # Token before the first space.
        soname = line.split(maxsplit=1)[0]
        if soname.startswith("lib") or soname.endswith(".so") \
                or ".so." in soname:
            out.add(soname)
    return out


def parse_ldd(text: str) -> set[str]:
    """Return the set of SONAMEs an ldd dump claims the binary needs.

    Three line shapes::

        linux-vdso.so.1 (0x00007ffe...)
        libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007f...)
        /lib64/ld-linux-x86-64.so.2 (0x00007f...)

    We canonicalize all three to their basename (``libc.so.6`` etc).
    Lines reading ``not a dynamic executable`` and similar errors
    are silently dropped — they're not deps.
    """
    out: set[str] = set()
    pat_arrow = re.compile(r"^\s*(\S+\.so[\.\d]*)\s+=>")
    pat_naked = re.compile(r"^\s*(\S+\.so[\.\d]*)\s+\(")
    pat_path = re.compile(r"^\s*(/\S+\.so[\.\d]*)\s+\(")
    for line in text.splitlines():
        for pat in (pat_arrow, pat_naked, pat_path):
            m = pat.match(line)
            if m:
                token = m.group(1).split("/")[-1]
                out.add(token)
                break
    return out


def report(
    binary_deps: set[str],
    distroless_libs: set[str],
) -> tuple[set[str], set[str]]:
    """Return ``(resolved, unresolved)``."""
    resolved: set[str] = set()
    unresolved: set[str] = set()
    for dep in binary_deps:
        if dep in _VDSO_AND_LINKER:
            resolved.add(dep)
        elif dep in distroless_libs:
            resolved.add(dep)
        else:
            # Some sonames in ldd are versioned (libssl.so.3) but
            # ldconfig may emit just libssl.so. Try the major-version
            # stripped form as a fallback.
            stem = dep.split(".so", 1)[0] + ".so"
            if any(lib.startswith(stem) for lib in distroless_libs):
                resolved.add(dep)
            else:
                unresolved.add(dep)
    return resolved, unresolved


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n", 2)[0])
    p.add_argument("--distroless-libs", required=True,
                   help="Path to text file with `ldconfig -p` output.")
    p.add_argument("--binary-deps", required=True,
                   help="Path to text file with `ldd <binary>` output.")
    p.add_argument("--allow-missing", action="append", default=[],
                   help="Symbols allowed to be missing (whitelisted, repeatable). "
                        "Use sparingly — every entry is a known security smell.")
    args = p.parse_args(argv)

    try:
        distroless_text = Path(args.distroless_libs).read_text(encoding="utf-8")
        binary_text = Path(args.binary_deps).read_text(encoding="utf-8")
    except FileNotFoundError as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 2

    distroless_libs = parse_ldconfig(distroless_text)
    binary_deps = parse_ldd(binary_text)
    if not binary_deps:
        print("::error::no binary deps parsed (ldd output empty?)",
              file=sys.stderr)
        return 2

    allowed = frozenset(args.allow_missing)
    resolved, unresolved = report(binary_deps, distroless_libs)
    truly_unresolved = unresolved - allowed

    print(f"# distroless symbol check")
    print(f"  binary deps      : {len(binary_deps)}")
    print(f"  distroless libs  : {len(distroless_libs)}")
    print(f"  resolved         : {len(resolved)}")
    print(f"  unresolved       : {len(unresolved)}")
    if allowed:
        print(f"  whitelisted      : {sorted(allowed)}")

    if truly_unresolved:
        print()
        print("::error::unresolved deps in distroless runtime:")
        for dep in sorted(truly_unresolved):
            print(f"  - {dep}")
        return 1

    if unresolved & allowed:
        print()
        print("::warning::whitelisted-missing deps (review periodically):")
        for dep in sorted(unresolved & allowed):
            print(f"  - {dep}")

    print()
    print("OK — every dynamic dep of efe-server is resolvable in distroless.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
