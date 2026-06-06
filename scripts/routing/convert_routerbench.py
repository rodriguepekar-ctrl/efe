#!/usr/bin/env python3
# SPDX-License-Identifier: UNLICENSED
"""convert_routerbench.py — telecharge RouterBench (withmartian/routerbench) et le convertit en
matrice JSON consommable par route_eval.py (`--matrix`).

RouterBench (0-shot) : 36 497 prompts x 11 modeles. Pour chaque (prompt, modele) : une QUALITE
(colonne `<modele>`, 0-1) et un COUT (`<modele>|total_cost`, par-prompt). On subsample de facon
STRATIFIEE par tache (`eval_name`) pour garder la diversite, on embedde les prompts (sentence-
transformers multilingue par defaut — RouterBench est multilingue) pour donner des features au
kNN-routeur, et on ecrit le format route_eval :
    {model_names, latency_ms, prompts:[{id, features, quality:[K], cost:[K]}]}

Dependances : huggingface_hub + pandas (lecture du pickle). Sortie : 0 dependance pour route_eval.

Usage :
  ./convert_routerbench.py --n 2500 --out scripts/routing/routerbench_matrix.json
  ./convert_routerbench.py --n 2500 --embed tfidf      # sans modele (plus rapide, multilingue faible)
"""
from __future__ import annotations

import argparse
import json

import numpy as np

# Les 11 modeles RouterBench (colonnes de performance ; chacun a un `|total_cost`).
MODELS = [
    "WizardLM/WizardLM-13B-V1.2",
    "mistralai/mistral-7b-chat",
    "meta/llama-2-70b-chat",
    "mistralai/mixtral-8x7b-chat",
    "zero-one-ai/Yi-34B-Chat",
    "meta/code-llama-instruct-34b-chat",
    "claude-instant-v1",
    "gpt-3.5-turbo-1106",
    "claude-v1",
    "claude-v2",
    "gpt-4-1106-preview",
]


def _text(v) -> str:
    if isinstance(v, (list, tuple, np.ndarray)):
        return " ".join(str(x) for x in v)
    return str(v)


def main() -> int:
    ap = argparse.ArgumentParser(description="RouterBench -> matrice JSON route_eval")
    ap.add_argument("--shot", default="0shot", choices=["0shot", "5shot"])
    ap.add_argument("--n", type=int, default=2500, help="taille du subsample stratifie")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--embed", choices=["st", "tfidf"], default="st")
    ap.add_argument("--st-model", default="paraphrase-multilingual-MiniLM-L12-v2")
    ap.add_argument("--out", default="scripts/routing/routerbench_matrix.json")
    args = ap.parse_args()

    from huggingface_hub import hf_hub_download
    import pandas as pd

    print(f"▶ telechargement routerbench_{args.shot}.pkl …")
    path = hf_hub_download("withmartian/routerbench", f"routerbench_{args.shot}.pkl", repo_type="dataset")
    df = pd.read_pickle(path)
    print(f"  charge : {df.shape[0]} prompts, {df.shape[1]} colonnes")

    models = [m for m in MODELS if m in df.columns and f"{m}|total_cost" in df.columns]
    if len(models) < 2:
        print("❌ colonnes modeles introuvables", flush=True)
        return 2
    cost_cols = [f"{m}|total_cost" for m in models]

    # Drop lignes avec qualite/cout manquant.
    df = df.dropna(subset=models + cost_cols).reset_index(drop=True)

    # Subsample STRATIFIE par eval_name (tache).
    n_total = len(df)
    parts = []
    for _, g in df.groupby("eval_name"):
        k = max(1, round(args.n * len(g) / n_total))
        parts.append(g.sample(min(k, len(g)), random_state=args.seed))
    df_s = pd.concat(parts)
    if len(df_s) > args.n:
        df_s = df_s.sample(args.n, random_state=args.seed)
    df_s = df_s.reset_index(drop=True)
    print(f"  subsample stratifie : {len(df_s)} prompts sur {df_s['eval_name'].nunique()} taches")

    Q = np.clip(df_s[models].to_numpy(dtype=float), 0.0, 1.0)        # (N, K) qualite
    C = df_s[cost_cols].to_numpy(dtype=float)                         # (N, K) cout par-prompt
    texts = [_text(t) for t in df_s["prompt"].tolist()]
    ids = [str(s) for s in df_s["sample_id"].tolist()]

    print(f"  qualite ∈ [{Q.min():.2f}, {Q.max():.2f}]  ·  cout/req moyen par modele :")
    for m, c in zip(models, C.mean(axis=0)):
        print(f"    {m:<38} {c:.5f}")

    # Embedding des prompts -> features pour le kNN.
    print(f"▶ embedding ({args.embed}) de {len(texts)} prompts …")
    if args.embed == "st":
        from sentence_transformers import SentenceTransformer
        F = SentenceTransformer(args.st_model).encode(texts, normalize_embeddings=True, show_progress_bar=False)
        F = np.asarray(F, dtype=float)
    else:
        from sklearn.feature_extraction.text import TfidfVectorizer
        F = TfidfVectorizer(min_df=2, max_features=512).fit_transform(texts).toarray().astype(float)
        F = F / np.maximum(np.linalg.norm(F, axis=1, keepdims=True), 1e-9)

    prompts = [
        {"id": ids[i], "features": [round(float(x), 4) for x in F[i]],
         "quality": [round(float(x), 4) for x in Q[i]], "cost": [float(x) for x in C[i]]}
        for i in range(len(texts))
    ]
    out = {
        "_source": f"withmartian/routerbench {args.shot}",
        "model_names": models,
        "latency_ms": [0.0] * len(models),  # RouterBench ne fournit pas la latence
        "prompts": prompts,
    }
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(out, f, ensure_ascii=False)
    print(f"✅ matrice ecrite : {args.out}  ({len(prompts)} prompts x {len(models)} modeles)")
    print(f"   verdict : python scripts/routing/route_eval.py --matrix {args.out} --repeats 30")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
