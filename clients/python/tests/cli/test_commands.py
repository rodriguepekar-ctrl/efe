"""Unit tests for the subcommand implementations that do not require
a running server or a downloaded embedder: ``validate``, ``version``,
``completions``.

The network-dependent subcommands (``classify``, ``healthcheck``,
``compile-catalog``, ``eval``) are covered in
``tests/cli/test_main_integration.py`` via subprocess invocations.
"""

from __future__ import annotations

import io
import json
import sys
from pathlib import Path

import pytest

from efe_tool.cli.errors import CatalogInvalidError, CatalogNotFoundError
from efe_tool.cli.main import main


class TestVersion:
    def test_json(self, capsys):
        rc = main(["version", "--format", "json"])
        assert rc == 0
        out, _err = capsys.readouterr()
        doc = json.loads(out)
        assert doc["program"] == "efe-admin"
        assert "version" in doc
        assert doc["schema_version"] >= 1

    def test_text(self, capsys):
        rc = main(["version", "--format", "text"])
        assert rc == 0
        out, _err = capsys.readouterr()
        assert "version" in out.lower()


class TestCompletions:
    def test_bash(self, capsys):
        rc = main(["completions", "bash"])
        assert rc == 0
        out, _err = capsys.readouterr()
        assert "_efe_admin_completions" in out
        assert "classify" in out

    def test_zsh(self, capsys):
        rc = main(["completions", "zsh"])
        assert rc == 0
        out, _err = capsys.readouterr()
        assert "#compdef efe-admin" in out

    def test_fish(self, capsys):
        rc = main(["completions", "fish"])
        assert rc == 0
        out, _err = capsys.readouterr()
        assert "__fish_use_subcommand" in out


class TestValidate:
    def test_valid_catalog(self, valid_yaml: Path, capsys):
        rc = main(["validate", str(valid_yaml)])
        assert rc == 0
        out, _err = capsys.readouterr()
        doc = json.loads(out)
        assert doc["valid"] is True
        assert doc["issues"] == []

    def test_missing_file_exits_3(self, tmp_path: Path, capsys):
        rc = main(["validate", str(tmp_path / "nope.yaml")])
        assert rc == 3

    def test_invalid_schema_exits_4(self, invalid_yaml: Path, capsys):
        rc = main(["validate", str(invalid_yaml)])
        assert rc == 4

    def test_missing_top_level_exits_4(
        self, missing_version_yaml: Path, capsys,
    ):
        rc = main(["validate", str(missing_version_yaml)])
        assert rc == 4

    def test_empty_yaml_exits_4(self, tmp_path: Path, capsys):
        """An empty YAML file yields a clean EMPTY_DOCUMENT issue."""
        p = tmp_path / "empty.yaml"
        p.write_text("", encoding="utf-8")
        rc = main(["validate", str(p)])
        assert rc == 4
        out, _err = capsys.readouterr()
        doc = json.loads(out)
        codes = {i["code"] for i in doc["issues"]}
        assert "EMPTY_DOCUMENT" in codes

    def test_comments_only_yaml_exits_4(self, tmp_path: Path, capsys):
        p = tmp_path / "comments.yaml"
        p.write_text("# just a comment\n# no content\n", encoding="utf-8")
        rc = main(["validate", str(p)])
        assert rc == 4
        out, _err = capsys.readouterr()
        doc = json.loads(out)
        codes = {i["code"] for i in doc["issues"]}
        assert "EMPTY_DOCUMENT" in codes

    def test_list_root_yaml_exits_4(self, tmp_path: Path, capsys):
        """A YAML document whose root is a list (not a mapping) must
        be rejected cleanly with exit 4, not crash on dict.get."""
        p = tmp_path / "list.yaml"
        p.write_text("- one\n- two\n", encoding="utf-8")
        rc = main(["validate", str(p)])
        assert rc == 4
        out, _err = capsys.readouterr()
        doc = json.loads(out)
        codes = {i["code"] for i in doc["issues"]}
        assert "NOT_A_MAPPING" in codes

    def test_scalar_root_yaml_exits_4(self, tmp_path: Path, capsys):
        p = tmp_path / "scalar.yaml"
        p.write_text("42\n", encoding="utf-8")
        rc = main(["validate", str(p)])
        assert rc == 4
        out, _err = capsys.readouterr()
        doc = json.loads(out)
        codes = {i["code"] for i in doc["issues"]}
        assert "NOT_A_MAPPING" in codes

    def test_malformed_yaml_exits_4(self, tmp_path: Path, capsys):
        """Syntactically broken YAML must yield exit 4 with a clear error."""
        p = tmp_path / "broken.yaml"
        p.write_text("key: [unclosed list\n", encoding="utf-8")
        rc = main(["validate", str(p)])
        assert rc == 4


class TestClassifyEarlyExits:
    """Paths in classify that exit before any gRPC or embedder is touched.

    These tests verify that the exit-code discipline holds end-to-end
    for scenarios the CLI must resolve on its own, without depending
    on a running server.
    """

    def test_no_catalog_exits_2(self, capsys):
        # --catalog is required and has no env fallback configured in
        # this test (the autouse fixture scrubs EFE_*).
        rc = main(["classify", "--text", "hi"])
        assert rc == 2

    def test_catalog_not_found_exits_3(self, tmp_path: Path, capsys):
        rc = main(["classify", "--text", "hi",
                   "--catalog", str(tmp_path / "nope.yaml")])
        assert rc == 3

    def test_conflicting_text_sources_exits_2(
        self, tmp_path: Path, capsys,
    ):
        rc = main(["classify", "--text", "a", "--stdin",
                   "--catalog", str(tmp_path / "nope.yaml")])
        assert rc == 2


class TestGlobalDispatch:
    def test_no_subcommand_exits_2(self, capsys):
        rc = main([])
        assert rc == 2

    def test_unknown_subcommand_exits_2(self, capsys):
        rc = main(["not-a-cmd"])
        assert rc == 2
