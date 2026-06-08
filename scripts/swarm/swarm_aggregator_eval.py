#!/usr/bin/env python3
# SPDX-License-Identifier: UNLICENSED
"""swarm_aggregator_eval.py — Phase 3 gate (suite d'ADR-SWARM-001).

QUESTION : un AGRÉGATEUR réel sur les K candidats bon marché approche-t-il le PLAFOND OR
(≈0,91, qui bat le monolithe à 3× moins cher) plutôt que le PLANCHER vote (0,43) ?

DEUX FAMILLES D'AGRÉGATEURS — et ce que chacune exige :
  A. A-PRIORI (sélection avant de voir les réponses, depuis les FEATURES du prompt).
     → MESURABLE sur RouterBench (features + matrice qualité). C'est R0 appliqué aux K cheap.
     Politique `selector` : kNN appris (labels = un candidat correct sur le train) généralisé
     au held-out. Rigueur : splits répétés (moy ± σ).
  B. A-POSTERIORI (vérificateur/jury/débat qui LIT les réponses).
     → NON mesurable sur RouterBench (pas de texte de réponse). On le SPÉCIFIE : un vérificateur
     de précision v (probabilité d'identifier un candidat correct quand il en existe un) donne,
     en forme close,  acc(v) = v·OR + (1−v)·rand   (rand = accuracy d'un pick aléatoire parmi K).
     On calcule v* = précision MINIMALE pour battre le monolithe. C'est le CAHIER DES CHARGES
     du build Phase 3. Le test pleinement réel = run modèles réels (swarm_slm.rs + agrégateur).

VERDICT :
  - GO a-priori   : le selector appris bat le monolithe à coût inférieur (robuste) → routage suffit.
  - SPEC          : sinon, on rend v* (la barre que doit franchir un vérificateur a-posteriori) ;
                    si v* est atteignable par les briques EFE (gold-test-gate, jury) sur tâches
                    vérifiables → GO conditionnel ciblé ; sinon → l'agrégation est intrinsèquement dure.

Usage :
  ./swarm_aggregator_eval.py --matrix scripts/routing/routerbench_matrix.json --repeats 30
"""
from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone

import numpy as np
from sklearn.neighbors import KNeighborsClassifier


def load(path):
    """Charge → (names, Q[N,K], C[N,K], F[N,D]) triés par coût croissant (0 = moins cher)."""
    with open(path, encoding="utf-8") as f:
        d = json.load(f)
    names = list(d["model_names"])
    prompts = d["prompts"]
    Q = np.asarray([p["quality"] for p in prompts], dtype=float)
    if all("cost" in p for p in prompts):
        C = np.asarray([p["cost"] for p in prompts], dtype=float)
    else:
        C = np.broadcast_to(np.asarray(d["cost"], dtype=float), Q.shape).copy()
    if all("features" in p for p in prompts):
        F = np.asarray([p["features"] for p in prompts], dtype=float)
    else:
        raise SystemExit("matrice sans 'features' : le sélecteur a-priori en a besoin.")
    F = F / np.maximum(np.linalg.norm(F, axis=1, keepdims=True), 1e-9)
    order = np.argsort(C.mean(axis=0))
    return [names[i] for i in order], Q[:, order], C[:, order], F


def selector_run(Qb_k, F, k, eval_frac, seed):
    """kNN appris : label = 1er candidat correct (sur le train), généralisé au held-out depuis
    les features. Renvoie l'accuracy du candidat sélectionné sur l'eval (a-priori, A)."""
    n = Qb_k.shape[0]
    rng = np.random.default_rng(seed)
    perm = rng.permutation(n)
    ne = max(1, int(round(n * eval_frac)))
    ev, tr = perm[:ne], perm[ne:]

    # Labels train : index d'un candidat correct (1er correct) ; -1 si aucun correct.
    lbl = np.full(len(tr), -1, dtype=int)
    for i, p in enumerate(tr):
        corrects = np.flatnonzero(Qb_k[p] >= 0.5)
        if corrects.size:
            lbl[i] = int(corrects[0])
    keep = lbl >= 0
    if keep.sum() < 2 or len(np.unique(lbl[keep])) < 2:
        # dégénéré : pas assez de signal → prédit le candidat le + souvent correct.
        pred_idx = int(np.argmax(Qb_k[tr].mean(axis=0)))
        sel = np.full(len(ev), pred_idx)
    else:
        kk = min(k, int(keep.sum()))
        clf = KNeighborsClassifier(n_neighbors=kk, weights="distance").fit(F[tr][keep], lbl[keep])
        sel = clf.predict(F[ev])
    acc = float(Qb_k[ev, sel].mean())
    return acc


def analyze(names, Q, C, F, kset, theta, eval_frac, repeats, seed):
    Qb = (Q >= theta).astype(float)
    N, K = Qb.shape
    acc_per_model = Qb.mean(axis=0)
    cost_per_model = C.mean(axis=0)
    mono_best = float(acc_per_model.max())
    mono_best_cost = float(cost_per_model[int(np.argmax(acc_per_model))])

    res = {"mono_best": {"acc": mono_best, "cost": mono_best_cost}, "K": {}}
    for k in kset:
        k = min(k, K)
        Qb_k = Qb[:, :k]
        cost_sum = float(C[:, :k].sum(axis=1).mean())
        votes = Qb_k.sum(axis=1)
        vote_acc = float((votes > k / 2.0).mean())
        or_acc = float((votes >= 1).mean())
        rand_acc = float(Qb_k.mean())                 # pick aléatoire parmi K = accuracy moyenne
        # Sélecteur a-priori (mesuré, splits répétés).
        sel = np.array([selector_run(Qb_k, F, 15, eval_frac, seed + r) for r in range(repeats)])
        sel_acc, sel_sd = float(sel.mean()), float(sel.std())
        # Cahier des charges vérificateur a-posteriori : v* pour battre le monolithe.
        # acc(v) = v·OR + (1−v)·rand  (linéaire) → v* = (mono−rand)/(OR−rand).
        denom = or_acc - rand_acc
        v_star = (mono_best - rand_acc) / denom if denom > 1e-9 else float("inf")
        # capture du sélecteur entre rand (0) et OR (1).
        sel_capture = (sel_acc - rand_acc) / denom if denom > 1e-9 else float("nan")
        res["K"][k] = {
            "cost": cost_sum, "vote": vote_acc, "or": or_acc, "rand": rand_acc,
            "selector": (sel_acc, sel_sd), "sel_capture": sel_capture,
            "v_star": v_star, "cheaper_than_mono": cost_sum < mono_best_cost,
        }
    return res


def verdict(res):
    mono = res["mono_best"]["acc"]
    # 1. Un sélecteur a-priori bat-il le monolithe à coût inférieur (robuste) ?
    for k, r in res["K"].items():
        sa, sd = r["selector"]
        if (sa - sd) >= mono and r["cheaper_than_mono"]:
            return ("GO a-priori", f"selector(K={k}) {sa:.3f} ≥ monolithe {mono:.3f} à coût inférieur "
                    "— la sélection sur features suffit (surprenant)")
    # 2. Sinon : la cible ACTIONNABLE = le K le moins cher que le monolithe, avec le v* le plus
    #    atteignable (et v* ≤ 1, sinon même un vérificateur parfait ne suffit pas à ce K).
    feasible = [k for k, r in res["K"].items()
                if r["cheaper_than_mono"] and np.isfinite(r["v_star"]) and r["v_star"] <= 1.0]
    if not feasible:
        return ("NO-GO", "à coût inférieur au monolithe, même un vérificateur parfait (OR) ne "
                "rattrape pas le monolithe : la capacité des K moins chers est insuffisante")
    best_k = min(feasible, key=lambda k: res["K"][k]["v_star"])
    r = res["K"][best_k]
    sa = r["selector"][0]
    return ("SPEC", f"a-priori insuffisant (selector K={best_k}={sa:.3f} ≈ rand {r['rand']:.3f}, "
            f"loin du plafond OR {r['or']:.3f}). Cible actionnable : un VÉRIFICATEUR a-posteriori de "
            f"précision v* ≥ {r['v_star']:.0%} sur K={best_k} candidats → bat le monolithe ({mono:.3f}) "
            f"à {res['mono_best']['cost']/r['cost']:.1f}× moins cher. "
            "→ build Phase 3 : jury/débat sur les RÉPONSES, pas routage sur features")


def main() -> int:
    ap = argparse.ArgumentParser(description="Phase 3 — agrégateur réel : a-priori mesuré + spec a-posteriori")
    ap.add_argument("--matrix", required=True)
    ap.add_argument("--theta", type=float, default=0.5)
    ap.add_argument("--kset", default="3,5,7,9")
    ap.add_argument("--eval-frac", type=float, default=0.4)
    ap.add_argument("--repeats", type=int, default=30)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--out-dir", default="scripts/swarm")
    args = ap.parse_args()

    names, Q, C, F = load(args.matrix)
    kset = [int(x) for x in args.kset.split(",") if int(x) <= Q.shape[1]] or [Q.shape[1]]
    res = analyze(names, Q, C, F, kset, args.theta, args.eval_frac, args.repeats, args.seed)
    vd, why = verdict(res)

    payload = {
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "source": args.matrix, "n_prompts": int(Q.shape[0]), "n_models": int(Q.shape[1]),
        "theta": args.theta, "kset": kset, "repeats": args.repeats,
        "result": res, "verdict": vd, "why": why,
    }
    stamp = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    base = f"{args.out_dir}/EFE_SWARM_AGG_{stamp}"
    with open(f"{base}.json", "w", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False, indent=2)
    _report(f"{base}.md", payload)
    _console(payload)
    print(f"\n✅ Rapport : {base}.md  ·  données : {base}.json")
    return 0


def _console(p):
    r = p["result"]
    print("═" * 80)
    print(" AGRÉGATEUR (Phase 3) — a-priori MESURÉ vs plafond OR · spec vérificateur a-posteriori")
    print("═" * 80)
    print(f" Source {p['source'].split('/')[-1]} · {p['n_prompts']} prompts · monolithe={r['mono_best']['acc']:.3f} "
          f"@ {r['mono_best']['cost']:.4g} · {p['repeats']} splits")
    print("─" * 80)
    print(f" {'K':>3} {'coût':>10} {'vote':>7} {'rand':>7} {'selector(a-priori)':>20} {'OR(plafond)':>12} {'v* requis':>10}")
    for k, d in r["K"].items():
        sa, sd = d["selector"]
        cheaper = "↓" if d["cheaper_than_mono"] else " "
        print(f" {k:>3} {d['cost']:>9.4g}{cheaper} {d['vote']:>7.3f} {d['rand']:>7.3f} "
              f"{sa:>11.3f} ± {sd:>4.3f} {d['or']:>12.3f} {d['v_star']:>9.0%}")
    print("─" * 80)
    print(f" VERDICT : {p['verdict']}")
    print(f"   → {p['why']}")
    print("═" * 80)


def _report(path, p):
    r = p["result"]
    L = [
        "# EFE Phase 3 — agrégateur réel : a-priori mesuré + cahier des charges a-posteriori",
        "",
        f"> Suite d'ADR-SWARM-001 (le pari = l'agrégateur). {p['n_prompts']} prompts × {p['n_models']} "
        f"modèles, **{p['repeats']} splits** (moy ± σ). Monolithe = {r['mono_best']['acc']:.3f} @ "
        f"{r['mono_best']['cost']:.4g}. θ={p['theta']}. Généré {p['generated_utc']}.",
        "",
        "## Verdict",
        "",
        f"**{p['verdict']}** — {p['why']}",
        "",
        "## A-priori mesuré (selector kNN) vs plafond OR · spec vérificateur v*",
        "",
        "| K | coût (somme) | vote | rand | selector (a-priori, ±σ) | OR (plafond) | v* requis |",
        "|---|---|---|---|---|---|---|",
    ]
    for k, d in r["K"].items():
        sa, sd = d["selector"]
        ch = " (↓ vs mono)" if d["cheaper_than_mono"] else ""
        L.append(f"| {k} | {d['cost']:.4g}{ch} | {d['vote']:.3f} | {d['rand']:.3f} | "
                 f"{sa:.3f} ± {sd:.3f} | {d['or']:.3f} | {d['v_star']:.0%} |")
    L += [
        "",
        "## Comment lire",
        "",
        "- **selector** = agrégateur A-PRIORI réel (kNN appris sur les features → choisit un candidat). "
        "MESURÉ sur vraies données, splits répétés.",
        "- **v\\*** = précision MINIMALE d'un vérificateur A-POSTERIORI (qui lit les réponses) pour que "
        "l'essaim batte le monolithe. Forme close : `v* = (mono − rand)/(OR − rand)`.",
        "- Si **selector ≈ rand** → la sélection sur features ne capte pas la valeur (écho R0) → il faut "
        "un vérificateur qui exploite les RÉPONSES (jury, débat, gold-test-gate sur tâches vérifiables).",
        "",
        "## ⚠️ Portée",
        "",
        "Le **selector** est pleinement mesuré (vrai ML, vraies features). Le **v\\*** est un CAHIER DES "
        "CHARGES (forme close), pas une mesure : RouterBench ne fournit pas le TEXTE des réponses, donc un "
        "vrai vérificateur/jury n'est pas mesurable ici. Le test pleinement réel = run modèles réels "
        "(`swarm_slm.rs` + agrégateur branché) sur une tâche vérifiable, à exécuter quand un backend réel "
        "(llama-server / provider) est disponible. Pas de latence dans RouterBench.",
        "",
    ]
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(L))


if __name__ == "__main__":
    raise SystemExit(main())
