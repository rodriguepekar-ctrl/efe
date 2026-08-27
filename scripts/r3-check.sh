#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────
#  R3 — la suite qui crie au loup : REPRODUIRE d'abord.
#
#  La feuille de route du 24 août est, sur ce point précis, plus honnête
#  que sur tout le reste :
#
#      « Contrairement à tout le reste de ce document, ce constat n'a PAS
#        été re-mesuré. Il vient d'une observation antérieure consignée.
#        Le premier geste de R3 n'est donc pas de corriger, c'est de
#        REPRODUIRE — et si la suite est devenue stable, de RETIRER R3
#        plutôt que de chercher un défaut qui n'existe plus. »
#
#  Ce script est ce premier geste, rendu répétable.
#
#  ## Ce qu'il fait que « relancer les tests » ne fait pas
#
#  Un échec dans une exécution de workspace ne dit pas s'il est RÉEL ou
#  INSTABLE. La distinction est tout le sujet de R3 : un test qui échoue
#  en masse et passe en isolement partage une ressource avec un autre ;
#  un test qui échoue dans les deux cas est une régression. Le script
#  rejoue donc CHAQUE échec seul, et c'est ce second passage qui tranche.
#
#  ## Ce qu'il ne fait pas
#
#  Il ne corrige rien. Le remède que R3 prescrit — `serial_test` avec la
#  ressource partagée nommée en commentaire — demande de savoir QUELLE
#  ressource, et cela ne se devine pas depuis un compte d'échecs.
#
#  Usage :  bash scripts/r3-check.sh            (un tour)
#           R3_TOURS=3 bash scripts/r3-check.sh (le critère complet)
#  Sortie 0 si aucun échec ; 1 si des échecs INSTABLES ; 2 si des échecs
#  REPRODUCTIBLES (une vraie régression, pas le sujet de R3).
# ─────────────────────────────────────────────────────────────────────
set -uo pipefail
cd "$(dirname "$0")/.."

TOURS=${R3_TOURS:-1}
TRAVAIL=$(mktemp -d)
trap 'rm -rf "$TRAVAIL"' EXIT

titre() { printf '\n\033[1;34m%s\033[0m\n' "$1"; }
ok()    { printf '  \033[0;32m✓\033[0m %s\n' "$1"; }
ko()    { printf '  \033[0;31m✗\033[0m %s\n' "$1"; }
att()   { printf '  \033[0;33m!\033[0m %s\n' "$1"; }
note()  { printf '      %s\n' "$1"; }

titre "Reproduction — $TOURS tour(s) de « cargo test --workspace »"
note "le critère de R3 en demande TROIS d'affilée ; R3_TOURS le règle."

for t in $(seq 1 "$TOURS"); do
    printf '  tour %s/%s … ' "$t" "$TOURS"
    cargo test --workspace --no-fail-fast > "$TRAVAIL/tour$t.log" 2>&1
    # Le binaire de test précède ses cas : on garde le contexte pour pouvoir rejouer SEUL.
    # On ne garde QUE le nom du test. La tentation est de retenir aussi le binaire lu sur la
    # ligne « Running » qui précède — c'est faux : avec --workspace, cargo exécute plusieurs
    # binaires EN PARALLÈLE et entrelace leurs sorties, si bien que la ligne « Running » la plus
    # proche d'un échec appartient souvent à un autre crate. Cette erreur a fait déclarer
    # « reproductible » deux tests qui passaient parfaitement seuls — c'est-à-dire l'exact
    # contraire du verdict, sur le seul point que R3 demande de trancher.
    grep -E '^test .* FAILED$' "$TRAVAIL/tour$t.log" | awk '{print $2}' | sort -u \
      > "$TRAVAIL/echecs$t.txt"
    E=$(wc -l < "$TRAVAIL/echecs$t.txt" | tr -d ' ')
    # **Distinguer construire de passer.** Cargo écrit « error: test failed, to rerun pass … »
    # quand des TESTS échouent — ce qui est précisément ce que R3 vient mesurer. Un garde qui
    # confond les deux déclare « rien à reproduire » au moment exact où il y a quelque chose à
    # reproduire. Seuls comptent ici les diagnostics du COMPILATEUR : un code E0000, ou l'aveu
    # « could not compile ».
    if grep -qE '^error\[E[0-9]+\]|could not compile' "$TRAVAIL/tour$t.log"; then
        printf '\n'
        ko "la suite NE COMPILE PAS — rien a reproduire tant que c'est vrai"
        grep -E '^error\[E[0-9]+\]|could not compile' "$TRAVAIL/tour$t.log" | head -6 | sed 's/^/      /'
        note "un « 0 echec » sur une suite qui ne construit pas serait un vert mensonger."
        exit 2
    fi
    # `grep -c` sort 1 quand il ne compte rien, ET imprime déjà « 0 » : un `|| echo 0` en ajoutait
    # un second, et la variable valait « 0\n0 ». Le premier tour de R3 a ainsi affiché « 0 test(s)
    # verts » là où la vérité était « la suite ne compile pas » — un compteur cassé qui ressemble à
    # une mesure est pire qu'une absence de mesure.
    P=$(grep -c '^test .* ok$' "$TRAVAIL/tour$t.log" 2>/dev/null || true)
    printf '%s test(s) verts, %s échec(s)\n' "$P" "$E"
done

cat "$TRAVAIL"/echecs*.txt 2>/dev/null | sort -u > "$TRAVAIL/tous.txt"
TOTAL=$(wc -l < "$TRAVAIL/tous.txt" | tr -d ' ')

if [ "$TOTAL" -eq 0 ]; then
    titre "Verdict"
    ok "aucun échec sur $TOURS tour(s)"
    if [ "$TOURS" -ge 3 ]; then
        printf '\n  \033[0;32mLe critère de R3 est TENU : trois tours sans échec.\n'
        printf '  Sa propre note dit alors quoi faire — RETIRER R3, plutôt que de chercher\n'
        printf '  un défaut qui n'\''existe plus.\033[0m\n\n'
    else
        note "un seul tour ne tient pas le critère : relancer avec R3_TOURS=3."
    fi
    exit 0
fi

titre "Discrimination — chaque échec, rejoué SEUL"
note "c'est ce second passage qui sépare l'instable du cassé."
INSTABLES=0; REELS=0
# D'OÙ vient ce test — trouvé dans la SOURCE, pas deviné dans un journal.
#
# Trois méthodes plus simples ont été essayées et sont fausses :
#   · lire la ligne « Running » qui précède l'échec — cargo entrelace des binaires exécutés en
#     parallèle, la ligne la plus proche appartient souvent à un autre crate ;
#   · `cargo test --workspace -- <nom> --exact` — le code de sortie global dépend de N binaires
#     dont un seul concerne le test : on mesure la santé du workspace, pas celle du test ;
#   · interroger `--list` sur chaque binaire de `target/debug/deps` — des centaines d'exécutables,
#     dont certains ne répondent pas.
#
# La source, elle, dit exactement où la fonction est déclarée, et son chemin donne le crate et la
# cible. C'est le seul index qui ne dépende ni du parallélisme ni de l'état de compilation.
provenance_du_test() {
    local nom="$1" f
    f=$(grep -rl --include='*.rs' -E "fn[[:space:]]+${nom}[[:space:]]*\\(" crates/ 2>/dev/null | head -1)
    [ -z "$f" ] && return 1
    local crate cible
    crate=$(printf '%s' "$f" | cut -d/ -f2)
    case "$f" in
        */tests/*) cible=$(basename "$f" .rs); printf -- '-p %s --test %s' "$crate" "$cible" ;;
        */benches/*) cible=$(basename "$f" .rs); printf -- '-p %s --bench %s' "$crate" "$cible" ;;
        *) printf -- '-p %s --lib' "$crate" ;;
    esac
}

while read -r nom; do
    [ -z "$nom" ] && continue
    CIBLE=$(provenance_du_test "$nom") || { ko "INTROUVABLE — $nom"; note "aucune source ne declare ce nom : la mesure est caduque."; REELS=$((REELS + 1)); continue; }
    note "cible : cargo test $CIBLE"
    # shellcheck disable=SC2086
    if cargo test $CIBLE -- "$nom" --exact --test-threads=1 > "$TRAVAIL/seul.log" 2>&1; then
        att "INSTABLE — $nom"
        note "passe seul, échoue en masse : il partage une ressource. C'est le sujet de R3."
        INSTABLES=$((INSTABLES + 1))
    else
        ko "REPRODUCTIBLE — $nom"
        note "échoue aussi seul : ce n'est PAS de l'instabilité, c'est une régression."
        REELS=$((REELS + 1))
    fi
done < "$TRAVAIL/tous.txt"

titre "Verdict"
note "$INSTABLES instable(s) · $REELS reproductible(s)"
if [ "$REELS" -gt 0 ]; then
    printf '\n  \033[0;31mDes échecs se reproduisent en isolement : traiter la régression AVANT R3.\033[0m\n\n'
    exit 2
fi
printf '\n  \033[0;33mR3 est confirmé : %s test(s) instable(s).\n' "$INSTABLES"
printf '  Le remede prescrit par R3 est serial_test sur chacun, AVEC la ressource partagee\n'
printf '  nommee en commentaire — le depot le fait deja pour la table des gestionnaires de\n'
printf '  signaux (efe-meta-swarm-server/Cargo.toml, PR #12).\n\n'
printf '  MAIS : serial_test serialise DANS UN BINAIRE. Un test qui passe quand son binaire\n'
printf '  tourne seul et tombe sous --workspace est en contention avec dAUTRES PROCESSUS —\n'
printf '  espace des ports ephemeres, systeme de fichiers, charge machine. Sur ce mode de\n'
printf '  defaillance le remede prescrit est INOPERANT, et lappliquer donnerait limpression\n'
printf '  davoir traite le probleme. Mesure du 2026-08-27 : les deux instables observes\n'
printf '  (m1b_pas_de_degradation_silencieuse) sont de cette famille.\033[0m\n\n'
exit 1
