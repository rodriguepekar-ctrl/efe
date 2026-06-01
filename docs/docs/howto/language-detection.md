# How-to — Multilingual classification

EFE's bundled embedder
(`paraphrase-multilingual-MiniLM-L12-v2`) covers **50+ languages
in the same vector space**. You don't need separate per-language
catalogs.

## Best practice: mix anchors per class

Authoring a single catalog with multilingual anchors is more
robust than maintaining N copies:

```yaml
classes:
  - name: track_order
    anchors:
      - "Where is my order?"           # EN
      - "Quand vais-je recevoir mon colis ?"  # FR
      - "Wo ist mein Paket?"           # DE
      - "¿Dónde está mi pedido?"       # ES
```

A user query in any language (EN/FR/DE/ES + 46 more) embeds close
to this multi-language cluster, so cosine similarity routes it to
the right class regardless of language.

## Performance under multilingual load

* **No language-detection step needed.** The embedder is the
  language detector — implicitly, via vector proximity.
* **Adding a 5th language to a catalog is ≈ 2-3 anchors per class**,
  not a re-train.
* **Latency is unaffected** — embedding cost depends on token
  count, not language.

## Edge cases

* **Code-switching** ("Where is mon colis?") — embeds OK, classifies
  correctly if at least one anchor in either language exists.
* **Languages not in the bundled model's set** — most CJK languages
  work; some low-resource African languages don't. Fall back to
  English or run a translation step upstream.
* **Romanized text in non-Latin scripts** — generally OK; native
  scripts are usually better.

## Reference

`examples/03_multilang_streaming.py` — concurrent classification
of an FR + EN stream against the bundled `support_triage`
catalog.
