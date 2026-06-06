#!/usr/bin/env python3
# SPDX-License-Identifier: UNLICENSED
"""route_eval.py — EFE P6 R0 : le routage kNN bat-il le routage NAIF ?

Question (ADR-P6-001, post-pivot) : un routeur qui classe le prompt (embedding + kNN)
et choisit le modele en consequence obtient-il une meilleure FRONTIERE cout-qualite que
les baselines naives (always-premium, always-cheap, random) ? Sa valeur = la fraction des
economies de l'ORACLE qu'il capture A ISO-QUALITE.

PRINCIPE (RouterBench-style) : separer le COUTEUX du GRATUIT.
  - Mesurer la qualite de K modeles x N prompts = couteux -> on le fait UNE fois -> une MATRICE
    Q[prompt, modele] + cout[modele] + latence[modele].
  - Evaluer une POLITIQUE = gratuit : on regarde quel modele elle choisit par prompt et on
    somme cout/latence/qualite depuis la matrice. Toutes les politiques, tous les splits -> $0.

DEUX SOURCES DE MATRICE :
  - `--matrix <fichier.json>` : matrice REELLE (RouterBench-like). -> VERDICT reel.
  - `--synth` (defaut) : matrice SYNTHETIQUE controlee. ⚠ valide la MECANIQUE + montre QUAND le
    routage a de la valeur (sensibilite au gap de qualite cheap<->premium). PAS un verdict reel.

Le routeur kNN est APPRIS : sur le train, chaque prompt recoit le choix de l'oracle (modele le
moins cher a iso-qualite) ; un kNN sur les features generalise cette politique au held-out (il ne
voit QUE les features, jamais la qualite du held-out). Rigueur : splits repetes (moy ± sigma).

Usage :
  ./route_eval.py --synth --repeats 30
  ./route_eval.py --synth --sweep-gap            # sensibilite : routage utile SI gap de qualite
  ./route_eval.py --matrix scripts/routing/example_matrix.json --repeats 30   # matrice reelle
"""
from __future__ import annotations

import argparse
import json
from dataclasses import dataclass
from datetime import datetime, timezone

import numpy as np
from sklearn.neighbors import KNeighborsClassifier


# ─────────────────────────────────────────────────────────────────────────
@dataclass
class Matrix:
    features: np.ndarray   # (N, D) — features prompt (pour le kNN)
    quality: np.ndarray    # (N, K) — qualite par modele dans [0,1]
    cost: np.ndarray       # (K,)   — cout relatif par modele
    latency: np.ndarray    # (K,)   — latence par modele
    names: list[str]       # (K,)
    synthetic: bool = False
    cheap_cap: float = 0.0  # (synth) capacite du modele cheap — le bouton de sensibilite


# ─────────────────────────────────────────────────────────────────────────
# Generateur synthetique controle.
# ─────────────────────────────────────────────────────────────────────────
def synth_matrix(n, seed, caps, gap, noise, n_classes=6, feat_noise=0.5) -> Matrix:
    """K modeles de capacite croissante (cheap..premium). Chaque prompt a une CLASSE et une
    DIFFICULTE (certaines classes plus dures). Q[p,m] = sigmoid(8*(cap_m - gap*diff_p)) + bruit :
    le premium reste haut partout ; le cheap CHUTE sur les prompts durs -> il existe un gap de
    qualite, donc une valeur de routage POTENTIELLE. `caps[0]` (capacite cheap) = le bouton qui
    fait apparaitre ou disparaitre cette valeur. Features = signal classe+difficulte bruite."""
    rng = np.random.default_rng(seed)
    cls = rng.integers(0, n_classes, n)
    class_diff = np.linspace(0.15, 0.85, n_classes)
    class_diff = class_diff[rng.permutation(n_classes)]  # difficulte assignee aux classes
    diff = np.clip(class_diff[cls] + rng.normal(0, 0.12, n), 0, 1)

    K = len(caps)
    Q = np.zeros((n, K))
    for m, cap in enumerate(caps):
        logit = 8.0 * (cap - gap * diff)
        Q[:, m] = np.clip(1.0 / (1.0 + np.exp(-logit)) + rng.normal(0, noise, n), 0.0, 1.0)

    cost = np.array([1.0, 5.0, 20.0])[:K]      # ratios ~ Haiku/Sonnet/Opus
    latency = np.array([300.0, 800.0, 2000.0])[:K]

    F = np.zeros((n, n_classes + 1))
    F[np.arange(n), cls] = 1.0
    F[:, -1] = diff
    F = F + rng.normal(0, feat_noise, F.shape)
    F = F / np.maximum(np.linalg.norm(F, axis=1, keepdims=True), 1e-9)

    names = ["cheap", "mid", "premium"][:K]
    return Matrix(F, Q, cost, latency, names, synthetic=True, cheap_cap=caps[0])


# ─────────────────────────────────────────────────────────────────────────
# Chargeur de matrice REELLE (RouterBench-like).
# Format JSON : {model_names, cost, latency_ms, prompts:[{id, text|features, quality:[...]}]}.
# ─────────────────────────────────────────────────────────────────────────
def load_matrix(path) -> Matrix:
    with open(path, encoding="utf-8") as f:
        d = json.load(f)
    names = list(d["model_names"])
    latency = np.asarray(d.get("latency_ms", [0.0] * len(names)), dtype=float)
    prompts = d["prompts"]
    Q = np.asarray([p["quality"] for p in prompts], dtype=float)
    # Cout : par-prompt (prompts[].cost -> (N,K)) si present, sinon par-modele (top-level -> (K,)).
    if all("cost" in p for p in prompts):
        cost = np.asarray([p["cost"] for p in prompts], dtype=float)
    else:
        cost = np.asarray(d["cost"], dtype=float)
    # Features : fournies, sinon TF-IDF sur le texte.
    if all("features" in p for p in prompts):
        F = np.asarray([p["features"] for p in prompts], dtype=float)
    else:
        from sklearn.feature_extraction.text import TfidfVectorizer
        F = TfidfVectorizer(min_df=1).fit_transform([p.get("text", "") for p in prompts]).toarray().astype(float)
    F = F / np.maximum(np.linalg.norm(F, axis=1, keepdims=True), 1e-9)
    # Ordonne les modeles par cout CROISSANT -> index 0 = cheap, K-1 = premium (robustesse policies).
    mean_cost = cost.mean(axis=0) if cost.ndim == 2 else cost
    order = np.argsort(mean_cost)
    names = [names[i] for i in order]
    Q = Q[:, order]
    cost = cost[:, order] if cost.ndim == 2 else cost[order]
    latency = latency[order]
    return Matrix(F, Q, cost, latency, names, synthetic=False)


# ─────────────────────────────────────────────────────────────────────────
# Politiques de routage.
# ─────────────────────────────────────────────────────────────────────────
def oracle_choice(Q, cost, tol):
    """Par prompt : le modele le MOINS CHER dont la qualite est a `tol` du max (iso-qualite).
    `cost` accepte (K,) [cout par-modele] OU (N,K) [cout par-(prompt,modele) — fidelite RouterBench]."""
    best = Q.max(axis=1, keepdims=True)
    ok = Q >= (best - tol)
    C = cost if cost.ndim == 2 else np.broadcast_to(cost, Q.shape)
    return np.where(ok, C, np.inf).argmin(axis=1)


def eval_route(route, Q, cost, latency):
    idx = np.arange(len(route))
    c = cost[idx, route] if cost.ndim == 2 else cost[route]
    return {
        "cost": float(np.asarray(c).mean()),
        "quality": float(Q[idx, route].mean()),
        "latency": float(latency[route].mean()),
    }


def run_once(M: Matrix, tol, k, eval_frac, seed):
    n, K = M.quality.shape
    rng = np.random.default_rng(seed)
    perm = rng.permutation(n)
    ne = max(1, int(round(n * eval_frac)))
    ev, tr = perm[:ne], perm[ne:]

    Qe, Fe = M.quality[ev], M.features[ev]
    Ce = M.cost[ev] if M.cost.ndim == 2 else M.cost     # cout eval (par-prompt si 2D)
    Ctr = M.cost[tr] if M.cost.ndim == 2 else M.cost     # cout train (pour les labels oracle)
    premium = K - 1
    out = {}
    out["premium"] = eval_route(np.full(len(ev), premium), Qe, Ce, M.latency)
    out["cheap"] = eval_route(np.zeros(len(ev), int), Qe, Ce, M.latency)
    out["random"] = eval_route(rng.integers(0, K, len(ev)), Qe, Ce, M.latency)
    out["oracle"] = eval_route(oracle_choice(Qe, Ce, tol), Qe, Ce, M.latency)

    # kNN appris : labels oracle sur le TRAIN, generalises par kNN sur features.
    lbl_tr = oracle_choice(M.quality[tr], Ctr, tol)
    if len(np.unique(lbl_tr)) < 2:
        route_knn = np.full(len(ev), int(lbl_tr[0]))  # degenere : un seul modele optimal
    else:
        kk = min(k, len(tr))
        clf = KNeighborsClassifier(n_neighbors=kk, weights="distance").fit(M.features[tr], lbl_tr)
        route_knn = clf.predict(Fe)
    out["knn"] = eval_route(route_knn, Qe, Ce, M.latency)

    # Metriques de valeur (a iso-qualite vs premium).
    cprem, cor, cknn = out["premium"]["cost"], out["oracle"]["cost"], out["knn"]["cost"]
    orac_sav = cprem - cor
    out["_capture"] = ((cprem - cknn) / orac_sav) if orac_sav > 1e-9 else float("nan")
    out["_dq_knn_vs_prem"] = out["knn"]["quality"] - out["premium"]["quality"]
    out["_dq_cheap_vs_prem"] = out["cheap"]["quality"] - out["premium"]["quality"]
    out["_save_knn_pct"] = (cprem - cknn) / cprem if cprem > 0 else 0.0
    # Capture de QUALITE : fraction de l'amelioration qualite de l'oracle (vs cheap) que le kNN
    # realise. C'est LE metric quand le routeur peut tricher en routant cheap (cout bas, qualite basse).
    qor, qch, qkn = out["oracle"]["quality"], out["cheap"]["quality"], out["knn"]["quality"]
    out["_qcapture"] = ((qkn - qch) / (qor - qch)) if (qor - qch) > 1e-6 else float("nan")
    return out


def repeated(M, tol, k, eval_frac, repeats, base_seed):
    runs = [run_once(M, tol, k, eval_frac, base_seed + r) for r in range(repeats)]
    agg = {}
    pol_keys = ["premium", "cheap", "random", "oracle", "knn"]
    for p in pol_keys:
        agg[p] = {}
        for metric in ("cost", "quality", "latency"):
            v = np.array([r[p][metric] for r in runs])
            agg[p][metric] = (float(v.mean()), float(v.std()))
    for key in ("_capture", "_qcapture", "_dq_knn_vs_prem", "_dq_cheap_vs_prem", "_save_knn_pct"):
        v = np.array([r[key] for r in runs], dtype=float)
        v = v[~np.isnan(v)]
        agg[key] = (float(v.mean()), float(v.std())) if len(v) else (float("nan"), 0.0)
    return agg


# ─────────────────────────────────────────────────────────────────────────
def verdict(M, agg, q_floor):
    if M.synthetic:
        return "SYNTHETIQUE — valide la mecanique + sensibilite ; verdict reel = --matrix <vraie matrice>"
    qcap_mu, qcap_sd = agg["_qcapture"]      # capture de la VALEUR-QUALITE de l'oracle (le bon metric)
    dqc_mu = agg["_dq_cheap_vs_prem"][0]
    if dqc_mu > -q_floor:
        return "NO-GO — le cheap suffit partout (qualite ≈ premium) : routage inutile -> P6 = P1"
    if qcap_mu - qcap_sd > 0.6:
        return "GO — le kNN capte une part ROBUSTE de la valeur-qualite de l'oracle a bas cout"
    if qcap_mu + qcap_sd < 0.35:
        return ("NO-GO — la valeur EXISTE (oracle >> cheap) mais le kNN ne la capte PAS : "
                "le ROUTEUR (embedding -> modele) est le goulot, pas l'opportunite")
    return "INCONCLUS — capture de qualite intermediaire : ameliorer le routeur (features) ou la matrice"


def main() -> int:
    ap = argparse.ArgumentParser(description="P6 R0 — routage kNN vs naif (frontiere cout-qualite)")
    src = ap.add_mutually_exclusive_group()
    src.add_argument("--synth", action="store_true", help="matrice synthetique (defaut)")
    src.add_argument("--matrix", help="matrice reelle JSON (RouterBench-like) -> verdict reel")
    ap.add_argument("--n", type=int, default=400, help="(synth) nb de prompts")
    ap.add_argument("--caps", default="0.55,0.78,0.97", help="(synth) capacites cheap,mid,premium")
    ap.add_argument("--gap", type=float, default=0.6, help="(synth) impact de la difficulte sur la qualite")
    ap.add_argument("--noise", type=float, default=0.05)
    ap.add_argument("--quality-tol", type=float, default=0.02, help="iso-qualite (tolerance vs max)")
    ap.add_argument("--k", type=int, default=15, help="voisins du kNN-routeur")
    ap.add_argument("--eval-frac", type=float, default=0.4)
    ap.add_argument("--repeats", type=int, default=30)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--sweep-gap", action="store_true", help="sensibilite : varie la capacite cheap")
    ap.add_argument("--out-dir", default="scripts/routing")
    args = ap.parse_args()

    caps = tuple(float(x) for x in args.caps.split(","))

    def make(cheap_cap=None):
        if args.matrix:
            return load_matrix(args.matrix)
        c = (cheap_cap, caps[1], caps[2]) if cheap_cap is not None else caps
        return synth_matrix(args.n, args.seed, c, args.gap, args.noise)

    # ── Mode SENSIBILITE : varie la capacite du cheap -> quand le routage a-t-il de la valeur ? ──
    if args.sweep_gap:
        print("═" * 76)
        print(" P6 R0 — SENSIBILITE : valeur du routage selon la capacite du modele CHEAP")
        print("═" * 76)
        print(f" {'cheap_cap':>9} {'Q_cheap':>8} {'Q_prem':>7} {'Q_knn':>7} {'VALEUR routage':>16} {'econ.kNN':>9}")
        print(f" {'':>9} {'':>8} {'':>7} {'':>7} {'(Qknn-Qcheap)':>16}")
        print("─" * 76)
        rows = []
        for cc in (0.50, 0.60, 0.70, 0.80, 0.90, 0.95):
            M = make(cc)
            agg = repeated(M, args.quality_tol, args.k, args.eval_frac, args.repeats, args.seed)
            qc, qp, qk = agg["cheap"]["quality"][0], agg["premium"]["quality"][0], agg["knn"]["quality"][0]
            sav = agg["_save_knn_pct"]; rv = qk - qc
            print(f" {cc:>9.2f} {qc:>8.2f} {qp:>7.2f} {qk:>7.2f} {rv:>+14.3f}   {sav[0]:>7.0%}")
            rows.append({"cheap_cap": cc, "q_cheap": qc, "q_premium": qp, "q_knn": qk,
                         "routing_value": rv, "capture": agg["_capture"], "save_knn": sav})
        print("─" * 76)
        print(" Lecture : VALEUR routage = qualité que le routeur RÉCUPÈRE vs tout-cheap.")
        print("           Elle s'EFFONDRE quand le cheap devient bon (Q_cheap -> Q_premium)")
        print("           -> always-cheap suffit. Le routage ne paie QUE s'il existe un GAP.")
        print("═" * 76)
        stamp = datetime.now(timezone.utc).strftime("%Y-%m-%d")
        with open(f"{args.out_dir}/EFE_ROUTE_R0_SWEEP_{stamp}.json", "w", encoding="utf-8") as f:
            json.dump({"sweep": rows, "gap": args.gap, "repeats": args.repeats}, f, ensure_ascii=False, indent=2)
        return 0

    # ── Mode standard ──
    M = make()
    agg = repeated(M, args.quality_tol, args.k, args.eval_frac, args.repeats, args.seed)
    vd = verdict(M, agg, args.quality_tol)
    payload = {
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "source": "synthetique" if M.synthetic else f"matrice:{args.matrix}",
        "models": M.names,
        "cost": (M.cost.mean(axis=0) if M.cost.ndim == 2 else M.cost).tolist(),
        "cost_per_prompt": bool(M.cost.ndim == 2), "latency": M.latency.tolist(),
        "n_prompts": int(M.quality.shape[0]), "repeats": args.repeats, "quality_tol": args.quality_tol,
        "agg": agg, "verdict": vd,
    }
    stamp = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    base = f"{args.out_dir}/EFE_ROUTE_R0_{stamp}"
    with open(f"{base}.json", "w", encoding="utf-8") as f:
        json.dump(payload, f, ensure_ascii=False, indent=2)
    _report(f"{base}.md", payload)
    _console(payload)
    print(f"\n✅ Rapport : {base}.md  ·  donnees : {base}.json")
    return 0


def _pp(t, pct=False):
    return (f"{t[0]:.0%} ± {t[1]:.0%}" if pct else f"{t[0]:.2f} ± {t[1]:.2f}")


def _c(t):  # cout : sig figs (gere 1e-4 comme 20)
    return f"{t[0]:.4g} ± {t[1]:.2g}"


def _console(p):
    a = p["agg"]
    print("═" * 72)
    print(" P6 R0 — routage kNN vs naïf  (frontière coût-qualité, moy ± σ)")
    print("═" * 72)
    cmin, cmax = min(p["cost"]), max(p["cost"])
    print(f" Source {p['source']} · {p['n_prompts']} prompts · {len(p['models'])} modeles "
          f"(cout/req {'par-prompt' if p.get('cost_per_prompt') else 'par-modele'} ~[{cmin:.4g}, {cmax:.4g}])")
    print("─" * 72)
    print(f" {'politique':<10} {'coût/req':>18} {'qualité':>14}")
    for pol in ("premium", "cheap", "random", "knn", "oracle"):
        print(f" {pol:<10} {_c(a[pol]['cost']):>18} {_pp(a[pol]['quality']):>14}")
    print("─" * 72)
    print(f" Capture VALEUR-QUALITÉ de l'oracle (kNN) : {a['_qcapture'][0]:+.0%} ± {a['_qcapture'][1]:.0%}   ← décide")
    print(f" Économie coût kNN vs always-premium      : {a['_save_knn_pct'][0]:.0%}")
    print(f" Qualité : cheap {a['cheap']['quality'][0]:.2f} → kNN {a['knn']['quality'][0]:.2f} → "
          f"premium {a['premium']['quality'][0]:.2f} → oracle {a['oracle']['quality'][0]:.2f}")
    print("─" * 72)
    print(f" VERDICT : {p['verdict']}")
    print("═" * 72)


def _report(path, p):
    a = p["agg"]
    L = [
        f"# EFE P6 R0 — routage kNN vs naïf · {p['source']}",
        "",
        f"> Frontière coût-qualité sur **{p['repeats']} splits** (moy ± σ). La valeur de routage = "
        f"la **part des économies de l'oracle capturée à iso-qualité**. {p['n_prompts']} prompts ; "
        f"modèles {p['models']} (coûts {p['cost']}). Généré {p['generated_utc']}.",
        "",
        "## Verdict",
        "",
        f"**{p['verdict']}**",
        "",
        f"- **Capture de la VALEUR-QUALITÉ de l'oracle (kNN) : {a['_qcapture'][0]:+.0%} ± {a['_qcapture'][1]:.0%}** ← décide "
        "(fraction de l'amélioration qualité oracle vs cheap que le kNN réalise).",
        f"- Économie coût kNN vs always-premium : **{a['_save_knn_pct'][0]:.0%}**.",
        f"- Qualité : cheap {a['cheap']['quality'][0]:.2f} → kNN {a['knn']['quality'][0]:.2f} → "
        f"premium {a['premium']['quality'][0]:.2f} → oracle {a['oracle']['quality'][0]:.2f}.",
        "",
        "## Frontière coût-qualité (moyenne ± σ)",
        "",
        "| politique | coût/req | qualité |",
        "|---|---|---|",
    ]
    for pol in ("always-premium", "always-cheap", "random", "kNN (appris)", "oracle (plafond)"):
        key = {"always-premium": "premium", "always-cheap": "cheap", "random": "random",
               "kNN (appris)": "knn", "oracle (plafond)": "oracle"}[pol]
        L.append(f"| {pol} | {_c(a[key]['cost'])} | {_pp(a[key]['quality'])} |")
    L += [
        "",
        "## Comment lire",
        "",
        "- **oracle** = plafond (modèle le moins cher à iso-qualité, par prompt) → ses économies sont le max atteignable.",
        "- **kNN** doit s'en approcher **sans perdre de qualité** (Δqualité vs premium ≈ 0) ET battre **always-cheap** en qualité.",
        "- Si **always-cheap ≈ premium** en qualité (Δ ~ 0), le cheap suffit → **routage inutile** → P6 = P1.",
        "- ⚠️ Source **synthétique** = mécanique + sensibilité, PAS un verdict réel. Verdict : `--matrix <vraie matrice>` "
        "(RouterBench export ou matrice custom Phase 1).",
        "",
    ]
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(L))


if __name__ == "__main__":
    raise SystemExit(main())
