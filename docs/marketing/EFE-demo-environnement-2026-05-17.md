# EFE Demo — Environnement complet (master setup reference)

**Date drafted** : 2026-05-17
**Companion docs** :
- `docs/marketing/EFE-demo-script-2026-05-17.md` — narration + 6 acts
- `docs/marketing/EFE-demo-contournements-2026-05-17.md` — survie + recovery
**Owner** : @bench-infra-lead
**Statut** : LIVE — toutes les vérifications ci-dessous ont été exécutées sur la machine cible le 2026-05-17

---

## 0. Comment lire ce document

C'est la **fiche d'environnement totale** : tout ce qu'il faut
vérifier, configurer, ou avoir prêt sur la machine, dans la pièce,
sur le réseau, sur l'écran, sur l'audio, et dans la tête, avant de
faire la démo.

3 manières de l'utiliser :

1. **La veille (T-12h)** : lis le doc en entier, fais les § 1-6
   (hardware, power, OS, display, network, software).
2. **Le matin (T-2h)** : exécute le pre-flight script du § 18 et
   refais les § 7-13.
3. **Juste avant (T-5 min)** : ouvre le cheat-sheet § 24.

### 0.1 État verifié de la machine cible (snapshot 2026-05-17 22:30 UTC)

| Composant | Valeur | Statut |
|-----------|--------|--------|
| OS | macOS 15.1.1 (Sequoia) Build 24B91 | ✅ |
| Architecture | arm64 (Apple Silicon) | ✅ |
| CPU | Apple M3 Pro, 11 cores | ✅ |
| Memory | 36 GB | ✅ |
| Disk | 926 GB total, **13 GB libres** | ⚠️ TENDU |
| `target/` size | 31 GB | À surveiller |
| Rust | 1.88.0 (toolchain pinned via `rust-toolchain.toml`) | ✅ |
| Python 3.12 | `/usr/local/bin/python3.12` (= 3.12.0) | ✅ |
| System Python | 3.9.6 (insuffisant pour pyo3) | ⚠️ Workaround required |
| git, jq, bash, perl, caffeinate, xattr | tous présents | ✅ |
| `gtimeout` / `timeout` | **absents** | ⚠️ Workaround perl required |

### 0.2 Les 4 vérifications minimales (T-5 min)

Si tu ne devais faire QUE 4 choses :

```bash
cd /Users/rodrigue.pekar/Downloads/jjjj/efe              # cwd correct
export PYO3_PYTHON=python3.12 CARGO_TERM_COLOR=always    # env vars
df -h ~ | awk 'NR==2{print "disk free:", $4}'             # ≥ 5 Gi
cargo test -p efe-bench-harness --test g11_round_trip --quiet | tail -1
# attendu : test result: ok. 9 passed; ...
```

Si l'une des 4 ne donne pas le résultat attendu, **AVORTE et
debugger**.

---

## 1. Hardware setup

### 1.1 Laptop principal (machine de démo)

| Aspect | Recommandation | Vérification |
|--------|----------------|--------------|
| Modèle | Apple MacBook Pro M3 Pro (la machine validée 2026-05-17) | `sysctl -n machdep.cpu.brand_string` → "Apple M3 Pro" |
| Memory | ≥ 32 GB (recommandé pour `cargo build` + slide deck + Zoom) | `sysctl -n hw.memsize` → ≥ 34359738368 (32 GB en bytes) |
| Storage | ≥ 100 GB libres recommandé (13 GB libres aujourd'hui = tendu) | `df -h /` → vérifier disque > 5 Gi avant démo |
| Battery health | ≥ 80 % (sinon prendre le chargeur) | `system_profiler SPPowerDataType | grep "Cycle Count\|Condition"` |
| Charger | OBLIGATOIRE — la batterie peut chuter pendant `cargo build` | Vérifier avant départ : 96W (M3 Pro) ou plus |

### 1.2 Peripherals

| Item | Status |
|------|--------|
| Charger USB-C 96W (Apple original ou GaN équivalent) | OBLIGATOIRE |
| Adaptateur HDMI / USB-C-vers-HDMI | OBLIGATOIRE pour projection |
| Câble HDMI ≥ 2m | OBLIGATOIRE |
| Souris externe / trackpad bluetooth | RECOMMANDÉ (en backup si trackpad freeze) |
| Clavier externe USB | OPTIONNEL (préfère builtin pour le rythme) |
| Webcam externe (Logitech C920+) | OBLIGATOIRE si Zoom — caméra intégrée Mac sous-exposée |
| Micro-cravate ou casque | RECOMMANDÉ si pièce > 6 m² |

### 1.3 Backup hardware (en cas de panne primaire)

| Composant | Backup |
|-----------|--------|
| Laptop primaire | Avoir un 2e laptop avec le repo cloné, `cargo test --no-run` pré-exécuté, et ce doc imprimé |
| Charger | 2e charger USB-C dans le sac |
| Câble HDMI | 2e câble |
| Adaptateur | 2e adaptateur |
| Power bank USB-PD 100W | Permet 30 min de buffer si pas d'accès prise |
| Internet | Hotspot téléphone activé (pas pour la démo elle-même, mais pour follow-up email) |

### 1.4 Vérification matérielle (T-1h)

```bash
# Battery
pmset -g batt | head -2

# Power source
pmset -g ps

# CPU temp baseline (avant cargo qui chauffe)
sudo powermetrics --samplers smc -i 1 -n 1 2>/dev/null | grep -i "CPU die" | head -2 || true

# Disk
df -h ~ | head -2

# Memory
vm_stat | head -10
```

---

## 2. Power management

### 2.1 Modes Mac à activer

| Setting | Valeur | Pourquoi |
|---------|--------|----------|
| System Settings → Lock Screen → "Require password after sleep" | Désactivé pendant la démo | Évite popup mot de passe |
| System Settings → Displays → "Turn display off when inactive" | Never | Évite black screen mid-démo |
| `caffeinate -dimsu &` (lancé en arrière-plan) | Actif | Override software : empêche tout sleep |
| Énergie → Battery → "Slightly dim the display on battery" | Désactivé | Couleur constante |
| Énergie → "Optimize battery charging" | Désactivé temporairement | Charge full pour la session |

### 2.2 Lancement de caffeinate

```bash
# T-30 min avant la démo
caffeinate -dimsu &
CAFFEINATE_PID=$!
echo "caffeinate PID: $CAFFEINATE_PID — kill avec : kill $CAFFEINATE_PID"

# Vérifier que c'est lancé
pgrep -lf caffeinate
```

Options de `caffeinate` :
- `-d` : empêche le display de dormir
- `-i` : empêche l'idle sleep
- `-m` : empêche le disk sleep
- `-s` : empêche le system sleep (laptop fermé)
- `-u` : déclare une activité utilisateur (équivalent click souris)

### 2.3 Prise électrique

| Quoi | Comment |
|------|---------|
| Repérer la prise dispo dans la salle | T-15 min avant — pas pendant la démo |
| Brancher avant de commencer | T-5 min |
| Vérifier que le charger fonctionne | LED verte sur Apple chargers |
| Si pas de prise — power bank USB-PD 100W | Donne ~ 30 min de buffer |

### 2.4 Battery monitoring pendant la démo

`cargo build --workspace --bins` consomme ~ 4-6 W de plus que idle.
Sur un M3 Pro avec batterie 80 % :
- 13 min de démo + warmup = baisse de ~ 6-8 % de batterie
- Trivial si chargé.  Critique si pas chargé.

**Règle simple** : la batterie doit être ≥ 60 % au début de la
démo, ou tu es branché.  Sinon : refuser de commencer.

---

## 3. macOS configuration

### 3.1 Focus mode "Demo"

Créer un mode Focus dédié :

| Setting | Valeur |
|---------|--------|
| Nom | "Demo" |
| Allow notifications from | NONE |
| Allow time-sensitive notifications | Off |
| Hide notification previews | Always |
| Calendar | Not allowed |
| Mail | Not allowed |
| Messages | Not allowed |
| Lock screen → Show notifications | Off |

L'activer T-5 min via Control Center → Focus → Demo.

### 3.2 Apps à quitter

```bash
# Quitter les apps susceptibles d'interrompre
osascript -e 'quit app "Slack"'
osascript -e 'quit app "Discord"'
osascript -e 'quit app "Mail"'
osascript -e 'quit app "Calendar"'
osascript -e 'quit app "Messages"'
osascript -e 'quit app "Spotify"'
osascript -e 'quit app "Music"'

# Vérifier qu'ils ne tournent plus
ps aux | grep -iE "slack|discord|messages" | grep -v grep | head
```

### 3.3 Apps à GARDER ouvertes

- Terminal (ou iTerm2, ou warp.dev — celui de ta démo)
- Code editor (VS Code / Cursor / Sublime — celui que tu utilises)
- 1 navigateur si tu prévois de montrer le repo en page web (sinon
  fermer aussi)

### 3.4 Bureau (Desktop) state

| Vérifier | Action |
|----------|--------|
| Pas de fichiers personnels sur le bureau | Glisser dans un dossier `_hidden` avant la démo |
| Wallpaper neutre (pas une photo perso) | Système : couleur unie ou logo company |
| Stage Manager désactivé | System Settings → Desktop & Dock → Stage Manager Off |
| Mission Control configuré à 1 desktop space | Trois doigts vers le haut → fermer les espaces supplémentaires |

### 3.5 Dock et menu bar

| Item | Action |
|------|--------|
| Hide Dock automatically | OFF (rester visible)  — sauf si projection 4K où ça mange du contenu |
| Menu bar : auto-hide | OFF |
| Battery indicator | Show percentage ON (audience voit) |
| Time format | 24h pour cohérence avec timestamps UTC du code |

### 3.6 Notifications système éventuelles à anticiper

| Type | Quand | Action |
|------|-------|--------|
| Software Update available | Aléatoire | Désactiver : System Settings → General → Software Update → "Automatically install" Off (déjà fait par défaut sur Sequoia) |
| Time Machine reminder | Si Time Machine actif | Pause TM : `tmutil disable` (réactiver après) |
| iCloud sync | Aléatoire | Pause iCloud : System Settings → Apple ID → iCloud Drive → "Sync this Mac" Off (réactiver après) |
| Photos analysis | Si Photos vient d'être ouvert | Quit Photos avant démo |
| Spotlight indexing | Après upgrade ou mass copy | Vérifier `mdutil -s /` ne montre pas "Indexing" |

---

## 4. Display & projection setup

### 4.1 Résolution écran laptop

| Mode | Cas |
|------|-----|
| 1920×1200 native (mon laptop) | Démo en présence avec projecteur |
| 1440×900 scaled | Projecteur basse résolution (≤ 1080p) — augmenter taille police |
| 2560×1600 (default M3 Pro) | Idéal pour screen-share Zoom HD |
| Plus que 2880 | RISQUE : trop de pixels = audience ne distingue pas |

Vérifier : System Settings → Displays → Resolution.

### 4.2 Projection / écran externe

| Aspect | Valeur cible |
|--------|--------------|
| Résolution projecteur | ≥ 1920×1080 (1080p) |
| Refresh rate | 60 Hz minimum |
| Mirroring vs Extended | **Mirroring** pour la démo (audience voit la même chose que toi) |
| Gamma | Si écran a l'air délavé : System Settings → Displays → Color → calibrate |
| Aspect ratio | 16:9 — si projo 4:3, prévoir 20 % de marge horizontale dans les commandes |

### 4.3 Test projection T-15 min

Sur le projecteur final :

```bash
# Tester la lisibilité d'une couleur cargo
cargo test -p efe-bench-harness --test g11_round_trip --quiet 2>&1 | head -3
# → Le "ok" en vert doit être lisible depuis 5m de distance

# Tester les caractères Unicode box-drawing
bash scripts/backfill-tolerance-used.sh --dry-run bench/fixtures/pre_g11_ndjson/ 2>&1 | head -3
# → Les ═══════ doivent s'afficher proprement
```

Si les `═` se transforment en `?` ou des carrés vides : le projecteur a un fallback ASCII forcé.
- **Option 1** : changer la police terminal pour Menlo Regular qui a Unicode
- **Option 2** : désactiver le `CARGO_TERM_COLOR` et préparer une variante sans box-drawing

### 4.4 Si projection refuse de marcher

| Symptôme | Cause probable | Recovery |
|----------|---------------|----------|
| Écran noir externe | Adaptateur HDMI défaillant | Tester avec 2e adaptateur (cf § 1.3) |
| Image floue | Refresh rate incompatible | Settings → Displays → Force 60 Hz |
| Couleurs déformées | Gamma | Régler manuellement ou switch projection mode |
| Lag video | Câble trop long ou damaged | Câble alternatif |
| L'image clignote | Refresh rate fluctuant ou EMI | Changer le câble, déplacer le laptop loin des sources EM |
| L'image apparaît en miroir inversé (texte miroir) | Setting "rotation" | Settings → Displays → Rotation → 0° |

**Fallback ultime** : si la projection ne marche absolument pas,
faire la démo sur l'écran laptop directement, audience proche.
Audience max ~ 4 personnes.

---

## 5. Network configuration

### 5.1 Démo offline-safe : pas de dépendance réseau

Cf `docs/marketing/EFE-demo-contournements-2026-05-17.md` § 1.6 et
§ 7.  **Toutes** les commandes du démo script tournent en local.

| Surface | Network needed? |
|---------|-----------------|
| `cargo test` (compiled) | NO |
| `cargo build` (cache chaud) | NO |
| `bash scripts/audit-prompt-validator.sh` | NO |
| `bash scripts/backfill-tolerance-used.sh --dry-run` | NO |
| Static binaries (`efe-bench-harness version` etc.) | NO |
| **Follow-up email après démo** | YES (wifi ou cellular) |

### 5.2 Wifi setup

| Quand | Action |
|-------|--------|
| T-30 min | Joindre le wifi de la salle, vérifier ping vers github.com |
| T-15 min | Vérifier la stabilité (`ping -c 30 github.com | tail -5`) |
| T-5 min | NE PAS changer de wifi (cause des micro-coupures parfois) |
| Pendant démo | Ignorer le wifi.  Tout est en local. |

### 5.3 Cellular hotspot fallback

Si le wifi de la salle est lent ou tombe :
- Activer hotspot du téléphone (`Settings → Personal Hotspot`).
- Joindre le réseau hotspot sur le laptop.
- Bande passante typique : 50-100 Mbps en 4G/5G, suffisant pour
  follow-up.

### 5.4 VPN considerations

| Setting | Recommandation |
|---------|----------------|
| VPN d'entreprise actif | DÉSACTIVER pendant la démo — peut introduire de la latence sur Slack si tu reçois un message |
| WireGuard / Tailscale | OK si nécessaire pour accéder à un fichier interne |
| TOR | NON — peut bloquer GitHub si demande à montrer le repo |

---

## 6. Software stack verification

### 6.1 Rust toolchain

```bash
# Doit afficher 1.88.0 ou ≥ ; pinned via rust-toolchain.toml
rustc --version
# attendu : rustc 1.88.0 (6b00bc388 2025-06-23)

cargo --version
# attendu : cargo 1.88.0 (873a06493 2025-05-10)

# Le toolchain ACTIF (doit être pinned par rust-toolchain.toml)
rustup show active-toolchain
# attendu : 1.88-aarch64-apple-darwin (overridden by '...rust-toolchain.toml')

# Si pas 1.88 : rustup install 1.88
```

### 6.2 Python (pour pyo3)

```bash
# Vérifier la présence de Python ≥ 3.10
ls /usr/local/bin/python3.12 2>&1 | head -1
/usr/local/bin/python3.12 --version
# attendu : Python 3.12.x

# Si absent : brew install python@3.12
```

### 6.3 Tools shell

```bash
# Vérifier la présence des 6 commandes critiques
for cmd in git jq bash perl caffeinate xattr; do
  which $cmd >/dev/null && echo "  $cmd : OK" || echo "  $cmd : MISSING"
done

# Si l'une manque sur macOS :
#   git    : xcode-select --install
#   jq     : brew install jq
#   bash   : déjà builtin (mais c'est bash 3.2 — OK pour notre code)
#   perl   : déjà builtin
#   caffeinate : déjà builtin
#   xattr  : déjà builtin
```

### 6.4 `gtimeout` / `timeout` absent sur macOS — workaround

Vérifié : ni `gtimeout` ni `timeout` ne sont disponibles par défaut.

**Workaround portable** :

```bash
# Fonction utilitaire à charger avant la démo (cf § 17 timeline)
timeout_perl() {
  local s=$1; shift
  perl -e 'alarm shift; exec @ARGV' "$s" "$@"
}

# Usage : timeout_perl 5 ./target/debug/efe-server --help
```

**Alternative** : `brew install coreutils` ajoute `gtimeout` à
`/opt/homebrew/bin/`.

### 6.5 Audit-prompt-validator dépendances

```bash
# Le validator est pure bash + grep ; aucune dépendance externe
file scripts/audit-prompt-validator.sh
head -5 scripts/audit-prompt-validator.sh
```

### 6.6 Backfill script dépendances

```bash
# Le script utilise bash + python3 embedded
grep "python3" scripts/backfill-tolerance-used.sh | head -3
# → utilise python3 (qui est 3.9 ou 3.12, peu importe pour le JSON)
```

---

## 7. Environment variables — le canonical export block

À coller-coller en tête de chaque session de pre-flight + démo :

```bash
# CWD (impératif)
cd /Users/rodrigue.pekar/Downloads/jjjj/efe

# Python (pour cargo build des crates avec pyo3)
export PYO3_PYTHON=python3.12

# Cargo colors (pour l'audience qui voit la sortie cargo)
export CARGO_TERM_COLOR=always

# Cargo cache mode (économise le disque pendant la démo)
export CARGO_INCREMENTAL=0

# Rust backtrace pour debug live si nécessaire
export RUST_BACKTRACE=1

# Locale stable (évite les caractères Unicode déformés sur certains shells)
export LC_ALL=en_US.UTF-8
export LANG=en_US.UTF-8

# Caffeinate (déjà en background)
caffeinate -dimsu & CAFFEINATE_PID=$!
```

### 7.1 Vérification de l'export

```bash
env | grep -E "PYO3|CARGO|RUST_BACKTRACE|LC_ALL|LANG" | sort
```

Doit afficher 6 lignes.  Si en manque, re-exporter.

### 7.2 Variables à NE PAS exporter

| Variable | Pourquoi pas |
|----------|--------------|
| `RUST_LOG=debug` ou trace | Trop verbeux ; pollue la sortie démo |
| `CARGO_OFFLINE=1` | Pas nécessaire (cache déjà chaud) — risque de bloquer si quelque chose veut tirer un crate |
| Custom `PATH` qui shadow `/usr/local/bin` | Casserait l'accès à python3.12 |

---

## 8. Repository state validation

### 8.1 Working tree clean

```bash
cd /Users/rodrigue.pekar/Downloads/jjjj/efe
git status --short
```

**Attendu** : aucune sortie (working tree clean), OU uniquement les
fichiers créés cette session (cf `git status` pendant le pre-flight).

Si des fichiers inattendus sont touchés (par exemple un test l'a
écrit en debug) : `git status` et décider si stash, commit, ou
restore.

### 8.2 Branch + commit

```bash
git branch --show-current
git rev-parse HEAD | head -c 12
```

Vérifier que tu es sur la bonne branche.  Si pas : `git checkout
<branch-name>`.

### 8.3 Aucun changement non-commité dans les fichiers critiques

```bash
git diff --stat \
  bench/harness/src/compare/effective_tolerance.rs \
  bench/harness/tests/g11_round_trip.rs \
  bench/harness/tests/s9_s12_monitoring_chain_integrity.rs \
  bench/fixtures/backfill_golden_diff.txt \
  docs/lessons/PR-27-lessons.md \
  docs/gaps/master_tracker.md \
  docs/lessons/monitoring/PR-27-monitoring-week-11.md
```

Attendu : aucune sortie.  Si oui : `git diff <fichier>` pour voir
le delta, et décider.

### 8.4 Vérification de la présence des fichiers utilisés par le démo

```bash
critical_files=(
  "docs/lessons/PR-27-lessons.md"
  "docs/gaps/master_tracker.md"
  "docs/lessons/monitoring/PR-27-monitoring-week-9.md"
  "docs/lessons/monitoring/PR-27-monitoring-week-10.md"
  "docs/lessons/monitoring/PR-27-monitoring-week-11.md"
  "docs/lessons/monitoring/PR-27-monitoring-week-12.md"
  "docs/lessons/monitoring/PR-27-quarterly-dry-run-2026-07-31.md"
  "docs/lessons/monitoring/PR-27-closure-2026-08-07.md"
  "docs/lessons/checklists/PR-27-S3-P1.md"
  "docs/lessons/checklists/PR-27-S3-P2.md"
  "docs/lessons/checklists/PR-27-S3-P3.md"
  "docs/lessons/checklists/PR-27-S3-P4.md"
  "docs/lessons/checklists/PR-27-S3-P5.md"
  "docs/lessons/checklists/PR-27-S3-P6.md"
  "docs/lessons/checklists/S3-standup-notes.md"
  "docs/lessons/PR-27-chirurgical-audit-2026-06-05.md"
  "bench/fixtures/backfill_golden_diff.txt"
  "bench/fixtures/audit_reader_golden_explain.txt"
  "bench/fixtures/scenarios/G11_synthetic.yaml"
  "scripts/audit-prompt-validator.sh"
  "scripts/backfill-tolerance-used.sh"
)

missing=0
for f in "${critical_files[@]}"; do
  [ -f "$f" ] && echo "  ✓ $f" || { echo "  ✗ MISSING: $f" ; missing=$((missing+1)) ; }
done
[ "$missing" -eq 0 ] && echo "All critical files present" || { echo "$missing files missing — AVORT" ; exit 1 ; }
```

### 8.5 Fixtures pre-G.11 NDJSON présents et lisibles

```bash
ls -la bench/fixtures/pre_g11_ndjson/*.ndjson
# attendu : 5 fichiers (case_a, case_b, case_c, case_d, case_e)

for f in bench/fixtures/pre_g11_ndjson/*.ndjson; do
  head -1 "$f" | jq -e . > /dev/null && echo "  ✓ $f (valid JSON)" || echo "  ✗ $f (BAD JSON)"
done
```

### 8.6 Vérification que les 7 tests PR #27 passent

```bash
cargo test -p efe-bench-harness \
  --test g11_round_trip \
  --test g11_tolerance_used_populated \
  --test stability_contract_v17 \
  --test backfill_edge_cases \
  --test pr27_workflow_yaml_invariants \
  --test s3_audit_chain_integrity \
  --test s9_s12_monitoring_chain_integrity 2>&1 | grep -E "^test result:"
# attendu : 7 lignes "ok. N passed; 0 failed"
```

---

## 9. Cargo cache & build artifacts

### 9.1 Vérifier l'état du cache

```bash
du -sh target/ 2>&1
# attendu : 25-40 GB.  Si < 5 GB : le cache n'est pas chaud → rebuild.

du -sh target/debug/deps/ 2>&1
# attendu : 15-25 GB

du -sh target/debug/incremental/ 2>&1
# variable ; peut être > 5 GB ou 0
```

### 9.2 Préchauffer le cache si nécessaire (T-60 min)

```bash
# Pre-build tous les test binaries utilisés en démo (5-10 min cache froid)
cargo test -p efe-bench-harness --no-run \
  --test g11_round_trip \
  --test g11_tolerance_used_populated \
  --test stability_contract_v17 \
  --test backfill_edge_cases \
  --test pr27_workflow_yaml_invariants \
  --test s3_audit_chain_integrity \
  --test s9_s12_monitoring_chain_integrity

# Pre-build les 2 binaires utilisés en démo
cargo build -p efe-bench-harness --bin efe-bench-harness
cargo build -p efe-bench         --bin efe-bench
```

### 9.3 Si le cache est saturé (disque < 5 Gi libres)

```bash
# Cleanup SANS casser le cache test
rm -rf target/release            # ~ 6-7 GB
rm -rf target/criterion          # ~ 1.5 MB
rm -rf target/doc                # ~ 8 MB

# Re-vérifier
df -h ~ | head -2
du -sh target/

# IL EST INTERDIT de faire :
#   cargo clean        # tout le cache, 8+ min rebuild
#   rm -rf target/debug/incremental  # OK SI tu n'utilises pas l'incremental
#   rm -rf target/debug/deps  # rebuild des tests, 5+ min
```

### 9.4 Que NE PAS faire au cache pendant la démo

| Action | Conséquence | Recovery |
|--------|-------------|----------|
| `cargo clean` | 8+ min de rebuild | Abort démo |
| `cargo update` | 2-4 min de rebuild des deps modifiées | Abort démo |
| `git checkout <autre-branche>` | Cargo.lock peut changer → rebuild | Revenir à la branche, attendre 1m 08s |
| Toucher `bench/harness/Cargo.toml` | Recompil de tous les binaires de bench | Évite |
| Touch `bench/harness/src/...` | Recompil de cette crate (~30s) | Évite |

### 9.5 Quand le cache est en état idéal pour démo

Indicateurs :

```bash
# 1. les binaires existent et sont récents (< 24h)
stat -f "%m %N" target/debug/efe-bench-harness target/debug/efe-bench | head

# 2. les tests sont compilés et présents
ls -la target/debug/deps/g11_round_trip-* | head -3
ls -la target/debug/deps/s9_s12_monitoring_chain_integrity-* | head -3

# 3. `cargo test --no-run` est instantané (< 5 sec)
time cargo test -p efe-bench-harness --test g11_round_trip --no-run 2>&1 | tail -2
```

---

## 10. Terminal configuration

### 10.1 Choix du terminal

| Option | Pour | Contre |
|--------|------|--------|
| Terminal.app (built-in macOS) | Stable, pas d'install | Manque de fonctionnalités |
| iTerm2 | Très complet, splits | Légère courbe d'apprentissage |
| Alacritty | GPU-accelerated, rapide | Pas de splits natifs |
| WezTerm | GPU + tabs + multiplexer | Plus lourd |
| Warp.dev | UI moderne, IA-assist | Cloud, peut leak |

**Recommandation** : iTerm2 (le standard de fait pour démos).

### 10.2 Configuration iTerm2

| Setting | Valeur |
|---------|--------|
| Profile → Text → Font | Menlo Regular ou JetBrains Mono ≥ 18 pt |
| Profile → Text → Bold font | Bold variant |
| Profile → Window → Columns | 120 (largeur) |
| Profile → Window → Rows | 35 (hauteur) |
| Profile → Window → Style | Normal (pas Full Screen — l'audience verra mieux) |
| Profile → Colors → Background | Pur noir (#000000) ou très sombre (#0a0a0a) |
| Profile → Colors → Foreground | Blanc cassé (#e0e0e0) |
| Profile → Colors → ANSI Green | Vif (#33ff33) — pour les "ok" cargo |
| Profile → Colors → ANSI Red | Vif (#ff3333) — pour les "FAILED" |
| Profile → Terminal → Scrollback lines | 10,000 (pour revenir en arrière en Q&A) |
| Profile → Session → Idle behavior | Send "" (rien) — évite "broken pipe" |

### 10.3 Pre-démo terminal commands

```bash
# Clear scrollback (audience commence vierge)
clear
printf '\033[3J'   # vide le scrollback

# Vérifier les dimensions
tput cols && tput lines

# Tester un échantillon de sortie cargo (couleur)
cargo --version
```

### 10.4 Multiplexer (tmux / screen) — recommandé NON

Pour une démo : un terminal simple est plus lisible qu'un tmux avec
status-bar.  **NE PAS lancer tmux** pendant la démo sauf si tu es
habitué.

---

## 11. Editor configuration

### 11.1 Choix de l'editor

| Editor | Pour démo | Notes |
|--------|-----------|-------|
| VS Code | Excellent | UI claire, on peut pré-ouvrir tabs |
| Cursor | Excellent | Variant VS Code avec IA |
| Sublime Text | Bon | Rapide, simple |
| nvim | NON pour la démo | Vis trop compliqué pour audience non-tech |
| TextEdit | NON | Pas de syntax highlight |

**Recommandation** : VS Code ou Cursor (le standard moderne).

### 11.2 Configuration éditeur

| Setting | Valeur |
|---------|--------|
| Font size | ≥ 16 pt |
| Theme | Dark+ ou One Dark — high contrast |
| Word wrap | OFF (préserver la mise en forme markdown) |
| Minimap | OFF (distrayant en démo) |
| Activity bar | OFF |
| Status bar | OFF |
| Side bar | OFF (ouvrir au besoin) |
| Show file paths in title bar | ON |
| Editor → Cursor blinking | smooth |

### 11.3 Désactiver les extensions intrusives

```bash
# Lister extensions VS Code
code --list-extensions | head -20
```

Désactiver pour la démo :
- GitHub Copilot autocompletion (peut popup une suggestion pendant qu'on scroll un fichier)
- Auto-save toutes les 100 ms (peut afficher "Saving..." en barre)
- Spell checker pour markdown (souligne tout en rouge → confusion audience)

---

## 12. Pre-opened tabs and files (T-15 min)

Ouvrir dans cet ordre exact dans VS Code :

| Onglet # | Fichier | Cmd shortcut |
|----------|---------|--------------|
| 1 | `docs/gaps/master_tracker.md` | ⌘+1 |
| 2 | `docs/lessons/PR-27-lessons.md` | ⌘+2 |
| 3 | `docs/lessons/monitoring/PR-27-closure-2026-08-07.md` | ⌘+3 |
| 4 | `bench/harness/tests/g11_round_trip.rs` | ⌘+4 |
| 5 | `bench/harness/tests/s9_s12_monitoring_chain_integrity.rs` | ⌘+5 |

Pour ouvrir tous en une commande :

```bash
code docs/gaps/master_tracker.md \
     docs/lessons/PR-27-lessons.md \
     docs/lessons/monitoring/PR-27-closure-2026-08-07.md \
     bench/harness/tests/g11_round_trip.rs \
     bench/harness/tests/s9_s12_monitoring_chain_integrity.rs
```

Puis : ⌘+1 ramène au premier, ⌘+5 va au cinquième.  Ces 5 fichiers
suffisent pour toute la démo + Q&A.

### 12.1 Position cursor avant démo

Sur chaque fichier, scroll au début (Cmd+Up, puis Home).

Sur `master_tracker.md` :
- Avoir déjà cmd+F (find) ouvert avec recherche pour "G.11"
- Cursor pointing au début de la ligne "G.11 — tolerance_used"

### 12.2 Splits (optionnel)

Si projo grand format (≥ 24"), splitter VS Code en 2 : à gauche la
fenêtre principale, à droite le terminal intégré (Cmd+`).

---

## 13. Screen-sharing platform tuning

### 13.1 Zoom

| Setting | Valeur |
|---------|--------|
| Settings → Video → HD | ON |
| Settings → Video → Enable HD | ON (jusqu'à 1080p) |
| Settings → Screen Share → Share entire screen | OUI pour la démo |
| Settings → Screen Share → "Optimize Screen Share for video clips" | OFF (text/code) |
| Settings → Audio → Suppress background noise | High |
| Settings → Audio → Echo cancellation | ON |
| Settings → General → "Use dual monitors" | Selon setup |
| Settings → General → "Maximum participants in gallery" | 25 (suffit pour VC pitch) |

### 13.2 Google Meet

| Setting | Valeur |
|---------|--------|
| Settings → Video → Send resolution | 720p HD (Auto si bande passante OK) |
| Settings → Video → Receive resolution | Auto |
| Settings → Audio → Noise cancellation | ON |
| Présentation → Présenter la fenêtre Terminal seule (pas tout l'écran) | EN OPTION |
| Présentation → Présenter un onglet Chrome | NON pour démo CLI |

### 13.3 Tencent VooV (cas Kovant)

| Setting | Valeur |
|---------|--------|
| Video → "Auto adjust resolution" | OFF, force 1080p |
| Audio → Noise reduction | ON |
| Screen share → "Optimize for code" | ON si présent |

### 13.4 Audio settings (mic)

| Item | Valeur |
|------|--------|
| Mic | Soit AirPods Pro, soit casque-mic filaire ; pas le mic intégré laptop |
| Test mic | "Are you hearing me clearly?" au début |
| Volume input | -12 dB à -6 dB sur peak |
| Side-tone (s'entendre soi-même) | OFF |

---

## 14. Audience environment

### 14.1 Disposition de salle (cas type VC pitch en présence)

```
                    [écran projection 65"+]
                            |
            ┌───────────────┴───────────────┐
            │                               │
            │    Toi (debout)               │
            │    Laptop sur table           │
            │                               │
            ├───────────────────────────────┤
            │ Partner A    Partner B        │
            │                               │
            │ Partner C    Partner D        │
            │                               │
            │ (associate, prise notes)      │
            └───────────────────────────────┘
                  Table boardroom
```

| Aspect | Recommandation |
|--------|----------------|
| Toi | Debout, légèrement à gauche de l'écran pour ne pas le bloquer |
| Laptop | Sur table fixe, ouvert à 110° (visible audience + écran ext) |
| Audience | 4-6 personnes max sur une démo intime ; 8-12 sur un investor demo day |
| Distance toi-audience | 2-3 m (lecture distance + énergie) |
| Lumière | Eclairage neutre — éviter contre-jour sur ton visage |

### 14.2 Disposition de salle (cas distance / Zoom)

| Aspect | Recommandation |
|--------|----------------|
| Toi | Centré dans la caméra, 1/3 du frame du haut (règle des tiers) |
| Background | Bibliothèque ou mur uni — pas porte ouverte (passages distrayants) |
| Lumière | Source lumière devant toi (fenêtre + ring light), pas derrière |
| Lookat camera | Pas l'écran — la caméra |
| Hauteur caméra | Au niveau des yeux (élever le laptop si besoin) |

### 14.3 Notes audience (qui prend, qui parle)

Avant la démo, savoir :
- **Qui** prendra la décision d'investir (lead partner) — adresser à lui en priorité.
- **Qui** prendra les notes (associate) — peut interrompre pour clarifications techniques.
- **Qui** est l'expert technique parmi eux (peut poser questions hostiles).
- **Si l'un** veut prendre une photo de ton écran — accepter (c'est bon signe d'intérêt).

### 14.4 Time-box

| Slot | Durée typique | Composition |
|------|---------------|-------------|
| YC partner call (round 1) | 10 min total | 2 min pitch + 8 min Q&A |
| YC interview | 10 min total | 5 min pitch + 5 min Q&A |
| Kovant pitch | 30 min total | 15 min pitch + 15 min Q&A |
| Demo day | 5 min total | Demo + 1 min Q |
| Customer demo | 30-45 min | 10 min context + 20 min demo + 5-15 min Q |

Adapter la démo : core 12 min pour Kovant/customer ; **réduire à 6
min** pour YC en gardant Act 3 + Act 4 + Closing.

---

## 15. Time zone and clock

### 15.1 Pourquoi ça compte

Les docs S9-S12 contiennent des timestamps UTC (closure 2026-08-07
17:00 UTC, daily pinning 09:15 UTC, etc.).  Si quelqu'un demande
"c'est quoi 09:15 UTC pour moi", il faut savoir répondre vite.

### 15.2 Cheat sheet conversions

| 09:15 UTC est | À mémoriser |
|----------------|--------------|
| 10:15 / 11:15 CET / CEST (heure suisse) | Suisse, France |
| 02:15 / 01:15 PST / PDT (heure SF) | YC, west coast US |
| 05:15 / 04:15 EST / EDT (heure NY) | east coast US |
| 17:15 SGT (heure Singapour) | APAC investors |
| 18:15 JST (heure Tokyo) | Japan |

### 15.3 Le système macOS

Vérifier que l'horloge système est en UTC ou affichée en local cohérent :

```bash
date
date -u
```

Si tu présentes en Suisse mais tes timestamps sont UTC, dire "ce
17:00 UTC c'est 19:00 chez nous le soir" en passant — montre que tu
maîtrises.

---

## 16. The full pre-flight timeline

### 16.1 T-12 heures (la veille au soir)

| Tâche | Durée |
|-------|-------|
| Lire le démo script en entier | 20 min |
| Lire le contournements doc | 20 min |
| Vérifier que le repo est à jour (pull si nécessaire) | 5 min |
| Pre-build complet : `cargo build --workspace --bins` | 5-10 min |
| Pre-compile tests : `cargo test --workspace --no-run` (PR #27 only) | 5 min |
| Battery charge ≥ 80 % | passive |
| Téléphone chargé | passive |

### 16.2 T-2 heures

| Tâche | Durée |
|-------|-------|
| Faire un dry-run complet du démo à vide dans une chambre | 20 min |
| Chronométrer chaque act | passive |
| Re-vérifier les 6 commandes critiques (§ 1.5 contournements) | 2 min |
| Vérifier disque libre ≥ 5 Gi | 30 sec |
| Préparer un onglet navigateur ouvert sur le rapport produit-test | 1 min |
| Habiller (présentable, mais confortable) | 15 min |

### 16.3 T-60 minutes (arrivée sur site)

| Tâche | Durée |
|-------|-------|
| Salle ouverte ? Si non, prévenir l'host | 1 min |
| Prise électrique repérée + libre | 1 min |
| Brancher le chargeur | 30 sec |
| Tester la projection | 5 min |
| Connecter au wifi de la salle | 2 min |
| Activer Focus "Demo" | 10 sec |
| Quitter les apps non-essentielles | 1 min |
| Lancer `caffeinate -dimsu &` | 5 sec |

### 16.4 T-30 minutes

| Tâche | Durée |
|-------|-------|
| `cd` au bon dir + export env vars | 1 min |
| Run pre-flight script (§ 18) | 2-3 min |
| Vérifier les 7 tests PR #27 passent | 30 sec |
| Re-vérifier `ls docs/lessons/monitoring/PR-27-monitoring-week-11.md` (wow-moment ready) | 5 sec |
| Refaire dry-run du wow-moment dans un sub-shell | 1 min |
| Si Zoom : tester audio + video avec ami | 5 min |

### 16.5 T-15 minutes

| Tâche | Durée |
|-------|-------|
| Ouvrir les 5 onglets éditeur dans l'ordre | 2 min |
| Cmd+F dans master_tracker.md sur "G.11" | 10 sec |
| Préparer terminal avec `clear` | 5 sec |
| Mettre cmd+1 sur le terminal | 5 sec |
| Prendre une gorgée d'eau | 30 sec |
| Respirer 4-7-8 (4 sec in, 7 hold, 8 out) x 3 | 1 min |

### 16.6 T-5 minutes

| Tâche | Durée |
|-------|-------|
| Vérifier le cheat-sheet § 24 sous la main | 5 sec |
| Battery indicator visible ; ≥ 60 % | 5 sec |
| Sourire à l'audience, présenter rapidement | 30 sec |

### 16.7 T-0 — GO

Ton ouverture : "On a passé 3 mois à fermer une ligne fausse..."

---

## 17. The pre-flight shell script (companion deliverable)

Un script à exécuter automatiquement pour vérifier tout d'un coup :

`scripts/efe-demo-preflight.sh` (à créer si pas présent — cf § 18).

```bash
#!/usr/bin/env bash
# Pre-flight check for EFE demo. Returns 0 if all OK, ≥ 1 otherwise.
# Run from /Users/rodrigue.pekar/Downloads/jjjj/efe

set -u
FAIL=0

ok()   { printf "  \033[32m✓\033[0m %s\n" "$1" ; }
warn() { printf "  \033[33m!\033[0m %s\n" "$1" ; }
fail() { printf "  \033[31m✗\033[0m %s\n" "$1" ; FAIL=$((FAIL+1)) ; }

echo "═══════════════════════════════════════════════════════════"
echo "  EFE Demo Pre-flight — $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "═══════════════════════════════════════════════════════════"
echo ""

# 1. CWD
if [ "$(pwd)" = "/Users/rodrigue.pekar/Downloads/jjjj/efe" ]; then
  ok "cwd : $(pwd)"
else
  fail "cwd : $(pwd) — expected /Users/rodrigue.pekar/Downloads/jjjj/efe"
fi

# 2. Env vars
[ "${PYO3_PYTHON:-}" = "python3.12" ] && ok "PYO3_PYTHON=python3.12" || fail "PYO3_PYTHON not set"
[ "${CARGO_TERM_COLOR:-}" = "always" ] && ok "CARGO_TERM_COLOR=always" || warn "CARGO_TERM_COLOR not 'always'"

# 3. Disk
AVAIL_KB=$(df -k ~ | awk 'NR==2{print $4}')
AVAIL_GB=$((AVAIL_KB / 1024 / 1024))
if [ "$AVAIL_GB" -ge 5 ]; then
  ok "disk free : ${AVAIL_GB} GB"
else
  fail "disk free : ${AVAIL_GB} GB (< 5 GB)"
fi

# 4. Python
if /usr/local/bin/python3.12 --version >/dev/null 2>&1; then
  ok "python3.12 at /usr/local/bin/python3.12"
else
  fail "python3.12 not found at /usr/local/bin/python3.12"
fi

# 5. Rust
if rustc --version | grep -q "1\.88"; then
  ok "rustc 1.88.x"
else
  warn "rustc version : $(rustc --version)"
fi

# 6. Cargo cache
if [ -x target/debug/efe-bench-harness ] && [ -x target/debug/efe-bench ]; then
  ok "binaries pre-built (efe-bench-harness, efe-bench)"
else
  fail "binaries NOT pre-built"
fi

# 7. Critical files
CRITICAL_FILES=(
  "docs/lessons/PR-27-lessons.md"
  "docs/gaps/master_tracker.md"
  "docs/lessons/monitoring/PR-27-monitoring-week-11.md"
  "docs/lessons/monitoring/PR-27-closure-2026-08-07.md"
  "bench/fixtures/backfill_golden_diff.txt"
  "scripts/audit-prompt-validator.sh"
  "scripts/backfill-tolerance-used.sh"
)
for f in "${CRITICAL_FILES[@]}"; do
  [ -f "$f" ] && ok "file present : $f" || fail "file MISSING : $f"
done

# 8. Test sanity (the pinning test must pass)
if cargo test -p efe-bench-harness --test g11_round_trip --quiet 2>&1 | grep -q "^test result: ok. 9 passed"; then
  ok "g11_round_trip : 9 PASS"
else
  fail "g11_round_trip : did NOT pass cleanly"
fi

# 9. Backfill dry-run sanity
if bash scripts/backfill-tolerance-used.sh --dry-run bench/fixtures/pre_g11_ndjson/ 2>&1 | grep -q "End of golden diff"; then
  ok "backfill dry-run : end-of-golden reached"
else
  fail "backfill dry-run : did NOT complete"
fi

# 10. Validator sanity
if bash scripts/audit-prompt-validator.sh docs/lessons/PR-27-lessons.md 2>&1 | grep -q "passes schema validation"; then
  ok "audit-prompt validator : lessons file PASS"
else
  fail "audit-prompt validator : lessons file FAIL"
fi

# 11. caffeinate alive
if pgrep -f caffeinate >/dev/null; then
  ok "caffeinate running"
else
  warn "caffeinate NOT running — launch with : caffeinate -dimsu &"
fi

# 12. Focus mode (cosmetic)
warn "Focus 'Demo' mode : verify manually in Control Center"

# 13. Battery
if pmset -g batt 2>/dev/null | grep -q "AC Power"; then
  ok "AC power : connected"
else
  BATT=$(pmset -g batt | head -2 | tail -1 | grep -o '[0-9]*%' | head -1)
  if [ -n "$BATT" ]; then
    BATT_INT=${BATT%\%}
    if [ "$BATT_INT" -ge 60 ]; then
      ok "battery : $BATT (on battery, but OK)"
    else
      fail "battery : $BATT (< 60% AND not on AC)"
    fi
  fi
fi

# Final
echo ""
echo "═══════════════════════════════════════════════════════════"
if [ "$FAIL" -eq 0 ]; then
  echo "  \033[32mPRE-FLIGHT PASS\033[0m"
else
  echo "  \033[31mPRE-FLIGHT FAIL ($FAIL issues)\033[0m"
fi
echo "═══════════════════════════════════════════════════════════"

exit "$FAIL"
```

Sauvegarder dans `scripts/efe-demo-preflight.sh`, `chmod +x`,
exécuter à T-30 min.

---

## 18. Vérifier que le script preflight existe

```bash
ls -la scripts/efe-demo-preflight.sh 2>&1
# Si absent : créer le script à partir du § 17, chmod +x, et l'exécuter
```

---

## 19. Backup machine strategy

### 19.1 La règle des 2 laptops

Pour toute démo importante, avoir 2 laptops prêts :
- **Primaire** : la machine principale, déjà setup
- **Backup** : un 2e laptop avec le repo cloné, env identique

### 19.2 Setup du backup laptop (à faire la veille)

```bash
# Sur le backup laptop
cd ~
git clone <repo-url> efe-backup
cd efe-backup
git checkout <demo-branch>

# Setup env
export PYO3_PYTHON=python3.12
# (vérifier que python3.12 est installé)
brew install python@3.12  # si absent

# Pre-build
cargo test --workspace --no-run

# Vérifier les 7 tests PR #27
for t in g11_round_trip g11_tolerance_used_populated stability_contract_v17 \
         backfill_edge_cases pr27_workflow_yaml_invariants \
         s3_audit_chain_integrity s9_s12_monitoring_chain_integrity ; do
  cargo test -p efe-bench-harness --test "$t" --quiet 2>&1 | tail -1
done

# Vérifier les 2 binaires
cargo build -p efe-bench-harness --bin efe-bench-harness
cargo build -p efe-bench         --bin efe-bench
```

### 19.3 Si le primaire meurt en plein démo

| Symptôme | Action |
|----------|--------|
| Laptop freeze | Hard-restart : tenir power button 10 sec, redémarrer (1 min) |
| Écran noir + bluescreen | Switch au backup laptop (90 sec) |
| Wifi/projection devient incompatible | Switch au backup laptop |
| Audio cassé en Zoom | Switch au backup |

**Procédure de switch (90 sec)** :
1. Annoncer : "Pardon — je passe sur mon laptop backup."
2. Débrancher l'HDMI du primaire.  Rebrancher sur le backup.
3. Réveiller le backup (déjà ouvert et signé in).
4. Cmd+1 sur Terminal.  Cd au repo.  Reprendre où tu étais.

**Pré-requis** : backup laptop ouvert + signé in + Terminal ouvert
au bon dossier.  À faire en arrivant dans la salle.

---

## 20. Multi-monitor / no-monitor scenarios

### 20.1 1 écran (laptop seul, audience autour)

| Aspect | Notes |
|--------|-------|
| Police terminal | 22-24 pt (compenser pour la distance audience) |
| Audience max | 4 personnes |
| Posture | Tu pivotes le laptop selon qui parle |

### 20.2 2 écrans : laptop + projection mirroring

C'est le mode normal.

| Aspect | Notes |
|--------|-------|
| Mode | Mirror (audience voit la même chose que toi) |
| Police terminal | 18-20 pt |
| Cursor visibility | Display Settings → Accessibility → Pointer → "Larger" |

### 20.3 3+ écrans : laptop + projection + écran secondaire

| Aspect | Notes |
|--------|-------|
| Écran secondaire | Notes / cheat-sheet pour toi, non visible audience |
| Risque | Cliquer par accident sur l'écran secondaire → la projection ne suit pas |

**Recommandation** : 2 écrans est l'optimum.  3 écrans seulement
si tu as l'habitude.

### 20.4 Cas sans écran externe

Si le projecteur tombe à T-1 min :
- Faire la démo sur l'écran laptop directement.
- Augmenter la police terminal à 24-26 pt.
- Audience max 4 personnes (proches du laptop).
- Tu te positionnes derrière le laptop ; audience face à l'écran.

---

## 21. Recovery scripts (par failure mode)

### 21.1 Recovery 1 — terminal complètement freeze

```bash
# Sur un nouveau Terminal (Cmd+T) :
cd /Users/rodrigue.pekar/Downloads/jjjj/efe
export PYO3_PYTHON=python3.12 CARGO_TERM_COLOR=always
clear
# Reprendre là où tu étais
```

### 21.2 Recovery 2 — un test soudain rouge

```bash
# Identifier ce qui a bougé
git status --short

# Vérifier le test isolé
cargo test -p efe-bench-harness --test g11_round_trip --quiet 2>&1 | tail -5

# Si la régression vient d'un fichier touché :
git diff <fichier-touché>

# Restore si nécessaire
git checkout <fichier-touché>
```

### 21.3 Recovery 3 — disk full pendant la démo

```bash
# Sur un Terminal séparé :
rm -rf /tmp/.cargo-build-* /tmp/cargo-target-* 2>/dev/null
rm -rf ~/Library/Caches/com.apple.* 2>/dev/null  # Apple temp caches
df -h ~ | head -2

# Si toujours full :
rm -rf target/release target/criterion target/doc 2>/dev/null
```

### 21.4 Recovery 4 — un binaire crash

```bash
# Diagnose
./target/debug/efe-bench-harness version 2>&1
echo $?

# Si "Killed: 9" ou "Segmentation fault" :
RUST_BACKTRACE=full ./target/debug/efe-bench-harness version 2>&1

# Si vraiment cassé : rebuild
cargo build -p efe-bench-harness --bin efe-bench-harness
```

### 21.5 Recovery 5 — wow-moment file lost

```bash
# Si tu ne trouves plus /tmp/.efe-wm-week11.md :
# Solution 1 : restore depuis git
git checkout docs/lessons/monitoring/PR-27-monitoring-week-11.md

# Solution 2 : copier depuis le backup laptop
# scp backup-laptop:efe-backup/docs/lessons/monitoring/PR-27-monitoring-week-11.md \
#     docs/lessons/monitoring/PR-27-monitoring-week-11.md
```

### 21.6 Recovery 6 — internet a une coupure pendant follow-up

L'email follow-up peut attendre.  Tu envoies plus tard.  Ne PAS
laisser ta démo se distraire pour ça.

---

## 22. Post-demo cleanup

### 22.1 Immédiatement (T+2 min, encore dans la salle)

```bash
# Vérifier qu'aucun temp file n'est resté
ls /tmp/.efe-wm-* /tmp/.demo-test* 2>/dev/null
# Si quelque chose s'affiche :
mv /tmp/.efe-wm-* docs/lessons/monitoring/ 2>/dev/null

# Vérifier que les 7 tests PR #27 passent encore
cargo test -p efe-bench-harness --test g11_round_trip --quiet | tail -1
```

### 22.2 Quand tu sors de la salle (T+5 min)

```bash
# Kill caffeinate
kill ${CAFFEINATE_PID:-$(pgrep caffeinate | head -1)} 2>/dev/null

# Désactiver Focus "Demo"
# (manuel : Control Center → Focus → Demo Off)

# Re-ouvrir Slack, Mail, etc.
# (manuel ou : open -a Slack)

# Réactiver iCloud sync, Time Machine si désactivés
# tmutil enable
```

### 22.3 Backup du state final (pour debug post-mortem)

```bash
mkdir -p ~/Desktop/efe-demo-$(date -u +%Y%m%dT%H%M)
cd ~/Desktop/efe-demo-$(date -u +%Y%m%dT%H%M)

git -C /Users/rodrigue.pekar/Downloads/jjjj/efe log --oneline -10 > git-log.txt
git -C /Users/rodrigue.pekar/Downloads/jjjj/efe status --short > git-status.txt
cd /Users/rodrigue.pekar/Downloads/jjjj/efe && \
  cargo test -p efe-bench-harness --test g11_round_trip --quiet 2>&1 > ~/Desktop/efe-demo-$(date -u +%Y%m%dT%H%M)/pinning-test-post.txt

ls -la ~/Desktop/efe-demo-*
```

---

## 23. Annexe — Verified state of the demo machine (snapshot 2026-05-17)

| Composant | Valeur observée | Conformité |
|-----------|-----------------|------------|
| Hostname | MacBook-Pro-3.local | — |
| Kernel | Darwin 24.1.0 | — |
| macOS | 15.1.1 (Build 24B91) | ≥ 15.0 ✅ |
| Architecture | arm64 (Apple Silicon) | ARM64 ✅ |
| CPU | Apple M3 Pro, 11 cores | ≥ 8 ✅ |
| RAM | 36 GB | ≥ 32 GB ✅ |
| Disk total | 926 Gi | ≥ 500 Gi ✅ |
| Disk free | 13 Gi | ≥ 5 Gi ⚠️ tendu |
| target/ | 31 Gi | < 50 Gi ✅ |
| Rust | 1.88.0 | pinned via toolchain.toml ✅ |
| Cargo | 1.88.0 | conforme ✅ |
| Python 3.12 | /usr/local/bin/python3.12 (3.12.0) | requis ✅ |
| System Python 3 | /usr/bin/python3 (3.9.6) | trop vieux pour pyo3 ⚠️ |
| git | /usr/bin/git | présent ✅ |
| jq | /usr/bin/jq | présent ✅ |
| bash | /bin/bash (3.2.x) | présent ✅ (compatibility-tested for backfill script) |
| perl | /usr/bin/perl | présent ✅ |
| caffeinate | /usr/bin/caffeinate | présent ✅ |
| xattr | /usr/bin/xattr | présent ✅ |
| gtimeout | absent | utiliser perl workaround ⚠️ |
| timeout | absent | idem |

---

## 24. Cheat sheet en 1 page (à imprimer)

```
═══════════════════════════════════════════════════════════════════
  EFE ENVIRONNEMENT — CHEAT SHEET — 2026-05-17
═══════════════════════════════════════════════════════════════════

PRE-FLIGHT (T-30)

  cd /Users/rodrigue.pekar/Downloads/jjjj/efe
  export PYO3_PYTHON=python3.12
  export CARGO_TERM_COLOR=always
  export CARGO_INCREMENTAL=0
  export RUST_BACKTRACE=1
  export LC_ALL=en_US.UTF-8 LANG=en_US.UTF-8

  caffeinate -dimsu &              # empêche sleep
  CAFFEINATE_PID=$!

  # 7 tests PR #27 must PASS
  for t in g11_round_trip g11_tolerance_used_populated \
           stability_contract_v17 backfill_edge_cases \
           pr27_workflow_yaml_invariants s3_audit_chain_integrity \
           s9_s12_monitoring_chain_integrity ; do
    cargo test -p efe-bench-harness --test "$t" --quiet 2>&1 | tail -1
  done

  # binaries pre-built
  ls -la target/debug/efe-bench-harness target/debug/efe-bench
  ./target/debug/efe-bench-harness version

  # disk ≥ 5 Gi libres
  df -h ~ | awk 'NR==2{print "free:", $4}'

EDITOR (open these 5, in order)

  code docs/gaps/master_tracker.md \
       docs/lessons/PR-27-lessons.md \
       docs/lessons/monitoring/PR-27-closure-2026-08-07.md \
       bench/harness/tests/g11_round_trip.rs \
       bench/harness/tests/s9_s12_monitoring_chain_integrity.rs

TERMINAL (iTerm2, profile "Demo")

  font ≥ 18 pt, 120 col × 35 lines
  background : pure black
  green ANSI : vif (#33ff33)

NEVER DO LIST

  cargo clean              # 8+ min rebuild
  cargo update             # idem
  cargo test --workspace   # 8 min, 13 known failures
  efe-bench-harness list   # G-ENGINE-CB
  efe-bench-harness validate  # idem
  efe-server / -agent / controller / participant  # blocks
  backfill --write         # modifies fixtures

IF SOMETHING BREAKS

  1. Sourire. Narrer. Continuer.
  2. cd /Users/rodrigue.pekar/Downloads/jjjj/efe
  3. ls /tmp/.efe-wm-* — restore si présent
  4. cargo test -p efe-bench-harness --test g11_round_trip --quiet
     → si 9 PASS : tu reprends. Si pas : pivot vers Q&A.

PRESENTATION RECAP

  Hook (90s) → Master tracker (2m) → Pinning + freeze (3m)
  → Wow-moment audit chain (3m) → Backfill (2m) → Cumul (2m)
  → Closing (1m) → Q&A
  
  TOTAL CORE : 14:30 ; budget pause 30s ; hard ceiling 15 min.

═══════════════════════════════════════════════════════════════════
```

---

## 25. Trois-doc package recap

Pour la démo, **toujours** 3 docs en main :

| Doc | Quand consulter |
|-----|-----------------|
| `EFE-demo-script-2026-05-17.md` | Pour la narration acte-par-acte |
| `EFE-demo-contournements-2026-05-17.md` | Quand quelque chose casse, ou pendant Q&A |
| `EFE-demo-environnement-2026-05-17.md` | Avant la démo (setup + pre-flight) |

Total : 3 docs, ~ 2,700 lignes de référence opérationnelle.
Imprimer cheat-sheets des § 24 de chaque doc (3 pages A4).

---

_Bonne démo.  L'environnement est maintenant complet et testé._
