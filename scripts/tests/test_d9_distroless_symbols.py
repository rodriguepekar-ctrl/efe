"""Tests for ``scripts/check_distroless_symbols.py`` — Sprint D9.A4."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest

_HERE = Path(__file__).resolve().parent
_ROOT = _HERE.parents[1]
_SCRIPT = _ROOT / "scripts" / "check_distroless_symbols.py"
_spec = importlib.util.spec_from_file_location("check_distroless_symbols", _SCRIPT)
assert _spec and _spec.loader
mod = importlib.util.module_from_spec(_spec)
sys.modules["check_distroless_symbols"] = mod
_spec.loader.exec_module(mod)


_LDCONFIG_SAMPLE = """100 libs found in cache `/etc/ld.so.cache'
        libssl.so.3 (libc6,x86-64) => /usr/lib/x86_64-linux-gnu/libssl.so.3
        libpthread.so.0 (libc6,x86-64) => /lib/x86_64-linux-gnu/libpthread.so.0
        libm.so.6 (libc6,x86-64) => /lib/x86_64-linux-gnu/libm.so.6
        libgcc_s.so.1 (libc6,x86-64) => /lib/x86_64-linux-gnu/libgcc_s.so.1
        libc.so.6 (libc6,x86-64) => /lib/x86_64-linux-gnu/libc.so.6
        libdl.so.2 (libc6,x86-64) => /lib/x86_64-linux-gnu/libdl.so.2
"""


_LDD_HAPPY = """    linux-vdso.so.1 (0x00007ffd...)
    libgcc_s.so.1 => /lib/x86_64-linux-gnu/libgcc_s.so.1 (0x00007f...)
    libpthread.so.0 => /lib/x86_64-linux-gnu/libpthread.so.0 (0x00007f...)
    libdl.so.2 => /lib/x86_64-linux-gnu/libdl.so.2 (0x00007f...)
    libm.so.6 => /lib/x86_64-linux-gnu/libm.so.6 (0x00007f...)
    libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007f...)
    /lib64/ld-linux-x86-64.so.2 (0x00007f...)
"""


_LDD_WITH_MISSING_LIB = _LDD_HAPPY + (
    "    libsqlite3.so.0 => /usr/lib/x86_64-linux-gnu/libsqlite3.so.0 "
    "(0x00007f...)\n"
)


def test_parse_ldconfig_extracts_sonames():
    libs = mod.parse_ldconfig(_LDCONFIG_SAMPLE)
    assert "libssl.so.3" in libs
    assert "libc.so.6" in libs
    assert len(libs) == 6


def test_parse_ldconfig_skips_header_line():
    libs = mod.parse_ldconfig(_LDCONFIG_SAMPLE)
    assert not any(soname.startswith("100") for soname in libs)


def test_parse_ldd_handles_three_line_shapes():
    deps = mod.parse_ldd(_LDD_HAPPY)
    # vdso, ld-linux, plus 5 normal deps
    assert "linux-vdso.so.1" in deps
    assert "libc.so.6" in deps
    assert "ld-linux-x86-64.so.2" in deps


def test_report_marks_vdso_as_resolved():
    deps = {"linux-vdso.so.1", "libc.so.6"}
    libs = {"libc.so.6"}
    resolved, unresolved = mod.report(deps, libs)
    assert "linux-vdso.so.1" in resolved
    assert not unresolved


def test_report_flags_unknown_library():
    deps = {"libfoo.so.1", "libc.so.6"}
    libs = {"libc.so.6"}
    resolved, unresolved = mod.report(deps, libs)
    assert unresolved == {"libfoo.so.1"}


def test_report_resolves_via_unversioned_stem():
    """`ldd` may emit `libssl.so.3` but `ldconfig -p` only sees
    `libssl.so` — the checker should still pass via stem fallback.
    """
    deps = {"libssl.so.3"}
    libs = {"libssl.so"}
    resolved, unresolved = mod.report(deps, libs)
    assert "libssl.so.3" in resolved
    assert not unresolved


def test_main_exit_zero_on_happy_path(tmp_path: Path):
    distroless = tmp_path / "distroless.txt"
    binary = tmp_path / "binary.txt"
    distroless.write_text(_LDCONFIG_SAMPLE)
    binary.write_text(_LDD_HAPPY)
    rc = mod.main([
        "--distroless-libs", str(distroless),
        "--binary-deps", str(binary),
    ])
    assert rc == 0


def test_main_exit_one_when_missing_dep(tmp_path: Path):
    distroless = tmp_path / "distroless.txt"
    binary = tmp_path / "binary.txt"
    distroless.write_text(_LDCONFIG_SAMPLE)
    binary.write_text(_LDD_WITH_MISSING_LIB)
    rc = mod.main([
        "--distroless-libs", str(distroless),
        "--binary-deps", str(binary),
    ])
    assert rc == 1


def test_main_allow_missing_whitelists(tmp_path: Path):
    distroless = tmp_path / "distroless.txt"
    binary = tmp_path / "binary.txt"
    distroless.write_text(_LDCONFIG_SAMPLE)
    binary.write_text(_LDD_WITH_MISSING_LIB)
    rc = mod.main([
        "--distroless-libs", str(distroless),
        "--binary-deps", str(binary),
        "--allow-missing", "libsqlite3.so.0",
    ])
    assert rc == 0


def test_main_returns_two_when_input_missing(tmp_path: Path):
    rc = mod.main([
        "--distroless-libs", str(tmp_path / "nope.txt"),
        "--binary-deps", str(tmp_path / "also-nope.txt"),
    ])
    assert rc == 2
