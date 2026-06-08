#!/usr/bin/env python3
# SPDX-License-Identifier: UNLICENSED
"""swarm_eval.py — Phase 0 / Lot S1 : le gate scientifique H-SWARM.

QUESTION (la thèse EFE) : un ESSAIM de petits modèles bon marché, agrégé, bat-il un
modèle MONOLITHE unique sur le composite (qualité × coût) ? C'est le point de rupture
que ni Lot 1 (consensus tensoriel ≈ kNN) ni R0 (routage = goulot) n'ont tranché.

PRINCIPE (RouterBench-like, comme route_eval.py) : séparer le COÛTEUX du GRATUIT.
  - Une MATRICE Q[prompt, modèle] (qualité 0-1) + cost[prompt, modèle] — collectée une fois.
  - Évaluer une POLITIQUE = gratuit. Toutes les politiques, tous les splits → 0 $.

POLITIQUES (qualité binarisée à θ pour rendre le VOTE bien défini et comparable) :
  - mono_best   : LE meilleur modèle unique (accuracy max)         → le monolithe à battre
  - mono_cheap  : le modèle le moins cher seul                     → plancher
  - swarm_voteK : VOTE MAJORITAIRE des K modèles les moins chers   → essaim RÉALISTE
                  (self-consistency : correct si majorité corrects). coût = somme des K.
  - swarm_orK   : OR des K moins chers (vérificateur oracle)       → PLAFOND de l'essaim
                  (correct si AU MOINS un correct). coût = somme des K.

VERDICT H-SWARM (proxy) :
  - GO            : ∃K, swarm_voteK atteint la qualité de mono_best À COÛT INFÉRIEUR (robuste ±σ).
  - NO-GO qualité : même swarm_orK (plafond) < mono_best → les petits ne rattrapent pas le gros.
  - INCONCLUS     : le plafond gagne mais le vote non → l'AGRÉGATEUR est le goulot (écho R0).

⚠ PORTÉE (honnêteté chirurgicale) : ce proxy teste la forme la PLUS FAIBLE d'essaim —
  ensemble indépendant + agrégation, MÊME prompt, SANS passage d'infos inter-agent. Il ne
  teste PAS le substrat parallèle (→ test d'invariant CAS) ni la décomposition/émergence.
  C'est le gate le moins cher : un NO-GO ici est un signal négatif FORT ; un GO MOTIVE les
  topologies riches (Phase 2). Verdict réel sur tâches décomposables = run modèles réels.

Usage :
  ./swarm_eval.py --matrix scripts/routing/routerbench_matrix.json --repeats 50
  ./swarm_eval.py --matrix ... --theta 0.5 --kset 3,5,7,9
"""
from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone

import numpy as np


# ─────────────────────────────────────────────────────────────────────────────
def load(path):
    """Charge une matrice RouterBench-like → (names, Q[N,K] 0-1, C[N,K] coût), modèles
    triés par coût CROISSANT (index 0 = le moins cher)."""
    with open(path, encoding="utf-8") as f:
        d = json.load(f)
    names = list(d["model_names"])
    prompts = d["prompts"]
    Q = np.asarray([p["quality"] for p in prompts], dtype=float)
    if all("cost" in p for p in prompts):
        C = np.asarray([p["cost"] for p in prompts], dtype=float)          # (N,K) par-prompt
    else:
        C = np.broadcast_to(np.asarray(d["cost"], dtype=float), Q.shape).copy()
    order = np.argsort(C.mean(axis=0))                                      # coût croissant
    return [names[i] for i in order], Q[:, order], C[:, order]


# ─────────────────────────────────────────────────────────────────────────────
def policies(Qb, C, kset):
    """Calcule accuracy + coût/prompt de chaque politique sur un sous-ensemble de prompts.
    Qb = qualité BINARISÉE (0/1), C = coût (N,K). Renvoie {pol: (acc, cost)}."""
    n, K = Qb.shape
    out = {}
    # Monolithes : chaque modèle seul.
    acc_per_model = Qb.mean(axis=0)                 # (K,)
    cost_per_model = C.mean(axis=0)                 # (K,)
    best = int(np.argmax(acc_per_model))            # meilleur modèle unique
    out["mono_best"] = (float(acc_per_model[best]), float(cost_per_model[best]), best)
    out["mono_cheap"] = (float(acc_per_model[0]), float(cost_per_model[0]), 0)
    out["mono_premium"] = (float(acc_per_model[-1]), float(cost_per_model[-1]), K - 1)

    # Essaims sur les K moins chers.
    for k in kset:
        k = min(k, K)
        sub = Qb[:, :k]                              # (n,k) les k moins chers
        csub = C[:, :k].sum(axis=1)                  # coût = somme des k (on les fait tous)
        votes = sub.sum(axis=1)                      # nb de modèles "corrects" par prompt
        vote_ok = (votes > (k / 2.0)).astype(float)  # majorité stricte
        or_ok = (votes >= 1).astype(float)           # au moins un correct (plafond)
        out[f"vote{k}"] = (float(vote_ok.mean()), float(csub.mean()), k)
        out[f"or{k}"] = (float(or_ok.mean()), float(csub.mean()), k)
    return out


def repeated(Qb, C, kset, frac, repeats, seed):
    """Splits répétés (sous-échantillons de prompts) → moy ± σ par politique (rigueur Lot 1)."""
    n = Qb.shape[0]
    rng = np.random.default_rng(seed)
    keys = None
    acc_runs, cost_runs = {}, {}
    for r in range(repeats):
        idx = rng.choice(n, size=max(1, int(round(n * frac))), replace=False)
        p = policies(Qb[idx], C[idx], kset)
        if keys is None:
            keys = list(p.keys())
            acc_runs = {kk: [] for kk in keys}
            cost_runs = {kk: [] for kk in keys}
        for kk in keys:
            acc_runs[kk].append(p[kk][0])
            cost_runs[kk].append(p[kk][1])
    agg = {}
    for kk in keys:
        a = np.array(acc_runs[kk]); c = np.array(cost_runs[kk])
        agg[kk] = {"acc": (float(a.mean()), float(a.std())),
                   "cost": (float(c.mean()), float(c.std()))}
    return agg


# ─────────────────────────────────────────────────────────────────────────────
def verdict(agg, kset, K, margin):
    """Tranche H-SWARM. mono_best = monolithe à battre."""
    qm, qm_sd = agg["mono_best"]["acc"]
    cm = agg["mono_best"]["cost"][0]

    # 1. ∃K : le VOTE atteint la qualité du monolithe à coût inférieur (robuste) ?
    go = []
    for k in kset:
        k = min(k, K)
        qv, qv_sd = agg[f"vote{k}"]["acc"]
        cv = agg[f"vote{k}"]["cost"][0]
        # robuste : la borne basse du vote ≥ borne haute du mono - margin, et moins cher.
        if (qv - qv_sd) >= (qm - qm_sd - margin) and cv < cm:
            go.append((k, qv, cv))
    if go:
        k, qv, cv = max(go, key=lambda t: t[1])
        return ("GO", f"vote{k} atteint la qualité du monolithe ({qv:.3f} vs {qm:.3f}) "
                f"à coût inférieur ({cv:.5g} vs {cm:.5g}) — l'essaim bon marché bat le monolithe")

    # 2. Même le PLAFOND (OR) sous le monolithe → NO-GO qualité.
    best_or = max(min(k, K) for k in kset)
    qor = agg[f"or{best_or}"]["acc"][0]
    if qor < qm - margin:
        return ("NO-GO", f"même le plafond or{best_or} ({qor:.3f}) < monolithe ({qm:.3f}) : "
                "les petits modèles ne rattrapent pas le gros, même agrégés parfaitement")

    # 3. Le plafond gagne mais le vote non → l'agrégateur est le goulot.
    return ("INCONCLUS", f"le plafond OR atteint {qor:.3f} (≥ monolithe {qm:.3f}) mais le VOTE "
            "ne capte pas cette valeur → l'AGRÉGATEUR est le goulot (Phase 3), pas la capacité brute")


# ─────────────────────────────────────────────────────────────────────────────
def main() -> int:
    ap = argparse.ArgumentParser(description="H-SWARM — essaim de petits modèles vs monolithe")
    ap.add_argument("--matrix", required=True, help="matrice RouterBench-like JSON")
    ap.add_argument("--theta", type=float, default=0.5, help="seuil de binarisation 'correct'")
    ap.add_argument("--kset", default="3,5,7,9", help="tailles d'essaim (modèles les moins chers)")
    ap.add_argument("--frac", type=float, default=0.5, help="fraction de prompts par split")
    ap.add_argument("--repeats", type=int, default=50)
    ap.add_argument("--margin", type=float, default=0.01, help="marge d'égalité de qualité")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--out-dir", default="scripts/swarm")
    args = ap.parse_args()

    kset = [int(x) for x in args.kset.split(",")]
    names, Q, C = load(args.matrix)
    N, K = Q.shape
    Qb = (Q >= args.theta).astype(float)            # binarisation 'correct'
    kset = [k for k in kset if k <= K] or [K]

    agg = repeated(Qb, C, kset, args.frac, args.repeats, args.seed)
    vd, why = verdict(agg, kset, K, args.margin)

    payload = {
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "source": args.matrix, "models": names, "n_prompts": N, "n_models": K,
        "theta": args.theta, "kset": kset, "frac": args.frac, "repeats": args.repeats,
        "agg": agg, "verdict": vd, "why": why,
    }
    stamp = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    base = f"{args.out_dir}/EFE_SWARM_S1_{stamp}"
    with open(f"{base}.json", "w", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False, indent=2)
    _report(f"{base}.md", payload)
    _console(payload)
    print(f"\n✅ Rapport : {base}.md  ·  données : {base}.json")
    return 0


def _pp(t):
    return f"{t[0]:.3f} ± {t[1]:.3f}"


def _c(t):
    return f"{t[0]:.4g} ± {t[1]:.2g}"


def _rows(p):
    a = p["agg"]
    order = ["mono_cheap", "mono_best", "mono_premium"]
    order += [f"vote{k}" for k in p["kset"]] + [f"or{k}" for k in p["kset"]]
    seen = set()
    rows = []
    for key in order:
        if key in a and key not in seen:
            seen.add(key)
            rows.append((key, a[key]["acc"], a[key]["cost"]))
    return rows


def _console(p):
    print("═" * 74)
    print(" H-SWARM (Lot S1) — essaim de petits modèles vs monolithe  (moy ± σ)")
    print("═" * 74)
    print(f" Source {p['source'].split('/')[-1]} · {p['n_prompts']} prompts · {p['n_models']} modèles · "
          f"θ={p['theta']} · {p['repeats']} splits")
    print("─" * 74)
    print(f" {'politique':<13} {'accuracy':>16} {'coût/prompt':>20}")
    for key, acc, cost in _rows(p):
        print(f" {key:<13} {_pp(acc):>16} {_c(cost):>20}")
    print("─" * 74)
    print(f" VERDICT : {p['verdict']}")
    print(f"   → {p['why']}")
    print("═" * 74)


def _report(path, p):
    L = [
        f"# EFE H-SWARM (Lot S1) — essaim de petits modèles vs monolithe",
        "",
        f"> Gate scientifique central de la roadmap (Phase 0). Sur **{p['repeats']} splits** "
        f"(moy ± σ). {p['n_prompts']} prompts × {p['n_models']} modèles, source `{p['source']}`. "
        f"Qualité binarisée à θ={p['theta']}. Généré {p['generated_utc']}.",
        "",
        "## Verdict",
        "",
        f"**{p['verdict']}** — {p['why']}",
        "",
        "## Frontière accuracy-coût (moyenne ± σ)",
        "",
        "| politique | accuracy | coût/prompt |",
        "|---|---|---|",
    ]
    for key, acc, cost in _rows(p):
        L.append(f"| {key} | {_pp(acc)} | {_c(cost)} |")
    L += [
        "",
        "## Comment lire",
        "",
        "- **mono_best** = meilleur modèle UNIQUE (accuracy max) = le monolithe à battre.",
        "- **voteK** = vote majoritaire des K modèles les moins chers (essaim réaliste, self-consistency).",
        "- **orK** = correct si AU MOINS un des K corrects (plafond : agrégateur/vérificateur parfait).",
        "- coût d'un essaim K = **somme** des K modèles (on les exécute tous, en parallèle).",
        "- **GO** ⇔ un voteK atteint la qualité de mono_best **à coût inférieur**, robuste (±σ).",
        "",
        "## ⚠️ Portée",
        "",
        "Ce proxy teste la forme la plus FAIBLE d'essaim (ensemble indépendant + agrégation, même "
        "prompt, sans passage d'infos inter-agent). Il NE teste PAS le substrat parallèle (→ test "
        "d'invariant CAS `swarm_cas_invariant.rs`) ni la décomposition/émergence. C'est le gate le "
        "moins cher : NO-GO = signal négatif fort ; GO = motive les topologies riches (Phase 2). "
        "Latence non fournie par RouterBench ; en réel l'essaim s'exécute en parallèle (≈ latence du "
        "plus lent des K, souvent < monolithe).",
        "",
    ]
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(L))


if __name__ == "__main__":
    raise SystemExit(main())
