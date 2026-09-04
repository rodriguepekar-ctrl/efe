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
# La borne d'un tour, en secondes. Généreuse par défaut : elle existe pour rendre visible une
# course qui ne finit pas, pas pour arbitrer la vitesse d'une machine.
DELAI="${R3_DELAI_S:-5400}"
TRAVAIL=$(mktemp -d)
trap 'rm -rf "$TRAVAIL"' EXIT

titre() { printf '\n\033[1;34m%s\033[0m\n' "$1"; }
ok()    { printf '  \033[0;32m✓\033[0m %s\n' "$1"; }
ko()    { printf '  \033[0;31m✗\033[0m %s\n' "$1"; }
att()   { printf '  \033[0;33m!\033[0m %s\n' "$1"; }
note()  { printf '      %s\n' "$1"; }

# **Rejouer LE MÊME BINAIRE — pas un binaire rebâti.**
#
# C'est le point sur lequel ce script s'est trompé, et le plus contre-intuitif du lot.
# `cargo test -p <crate>` ne rejoue pas : il RECONSTRUIT. Or Cargo unifie les features par
# caisse sur l'ensemble résolu, si bien que le binaire d'une course `--workspace` n'est PAS
# celui d'une course `-p` — empreintes différentes, et jusqu'aux bibliothèques liées. Mesuré
# le 2026-08-27 : sous `--workspace`, `efe-agent-runner → axum-server "tls-rustls"` allume un
# SECOND fournisseur cryptographique rustls ; le binaire lie les deux (19 symboles aws-lc + 54
# ring, contre 0 + 77 sous `-p`) et rustls refuse de deviner lequel employer.
#
# Conséquence : deux tests ont été déclarés « instables » parce qu'ils « passaient seuls ».
# Ils ne passaient pas seuls — c'est un AUTRE binaire qui passait. La panique était
# parfaitement déterministe. Comparer deux binaires différents et appeler l'écart
# « instabilité » est une erreur de méthode qui produit un verdict crédible et faux.
#
# On interroge donc `--no-run --message-format=json`, qui donne les exécutables RÉELLEMENT
# produits par la course de workspace, et on lance celui-là, directement, sans repasser par
# cargo.
executables_du_workspace() {
    cargo test --workspace --no-run --message-format=json 2>/dev/null \
      | python3 -c "
import json,sys
for l in sys.stdin:
    try: m = json.loads(l)
    except Exception: continue
    if m.get('executable') and m.get('target'):
        print(m['target']['name'] + '\t' + m['executable'])
"
}

# L'exécutable qui CONTIENT ce test — on le demande aux binaires, on ne le déduit pas.
#
# La version d'avant cherchait le nom dans les sources puis devinait la cible. Elle a produit un
# faux verdict le 2026-09-02, par DEUX défauts indépendants dont chacun suffisait :
#
#   1. libtest nomme un test unitaire « tests::le_nom ». Le grep cherchait donc
#      « fn tests::le_nom( », qui n'existe nulle part : le script n'a JAMAIS su discriminer un
#      test unitaire, il n'avait simplement jamais eu à en juger un.
#   2. `cut -d/ -f2` rend le nom du PAQUET. La table des exécutables est indexée par le nom de la
#      CIBLE, et les deux diffèrent dès qu'un `[[bin]]` se renomme — efe-nano-inference-daemon
#      produit « inference-daemon ». Le point 1 corrigé, la recherche aurait échoué ici.
#
# Un binaire de test sait ce qu'il contient : `--list` l'énumère. Demander est plus court que
# déduire, et surtout ce n'est trompable ni par un préfixe de module, ni par une cible renommée,
# ni par un test qu'une macro engendre — trois hypothèses que l'index par la source portait sans
# les dire.
exe_qui_contient() {
    local nom="$1" exe
    while IFS=$'\t' read -r _ exe; do
        [ -x "$exe" ] || continue
        if "$exe" --list 2>/dev/null | grep -qxF "$nom: test"; then
            printf '%s' "$exe"
            return 0
        fi
    done < "$TRAVAIL/exes"
    return 1
}

# Les deux fonctions ci-dessus sont definies AVANT leur premier appel : bash ne connait pas
# une fonction declaree plus bas dans le fichier, et l'appel echouait en silence sur
# « command not found », faisant compter deux instables comme deux regressions.

# Une course BORNÉE dans le temps.
#
# Sans borne, ce script peut tourner indéfiniment sans rien écrire : mesuré le 2026-09-02, un tour
# est resté 4 h sans produire une ligne. Ce n'était PAS un blocage — `realism_loop_does_not_leak_memory`
# (efe-server) passe, en 467 s, quand on le joue seul — mais la machine paginait (13,4 Gio de swap
# sur 14,3), et sous `--workspace` les binaires se disputent la mémoire. C'est exactement le mode
# de défaillance que la note de clôture de ce script décrit : contention avec d'AUTRES PROCESSUS,
# charge machine.
#
# Un instrument qui met quatre heures sans le dire n'est pas plus honnête que celui qui rend un
# faux verdict : dans les deux cas l'exploitant ne sait pas où il en est. La borne ne juge pas le
# code — elle rend la NON-CONCLUSION visible, et distincte d'un échec.
#
# `timeout(1)` n'existe pas partout (macOS n'en a pas), d'où le tueur en tâche de fond.
tour_borne() {
    local log="$1" delai="$2" pid tueur code
    cargo test --workspace --no-fail-fast > "$log" 2>&1 &
    pid=$!
    ( sleep "$delai"; kill -9 "$pid" 2>/dev/null ) & tueur=$!
    wait "$pid" 2>/dev/null; code=$?
    kill "$tueur" 2>/dev/null; wait "$tueur" 2>/dev/null || true
    return "$code"
}

titre "0. La CAUSE du faux verdict est-elle encore hors de portée ?"
#
# Avant de compter des tours, dire POURQUOI ils passent. Le mode de défaillance du
# 2026-08-27 — deux fournisseurs cryptographiques rustls liés dans un même binaire, panique
# au démarrage du processus AVANT tout test — n'a pas été réparé : il est hors de portée
# depuis que É0 a retiré `efe-agent-runner`, et `axum-server`/`aws-lc-rs` du verrou avec lui.
#
# C'est une propriété fragile et invisible : elle ne tient à aucune ligne de code. Une seule
# dépendance ajoutée la casse, et la panique revient EN RESSEMBLANT à de l'instabilité —
# c'est-à-dire au verdict que R3 a déjà rendu, et qui était faux.
#
# Trois tours verts ne disent rien de cela. Ce garde, si.
if cargo test -q --locked -p efe-swarm-gateway --test r3_fournisseur_unique >/dev/null 2>&1; then
    ok "un seul fournisseur cryptographique dans Cargo.lock — la panique de 2026-08-27 est hors de portée"
else
    ko "le verrou porte PLUSIEURS fournisseurs cryptographiques"
    note "compter des tours n'a plus de sens : sous --workspace, un binaire peut paniquer AVANT"
    note "d'executer un seul test, et l'echec ressemblera a de l'instabilite. Voir"
    note "crates/efe-swarm-gateway/tests/r3_fournisseur_unique.rs pour le detail et le remede."
    exit 2
fi

titre "Reproduction — $TOURS tour(s) de « cargo test --workspace »"
note "le critère de R3 en demande TROIS d'affilée ; R3_TOURS le règle."

for t in $(seq 1 "$TOURS"); do
    printf '  tour %s/%s … ' "$t" "$TOURS"
    # ⭐ **LE DÉFAUT QUI A EMPÊCHÉ CE SCRIPT D'ABOUTIR, ET QUI NE SE VOIT QUE QUAND TOUT VA BIEN.**
    #
    # Il y avait ici une paire `set +e` / `set -e` autour de l'appel. La première moitié était
    # inerte — l'en-tête de ce fichier a délibérément choisi `set -uo pipefail`, SANS `-e`, et
    # écrit pourquoi : un contrôle doit CONCLURE, pas mourir à la première commande qui rend 1.
    # La seconde moitié, elle, ALLUMAIT errexit pour tout le reste de la boucle — un état que ce
    # script n'a jamais voulu.
    #
    # Conséquence, mesurée le 4 septembre 2026 : quinze lignes plus bas, `grep '^test .* FAILED$'`
    # rend **1 quand il ne trouve rien**, c'est-à-dire quand le tour est VERT. Avec errexit et
    # `pipefail`, le script se tuait là, en silence, et rendait **1** — que son propre contrat
    # documente comme « des échecs INSTABLES ». Une suite parfaitement verte était donc rapportée
    # comme instable, sans une ligne de sortie.
    #
    # Le défaut est INVISIBLE tant qu'il y a des échecs : le 27 août il y en avait, `grep`
    # trouvait, le script survivait. Il ne pouvait se manifester qu'au moment exact où R3 allait
    # enfin pouvoir conclure. Ce n'est pas le disque qui a empêché R3 d'aboutir — c'est ceci.
    tour_borne "$TRAVAIL/tour$t.log" "$DELAI"
    CODE_CARGO=$?
    # 137 = tué par la borne. Ni vert ni rouge : NON CONCLU. Le dire, et s'arrêter là — poursuivre
    # sur une course tronquée ferait passer pour « aucun échec » ce qui n'a jamais été exécuté.
    if [ "$CODE_CARGO" = "137" ]; then
        printf 'NON CONCLU\n'
        ko "la course n'a pas abouti en ${DELAI}s"
        note "ce n'est ni un echec ni un succes : la mesure n'a pas eu lieu."
        note "R3_DELAI_S rallonge la borne ; verifier d'abord la charge et le swap de la machine,"
        note "car sous pagination un tour de workspace se compte en heures, pas en minutes."
        exit 3
    fi
    # Le binaire de test précède ses cas : on garde le contexte pour pouvoir rejouer SEUL.
    # On ne garde QUE le nom du test. La tentation est de retenir aussi le binaire lu sur la
    # ligne « Running » qui précède — c'est faux : avec --workspace, cargo exécute plusieurs
    # binaires EN PARALLÈLE et entrelace leurs sorties, si bien que la ligne « Running » la plus
    # proche d'un échec appartient souvent à un autre crate. Cette erreur a fait déclarer
    # « reproductible » deux tests qui passaient parfaitement seuls — c'est-à-dire l'exact
    # contraire du verdict, sur le seul point que R3 demande de trancher.
    # `|| true` : `grep` rend 1 quand il ne trouve RIEN — donc quand le tour est vert. Ce n'est
    # pas une politesse défensive, c'est la correction du défaut décrit plus haut ; elle reste
    # nécessaire même sans errexit, pour que `pipefail` ne rende pas l'absence d'échec fatale si
    # quelqu'un rallumait `-e` un jour.
    grep -E '^test .* FAILED$' "$TRAVAIL/tour$t.log" | awk '{print $2}' | sort -u \
      > "$TRAVAIL/echecs$t.txt" || true
    E=$(wc -l < "$TRAVAIL/echecs$t.txt" | tr -d ' ')
    # **Croire le CODE DE SORTIE, pas seulement le comptage.** Un binaire de test qui s'ABORTE
    # (SIGABRT — bibliothèque dynamique manquante, panique dans un initialiseur) n'imprime
    # AUCUNE ligne « test … FAILED ». Le comptage rend alors 0 pendant que cargo rend 101, et le
    # script conclurait « aucun échec » sur une course qui s'est effondrée. Un compteur qui ne
    # sait pas voir un mort n'est pas un compteur.
    if [ "$CODE_CARGO" -ne 0 ] && [ "$E" -eq 0 ]; then
        printf '\n'
        ko "cargo sort en $CODE_CARGO alors qu'aucune ligne FAILED n'a ete lue"
        note "un binaire s'est probablement aborte sans passer par le harnais de test."
        grep -E "^error: test failed|SIGABRT|dyld|Library not loaded" "$TRAVAIL/tour$t.log" \
          | head -5 | sed 's/^/      /'
        exit 2
    fi
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
        printf '\n  \033[0;32mLe critère de R3 est TENU : trois tours sans échec.\033[0m\n\n'
        printf '  Sa note d'\''origine disait alors quoi faire — RETIRER R3, plutôt que de chercher\n'
        printf '  un défaut qui n'\''existe plus. Cette phrase a cessé d'\''être juste le jour où\n'
        printf '  l'\''étape 0 est apparue, et la corriger vaut mieux que la laisser vieillir :\n\n'
        printf '    · le défaut de 2026-08-27 n'\''a pas été RÉPARÉ, il est HORS DE PORTÉE — la\n'
        printf '      caisse qui tirait le second fournisseur cryptographique est partie avec les\n'
        printf '      47 membres que É0 a retirés du workspace ;\n'
        printf '    · c'\''est une propriété du VERROU, pas du code : aucune ligne ne la tient, et\n'
        printf '      une seule dépendance TLS ajoutée la casse ;\n'
        printf '    · retirer ce script effacerait le seul endroit du dépôt qui écrit ce\n'
        printf '      mécanisme, et son étape 0 avec.\n\n'
        printf '  Ce que R3 CHERCHAIT — de l'\''instabilité — n'\''existe pas ici, et n'\''a jamais\n'
        printf '  existé : la mesure du 27 août l'\''avait déjà réfuté. Ce que R3 SURVEILLE\n'
        printf '  désormais existe, et il vaut d'\''être surveillé.\n\n'
    else
        note "un seul tour ne tient pas le critère : relancer avec R3_TOURS=3."
    fi
    exit 0
fi

titre "Discrimination — chaque échec, rejoué SEUL (le MÊME binaire)"
executables_du_workspace > "$TRAVAIL/exes"
note "c'est ce second passage qui sépare l'instable du cassé."
# NON_MESURES est un compteur à part, et c'est le correctif de fond. La version d'avant écrivait
# « la mesure est caduque » puis faisait REELS=REELS+1 : elle transformait « je n'ai pas pu
# mesurer » en « le code a régressé », c'est-à-dire un verdict sur ce qu'elle venait de déclarer
# inconnaissable. Un instrument qui tombe en panne doit dire qu'il est en panne.
INSTABLES=0; REELS=0; NON_MESURES=0
while read -r nom; do
    [ -z "$nom" ] && continue
    EXE=$(exe_qui_contient "$nom") || {
        ko "NON MESURÉ — $nom"
        note "aucun binaire de la course ne declare ce test : c'est l'INSTRUMENT qui a echoue,"
        note "pas forcement le code. Le verdict le dira comme tel."
        NON_MESURES=$((NON_MESURES + 1))
        continue
    }
    note "binaire du workspace : $EXE"
    if "$EXE" "$nom" --exact --test-threads=1 > "$TRAVAIL/seul.log" 2>&1; then
        att "INSTABLE — $nom"
        note "LE MEME binaire passe seul et echoue en masse : contention reelle, pas un artefact"
        note "de reconstruction. C'est le sujet de R3."
        INSTABLES=$((INSTABLES + 1))
    else
        ko "REPRODUCTIBLE — $nom"
        note "échoue aussi seul : ce n'est PAS de l'instabilité, c'est une régression."
        REELS=$((REELS + 1))
    fi
done < "$TRAVAIL/tous.txt"

titre "Verdict"
note "$INSTABLES instable(s) · $REELS reproductible(s) · $NON_MESURES non mesure(s)"
if [ "$NON_MESURES" -gt 0 ]; then
    printf '\n  \033[0;31mL INSTRUMENT a echoue sur %s cas : ni instable ni regression, INCONNU.\033[0m\n' "$NON_MESURES"
    printf '  Reparer le script AVANT de conclure quoi que ce soit sur la suite.\n\n'
    exit 3
fi
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
