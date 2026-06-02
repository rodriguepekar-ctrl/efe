#!/usr/bin/env bash
# SPDX-License-Identifier: UNLICENSED
#
# scripts/backup.sh — Backup LOCAL complet et restaurable du dépôt EFE.
#
# Pourquoi : ce dépôt n'a (à ce jour) AUCUN remote → l'historique git n'existe
# qu'ici. Une suppression du dossier = perte de TOUT. Ce script crée un filet
# PORTABLE sans remote ni credentials (aucune publication réseau).
#
# Produit, dans $EFE_BACKUP_DIR (défaut ~/Downloads/efe-backups) :
#   1. efe-<ts>.bundle           — git bundle --all : TOUTE l'historique, clonable
#   2. efe-<ts>-uncommitted.patch — diff des modifs tracked NON commitées (la WIP)
#   3. efe-<ts>-untracked.txt     — liste des fichiers untracked (hors ignorés)
#   4. efe-<ts>-RESTORE.md        — instructions de restauration
# Rotation : ne garde que les EFE_BACKUP_KEEP derniers (défaut 5).
#
# Usage :
#   ./scripts/backup.sh
#   EFE_BACKUP_DIR=/Volumes/USB/efe-bak ./scripts/backup.sh   # vers un disque externe
#   EFE_BACKUP_KEEP=10 ./scripts/backup.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

if ! git rev-parse --git-dir >/dev/null 2>&1; then
    echo "✗ Pas un dépôt git : $REPO_ROOT" >&2
    exit 1
fi

DEST="${EFE_BACKUP_DIR:-$HOME/Downloads/efe-backups}"
KEEP="${EFE_BACKUP_KEEP:-5}"
TS="$(date +%Y%m%d-%H%M%S)"
NAME="efe-${TS}"

mkdir -p "$DEST"
echo "─── Backup EFE → $DEST ───"

# ── 1. Bundle : toute l'historique (toutes les refs), clonable ──
BUNDLE="$DEST/${NAME}.bundle"
git bundle create "$BUNDLE" --all
# Vérification d'intégrité (le bundle doit être un dépôt valide).
git bundle verify "$BUNDLE" >/dev/null
NCOMMITS="$(git rev-list --all --count)"
echo "  ✓ bundle : $(du -h "$BUNDLE" | cut -f1) · ${NCOMMITS} commits · $(git rev-parse --short HEAD)"

# ── 2. Patch des modifs NON commitées (tracked) — la WIP en cours ──
if ! git diff --quiet 2>/dev/null; then
    git diff > "$DEST/${NAME}-uncommitted.patch"
    NMOD="$(git diff --name-only | wc -l | tr -d ' ')"
    echo "  ✓ patch WIP : ${NMOD} fichier(s) modifié(s) non commité(s)"
else
    echo "  · aucune modif non commitée (working tree propre côté tracked)"
fi

# ── 3. Liste des fichiers untracked (hors ignorés) ──
git status --short -u 2>/dev/null | awk '/^\?\?/{sub(/^\?\? /,"");print}' > "$DEST/${NAME}-untracked.txt" || true
NUNTR="$(wc -l < "$DEST/${NAME}-untracked.txt" | tr -d ' ')"
echo "  ✓ untracked listés : ${NUNTR} (hors .gitignore)"

# ── 4. README de restauration ──
cat > "$DEST/${NAME}-RESTORE.md" <<EOF
# Restauration du backup EFE — ${TS}

Backup créé depuis : \`${REPO_ROOT}\`
Commits : **${NCOMMITS}** · HEAD : \`$(git rev-parse --short HEAD)\` — $(git log -1 --format=%s | head -c 70)

## 1. Cloner l'historique complet depuis le bundle
\`\`\`bash
git clone "${NAME}.bundle" efe-restored
cd efe-restored
git log --oneline -5        # vérifie l'historique
\`\`\`

## 2. Réappliquer la WIP non commitée (si le .patch existe)
Depuis le clone restauré (peu importe son emplacement), chemin ABSOLU du patch :
\`\`\`bash
git apply "${DEST}/${NAME}-uncommitted.patch"
\`\`\`

## 3. Vérifier l'intégrité du bundle (sans cloner)
\`\`\`bash
git bundle verify "${NAME}.bundle"
\`\`\`

> ⚠️ Les fichiers untracked (cf. ${NAME}-untracked.txt) et les artefacts ignorés
> (node_modules, .venv, etc.) ne sont PAS dans le bundle — réinstallables depuis
> les manifestes (Cargo.lock, pnpm-lock.yaml).
EOF
echo "  ✓ README de restauration"

# ── 5. Rotation : garde les KEEP derniers backups ──
# (boucle `while read` portable bash 3.2 — macOS n'a pas `mapfile`/bash 4+.)
ls -1t "$DEST"/efe-*.bundle 2>/dev/null | tail -n +"$((KEEP + 1))" | while IFS= read -r old; do
    [ -z "$old" ] && continue
    base="${old%.bundle}"
    rm -f "${base}.bundle" "${base}-uncommitted.patch" "${base}-untracked.txt" "${base}-RESTORE.md"
    echo "  ↻ rotation : purgé $(basename "$base")"
done

echo "─── ✓ Backup terminé : ${BUNDLE} ───"
echo "    Restaurer : git clone \"${BUNDLE}\" efe-restored"
