# Lettre A — CLI one-shot · Architecture détaillée

> **Statut** : Draft v1.0.0 — Document de référence pour l'adoption de niveau A du Mode 1 EFE.
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-19
> **Dépendances doc** : `MODE1_ARCHITECTURE.md`, `M10_ARCHITECTURE.md`, `M11_ARCHITECTURE.md`.
> **Portée** : l'interface en ligne de commande `efe-admin` telle qu'exposée par le package Python `efe-tool`, pour l'usage « one-shot » (une invocation → une réponse).

---

## Table des matières

1. Positionnement et contrats
2. État actuel (AS-IS) — audit du code en place
3. Cas d'usage canoniques et anti-usages
4. Vue d'ensemble architecturale
5. Spécification CLI cible (TO-BE)
6. Flux d'exécution détaillé
7. Résolution de configuration en couches
8. Contrats d'entrée / sortie (stdin, stdout, stderr)
9. Schéma de sortie stable et versionné
10. Taxonomie d'erreurs et codes de sortie
11. Budget de performance
12. Sécurité
13. Observabilité et télémétrie
14. Stratégie de tests
15. Packaging et distribution
16. Versioning et compatibilité
17. Ergonomie et UX
18. Dépendances inter-modules EFE
19. Plan d'implémentation détaillé (AS-IS → TO-BE)
20. Matrice de risques
21. Critères Go / No-Go spécifiques à A
22. Annexes

---

## §1. Positionnement et contrats

### §1.1 Définition opérationnelle de A

La **Lettre A — CLI one-shot** est la forme d'intégration d'EFE Mode 1 qui satisfait la propriété suivante, et **uniquement** celle-ci :

> *Étant donné une machine sur laquelle `efe-server` tourne sur un port gRPC connu, un utilisateur humain tape une commande unique dans un terminal et reçoit, en moins de deux secondes en régime chaud, un verdict de classification structuré sur stdout, sans avoir écrit une seule ligne de code Python.*

Cette propriété a trois corollaires invariants :

- **C1 — Aucun code utilisateur.** L'utilisateur de A n'importe rien, ne sous-classe rien, ne monkey-patche rien. Son seul « code » est la ligne de commande elle-même.
- **C2 — Aucune persistance entre invocations.** Chaque invocation est un processus neuf. Pas de session, pas de cache dans le CLI, pas de connexion gRPC réutilisée d'un appel à l'autre.
- **C3 — Sortie machine-parseable par défaut.** Le stdout par défaut est un document JSON stable, afin que `efe-admin classify … | jq '.label'` fonctionne sans flag particulier.

### §1.2 Place dans le chemin A → E

| Lettre | Forme | Ce qui la distingue strictement de A |
|---|---|---|
| **A** | CLI one-shot | — (lettre de référence) |
| B | REPL | Persiste un processus entre requêtes ; recharge un catalog à chaud |
| C | Script applicatif | Enchaîne EFE + un LLM hôte ; impose Python et un code utilisateur |
| D | Intégration lib | EFE partage le process d'une app tierce ; cycle de vie couplé |
| E | Service backend | EFE est un microservice ; cible des clients multi-langages |

A est la seule lettre qui suppose zéro changement dans l'écosystème du consommateur (pas de venv applicatif, pas de dépendance Python dans son projet, pas d'infra). Il ne demande que deux choses : un shell et une adresse gRPC.

### §1.3 Contrats externes

A expose trois surfaces contractuelles, toutes stabilisées par la présente architecture :

- **Surface CLI** : le nom des subcommands, le nom et la sémantique des flags, leurs valeurs par défaut. Toute rupture de cette surface incrémente la version majeure du package `efe-tool`.
- **Surface JSON de sortie** : la liste des champs du document retourné sur stdout, leur type, leur valeur de défaut en cas d'absence. Versionnée indépendamment via `schema_version` (voir §9).
- **Surface des codes de sortie** : l'ensemble fini des exit codes et leur signification. Immuable une fois publiée (§10).

Ces trois surfaces définissent l'« API de A ». Tout le reste (logs stderr, ordre des lignes en mode pretty, couleurs) est explicitement **non-contractuel** et peut évoluer en patch.

### §1.4 Principes invariants de A

- **P1 — JSON d'abord.** stdout est toujours machine-parseable par défaut. Le mode humain existe mais doit être demandé (`--format text`).
- **P2 — stderr n'est jamais stdout.** Les logs, diagnostics, warnings vont exclusivement sur stderr. stdout ne contient que le résultat ou rien.
- **P3 — Zéro magie.** Pas de détection implicite de catalog, pas de config cachée dans `$HOME`. Toute source de config est explicite (flag, env, ou fichier passé en flag).
- **P4 — Lazy par défaut.** Aucun import lourd (sentence-transformers, grpcio) ne se produit avant la fin du parsing des arguments.
- **P5 — Un binaire, un job.** A ne fait pas de reporting, pas de dashboards, pas d'orchestration. Il classifie un texte, ou il échoue.

---

## §2. État actuel (AS-IS) — audit du code en place

Cette section décrit exactement ce qui existe au moment de la rédaction. Chaque fait est vérifiable dans `clients/python/efe_tool/cli.py`.

### §2.1 Commande exposée

Entrée dans `pyproject.toml` :

```toml
[project.scripts]
efe-admin = "efe_tool.cli:main"
```

**Constat 1** — La commande s'appelle `efe-admin`, pas `efe`. Le raccourci `efe` n'existe pas en AS-IS. La roadmap présentée dans `EFE_MVP_Roadmap_Lettres.pdf` assumait `efe classify` ; c'est un écart à réconcilier (§19).

### §2.2 Subcommands existants

| Subcommand | Fonction | Maturité |
|---|---|---|
| `compile-catalog PATH [--embedder MODEL]` | Encode les canonical sentences, écrit `.compiled.json`, imprime un résumé. | Fonctionnel, utilisé en démo. |
| `classify TEXT --catalog PATH [--endpoint HOST:PORT]` | Classe un texte unique via le serveur EFE. | Fonctionnel ; c'est la surface principale de A. |
| `eval --catalog PATH --golden PATH --endpoint …` | Lance une évaluation sur un golden set et imprime des métriques. | Utilisé en bench ; n'est pas strictement « A ». |
| `healthcheck --endpoint HOST:PORT` | Sonde la santé gRPC. | Fonctionnel, binaire (SERVING / NOT_SERVING). |

**Constat 2** — `eval` appartient plutôt à l'outillage de qualité (CI bench, gate de régression), mais il est packagé avec le CLI A faute de sous-commande dédiée. Il reste ici **par convention** tant qu'aucune lettre distincte n'est créée.

### §2.3 Flux `classify` détaillé

Code trait pour trait :

```python
def _cmd_classify(args):
    catalog = AnchorCatalog.load(args.catalog, embedder=LocalSTEmbedder(args.embedder))
    os.environ.setdefault("EFE_ENDPOINT", args.endpoint)
    os.environ["EFE_TIMEOUT_S"] = str(args.timeout_s)
    client = EfeClient.from_env()
    try:
        result = client.classify(
            args.text, catalog,
            return_sub_scores=True,
            budget_wall_clock_ms=args.budget_ms,
            max_rounds=args.max_rounds,
        )
    finally:
        client.close()
    out = result.as_debug_dict() if args.verbose else result.as_tool_result()
    print(json.dumps(out, indent=2, ensure_ascii=False))
    return 0
```

Observations précises :

- **O1 — `LocalSTEmbedder` est instancié à chaque appel.** Coût typique 150-300 ms (Torch warm-up + modèle depuis le cache disque). Pour A c'est acceptable (one-shot), mais cela implique que `compile-catalog` doit pouvoir être exécuté une fois à froid pour générer le `.compiled.json` — sinon chaque `classify` paiera le coût de compilation en plus.
- **O2 — L'endpoint est injecté via variable d'environnement.** `os.environ.setdefault("EFE_ENDPOINT", args.endpoint)` suit la précédence « env existante > flag CLI ». C'est **l'inverse** de la précédence attendue par P3 (flag explicite gagne toujours). À corriger (§7).
- **O3 — `EFE_TIMEOUT_S` est écrasé sans setdefault.** Ici le flag CLI gagne, mais sans homogénéité avec O2. Incohérence à harmoniser.
- **O4 — `return_sub_scores=True` est toujours demandé côté client.** Cela surcharge la réponse. En mode non-verbose, `as_tool_result()` filtre ces champs. Acceptable mais inefficace : on paye le transport d'informations qu'on jette.
- **O5 — Le JSON est toujours indenté (`indent=2`).** Casse la propriété « stdout par défaut est compact » attendue par les outils Unix. À inverser : compact par défaut, `--pretty` pour l'indenté.
- **O6 — Aucun schema_version dans la sortie.** La stabilité du schéma n'est pas observable par le consommateur.
- **O7 — `--verbose` mélange deux concepts** : verbosité des logs (stderr) et richesse de la sortie machine (stdout). Il faut les séparer en `--log-level` et `--format debug` ou `--with-diagnostics`.

### §2.4 Gestion d'erreurs AS-IS

```python
try:
    return int(args.func(args))
except BridgeError as e:
    print(f"error: {e}", file=sys.stderr)
    return 2
except FileNotFoundError as e:
    print(f"file not found: {e}", file=sys.stderr)
    return 2
```

**Constat 3** — Tous les échecs renvoient exit 2, sans discrimination. Un fichier de catalog absent (erreur utilisateur), un serveur injoignable (erreur réseau), un dépassement de budget (condition métier) renvoient le même code. C'est le principal défaut d'observabilité de l'AS-IS et le premier chantier (§10).

### §2.5 Flags globaux AS-IS

- `--version` : imprime `efe-admin 0.1.0`.
- Pas de `--log-level`, pas de `--log-format`, pas de `--quiet`, pas de `--no-color`, pas de `--request-id`.
- Pas de completions shell générées.

### §2.6 Synthèse AS-IS

Le CLI actuel est **fonctionnel** mais pas encore « Go A » au sens de la roadmap :

| Critère | AS-IS | Gap |
|---|---|---|
| Nom de la commande | `efe-admin` | OK (le gap `efe` n'est que cosmétique) |
| Sortie JSON parseable | Oui mais indentée | Inverser : compact par défaut |
| Schema versionné | Non | Ajouter `schema_version: 1` |
| Exit codes disjoints | Non | §10 |
| Flags texte/stdin | Texte positionnel uniquement | Ajouter `--text-file`, `--stdin` |
| Logs structurés | `print(f"…")` | `--log-format json` |
| Lazy imports | Partiel | Complet (voir §11) |
| Tests dédiés CLI | Implicites (via bibliothèque) | Tests subprocess dédiés |

---

## §3. Cas d'usage canoniques et anti-usages

### §3.1 UC-1 — Debug développeur (« est-ce que ça marche ? »)

**Contexte.** Un dev vient de modifier `support_triage.yaml` et veut vérifier que le moteur produit toujours le bon label sur une phrase de référence.

**Commande typique.**

```bash
efe-admin classify "J'ai été débité sans consentement" \
  --catalog catalogs/support_triage.yaml
```

**Attentes.**
- Sortie JSON avec `label: "fraud"`.
- Latence humaine : moins de 5 s, idéalement moins de 2.
- Si `efe-server` n'est pas lancé, message d'erreur explicite avec hint (« avez-vous démarré le serveur avec `cargo run -p efe-server` ? »).

**Critères de succès.** Exit 0, label attendu, absence de stack trace sur stderr.

### §3.2 UC-2 — Smoke test CI

**Contexte.** Une GitHub Action veut, en fin de build, vérifier qu'une classification basique fonctionne avant de tagger une release.

```yaml
- name: EFE smoke test
  run: |
    efe-admin healthcheck --endpoint 127.0.0.1:50051
    efe-admin classify "The app crashes on login" \
      --catalog catalogs/support_triage.yaml \
      --format json \
      | jq -e '.label == "bug"'
```

**Attentes.**
- `healthcheck` retourne 0 si le serveur répond.
- `classify` est pipable dans jq.
- Exit code non-zéro propagé clairement si la classification rate.

**Critères de succès.** Le job échoue si et seulement si le système est réellement cassé — pas de flakiness liée à la CLI.

### §3.3 UC-3 — Démo tech 30 secondes

**Contexte.** Montrer à un collègue que le moteur existe. Zéro slide.

```bash
$ efe-server &
$ efe-admin compile-catalog catalogs/support_triage.yaml
OK — compiled catalog 'support_triage@1.0.0' …

$ efe-admin classify "Someone hacked my account" \
    --catalog catalogs/support_triage.yaml --pretty
{
  "label": "fraud",
  "confidence": 0.575,
  "reject": false,
  "rounds": 4,
  "schema_version": 1
}
```

**Attentes.** Pretty output lisible à l'écran. Commande compacte (pas plus de 80 caractères par ligne).

### §3.4 UC-4 — Gate pre-commit sur un catalog

**Contexte.** Un hook pre-commit refuse un commit si un catalog modifié ne classifie plus correctement ses phrases canoniques.

```bash
#!/usr/bin/env bash
set -euo pipefail
for phrase in "$(yq '.classes[].canonical_sentences[]' catalogs/support_triage.yaml)"; do
  efe-admin classify "$phrase" \
    --catalog catalogs/support_triage.yaml \
    --strict  # exit ≠ 0 si reject=true ou confidence < confidence_high
done
```

**Attentes.** Flag `--strict` qui transforme un reject en exit code 8, permettant aux hooks d'arrêter net.

### §3.5 UC-5 — Health probe Docker / Kubernetes

```yaml
readinessProbe:
  exec:
    command:
      - efe-admin
      - healthcheck
      - --endpoint
      - 127.0.0.1:50051
  initialDelaySeconds: 5
  periodSeconds: 10
```

**Attentes.**
- Exit 0 ⇔ SERVING, exit 1 sinon.
- Temps total < 1 s.
- Aucun side-effect (ne crée pas de fichier, ne modifie pas l'env).

### §3.6 UC-6 — Intégration shell pipeline

```bash
curl -s https://api.example.com/ticket/42 \
  | jq -r '.body' \
  | efe-admin classify --catalog catalogs/support_triage.yaml --stdin \
  | jq '.label'
```

**Attentes.** Lecture de stdin via `--stdin`. `stdout` pipeable sans bruit.

### §3.7 Anti-usages explicites

A ne doit **pas** être utilisé pour :

- **Production traffic.** Chaque invocation paie 200-400 ms de cold start Python + load de modèle. Le débit soutenable est de l'ordre de 2-5 classifications/s. Au-delà, passer à D (lib embarquée) ou E (service long-running).
- **Pipelines LLM.** Chaîner un LLM qui appelle `efe-admin` via `subprocess.run` à chaque tour est un anti-pattern : c'est la Lettre C (script Python avec bridge) qui est faite pour ça.
- **Calibration de catalog.** Le tuning interactif appelle B (REPL). A ne sait pas recharger un catalog à chaud.
- **Monitoring long-running.** A n'a pas de boucle, pas de metrics locales. Monitoring = M12 côté serveur.

---

## §4. Vue d'ensemble architecturale

### §4.1 Diagramme de flux (classify)

```
┌──────────────────────────────────────────────────────────────────────┐
│                         PROCESS efe-admin                            │
│                                                                      │
│  stdin ──► ┌─────────────┐                                           │
│            │  argparse   │◄── flags                                  │
│  args  ──► │   parser    │◄── env vars (EFE_*)                       │
│            └──────┬──────┘                                           │
│                   │                                                  │
│                   ▼                                                  │
│            ┌─────────────┐                                           │
│            │  config     │  precedence: flag > env > file > default  │
│            │  resolver   │                                           │
│            └──────┬──────┘                                           │
│                   │                                                  │
│                   ▼                                                  │
│            ┌─────────────┐         ┌──────────────┐                  │
│            │  catalog    │────────►│  .compiled   │  cache disk      │
│            │  loader     │◄────────│   .json      │                  │
│            └──────┬──────┘         └──────────────┘                  │
│                   │                                                  │
│                   ▼                                                  │
│            ┌─────────────┐                                           │
│            │  EfeClient  │─── gRPC ──► efe-server :50051             │
│            │  .classify  │◄── gRPC ───                               │
│            └──────┬──────┘                                           │
│                   │                                                  │
│                   ▼                                                  │
│            ┌─────────────┐                                           │
│            │  output     │── stdout ─► JSON (compact | pretty)       │
│            │  formatter  │── stderr ─► logs (text | json)            │
│            └──────┬──────┘                                           │
│                   │                                                  │
│                   ▼                                                  │
│                exit(n)                                               │
└──────────────────────────────────────────────────────────────────────┘
```

### §4.2 Boundaries

- **Boundary 1 — argparse ↔ resolver.** argparse ne fait que parser. Il ne lit pas l'env. La résolution de précédence est faite par un resolver dédié (§7).
- **Boundary 2 — resolver ↔ EfeClient.** Le resolver produit un `ResolvedConfig` immuable. Le client ne lit plus jamais l'env après ce point.
- **Boundary 3 — EfeClient ↔ serveur.** gRPC uniquement. Aucune fuite d'état. `client.close()` est appelé dans un `finally`.
- **Boundary 4 — result ↔ formatter.** Le formatter ne fait que formatter un `ClassifyResult`. Il ne décide rien (pas de politique de seuils).

### §4.3 Invariants architecturaux

- **I1.** Entre le début de `main()` et la fin du parsing argparse, aucun import de `grpcio`, `sentence_transformers`, `numpy`, `torch`, `anthropic`.
- **I2.** Le formatter et le resolver sont **purs** : mêmes entrées → mêmes sorties, aucune I/O.
- **I3.** Aucun `print()` n'envoie sur stdout en dehors du formatter.
- **I4.** Aucun chemin ne peut lever une exception sans passer par le handler central qui produit un exit code déterministe.

---

## §5. Spécification CLI cible (TO-BE)

### §5.1 Nom de commande et alias

- **Canonique** : `efe-admin` (préservé par compatibilité avec l'AS-IS et avec `[project.scripts]` de `pyproject.toml`).
- **Alias court** : `efe` ajouté en parallèle dans `[project.scripts]`. Point d'entrée identique (`efe_tool.cli:main`).
- **Recommandation** : la documentation utilise `efe-admin` dans les snippets copiables (pour la non-ambiguïté), mais `efe` en usage quotidien.

### §5.2 Flags globaux (avant le subcommand)

| Flag | Type | Default | Rôle |
|---|---|---|---|
| `--version` | flag | — | Imprime `efe-admin X.Y.Z` et exit 0. |
| `--log-level` | enum | `info` | `debug`, `info`, `warn`, `error`. Logs sur stderr. |
| `--log-format` | enum | `text` | `text` ou `json`. |
| `--quiet` / `-q` | flag | false | Raccourci : `--log-level error`. |
| `--no-color` | flag | false | Désactive les couleurs. Auto-désactivé si stdout n'est pas un TTY. |
| `--request-id` | string | `uuid4()` | Injecté en metadata gRPC pour corréler côté serveur. |
| `--endpoint` | string | `$EFE_ENDPOINT` ou `127.0.0.1:50051` | Adresse gRPC, global pour tous les subcommands qui en ont besoin. |

### §5.3 Subcommands cibles

#### §5.3.1 `classify`

```
efe-admin classify [--text TEXT | --text-file PATH | --stdin]
                   --catalog PATH
                   [--embedder MODEL]
                   [--budget-ms INT]
                   [--timeout-s FLOAT]
                   [--max-rounds INT]
                   [--strict]
                   [--format FMT]
                   [--pretty]
                   [--with-diagnostics]
```

Mutation par rapport à AS-IS : **le texte n'est plus positionnel**. Il devient un flag parmi trois sources mutuellement exclusives, ce qui permet le pipeline Unix propre (§8.1).

| Flag | Default | Env override | Description |
|---|---|---|---|
| `--text` | — | — | Texte inline. Mutuellement exclusif avec `--text-file` et `--stdin`. |
| `--text-file` | — | — | Lit le texte depuis un fichier UTF-8. |
| `--stdin` | false | — | Lit stdin jusqu'à EOF. |
| `--catalog` | — | `$EFE_CATALOG_PATH` | Chemin du catalog YAML. Requis. |
| `--embedder` | `paraphrase-multilingual-MiniLM-L12-v2` | `$EFE_EMBEDDER` | Doit matcher le catalog. |
| `--budget-ms` | 5000 | `$EFE_BUDGET_MS` | Budget swarm wall-clock. |
| `--timeout-s` | 10.0 | `$EFE_TIMEOUT_S` | Timeout gRPC client. Doit être ≥ budget-ms / 1000. |
| `--max-rounds` | 16 | `$EFE_MAX_ROUNDS` | Max rounds de fusion. |
| `--strict` | false | — | Exit code 8 si `reject=True` ou `confidence < confidence_high`. |
| `--format` | `json` | — | `json`, `text`, `table`. |
| `--pretty` | false | — | Alias : `--format json --pretty`. JSON indenté. |
| `--with-diagnostics` | false | — | Ajoute les champs de debug dans la sortie JSON. |

**Décision d'ergonomie** : les defaults de `--budget-ms` et `--timeout-s` sont **réduits** par rapport à l'AS-IS (60 s). La raison : A cible l'interactif humain ; 5 s de budget est suffisant pour 1024 agents, et un utilisateur qui attend plus de 10 s croit que la machine est plantée. Les tests de charge restent sur B ou des outils dédiés.

#### §5.3.2 `compile-catalog`

```
efe-admin compile-catalog PATH
                          [--embedder MODEL]
                          [--out PATH]
                          [--force]
                          [--format FMT]
```

Ajouts TO-BE :
- `--out` : écrit le `.compiled.json` ailleurs que à côté du YAML (utile pour des caches CI en lecture seule).
- `--force` : recompile même si le fingerprint est identique (debug).
- Sortie JSON structurée avec `--format json` (pour pipelines).

#### §5.3.3 `validate`

Nouveau subcommand. Vérifie qu'un catalog YAML est syntaxiquement et sémantiquement valide **sans lancer d'embedder**.

```
efe-admin validate PATH [--strict-thresholds]
```

Checks :
- YAML parseable.
- `apiVersion: efe.mode1/v1` et `kind: AnchorCatalog` présents.
- Au moins 2 classes ; chaque classe a au moins 1 canonical sentence.
- Thresholds : `0 < confidence_medium ≤ confidence_high < 1`.
- Avec `--strict-thresholds` : alerte si `confidence_high - confidence_medium < 0.05`.

Exit codes : 0 valide, 4 invalide.

#### §5.3.4 `healthcheck`

```
efe-admin healthcheck [--endpoint HOST:PORT]
                      [--format FMT]
                      [--timeout-s FLOAT]
```

Sortie default JSON (TO-BE) :

```json
{"status": "SERVING", "latency_ms": 12, "schema_version": 1}
```

#### §5.3.5 `eval`

Conservé tel quel (hors périmètre strict de A, mais packagé avec).

#### §5.3.6 `version` et `completions`

```
efe-admin version [--format FMT]          # idem --version mais comme subcommand
efe-admin completions {bash|zsh|fish}     # génère et imprime le script de complétion
```

### §5.4 Convention de nommage

- Flags long toujours `--kebab-case`.
- Flags court uniquement pour les 3 plus fréquents : `-q` (quiet), `-v` (verbose — à renommer, conflit avec l'AS-IS), `-h` (help).
- Pas de flag court ambigu : pas de `-t` (pourrait être timeout, text, type…).

---

## §6. Flux d'exécution détaillé

### §6.1 Phases chronologiques

```
phase 0 — python startup                    ~80 ms (cold), ~30 ms (warm bytecache)
phase 1 — argparse                          ~10 ms
phase 2 — config resolution                 <1 ms
phase 3 — lazy imports                      ~50 ms (grpcio, numpy)
phase 4 — catalog load                      ~50 ms (YAML parse + .compiled.json read)
                                            ~200 ms si compilation (une seule fois)
phase 5 — embedder load                     ~150 ms (cache) | ~2 s (première fois)
                                            SAUTÉE si le .compiled.json est valide
phase 6 — gRPC client init                  ~20 ms
phase 7 — RPC classify                      ~200-2000 ms (swarm)
phase 8 — output formatting                 <5 ms
phase 9 — cleanup (client.close)            ~5 ms
phase 10 — exit                             immédiat
────────────────────────────────────────────────────
cold path                                   ~500 ms + RPC
warm path (catalog déjà compilé, embedder en cache disque) ~300 ms + RPC
```

### §6.2 Budget cible par phase (TO-BE)

| Phase | Budget | Monitoring |
|---|---|---|
| 0 | < 100 ms | `time python -c "import efe_tool.cli"` |
| 1 | < 20 ms | benchmark argparse |
| 2 | < 2 ms | fonction pure, testable unitairement |
| 3 | < 80 ms | mesuré avec `importtime` |
| 4 | < 100 ms (cache hit) | log `catalog_load_ms` |
| 5 | SAUTÉ si cache | log `embedder_loaded: bool` |
| 6 | < 50 ms | log `grpc_channel_ms` |
| 7 | < 3000 ms (p95) | budget contrôlé par `--budget-ms` |
| 8 | < 10 ms | test snapshot |

### §6.3 Pseudocode canonique

```python
def main(argv: list[str] | None = None) -> int:
    try:
        args = parse_args(argv)             # phase 1
        cfg  = resolve_config(args, env=os.environ)  # phase 2
        setup_logging(cfg)                  # toujours avant les imports lourds
        lazy_imports()                      # phase 3, sauf pour --help/--version
        return dispatch(cfg)                # phase 4 → 9
    except UsageError as e:
        report_error(e); return 2
    except CatalogError as e:
        report_error(e); return 4
    except NetworkError as e:
        report_error(e); return 5
    except ServerError as e:
        report_error(e); return 6
    except TimeoutError as e:
        report_error(e); return 7
    except RejectedError as e:
        report_error(e); return 8
    except KeyboardInterrupt:
        report_error("interrupted"); return 130
    except Exception as e:                  # dernier recours
        report_bug(e); return 126
```

L'idée clé : **un exception family par exit code**, pas d'exception générique. `dispatch` lève toujours une exception typée.

---

## §7. Résolution de configuration en couches

### §7.1 Précédence

Du plus fort au plus faible :

1. **Flag CLI explicitement passé.**
2. **Variable d'environnement** (`EFE_*`).
3. **Fichier de configuration** (`--config path.toml`), optionnel et non-activé au MVP.
4. **Default compilé** dans le code.

### §7.2 Détection d'un flag « explicitement passé »

Piège classique : `args.endpoint` a toujours une valeur (le default argparse). Il faut distinguer « pas fourni » de « fourni avec le même contenu que le default ».

**Solution retenue** : deux passes.

```python
# Première passe : default = _MISSING
raw = parse(argv, default=_MISSING)
# Seconde passe : appliquer les defaults explicites après résolution env
resolved = {
    k: (raw[k] if raw[k] is not _MISSING else env.get(ENV_MAP[k], DEFAULTS[k]))
    for k in ALL_KEYS
}
```

### §7.3 Table canonique

| Clé logique | Flag CLI | Env var | Default |
|---|---|---|---|
| endpoint | `--endpoint` | `EFE_ENDPOINT` | `127.0.0.1:50051` |
| catalog_path | `--catalog` | `EFE_CATALOG_PATH` | (requis) |
| embedder_model | `--embedder` | `EFE_EMBEDDER` | `paraphrase-multilingual-MiniLM-L12-v2` |
| budget_ms | `--budget-ms` | `EFE_BUDGET_MS` | `5000` |
| timeout_s | `--timeout-s` | `EFE_TIMEOUT_S` | `10.0` |
| max_rounds | `--max-rounds` | `EFE_MAX_ROUNDS` | `16` |
| log_level | `--log-level` | `EFE_LOG_LEVEL` | `info` |
| log_format | `--log-format` | `EFE_LOG_FORMAT` | `text` |
| request_id | `--request-id` | `EFE_REQUEST_ID` | `uuid4()` |
| strict | `--strict` | `EFE_STRICT` | `false` |
| format | `--format` | `EFE_OUTPUT_FORMAT` | `json` |

### §7.4 Validation post-résolution

- `timeout_s >= budget_ms / 1000 + 0.5` (marge de 500 ms pour le transport). Sinon warning explicite et ajustement automatique de `timeout_s`.
- `max_rounds in [1, 256]`. Hors bornes → `UsageError`.
- `log_level in {debug, info, warn, error}`.
- `format in {json, text, table}`.
- `catalog_path` existe et est lisible.

### §7.5 Bug AS-IS à corriger

Dans le code actuel, `os.environ.setdefault("EFE_ENDPOINT", args.endpoint)` inverse la précédence : si `EFE_ENDPOINT` est déjà défini, le flag CLI est ignoré. **C'est en contradiction avec P3.** La correction est de retirer `setdefault` et d'utiliser un resolver explicite.

---

## §8. Contrats d'entrée / sortie

### §8.1 Entrée — trois sources mutuellement exclusives

```bash
# 1. inline
efe-admin classify --text "Ma phrase" --catalog x.yaml

# 2. fichier
efe-admin classify --text-file message.txt --catalog x.yaml

# 3. stdin
echo "Ma phrase" | efe-admin classify --stdin --catalog x.yaml
cat message.txt | efe-admin classify --stdin --catalog x.yaml
```

**Règle de combinaison** : exactement une des trois sources doit être fournie. Sinon `UsageError` (exit 2).

**Détection automatique** : si `--text` est absent et `stdin` n'est pas un TTY, `--stdin` est implicite. Cela permet `echo x | efe-admin classify --catalog …` sans `--stdin` explicite. Mais si stdin EST un TTY, on refuse plutôt que d'attendre indéfiniment.

**Taille max** : 100 000 caractères. Au-delà, erreur explicite (le modèle ne supporte pas). Warning dès 10 000 caractères.

**Encodage** : UTF-8 strict. Les entrées non-UTF-8 causent `UsageError` immédiat, pas un silent-mangle.

### §8.2 stdout — uniquement le résultat

stdout ne contient **rien d'autre** que le document de sortie (§9), suivi d'un unique `\n`. Jamais de log, jamais de prompt, jamais de progress bar.

En mode `--format json` (défaut) : une seule ligne, compact, sans indentation.
En mode `--format json --pretty` : JSON indenté à 2 espaces.
En mode `--format text` : tabulation humaine (anti-parseable). Ex. :

```
Label:       fraud
Confidence:  0.575
Rounds:      4
Reject:      no
```

En mode `--format table` : table ASCII simple (utile pour `eval` qui produit plusieurs lignes).

### §8.3 stderr — logs structurés

stderr reçoit tout le reste :

- En `--log-format text` (défaut), lignes humaines colorées si TTY :
  ```
  INFO  catalog loaded support_triage@1.0.0 (fingerprint=abc123, 4 classes)
  INFO  gRPC channel 127.0.0.1:50051 opened in 12 ms
  WARN  timeout_s (3.0) < budget_ms/1000 + 0.5, bumped to 5.5
  ```
- En `--log-format json`, un objet JSON par ligne :
  ```json
  {"ts":"2026-04-19T14:58:01Z","level":"info","msg":"catalog loaded","catalog":"support_triage","fingerprint":"abc123"}
  ```

### §8.4 Progress indicator

Option `--progress` (défaut : `auto`) :
- `auto` : affiche un spinner sur stderr uniquement si stderr est un TTY **et** que l'opération dure plus de 500 ms.
- `never` : jamais.
- `always` : toujours (utile en démo).

Le spinner n'apparaît pas sur stdout — jamais.

### §8.5 Terminaison

- Le process termine après un unique exit code.
- Aucun thread démon résiduel (validé par test).
- `client.close()` est toujours appelé (via `finally`).

---

## §9. Schéma de sortie stable et versionné

### §9.1 Document canonique

```json
{
  "schema_version": 1,
  "label": "fraud",
  "confidence": 0.575,
  "reject": false,
  "alignment_delta": 0.12,
  "rounds": 4,
  "catalog": {
    "name": "support_triage",
    "version": "1.0.0",
    "fingerprint": "sha256:…"
  },
  "timing": {
    "elapsed_ms": 842,
    "server_ms": 798
  },
  "request_id": "018f8d3c-9b1d-7b21-8e83-aefc98e2d7e0"
}
```

### §9.2 Champs

| Champ | Type | Obligatoire | Commentaire |
|---|---|---|---|
| `schema_version` | int | **oui** | Entier. Incrémenté à chaque breaking change. Permet au consommateur de gate. |
| `label` | string | oui | Nom exact de la classe gagnante. |
| `confidence` | float | oui | ∈ [0, 1]. |
| `reject` | bool | oui | Vrai si confidence < confidence_medium. |
| `alignment_delta` | float | oui | Écart au cône (distance normalisée). |
| `rounds` | int | oui | Nombre de rounds de fusion consommés. |
| `catalog.name` | string | oui | Échoing utile pour le consommateur. |
| `catalog.version` | string | oui | SemVer du catalog. |
| `catalog.fingerprint` | string | oui | Empreinte SHA-256 du catalog compilé. |
| `timing.elapsed_ms` | int | oui | Mesure côté client, du début d'envoi à la fin de réception. |
| `timing.server_ms` | int | oui | Tel que rapporté par le serveur dans sa réponse. |
| `request_id` | string | oui | UUID, permet la corrélation avec les logs serveur. |
| `sub_scores` | object | non | Présent seulement avec `--with-diagnostics`. Dict `class → score`. |
| `swarm_trace` | object | non | Présent avec `--with-diagnostics`. Détails de convergence. |

### §9.3 Sortie d'erreur structurée

En cas d'erreur, si `--format json`, stderr contient un document JSON :

```json
{
  "schema_version": 1,
  "error": {
    "code": "CATALOG_NOT_FOUND",
    "exit_code": 3,
    "message": "catalog file does not exist: catalogs/foo.yaml",
    "hint": "did you mean catalogs/support_triage.yaml?",
    "cause": {
      "type": "FileNotFoundError",
      "detail": "[Errno 2] No such file or directory: 'catalogs/foo.yaml'"
    }
  },
  "request_id": "018f8d3c-…"
}
```

### §9.4 Politique de versioning du schéma

- **Patch** (`schema_version` inchangé) : ajout d'un champ optionnel, amélioration de `hint`.
- **Mineur** (schema_version inchangé, version CLI bumpée) : ajout d'un champ optionnel avec valeur par défaut stable.
- **Majeur** (schema_version = 2) : suppression ou renommage d'un champ obligatoire, changement de type.

Chaque breaking change donne lieu à :
1. Un flag `--schema-version N` pour les consommateurs qui n'ont pas migré (supporté pendant 2 versions).
2. Une entrée dans `CHANGELOG.md`.
3. Un warning en version majeure + 1 : « schema v1 déprécié, passez à v2 ».

---

## §10. Taxonomie d'erreurs et codes de sortie

### §10.1 Table exhaustive

| Code | Nom symbolique | Classe | Déclencheur |
|---|---|---|---|
| 0 | OK | — | Classification réussie, y compris `reject=true` (sauf `--strict`). |
| 2 | USAGE | UsageError | Flag manquant, combinaison invalide, valeur hors domaine. |
| 3 | CATALOG_NOT_FOUND | CatalogError | Fichier catalog absent. |
| 4 | CATALOG_INVALID | CatalogError | YAML malformé, schéma non conforme, embedder mismatch. |
| 5 | SERVER_UNREACHABLE | NetworkError | gRPC UNAVAILABLE / connexion refusée. |
| 6 | SERVER_ERROR | ServerError | gRPC INTERNAL, RESOURCE_EXHAUSTED hors budget. |
| 7 | TIMEOUT | TimeoutError | DEADLINE_EXCEEDED côté client ou budget dépassé côté serveur. |
| 8 | REJECTED | RejectedError | `reject=true` **et** `--strict`. |
| 9 | LOW_CONFIDENCE | RejectedError | `confidence < confidence_high` **et** `--strict`. |
| 126 | BUG | — | Exception non catchée (bug à reporter). |
| 130 | INTERRUPTED | — | SIGINT (Ctrl-C). |

### §10.2 Règles de mapping

- Deux codes ne doivent jamais cohabiter : l'erreur la plus spécifique gagne.
- `CATALOG_INVALID` (4) englobe « catalog fingerprint ≠ embedder fingerprint » (cas particulier documenté dans le message).
- `SERVER_ERROR` (6) n'est jamais retourné pour un timeout ; timeout = 7.
- `BUG` (126) ne doit jamais être vu en production. Son apparition déclenche une alerte dans les tests.

### §10.3 Message d'erreur

Format texte (stderr, `--log-format text`) :

```
error: CATALOG_NOT_FOUND
  file: catalogs/foo.yaml
  hint: did you mean catalogs/support_triage.yaml?
  request_id: 018f8d3c-9b1d-7b21-8e83-aefc98e2d7e0
```

Format JSON : voir §9.3.

### §10.4 Compatibilité POSIX `sysexits(3)`

Les codes 2-9 ne collent pas strictement aux `sysexits`, mais ils respectent la convention :
- 2 pour erreur d'usage (comme argparse lui-même).
- 64–78 range POSIX réservé pour des extensions futures si besoin.
- 126 pour « commande non exécutable » (étendu ici à « bug interne »).
- 130 pour SIGINT (128 + 2).

---

## §11. Budget de performance

### §11.1 Cold start — décomposition mesurée

Mesure de référence sur MacBook Pro M3 Pro, cache disque chaud, venv activé :

```
python -c "pass"                                       28 ms
python -c "import efe_tool.cli"                       180 ms  (← OPTIMISABLE)
python -c "from efe_tool.client import EfeClient"     410 ms
python -c "from efe_tool.embedder import LocalSTEmbedder"  1 800 ms
```

L'import de `sentence_transformers` coûte ~1.5 s à froid. Le CLI ne doit l'importer **que si on le compile** (subcommand `compile-catalog` avec cache manquant).

### §11.2 Optimisations lazy-import

Dans `cli.py` :

```python
# Top of file — uniquement stdlib
import argparse, json, os, sys, uuid
from pathlib import Path

def main(argv=None):
    args = parse_args(argv)
    cfg  = resolve_config(args)
    setup_logging(cfg)

    # Tardif : on n'importe que ce qui est nécessaire au subcommand
    if args.cmd == "classify":
        from efe_tool.catalog import AnchorCatalog
        from efe_tool.client import EfeClient
        # Embedder seulement si le cache .compiled.json est absent
        if not AnchorCatalog.has_compiled_cache(args.catalog):
            from efe_tool.embedder import LocalSTEmbedder
            embedder = LocalSTEmbedder(cfg.embedder_model)
        else:
            embedder = None
        ...
```

Cette disposition garde `--version` et `--help` sous 80 ms (cold) et `classify` avec cache chaud sous 300 ms.

### §11.3 Benchmark CI

Un test bench (hors coverage) mesure :

```python
def test_cold_start_help(benchmark):
    result = benchmark(subprocess.run, ["efe-admin", "--help"], capture_output=True)
    assert result.returncode == 0
```

Threshold : `stats.mean < 150 ms` sur Linux x86_64 CI.

### §11.4 Hot-path classification

Pour un catalog compilé en cache :
- Load catalog : 50 ms (lecture `.compiled.json`, ~200 lignes JSON).
- gRPC init : 20 ms (canal + stub).
- Classify RPC : 200–2000 ms selon budget serveur.
- Output : < 5 ms.

Total : **280–2080 ms** sur une requête réelle. Le p95 observé en validation MVP est de **850 ms** sur fraud FR.

---

## §12. Sécurité

### §12.1 Exposition du texte classifié

- **Risque** : `efe-admin classify --text "mon mot de passe est xyz"` inscrit le texte dans l'historique shell et dans la table des processus (`ps`, `/proc/*/cmdline`).
- **Mitigation 1** : détection heuristique de patterns sensibles (regex sur `password`, `api[_-]?key`, `token`, nombres de 16 chiffres etc.) → warning sur stderr.
- **Mitigation 2** : recommandation ferme dans la doc : utiliser `--stdin` ou `--text-file` pour tout texte réel.
- **Mitigation 3** (V1.1) : flag `--no-history` qui injecte un préfixe espace (heuristique bash / zsh pour ne pas historiser).

### §12.2 Secrets

A **ne manipule aucun secret**. En particulier :
- Pas d'appel Claude → pas d'`ANTHROPIC_API_KEY` requis.
- Pas d'auth gRPC dans le MVP (le serveur est local).
- En V1.1, si le serveur exige mTLS ou JWT, les secrets viennent de `$EFE_TOKEN` / fichier monté, jamais de flag.

### §12.3 Logs et redaction

- Par défaut, `--log-level info` ne log **jamais** le texte classifié. Il log la longueur et un hash court.
- `--log-level debug` log le texte, avec un warning « texte enregistré en clair dans les logs » en première ligne.

### §12.4 Connexion gRPC

- Par défaut, `insecure_channel` (plaintext). C'est sûr en local seulement.
- Flag `--tls` (TO-BE) : active `secure_channel(creds)` avec CA bundle système.
- Flag `--tls-ca PATH` : bundle custom.
- Rappel doc : si l'endpoint n'est pas 127.0.0.1, **toujours** passer `--tls`.

### §12.5 Catalog non fiable

- Un catalog YAML arbitraire est **traité comme de la donnée**, pas du code. Pas de `yaml.load` non safe (utiliser `yaml.safe_load`).
- Les noms de classes sont validés (regex `^[a-z][a-z0-9_]{0,31}$`) pour empêcher les injections dans les logs / la sortie.

---

## §13. Observabilité et télémétrie

### §13.1 Logs — format text

```
2026-04-19T14:58:01.234Z  INFO  argparse_done
2026-04-19T14:58:01.240Z  INFO  config_resolved endpoint=127.0.0.1:50051 catalog=catalogs/support_triage.yaml
2026-04-19T14:58:01.310Z  INFO  catalog_loaded name=support_triage fingerprint=sha256:abc fp_age_days=2
2026-04-19T14:58:01.330Z  INFO  grpc_channel_opened endpoint=127.0.0.1:50051 ms=18
2026-04-19T14:58:02.120Z  INFO  classify_done label=fraud confidence=0.575 rounds=4 server_ms=787
```

### §13.2 Logs — format JSON

```json
{"ts":"2026-04-19T14:58:01.234Z","lvl":"info","evt":"argparse_done","request_id":"018f8d3c-…"}
{"ts":"2026-04-19T14:58:01.310Z","lvl":"info","evt":"catalog_loaded","catalog":"support_triage","fingerprint":"sha256:abc","request_id":"018f8d3c-…"}
```

Propriétés :
- Une ligne = un objet JSON (ndjson).
- Clés stables : `ts`, `lvl`, `evt`, `request_id`, plus les clés spécifiques à l'événement.
- Pas de `msg` en format libre : chaque log est un événement nommé (`evt`).

### §13.3 Propagation du request_id

`--request-id` (ou auto-UUID) est injecté :
- Dans chaque ligne de log (champ `request_id`).
- Dans les métadonnées gRPC (header `x-efe-request-id`), pour corrélation côté serveur.
- Dans le document de sortie (§9).

### §13.4 Télémétrie anonyme

**Non au MVP.** A n'envoie rien en dehors de la machine locale. La discussion d'un endpoint `efe-telemetry` est reportée post-V1.

### §13.5 Métriques locales

A étant one-shot, il ne publie pas de metrics Prometheus. Le couplage monitoring se fait côté serveur (M12).

---

## §14. Stratégie de tests

### §14.1 Pyramide

```
        ▲  e2e subprocess       ~10 tests
       ╱ ╲ (vrai serveur)
      ╱   ╲
     ╱     ╲ integration        ~30 tests
    ╱───────╲ (subprocess + mock gRPC)
   ╱         ╲
  ╱           ╲ unit              ~60 tests
 ╱─────────────╲ (parse, resolve, format, error)
```

### §14.2 Tests unitaires

Cibles : `parse_args`, `resolve_config`, `format_output`, `map_error`, tous les pydantic / dataclass validators.

```python
def test_resolve_config_flag_beats_env(monkeypatch):
    monkeypatch.setenv("EFE_ENDPOINT", "from-env:1111")
    args = parse_args(["classify", "--text", "x",
                      "--catalog", "c.yaml",
                      "--endpoint", "from-flag:2222"])
    cfg = resolve_config(args, env=os.environ)
    assert cfg.endpoint == "from-flag:2222"
```

### §14.3 Tests d'intégration (subprocess + mock serveur)

Lancement d'un mock gRPC `Mode1Server` qui retourne une `ClassifyResponse` canonique, puis subprocess :

```python
def test_classify_happy_path(mock_server, tmp_catalog):
    res = subprocess.run(
        ["efe-admin", "classify", "--text", "hello",
         "--catalog", str(tmp_catalog), "--endpoint", mock_server.endpoint],
        capture_output=True, text=True)
    assert res.returncode == 0
    payload = json.loads(res.stdout)
    assert payload["label"] == "bug"
    assert payload["schema_version"] == 1
```

### §14.4 Tests e2e (vrai serveur, optionnels en CI)

Gatekeeper : `RUN_E2E=1`. Nécessite `efe-server` lancé (fixture autouse possible via `subprocess.Popen`).

### §14.5 Tests snapshot

- `efe-admin --help` stdout comparé à `tests/snapshots/help.txt`.
- `efe-admin classify --help` idem.

Le snapshot de `--help` sert de garde-fou contre les breaking changes non documentés.

### §14.6 Tests fuzz

Faible priorité mais prévu : `hypothesis` sur les combinaisons de flags, vérifie qu'aucune combinaison n'atterrit en `BUG` (exit 126).

### §14.7 Couverture cible

- `cli.py` : ≥ 85 % lignes, ≥ 80 % branches.
- Exclus de la couverture : le bloc `if __name__ == "__main__"` (marquer `# pragma: no cover`).

---

## §15. Packaging et distribution

### §15.1 Entry points pyproject

```toml
[project.scripts]
efe-admin = "efe_tool.cli:main"
efe       = "efe_tool.cli:main"   # TO-BE, alias court
```

### §15.2 Wheel et sdist

- Build via `python -m build`.
- sdist contient `catalogs/support_triage.yaml` comme data d'exemple (sous `efe_tool/data/`).
- Wheel universel (`py3-none-any`).

### §15.3 Dépendances runtime

Héritées de `efe-tool` : `grpcio`, `grpcio-tools`, `protobuf`, `pyyaml`, `numpy`, `sentence-transformers`.

**Observation** : `sentence-transformers` est lourd (~500 MB avec modèle). Pour l'usage A pur (pas de compile-catalog), on pourrait le reléguer en extra optionnel :

```toml
[project.optional-dependencies]
compile = ["sentence-transformers>=2.7", "numpy>=1.26"]
```

Et faire un import conditionnel avec un message d'erreur clair si manquant. Gain : installation 10× plus légère pour les usages A qui n'ont qu'un `.compiled.json` déjà généré (cas CI).

### §15.4 Homebrew (V1.1)

Formule `efe-admin.rb` minimale :

```ruby
class EfeAdmin < Formula
  include Language::Python::Virtualenv
  desc "Ephemeral Fluid Swarm — CLI client"
  homepage "https://github.com/efe-project/efe"
  url "https://files.pythonhosted.org/packages/source/e/efe-tool/efe-tool-1.0.0.tar.gz"
  sha256 "…"
  depends_on "python@3.12"
  def install
    virtualenv_install_with_resources
  end
end
```

### §15.5 Docker minimal

```dockerfile
FROM python:3.12-slim
RUN pip install --no-cache-dir efe-tool
ENTRYPOINT ["efe-admin"]
```

Image ~150 MB sans le modèle. Avec modèle preload : prévoir stage multi et copy des caches.

### §15.6 Completions shell

Via `shtab` (génère argparse → bash/zsh/fish) ou implémentation manuelle.

```bash
efe-admin completions bash > ~/.bash_completion.d/efe-admin
```

---

## §16. Versioning et compatibilité

### §16.1 Axes de version

- **Version du package** `efe-tool` : SemVer. Distribuée sur PyPI.
- **Version du CLI** : `efe-admin --version` imprime `efe-admin X.Y.Z`. Tracké avec le package.
- **Version du schéma de sortie** : `schema_version` dans le JSON. Incrémenté **uniquement** à la rupture.
- **Version du protobuf** (`efe.mode1.v1`) : namespace dans le `.proto`. Incrément = nouveau package.

### §16.2 Garanties

- **Cas CLI majeur** (`efe-tool 1.0 → 2.0`) : possible suppression de subcommand, renommage de flag, changement de default. Changelog explicite.
- **Cas CLI mineur** : ajout de subcommand, ajout de flag non-breaking.
- **Cas CLI patch** : bugfix, perf.

### §16.3 Dépréciation progressive

Flag ou subcommand à retirer :

1. Version N : annonce dans `--help` (« deprecated, use X »).
2. Version N+1 : warning sur stderr à chaque usage.
3. Version N+2 : suppression.

### §16.4 Politique d'environnement

- Si `EFE_STRICT_COMPAT=1`, le CLI refuse un `schema_version` ≠ 1 sur le résultat serveur (gate pour les consommateurs paranoïaques).

---

## §17. Ergonomie et UX

### §17.1 Aide

- `efe-admin --help` : vue globale, liste des subcommands.
- `efe-admin classify --help` : détail d'un subcommand, flags triés par fréquence d'usage (les plus utiles en premier).
- Exemples en fin de chaque `--help` :
  ```
  Examples:
    efe-admin classify --text "Ma carte a été piratée" --catalog catalogs/support_triage.yaml
    echo "hello" | efe-admin classify --stdin --catalog catalogs/support_triage.yaml
  ```

### §17.2 Suggestions en cas d'erreur

En `UsageError` typo :

```
$ efe-admin classif --text x --catalog c.yaml
error: unknown subcommand 'classif'
  did you mean 'classify'?
```

Implémenté via `difflib.get_close_matches`.

### §17.3 Couleurs

- Utilise `colorama` ou les codes ANSI manuels.
- Auto-détection TTY : `sys.stderr.isatty()`.
- `--no-color` ou `NO_COLOR=1` (standard) désactive tout.
- Palette restreinte : rouge pour erreur, jaune pour warning, bleu pour info. Pas de bleu foncé sur fond noir.

### §17.4 Progression

Spinner (`|/-\`) uniquement pour les opérations > 500 ms. Effacé en fin.

### §17.5 Internationalisation

- Messages CLI en **anglais** uniquement au MVP. Les catalogs, eux, peuvent être multilingues.
- Justification : le public cible du CLI est international ; une localisation en FR uniquement serait contre-productive.
- Hook d'i18n prévu (`gettext`) pour V1.2 si demande.

### §17.6 Raccourcis clavier dans l'aide

`less`-like paging si stderr est TTY et `--help` est long (> 50 lignes) : `click` gère ça nativement ; avec argparse, à implémenter via `os.environ.get("PAGER", "less")` pipé.

---

## §18. Dépendances inter-modules EFE

### §18.1 Graphe

```
           ┌────────────────────┐
           │  A = cli.py        │
           │  (efe_tool.cli)    │
           └────┬──────┬────────┘
                │      │
      ┌─────────┘      └──────────┐
      │                           │
      ▼                           ▼
 M11 client.py             M1/M2 catalog+embedder
      │                           │
      ▼                           │
 proto stubs                      │
      │                           │
      ▼                           │
 M10 gRPC service ◄───────────────┘
      │
      ▼
 M5–M8 swarm pipeline
      │
      ▼
 M12 observabilité (serveur)
```

### §18.2 Hard deps (bloquants)

- **M11 client** (`EfeClient.from_env`, `classify`, `close`). Sans M11, pas de CLI.
- **M1 catalog** (`AnchorCatalog.load`, `.compile`, cache `.compiled.json`).
- **M2 embedder** (pour `compile-catalog`).
- **M10 service gRPC** + stubs protobuf (runtime).

### §18.3 Soft deps

- **M9 admin** : seulement si on ajoute `--use-admin` pour récupérer un catalog distant (V1.1).
- **M12 observabilité** : seulement si `--request-id` propage jusqu'au serveur, déjà prévu.

### §18.4 Anti-dépendances

A ne dépend **jamais** de :
- `anthropic` (réservé aux bridges → C).
- Frameworks applicatifs (Django, FastAPI, Flask) → D.
- Outils d'orchestration (Celery, RQ, …).

---

## §19. Plan d'implémentation détaillé (AS-IS → TO-BE)

### §19.1 Sprint S1 (2 jours) — Fondations de configuration

- **S1.1** Ajouter `resolve_config(args, env)` pur. Tests unitaires complets.
- **S1.2** Retirer les `os.environ.setdefault` de `_cmd_classify`. Lire depuis le `ResolvedConfig`.
- **S1.3** Ajouter flags globaux `--log-level`, `--log-format`, `--quiet`, `--no-color`, `--request-id`.
- **Gate S1** : test `test_resolve_config_flag_beats_env` vert.

### §19.2 Sprint S2 (1 jour) — Schéma de sortie stable

- **S2.1** Ajouter `schema_version: 1` dans le document de sortie.
- **S2.2** Séparer `--verbose` en `--with-diagnostics` (stdout) et `--log-level debug` (stderr).
- **S2.3** Inverser la politique JSON : compact par défaut, `--pretty` en opt-in.
- **S2.4** Ajouter `--format {json, text, table}`.
- **Gate S2** : snapshot tests verts sur les trois formats.

### §19.3 Sprint S3 (1 jour) — Taxonomie d'erreurs

- **S3.1** Définir les exceptions `UsageError`, `CatalogError`, `NetworkError`, `ServerError`, `TimeoutError`, `RejectedError`.
- **S3.2** Mapping gRPC → exception dans `EfeClient._map_grpc_error` (étendre).
- **S3.3** Handler central dans `main()` qui mappe exception → exit code.
- **S3.4** Document de sortie d'erreur JSON structuré.
- **S3.5** Flag `--strict` déclenche `RejectedError` si applicable.
- **Gate S3** : table `test_exit_codes` exhaustive, un test par code.

### §19.4 Sprint S4 (1 jour) — I/O et entrée flexible

- **S4.1** Remplacer `TEXT` positionnel par `--text / --text-file / --stdin`.
- **S4.2** Détection stdin TTY avec erreur claire si ambigu.
- **S4.3** Limite de taille 100 000 caractères, warning à 10 000.
- **S4.4** Migration path : accepter l'ancienne forme positionnelle avec warning de dépréciation (deprecated, sera retirée en 2.0).
- **Gate S4** : UC-6 (pipeline) vert en test e2e.

### §19.5 Sprint S5 (1 jour) — Observabilité

- **S5.1** Logger structuré (text + json) dans `efe_tool.logging`.
- **S5.2** Propagation `request_id` en gRPC metadata.
- **S5.3** Événements nommés (`argparse_done`, `catalog_loaded`, `classify_done`).
- **S5.4** Spinner conditionnel stderr.
- **Gate S5** : `efe-admin classify … --log-format json 2>log.ndjson` ; `log.ndjson` parseable ligne à ligne.

### §19.6 Sprint S6 (0.5 jour) — Packaging et completions

- **S6.1** Ajouter alias `efe` dans `[project.scripts]`.
- **S6.2** Subcommand `completions {bash,zsh,fish}` via shtab.
- **S6.3** Extra optionnel `compile` pour le modèle.
- **Gate S6** : `pip install efe-tool[compile]` fonctionne séparément.

### §19.7 Sprint S7 (1 jour) — Tests et CI

- **S7.1** Couverture ≥ 85 % sur `cli.py`.
- **S7.2** Tests e2e subprocess + mock server.
- **S7.3** Bench cold start dans CI.
- **S7.4** Snapshot tests pour `--help`.
- **Gate S7** : CI verte 5 runs consécutifs.

### §19.8 Sprint S8 (0.5 jour) — Documentation

- **S8.1** `README` section « CLI » réécrite.
- **S8.2** Page de doc dédiée `docs/cli.md` avec tous les subcommands.
- **S8.3** Exemples en UC-1 à UC-6 (copier depuis §3).
- **Gate S8** : `efe-admin --help` et la doc sont cohérents au mot près (test automatisé).

### §19.9 Total

**6.5 jours-homme**. Livré dans une pull request unique (découpage possible si conflit).

---

## §20. Matrice de risques

| ID | Risque | Probabilité | Impact | Mitigation | Détection |
|---|---|---|---|---|---|
| R-A1 | Texte sensible dans `ps` | Moyenne | Haut | Heuristique + doc + `--stdin` | Warning automatique, revue périodique |
| R-A2 | Embedder mismatch catalog ↔ CLI | Moyenne | Haut | Validation fingerprint au load | CatalogError exit 4 |
| R-A3 | `timeout_s < budget_ms/1000` | Haute | Moyen | Auto-ajustement + warning | Test unitaire |
| R-A4 | Breaking change du schéma non annoncé | Faible | Critique | `schema_version` + snapshot tests | Test snapshot dans CI |
| R-A5 | Imports lourds au cold start | Certaine (AS-IS) | Moyen | Lazy imports + bench CI | Bench CI avec seuil 150 ms |
| R-A6 | Logs qui leaktent le texte en debug | Moyenne | Haut | Redaction par défaut | Tests unitaires sur redaction |
| R-A7 | Subcommand `eval` ne fait pas partie de A | Faible | Bas | Packagé à part en V1.1 | Audit de surface |
| R-A8 | Nom `efe-admin` perçu comme admin-only | Moyenne | Bas | Alias `efe`, doc | Feedback usage |
| R-A9 | Exit codes non documentés côté ops | Moyenne | Moyen | Table exhaustive §10 + doc | Audit runbook |
| R-A10 | Dépendance `sentence-transformers` lourde | Certaine | Moyen | Extra optionnel `[compile]` | Taille wheel tracée |
| R-A11 | Couleurs cassent un pipe avec `less -R` | Faible | Bas | Auto-détection TTY | Test snapshot sans TTY |
| R-A12 | UTF-8 non garanti sur stdin Windows | Moyenne | Moyen | `sys.stdin.reconfigure(encoding='utf-8')` | Test sur Windows CI |

---

## §21. Critères Go / No-Go spécifiques à A

### §21.1 Critères bloquants

- **G-A1 Build** : `pip install efe-tool` expose `efe-admin` et `efe` dans `$PATH`.
- **G-A2 Sanity** : `efe-admin --version` imprime une version valide, exit 0, en < 150 ms.
- **G-A3 Happy path** : `efe-admin classify --text "…" --catalog …` exit 0 et émet un JSON valide contenant `schema_version`, `label`, `confidence`, `reject`, `rounds`.
- **G-A4 Pipe** : `echo "…" | efe-admin classify --stdin --catalog …` fonctionne sans flag supplémentaire si stdin ≠ TTY.
- **G-A5 Exit codes** : les 9 exit codes documentés sont atteignables par un test dédié. Le code 126 (BUG) est inaccessible par des entrées valides.
- **G-A6 Performance** : cold start `--help` < 150 ms (p95 CI), cold start `classify` < 500 ms hors RPC.
- **G-A7 Erreurs** : chaque erreur produit un JSON sur stderr **et** un message texte lisible, selon `--log-format`.
- **G-A8 Schéma** : `schema_version: 1` stabilisé ; snapshot test gate.
- **G-A9 Tests** : couverture ≥ 85 % sur `cli.py` ; CI verte.
- **G-A10 Doc** : `docs/cli.md` décrit l'intégralité des subcommands et flags.

### §21.2 Critères non-bloquants (nice-to-have)

- Alias `efe` installé.
- Completions bash/zsh/fish.
- Formule Homebrew.
- Image Docker légère.
- Extra optionnel `[compile]`.
- Support `--tls`.

### §21.3 Procédure de passage Go

1. CI verte sur 5 runs consécutifs (release candidate).
2. Audit manuel du diff par un second dev (max 400 lignes modifiées).
3. Test manuel live du UC-1 à UC-6.
4. Tag `efe-tool 1.0.0`, publication PyPI.

---

## §22. Annexes

### §22.A Matrice complète flags ↔ env ↔ defaults

| Clé | Flag CLI | Env var | Default | Domain |
|---|---|---|---|---|
| endpoint | `--endpoint` | `EFE_ENDPOINT` | `127.0.0.1:50051` | `host:port` |
| catalog_path | `--catalog` | `EFE_CATALOG_PATH` | (requis) | path |
| text | `--text` | — | (requis, sauf stdin/file) | UTF-8 |
| text_file | `--text-file` | — | (requis, sauf text/stdin) | path |
| stdin | `--stdin` | — | false | bool |
| embedder_model | `--embedder` | `EFE_EMBEDDER` | `paraphrase-multilingual-MiniLM-L12-v2` | string |
| budget_ms | `--budget-ms` | `EFE_BUDGET_MS` | 5000 | int > 0 |
| timeout_s | `--timeout-s` | `EFE_TIMEOUT_S` | 10.0 | float > 0 |
| max_rounds | `--max-rounds` | `EFE_MAX_ROUNDS` | 16 | int ∈ [1, 256] |
| strict | `--strict` | `EFE_STRICT` | false | bool |
| format | `--format` | `EFE_OUTPUT_FORMAT` | `json` | {json,text,table} |
| pretty | `--pretty` | — | false | bool |
| with_diagnostics | `--with-diagnostics` | — | false | bool |
| log_level | `--log-level` | `EFE_LOG_LEVEL` | `info` | {debug,info,warn,error} |
| log_format | `--log-format` | `EFE_LOG_FORMAT` | `text` | {text,json} |
| quiet | `--quiet`, `-q` | — | false | bool |
| no_color | `--no-color` | `NO_COLOR` | auto | bool |
| request_id | `--request-id` | `EFE_REQUEST_ID` | `uuid4()` | UUID |
| tls | `--tls` (V1.1) | `EFE_TLS` | false | bool |
| tls_ca | `--tls-ca` (V1.1) | `EFE_TLS_CA` | (system) | path |
| progress | `--progress` | — | `auto` | {auto,always,never} |

### §22.B Exit codes exhaustifs

| Code | Symbole | Catégorie | Signification | Déclencheur typique |
|---|---|---|---|---|
| 0 | OK | success | Classification réussie | — |
| 2 | USAGE | user | Mauvais usage CLI | flag inconnu, valeur hors domaine |
| 3 | CATALOG_NOT_FOUND | user/data | Catalog absent | `--catalog x.yaml` inexistant |
| 4 | CATALOG_INVALID | data | Catalog malformé | YAML incorrect, fingerprint mismatch |
| 5 | SERVER_UNREACHABLE | network | gRPC injoignable | serveur éteint |
| 6 | SERVER_ERROR | server | Erreur serveur | panic Rust, INTERNAL |
| 7 | TIMEOUT | budget | Dépassement | budget ou timeout client |
| 8 | REJECTED | business | reject + `--strict` | seuil low_confidence + strict |
| 9 | LOW_CONFIDENCE | business | conf < high + `--strict` | idem |
| 126 | BUG | internal | Bug inattendu | exception non catchée |
| 130 | INTERRUPTED | signal | Ctrl-C | SIGINT |

### §22.C Exemples d'invocations (sélection)

```bash
# 1. Basique, texte inline
efe-admin classify --text "Ma carte a été piratée" --catalog catalogs/support_triage.yaml

# 2. stdin
echo "Ma carte a été piratée" | efe-admin classify --stdin --catalog catalogs/support_triage.yaml

# 3. fichier
efe-admin classify --text-file incident.txt --catalog catalogs/support_triage.yaml

# 4. pretty + diagnostics
efe-admin classify --text "hello" --catalog x.yaml --pretty --with-diagnostics

# 5. format texte humain
efe-admin classify --text "hello" --catalog x.yaml --format text

# 6. strict, pour pre-commit
efe-admin classify --text "hello" --catalog x.yaml --strict

# 7. pipeline jq
efe-admin classify --text "hello" --catalog x.yaml | jq -r '.label'

# 8. log JSON corrélé
efe-admin classify --text "hello" --catalog x.yaml \
  --log-format json --request-id run-42 2>logs.ndjson

# 9. compile puis classify
efe-admin compile-catalog catalogs/support_triage.yaml
efe-admin classify --text "hello" --catalog catalogs/support_triage.yaml

# 10. healthcheck
efe-admin healthcheck --endpoint 127.0.0.1:50051

# 11. validate seul (sans embedder)
efe-admin validate catalogs/support_triage.yaml --strict-thresholds

# 12. completions
efe-admin completions zsh > ~/.zsh/completions/_efe-admin
```

### §22.D Comparaison avec CLI de référence

| Propriété | kubectl | git | curl | gh | **efe-admin** |
|---|---|---|---|---|---|
| Subcommands | ✓ | ✓ | ✗ | ✓ | ✓ |
| JSON par défaut | ✗ | ✗ | ✗ | ✗ (JSON opt-in) | ✓ |
| Stdin | ✓ (`-f -`) | ✓ | ✓ | ✓ | ✓ |
| Exit codes spécifiques | ✓ | ✓ | ✓ | ✓ | ✓ |
| Completion shell | ✓ | ✓ | ✗ | ✓ | ✓ (TO-BE) |
| Schéma versionné | ✓ | ✗ | — | ✓ | ✓ |
| Logs sur stderr | ✓ | ✓ | ✓ | ✓ | ✓ |

### §22.E ADR — décisions d'architecture clés

- **ADR-A-001** : `argparse` plutôt que `click`. Raison : zéro dépendance supplémentaire, suffisant pour le périmètre. À reconsidérer si UX devient un goulot.
- **ADR-A-002** : JSON compact par défaut, `--pretty` opt-in. Raison : compatibilité Unix, pipeline jq.
- **ADR-A-003** : Texte non-positionnel. Raison : permet `--stdin` et `--text-file` propres, plus homogène.
- **ADR-A-004** : Entry point `efe-admin` préservé, alias `efe` ajouté. Raison : back-compat totale.
- **ADR-A-005** : `schema_version` dans la sortie. Raison : API stable côté consommateur, versioning indépendant du package.
- **ADR-A-006** : Exit codes distincts par catégorie d'erreur. Raison : ops / CI peuvent gater sans parser stderr.
- **ADR-A-007** : Lazy imports. Raison : cold start `--help` < 150 ms.
- **ADR-A-008** : Request ID propagé en gRPC metadata. Raison : corrélation client-serveur indispensable en debug prod.
- **ADR-A-009** : `sentence-transformers` en extra optionnel. Raison : wheel léger pour l'usage A avec cache précompilé.
- **ADR-A-010** : Pas de télémétrie dans le MVP. Raison : surface d'attaque et friction d'adoption minimales.

### §22.F Glossaire

- **Catalog** — document YAML décrivant les classes + leurs canonical sentences.
- **Fingerprint** — empreinte SHA-256 d'un catalog compilé, validée à chaque load.
- **Embedder** — transformeur phrase → vecteur (défaut : `paraphrase-multilingual-MiniLM-L12-v2`).
- **Reject** — verdict retourné par le serveur si `confidence < confidence_medium`.
- **Schema version** — entier versionnant le contrat JSON de sortie, indépendant du package.
- **Request ID** — UUID propagé dans les logs et en metadata gRPC pour corrélation.

---

**Fin du document.**

*Ce document est la source de vérité pour la Lettre A. Toute évolution doit se faire par PR sur ce fichier, avec revue d'un second mainteneur. Les décisions d'architecture (§22.E) sont append-only : on ajoute un nouvel ADR qui remplace un précédent, on ne réécrit pas l'historique.*
