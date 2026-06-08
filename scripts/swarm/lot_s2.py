#!/usr/bin/env python3
# SPDX-License-Identifier: UNLICENSED
"""lot_s2.py — Phase 2, gate : T1 Generate-Verify (vérificateur a-posteriori) vs MONOLITHE,
sur 3 FAMILLES de tâches. Tranche la question ouverte d'ADR-SWARM-001.

TOPOLOGIE T1 testée : K générateurs bon marché produisent des candidats EN PARALLÈLE ; un
VÉRIFICATEUR a-posteriori LIT les candidats et sélectionne. On la compare au monolithe unique.

3 FAMILLES (le vérificateur diffère selon la vérifiabilité) :
  F1 vérifiable_facile  : vérificateur DÉTERMINISTE (v=1.0) → atteint le plafond OR.
  F2 vérifiable_dur     : idem, mais tâches plus dures (OR plus bas).
  F3 non_vérifiable     : vérificateur APPROXIMATIF (juge LLM, précision v<1) → entre vote et OR.

POLITIQUES : single (1 cheap), vote (majorité), verifier (T1), OR (plafond), monolith (à battre).
Coût : K appels cheap vs 1 appel monolithe. Rigueur : bootstrap sur les tâches (moy ± σ).

⚠ PORTÉE : générateurs FAILLIBLES SIMULÉS (modèle honnête des petits modèles corrélés —
mêmes hypothèses que S1/AGG). Le vérificateur DÉTERMINISTE (F1/F2) rend le résultat
LOGIQUEMENT certain (verifier == OR par construction). Le run pleinement RÉEL (texte de
réponses) = `swarm_slm.rs` + llama-server + un CandidateVerifier réel (cf. ADR-SWARM-002).

Usage :
  ./lot_s2.py --k 7 --tasks 2000 --boot 200
"""
from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone

import numpy as np


def simulate_family(name, rng, k, tasks, diff_lo, diff_hi, verifier_v,
                    cheap_lo=0.45, cheap_hi=0.55, gamma=0.6,
                    mono_skill=0.90, mono_gamma=0.30, cost_cheap=1.0, cost_mono=20.0):
    """Simule une famille. Renvoie un dict de tableaux booléens (tasks,) par politique + coûts."""
    d = rng.uniform(diff_lo, diff_hi, tasks)                      # difficulté par tâche
    skills = np.linspace(cheap_lo, cheap_hi, k)                   # skills hétérogènes des cheap

    # correctness des K candidats : P(correct) = clip(skill - gamma*difficulté).
    p = np.clip(skills[:, None] - gamma * d[None, :], 0.0, 1.0)   # (K, tasks)
    correct = rng.random((k, tasks)) < p                         # (K, tasks)

    # valeurs des candidats (pour le vote) : 0=correct ; sinon leurre commun (1) corrélé, sinon bruit.
    vals = np.zeros((k, tasks), dtype=int)
    wrong = ~correct
    lure = rng.random((k, tasks)) < 0.6
    noise = rng.integers(2, 6, (k, tasks))
    vals[wrong & lure] = 1
    vals[wrong & ~lure] = noise[wrong & ~lure]

    # vote majoritaire par tâche (mode des valeurs == 0 ?).
    vote_correct = np.zeros(tasks, dtype=bool)
    for m in range(tasks):
        v, c = np.unique(vals[:, m], return_counts=True)
        vote_correct[m] = v[np.argmax(c)] == 0

    or_correct = correct.any(axis=0)                             # plafond
    single_correct = correct[0, :]                              # un cheap seul

    # vérificateur a-posteriori de précision v : si un correct existe, le trouve avec prob v ;
    # sinon retombe sur le vote.
    u = rng.random(tasks)
    verifier_correct = np.where(or_correct & (u < verifier_v), True, vote_correct)

    # monolithe : un seul candidat fort.
    p_mono = np.clip(mono_skill - mono_gamma * d, 0.0, 1.0)
    mono_correct = rng.random(tasks) < p_mono

    return {
        "name": name,
        "single": single_correct, "vote": vote_correct, "verifier": verifier_correct,
        "or": or_correct, "monolith": mono_correct,
        "cost_swarm": cost_cheap * k, "cost_mono": cost_mono,
        "verifier_v": verifier_v,
    }


def boot_stats(fam, boot, rng):
    """Bootstrap sur les tâches → moy ± σ par politique."""
    n = len(fam["single"])
    out = {}
    for pol in ("single", "vote", "verifier", "or", "monolith"):
        arr = fam[pol]
        means = np.empty(boot)
        for b in range(boot):
            idx = rng.integers(0, n, n)
            means[b] = arr[idx].mean()
        out[pol] = (float(means.mean()), float(means.std()))
    return out


def verdict_family(stats, fam):
    """T1 (verifier) bat-il le monolithe à coût inférieur, robustement (±σ) ?"""
    vmu, vsd = stats["verifier"]
    mmu, msd = stats["monolith"]
    cheaper = fam["cost_swarm"] < fam["cost_mono"]
    robust_win = (vmu - vsd) >= (mmu - msd) and cheaper
    if robust_win:
        ratio = fam["cost_mono"] / fam["cost_swarm"]
        return "GO", f"verifier {vmu:.3f} ≥ monolithe {mmu:.3f} à {ratio:.1f}× moins cher"
    if stats["or"][0] < mmu:
        return "NO-GO", f"plafond OR {stats['or'][0]:.3f} < monolithe {mmu:.3f} : capacité insuffisante"
    return "INCONCLUS", f"verifier {vmu:.3f} < monolithe {mmu:.3f} (vérificateur trop faible : v={fam['verifier_v']})"


def main() -> int:
    ap = argparse.ArgumentParser(description="Lot S2 — T1 Generate-Verify vs monolithe (3 familles)")
    ap.add_argument("--k", type=int, default=7, help="nb de générateurs cheap (taille essaim)")
    ap.add_argument("--tasks", type=int, default=2000)
    ap.add_argument("--boot", type=int, default=200)
    ap.add_argument("--judge-v", type=float, default=0.85, help="précision du juge (famille non-vérifiable)")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--out-dir", default="scripts/swarm")
    args = ap.parse_args()

    rng = np.random.default_rng(args.seed)
    families = [
        simulate_family("F1_verifiable_facile", rng, args.k, args.tasks, 0.10, 0.55, verifier_v=1.0),
        simulate_family("F2_verifiable_dur", rng, args.k, args.tasks, 0.40, 0.90, verifier_v=1.0),
        simulate_family("F3_non_verifiable", rng, args.k, args.tasks, 0.20, 0.70, verifier_v=args.judge_v),
    ]

    results = []
    for fam in families:
        stats = boot_stats(fam, args.boot, rng)
        vd, why = verdict_family(stats, fam)
        results.append({"family": fam["name"], "verifier_v": fam["verifier_v"],
                        "cost_swarm": fam["cost_swarm"], "cost_mono": fam["cost_mono"],
                        "stats": stats, "verdict": vd, "why": why})

    n_go = sum(1 for r in results if r["verdict"] == "GO")
    overall = ("GO" if n_go >= 1 else "NO-GO")
    payload = {
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "k": args.k, "tasks": args.tasks, "boot": args.boot, "judge_v": args.judge_v,
        "families": results, "families_go": n_go, "overall": overall,
        "_note": "Générateurs faillibles SIMULÉS (modèle honnête des petits modèles corrélés). "
                 "Vérificateur déterministe (F1/F2) → verifier==OR par construction. Run réel "
                 "(texte de réponses) = swarm_slm.rs + llama-server + CandidateVerifier (ADR-SWARM-002).",
    }
    stamp = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    base = f"{args.out_dir}/EFE_SWARM_S2_{stamp}"
    with open(f"{base}.json", "w", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False, indent=2)
    _report(f"{base}.md", payload)
    _console(payload)
    print(f"\n✅ Rapport : {base}.md  ·  données : {base}.json")
    return 0


def _pp(t):
    return f"{t[0]:.3f}±{t[1]:.3f}"


def _console(p):
    print("═" * 88)
    print(" LOT S2 — T1 Generate-Verify vs monolithe · 3 familles · bootstrap (moy ± σ)")
    print("═" * 88)
    print(f" K={p['k']} générateurs · {p['tasks']} tâches/famille · {p['boot']} bootstraps · juge v={p['judge_v']}")
    print("─" * 88)
    print(f" {'famille':<22} {'single':>12} {'vote':>12} {'verifier':>12} {'OR':>12} {'monolith':>12}")
    for r in p["families"]:
        s = r["stats"]
        print(f" {r['family']:<22} {_pp(s['single']):>12} {_pp(s['vote']):>12} "
              f"{_pp(s['verifier']):>12} {_pp(s['or']):>12} {_pp(s['monolith']):>12}")
    print("─" * 88)
    for r in p["families"]:
        ratio = r["cost_mono"] / r["cost_swarm"]
        print(f" {r['family']:<22} coût essaim {r['cost_swarm']:.0f} vs mono {r['cost_mono']:.0f} "
              f"({ratio:.1f}× moins cher) · {r['verdict']} — {r['why']}")
    print("─" * 88)
    print(f" OVERALL : {p['overall']}  ({p['families_go']}/3 familles GO)")
    print("═" * 88)


def _report(path, p):
    L = [
        "# EFE Lot S2 — T1 Generate-Verify vs monolithe (3 familles)",
        "",
        f"> Gate de Phase 2 (ADR-SWARM-002). K={p['k']} générateurs bon marché + vérificateur "
        f"a-posteriori vs monolithe unique. {p['tasks']} tâches/famille, {p['boot']} bootstraps "
        f"(moy ± σ). Généré {p['generated_utc']}.",
        "",
        f"**OVERALL : {p['overall']}** — {p['families_go']}/3 familles GO.",
        "",
        "## Frontière accuracy-coût par famille",
        "",
        "| famille | single | vote | **verifier (T1)** | OR (plafond) | monolithe | coût | verdict |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for r in p["families"]:
        s = r["stats"]
        ratio = r["cost_mono"] / r["cost_swarm"]
        L.append(f"| {r['family']} | {_pp(s['single'])} | {_pp(s['vote'])} | **{_pp(s['verifier'])}** | "
                 f"{_pp(s['or'])} | {_pp(s['monolith'])} | {ratio:.1f}× moins cher | {r['verdict']} |")
    L += [
        "",
        "## Comment lire",
        "",
        "- **verifier (T1)** = K générateurs cheap ∥ + vérificateur a-posteriori (le pari AGG).",
        "- **F1/F2** : vérificateur DÉTERMINISTE (v=1) → `verifier == OR` PAR CONSTRUCTION (tâche "
        "vérifiable : on peut checker un candidat). Le GO y est logiquement certain SI OR > monolithe.",
        "- **F3** : vérificateur APPROXIMATIF (juge v<1) → `verifier ≈ v·OR + (1−v)·vote`. C'est le "
        "cas qui exige une MESURE réelle du juge.",
        "",
        "## ⚠️ Portée & run réel",
        "",
        "Générateurs faillibles SIMULÉS (modèle honnête des petits modèles corrélés, mêmes "
        "hypothèses que S1/AGG). Pour le verdict pleinement réel (texte de réponses + vrai juge LLM), "
        "rejouer T1 avec `swarm_slm.rs` (génération) + `swarm_verifier.rs` `CandidateVerifier` + "
        "`EFE_SWARM_LLM=llama` (llama-server). Sur tâches vérifiables, un vérificateur déterministe "
        "(`efe-gold-test-gate`) atteint réellement OR.",
        "",
    ]
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(L))


if __name__ == "__main__":
    raise SystemExit(main())
