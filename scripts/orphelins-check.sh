#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  R2 — le recensement des orphelins stricts, RENDU RÉPÉTABLE.
#
#  La feuille de route du 24 août demandait de retirer deux crates
#  « que rien n'atteint », et ajoutait la phrase qui compte :
#
#      « Le geste importe moins que sa RÉPÉTABILITÉ. […] Sans cela, la
#        même mesure sera refaite dans six mois par quelqu'un qui croira
#        découvrir quelque chose. »
#
#  Elle avait raison, et pour une raison qu'elle n'avait pas prévue :
#  SES DEUX CRATES ÉTAIENT MAL CLASSÉS. Ce script existe pour que
#  l'erreur ne se refasse pas.
#
#  ## Les deux défauts du recensement d'origine
#
#  1. **La requête était trop étroite.** Elle regardait `crates/*`.
#     `efe-cluster-provisioner` est pourtant une dépendance déclarée de
#     `smoke/runner` et de `bench/harness` — deux membres du workspace
#     qui ne vivent pas sous `crates/` — et un scénario de bench invoque
#     son binaire. Il n'a jamais été orphelin.
#
#  2. **La notion d'« atteint » était trop étroite.** Un harnais
#     d'expérimentation ne se prend pas en dépendance : il s'exécute par
#     `cargo run --example`. `efe-bench-inference` porte 118 tests verts,
#     six exemples exécutables et six comptes rendus de recherche datés.
#     Le supprimer aurait détruit le harnais qui a produit ces résultats,
#     au motif que personne ne l'importe — alors que personne n'importe
#     JAMAIS un banc d'essai.
#
#  ## Ce que ce script mesure, donc
#
#  Trois classes, pas deux. Un membre est ORPHELIN STRICT seulement si
#  aucun autre membre n'en dépend, ET rien ne le cite hors de lui-même,
#  ET il n'offre aucune porte d'entrée (`[[bin]]` ou `examples/`). Les
#  deux premières conditions ne suffisent pas : c'est la troisième qui
#  sépare le code mort du banc d'essai.
#
#  Usage :  bash scripts/orphelins-check.sh
#  Sortie 0 si aucun orphelin strict ; 1 sinon.
# ─────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/.."

printf '\n\033[1;34mRecensement des membres du workspace\033[0m\n'

python3 - <<'PY'
import json, subprocess, pathlib, sys

md = json.loads(subprocess.run(
    ["cargo", "metadata", "--no-deps", "--format-version", "1"],
    capture_output=True, text=True, check=True).stdout)

racine = pathlib.Path(".").resolve()
dossier = {p["name"]: pathlib.Path(p["manifest_path"]).parent for p in md["packages"]}

# Les arêtes de dépendance, TOUTES catégories confondues (normal, dev, build) : un crate qui
# n'est tiré qu'en dev-dependency est atteint — il l'est par la suite de tests, qui est du code
# qu'on exécute.
depends = {d["name"] for p in md["packages"] for d in p["dependencies"] if d["name"] in dossier}

def portes(nom):
    """Les portes d'entrée : un binaire, ou des exemples exécutables."""
    d = dossier[nom]
    manifeste = (d / "Cargo.toml").read_text(encoding="utf-8", errors="replace")
    bins = manifeste.count("[[bin]]")
    ex = len(list((d / "examples").glob("*.rs"))) if (d / "examples").is_dir() else 0
    # `src/main.rs` vaut un binaire même sans section [[bin]] explicite.
    if bins == 0 and (d / "src" / "main.rs").is_file():
        bins = 1
    return bins, ex

def cites(nom):
    """Fichiers qui nomment ce crate, hors de son propre dossier, hors artefacts."""
    rel = str(dossier[nom].relative_to(racine))
    out = subprocess.run(
        ["grep", "-rl", "--include=*.toml", "--include=*.rs", "--include=*.sh",
         "--include=*.yml", "--include=*.yaml", "--include=*.py", "--include=*.md", nom, "."],
        capture_output=True, text=True).stdout.split()
    return [f for f in out
            if not f.startswith("./target")
            and not f.startswith(f"./{rel}/")
            # L'inscription au workspace n'est pas un usage — et CE SCRIPT non plus. Il nomme deux
            # crates pour documenter la réfutation de R2 ; sans cette exclusion il les compterait
            # comme « cités », et masquerait un orphelin réel dont le nom viendrait à figurer ici.
            # Un recenseur qui se compte lui-même finit par ne plus rien recenser.
            and f not in ("./Cargo.lock", "./Cargo.toml", "./scripts/orphelins-check.sh")]

stricts, terminaux = [], []
for nom in sorted(dossier):
    if nom in depends:
        continue
    refs = cites(nom)
    bins, ex = portes(nom)
    if refs:
        continue                       # cité quelque part : atteint
    if bins or ex:
        terminaux.append((nom, bins, ex))
    else:
        stricts.append(nom)

print(f"  {len(dossier)} membres · {len(depends)} pris en dépendance par un autre membre")

if terminaux:
    print("\n  \033[1;33mTerminaux invocables\033[0m — personne n'en dépend, et c'est NORMAL :")
    print("  un banc d'essai ou un outil s'exécute, il ne s'importe pas.")
    for nom, b, e in terminaux:
        quoi = " · ".join(filter(None, [f"{b} binaire(s)" if b else "", f"{e} exemple(s)" if e else ""]))
        print(f"    · {nom:<28} {quoi}")

if stricts:
    print("\n  \033[0;31m✗ ORPHELINS STRICTS\033[0m — aucun dépendant, aucune citation, aucune porte :")
    for nom in stricts:
        print(f"    · {nom}  ({dossier[nom].relative_to(racine)})")
    print("\n  Un crate dans cet état ne peut être atteint par AUCUN chemin. Avant de le")
    print("  retirer, ouvrez-le : s'il porte des tests verts ou des comptes rendus, ce")
    print("  n'est pas du code mort, c'est un recensement trop étroit — celui que R2 a fait.")
    sys.exit(1)

print("\n  \033[0;32m✓ aucun orphelin strict\033[0m")
sys.exit(0)
PY
CODE=$?

printf '\n\033[1;34mCe que R2 affirmait, et ce que la mesure en dit\033[0m\n'
verdict() { printf '  %s %-28s %s\n' "$1" "$2" "$3"; }
if grep -q 'efe-cluster-provisioner' smoke/runner/Cargo.toml bench/harness/Cargo.toml 2>/dev/null; then
    verdict '✗' 'efe-cluster-provisioner' 'PAS orphelin : smoke/runner et bench/harness en dépendent'
else
    verdict '?' 'efe-cluster-provisioner' 'les dépendants attendus ont disparu — reprendre la mesure'
fi
if [ -d crates/efe-bench-inference ]; then
    T=$(grep -rc '#\[test\]\|#\[tokio::test\]' crates/efe-bench-inference/src crates/efe-bench-inference/tests 2>/dev/null | awk -F: '{s+=$2} END {print s+0}')
    E=$(ls crates/efe-bench-inference/examples/*.rs 2>/dev/null | wc -l | tr -d ' ')
    M=$(ls crates/efe-bench-inference/*.md 2>/dev/null | wc -l | tr -d ' ')
    verdict '✗' 'efe-bench-inference' "PAS mort : $T tests, $E exemples, $M comptes rendus de recherche"
else
    verdict '!' 'efe-bench-inference' 'le crate a été SUPPRIMÉ — relire R2 avant de s'\''en réjouir'
fi
printf '\n  \033[0;32mR2 : le recensement est un contrôle, et il refuse les deux suppressions\n  que la feuille de route proposait.\033[0m\n\n'
exit $CODE
