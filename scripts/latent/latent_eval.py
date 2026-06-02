#!/usr/bin/env python3
# SPDX-License-Identifier: UNLICENSED
"""latent_eval.py — EFE P6 Lot 1 : l'experience go/no-go du moat (version rigoureuse).

UNE question : le CONSENSUS tensoriel (raffinement iteratif sur les ancres) classe-t-il
les prompts en TaskClass MIEUX qu'un simple CENTROIDE-kNN (one-shot) — surtout sur les
prompts AMBIGUS ? Si non, P2/P6 reposent sur du vide (on ferait du kNN).

Design CONTROLE : kNN et consensus partagent EXACTEMENT le meme code, le meme embedding et
les memes ancres. SEULE difference = le nombre d'iterations de raffinement :
    kNN       = consensus a 1 iteration ;  consensus = K iterations + entropie (confiance).
=> tout delta est imputable a l'ITERATION.

RIGUEUR STATISTIQUE (le coeur de cette version) : le verdict ne repose PAS sur un seul split
(n petit -> bruit). On fait `--repeats` splits stratifies aleatoires et on rapporte
MOYENNE ± ECART-TYPE. L'embedding est calcule UNE FOIS (couteux), les splits sont ~gratuits.
Le verdict regarde si le Delta(ambigu) est robuste (moyenne ± sigma vs 0 / seuil), et donne
aussi une accuracy "lenient" (un ambigu est juste si pred ∈ {classe primaire} ∪ `also`).

Offline, 0 $, deterministe (a seed fixe). Embedder PLUGGABLE (hashing / sentence-transformers).
Le VERDICT a besoin d'un vrai embedding (--embedder st, ideal --st-model BAAI/bge-m3).

Usage :
  ./latent_eval.py --corpus scripts/latent/corpus/taskclass_seed_v1.json --repeats 30
  ./latent_eval.py --corpus ... --embedder st --repeats 30
  ./latent_eval.py --corpus ... --embedder st --sweep            # grille d'hyperparams reproductible
"""
from __future__ import annotations

import argparse
import json
import math
import re
from datetime import datetime, timezone

import numpy as np
from sklearn.metrics import f1_score

# ─────────────────────────────────────────────────────────────────────────
# Embedders (pluggables) — embed(list[str]) -> (n, dim) L2-norme.
# ─────────────────────────────────────────────────────────────────────────
def _l2norm(X: np.ndarray) -> np.ndarray:
    n = np.linalg.norm(X, axis=1, keepdims=True)
    n[n == 0] = 1.0
    return X / n


class HashingEmbedder:
    """Embedding deterministe SANS dependance : sac de n-grams (mots + char-3grams) hashes,
    sqrt(tf), L2-norme. Crude mais l'embedding est une variable CONTROLEE (kNN et consensus
    l'utilisent a l'identique) -> la comparaison reste valide. Verdict reel : --embedder st."""

    def __init__(self, dim: int = 512):
        self.dim = dim
        self.name = f"hashing(dim={dim})"

    @staticmethod
    def _tokens(text: str) -> list[str]:
        words = re.findall(r"[a-zà-ÿ0-9]+", text.lower())
        toks = list(words)
        for w in words:
            ww = f"#{w}#"
            for i in range(len(ww) - 2):
                toks.append(ww[i:i + 3])
        return toks

    def _bucket(self, tok: str) -> int:
        h = 1469598103934665603
        for ch in tok.encode("utf-8"):
            h = (h ^ ch) * 1099511628211 & 0xFFFFFFFFFFFFFFFF
        return h % self.dim

    def embed(self, texts: list[str]) -> np.ndarray:
        X = np.zeros((len(texts), self.dim), dtype=np.float64)
        for i, text in enumerate(texts):
            counts: dict[int, float] = {}
            for tok in self._tokens(text):
                b = self._bucket(tok)
                counts[b] = counts.get(b, 0.0) + 1.0
            for b, c in counts.items():
                X[i, b] = math.sqrt(c)
        return _l2norm(X)


class StEmbedder:
    """sentence-transformers — vrai embedding (cible prod : bge-m3)."""

    def __init__(self, model: str):
        from sentence_transformers import SentenceTransformer

        self.model = SentenceTransformer(model)
        self.name = f"st:{model}"

    def embed(self, texts: list[str]) -> np.ndarray:
        v = self.model.encode(texts, normalize_embeddings=True, show_progress_bar=False)
        return np.asarray(v, dtype=np.float64)


# ─────────────────────────────────────────────────────────────────────────
# Ancres + consensus (inchanges : la math du test).
# ─────────────────────────────────────────────────────────────────────────
def build_anchors(X, y, n_classes, per_class, seed):
    anchors, anchor_cls = [], []
    for ci in range(n_classes):
        Xc = X[y == ci]
        if len(Xc) == 0:
            continue
        if per_class <= 1 or len(Xc) <= per_class:
            anchors.append(Xc.mean(axis=0)); anchor_cls.append(ci)
        else:
            from sklearn.cluster import KMeans
            km = KMeans(n_clusters=per_class, random_state=seed, n_init=10).fit(Xc)
            for c in km.cluster_centers_:
                anchors.append(c); anchor_cls.append(ci)
    return _l2norm(np.vstack(anchors)), np.array(anchor_cls)


def _softmax(Z):
    Z = Z - Z.max(axis=1, keepdims=True)
    E = np.exp(Z)
    return E / E.sum(axis=1, keepdims=True)


def consensus_class_dist(Q, A, anchor_cls, n_classes, iters, tau, alpha, beta, eps):
    """(n, n_classes). iters=1 -> kNN one-shot ; iters>1 -> raffinement."""
    S = _softmax((Q @ A.T) / tau)
    for _ in range(max(0, iters - 1)):
        ctx = S @ A
        Qx = _l2norm(alpha * Q + beta * ctx)
        S_new = _softmax((Qx @ A.T) / tau)
        if np.abs(S_new - S).max() < eps:
            S = S_new
            break
        S = S_new
    D = np.zeros((Q.shape[0], n_classes))
    for m in range(A.shape[0]):
        D[:, anchor_cls[m]] += S[:, m]
    D /= D.sum(axis=1, keepdims=True)
    return D


def predict(D):
    pred = D.argmax(axis=1)
    C = D.shape[1]
    with np.errstate(divide="ignore", invalid="ignore"):
        H = -np.sum(np.where(D > 0, D * np.log(D), 0.0), axis=1) / math.log(C)
    return pred, 1.0 - H


# ─────────────────────────────────────────────────────────────────────────
# Metriques.
# ─────────────────────────────────────────────────────────────────────────
def macro_f1(yt, yp, n):
    return float(f1_score(yt, yp, labels=list(range(n)), average="macro", zero_division=0)) if len(yt) else float("nan")


def ece(yt, yp, conf, n_bins=10):
    if len(yt) == 0:
        return float("nan")
    yt, yp, conf = map(np.asarray, (yt, yp, conf))
    correct = (yt == yp).astype(float)
    e, n = 0.0, len(yt)
    for b in range(n_bins):
        lo, hi = b / n_bins, (b + 1) / n_bins
        m = (conf >= lo) & (conf < hi if b < n_bins - 1 else conf <= hi)
        if m.sum():
            e += (m.sum() / n) * abs(correct[m].mean() - conf[m].mean())
    return float(e)


# ─────────────────────────────────────────────────────────────────────────
# UN run = un split stratifie + eval. Retourne des scalaires.
# ─────────────────────────────────────────────────────────────────────────
def one_run(X, y, also, clear_by_cls, ambig_idx, n_classes, cfg, eval_frac, seed):
    rng = np.random.default_rng(seed)
    train_idx, eval_clear_idx = [], []
    for _, idxs in clear_by_cls.items():
        perm = [idxs[k] for k in rng.permutation(len(idxs))]
        ne = max(1, int(round(len(perm) * eval_frac)))
        eval_clear_idx += perm[:ne]
        train_idx += perm[ne:]
    eval_idx = eval_clear_idx + ambig_idx
    is_amb = np.array([False] * len(eval_clear_idx) + [True] * len(ambig_idx))

    A, acl = build_anchors(X[train_idx], y[train_idx], n_classes, cfg["per_class"], seed)
    Q, yev = X[eval_idx], y[eval_idx]
    also_ev = [also[i] for i in eval_idx]

    Dk = consensus_class_dist(Q, A, acl, n_classes, 1, cfg["tau"], cfg["alpha"], cfg["beta"], cfg["eps"])
    Dc = consensus_class_dist(Q, A, acl, n_classes, cfg["iters"], cfg["tau"], cfg["alpha"], cfg["beta"], cfg["eps"])
    pk, ck = predict(Dk)
    pc, cc = predict(Dc)
    pdf = np.zeros(len(yev), int)  # defaut DTO = classe 0 (FactualQa)

    def acc(mask, p):
        return float(np.mean(p[mask] == yev[mask])) if mask.sum() else float("nan")

    def acc_lenient(mask, p):
        if not mask.sum():
            return float("nan")
        ok = 0
        for j in np.where(mask)[0]:
            if p[j] == yev[j] or p[j] in also_ev[j]:
                ok += 1
        return ok / int(mask.sum())

    gall = np.ones(len(yev), bool)
    out = {
        "acc_def_all": acc(gall, pdf),
        "acc_knn_all": acc(gall, pk), "acc_con_all": acc(gall, pc),
        "acc_knn_fac": acc(~is_amb, pk), "acc_con_fac": acc(~is_amb, pc),
        "acc_knn_amb": acc(is_amb, pk), "acc_con_amb": acc(is_amb, pc),
        "len_knn_amb": acc_lenient(is_amb, pk), "len_con_amb": acc_lenient(is_amb, pc),
        "f1_knn": macro_f1(yev, pk, n_classes), "f1_con": macro_f1(yev, pc, n_classes),
        "ece_knn": ece(yev, pk, ck), "ece_con": ece(yev, pc, cc),
    }
    diff = pk != pc
    out["disag"] = int(diff.sum())
    out["disag_con_minus_knn"] = int(np.sum((pc == yev) & diff)) - int(np.sum((pk == yev) & diff))
    out["delta_amb"] = out["acc_con_amb"] - out["acc_knn_amb"]
    out["delta_amb_len"] = out["len_con_amb"] - out["len_knn_amb"]
    return out


def repeated(X, y, also, clear_by_cls, ambig_idx, n_classes, cfg, eval_frac, repeats, base_seed):
    runs = [one_run(X, y, also, clear_by_cls, ambig_idx, n_classes, cfg, eval_frac, base_seed + r)
            for r in range(repeats)]
    keys = runs[0].keys()
    agg = {}
    for k in keys:
        vals = np.array([r[k] for r in runs], dtype=float)
        vals = vals[~np.isnan(vals)]
        agg[k] = (float(vals.mean()), float(vals.std())) if len(vals) else (float("nan"), 0.0)
    return agg


# ─────────────────────────────────────────────────────────────────────────
def _verdict(embedder_is_hashing, agg):
    if embedder_is_hashing:
        return "INDICATIF (embedder hashing) — relancer --embedder st pour le verdict reel"
    dmu, dsd = agg["delta_amb"]
    if dmu - dsd > 0.05:
        return "GO — consensus bat le kNN sur l'ambigu de facon ROBUSTE (moy - sigma > +5pts)"
    if dmu + dsd < 0.02:
        return "NO-GO — consensus n'ajoute rien sur l'ambigu (moy + sigma < +2pts) : la methode = kNN"
    return "INCONCLUS — le gain ambigu chevauche 0 (sous-puissance) : agrandir le seed / passer a bge-m3"


def main() -> int:
    ap = argparse.ArgumentParser(description="P6 Lot 1 — consensus latent vs kNN (rigueur stat)")
    ap.add_argument("--corpus", required=True)
    ap.add_argument("--embedder", choices=["hashing", "st"], default="hashing")
    ap.add_argument("--st-model", default="paraphrase-multilingual-MiniLM-L12-v2")
    ap.add_argument("--dim", type=int, default=512)
    ap.add_argument("--iters", type=int, default=6)
    ap.add_argument("--tau", type=float, default=0.1)
    ap.add_argument("--alpha", type=float, default=1.0)
    ap.add_argument("--beta", type=float, default=1.0)
    ap.add_argument("--eps", type=float, default=1e-4)
    ap.add_argument("--per-class", type=int, default=1)
    ap.add_argument("--eval-frac", type=float, default=0.4)
    ap.add_argument("--repeats", type=int, default=20, help="splits stratifies aleatoires (moy ± sigma)")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--sweep", action="store_true", help="grille d'hyperparams reproductible")
    ap.add_argument("--out-dir", default="scripts/latent")
    args = ap.parse_args()

    with open(args.corpus, encoding="utf-8") as f:
        corpus = json.load(f)
    classes = corpus["classes"]
    cidx = {c: i for i, c in enumerate(classes)}
    prompts = corpus["prompts"]
    nC = len(classes)

    # Embedding UNE FOIS (tous les prompts), puis les splits re-partitionnent les indices.
    emb = HashingEmbedder(args.dim) if args.embedder == "hashing" else StEmbedder(args.st_model)
    X = emb.embed([p["text"] for p in prompts])
    y = np.array([cidx[p["task_class"]] for p in prompts])
    also = [frozenset(cidx[a] for a in p.get("also", []) if a in cidx) for p in prompts]
    clear_by_cls: dict[int, list[int]] = {}
    ambig_idx: list[int] = []
    for i, p in enumerate(prompts):
        if p.get("ambiguous"):
            ambig_idx.append(i)
        else:
            clear_by_cls.setdefault(cidx[p["task_class"]], []).append(i)

    def mkcfg(iters, tau, beta):
        return {"iters": iters, "tau": tau, "alpha": args.alpha, "beta": beta,
                "eps": args.eps, "per_class": args.per_class}

    is_hash = args.embedder == "hashing"

    # ── Mode SWEEP : grille reproductible ──
    if args.sweep:
        grid = [(it, ta, be)
                for it in (4, 6, 8, 12) for ta in (0.05, 0.1, 0.15) for be in (1.0, 2.0)]
        print("═" * 74)
        print(f" P6 Lot 1 — SWEEP ({emb.name}, repeats={args.repeats})")
        print("═" * 74)
        print(f" {'iters':>5} {'tau':>5} {'beta':>5}  {'Δamb strict (moy±σ)':>22} {'Δamb lenient':>16}")
        print("─" * 74)
        best = None
        rows = []
        for it, ta, be in grid:
            agg = repeated(X, y, also, clear_by_cls, ambig_idx, nC, mkcfg(it, ta, be),
                           args.eval_frac, args.repeats, args.seed)
            dmu, dsd = agg["delta_amb"]; lmu, lsd = agg["delta_amb_len"]
            print(f" {it:>5} {ta:>5} {be:>5}  {dmu:>+8.1%} ± {dsd:>5.1%}        {lmu:>+6.1%} ± {lsd:>4.1%}")
            rows.append({"iters": it, "tau": ta, "beta": be, "delta_amb": [dmu, dsd], "delta_amb_len": [lmu, lsd]})
            if best is None or dmu > best[0]:
                best = (dmu, dsd, it, ta, be)
        print("─" * 74)
        print(f" Meilleur Δamb : {best[0]:+.1%} ± {best[1]:.1%}  @ iters={best[2]} tau={best[3]} beta={best[4]}")
        rob = "AUCUNE config ne bat le kNN de facon robuste" if best[0] - best[1] <= 0.05 \
            else "une config bat le kNN de facon robuste"
        print(f" Robustesse : {rob}")
        print("═" * 74)
        stamp = datetime.now(timezone.utc).strftime("%Y-%m-%d")
        with open(f"{args.out_dir}/EFE_LATENT_LOT1_SWEEP_{stamp}.json", "w", encoding="utf-8") as f:
            json.dump({"embedder": emb.name, "repeats": args.repeats, "grid": rows,
                       "best": {"delta_amb": best[0], "std": best[1], "iters": best[2],
                                "tau": best[3], "beta": best[4]}, "robust": best[0] - best[1] > 0.05},
                      f, ensure_ascii=False, indent=2)
        return 0

    # ── Mode standard : un cfg, repeats splits, moy ± sigma ──
    cfg = mkcfg(args.iters, args.tau, args.beta)
    agg = repeated(X, y, also, clear_by_cls, ambig_idx, nC, cfg, args.eval_frac, args.repeats, args.seed)
    verdict = _verdict(is_hash, agg)

    payload = {
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "embedder": emb.name, "repeats": args.repeats,
        "iters": args.iters, "tau": args.tau, "alpha": args.alpha, "beta": args.beta,
        "per_class": args.per_class, "eval_frac": args.eval_frac,
        "n_prompts": len(prompts), "n_clear": sum(len(v) for v in clear_by_cls.values()),
        "n_ambig": len(ambig_idx), "agg": agg, "verdict": verdict,
    }
    stamp = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    base = f"{args.out_dir}/EFE_LATENT_LOT1_{stamp}"
    with open(f"{base}.json", "w", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False, indent=2)
    _write_report(f"{base}.md", payload)
    _console(payload)
    print(f"\n✅ Rapport : {base}.md  ·  donnees : {base}.json")
    return 0


def _pm(t):  # (mean, std) -> "x% ± y%"
    return f"{t[0]:+.1%} ± {t[1]:.1%}"


def _pp(t):
    return f"{t[0]:.0%} ± {t[1]:.0%}"


def _console(p):
    a = p["agg"]
    print("═" * 72)
    print(" P6 Lot 1 — consensus latent vs kNN  (moyenne ± σ sur splits répétés)")
    print("═" * 72)
    print(f" Embedder {p['embedder']} · repeats={p['repeats']} · iters={p['iters']} tau={p['tau']} beta={p['beta']}")
    print(f" {p['n_clear']} clairs · {p['n_ambig']} ambigus")
    print("─" * 72)
    print(f" {'groupe':<8} {'kNN':>14} {'consensus':>14}")
    print(f" {'facile':<8} {_pp(a['acc_knn_fac']):>14} {_pp(a['acc_con_fac']):>14}")
    print(f" {'ambigu':<8} {_pp(a['acc_knn_amb']):>14} {_pp(a['acc_con_amb']):>14}")
    print(f" {'amb.len':<8} {_pp(a['len_knn_amb']):>14} {_pp(a['len_con_amb']):>14}")
    print("─" * 72)
    print(f" Δaccuracy AMBIGU strict  : {_pm(a['delta_amb'])}   ← le chiffre qui décide")
    print(f" Δaccuracy AMBIGU lenient : {_pm(a['delta_amb_len'])}")
    print(f" ECE (plus bas=mieux)     : kNN {a['ece_knn'][0]:.3f}±{a['ece_knn'][1]:.3f}  consensus {a['ece_con'][0]:.3f}±{a['ece_con'][1]:.3f}")
    print(f" Desaccords nets consensus: {a['disag_con_minus_knn'][0]:+.1f} ± {a['disag_con_minus_knn'][1]:.1f}")
    print("─" * 72)
    print(f" VERDICT : {p['verdict']}")
    print("═" * 72)


def _write_report(path, p):
    a = p["agg"]
    L = [
        f"# EFE P6 Lot 1 — consensus latent vs centroïde-kNN · {p['embedder']}",
        "",
        f"> Go/no-go du moat, **{p['repeats']} splits stratifiés répétés** (moyenne ± σ — le verdict "
        f"ne dépend plus d'un seul tirage). kNN = consensus@1iter ; consensus = {p['iters']} iters. "
        f"Même embedding & ancres → tout écart vient de l'itération. {p['n_clear']} clairs, "
        f"{p['n_ambig']} ambigus. Généré {p['generated_utc']}.",
        "",
        "## Verdict",
        "",
        f"**{p['verdict']}**",
        "",
        f"- **Δaccuracy AMBIGU strict (consensus − kNN) : {_pm(a['delta_amb'])}** ← décide.",
        f"- Δaccuracy AMBIGU **lenient** (pred ∈ primaire ∪ `also`) : {_pm(a['delta_amb_len'])}.",
        f"- ECE : kNN {a['ece_knn'][0]:.3f} ± {a['ece_knn'][1]:.3f} · consensus {a['ece_con'][0]:.3f} ± {a['ece_con'][1]:.3f}.",
        f"- Désaccords nets en faveur du consensus : {a['disag_con_minus_knn'][0]:+.1f} ± {a['disag_con_minus_knn'][1]:.1f} prompts.",
        "",
        "## Accuracy par groupe (moyenne ± σ)",
        "",
        "| groupe | kNN (1 iter) | consensus (K iter) |",
        "|---|---|---|",
        f"| facile | {_pp(a['acc_knn_fac'])} | {_pp(a['acc_con_fac'])} |",
        f"| ambigu (strict) | {_pp(a['acc_knn_amb'])} | {_pp(a['acc_con_amb'])} |",
        f"| ambigu (lenient) | {_pp(a['len_knn_amb'])} | {_pp(a['len_con_amb'])} |",
        f"| macro-F1 (global) | {a['f1_knn'][0]:.2f} ± {a['f1_knn'][1]:.2f} | {a['f1_con'][0]:.2f} ± {a['f1_con'][1]:.2f} |",
        "",
        "## Comment lire",
        "",
        "- **σ** dit si l'écart est robuste ou du bruit. Si la moyenne ± σ du Δambigu **chevauche 0**, "
        "le gain n'est pas prouvé (sous-puissance) → agrandir le seed ou passer à **bge-m3**.",
        "- **lenient** : un ambigu est juste si la prédiction tombe sur la classe primaire **ou** une "
        "classe `also` légitime (frontière) — métrique plus équitable pour les ambigus.",
        "- ⚠️ Embedder **hashing** = indicatif. **Verdict réel : `--embedder st`** (idéal `BAAI/bge-m3`).",
        "- Sweep reproductible : `--sweep` (grille iters×tau×beta).",
        "",
    ]
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(L))


if __name__ == "__main__":
    raise SystemExit(main())
