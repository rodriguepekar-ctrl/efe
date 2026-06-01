# Lettre B — REPL interactif · Architecture détaillée

> **Statut** : Draft v1.0.0 — Document de référence pour l'adoption de niveau B du Mode 1 EFE.
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-19
> **Dépendances doc** : `LETTER_A_ARCHITECTURE.md`, `MODE1_ARCHITECTURE.md`, `M10_ARCHITECTURE.md`, `M11_ARCHITECTURE.md`.
> **Portée** : la boucle de lecture-évaluation-impression exposée sous forme de subcommand `efe-admin repl`, qui tient dans un processus long, partage un embedder et un client gRPC entre requêtes, et permet de tuner un catalog à chaud.

---

## Table des matières

1. Positionnement et contrats
2. État actuel (AS-IS) — constat de terrain vierge
3. Cas d'usage canoniques et anti-usages
4. Vue d'ensemble architecturale
5. Spécification du REPL — langage, commandes, états
6. Flux d'exécution détaillé
7. Résolution de configuration en couches (statique + runtime)
8. Contrats d'entrée : readline, multi-lignes, historique
9. Contrats de sortie : stdout humain, stderr discret
10. Rechargement à chaud du catalog
11. Récupération d'erreur et robustesse de session
12. Budget de performance
13. Sécurité
14. Observabilité et télémétrie
15. Stratégie de tests
16. Packaging et distribution
17. Versioning et compatibilité
18. Ergonomie et UX
19. Dépendances inter-modules EFE
20. Plan d'implémentation détaillé
21. Matrice de risques
22. Critères Go / No-Go spécifiques à B
23. Annexes

---

## §1. Positionnement et contrats

### §1.1 Définition opérationnelle de B

La **Lettre B — REPL interactif** est la forme d'intégration d'EFE Mode 1 qui satisfait la propriété :

> *Un humain lance un processus unique, reste connecté pendant plusieurs minutes à plusieurs heures, tape des phrases en langage naturel ou des commandes préfixées par `:`, et reçoit à chaque tour un verdict de classification lisible, avec la possibilité de modifier le catalog, les seuils ou l'embedder sans redémarrer.*

Cette propriété implique trois corollaires rigides, qui définissent la **différence fondamentale** entre B et les quatre autres lettres :

- **C1 — Processus long.** Contrairement à la Lettre A qui meurt après chaque appel, le processus B persiste. Il paye le coût d'import de `sentence-transformers`, de `torch`, de la compilation du catalog **une seule fois**, amortissant ce coût sur des dizaines de requêtes.
- **C2 — État mutable de session.** Entre deux requêtes, l'utilisateur peut `:reload`, `:catalog autre.yaml`, `:budget 2000`, `:diagnostics on`. L'état courant est stocké dans un objet `SessionState` unique, vivant pour la durée du processus.
- **C3 — Sortie humaine d'abord.** Le stdout par défaut est formaté pour les yeux humains, pas pour `jq`. L'utilisateur peut basculer en JSON avec `:format json`, mais c'est un choix explicite, pas le défaut.

### §1.2 Place dans la trajectoire A → E

| Lettre | Durée de vie | Destinataire stdout | État entre requêtes |
|---|---|---|---|
| **A** | un appel | pipeline Unix (JSON) | aucun |
| **B** | minutes à heures | **humain (texte coloré)** | **session mutable** |
| C | scénario complet | logs applicatifs | scripté |
| D | infinie | app consommatrice | framework hôte |
| E | infinie | clients multiples | multi-tenant |

B est la seule lettre dont l'utilisateur est **un humain en train de tuner le catalog**. Ce n'est ni un outil de CI (A), ni un produit utilisateur final (C, D), ni une infrastructure partagée (E). C'est un **atelier personnel**.

### §1.3 Contrats externes

B expose quatre surfaces contractuelles :

- **Surface de lancement** : `efe-admin repl [--catalog PATH] [--endpoint HOST:PORT] [--embedder MODEL] [--history PATH] [--no-color] [--no-history]`. La grammaire des arguments d'entrée est versionnée comme pour A (ADR-B-012).
- **Surface du DSL de commandes** : l'ensemble des commandes préfixées par `:`, leurs noms et leurs arguments. Toute modification incompatible (renommage, suppression, changement de sémantique) incrémente la version majeure de `efe-tool`.
- **Surface du format de session exporté** : le format JSON produit par `:save PATH` est versionné par un `schema_version` indépendant (voir §23.D).
- **Surface de l'historique** : le format du fichier `~/.efe/history` est ligne-par-ligne, UTF-8 strict. Compatible avec `fc` et `history` Unix.

### §1.4 Principes invariants de B

- **P1 — Un processus, une session.** Pas de fork, pas de thread worker, pas de concurrence interne. Les requêtes sont séquentielles.
- **P2 — Aucun reset entre requêtes.** L'embedder, le client gRPC, le catalog compilé vivent toute la session.
- **P3 — Zéro crash utilisateur.** Une requête qui lève une exception **ne tue jamais la session**. Le REPL journalise et rend la main.
- **P4 — Ctrl-C ne tue pas.** SIGINT abandonne la requête en cours (ou nettoie la ligne au prompt), mais garde la session vivante. Seul `:quit`, Ctrl-D ou `SIGTERM` terminent.
- **P5 — stdout et stderr séparés.** Comme A, stdout ne reçoit que des résultats ; stderr reçoit les logs. Dans B, les deux flux doivent cohabiter sans se mélanger dans le terminal (le readline loop synchronise bien).
- **P6 — Hot-reload fiable.** `:reload` doit produire un catalog **identique** à celui qu'obtiendrait un nouveau `efe-admin classify --catalog …` ; pas de coin de cache incohérent possible.
- **P7 — Historique consentement.** L'historique n'est persisté que si l'utilisateur ne passe **pas** `--no-history`.

### §1.5 Différences contractuelles fortes entre A et B

Pour tout dev venant de A, les changements mentaux clés sont :

1. **Le JSON n'est plus le défaut.** `--format json` existe toujours mais à la place d'un défaut on a une bascule runtime (`:format json`).
2. **Les seuils sont reconfigurables à chaud** (§23.F). Dans A, les seuils viennent du catalog et sont immuables pour la requête ; dans B, l'utilisateur peut tester une autre politique sans éditer le YAML.
3. **Le coût d'import est amorti.** Si un catalog est déjà compilé, le boot se fait en ~200 ms (pas 500 comme A) parce qu'on importe les modules lourds une fois.
4. **`cfg.text` devient transitoire.** On ne parse plus `--text` / `--stdin` pour chaque requête : la ligne courante *est* le texte.

---

## §2. État actuel (AS-IS) — constat de terrain vierge

À la date de ce document, **aucun code de la Lettre B n'existe dans le dépôt**. La Lettre A (`efe-admin` CLI avec 7 subcommands) est livrée et testée (223 tests verts) ; la Lettre B est un greenfield.

### §2.1 Ce qui existe et sera réutilisé

| Composant | Source | Rôle pour B |
|---|---|---|
| `efe_tool.client.EfeClient` | M11 | Client gRPC réutilisé en singleton de session |
| `efe_tool.catalog.AnchorCatalog` | M1 | Loader + cache `.compiled.json` |
| `efe_tool.embedder.LocalSTEmbedder` | M2 | Embedder en singleton de session |
| `efe_tool.cli.config.ResolvedConfig` | Lettre A | Boot config ; **devient une "CFG initiale" figée**, distincte de `SessionState` mutable |
| `efe_tool.cli.errors.CliError*` | Lettre A | Taxonomie exceptions, adaptée pour le REPL (exit codes mappés en codes d'abandon de tour) |
| `efe_tool.cli.logging.Logger` | Lettre A | Logger structuré sur stderr |
| `efe_tool.cli.output.render` | Lettre A | Formateurs JSON / text / table — **étendus** avec un format « REPL human » (§9.3) |
| `efe_tool.cli.io_input.redact` | Lettre A | Utilisé pour les logs |

### §2.2 Ce qui manque et doit être créé

| Composant neuf | Rôle |
|---|---|
| `efe_tool.cli.commands.repl` | Point d'entrée du subcommand `repl` |
| `efe_tool.repl.session` | `SessionState` dataclass + mutations |
| `efe_tool.repl.commands` | Registry des commandes `:xxx` + dispatch |
| `efe_tool.repl.input_loop` | Boucle readline, parsing ligne / multi-ligne |
| `efe_tool.repl.prompt` | Prompt dynamique coloré, réactif au state |
| `efe_tool.repl.history` | Lecture/écriture de `~/.efe/history` |
| `efe_tool.repl.render` | Format « REPL human » (tableaux compacts colorés) |
| `efe_tool.repl.completer` | Complétion Tab (commandes, chemins, classes) |
| `tests/repl/` | ~60 tests unitaires + intégration avec `pexpect` |

### §2.3 Contraintes hors-code

- **Python stdlib `readline`** est obligatoire. Disponible par défaut sur macOS et Linux. Sur Windows, utiliser `pyreadline3` en extra optionnel (`[repl-win]` dans pyproject).
- Pas de nouvelle dépendance à installer par défaut (hors de `readline` stdlib).

### §2.4 Synthèse AS-IS

Aucun code existant à auditer ; pas de dette technique à résorber. Le greenfield permet de concevoir d'emblée les bons contrats. La discipline déjà établie en Lettre A (résolution config en couches, taxonomie d'erreurs, stdout pristine) est conservée **intégralement** en B.

---

## §3. Cas d'usage canoniques et anti-usages

### §3.1 UC-B1 — Tuning de catalog interactif (**principal**)

**Contexte.** Un product owner relit `catalogs/support_triage.yaml` et soupçonne que la classe `fraud` englobe mal le cas « carte perdue ». Il veut tester une dizaine de formulations, ajuster les `canonical_sentences`, recharger et re-tester, sans quitter son terminal.

**Session type.**

```
$ efe-admin repl --catalog catalogs/support_triage.yaml

EFE support_triage> Ma carte a été volée à Paris
  label:       fraud      (confidence 0.575, rounds 4)
  verdict:     trusted

EFE support_triage> J'ai perdu ma carte au restaurant
  label:       billing    (confidence 0.575, rounds 4)    ← problème !
  verdict:     trusted

EFE support_triage> # (edits catalogs/support_triage.yaml in another pane)
EFE support_triage> :reload
reloaded support_triage@1.0.1 (fingerprint changed, 4 classes)

EFE support_triage> J'ai perdu ma carte au restaurant
  label:       fraud      (confidence 0.575, rounds 4)    ← corrigé
  verdict:     trusted
```

**Attentes.**
- Le `:reload` re-compile le catalog en <500 ms (embedder déjà chaud).
- Aucun re-import Python, aucun re-téléchargement du modèle.
- L'historique des phrases testées est conservé dans la session (commande `:history`).
- Les couleurs distinguent les classes visuellement (rouge=fraud, orange=bug, bleu=question, vert=billing — depuis le catalog YAML).

### §3.2 UC-B2 — Debug support ticket live

**Contexte.** Un agent support copie-colle un message client ambigu et veut voir sur quels labels oscille le moteur.

```
EFE support_triage> :diagnostics on
diagnostics: on

EFE support_triage> Mon abonnement a été renouvelé sans ma permission
  label:       billing    (confidence 0.575, rounds 4)
  verdict:     trusted
  diagnostics:
    fraud      0.4821
    bug        0.2913
    question   0.3110
    billing    0.6277  ← top
```

**Attentes.**
- `:diagnostics on` affiche immédiatement les scores de toutes les classes.
- Format compact, colonnes alignées.
- L'agent voit en un coup d'œil que `fraud` était second → confirme l'ambiguïté métier.

### §3.3 UC-B3 — Démonstration pédagogique

**Contexte.** Un tech lead explique EFE à une équipe, en live devant un écran partagé.

```
$ efe-admin repl --catalog catalogs/support_triage.yaml --format table
EFE support_triage> :help
Available commands:
  :help [CMD]        Show this help (or help for CMD)
  :catalog [PATH]    Print current catalog, or switch
  :reload            Recompile and reload the current catalog
  :thresholds ...    Inspect or override decision thresholds
  ...
EFE support_triage> J'ai été débité deux fois
+------------+-----------+
| label      | billing   |
| confidence | 0.575     |
| rounds     | 4         |
| verdict    | trusted   |
+------------+-----------+
```

**Attentes.**
- Format `table` visible de loin.
- Les commandes `:help` et `:classes` sont des supports pédagogiques propres.

### §3.4 UC-B4 — Exploration comparative inter-catalogs

**Contexte.** Un linguiste compare deux variantes d'un catalog (FR vs FR+EN) pour décider laquelle garder.

```
EFE support_triage> My card was stolen
  label:       fraud      (confidence 0.575, rounds 4)
  verdict:     trusted

EFE support_triage> :catalog catalogs/support_triage_fr_only.yaml
switched to support_triage_fr_only@0.3.0 (4 classes, embedder fingerprint unchanged)

EFE support_triage_fr_only> My card was stolen
  label:       billing    (confidence 0.515, rounds 6)     ← dégradation attendue
  verdict:     warn
```

**Attentes.**
- `:catalog` prend un chemin et bascule. L'embedder ne change pas (même modèle).
- Le prompt s'adapte au nom du catalog courant.
- La session garde son historique des phrases testées même à travers les switch.

### §3.5 UC-B5 — Smoke check personnel du serveur

**Contexte.** Chaque matin, un dev lance un REPL pour confirmer en 30 secondes que la stack fonctionne.

```
$ efe-admin repl
EFE (no catalog)> :catalog catalogs/support_triage.yaml
loaded support_triage@1.0.0
EFE support_triage> :healthcheck
SERVING (latency 12 ms)
EFE support_triage> hello
  label:       question   (confidence 0.525, rounds 5)
  verdict:     warn
EFE support_triage> :quit
bye (187 queries, session 1h 23m 14s)
```

### §3.6 Anti-usages explicites

B **ne doit pas** être utilisé pour :

- **CI / scripts automatisés.** L'interactivité est le point ; scripter B via `echo … | efe-admin repl` est fragile (parsing de stdout coloré, séquences ANSI, etc.). Pour ça, c'est A (avec `efe-admin classify --stdin`).
- **Benchmarks rigoureux.** Le premier appel paye le boot ; les suivants sont chauds. Impossible d'isoler ce qu'on mesure. Utiliser des outils dédiés de bench.
- **Production.** Un REPL n'est pas un service ; pas de SLA, pas de logs structurés consommables.
- **Sessions mutualisées.** Un seul utilisateur humain par processus. Partager une session via `tmux` est toléré mais sans garantie (pas de reentrance).

---

## §4. Vue d'ensemble architecturale

### §4.1 Diagramme de flux (une session complète)

```
┌──────────────────────────────────────────────────────────────────────┐
│                    PROCESS  efe-admin repl                           │
│                                                                      │
│  ┌──────────────┐                                                    │
│  │  BOOT        │  argparse → ResolvedConfig →                       │
│  │              │  LocalSTEmbedder (singleton) → EfeClient →         │
│  │              │  AnchorCatalog (optional) → SessionState           │
│  └──────┬───────┘                                                    │
│         │                                                            │
│         ▼                                                            │
│  ╔═════════════════════════════════════════════════════════════╗     │
│  ║                  MAIN LOOP  (while True)                    ║     │
│  ║                                                             ║     │
│  ║   ┌──────────────┐       ┌─────────────────┐                ║     │
│  ║   │ read_line()  │──────▶│  parse_input    │                ║     │
│  ║   │ (readline +  │       └─────┬───────────┘                ║     │
│  ║   │  prompt)     │             │                            ║     │
│  ║   └──────────────┘             ▼                            ║     │
│  ║         ▲          ┌───────────┴───────────┐                ║     │
│  ║         │          │ cmd?     │    text?   │                ║     │
│  ║         │          ▼          │            ▼                ║     │
│  ║         │   dispatch_cmd()    │     run_query()             ║     │
│  ║         │   (registry)        │     (client.classify)       ║     │
│  ║         │          │          │            │                ║     │
│  ║         │          ▼          │            ▼                ║     │
│  ║         │    render_cmd_result│    render_query_result      ║     │
│  ║         │                     │                             ║     │
│  ║         │                     ▼                             ║     │
│  ║         └────────────── SessionState updated ───────────────╯     │
│  ╚═════════════════════════════════════════════════════════════╝     │
│                                                                      │
│  ┌──────────────┐                                                    │
│  │  SHUTDOWN    │  client.close() → flush history → exit 0           │
│  └──────────────┘                                                    │
└──────────────────────────────────────────────────────────────────────┘
```

### §4.2 Boundaries

- **Boundary 1 — argparse ↔ boot.** Le parser fournit un `ResolvedConfig` figé. Le boot le consomme et construit le `SessionState`. Après le boot, `ResolvedConfig` n'est plus consulté.
- **Boundary 2 — SessionState ↔ commandes.** Toute commande `:xxx` reçoit `(state, args, logger)` et peut muter `state`. Elle ne peut pas faire d'I/O arbitraire hors du pipe render.
- **Boundary 3 — input_loop ↔ dispatcher.** `input_loop` produit des `Token`s (soit `QueryToken(text)`, soit `CommandToken(name, args)`). Le dispatcher consomme. Pas de couplage bidirectionnel.
- **Boundary 4 — render ↔ output.** `repl.render` renvoie des strings ; le `input_loop` les écrit sur stdout. Pas d'I/O dans render.
- **Boundary 5 — client gRPC ↔ session.** Une seule instance d'EfeClient par session. Les changements d'endpoint passent par `:endpoint` qui ferme-recrée.

### §4.3 Invariants architecturaux

- **I1.** Le processus n'importe jamais `torch`, `sentence_transformers`, `grpcio` après le boot. Tous les imports lourds sont dans la phase 1.
- **I2.** La boucle principale n'alloue jamais l'embedder ni le client. Ils sont dans `SessionState`.
- **I3.** Aucune commande ne sort du REPL par `sys.exit`. Seul `:quit` (ou EOF) déclenche le shutdown, via un return de la boucle.
- **I4.** Une exception dans `run_query` est **toujours** catchée. La session continue.
- **I5.** stdout ne reçoit que des résultats formatés. Jamais de prompt, jamais de log. (readline écrit son prompt, mais on contrôle le contenu.)
- **I6.** Les lignes commençant par `:` sont **toujours** interprétées comme commandes, même si le texte qui suit est du langage naturel. Pour classifier un texte qui commence par `:`, utiliser `:classify <texte>` explicite.

---

## §5. Spécification du REPL

### §5.1 Lancement

```
efe-admin repl [--catalog PATH]
               [--endpoint HOST:PORT]
               [--embedder MODEL]
               [--budget-ms INT]
               [--timeout-s FLOAT]
               [--max-rounds INT]
               [--format {text|json|table}]
               [--pretty]
               [--history PATH | --no-history]
               [--prompt TEMPLATE]
               [--no-color]
               [--log-level LEVEL]
               [--log-format {text|json}]
```

**Observations.**

- `--catalog` est **optionnel**. Sans catalog, le REPL démarre en mode « orphelin » : seules les commandes qui n'en ont pas besoin fonctionnent (`:help`, `:catalog PATH`, `:version`, `:healthcheck`, `:quit`). Une tentative de classification sans catalog retourne une erreur lisible.
- `--history PATH` pointe vers le fichier d'historique ; défaut `$XDG_DATA_HOME/efe/history` ou `~/.local/share/efe/history` ou `~/.efe/history` selon la plateforme.
- `--prompt` accepte un template avec placeholders : `{catalog}`, `{queries}`, `{elapsed}`. Défaut : `EFE {catalog}> `.

### §5.2 Grammaire des entrées

Chaque ligne est classée par **une** règle simple :

```
line := command | query | empty

command := ':' name (WS arg)*
query   := any UTF-8 string not starting with ':'
empty   := (WS)*
```

**Règles complémentaires.**

- Une ligne commençant par `\` à son extrémité signale une continuation : la ligne suivante sera concaténée.
- Un bloc `"""` entre deux lignes démarre et termine un input multi-lignes (query ou argument de commande).
- Les commandes qui acceptent un argument `PATH` acceptent les chemins relatifs, résolus par rapport au cwd au lancement (pas au répertoire courant, pour éviter la confusion après des `os.chdir` indirects via commandes).

### §5.3 Registre complet des commandes

| Commande | Signature | Effet |
|---|---|---|
| `:help` | `:help [NAME]` | Liste toutes les commandes ou détaille une commande précise. |
| `:quit` / `:exit` / `:q` | — | Quitte la session, exit 0. Ctrl-D équivalent. |
| `:catalog` | `:catalog [PATH]` | Sans argument : imprime le catalog courant. Avec : bascule. Lève `CatalogInvalidError` en cas d'échec **mais** ne quitte pas. |
| `:reload` | — | Force la recompilation du catalog courant. Utile après un edit YAML. |
| `:embedder` | `:embedder [MODEL]` | Imprime ou bascule le modèle d'embeddings. Un changement invalide le catalog courant (à `:reload`). |
| `:endpoint` | `:endpoint [HOST:PORT]` | Imprime ou bascule l'endpoint gRPC. Reconstruit le client. |
| `:thresholds` | `:thresholds [high F] [medium F]` | Override runtime des seuils (local à la session). Reset via `:thresholds reset`. |
| `:budget` | `:budget [MS]` | Imprime ou fixe `budget_wall_clock_ms`. |
| `:timeout` | `:timeout [S]` | Imprime ou fixe `timeout_s`. |
| `:rounds` | `:rounds [N]` | Imprime ou fixe `max_rounds`. |
| `:format` | `:format {text|json|table|human}` | Bascule le format stdout. `human` est le défaut REPL, absent de A. |
| `:pretty` | `:pretty on|off` | JSON pretty on/off. |
| `:diagnostics` | `:diagnostics on|off` | Affiche les sub_scores. |
| `:strict` | `:strict on|off` | Active/désactive le gate strict (sans effet sur l'exit code, mais marque les résultats). |
| `:log` | `:log {debug|info|warn|error|off}` | Niveau de log stderr à la volée. |
| `:quiet` | — | Alias de `:log error`. |
| `:verbose` | — | Alias de `:log debug`. |
| `:history` | `:history [N]` | Affiche les N dernières entrées (queries + commandes). Défaut N=20. |
| `:save` | `:save PATH` | Exporte la session (transcript JSON versionné) vers PATH. |
| `:stats` | — | Compte de requêtes, distribution des labels, temps moyen. |
| `:classes` | — | Liste les classes du catalog courant avec couleurs. |
| `:clear` | — | Efface l'écran (séquence ANSI ; no-op si `--no-color`). |
| `:healthcheck` | — | Sonde le serveur ; affiche SERVING/NOT_SERVING + latence. |
| `:version` | — | Affiche la version d'`efe-tool`. |
| `:classify` | `:classify TEXT` | Force l'interprétation d'une chaîne comme requête (utile si le texte commence par `:`). |
| `:cancel` | — | Annule la requête en cours (utilisé depuis un autre terminal via signal, si exposé). |
| `:info` | — | État complet : session_id, catalog, endpoint, budget, timeout, rounds, format, diagnostics. |

**Nombre total** : 26 commandes. Toutes sont réalisables en pur Python sans dépendance.

### §5.4 `SessionState` — contrat de données

```python
@dataclass
class SessionState:
    # --- identité session ---
    session_id: str                      # UUID v4
    started_at: float                    # time.monotonic() au boot
    queries_count: int = 0
    commands_count: int = 0

    # --- ressources long-lived ---
    embedder: Embedder | None
    client: EfeClient | None
    catalog: AnchorCatalog | None

    # --- cibles configurables à chaud ---
    endpoint: str
    budget_ms: int
    timeout_s: float
    max_rounds: int
    strict: bool
    output_format: str                   # "human" | "text" | "json" | "table"
    pretty: bool
    diagnostics: bool
    log_level: str

    # --- overrides de seuils (optionnels, locaux à la session) ---
    threshold_high_override: float | None = None
    threshold_medium_override: float | None = None

    # --- historique et traces ---
    history: list[str] = field(default_factory=list)
    last_results: list[ClassifyResult] = field(default_factory=list)
    label_counts: dict[str, int] = field(default_factory=dict)

    # --- boot config (figée, source d'origine) ---
    boot_cfg: ResolvedConfig
```

`SessionState` **n'est pas frozen**. Les commandes mutent en place. Un seul thread → pas de lock.

### §5.5 Signal handling

- **SIGINT (Ctrl-C)** :
  - Au prompt (avant qu'une requête ne démarre) : vide la ligne courante, affiche un hint.
  - Pendant une requête : annule (tentative de cancel gRPC, fallback : on ignore le résultat). Affiche `<interrupted>` et rend la main.
- **SIGTERM** : flush history, close client, exit 143.
- **SIGHUP** : pareil que SIGTERM.
- **Ctrl-D (EOF)** : sortie propre, comme `:quit`.

### §5.6 Exemples de sessions types

Voir §3 pour les scénarios UC. L'annexe §23.G contient une session de bout en bout annotée ligne par ligne.

---

## §6. Flux d'exécution détaillé

### §6.1 Phase 1 — Boot

```
t=0 ms   entry_point called (python -m efe_tool.cli)
t=10 ms  argparse parse_args
t=12 ms  resolve_config
t=14 ms  setup_logging
t=15 ms  branch vers commands/repl.run
t=15 ms  Welcome banner on stderr (1 ligne)
t=30 ms  lazy import efe_tool.client, efe_tool.embedder
t=180 ms LocalSTEmbedder(...).__init__  (résolution de fingerprint + lazy)
t=200 ms  si --catalog : AnchorCatalog.load(...)  — cache hit: ~50 ms
t=250 ms  EfeClient(...) construit (channel lazy)
t=250 ms  ReadlineHistory.load(PATH)
t=255 ms  Prompt affiché
```

Budget cible : **< 500 ms** de boot à prompt affiché (cache hit, pas de téléchargement).

### §6.2 Phase 2 — Main loop

```
t_prompt    readline.input(prompt)  ← bloquant
t_read + 0  parse_line
t_read + 1  si command: dispatch → state mutated → render_cmd
            si query  : t_q0 = now
                        client.classify(text, state.catalog, ...)
                        t_q1 = now
                        update state.last_results, label_counts
                        render_query
t_print     stdout.write(…)
→ retour au prompt
```

Budget cible par tour :
- Parsing ligne : < 1 ms
- Dispatch commande : < 5 ms
- Requête `classify` : p95 < 2 s (identique à A hors RPC)
- Render : < 10 ms

### §6.3 Phase 3 — Shutdown

Déclenché par `:quit`, Ctrl-D, SIGTERM, SIGHUP.

```
flush history to PATH  (ou skip si --no-history)
client.close()
farewell line on stderr ("bye — N queries, Hh Mm Ss")
exit(0)
```

### §6.4 Diagramme d'état

```
   ┌──────┐
   │ BOOT │───────▶ READY ◀──┐
   └──────┘          │       │
                     ▼       │
              ┌─── READ ─────┤
              │       │      │
        EOF/  │       │ line │
        :quit │       ▼      │
              │    DISPATCH ─┤ error handled
              │       │      │
              │       ▼      │
              │    RENDER ───┘
              │
              ▼
           SHUTDOWN ──▶ exit 0
```

---

## §7. Résolution de configuration

### §7.1 Deux niveaux distincts

B introduit une dichotomie qui n'existait pas en A :

- **Config de boot** (`ResolvedConfig`, figée) : héritée de la Lettre A, déterminée par argparse + env + defaults au démarrage. Sa valeur ne change pas pendant la session.
- **État de session** (`SessionState`, mutable) : initialisé depuis la config de boot, mais modifiable à chaud via les commandes `:xxx`.

La confusion entre ces deux niveaux est le piège #1 du REPL. Règle :

> Si une valeur doit rester stable toute la session, elle reste dans `boot_cfg`. Si elle peut changer à chaud, elle est répliquée dans `SessionState`.

### §7.2 Précédence au boot

Identique à la Lettre A (§7.1 de `LETTER_A_ARCHITECTURE.md`) : flag > env > default.

### §7.3 Modifications runtime

Table canonique des commandes qui mutent un champ de `SessionState` :

| Clé | Commande | Rollback via | Persisté |
|---|---|---|---|
| `endpoint` | `:endpoint HOST:PORT` | `:endpoint` (sans arg affiche courant) | non |
| `budget_ms` | `:budget MS` | `:budget reset` | non |
| `timeout_s` | `:timeout S` | `:timeout reset` | non |
| `max_rounds` | `:rounds N` | `:rounds reset` | non |
| `strict` | `:strict on/off` | idem | non |
| `output_format` | `:format FMT` | `:format text` | non |
| `pretty` | `:pretty on/off` | idem | non |
| `diagnostics` | `:diagnostics on/off` | idem | non |
| `threshold_high_override` | `:thresholds high F` | `:thresholds reset` | non |
| `threshold_medium_override` | `:thresholds medium F` | idem | non |
| `log_level` | `:log LEVEL` | `:log LEVEL` | non |

**Décision** : aucun état runtime n'est persisté entre sessions (autre que l'historique). Les overrides `:` sont éphémères par design — c'est cohérent avec la nature de l'atelier personnel.

### §7.4 Validation des overrides

Les commandes `:` appliquent les mêmes validateurs que `validate_config` de la Lettre A. Un `:budget -1` lève `UsageError` au tour courant ; la session continue, l'ancien budget reste.

---

## §8. Contrats d'entrée

### §8.1 Lecture via `readline`

```python
import readline
line = input(prompt)
```

`readline` fournit :

- **Édition de ligne** : Ctrl-A, Ctrl-E, Ctrl-K, Ctrl-W, etc.
- **Historique navigable** : flèches haut / bas.
- **Recherche incrémentale** : Ctrl-R.
- **Tab completion** (on configure la fonction de complétion).

Sur Windows, `pyreadline3` (extra optionnel `[repl-win]`) émule l'interface.

### §8.2 Tab completion

La fonction de complétion (`readline.set_completer`) couvre trois cas :

1. **Début de ligne, texte commence par `:`** : proposer les noms de commandes commençant par le préfixe.
2. **Argument d'une commande qui prend un PATH** (`:catalog`, `:save`, `:history file`) : déléguer à `readline.complete_filename`.
3. **Argument d'une commande qui prend une valeur énumérée** : proposer (`:format` → `text/json/table/human`, `:log` → `debug/info/...`).

Pour une ligne qui ne commence pas par `:`, **aucune complétion** : le texte libre n'a pas de vocabulaire à proposer (on n'essaie pas de deviner du langage naturel).

### §8.3 Multi-lignes

```
EFE> """
...  Un message long, sur plusieurs lignes.
...  Avec des sauts de ligne préservés.
...  """
  label:  …
```

Le parser détecte `"""` en début de ligne, accumule les lignes jusqu'à la clôture `"""`, et soumet le bloc entier comme query.

Alternative : `\` en fin de ligne pour continuation simple :

```
EFE> première partie \
...  deuxième partie
```

### §8.4 Historique persistant

**Fichier** : `$EFE_HISTORY_PATH` > `$XDG_DATA_HOME/efe/history` > `~/.local/share/efe/history` > `~/.efe/history`.

**Format** : une ligne par entrée, UTF-8 strict, sans horodatage (compatible avec `fc` de bash si importé).

**Limites** : 10 000 lignes (rotation FIFO).

**Permissions** : 0600 (uniquement le propriétaire). Le fichier peut contenir des données sensibles saisies par l'utilisateur.

**`--no-history`** : désactive la persistance pour la session courante (pas d'écriture, lecture quand même autorisée).

### §8.5 TTY handling

- Si stdin n'est **pas** un TTY au démarrage (redirection), le REPL passe en mode **batch** : lit les lignes jusqu'à EOF, les traite comme des queries séquentielles, pas de prompt affiché. Cas d'usage : tests `pexpect`.
- Si stdout n'est pas un TTY : désactive les couleurs (identique à la logique `NO_COLOR` d'A).

---

## §9. Contrats de sortie

### §9.1 Format `human` (défaut REPL)

Par défaut, le REPL utilise un format optimisé pour la lecture écran :

```
  label:       fraud      (confidence 0.575, rounds 4)
  verdict:     trusted
```

Alignement à 12 caractères pour les clés. Couleur du label déterminée par `catalog.classes[label].color` (défini dans le YAML). Si non défini, palette fallback.

Avec `:diagnostics on` :

```
  label:       question   (confidence 0.525, rounds 4)
  verdict:     warn
  diagnostics:
    fraud      0.123
    bug        0.234
    question   0.651 ← top
    billing    0.189
```

### §9.2 Format `text`, `json`, `table`

Hérités de la Lettre A. Activables via `:format`.

### §9.3 Couleurs

Palette par zone de confidence :

- **trusted** : couleur du label (depuis catalog).
- **warn** : jaune si label visible, gras sinon.
- **reject** : rouge.

En `--no-color` ou stdout non-TTY, ajout d'un marqueur ASCII :

```
  verdict:     reject  [!]
```

### §9.4 stderr : logs au goutte-à-goutte

stderr reçoit :

- Les logs structurés (format `text` par défaut, `json` si `--log-format json`).
- Le banner au boot.
- Les erreurs / warnings des commandes.

Par défaut, `log_level=info`. Chaque requête émet `classify_start` et `classify_done` (1 ligne chacune). En REPL, on recommande `:quiet` pour réduire le bruit visuel dans le terminal.

### §9.5 Interdictions

- Jamais de `print()` vers stdout depuis le code applicatif — toujours via `render.*`.
- Jamais de couleur sur stderr (les logs JSON ne doivent pas être pollués par des séquences ANSI).

---

## §10. Rechargement à chaud du catalog

### §10.1 Cas usuel : `:reload`

```
EFE support_triage> :reload
reloaded support_triage@1.0.1 (fingerprint abc → def, 4 classes)
```

Séquence :

1. Prendre `state.catalog.source_path`.
2. `AnchorCatalog.load(path, embedder=state.embedder)` — ce qui déclenche la recompilation **si** le fingerprint du YAML a changé depuis le `.compiled.json`.
3. Remplacer `state.catalog` atomiquement.
4. Si le fingerprint embedder a changé (ex : l'utilisateur a fait `:embedder autre-modele` entretemps), la recompilation est obligatoire.
5. Afficher un résumé des différences (nombre de classes, nom, version).

**Invariant** : si le YAML n'a pas changé, `:reload` est un no-op (cache hit) exécuté en <10 ms.

### §10.2 Cas exceptionnel : swap d'embedder

```
EFE> :embedder sentence-transformers/all-MiniLM-L6-v2
embedder switched: now on 'sentence-transformers/all-MiniLM-L6-v2@384'
⚠  the current catalog 'support_triage' was compiled with a different
   embedder. Run `:reload` to recompile, or `:catalog PATH` to load
   a different catalog.
```

L'état devient incohérent (catalog vs embedder). Toute tentative de `classify` renvoie une erreur lisible jusqu'à `:reload` ou `:catalog`.

### §10.3 Cas avancé (non-MVP) : `--watch`

Option future : démarrer avec `--watch`, le REPL surveille le fichier YAML via `watchdog` ou `inotify` et reload automatiquement sur modification. **Pas dans le MVP de B** (complexifie la threadsafety, surcoût dépendance).

### §10.4 Cohérence avec le cache disque

Le `.compiled.json` est partagé entre A et B. Si un A en parallèle écrit le cache pendant que B le lit : au pire, B lit une version partiellement écrite → `json.JSONDecodeError` → `CatalogInvalidError` affichée dans le REPL, session continue, l'utilisateur peut `:reload` une seconde fois.

**Mitigation** : utilisation de l'atomic-write de `AnchorCatalog._save_compiled` (écriture dans un fichier temporaire + rename). À vérifier dans le code M1 actuel. Si absent, c'est un ajout non-bloquant pour B mais recommandé.

---

## §11. Récupération d'erreur et robustesse de session

### §11.1 Catalogue des fautes non-fatales

| Type d'erreur | Origine | Action REPL |
|---|---|---|
| `UsageError` (commande mal formée) | Utilisateur | Afficher, hint, re-prompt |
| `CatalogNotFoundError` | `:catalog PATH` inexistant | Afficher, re-prompt |
| `CatalogInvalidError` | YAML corrompu à `:reload` | Afficher, catalog courant préservé |
| `NetworkError` (server_unreachable) | RPC | Afficher, session continue |
| `CliTimeoutError` | RPC dépassement | Afficher, session continue |
| `ServerError` | RPC INTERNAL | Afficher, session continue |
| `RejectedError` / `LowConfidenceError` | strict mode | Marquer le résultat `⚠`, continuer |

### §11.2 Catalogue des fautes fatales

| Cause | Action |
|---|---|
| `KeyboardInterrupt` au prompt vide | Afficher hint `use :quit to exit`, re-prompt |
| `KeyboardInterrupt` pendant une query | Cancel RPC, retour au prompt |
| `EOFError` (Ctrl-D) | Shutdown propre, exit 0 |
| `:quit` | Shutdown propre, exit 0 |
| Exception non catchée dans le dispatcher | Log + afficher + re-prompt. N'a **pas** le droit de tuer la session. |
| SIGTERM / SIGHUP | Shutdown propre, exit 143 / 129 |
| Erreur fatale au boot (endpoint invalide, catalog nécessaire et impossible à charger) | Afficher, exit 2/3/4/5 selon le code |

### §11.3 Tampon d'exception de la boucle principale

```python
def main_loop(state, log):
    while True:
        try:
            line = read_line(state)
        except EOFError:
            break
        except KeyboardInterrupt:
            print()  # newline to clear partial prompt
            continue
        try:
            token = parse_line(line)
            if token.is_command:
                dispatch_command(token, state, log)
            elif token.is_query:
                run_query(token.text, state, log)
        except CliError as e:
            render_error_human(e, state)
            log.warn("repl_cli_error", code=e.error_code,
                     message=e.message)
        except KeyboardInterrupt:
            render_interruption(state)
            log.info("repl_query_interrupted")
        except Exception as e:
            # Never let an unexpected error kill the REPL.
            log.error("repl_unexpected", error=str(e),
                      type=type(e).__name__)
            render_error_human(
                CliError(f"unexpected: {type(e).__name__}: {e}",
                         cause=e), state)
```

**Règle de fer** : *aucune* exception ne doit propager hors de la `while True`.

### §11.4 Tests de robustesse

- **Fuzzing** : injecter aléatoirement dans stdin des séquences ANSI, null bytes, caractères de contrôle → la session ne doit pas crash.
- **Injection d'exceptions** via monkey-patching `client.classify` : vérifier que chaque type d'exception est rendu proprement.
- **Panic du serveur** : simuler RESOURCE_EXHAUSTED, INTERNAL, UNAVAILABLE ; vérifier la récupération.

---

## §12. Budget de performance

### §12.1 Boot cible

- Avec catalog cache hit : **< 500 ms** (P95) sur MacBook M3 Pro.
- Sans catalog : **< 200 ms**.
- Avec compile à froid (cache miss, modèle déjà téléchargé) : **< 3 s**.
- Avec téléchargement modèle (premier run ever) : non borné (dépend du réseau). Afficher un message explicite.

### §12.2 Requête

Identique à A : **p95 < 2 s, p99 < 3 s**. B n'a aucune pénalité par rapport à A — au contraire, les cold imports sont déjà payés.

### §12.3 Changements de state

- `:catalog PATH` : cache hit = <50 ms, cache miss = coût de compile.
- `:reload` : idem.
- `:endpoint H:P` : <30 ms (teardown + rebuild du client).
- `:budget` / `:timeout` / `:rounds` / `:format` etc. : <1 ms (mutation).

### §12.4 Mesures

Le REPL expose `:stats` qui affiche :

```
Queries:      42
Labels:       fraud=12, bug=7, question=15, billing=8
Latency p50:  0.812 s
Latency p95:  1.654 s
Session:      0h 23m 11s
```

Mesures locales, basées sur `time.perf_counter()` côté client.

### §12.5 Optimisations non-MVP

- **Cache LRU côté client** (`EfeClient._cache`) déjà en place dans M11. Le REPL en bénéficie naturellement.
- **Pool de canaux gRPC** : pas nécessaire en single-thread.
- **Pré-warm embedder** : pourrait encoder une phrase bidon au boot pour charger torch. Gain douteux pour un usage humain. Reporté.

---

## §13. Sécurité

### §13.1 Historique sensible

- Par défaut, tout ce qui est tapé est persisté. C'est cohérent avec `bash` / `zsh`.
- `--no-history` coupe complètement.
- `:history clear` efface l'historique du fichier (effet immédiat sur disque, pas seulement sur la session).
- Détection heuristique : si une ligne contient un pattern sensible (`password=`, `api_key`, JWT, card number), un warning stderr avant la persistance. Option `:no-history-last` pour exclure la dernière ligne.
- Permissions du fichier d'historique : 0600 forcé à l'ouverture (chmod best-effort).

### §13.2 Secrets de connexion

Comme A, B n'accepte **aucun secret en argument de la ligne de commande**. Les tokens (JWT, mTLS paths) viennent via env (`EFE_JWT`) ou fichiers de config référencés par chemin.

Cas spécial : `:endpoint` et `:jwt` (commande optionnelle non incluse au MVP) écrivent-ils le nouveau secret dans l'historique ? **Oui**, puisque la ligne est une commande REPL standard. Recommandation forte dans `:help :jwt` : utiliser un fichier.

### §13.3 Réseau

- `--tls` / `--tls-ca` applicables identiquement à A.
- `:endpoint` préserve la configuration TLS de la session (`state.tls`, `state.tls_ca`).

### §13.4 Signaux externes

- SIGUSR1 / SIGUSR2 : ignorés (pas utilisés). Pas de backdoor.
- Sockets Unix / pipes nommés : inutilisés.

### §13.5 Surface d'attaque

Le REPL n'écoute pas sur le réseau. Il n'expose aucune API. La seule surface d'attaque est le client gRPC sortant et le fichier d'historique. Pas de déséralisation de YAML non-sûr (on utilise `yaml.safe_load`).

---

## §14. Observabilité et télémétrie

### §14.1 Logs

Identiques à A (structured text / JSON, `--log-format` au boot, `:log` à chaud). Événements spécifiques à B :

- `repl_boot` (au démarrage, avec versions)
- `repl_ready` (prompt affiché pour la première fois)
- `repl_command_dispatched` (name, args)
- `repl_query_done` (elapsed, label, confidence)
- `repl_query_interrupted` (SIGINT pendant query)
- `repl_state_changed` (key, old, new)
- `repl_shutdown` (queries_count, commands_count, duration)

### §14.2 Métriques locales

`:stats` expose :

- Nombre de queries
- Distribution des labels
- Distribution des confidences (p50 / p95 / p99 sur la session)
- Latence p50 / p95
- Rechargements de catalog effectués
- Commandes les plus utilisées

Pas de Prometheus, pas d'OpenTelemetry côté CLI (c'est du monitoring perso, pas de l'observabilité infrastructure).

### §14.3 Transcript

`:save PATH` exporte la session complète en JSON :

```json
{
  "schema_version": 1,
  "session_id": "018f…",
  "started_at": "2026-04-19T14:58:01Z",
  "ended_at": null,
  "boot_config": {…},
  "entries": [
    {"kind": "query", "text": "…", "result": {…}, "elapsed_ms": 842, "at": "…"},
    {"kind": "command", "name": "reload", "args": [], "at": "…"},
    …
  ],
  "stats": {…}
}
```

Utile pour reproduire un bug en équipe.

---

## §15. Stratégie de tests

### §15.1 Pyramide

```
        ▲  interactive pexpect       ~15 tests
       ╱ ╲ (vrai processus, tty emulé)
      ╱   ╲
     ╱     ╲ integration subprocess  ~20 tests
    ╱───────╲ (batch mode: stdin piped)
   ╱         ╲
  ╱           ╲ unit                 ~60 tests
 ╱─────────────╲ (parser, dispatcher, state, commands)
```

### §15.2 Unité

- `test_repl_parser.py` : parsing ligne → token (commande vs query vs empty), multi-lignes, continuation `\`, `"""`.
- `test_repl_commands.py` : chaque commande (`:reload`, `:catalog`, `:thresholds`, etc.) testée isolément avec un `SessionState` factice.
- `test_repl_state.py` : mutations cohérentes, thresholds override, reset.
- `test_repl_completer.py` : complétion commandes / paths / enum values.
- `test_repl_history.py` : lecture / écriture / rotation / permissions.
- `test_repl_prompt.py` : rendu dynamique selon state.

### §15.3 Intégration subprocess (batch mode)

```python
def test_repl_classifies_and_quits(catalog_fixture):
    r = subprocess.run(
        ["efe-admin", "repl", "--catalog", str(catalog_fixture),
         "--no-history", "--no-color"],
        input="hello world\n:quit\n",
        capture_output=True, text=True, timeout=30)
    assert r.returncode == 0
    assert "label" in r.stdout
```

### §15.4 pexpect (interactive)

```python
def test_ctrlc_at_prompt_does_not_exit():
    p = pexpect.spawn("efe-admin", ["repl", "--no-history"])
    p.expect("EFE .*> ")
    p.sendcontrol("c")
    p.expect("EFE .*> ")  # still alive
    p.sendline(":quit")
    p.expect(pexpect.EOF)
    assert p.exitstatus == 0
```

Tests critiques à couvrir :

- Ctrl-C au prompt → pas d'exit.
- Ctrl-C pendant query → cancel, session continue.
- Ctrl-D → exit 0 propre.
- `:quit` → exit 0 propre.
- Reload après édition YAML → nouveau fingerprint observable.
- Switch de catalog → prompt change.
- Exception côté serveur simulée → session survit.

### §15.5 Couverture cible

- `efe_tool.repl.*` : ≥ 85 % lignes.
- `efe_tool.cli.commands.repl` : ≥ 90 %.

### §15.6 Fixtures communes

- `valid_catalog` : un YAML compilable fourni dans `tests/data/`.
- `fake_client` : un client qui renvoie un `ClassifyResult` canonique sans RPC.
- `tmp_history` : un chemin d'historique éphémère dans `tmp_path`.

---

## §16. Packaging et distribution

### §16.1 Nouveau subcommand

Ajout dans le parser de la Lettre A :

```python
def _build_repl(sub, global_parent):
    p = sub.add_parser("repl", help="Start an interactive REPL session.", …)
    …
```

Aucun nouveau binaire. `efe-admin repl` et `efe repl` fonctionnent.

### §16.2 Dépendances

Hard deps (déjà dans pyproject de `efe-tool`) :
- `sentence-transformers` (pour l'embedder)
- `grpcio`
- `pyyaml`
- `numpy`

Nouvelles deps MVP B :
- Aucune obligatoire. `readline` est dans la stdlib Unix.

Extra optionnels :
- `[repl-win]` = `pyreadline3>=3.4` pour Windows.

### §16.3 Image Docker

Identique à A ; ajouter une couche qui fait `CMD ["efe-admin", "repl"]` dans un Dockerfile d'exemple (`docker/repl.Dockerfile`).

### §16.4 Completion shell

Le subcommand `efe-admin completions` gagne les noms de commandes REPL ? **Non** : les complétions shell couvrent l'invocation du CLI, pas l'intérieur du REPL. Les `:` complétions sont internes au REPL via `readline`.

---

## §17. Versioning et compatibilité

### §17.1 Axes

- **Version package** : SemVer `efe-tool` (partagée avec A).
- **DSL de commandes REPL** : versionné implicitement par la version package.
- **`schema_version` du transcript** (`:save`) : entier indépendant (voir §23.D).

### §17.2 Dépréciations de commandes

Cycle identique à A (3 versions : annonce, warning, suppression).

Exemple projeté : renommer `:format` en `:output-format` serait breaking. Cycle :

- v1.1 : `:output-format` ajouté, `:format` conservé mais `:help :format` annonce la dépréciation.
- v1.2 : `:format` affiche un warning à chaque utilisation.
- v2.0 : `:format` supprimé.

### §17.3 Compatibilité de transcript

Le format `:save` avec `schema_version: 1` reste consommable par toute V1.x. Rupture en V2.0 uniquement.

---

## §18. Ergonomie et UX

### §18.1 Prompt

Template par défaut : `EFE {catalog}> ` où `{catalog}` est le nom court (pas le path).

Exemples :

- Sans catalog : `EFE (no catalog)> `
- Avec catalog : `EFE support_triage> `
- Après `:catalog autre.yaml` : `EFE autre> `

Prompt coloré : nom du catalog en cyan sur TTY.

### §18.2 Bannière de boot

```
EFE Mode 1 — interactive REPL v0.1.0
catalog: support_triage@1.0.0 (4 classes, multilingual-MiniLM)
endpoint: 127.0.0.1:50051 (insecure)
Type :help for commands, :quit to exit.
```

3-4 lignes sur stderr, au boot. Invisible en `--quiet`.

### §18.3 Aide contextuelle

- `:help` → liste toutes les commandes, groupées (query, catalog, server, output, session).
- `:help :reload` → doc détaillée de `:reload`.
- `:help` dans une session sans catalog → rappelle que `:catalog` est prioritaire.

### §18.4 Suggestions sur erreur

Si l'utilisateur tape `:relod`, le REPL suggère `:reload` via `difflib.get_close_matches`.

```
EFE> :relod
error: unknown command ':relod'
hint: did you mean ':reload'?
```

### §18.5 Couleurs et accessibilité

- `--no-color` et `NO_COLOR` env : désactivent toutes les séquences ANSI (identique A).
- Palette choisie pour être lisible en thème sombre ET clair.
- Mode `:colors high-contrast` (post-MVP) : palette adaptée malvoyants.

### §18.6 Localisation

Messages EN par défaut. Hook `gettext` prévu pour une future localisation (V1.2+).

### §18.7 Hygiène visuelle

- Jamais plus d'une ligne vide consécutive.
- Les tableaux ne dépassent pas 80 colonnes sauf si stdout le permet.
- Sortie tronquée avec `…` pour les très longs labels ou scores.

---

## §19. Dépendances inter-modules EFE

### §19.1 Graphe

```
          ┌─────────────────────┐
          │  B  = commands/repl │
          │       + repl/*      │
          └────┬──────┬─────────┘
               │      │
         ┌─────┘      └────────┐
         ▼                     ▼
  A = cli/ (parser,       M11 = EfeClient, AnchorCatalog,
      resolver, output)         LocalSTEmbedder
         │
         ▼
     stdlib readline
```

### §19.2 Hard deps

- **A** (tous les modules de `efe_tool.cli.*`) : parsing, resolver, output, logging, errors.
- **M1 / M2 / M11** : catalog, embedder, client.

### §19.3 Zero-dep additionnelle en runtime

Le REPL n'introduit **aucune** dépendance nouvelle côté Unix. Windows gagne `pyreadline3` en extra optionnel.

### §19.4 Anti-deps

- `anthropic` : non (c'est C).
- Frameworks : non.
- `prompt_toolkit` : considéré, refusé (trop lourd, UX marginalement meilleure pour ce scope).
- `rich` : considéré, refusé (même raison). Nous réimplémentons les 100 lignes de rendering nécessaires.

---

## §20. Plan d'implémentation détaillé

### §20.1 Sprint B-1 (3 jours) — Squelette

- B-1.1 : Subcommand `repl` dans le parser, args documentés, `default=argparse.SUPPRESS` respecté.
- B-1.2 : `commands/repl.py` → charge l'embedder/catalog/client, démarre la boucle vide (just `input(prompt)` → quit).
- B-1.3 : `SessionState` dataclass + tests de mutation.
- B-1.4 : Intégration dans `cli/main.py` dispatch.
- **Gate B-1** : `efe-admin repl --catalog X.yaml` démarre, affiche un prompt, `:quit` quitte.

### §20.2 Sprint B-2 (2 jours) — Dispatcher & commandes basiques

- B-2.1 : Parser de ligne → token (command vs query).
- B-2.2 : Registry des commandes, dispatch.
- B-2.3 : Commandes `:help`, `:quit`, `:info`, `:version`, `:classes`, `:clear`, `:format`.
- **Gate B-2** : les commandes simples répondent correctement, `:help` documente tout.

### §20.3 Sprint B-3 (2 jours) — Query

- B-3.1 : `run_query(text, state)` → appelle `client.classify`.
- B-3.2 : `repl.render.render_query_result_human` (format par défaut).
- B-3.3 : Intégration format text/json/table via `:format`.
- B-3.4 : Gestion d'erreur : `CliError` catchée, re-prompt.
- **Gate B-3** : on peut classifier des phrases, le REPL ne crash pas sur erreur serveur.

### §20.4 Sprint B-4 (2 jours) — Tuning live

- B-4.1 : `:catalog PATH` + reload propre du state.catalog.
- B-4.2 : `:reload` force recompilation.
- B-4.3 : `:budget`, `:timeout`, `:rounds`, `:strict`, `:pretty`, `:diagnostics`.
- B-4.4 : `:thresholds high X medium Y` + reset.
- **Gate B-4** : UC-B1 et UC-B4 passent end-to-end.

### §20.5 Sprint B-5 (1 jour) — Ergonomie

- B-5.1 : Prompt coloré dynamique.
- B-5.2 : Tab completion (commandes + chemins + enum values).
- B-5.3 : Couleurs par label (via `catalog.classes[].color`).
- B-5.4 : Bannière de boot.
- **Gate B-5** : UC-B3 (démo pédagogique) passe.

### §20.6 Sprint B-6 (1 jour) — Historique

- B-6.1 : Lecture `~/.efe/history` au boot.
- B-6.2 : Écriture atomique au shutdown.
- B-6.3 : Rotation FIFO 10 000 lignes.
- B-6.4 : `--no-history` / `:history` / `:history clear`.
- B-6.5 : chmod 0600.
- **Gate B-6** : l'historique survit aux redémarrages, pas de fuite de perms.

### §20.7 Sprint B-7 (1 jour) — Signals & robustesse

- B-7.1 : SIGINT handler (prompt vide / query en cours).
- B-7.2 : Ctrl-D → quit.
- B-7.3 : Try/except total sur la boucle.
- B-7.4 : `:healthcheck`, `:endpoint`, `:embedder`.
- **Gate B-7** : tests pexpect Ctrl-C / Ctrl-D passent.

### §20.8 Sprint B-8 (1 jour) — Transcript et stats

- B-8.1 : `:stats` (latences, distribution).
- B-8.2 : `:save PATH` → JSON transcript versionné.
- B-8.3 : `:history` (affichage des N dernières entrées).
- **Gate B-8** : UC-B2 passe, `:save` produit un JSON consommable.

### §20.9 Sprint B-9 (2 jours) — Tests & doc

- B-9.1 : Suite de tests unitaires (60 tests cibles).
- B-9.2 : Tests d'intégration subprocess (20 tests).
- B-9.3 : Tests pexpect (15 tests).
- B-9.4 : `docs/repl.md`.
- B-9.5 : Sections CLI dans README mise à jour.
- **Gate B-9** : couverture ≥ 85 %, CI verte, doc cohérente avec `--help`.

### §20.10 Synthèse

**Total : ~15 jours-homme.** Livrable : un subcommand `repl` complet avec 26 commandes `:`, historique persistant, couleurs, tab completion, robustesse aux interruptions, suite de tests, documentation.

---

## §21. Matrice de risques

| ID | Risque | Probabilité | Impact | Mitigation | Détection |
|---|---|---|---|---|---|
| R-B1 | `readline` absent sur Windows | Certaine | Haut | Extra `[repl-win]` + pyreadline3 ; fallback graceful | Test au boot |
| R-B2 | Ctrl-C tue la session par accident | Moyenne | Haut | Handler SIGINT dédié + tests pexpect | Tests pexpect |
| R-B3 | Exception non catchée tue le REPL | Moyenne | Haut | Tampon d'exception dans main loop (§11.3) | Tests fuzzing |
| R-B4 | Historique leake des secrets | Moyenne | Haut | `--no-history`, détection heuristique, 0600 | Audit de code |
| R-B5 | Fichier d'historique corrompu | Faible | Moyen | Tolerate parse errors, reset silencieux | Tests unitaires |
| R-B6 | `:reload` incohérent (fingerprint mal calculé) | Faible | Haut | Réutiliser le même code que A (M1) | Tests partagés avec A |
| R-B7 | Cold-start modèle trop long | Moyenne | Moyen | Message explicite, pas de blocage silencieux | Bench manuel |
| R-B8 | Fuites mémoire sur sessions longues | Moyenne | Moyen | Plafonnement de `state.history` et `state.last_results` à 1000 | Stress test 10k queries |
| R-B9 | `:endpoint` laisse un ancien canal gRPC ouvert | Faible | Moyen | `close()` systématique avant reconstruction | Tests unitaires |
| R-B10 | Tab completion bloque sur gros répertoires | Faible | Faible | Timeout + best-effort | Test manuel |
| R-B11 | Couleurs cassent un `less -R` sans `-r` | Faible | Faible | Détection TTY rigoureuse | Tests |
| R-B12 | Incompatibilité pyreadline3 version | Faible | Moyen | Test matriciel CI Windows | CI Windows |

---

## §22. Critères Go / No-Go spécifiques à B

### §22.1 Critères bloquants

- **G-B1 Boot** : `efe-admin repl --catalog SUPPORT.yaml` démarre en < 500 ms (cache hit) sur CI Linux.
- **G-B2 Séquence UC-B1** : loop → query → `:reload` après édition YAML → query → label changé. Scénario manuel reproductible.
- **G-B3 Ctrl-C safety** : 100 appuis successifs Ctrl-C au prompt → session toujours vivante, exit 0 à `:quit`.
- **G-B4 Exception safety** : injecter une exception dans `client.classify` via monkey-patch → la session survit, log stderr produit, prompt ré-affiché.
- **G-B5 Historique** : historique persisté entre deux sessions ; `--no-history` n'écrit rien.
- **G-B6 Tab completion** : `:re<TAB>` propose `:reload` ; `:catalog cat<TAB>` complète les chemins.
- **G-B7 Tests** : 95 tests verts (60 unit + 20 intégration + 15 pexpect) ; CI verte 5 runs consécutifs.
- **G-B8 Signaux** : SIGTERM → shutdown propre, exit 143. SIGHUP idem.
- **G-B9 Doc** : `docs/repl.md` décrit les 26 commandes ; cohérent avec `:help` (test automatisé).
- **G-B10 Migration de l'historique** : un fichier historique vide ou inexistant ne fait pas échouer le boot.

### §22.2 Non-bloquants (nice-to-have)

- `--watch` (auto-reload YAML).
- Mode multi-catalog dans une session (comparaison côte-à-côte).
- Export transcript en formats divers (CSV, HTML).
- Stats avancées (p99, distribution horaire).
- Plugin system pour ajouter des `:` commands externes.
- Autocomplete sur les classes (pour `:thresholds <CLASS>`).

### §22.3 Procédure de passage Go

1. Tous les tests verts en CI (Linux + macOS minimum ; Windows en warning).
2. Revue manuelle des 26 commandes par un second dev.
3. Session live de 30 minutes sans crash, sur 3 catalogs différents.
4. Benchmark : 100 queries successives sans fuite mémoire observable (`tracemalloc`).
5. Doc relue, `:help` aligné.
6. Tag `efe-tool X.(Y+1).0`.

---

## §23. Annexes

### §23.A Grammaire EBNF des commandes

```
command    = ":" name [ WS args ]
name       = ALPHA { ALPHA | DIGIT | "-" | "_" }
args       = arg { WS arg }
arg        = quoted | bare
quoted     = "\"" { char_not_quote } "\"" | "'" { char_not_quote } "'"
bare       = { char_not_ws }
query      = utf8_line (ne commence pas par ":")
empty      = WS*
WS         = " " | "\t"
```

### §23.B Matrice exhaustive des 26 commandes REPL

| # | Nom | Args | Mute state | Quit session | Cat requis |
|---|---|---|---|---|---|
| 1 | `:help` | `[NAME]` | non | non | non |
| 2 | `:quit` / `:exit` / `:q` | — | non | **oui** | non |
| 3 | `:catalog` | `[PATH]` | catalog | non | non |
| 4 | `:reload` | — | catalog | non | **oui** |
| 5 | `:embedder` | `[MODEL]` | embedder | non | non |
| 6 | `:endpoint` | `[HOST:PORT]` | client, endpoint | non | non |
| 7 | `:thresholds` | `[high F]` `[medium F]` `[reset]` | overrides | non | non |
| 8 | `:budget` | `[MS]` `[reset]` | budget_ms | non | non |
| 9 | `:timeout` | `[S]` `[reset]` | timeout_s | non | non |
| 10 | `:rounds` | `[N]` `[reset]` | max_rounds | non | non |
| 11 | `:format` | `{human|text|json|table}` | output_format | non | non |
| 12 | `:pretty` | `on|off` | pretty | non | non |
| 13 | `:diagnostics` | `on|off` | diagnostics | non | non |
| 14 | `:strict` | `on|off` | strict | non | non |
| 15 | `:log` | `{debug|info|warn|error|off}` | log_level | non | non |
| 16 | `:quiet` | — | log_level | non | non |
| 17 | `:verbose` | — | log_level | non | non |
| 18 | `:history` | `[N]` `[clear]` | history (clear seul) | non | non |
| 19 | `:save` | `PATH` | — | non | non |
| 20 | `:stats` | — | — | non | non |
| 21 | `:classes` | — | — | non | **oui** |
| 22 | `:clear` | — | — | non | non |
| 23 | `:healthcheck` | — | — | non | non |
| 24 | `:version` | — | — | non | non |
| 25 | `:classify` | `TEXT` | last_results | non | **oui** |
| 26 | `:info` | — | — | non | non |

### §23.C Exit codes de la Lettre B

Identiques à ceux de la Lettre A (§10), avec ajouts :

| Code | Signification spécifique à B |
|---|---|
| 0 | Quit propre (`:quit`, Ctrl-D). |
| 2 | Arg de lancement invalide (même qu'A). |
| 3 | `--catalog` fourni et introuvable au boot (abandon avant REPL). |
| 4 | Catalog fourni et invalide au boot. |
| 5 | `--endpoint` inatteignable au boot **si** `--require-healthy` passé (MVP : flag non exposé, exit 0 avec warning). |
| 130 | SIGINT **reçu au boot** avant l'ouverture du prompt (impossible en steady-state). |
| 143 | SIGTERM pendant une session. |

### §23.D Schéma du transcript `:save`

```json
{
  "schema_version": 1,
  "session_id": "018f8d3c-…",
  "program_version": "0.2.0",
  "boot_config": {
    "catalog": "catalogs/support_triage.yaml",
    "endpoint": "127.0.0.1:50051",
    "embedder": "sentence-transformers/…@384"
  },
  "started_at": "2026-04-19T14:58:01.000Z",
  "ended_at": "2026-04-19T15:21:14.000Z",
  "duration_s": 1393.0,
  "entries": [
    {
      "kind": "query",
      "at": "…",
      "text": "…",
      "result": {"label": "fraud", "confidence": 0.575, "rounds": 4, …},
      "elapsed_ms": 842
    },
    {
      "kind": "command",
      "at": "…",
      "name": "reload",
      "args": [],
      "outcome": "ok",
      "details": "recompiled (fingerprint abc → def)"
    },
    {
      "kind": "error",
      "at": "…",
      "code": "SERVER_UNREACHABLE",
      "message": "…"
    }
  ],
  "stats": {
    "queries": 42,
    "commands": 17,
    "errors": 2,
    "labels": {"fraud": 12, "bug": 7, "question": 15, "billing": 8},
    "latency_p50_ms": 812,
    "latency_p95_ms": 1654
  }
}
```

### §23.E Matrice des configurations mutables

| Clé | Valeur initiale | Source | Commande de mutation | Reset |
|---|---|---|---|---|
| endpoint | `ResolvedConfig.endpoint` | boot | `:endpoint H:P` | `:endpoint reset` |
| budget_ms | `ResolvedConfig.budget_ms` | boot | `:budget MS` | `:budget reset` |
| timeout_s | `ResolvedConfig.timeout_s` | boot | `:timeout S` | `:timeout reset` |
| max_rounds | `ResolvedConfig.max_rounds` | boot | `:rounds N` | `:rounds reset` |
| strict | `ResolvedConfig.strict` | boot | `:strict on/off` | `:strict reset` |
| output_format | `"human"` | fixe REPL | `:format FMT` | `:format human` |
| pretty | `ResolvedConfig.pretty` | boot | `:pretty on/off` | `:pretty reset` |
| diagnostics | `ResolvedConfig.with_diagnostics` | boot | `:diagnostics on/off` | idem |
| threshold_high_override | `None` | runtime | `:thresholds high F` | `:thresholds reset` |
| threshold_medium_override | `None` | runtime | `:thresholds medium F` | idem |
| log_level | `ResolvedConfig.log_level` | boot | `:log LEVEL` | `:log reset` |

### §23.F Session annotée

```
$ efe-admin repl --catalog catalogs/support_triage.yaml
# (boot, ~300 ms)
EFE Mode 1 — interactive REPL v0.2.0
catalog: support_triage@1.0.0 (4 classes, multilingual-MiniLM-L12)
endpoint: 127.0.0.1:50051 (insecure)
Type :help for commands, :quit to exit.

EFE support_triage> :info
session    018f…
catalog    support_triage@1.0.0 (fingerprint abc123)
endpoint   127.0.0.1:50051
embedder   sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2@384
budget     5000 ms
timeout    10.0 s
rounds     16
format     human
strict     off
diagnostics off

EFE support_triage> Mon compte a été piraté
  label:       fraud      (confidence 0.575, rounds 4)
  verdict:     trusted

EFE support_triage> :diagnostics on
diagnostics: on

EFE support_triage> J'ai perdu ma carte
  label:       billing    (confidence 0.515, rounds 5)
  verdict:     warn
  diagnostics:
    fraud      0.412
    bug        0.207
    question   0.314
    billing    0.531 ← top

EFE support_triage> # l'utilisateur édite le YAML pour ajouter « perdu » à fraud
EFE support_triage> :reload
reloaded support_triage@1.0.1 (fingerprint abc123 → def456, 4 classes)

EFE support_triage> J'ai perdu ma carte
  label:       fraud      (confidence 0.583, rounds 4)
  verdict:     trusted

EFE support_triage> :stats
Queries:      3
Labels:       fraud=2, billing=1
Latency p50:  0.812 s
Latency p95:  1.234 s
Session:      0h 02m 14s

EFE support_triage> :save /tmp/tuning-session.json
saved 3 entries to /tmp/tuning-session.json

EFE support_triage> :quit
bye — 3 queries, 2 commands, 0h 02m 31s
```

### §23.G ADR — décisions d'architecture clés

- **ADR-B-001** : Implémentation via `while True: input(...)` + `readline`. Rejeté : stdlib `cmd`, `prompt_toolkit`, `rich`. Raison : zéro dépendance, contrôle total du layout.
- **ADR-B-002** : Commandes préfixées par `:` (pas `/`, pas de prefix). Raison : convention familière (vim, irc, psql), pas de collision avec du texte en langage naturel.
- **ADR-B-003** : Un seul catalog actif à la fois, `:catalog PATH` pour basculer. Raison : simplicité, pas d'ambiguïté de routing. Mode multi-catalog post-MVP.
- **ADR-B-004** : Hot-reload explicite via `:reload` uniquement. Pas de watchdog filesystem au MVP. Raison : évite la complexité thread/event-loop, fait le job en pratique.
- **ADR-B-005** : Format stdout par défaut = `human` (pas JSON). Raison : l'utilisateur lit au terminal, pas au pipe.
- **ADR-B-006** : Historique dans `$XDG_DATA_HOME/efe/history` (fallback `~/.efe/history`). Raison : respecte XDG Base Directory Specification.
- **ADR-B-007** : Exceptions attrapées à tous les niveaux dans la boucle. Raison : invariant de robustesse (P3).
- **ADR-B-008** : SIGINT abandonne la requête, pas la session. Raison : convention REPL (psql, python REPL, etc.).
- **ADR-B-009** : Single-threaded. Raison : pas de besoin, évite 90 % des bugs.
- **ADR-B-010** : Embedder et client sont des singletons de session, jamais recréés sauf sur `:endpoint` ou `:embedder`. Raison : amortir le coût (C1).
- **ADR-B-011** : Pas de nouvelle dépendance runtime obligatoire (Unix). Raison : discipline zero-bloat.
- **ADR-B-012** : SemVer strict sur le DSL de commandes, dépréciation sur 3 versions. Raison : parité avec l'API CLI de la Lettre A.
- **ADR-B-013** : `SessionState` mutable (non frozen). Raison : chaque `:xxx` a besoin de muter un champ ; alternative (`replace()` systématique) est verbeuse pour un single-thread.

### §23.H Glossaire

- **REPL** — Read-Eval-Print-Loop. Boucle interactive classique.
- **Session** — L'instance vivante du REPL, du boot au shutdown.
- **SessionState** — L'objet Python qui contient l'état mutable courant.
- **Hot-reload** — Rechargement d'une ressource (ici, un catalog YAML) sans redémarrer le processus.
- **Tab completion** — Auto-complétion quand l'utilisateur presse Tab.
- **Transcript** — Export structuré d'une session (texte + verdicts + commandes).
- **Prompt** — La chaîne (colorée) affichée avant chaque input.
- **DSL** — Domain-Specific Language. Ici, le jeu de commandes `:xxx`.

### §23.I Comparaison avec des REPL de référence

| Propriété | `psql` | Python REPL | `jshell` | `redis-cli` | **efe-admin repl** |
|---|---|---|---|---|---|
| Prefix des commandes | `\` | `exit()`… | `/` | `> HELP` | `:` |
| Hot-reload de state | `\c` | reload modules | `/reset` | — | `:reload` / `:catalog` |
| Tab completion | oui | oui | oui | oui | oui |
| Historique persistent | oui | oui | oui | oui | oui (XDG) |
| Schema d'export | `\o` | — | `/save` | `MONITOR` | `:save` (JSON versionné) |
| Signal handling | OK | OK | OK | OK | OK (§5.5) |
| Couleurs | partielles | via IPython | non | non | natif |

---

**Fin du document.**

*Ce document est la source de vérité pour la Lettre B. Toute évolution passe par PR sur ce fichier avec revue d'un second mainteneur. Les ADR (§23.G) sont append-only.*
