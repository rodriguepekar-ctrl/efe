"""Tests for runner, bench and scaffold — the high-level scenario APIs.

Runner and bench use :class:`MockAnthropic` + fake EfeClient + fake
catalogs to exercise the end-to-end orchestration without hitting any
real network.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from efe_tool.bridges.mock import (
    MockAnthropic,
    MockResponse,
    make_usage,
    text_block,
    tool_use_block,
)
from efe_tool.scenarios import (
    bench_scenario,
    load_scenario,
    run_scenario,
    scaffold_scenario,
    validate_name,
)
from efe_tool.scenarios.scaffold import _NAME_RE  # regression on the regex


# ---------------------------------------------------------------------------
#  run_scenario
# ---------------------------------------------------------------------------


class TestRunScenario:
    def test_happy_path(
        self, scenario_dir: Path, fake_catalog, fake_efe_client,
        mock_anthropic_tool_then_ack,
    ):
        scn = load_scenario(scenario_dir.name, search_paths=[scenario_dir.parent])
        result = run_scenario(
            scn,
            user="hi",
            anthropic_client=mock_anthropic_tool_then_ack,
            efe_client=fake_efe_client,
            catalogs={fake_catalog.name: fake_catalog},
        )
        assert result.scenario.name == scn.name
        assert result.conversation.stop_reason == "end_turn"
        assert result.conversation.final_text == "acknowledged"
        assert result.boot_config["scenario"] == scn.name
        assert result.ended_at >= result.started_at

    def test_no_catalogs_raises(
        self, scenario_dir: Path, fake_efe_client, mock_anthropic_tool_then_ack,
    ):
        scn = load_scenario(scenario_dir.name, search_paths=[scenario_dir.parent])
        # Override catalog_names (prompt declares support_triage) with an empty map.
        from efe_tool.bridges.errors import LlmAuthError
        with pytest.raises(LlmAuthError):
            run_scenario(
                scn,
                user="hi",
                anthropic_client=mock_anthropic_tool_then_ack,
                efe_client=fake_efe_client,
                catalogs={},  # empty!
            )


# ---------------------------------------------------------------------------
#  bench_scenario
# ---------------------------------------------------------------------------


class TestBenchScenario:
    def test_all_cases_run(
        self, scenario_dir: Path, fake_catalog, fake_efe_client,
    ):
        scn = load_scenario(scenario_dir.name, search_paths=[scenario_dir.parent])
        # We have 2 golden cases → need 2 × 2 = 4 responses.
        responses = []
        for _ in range(len(scn.golden)):
            responses += [
                MockResponse(
                    stop_reason="tool_use",
                    content=[tool_use_block(
                        "t", "efe_classify",
                        {"text": "hi", "catalog": "support_triage"},
                    )],
                    usage=make_usage(input_tokens=100, output_tokens=20),
                ),
                MockResponse(
                    stop_reason="end_turn",
                    content=[text_block("ack")],
                    usage=make_usage(input_tokens=150, output_tokens=30),
                ),
            ]
        mock = MockAnthropic(responses=responses)
        report = bench_scenario(
            scn,
            anthropic_client=mock,
            efe_client=fake_efe_client,
            catalogs={fake_catalog.name: fake_catalog},
        )
        assert len(report.cases) == len(scn.golden)
        # Our fake classifier returns label='alpha' always; the golden
        # cases expect label='alpha' → everything passes.
        assert report.passed == len(scn.golden)
        assert report.accuracy == 1.0
        assert report.total_cost_usd > 0

    def test_summary_line_shape(
        self, scenario_dir: Path, fake_catalog, fake_efe_client,
    ):
        scn = load_scenario(scenario_dir.name, search_paths=[scenario_dir.parent])
        mock = MockAnthropic(responses=[
            MockResponse(
                stop_reason="end_turn",
                content=[text_block("ack")],
                usage=make_usage(input_tokens=50, output_tokens=10),
            )
        ] * len(scn.golden))
        report = bench_scenario(
            scn, anthropic_client=mock,
            efe_client=fake_efe_client,
            catalogs={fake_catalog.name: fake_catalog},
        )
        line = report.summary_line()
        assert "/" in line
        assert "cost $" in line


# ---------------------------------------------------------------------------
#  scaffold_scenario
# ---------------------------------------------------------------------------


class TestScaffold:
    def test_creates_four_files(self, tmp_path: Path):
        files = scaffold_scenario("new_scn", target_dir=tmp_path)
        names = {f.name for f in files}
        assert names == {"__init__.py", "prompt.md", "golden.json", "README.md"}

    def test_template_valid_yaml_frontmatter(self, tmp_path: Path):
        files = scaffold_scenario("new_scn", target_dir=tmp_path,
                                  default_catalog="cat")
        prompt = next(f for f in files if f.name == "prompt.md").read_text(
            encoding="utf-8")
        assert "name: new_scn" in prompt
        assert "cat" in prompt

    def test_existing_nonempty_refuses(self, tmp_path: Path):
        target = tmp_path / "xx"
        target.mkdir()
        (target / "dummy").write_text("", encoding="utf-8")
        with pytest.raises(FileExistsError):
            scaffold_scenario("xx", target_dir=tmp_path)

    def test_force_overwrites(self, tmp_path: Path):
        scaffold_scenario("xx", target_dir=tmp_path)
        # Second call must succeed with force=True.
        scaffold_scenario("xx", target_dir=tmp_path, force=True)

    def test_invalid_name_rejected(self):
        with pytest.raises(ValueError):
            validate_name("Invalid-Name")
        with pytest.raises(ValueError):
            validate_name("9starts_with_digit")
        with pytest.raises(ValueError):
            validate_name("")

    def test_valid_names(self):
        for good in ("a1", "snake_case", "short", "a_very_long_ok_name"):
            validate_name(good)


class TestScaffoldedGoldenIsValidJson:
    def test_bootstrap_roundtrip(self, tmp_path: Path):
        scaffold_scenario("snake_scn", target_dir=tmp_path)
        golden = tmp_path / "snake_scn" / "golden.json"
        data = json.loads(golden.read_text(encoding="utf-8"))
        assert isinstance(data, list)
