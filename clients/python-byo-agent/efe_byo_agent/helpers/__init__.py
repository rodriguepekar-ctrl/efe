"""M20 helper modules — direct provider mode (M20).

Each helper has a class with a `run()` method that mirrors the
mcp-server tool handler exactly — same prompt template, same
validation, same output schema.

Usage :
    from efe_byo_agent import Agent
    from efe_byo_agent.helpers import HELPERS

    agent = Agent(...)
    result = await agent.helper("summarize_classification", **kwargs)
"""
from __future__ import annotations

from .analyze_confusion_matrix import AnalyzeConfusionMatrixHelper
from .compare_bundles_quality import CompareBundlesQualityHelper
from .draft_runbook import DraftRunbookHelper
from .generate_test_sentences import GenerateTestSentencesHelper
from .summarize_classification import SummarizeClassificationHelper


# Dispatch table : `Agent.helper(name, ...)` looks up here.
HELPERS = {
    "summarize_classification": SummarizeClassificationHelper,
    "compare_bundles_quality": CompareBundlesQualityHelper,
    "generate_test_sentences": GenerateTestSentencesHelper,
    "analyze_confusion_matrix": AnalyzeConfusionMatrixHelper,
    "draft_runbook": DraftRunbookHelper,
}


__all__ = [
    "HELPERS",
    "SummarizeClassificationHelper",
    "CompareBundlesQualityHelper",
    "GenerateTestSentencesHelper",
    "AnalyzeConfusionMatrixHelper",
    "DraftRunbookHelper",
]
