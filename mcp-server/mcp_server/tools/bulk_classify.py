"""efe_bulk_classify (M17 D46) — streaming."""
from __future__ import annotations

from collections import Counter
from typing import Any

from .types import Tool, ToolContext


INPUT_SCHEMA: dict[str, Any] = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "title": "EfeBulkClassifyInput",
    "description": "Classify N texts in a single call ; streams progress.",
    "properties": {
        "catalog": {"type": "string", "minLength": 1, "maxLength": 100},
        "texts": {
            "type": "array",
            "minItems": 1,
            "maxItems": 1000,
            "items": {"type": "string", "minLength": 1, "maxLength": 8192},
        },
        "top_k": {"type": "integer", "minimum": 1, "maximum": 32, "default": 1},
        "include_explanations": {"type": "boolean", "default": False},
    },
    "required": ["catalog", "texts"],
    "additionalProperties": False,
}


_BATCH_SIZE = 50


async def handler(args: dict[str, Any], ctx: ToolContext) -> dict[str, Any]:
    catalog = args["catalog"]
    texts = args["texts"]
    top_k = int(args.get("top_k", 1))
    include_expl = bool(args.get("include_explanations", False))

    # Try the EFE 1.1 bulk endpoint first ; fall back to per-item classify
    # if the backend doesn't expose it.
    try:
        return await ctx.backend.bulk_classify(
            catalog=catalog,
            texts=texts,
            top_k=top_k,
            include_explanations=include_expl,
            auth=ctx.auth,
            progress=ctx.progress,
        )
    except AttributeError:
        pass  # backend doesn't have bulk_classify ; fallback below
    except NotImplementedError:
        pass
    except Exception as exc:  # noqa: BLE001
        # Handle transparent fallbacks when backend returns 404 / not_supported
        if "not_supported" not in str(exc) and "not found" not in str(exc):
            raise

    # Fallback : per-item classify with progress
    results: list[dict[str, Any]] = []
    by_class: Counter[str] = Counter()
    below_threshold = 0
    ambiguous = 0
    catalog_version = None
    n = len(texts)
    for batch_start in range(0, n, _BATCH_SIZE):
        if ctx.progress:
            ctx.progress.check_cancelled()
            await ctx.progress.emit(
                progress=batch_start, total=n,
                message=f"Classified {batch_start}/{n}",
            )
        batch = texts[batch_start: batch_start + _BATCH_SIZE]
        for offset, t in enumerate(batch):
            r = await ctx.backend.classify(
                catalog=catalog, text=t, top_k=top_k, auth=ctx.auth,
            )
            top = r["top_k"][0] if r.get("top_k") else None
            entry: dict[str, Any] = {
                "index": batch_start + offset,
                "text_preview": t[:80],
                "predicted_class": top["class_name"] if top else None,
                "score": top["score"] if top else None,
                "above_threshold": top.get("above_threshold") if top else False,
            }
            if include_expl:
                entry["top_k"] = r.get("top_k", [])[:top_k]
            results.append(entry)
            if top:
                by_class[top["class_name"]] += 1
                if not top.get("above_threshold"):
                    below_threshold += 1
            if r.get("top_k") and len(r["top_k"]) >= 2:
                gap = r["top_k"][0]["score"] - r["top_k"][1]["score"]
                if gap < 0.05:
                    ambiguous += 1
            catalog_version = r.get("catalog_version") or catalog_version

    if ctx.progress:
        await ctx.progress.emit(progress=n, total=n, message="Bulk classify done")

    return {
        "catalog": catalog,
        "version": catalog_version,
        "results": results,
        "summary": {
            "total": n,
            "by_class": dict(by_class),
            "below_threshold_count": below_threshold,
            "ambiguous_count": ambiguous,
        },
    }


TOOL = Tool(
    name="efe_bulk_classify",
    description=(
        "Classify up to 1000 texts in a single call. Streams progress "
        "every 50 items. Returns per-item predictions + a summary of "
        "class distribution."
    ),
    input_schema=INPUT_SCHEMA,
    handler=handler,
    streams_progress=True,
    profile_tier="agent-native",
)
