# ADR-SWARM-015 : Messagerie ciblée agent↔agent = **lane fiable adressée** — boîtes aux lettres sur la vérité, réveil-indice, conversation prouvée

- **Status** : Accepted · **Date** : 2026-06-12
- **Phase** : la quatrième voie du passage d'information (après broadcast-indice,
  vérité partagée, influence par tours)
- **Related** : architecture `docs/reports/EFE_Archi_Messagerie_Ciblee_2026-06-12.html` ;
  `reliable.rs` (WS1-F2, l'ancêtre direct) ; `work_queue.rs` (WS4, le témoin shardé) ;
  ADR-SWARM-013 (convention `_reads` + check DAG) ; ADR-SWARM-014 (appariement
  déterministe des ancres) ; Lot MSG · bench_mail (artefacts
  `scripts/swarm/EFE_SWARM_MSG_2026-06-12.json` · `EFE_SWARM_MAILBENCH_2026-06-12.json`)

## Contexte

Aucune voie du substrat ne savait dire « **toi, précisément** » : le broadcast est
anonyme et avec perte, la stigmergie n'a pas de destinataire, l'itératif influence de
vague à vague. Trois conventions d'adressage informelles existaient pourtant
(`ballot.<j>`, `shard_topic`, topics WS2) — sans enveloppe, sans fiabilité, sans
preuve. La baseline avait chiffré les alternatives : bus best-effort ~99 % de perte
sous charge ; claim broadcast 90→99,9 % de CAS gaspillés (O(N×K)). Le propriétaire a
ordonné le build sur l'architecture du 2026-06-12.

## Décision

1. **La messagerie est une SPÉCIALISATION, pas une primitive** : une boîte aux
   lettres est un namespace du Blackboard (`/mail/<to>/<seq:020>`, write-once,
   plafonné, transient d'époque) ; l'envoi écrit la VÉRITÉ d'abord puis publie le
   réveil-indice `mail.<to>` (best-effort) ; la réception draine par curseur
   (exactly-once, in-order, O(livrés)). `efe-blackboard/src/mailbox.rs` (~640 lignes
   tests compris) : `MsgKind` (enum FERMÉ {Request, Reply, Inform}), `Letter`
   (contenu + adresse exacte pointer@version + `signed_by`), `Mailboxes`
   (send/request/reply/reader/new_epoch), `MailReader`
   (drain/recv_batch/await_reply avec mise de côté).
2. **Trois anti-architectures REJETÉES, versionnées** (architecture §3) : pas de
   canal direct (`mpsc` — un message en vol ne décide jamais, invisible à la preuve) ;
   pas de bus fiabilisé (acks/retries/DLQ — la voie écartée par le keystone) ; pas de
   mailbox d'acteur (privatise ce qu'EFE veut public). **NON-features gravées** : pas
   de TTL, pas de priorités, pas d'accusés de réception (la lecture prouvée est plus
   forte), pas d'inter-process (le substrat est in-process ; le réseau est l'affaire
   du serveur).
3. **Déviation assumée vs l'ancêtre — le seq SANS gap** : `ReliableTopic` attribue
   le seq par `fetch_add` puis écrit (un échec d'écriture brûle un seq → gap
   définitif). Ici le seq est attribué **par la vérité** : boucle CAS
   `write_if(key(n), 0, …)`, le compteur n'est qu'un cache — un envoi refusé au cap
   (chemin NORMAL, I-MSG5) n'avance rien : jamais de gap par rejet, et la fenêtre de
   crash-entre-deux de l'ancêtre n'existe plus (l'attribution EST l'écriture).
4. **La conversation PROUVÉE par composition** — format de record GELÉ, zéro champ
   nouveau : l'envoi est une écriture ancrée (WriteSink + appariement ADR-014) avec
   **triple concordance** contrôlable (`from` = `written_by` = `agent_id` du record) ;
   la lecture est déclarée (`Letter::attach_to_task` → convention `_reads` → check
   DAG anti-version-fantôme) ; une réponse valide PORTE la corrélation ET déclare la
   lecture de la cellule de requête — la causalité conversationnelle est dans le DAG.
   Sémantique exacte du check (documentée par le gardien) : le DAG juge les versions
   des pointeurs **scellés** ; une lecture d'un pointeur que personne n'a scellé est
   une lecture externe, tolérée.
5. **Opt-in strict** (parité 013/014) : aucune topologie livrée ne change — T1/T3/
   Consensus appellent leurs agents par trait, moins cher que tout courrier. La
   primitive vise les agents découplés dans le temps (streaming WS2, délégation
   dynamique, requête/réponse contradictoire). Dashboard : `Lane::Mail` (le trafic
   de courrier est observable jour 1).

## Invariants introduits (gardiens nominatifs — 14 tests)

| Inv. | Énoncé | Gardien |
|---|---|---|
| I-MSG1 | vérité d'abord : pas de cellule ⇒ pas de notif | `mail_is_truth_first_no_cell_no_notify` |
| I-MSG2 | le bus ne décide jamais : perte totale d'indice ⇒ livraison totale | `all_mail_delivered_despite_total_notify_loss` · bench ① (4 000 lettres, **0 réveil consommé**) |
| I-MSG3 | exactly-once, in-order, gap-conservateur | `cursor_is_exactly_once_and_gap_conservative` · `multi_sender_single_box_is_seq_monotone` (8 expéditeurs ∥, 400 seq contigus) |
| I-MSG4 | envoi ancré + triple concordance | `send_is_anchored_and_from_matches_writer` |
| I-MSG5 | boîte pleine ⇒ Err à l'expéditeur, jamais d'éviction de non-lu | `full…` (couvert par `mail_is_truth_first…` : cap → Err + courrier intact) |
| I-MSG6 | transience d'époque (purge + seq remis à zéro) | `epoch_purges_mail_and_resets_seq` |
| I-MSG7 | corrélation prouvée par lecture déclarée (DAG) | `reply_correlates_and_declares_reading_the_request` (+ contre-épreuve : version fantôme → DAG inconsistant) |
| — | mise de côté sans perte ; timeout honnête ; queue après fermeture ; réveil ; quiescence WS2 ; lane dashboard ; bout-en-bout re-prouvé | `await_reply_sets_aside…` · `await_reply_times_out…` · `drain_after_router_dropped…` · `wakeup_path_delivers…` · `quiescence_counts_addressed_mail` · `dashboard classify /mail` · `request_reply_end_to_end_reproves_offline` |

## Verdicts des gates (exécutés le jour du build — artefacts datés)

- **Lot MSG : CONDITIONNEL — σ\* = 2,0** (délégation 1-parmi-K, 400 items × 8
  workers, 3 bras à coûts partagés, skew lognormal balayé {0 ; 0,5 ; 1 ; 1,5 ; 2 ;
  2,5}). Le broadcast confirme le prior **au dixième** : 87,5 % de CAS gaspillés =
  (K−1)/K. L'adressage élimine les CAS (0 partout), équilibre la charge ~8× mieux
  (σ_charge 32,6 → 3,98 à skew 1,5), à coût de coordination IDENTIQUE (×1,00) — mais
  le gain p99 vs le shard statique ne franchit la barre des 25 % (écrite avant
  mesure) qu'à partir de **σ = 2,0** : +12,3 % / +8,3 % / +15,9 % / +15,3 % /
  **+40,2 %** / **+58,1 %**. **Notice d'emploi versionnée** : sous σ\*≈2
  d'hétérogénéité, restez en shardé (WS4) ; au-delà, l'adressage dynamique gagne
  large. Honnêteté du harnais (dans l'artefact) : témoin shardé SANS vol (le vol WS4
  est temporel, hors harnais analytique) ; temps de travail virtuel (le harnais
  mesure l'affectation + la coordination réelles). Le p99 reste borné par
  makespan ≥ max(coût_item) — même l'équilibrage parfait ne scinde pas un item géant.
- **bench_mail : GO** — ① brutalité : 4 000 lettres, bus cap 1, **0 réveil
  consommé** → 100 % livré, in-order, exactly-once (la livraison n'a tenu qu'à la
  vérité — LE test qui sépare cette architecture d'un broker) ; ② coût nu :
  **send 1,69 µs · drain 1,20 µs/lettre** (≈ 2 % d'un appel d'agent Mode A, ~90 µs
  CPU au bench_width) ; ③ requête/réponse : 800 RTT, **p50 21 µs · p99 150 µs**,
  16 conversations croisées, 0 corrélation mélangée.

## Conséquences

### Positives
- « Qui a dit quoi **à qui**, et qui l'a lu » entre dans le moat — re-prouvable hors
  ligne depuis les seuls records, format gelé (le tiers du test
  `request_reply_end_to_end_reproves_offline` recompte la conversation entière).
- Trois conventions informelles unifiées en une primitive contractuelle, héritière
  des garanties TESTÉES de la lane WS1-F2 — ~640 LoC dont la moitié de gardiens.
- Le seq-par-la-vérité supprime une classe de gap de l'ancêtre (amélioration notée
  pour un éventuel backport vers `reliable.rs` — non fait ici : zéro churn).

### Négatives / risques tenus
- Le gate dit la vérité : sur la délégation HOMOGÈNE, le shard statique suffit —
  l'adressage ne se justifie qu'à forte hétérogénéité (σ\* = 2,0) ou pour ses cas
  propres (requête/réponse prouvé, agents long-vivants). C'est la notice, pas un échec.
- Boîte jamais drainée = accumulation jusqu'au cap puis Err aux expéditeurs —
  backpressure VOULU, visible (`rejections_total`, `namespace_bytes`).
- Le gap bloquant par seq manquant reste possible par écriture DIRECTE hors
  `Mailboxes` dans `/mail` (misuse du namespace) — documenté ; les cellules
  malformées se lisent en `Inform` conservateur, rien ne panique.
- In-process seulement ; le dispatcher du bras targeted est un MOTIF d'usage, pas
  une pièce du substrat.

## Références
- `crates/efe-blackboard/src/mailbox.rs` (+ exports lib.rs)
- `crates/efe-swarm-dashboard/src/event.rs` (Lane::Mail)
- `crates/efe-swarm-provenance/tests/mailbox_provenance.rs` ·
  `crates/efe-swarm-provenance/examples/swarm_mailbox.rs`
- `crates/efe-blackboard/examples/{lot_msg,bench_mail}.rs`
- `scripts/swarm/EFE_SWARM_MSG_2026-06-12.json` · `EFE_SWARM_MAILBENCH_2026-06-12.json`
