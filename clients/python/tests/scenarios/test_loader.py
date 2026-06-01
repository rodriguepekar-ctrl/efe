"""Tests for :mod:`efe_tool.scenarios.loader`."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from efe_tool.scenarios import (
    GoldenCase,
    Prompt,
    Scenario,
    list_builtin_scenarios,
    load_golden,
    load_prompt,
    load_scenario,
    scenario_summary,
)


class TestLoadPrompt:
    def test_with_frontmatter(self, tmp_path: Path):
        p = tmp_path / "prompt.md"
        p.write_text(
            "---\nname: scn\nversion: 1.2.3\nmodel: claude-sonnet-4-6\n"
            "catalogs: [a, b]\n---\n# Role\nBody text.\n",
            encoding="utf-8",
        )
        prompt = load_prompt(p)
        assert prompt.name == "scn"
        assert prompt.version == "1.2.3"
        assert prompt.catalog_names == ("a", "b")
        assert "Body text" in prompt.text
        assert prompt.hash.startswith("sha256:")

    def test_no_frontmatter(self, tmp_path: Path):
        p = tmp_path / "prompt.md"
        p.write_text("Just text.", encoding="utf-8")
        prompt = load_prompt(p)
        assert prompt.metadata == {}
        assert prompt.text == "Just text."

    def test_from_string(self):
        prompt = load_prompt("---\nname: inline\n---\nhello", is_text=True)
        assert prompt.name == "inline"
        assert prompt.source_path is None

    def test_hash_stable_across_metadata_edits(self, tmp_path: Path):
        """Changing only metadata (e.g. version) must not shift the body hash."""
        body = "# Role\nThe body.\n"
        p1 = tmp_path / "a.md"
        p1.write_text(f"---\nname: x\nversion: 1.0.0\n---\n{body}", encoding="utf-8")
        p2 = tmp_path / "b.md"
        p2.write_text(f"---\nname: x\nversion: 2.0.0\n---\n{body}", encoding="utf-8")
        assert load_prompt(p1).hash == load_prompt(p2).hash

    def test_malformed_frontmatter_scalar_raises(self, tmp_path: Path):
        p = tmp_path / "bad.md"
        p.write_text("---\n42\n---\nbody", encoding="utf-8")
        with pytest.raises(ValueError):
            load_prompt(p)


class TestLoadGolden:
    def test_parses_list_of_objects(self, tmp_path: Path):
        p = tmp_path / "g.json"
        p.write_text(json.dumps([
            {"user": "hi", "expected_label": "alpha"},
            {"user": "bye"},
        ]), encoding="utf-8")
        cases = load_golden(p)
        assert len(cases) == 2
        assert cases[0].user == "hi"
        assert cases[0].expected_label == "alpha"
        assert cases[1].expected_label is None

    def test_non_list_raises(self, tmp_path: Path):
        p = tmp_path / "g.json"
        p.write_text('{"user": "only one"}', encoding="utf-8")
        with pytest.raises(ValueError):
            load_golden(p)

    def test_case_without_user_raises(self, tmp_path: Path):
        p = tmp_path / "g.json"
        p.write_text(json.dumps([{"expected_label": "x"}]), encoding="utf-8")
        with pytest.raises(ValueError):
            load_golden(p)

    def test_empty_user_raises(self, tmp_path: Path):
        p = tmp_path / "g.json"
        p.write_text(json.dumps([{"user": "   "}]), encoding="utf-8")
        with pytest.raises(ValueError):
            load_golden(p)


class TestLoadScenario:
    def test_from_search_path(self, scenario_dir: Path):
        scn = load_scenario(scenario_dir.name,
                            search_paths=[scenario_dir.parent])
        assert isinstance(scn, Scenario)
        assert scn.name == "my_scenario"
        assert len(scn.golden) == 2
        assert scn.readme.startswith("# My Scenario")

    def test_missing_raises(self):
        with pytest.raises(FileNotFoundError):
            load_scenario("does_not_exist")

    def test_builtin_found(self):
        scn = load_scenario("support_triage_bilingual")
        assert scn.name == "support_triage_bilingual"
        assert scn.prompt.model == "claude-sonnet-4-6"


class TestListBuiltin:
    def test_contains_three(self):
        names = list_builtin_scenarios()
        assert "support_triage_bilingual" in names
        assert "billing_escalation" in names
        assert "fraud_alert" in names


class TestScenarioSummary:
    def test_has_all_fields(self, scenario_dir: Path):
        scn = load_scenario(scenario_dir.name,
                            search_paths=[scenario_dir.parent])
        summary = scenario_summary(scn)
        assert summary["name"] == "my_scenario"
        assert summary["version"] == "1.0.0"
        assert summary["golden_cases"] == 2
        assert summary["prompt_hash"].startswith("sha256:")
