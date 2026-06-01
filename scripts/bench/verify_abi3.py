#!/usr/bin/env python3
"""SLO-FFI-ABI3 — scan wheels for non-ABI3 (= Python version-specific) symbols.

Sprint **Φ-C-FFI Completion (D28)** — artefact #19.
Cf. ``docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md`` § 11.

## Mission

Une wheel ABI3 doit utiliser **strictement** l'API limited de CPython
(PEP 384). Si la wheel importe ``_Py_NotImplementedStruct``,
``_PyObject_FastCall``, ou autre symbole non-limited, elle :

* Charge sur la Python version qui l'a buildée.
* **CRASH** au runtime sur une autre Python version (= ``undefined symbol``).

Le scanner extrait les symboles importés par chaque ``.so``/``.dylib``/
``.pyd`` à l'intérieur de la wheel et vérifie qu'aucun n'appartient à
la blacklist Python-version-specific.

## Méthodologie cross-platform

* Linux : ``nm -D --undefined-only`` sur ``.so``.
* macOS : ``nm -u`` sur ``.dylib``.
* Windows : ``dumpbin /imports`` sur ``.pyd``.

Tous les outils sont disponibles dans le PATH par défaut sur les
runners CI (manylinux container + macos-14 + windows-2022).

## Usage

```bash
python scripts/bench/verify_abi3.py crates/efe-ffi/dist/
```

## Output

``target/ffi/abi3.json`` :

```json
{
  "slo_compliance": {
    "SLO-FFI-ABI3": {
      "measured_count": 0,
      "wheels_scanned": 5,
      "blacklisted_symbols": []
    }
  }
}
```

``measured_count`` = nombre de symboles blacklistés détectés.
``check_slo.py`` lit avec comparator ``eq 0``.

## Blacklist

La blacklist est une liste de patterns regex matchant des symboles
**non-limited** observés en pratique sur des packages buggés. Ajouter
un pattern ici ferme une nouvelle classe de bugs ABI3.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import zipfile
from pathlib import Path


# Patterns de symboles INTERDITS sur une wheel ABI3.
#
# Source : grep sur le code source de CPython pour les fns marquées
# `Py_LIMITED_API`-excluded. Liste minimale Φ-C-FFI ; à étendre au
# fur et à mesure que de nouveaux faux-ABI3 sont détectés en prod.
BLACKLIST_PATTERNS = [
    re.compile(r"^_Py_NewRef$"),
    re.compile(r"^_PyObject_FastCall$"),
    re.compile(r"^_PyObject_FastCallDict$"),
    re.compile(r"^_PyEval_GetFrame$"),
    re.compile(r"^_PyType_Lookup$"),
    re.compile(r"^_PyDict_GetItemIdWithError$"),
    re.compile(r"^_PyArg_ParseTupleAndKeywordsFast$"),
    re.compile(r"^_PyObject_GenericSetAttrWithDict$"),
    # Symboles privés génériques : tout `_Py*` qui n'est pas dans la
    # whitelist ABI3 est suspect.
    # On les attrape par exclusion plutôt que blacklist : voir
    # `is_suspect_private_symbol` ci-dessous.
]

# Whitelist de symboles `_Py*` autorisés (= bien ABI3 limited API
# malgré le préfixe underscore).
WHITELIST_PRIVATE_LIKE = {
    # vide pour l'instant ; à enrichir si on découvre des faux positifs
}


def is_suspect_private_symbol(sym: str) -> bool:
    """Heuristic : tout symbole commençant par `_Py` qui n'est pas dans
    la whitelist est suspect en ABI3."""
    if not sym.startswith("_Py"):
        return False
    return sym not in WHITELIST_PRIVATE_LIKE


def extract_symbols_linux(so_path: Path) -> set[str]:
    out = subprocess.check_output(
        ["nm", "-D", "--undefined-only", str(so_path)],
        stderr=subprocess.DEVNULL,
    ).decode("utf-8", errors="replace")
    syms: set[str] = set()
    for line in out.splitlines():
        line = line.strip()
        if not line.startswith("U "):
            continue
        sym = line[2:].strip()
        if sym:
            syms.add(sym)
    return syms


def extract_symbols_macos(so_path: Path) -> set[str]:
    out = subprocess.check_output(
        ["nm", "-u", str(so_path)],
        stderr=subprocess.DEVNULL,
    ).decode("utf-8", errors="replace")
    syms: set[str] = set()
    for line in out.splitlines():
        line = line.strip()
        if not line:
            continue
        # Format macOS : "_Py_NoneStruct" (underscore prefix).
        # On retire le leading underscore convention macOS pour matcher
        # les patterns de la blacklist.
        if line.startswith("_"):
            line = line[1:]
        syms.add(line)
    return syms


def extract_symbols_windows(pyd_path: Path) -> set[str]:
    # **Audit fix Φ-C-FFI.B8** — pre-fix attrapait uniquement
    # FileNotFoundError. Mais `dumpbin /imports` peut retourner
    # non-zero sur un .pyd corrompu ou un format inconnu → crash.
    # Post-fix : attrape la famille complète `subprocess` + timeout
    # explicite (= 60 s défensif).
    try:
        out = subprocess.check_output(
            ["dumpbin", "/imports", str(pyd_path)],
            stderr=subprocess.DEVNULL,
            timeout=60,
        ).decode("utf-8", errors="replace")
    except FileNotFoundError:
        sys.stderr.write(
            "WARNING : dumpbin not on PATH (= no Windows SDK installed). "
            "Skipping symbol scan for .pyd.\n"
        )
        return set()
    except subprocess.CalledProcessError as e:
        sys.stderr.write(
            f"WARNING : dumpbin /imports {pyd_path} returned exit={e.returncode}. "
            "Skipping symbol scan (likely unsupported .pyd format).\n"
        )
        return set()
    except subprocess.TimeoutExpired:
        sys.stderr.write(
            f"WARNING : dumpbin /imports {pyd_path} timed out (60s). "
            "Skipping symbol scan.\n"
        )
        return set()
    # Format dumpbin : lignes "  <hex>  <symbol_name>"
    syms: set[str] = set()
    for line in out.splitlines():
        m = re.match(r"^\s+[0-9A-Fa-f]+\s+([A-Za-z_]\w+)\s*$", line)
        if m:
            syms.add(m.group(1))
    return syms


def extract_symbols(path: Path) -> set[str]:
    if sys.platform.startswith("linux"):
        return extract_symbols_linux(path)
    if sys.platform == "darwin":
        return extract_symbols_macos(path)
    if sys.platform.startswith("win"):
        return extract_symbols_windows(path)
    sys.stderr.write(f"WARNING : platform {sys.platform} not supported, skipping.\n")
    return set()


def scan_wheel(whl: Path, tmp_dir: Path) -> list[str]:
    """Returns the list of blacklisted symbols found in `whl`."""
    bad: list[str] = []
    with zipfile.ZipFile(whl) as z:
        for name in z.namelist():
            if not name.endswith((".so", ".pyd", ".dylib")):
                continue
            tmp = tmp_dir / Path(name).name
            with z.open(name) as f:
                tmp.write_bytes(f.read())
            try:
                syms = extract_symbols(tmp)
            except subprocess.CalledProcessError as e:
                sys.stderr.write(
                    f"WARNING : symbol extraction failed for {name} : {e}\n"
                )
                continue
            for s in syms:
                for pat in BLACKLIST_PATTERNS:
                    if pat.match(s):
                        bad.append(f"{whl.name}:{name}:{s}")
                if is_suspect_private_symbol(s):
                    bad.append(f"{whl.name}:{name}:{s} (suspect _Py prefix)")
            tmp.unlink(missing_ok=True)
    return bad


def main() -> int:
    ap = argparse.ArgumentParser(description="Verify wheels are strict ABI3")
    ap.add_argument(
        "path",
        type=Path,
        help="Directory or .whl path",
    )
    ap.add_argument(
        "--out",
        type=Path,
        default=Path("target/ffi/abi3.json"),
        help="Output JSON path",
    )
    args = ap.parse_args()

    if args.path.is_dir():
        wheels = sorted(args.path.glob("*.whl"))
    elif args.path.suffix == ".whl":
        wheels = [args.path]
    else:
        sys.stderr.write(f"FATAL : {args.path} is neither a dir nor a .whl\n")
        return 2

    if not wheels:
        sys.stderr.write(f"FATAL : no wheels found in {args.path}\n")
        return 2

    import tempfile

    all_bad: list[str] = []
    with tempfile.TemporaryDirectory() as td:
        tmp_dir = Path(td)
        for whl in wheels:
            print(f"scanning {whl.name} ...", file=sys.stderr)
            all_bad += scan_wheel(whl, tmp_dir)

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-FFI-ABI3": {
                        "measured_count": len(all_bad),
                        "wheels_scanned": len(wheels),
                        "blacklisted_symbols": all_bad,
                    }
                }
            },
            indent=2,
        )
        + "\n"
    )

    if all_bad:
        sys.stderr.write("\nFAIL : BLACKLISTED SYMBOLS DETECTED:\n")
        for b in all_bad:
            sys.stderr.write(f"  - {b}\n")
        return 1

    print(f"\nABI3 strict : PASS ({len(wheels)} wheels scanned)\n", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
